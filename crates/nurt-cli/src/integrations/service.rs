// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `service` subcommand: the background service of the journal and the
//! schedules.
//!
//! The forms are `service run`, `service install [--mode user|system]`,
//! `service uninstall [--mode user|system]` and `service status`. `run` hosts
//! the modules in the foreground and prints nothing; the other forms print one
//! object as `key: value` pairs separated by two spaces.

use std::io::Write;
use std::sync::Arc;

use nurt_contracts::config::{Config, ServiceMode};
use nurt_contracts::error::Error;
use nurt_core::config::{ConfigLoader, ConfigSources};
use nurt_core::system::SystemHost;
use nurt_service::module::ServiceModule;
use nurt_service::{
    ConfigSource, InstallRequest, ScheduleService, ServicePaths, ServiceSetup, StatusRequest,
    UninstallRequest,
};
use serde::Serialize;
use serde_json::Value;

use crate::app::CliError;
use crate::exit;

/// A form of the subcommand with its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Runs the service in the foreground.
    Run,
    /// Registers the service with the service manager.
    Install(Option<ServiceMode>),
    /// Removes the registration of the service.
    Uninstall(Option<ServiceMode>),
    /// Reports whether the service is installed and running.
    Status,
}

/// Parses the arguments of `service`.
///
/// # Errors
///
/// Returns a usage error unless the arguments are one of the forms `run`,
/// `install`, `uninstall` and `status`, with `--mode` accepted by `install`
/// and `uninstall` only.
pub fn parse(args: &[String]) -> Result<Form, CliError> {
    let Some((name, rest)) = args.split_first() else {
        return Err(CliError::Usage(
            "`service` needs `run`, `install`, `uninstall` or `status`".to_owned(),
        ));
    };
    match name.as_str() {
        "run" => no_options("run", rest).map(|()| Form::Run),
        "status" => no_options("status", rest).map(|()| Form::Status),
        "install" => mode_option("install", rest).map(Form::Install),
        "uninstall" => mode_option("uninstall", rest).map(Form::Uninstall),
        other => Err(CliError::Usage(format!(
            "unknown form `{other}` of `service`; use `run`, `install`, `uninstall` or `status`"
        ))),
    }
}

fn no_options(form: &str, rest: &[String]) -> Result<(), CliError> {
    match rest.first() {
        None => Ok(()),
        Some(other) => Err(CliError::Usage(format!(
            "unknown argument `{other}` of `service {form}`"
        ))),
    }
}

fn mode_option(form: &str, rest: &[String]) -> Result<Option<ServiceMode>, CliError> {
    let mut mode = None;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        if word != "--mode" {
            return Err(CliError::Usage(format!(
                "unknown argument `{word}` of `service {form}`"
            )));
        }
        let name = words
            .next()
            .ok_or_else(|| CliError::Usage("`--mode` needs a value".to_owned()))?;
        let chosen = match name.as_str() {
            "user" => ServiceMode::User,
            "system" => ServiceMode::System,
            other => {
                return Err(CliError::Usage(format!(
                    "unknown mode `{other}`; use `user` or `system`"
                )));
            }
        };
        if mode.replace(chosen).is_some() {
            return Err(CliError::Usage(
                "`--mode` can be given only once".to_owned(),
            ));
        }
    }
    Ok(mode)
}

fn load_config() -> Result<Config, Error> {
    let host = SystemHost::detect();
    let loader =
        ConfigLoader::new(&ConfigSources::new(), &host).map_err(|error| error.to_error())?;
    let loaded = loader.load(None).map_err(|error| error.to_error())?;
    Ok(loaded.config().clone())
}

struct ReloadSource;

impl ConfigSource for ReloadSource {
    fn load(&self) -> Result<Config, Error> {
        load_config()
    }
}

fn paths(config: &Config) -> Result<ServicePaths, CliError> {
    ServicePaths::from_config(config).map_err(|error| {
        CliError::Failure(format!(
            "cannot determine the state directory: {error}; set `state.dir`"
        ))
    })
}

fn render(report: &impl Serialize) -> Result<String, CliError> {
    let value = serde_json::to_value(report)
        .map_err(|error| CliError::Failure(format!("cannot render the result: {error}")))?;
    let Value::Object(entries) = value else {
        return Err(CliError::Failure("the result is not an object".to_owned()));
    };
    let pairs: Vec<String> = entries
        .iter()
        .filter(|(_, value)| !is_empty(value))
        .map(|(key, value)| format!("{key}: {}", text(value)))
        .collect();
    Ok(pairs.join("  "))
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(entries) => entries.is_empty(),
        _ => false,
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(text).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .map(|(key, value)| format!("{key}: {}", text(value)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        other => other.to_string(),
    }
}

fn print(report: &impl Serialize) -> Result<u8, CliError> {
    let line = render(report)?;
    writeln!(std::io::stdout().lock(), "{line}")?;
    Ok(exit::SUCCESS)
}

/// Runs `nurt service`.
///
/// # Errors
///
/// Returns a usage error for invalid arguments, and the error of the service
/// crate (`E_EXISTS`, `E_PRIVILEGE`, `E_SERVICE`, `E_NOT_FOUND`, `E_IO` and
/// others) for a form that fails.
pub fn run(args: &[String]) -> Result<u8, CliError> {
    let form = parse(args)?;
    let config = load_config()?;
    match form {
        Form::Run => {
            let setup = ServiceSetup::new(config)?.config_source(Arc::new(ReloadSource));
            let modules: Vec<Arc<dyn ServiceModule>> = vec![Arc::new(ScheduleService::new())];
            nurt_service::run(setup, modules)?;
            Ok(exit::SUCCESS)
        }
        Form::Install(mode) => {
            let mut request = InstallRequest::new(paths(&config)?);
            request.mode = mode;
            print(&nurt_service::install(&request)?)
        }
        Form::Uninstall(mode) => {
            let request = UninstallRequest {
                mode,
                paths: paths(&config)?,
                process_host: None,
            };
            print(&nurt_service::uninstall(&request)?)
        }
        Form::Status => {
            let request = StatusRequest::new(paths(&config)?, &config);
            print(&nurt_service::status(&request)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn forms_without_options_take_no_arguments() {
        assert_eq!(parse(&words("run")), Ok(Form::Run));
        assert_eq!(parse(&words("status")), Ok(Form::Status));
        for bad in ["run now", "status --mode user", "run --mode system"] {
            assert!(
                matches!(parse(&words(bad)), Err(CliError::Usage(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn install_and_uninstall_take_an_optional_mode() {
        assert_eq!(parse(&words("install")), Ok(Form::Install(None)));
        assert_eq!(
            parse(&words("install --mode user")),
            Ok(Form::Install(Some(ServiceMode::User)))
        );
        assert_eq!(
            parse(&words("uninstall --mode system")),
            Ok(Form::Uninstall(Some(ServiceMode::System)))
        );
        assert_eq!(parse(&words("uninstall")), Ok(Form::Uninstall(None)));
    }

    #[test]
    fn invalid_modes_and_forms_are_usage_errors() {
        for bad in [
            "",
            "start",
            "Install",
            "install --mode",
            "install --mode root",
            "install --mode user --mode system",
            "install user",
            "uninstall --force",
        ] {
            assert!(
                matches!(parse(&words(bad)), Err(CliError::Usage(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_result_is_printed_as_key_value_pairs() {
        assert_eq!(render(&json!({"mode": "user"})).unwrap(), "mode: user");
        assert_eq!(render(&json!({"started": true})).unwrap(), "started: true");
        assert_eq!(render(&json!({"pid": 42})).unwrap(), "pid: 42");
    }

    #[test]
    fn empty_members_are_left_out_of_the_result() {
        let report = json!({"linger": false, "notes": [], "reason": null, "modules": {}});
        assert_eq!(render(&report).unwrap(), "linger: false");
        assert_eq!(render(&json!({})).unwrap(), "");
    }

    #[test]
    fn lists_and_maps_are_written_in_brackets_and_braces() {
        assert_eq!(
            render(&json!({"files": ["a", "b"]})).unwrap(),
            "files: [a, b]"
        );
        assert_eq!(
            render(&json!({"modules": {"schedule": "running"}})).unwrap(),
            "modules: {schedule: running}"
        );
        assert_eq!(
            render(&json!({"nested": [{"a": 1}, [true, null]]})).unwrap(),
            "nested: [{a: 1}, [true, null]]"
        );
    }

    #[test]
    fn several_members_are_separated_by_two_spaces() {
        let text = render(&json!({"mode": "user", "started": true})).unwrap();
        assert!(
            text == "mode: user  started: true" || text == "started: true  mode: user",
            "{text}"
        );
    }

    #[test]
    fn text_values_with_multi_byte_characters_are_not_quoted() {
        assert_eq!(
            render(&json!({"file": "\u{142}\u{f3}d\u{17a} \u{1f600}"})).unwrap(),
            "file: \u{142}\u{f3}d\u{17a} \u{1f600}"
        );
    }

    #[test]
    fn a_result_that_is_not_an_object_is_a_failure() {
        for value in [json!(3), json!([1]), json!("x"), json!(null)] {
            assert_eq!(
                render(&value),
                Err(CliError::Failure("the result is not an object".to_owned()))
            );
        }
    }

    #[test]
    fn emptiness_covers_null_and_empty_containers_only() {
        for empty in [json!(null), json!([]), json!({})] {
            assert!(is_empty(&empty));
        }
        for filled in [json!(false), json!(0), json!(""), json!([null]), json!({"a": 1})] {
            assert!(!is_empty(&filled), "{filled}");
        }
    }
}
