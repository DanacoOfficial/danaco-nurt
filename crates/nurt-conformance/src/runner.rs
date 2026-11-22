// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Execution of a suite and the conformance report.
//!
//! Every case runs in a new server process with its own working, state and
//! configuration directories, and with the fixtures it requires; cases run
//! in parallel.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Error;
use crate::case::{Case, Class, Condition, Kind, Suite, major};
use crate::client::{CommandLine, Connection, ProcessConnection};
use crate::compare::sha256;
use crate::execute::{self, Context, Outcome, Workspace};
use crate::fake::{CASE_ENV, HTTP_BASE_ENV, SECOND_VOLUME_ENV};
use crate::fixture::{self, Fixture, HttpFixture};
use crate::normalize::{
    BASE_PLACEHOLDER, SECOND_VOLUME_PLACEHOLDER, VolatilePaths, WORK_MOUNT_PLACEHOLDER,
};
use crate::protocol::{self, member, method};
use crate::report::{
    CaseResult, ConformanceReport, FORMAT, Failure, Implementation, Status, SuiteInfo, Summary,
    System,
};
use crate::schemas::SchemaSet;

/// Environment variable naming the state directory of an instance.
pub const STATE_ENV: &str = "NURT_STATE_DIR";

/// Environment variable naming the configuration file of an instance.
pub const CONFIG_ENV: &str = "NURT_CONFIG";

/// Name of the client in `initialize`.
const CLIENT: &str = "nurt-conformance";

/// Options of a run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Command line that starts the server.
    pub command: String,
    /// Root directory of the cases.
    pub cases: PathBuf,
    /// Identifier filters.
    pub filters: Vec<String>,
    /// Kinds to run; all when empty.
    pub kinds: Vec<Kind>,
    /// Number of cases run in parallel.
    pub jobs: usize,
    /// Time limit of the start of a server process and of its answer to
    /// `initialize`.
    pub startup_timeout: Duration,
    /// Time limit of one case, counted from the answer to `initialize`.
    pub timeout: Duration,
    /// Directory on another file system than the temporary directory, in
    /// which the cases that require `fixture:second-volume` get their
    /// second directory. Without it these cases are skipped. [`run`] does
    /// not check the volume; [`fixture::check_second_volume`] does.
    pub second_volume: Option<PathBuf>,
}

/// Description of the implementation obtained before the cases run.
#[derive(Debug, Clone, Default)]
struct Probe {
    initialize: Option<Value>,
    info: Option<Value>,
    features: BTreeSet<String>,
    language: String,
}

/// Runs the selected cases of the suite and returns the report.
///
/// # Errors
///
/// Returns an error when the command line is not valid, the suite cannot be
/// loaded, or the implementation does not answer `initialize`.
pub fn run(options: &RunOptions) -> Result<ConformanceReport, Error> {
    let started = jiff::Timestamp::now();
    let clock = Instant::now();
    let command = CommandLine::parse(&options.command)?;
    let schemas = SchemaSet::new()?;
    let suite = Suite::load(&options.cases, &schemas)?;
    let selected = suite.select(&options.filters, &options.kinds);
    let probe = probe(&command, options.startup_timeout, options.timeout)?;
    let volatile = Context::response_volatile_paths(&schemas);
    let mut fixtures = vec![Fixture::Http];
    if options.second_volume.is_some() {
        fixtures.push(Fixture::SecondVolume);
    }
    let shared = Shared {
        command: &command,
        schemas: &schemas,
        volatile: &volatile,
        probe: &probe,
        fixtures: &fixtures,
        second_volume: options.second_volume.as_deref(),
        startup_timeout: options.startup_timeout,
        timeout: options.timeout,
    };
    let results = run_parallel(&selected, &shared, options.jobs.max(1));
    let complete = options.filters.is_empty() && options.kinds.is_empty();
    let summary = Summary::from_results(&results, complete);
    let mut filter = options.filters.clone();
    filter.extend(
        options
            .kinds
            .iter()
            .map(|kind| format!("kind:{}", kind.name())),
    );
    Ok(ConformanceReport {
        format: FORMAT.to_owned(),
        suite: SuiteInfo {
            version: suite.version.clone(),
            digest: suite.digest.clone(),
            cases: suite.cases.len(),
            filter,
        },
        implementation: Implementation {
            command: options.command.clone(),
            executable_digest: command
                .executable()
                .and_then(|path| std::fs::read(path).ok())
                .map(|bytes| sha256(&bytes)),
            initialize: probe.initialize,
            info: probe.info,
        },
        system: System::current(),
        started: started.to_string(),
        duration_ms: millis(clock.elapsed()),
        results,
        summary,
    })
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Directories and fixtures of one server process.
struct Sandbox {
    _root: tempfile::TempDir,
    _second_root: Option<tempfile::TempDir>,
    http: Option<HttpFixture>,
    work: PathBuf,
    second_volume: Option<PathBuf>,
    env: Vec<(String, OsString)>,
}

impl Sandbox {
    /// Creates the directories of a case, or of the probe without a case,
    /// and starts the fixtures the case requires. `second_volume` is the
    /// directory of the run on the second volume.
    fn new(case: Option<&Case>, second_volume: Option<&Path>) -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(|e| format!("cannot create a directory: {e}"))?;
        let work = root.path().join("work");
        let state = root.path().join("state");
        let config_dir = root.path().join("config");
        let config = config_dir.join("config.toml");
        for dir in [&work, &state, &config_dir] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let requires = |fixture: Fixture| {
            case.is_some_and(|c| c.requires.contains(&Condition::Fixture(fixture)))
        };
        let second_root = match second_volume.filter(|_| requires(Fixture::SecondVolume)) {
            Some(dir) => Some(
                tempfile::tempdir_in(dir)
                    .map_err(|e| format!("cannot create a directory in {}: {e}", dir.display()))?,
            ),
            None => None,
        };
        let second = second_root.as_ref().map(|dir| dir.path().to_path_buf());
        let http = if requires(Fixture::Http) {
            let files = case.and_then(|c| c.http_files.as_ref());
            Some(HttpFixture::start(files).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let content = match case.and_then(|c| c.config.as_deref()) {
            Some(text) => expand_config(text, &work, second.as_deref())?,
            None => String::new(),
        };
        std::fs::write(&config, content).map_err(|e| format!("{}: {e}", config.display()))?;
        if let Some(snapshot) = case.and_then(|c| c.fs_before.as_ref()) {
            snapshot.materialize(&work).map_err(|e| e.to_string())?;
        }
        if let (Some(snapshot), Some(dir)) =
            (case.and_then(|c| c.second_volume_before.as_ref()), &second)
        {
            snapshot.materialize(dir).map_err(|e| e.to_string())?;
        }
        let mut env = vec![
            (STATE_ENV.to_owned(), state.into_os_string()),
            (CONFIG_ENV.to_owned(), config.into_os_string()),
        ];
        if let Some(case) = case {
            env.push((CASE_ENV.to_owned(), OsString::from(&case.id)));
        }
        if let Some(http) = &http {
            env.push((HTTP_BASE_ENV.to_owned(), OsString::from(http.base())));
        }
        if let Some(dir) = &second {
            env.push((SECOND_VOLUME_ENV.to_owned(), dir.clone().into_os_string()));
        }
        Ok(Self {
            _root: root,
            _second_root: second_root,
            http,
            work,
            second_volume: second,
            env,
        })
    }

    fn workspace(&self) -> Workspace<'_> {
        Workspace {
            work: &self.work,
            second_volume: self.second_volume.as_deref(),
            http_base: self.http.as_ref().map(HttpFixture::base),
        }
    }
}

/// Replaces the placeholders `${base}`, `${second_volume}` and
/// `${work_mount}` in the text of a configuration. The values are written
/// with `/` separators and escaped for a TOML basic string.
fn expand_config(text: &str, work: &Path, second: Option<&Path>) -> Result<String, String> {
    let slashes = |path: &Path| path.to_string_lossy().replace('\\', "/");
    let mut expanded = text.replace(BASE_PLACEHOLDER, &toml_escape(&slashes(work)));
    if expanded.contains(SECOND_VOLUME_PLACEHOLDER) {
        let second = second.ok_or_else(|| {
            format!(
                "{SECOND_VOLUME_PLACEHOLDER} of the configuration requires fixture:second-volume"
            )
        })?;
        expanded = expanded.replace(SECOND_VOLUME_PLACEHOLDER, &toml_escape(&slashes(second)));
    }
    if expanded.contains(WORK_MOUNT_PLACEHOLDER) {
        let mount = fixture::mount_point(work).map_err(|e| e.to_string())?;
        expanded = expanded.replace(WORK_MOUNT_PLACEHOLDER, &toml_escape(&mount));
    }
    Ok(expanded)
}

/// Escapes a text for a TOML basic string (TOML 1.0, "String").
fn toml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

fn initialize(connection: &mut ProcessConnection) -> Result<Value, String> {
    let params = protocol::initialize_params(CLIENT, env!("CARGO_PKG_VERSION"), true);
    connection
        .call(method::INITIALIZE, &params)
        .map_err(|error| format!("initialize: {error}"))
}

fn probe(command: &CommandLine, startup: Duration, timeout: Duration) -> Result<Probe, Error> {
    let sandbox = Sandbox::new(None, None).map_err(Error::Invalid)?;
    let mut connection = ProcessConnection::spawn(command, &sandbox.env, Instant::now() + startup)
        .map_err(|e| Error::Invalid(format!("the implementation does not start: {e}")))?;
    let initialized = initialize(&mut connection)
        .map_err(|e| Error::Invalid(format!("the implementation does not answer {e}")))?;
    connection.set_deadline(Instant::now() + timeout);
    let info = connection.call(method::INFO, &serde_json::json!({})).ok();
    connection.shutdown();
    let features = initialized
        .get(member::FEATURES)
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let language = initialized
        .get(member::LANGUAGE)
        .and_then(Value::as_str)
        .unwrap_or(crate::case::DEFAULT_LANGUAGE)
        .to_owned();
    Ok(Probe {
        initialize: Some(initialized),
        info,
        features,
        language,
    })
}

/// Data shared by the workers of a run.
struct Shared<'a> {
    command: &'a CommandLine,
    schemas: &'a SchemaSet,
    volatile: &'a VolatilePaths,
    probe: &'a Probe,
    fixtures: &'a [Fixture],
    second_volume: Option<&'a Path>,
    startup_timeout: Duration,
    timeout: Duration,
}

fn run_parallel(cases: &[&Case], shared: &Shared<'_>, jobs: usize) -> Vec<CaseResult> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<CaseResult>>> = Mutex::new(vec![None; cases.len()]);
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(cases.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(case) = cases.get(index) else {
                        return;
                    };
                    let result = run_case(case, shared);
                    if let Ok(mut slots) = results.lock() {
                        slots[index] = Some(result);
                    }
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect()
}

/// Returns why a case does not run against the probed implementation with
/// the fixtures of the run.
fn skip_reason(case: &Case, probe: &Probe, fixtures: &[Fixture]) -> Option<String> {
    if case.language_major() != major(&probe.language) {
        return Some(format!(
            "language {} differs from {} of the implementation",
            case.language, probe.language
        ));
    }
    if let Some(condition) = case
        .requires
        .iter()
        .find(|c| !c.holds(&probe.features, fixtures))
    {
        return Some(format!("requires {}", condition.name()));
    }
    match &case.class {
        Class::Optional(feature) if !probe.features.contains(feature) => Some(format!(
            "feature {feature} not listed by the implementation"
        )),
        _ => None,
    }
}

fn run_case(case: &Case, shared: &Shared<'_>) -> CaseResult {
    let clock = Instant::now();
    let mut result = CaseResult {
        id: case.id.clone(),
        kind: case.kind.name().to_owned(),
        class: case.class.name(),
        status: Status::Pass,
        reason: None,
        duration_ms: 0,
        failures: Vec::new(),
    };
    if let Some(reason) = skip_reason(case, shared.probe, shared.fixtures) {
        result.status = Status::Skip;
        result.reason = Some(reason);
        return result;
    }
    let outcome = execute_case(case, shared);
    if let Some(reason) = outcome.skipped {
        result.status = Status::Skip;
        result.reason = Some(reason);
    } else if !outcome.failures.is_empty() {
        result.status = Status::Fail;
        result.failures = outcome.failures;
    }
    result.duration_ms = millis(clock.elapsed());
    result
}

fn execute_case(case: &Case, shared: &Shared<'_>) -> Outcome {
    let failed = |message: String| Outcome {
        failures: vec![Failure::new(message)],
        skipped: None,
    };
    let sandbox = match Sandbox::new(Some(case), shared.second_volume) {
        Ok(sandbox) => sandbox,
        Err(message) => return failed(message),
    };
    let startup = Instant::now() + shared.startup_timeout;
    let mut connection = match ProcessConnection::spawn(shared.command, &sandbox.env, startup) {
        Ok(connection) => connection,
        Err(error) => return failed(error.to_string()),
    };
    if let Err(message) = initialize(&mut connection) {
        return failed(message);
    }
    connection.set_deadline(Instant::now() + shared.timeout);
    let context = Context {
        schemas: shared.schemas,
        response_volatile: shared.volatile,
    };
    let outcome = execute::execute(case, &mut connection, &sandbox.workspace(), &context);
    connection.shutdown();
    outcome
}

/// Parses a duration such as `30s`, `500ms`, `2m`, `2h` or `1d`.
///
/// # Errors
///
/// Returns an error for a missing number or an unknown unit.
///
/// # Examples
///
/// ```
/// use nurt_conformance::runner::parse_duration;
/// use std::time::Duration;
///
/// assert_eq!(parse_duration("2h")?, Duration::from_secs(7200));
/// # Ok::<(), nurt_conformance::Error>(())
/// ```
pub fn parse_duration(text: &str) -> Result<Duration, Error> {
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let invalid = || {
        Error::Invalid(format!(
            "{text:?} is not a duration such as 30s, 500ms, 2m, 2h or 1d"
        ))
    };
    let value: u64 = text[..digits].parse().map_err(|_| invalid())?;
    let seconds = |unit: u64| Duration::from_secs(value.saturating_mul(unit));
    match &text[digits..] {
        "ms" => Ok(Duration::from_millis(value)),
        "s" | "" => Ok(seconds(1)),
        "m" => Ok(seconds(SECONDS_PER_MINUTE)),
        "h" => Ok(seconds(SECONDS_PER_HOUR)),
        "d" => Ok(seconds(SECONDS_PER_DAY)),
        _ => Err(invalid()),
    }
}

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;

/// Returns the default number of parallel cases: the number of processors.
#[must_use]
pub fn default_jobs() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

/// Writes the report as JSON to `path`.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_report(report: &ConformanceReport, path: &Path) -> Result<(), Error> {
    let mut text =
        serde_json::to_string_pretty(report).map_err(|e| Error::Invalid(e.to_string()))?;
    text.push('\n');
    std::fs::write(path, text).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::Plan;
    use crate::snapshot::Snapshot;
    use serde_json::json;

    fn case(class: Class, requires: Vec<Condition>, language: &str) -> Case {
        Case {
            id: "c".to_owned(),
            kind: Kind::Exec,
            class,
            language: language.to_owned(),
            requires,
            summary: "s".to_owned(),
            variant: "a".to_owned(),
            code: None,
            volatile: Vec::new(),
            config: Some("[output]\n".to_owned()),
            fs_before: None,
            second_volume_before: None,
            http_files: None,
            plan: Plan::Steps(Vec::new()),
            source: "case.toml".into(),
        }
    }

    #[test]
    fn cases_are_skipped_by_language_conditions_and_features() {
        let probe = Probe {
            features: ["events".to_owned()].into(),
            language: "1.2".to_owned(),
            ..Probe::default()
        };
        let http = [Fixture::Http];
        assert_eq!(
            skip_reason(&case(Class::Mandatory, Vec::new(), "1.0"), &probe, &http),
            None
        );
        assert!(
            skip_reason(&case(Class::Mandatory, Vec::new(), "2.0"), &probe, &http)
                .unwrap()
                .contains("language")
        );
        let feature = Condition::Feature("testkit".to_owned());
        assert!(
            skip_reason(&case(Class::Mandatory, vec![feature], "1.0"), &probe, &http)
                .unwrap()
                .contains("requires")
        );
        let optional = Class::Optional("testkit".to_owned());
        assert!(
            skip_reason(&case(optional, Vec::new(), "1.0"), &probe, &http)
                .unwrap()
                .contains("not listed")
        );
        assert_eq!(
            skip_reason(
                &case(Class::Optional("events".to_owned()), Vec::new(), "1.0"),
                &probe,
                &http
            ),
            None
        );
        let volume = vec![Condition::Fixture(Fixture::SecondVolume)];
        assert_eq!(
            skip_reason(
                &case(Class::Mandatory, volume.clone(), "1.0"),
                &probe,
                &http
            )
            .as_deref(),
            Some("requires fixture:second-volume")
        );
        assert_eq!(
            skip_reason(
                &case(Class::Mandatory, volume, "1.0"),
                &probe,
                &Fixture::ALL
            ),
            None
        );
    }

    #[test]
    fn sandboxes_hold_the_configuration_and_the_environment() {
        let case = case(Class::Mandatory, Vec::new(), "1.0");
        let sandbox = Sandbox::new(Some(&case), None).unwrap();
        assert!(sandbox.work.is_dir());
        let names: Vec<&str> = sandbox.env.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, [STATE_ENV, CONFIG_ENV, CASE_ENV]);
        let config = std::fs::read_to_string(&sandbox.env[1].1).unwrap();
        assert_eq!(config, "[output]\n");
        assert!(sandbox.workspace().http_base.is_none());
    }

    #[test]
    fn sandboxes_start_the_required_fixtures() {
        let volume = tempfile::tempdir().unwrap();
        let mut case = case(
            Class::Mandatory,
            vec![
                Condition::Fixture(Fixture::Http),
                Condition::Fixture(Fixture::SecondVolume),
            ],
            "1.0",
        );
        case.config = Some(
            "[journal.volume.\"${work_mount}\"]\nmax_size = \"1mib\"\n[a]\nbase = \"${base}\"\nsecond = \"${second_volume}\"\n"
                .to_owned(),
        );
        case.second_volume_before = Some(
            Snapshot::from_json(
                &json!({"entries": [{"path": "b.txt", "type": "file", "content_utf8": "b"}]}),
            )
            .unwrap(),
        );
        case.http_files = Some(Snapshot::from_json(&json!({"entries": []})).unwrap());
        let sandbox = Sandbox::new(Some(&case), Some(volume.path())).unwrap();
        let workspace = sandbox.workspace();
        let second = workspace.second_volume.unwrap();
        assert!(second.starts_with(volume.path()));
        assert_eq!(std::fs::read_to_string(second.join("b.txt")).unwrap(), "b");
        let http = workspace.http_base.unwrap();
        assert!(http.starts_with("http://127.0.0.1:"));
        let names: Vec<&str> = sandbox.env.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                STATE_ENV,
                CONFIG_ENV,
                CASE_ENV,
                HTTP_BASE_ENV,
                SECOND_VOLUME_ENV
            ]
        );
        let config = std::fs::read_to_string(&sandbox.env[1].1).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        let mount = fixture::mount_point(&sandbox.work).unwrap();
        assert_eq!(
            parsed["journal"]["volume"][mount.as_str()]["max_size"].as_str(),
            Some("1mib")
        );
        let slashes = |path: &Path| path.to_string_lossy().replace('\\', "/");
        assert_eq!(
            parsed["a"]["base"].as_str(),
            Some(slashes(&sandbox.work).as_str())
        );
        assert_eq!(
            parsed["a"]["second"].as_str(),
            Some(slashes(second).as_str())
        );

        let without_volume = Sandbox::new(Some(&case), None).err().unwrap();
        assert!(
            without_volume.contains("fixture:second-volume"),
            "{without_volume}"
        );
    }

    #[test]
    fn configuration_values_are_escaped_for_toml() {
        assert_eq!(
            toml_escape("C:/a \"b\"\\c\u{1}"),
            "C:/a \\\"b\\\"\\\\c\\u0001"
        );
        let expanded = expand_config("x = \"${base}\"", Path::new("/w \"q\""), None).unwrap();
        assert_eq!(expanded, "x = \"/w \\\"q\\\"\"");
        assert!(expand_config("x = \"${work_mount}\"", Path::new("/no/such/dir"), None).is_err());
    }

    #[test]
    fn durations_are_parsed() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86_400));
        assert_eq!(parse_duration("7").unwrap(), Duration::from_secs(7));
        assert!(parse_duration("s").is_err());
        assert!(parse_duration("5w").is_err());
        assert!(default_jobs() >= 1);
        assert_eq!(millis(Duration::from_millis(3)), 3);
    }

    #[test]
    fn implementations_that_do_not_start_are_errors() {
        let options = RunOptions {
            command: "no-such-program-of-the-suite".to_owned(),
            cases: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance"),
            filters: Vec::new(),
            kinds: Vec::new(),
            jobs: 1,
            startup_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(5),
            second_volume: None,
        };
        assert!(
            run(&options)
                .unwrap_err()
                .to_string()
                .contains("does not start")
        );
        let bad = RunOptions {
            command: " ".to_owned(),
            ..options
        };
        assert!(run(&bad).is_err());
    }

    #[test]
    fn reports_are_written_as_json() {
        let dir = tempfile::tempdir().unwrap();
        let report = ConformanceReport {
            format: FORMAT.to_owned(),
            suite: SuiteInfo {
                version: "1.0.0".to_owned(),
                digest: String::new(),
                cases: 0,
                filter: Vec::new(),
            },
            implementation: Implementation {
                command: "x".to_owned(),
                executable_digest: None,
                initialize: None,
                info: None,
            },
            system: System::current(),
            started: String::new(),
            duration_ms: 0,
            results: Vec::new(),
            summary: Summary::from_results(&[], true),
        };
        let path = dir.path().join("r.json");
        write_report(&report, &path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("}\n"));
        assert!(write_report(&report, &dir.path().join("none/r.json")).is_err());
    }
}
