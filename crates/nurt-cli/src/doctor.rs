// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `doctor` subcommand: a table of checks of the installation.

use comfy_table::{Table, presets};
use nurt_core::Engine;
use nurt_core::paths::PlatformPaths;

use crate::EngineSource;
use crate::app::{CliError, Context, Io};
use crate::args::SUBCOMMANDS;
use crate::exit;
use crate::integrations;
use crate::text;

/// Name of the file created and removed to test that the state directory is
/// writable.
const PROBE_FILE: &str = ".doctor-probe";

/// Outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Ok,
    Unavailable,
    Failed,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }
}

struct Check {
    name: String,
    verdict: Verdict,
    detail: String,
}

fn check(name: &str, verdict: Verdict, detail: impl Into<String>) -> Check {
    Check {
        name: name.to_owned(),
        verdict,
        detail: detail.into(),
    }
}

fn state_directory(engine: &Engine) -> Check {
    let Some(state) = engine.state_dir() else {
        return check("state directory", Verdict::Unavailable, "not configured");
    };
    let probe = state.root().join(PROBE_FILE);
    let outcome = std::fs::write(&probe, b"").and_then(|()| std::fs::remove_file(&probe));
    let path = state.root().display();
    match outcome {
        Ok(()) => check("state directory", Verdict::Ok, format!("{path}, writable")),
        Err(error) => check(
            "state directory",
            Verdict::Failed,
            format!("{path}, not writable: {error}"),
        ),
    }
}

fn configuration_files(source: &dyn EngineSource) -> Vec<Check> {
    let paths = PlatformPaths::for_host(source.host().as_ref());
    [("system", paths.system_file), ("user", paths.user_file)]
        .into_iter()
        .map(|(level, file)| {
            let name = format!("{level} configuration");
            match file {
                Some(path) if path.is_file() => {
                    check(&name, Verdict::Ok, path.display().to_string())
                }
                Some(path) => check(&name, Verdict::Ok, format!("{}, absent", path.display())),
                None => check(&name, Verdict::Unavailable, "no location on this host"),
            }
        })
        .collect()
}

fn component(name: &str, outcome: Result<(), nurt_contracts::error::Error>) -> Check {
    match outcome {
        Ok(()) => check(name, Verdict::Ok, "available"),
        Err(error) => check(name, Verdict::Unavailable, error.code.as_str()),
    }
}

/// The features of the engine and the Cargo features that link components.
fn features(engine: &[String]) -> String {
    let linked = integrations::COMPONENTS
        .iter()
        .filter(|component| component.linked)
        .filter_map(|component| component.feature)
        .chain(cfg!(feature = "mcp-http").then_some("mcp-http"))
        .chain(cfg!(feature = "stub").then_some("stub"));
    let all: Vec<&str> = engine.iter().map(String::as_str).chain(linked).collect();
    if all.is_empty() {
        "none".to_owned()
    } else {
        all.join(", ")
    }
}

fn gather(context: &Context, source: &dyn EngineSource) -> Vec<Check> {
    let engine = &context.engine;
    let info = engine.info();
    let os = serde_json::to_value(info.os)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut checks = vec![
        check("version", Verdict::Ok, text::VERSION),
        check(
            "platform",
            Verdict::Ok,
            format!("{os} {}", info.architecture),
        ),
        check(
            "formats",
            Verdict::Ok,
            format!(
                "ast {}, bound {}, report {}, envelope {}, config {}",
                info.formats.ast,
                info.formats.bound,
                info.formats.report,
                info.formats.envelope,
                info.formats.config
            ),
        ),
        check("features", Verdict::Ok, features(&info.features)),
        state_directory(engine),
    ];
    checks.extend(configuration_files(source));
    checks.push(check(
        "services",
        if info.services.is_empty() {
            Verdict::Unavailable
        } else {
            Verdict::Ok
        },
        if info.services.is_empty() {
            "none".to_owned()
        } else {
            info.services.join(", ")
        },
    ));
    checks.push(component("journal", engine.space(None).map(|_| ())));
    checks.push(component(
        "language front end",
        engine.describe("core", None).map(|_| ()),
    ));
    checks.push(check(
        "file system features",
        Verdict::Unavailable,
        "not reported by this build",
    ));
    checks.extend(
        SUBCOMMANDS
            .iter()
            .filter(|name| !integrations::is_connected(name))
            .map(|name| unconnected(name)),
    );
    checks
}

/// The check of a subcommand whose component is not connected: whether the
/// crate of the component is linked, or the feature that links it.
fn unconnected(name: &str) -> Check {
    let detail = match integrations::component(name) {
        Some(component) if component.linked && component.feature.is_some() => {
            "linked, not available in this build".to_owned()
        }
        Some(component) => match component.feature {
            Some(feature) => format!("not available in this build; needs the feature `{feature}`"),
            None => "not available in this build".to_owned(),
        },
        None => "not available in this build".to_owned(),
    };
    check(&format!("subcommand {name}"), Verdict::Unavailable, detail)
}

/// Prints the checks of the installation.
///
/// # Errors
///
/// Returns the reason the engine cannot be opened or the output cannot be
/// written.
pub fn run(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("`doctor` takes no arguments".to_owned()));
    }
    let context = Context::open(io.terminal, source)?;
    let checks = gather(&context, source);
    let mut table = Table::new();
    table.load_style(presets::NOTHING);
    table.set_header(["check", "result", "detail"]);
    for entry in &checks {
        table.add_row([
            entry.name.as_str(),
            entry.verdict.label(),
            entry.detail.as_str(),
        ]);
    }
    writeln!(io.out, "{table}")?;
    let failed = checks.iter().any(|entry| entry.verdict == Verdict::Failed);
    Ok(if failed { exit::FAILURE } else { exit::SUCCESS })
}
