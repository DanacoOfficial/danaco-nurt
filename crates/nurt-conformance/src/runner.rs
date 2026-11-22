// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Execution of a suite and the conformance report.
//!
//! Every case runs in a new server process with its own working, state and
//! configuration directories; cases run in parallel.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Error;
use crate::case::{Case, Class, Kind, Suite, major};
use crate::client::{CommandLine, Connection, ProcessConnection};
use crate::compare::sha256;
use crate::execute::{self, Context};
use crate::fake::CASE_ENV;
use crate::normalize::VolatilePaths;
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
    /// Time limit of one case.
    pub timeout: Duration,
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
    let probe = probe(&command, options.timeout)?;
    let volatile = Context::response_volatile_paths(&schemas);
    let shared = Shared {
        command: &command,
        schemas: &schemas,
        volatile: &volatile,
        probe: &probe,
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

/// Directories of one server process.
struct Sandbox {
    _root: tempfile::TempDir,
    work: PathBuf,
    env: Vec<(String, OsString)>,
}

impl Sandbox {
    fn new(case: Option<&Case>) -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(|e| format!("cannot create a directory: {e}"))?;
        let work = root.path().join("work");
        let state = root.path().join("state");
        let config_dir = root.path().join("config");
        let config = config_dir.join("config.toml");
        for dir in [&work, &state, &config_dir] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let content = case.and_then(|c| c.config.as_deref()).unwrap_or_default();
        std::fs::write(&config, content).map_err(|e| format!("{}: {e}", config.display()))?;
        if let Some(snapshot) = case.and_then(|c| c.fs_before.as_ref()) {
            snapshot.materialize(&work).map_err(|e| e.to_string())?;
        }
        let mut env = vec![
            (STATE_ENV.to_owned(), state.into_os_string()),
            (CONFIG_ENV.to_owned(), config.into_os_string()),
        ];
        if let Some(case) = case {
            env.push((CASE_ENV.to_owned(), OsString::from(&case.id)));
        }
        Ok(Self {
            _root: root,
            work,
            env,
        })
    }
}

fn initialize(connection: &mut ProcessConnection) -> Result<Value, String> {
    let params = protocol::initialize_params(CLIENT, env!("CARGO_PKG_VERSION"), true);
    connection
        .call(method::INITIALIZE, &params)
        .map_err(|error| format!("initialize: {error}"))
}

fn probe(command: &CommandLine, timeout: Duration) -> Result<Probe, Error> {
    let sandbox = Sandbox::new(None).map_err(Error::Invalid)?;
    let deadline = Instant::now() + timeout;
    let mut connection = ProcessConnection::spawn(command, &sandbox.env, deadline)
        .map_err(|e| Error::Invalid(format!("the implementation does not start: {e}")))?;
    let initialized = initialize(&mut connection)
        .map_err(|e| Error::Invalid(format!("the implementation does not answer {e}")))?;
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

/// Returns why a case does not run against the probed implementation.
fn skip_reason(case: &Case, probe: &Probe) -> Option<String> {
    if case.language_major() != major(&probe.language) {
        return Some(format!(
            "language {} differs from {} of the implementation",
            case.language, probe.language
        ));
    }
    if let Some(condition) = case.requires.iter().find(|c| !c.holds(&probe.features)) {
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
    if let Some(reason) = skip_reason(case, shared.probe) {
        result.status = Status::Skip;
        result.reason = Some(reason);
        return result;
    }
    result.failures = execute_case(case, shared);
    if !result.failures.is_empty() {
        result.status = Status::Fail;
    }
    result.duration_ms = millis(clock.elapsed());
    result
}

fn execute_case(case: &Case, shared: &Shared<'_>) -> Vec<Failure> {
    let sandbox = match Sandbox::new(Some(case)) {
        Ok(sandbox) => sandbox,
        Err(message) => return vec![Failure::new(message)],
    };
    let deadline = Instant::now() + shared.timeout;
    let mut connection = match ProcessConnection::spawn(shared.command, &sandbox.env, deadline) {
        Ok(connection) => connection,
        Err(error) => return vec![Failure::new(error.to_string())],
    };
    if let Err(message) = initialize(&mut connection) {
        return vec![Failure::new(message)];
    }
    let context = Context {
        schemas: shared.schemas,
        response_volatile: shared.volatile,
    };
    let failures = execute::execute(case, &mut connection, &sandbox.work, &context);
    connection.shutdown();
    failures
}

/// Parses a duration such as `30s`, `500ms` or `2m`.
///
/// # Errors
///
/// Returns an error for a missing number or an unknown unit.
pub fn parse_duration(text: &str) -> Result<Duration, Error> {
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let invalid = || {
        Error::Invalid(format!(
            "{text:?} is not a duration such as 30s, 500ms or 2m"
        ))
    };
    let value: u64 = text[..digits].parse().map_err(|_| invalid())?;
    match &text[digits..] {
        "ms" => Ok(Duration::from_millis(value)),
        "s" | "" => Ok(Duration::from_secs(value)),
        "m" => Ok(Duration::from_secs(
            value.saturating_mul(SECONDS_PER_MINUTE),
        )),
        _ => Err(invalid()),
    }
}

const SECONDS_PER_MINUTE: u64 = 60;

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
    use crate::case::{Condition, Plan};

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
        assert_eq!(
            skip_reason(&case(Class::Mandatory, Vec::new(), "1.0"), &probe),
            None
        );
        assert!(
            skip_reason(&case(Class::Mandatory, Vec::new(), "2.0"), &probe)
                .unwrap()
                .contains("language")
        );
        let feature = Condition::Feature("testkit".to_owned());
        assert!(
            skip_reason(&case(Class::Mandatory, vec![feature], "1.0"), &probe)
                .unwrap()
                .contains("requires")
        );
        let optional = Class::Optional("testkit".to_owned());
        assert!(
            skip_reason(&case(optional, Vec::new(), "1.0"), &probe)
                .unwrap()
                .contains("not listed")
        );
        assert_eq!(
            skip_reason(
                &case(Class::Optional("events".to_owned()), Vec::new(), "1.0"),
                &probe
            ),
            None
        );
    }

    #[test]
    fn sandboxes_hold_the_configuration_and_the_environment() {
        let case = case(Class::Mandatory, Vec::new(), "1.0");
        let sandbox = Sandbox::new(Some(&case)).unwrap();
        assert!(sandbox.work.is_dir());
        let names: Vec<&str> = sandbox.env.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, [STATE_ENV, CONFIG_ENV, CASE_ENV]);
        let config = std::fs::read_to_string(&sandbox.env[1].1).unwrap();
        assert_eq!(config, "[output]\n");
    }

    #[test]
    fn durations_are_parsed() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("7").unwrap(), Duration::from_secs(7));
        assert!(parse_duration("s").is_err());
        assert!(parse_duration("5h").is_err());
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
            timeout: Duration::from_secs(5),
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
