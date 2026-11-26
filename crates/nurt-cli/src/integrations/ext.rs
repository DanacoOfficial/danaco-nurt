// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `ext` subcommand: installation, removal and listing of extensions.
//!
//! The forms are `ext list [--format text|json]`, `ext install <directory>
//! [--replace]` and `ext remove <name>`. The extension directory is the key
//! `ext.dir` of the configuration, by default `ext` in the configuration
//! directory of the user.

use std::io::Write;
use std::path::{Path, PathBuf};

use nurt_core::config::{ConfigLoader, ConfigSources};
use nurt_core::paths::PlatformPaths;
use nurt_core::registry::standard_registry;
use nurt_core::system::SystemHost;
use nurt_ext::locations::ExtLocations;
use nurt_ext::register::core_names;
use nurt_ext::store::{ExtListing, ExtStore};

use crate::app::CliError;

/// Output format of `ext list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    /// One line per extension.
    Text,
    /// A JSON array of the listings.
    Json,
}

/// A parsed form of `ext`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Lists the installed extensions.
    List(ListFormat),
    /// Installs the extension in a source directory.
    Install {
        /// Directory holding the manifest of the extension.
        source: PathBuf,
        /// Whether an installed version is replaced.
        replace: bool,
    },
    /// Removes an installed extension.
    Remove(String),
}

const USAGE: &str = "use `ext list`, `ext install <directory>` or `ext remove <name>`";

/// Parses the arguments of `ext`.
///
/// # Errors
///
/// Returns a usage error for a missing or unknown operation, a missing or
/// repeated operand and an unknown option.
pub fn parse(args: &[String]) -> Result<Action, CliError> {
    let Some((operation, rest)) = args.split_first() else {
        return Err(CliError::Usage(format!(
            "`ext` needs an operation; {USAGE}"
        )));
    };
    match operation.as_str() {
        "list" => parse_list(rest),
        "install" => parse_install(rest),
        "remove" => match rest {
            [name] if !name.starts_with('-') => Ok(Action::Remove(name.clone())),
            _ => Err(CliError::Usage(
                "`ext remove` takes one extension name".to_owned(),
            )),
        },
        other => Err(CliError::Usage(format!(
            "unknown operation `{other}` of `ext`; {USAGE}"
        ))),
    }
}

fn parse_list(args: &[String]) -> Result<Action, CliError> {
    let mut format = None;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word != "--format" {
            return Err(CliError::Usage(format!(
                "unknown argument `{word}` of `ext list`"
            )));
        }
        let name = words
            .next()
            .ok_or_else(|| CliError::Usage("`--format` needs a value".to_owned()))?;
        let chosen = match name.as_str() {
            "text" => ListFormat::Text,
            "json" => ListFormat::Json,
            other => {
                return Err(CliError::Usage(format!(
                    "unknown format `{other}`; use `text` or `json`"
                )));
            }
        };
        if format.replace(chosen).is_some() {
            return Err(CliError::Usage(
                "`ext list` takes `--format` only once".to_owned(),
            ));
        }
    }
    Ok(Action::List(format.unwrap_or(ListFormat::Text)))
}

fn parse_install(args: &[String]) -> Result<Action, CliError> {
    let mut source = None;
    let mut replace = false;
    for word in args {
        if word == "--replace" {
            if replace {
                return Err(CliError::Usage(
                    "`ext install` takes `--replace` only once".to_owned(),
                ));
            }
            replace = true;
        } else if word.starts_with('-') {
            return Err(CliError::Usage(format!(
                "unknown argument `{word}` of `ext install`"
            )));
        } else if source.replace(PathBuf::from(word)).is_some() {
            return Err(CliError::Usage(
                "`ext install` takes one source directory".to_owned(),
            ));
        }
    }
    let source = source
        .ok_or_else(|| CliError::Usage("`ext install` needs a source directory".to_owned()))?;
    Ok(Action::Install { source, replace })
}

fn locations() -> Result<ExtLocations, CliError> {
    let host = SystemHost::detect();
    let config_dir = PlatformPaths::for_host(&host)
        .user_file
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            CliError::Failure(
                "the account has no home directory; set `NURT_EXT_DIR` to an absolute path"
                    .to_owned(),
            )
        })?;
    let loaded = ConfigLoader::new(&ConfigSources::new(), &host)?.load(None)?;
    Ok(ExtLocations::from_config(
        &loaded.config().ext,
        &config_dir,
    )?)
}

fn store() -> Result<ExtStore, CliError> {
    let registry = standard_registry()?;
    Ok(ExtStore::new(locations()?, core_names(&registry)))
}

fn write_listing(out: &mut dyn Write, listing: &ExtListing) -> std::io::Result<()> {
    match &listing.problem {
        Some(problem) => writeln!(out, "{}\tunusable\t{problem}", listing.name),
        None => writeln!(
            out,
            "{}\t{}\t{}\t{}",
            listing.name,
            listing.version.as_deref().unwrap_or_default(),
            listing.api.as_deref().unwrap_or_default(),
            listing.commands.join(" ")
        ),
    }
}

/// Executes a parsed form of `ext` and writes its result to `out`.
///
/// # Errors
///
/// Returns the error of the extension store, the error of the configuration
/// or a failure when the output cannot be written.
pub fn execute(action: &Action, out: &mut dyn Write) -> Result<u8, CliError> {
    let store = store()?;
    match action {
        Action::List(format) => {
            let listings = store.list()?;
            match format {
                ListFormat::Json => {
                    let text = serde_json::to_string_pretty(&listings).map_err(|error| {
                        CliError::Failure(format!("cannot render the listing: {error}"))
                    })?;
                    writeln!(out, "{text}")?;
                }
                ListFormat::Text => {
                    for listing in &listings {
                        write_listing(out, listing)?;
                    }
                }
            }
        }
        Action::Install { source, replace } => {
            let installed = store.install(source, *replace)?;
            writeln!(
                out,
                "installed {} {} in {}",
                installed.name(),
                installed.manifest.ext.version,
                installed.dir.display()
            )?;
        }
        Action::Remove(name) => {
            let dir = store.remove(name)?;
            writeln!(out, "removed {name} from {}", dir.display())?;
        }
    }
    Ok(0)
}

/// Runs `nurt ext` and writes its result to the standard output.
///
/// # Errors
///
/// Returns a usage error for invalid arguments, and the error of the
/// extension store, of the configuration or of the output otherwise.
pub fn run(args: &[String]) -> Result<u8, CliError> {
    let action = parse(args)?;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    execute(&action, &mut out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn is_usage(result: Result<Action, CliError>) -> bool {
        matches!(result, Err(CliError::Usage(_)))
    }

    #[test]
    fn list_defaults_to_text_and_takes_one_format() {
        assert_eq!(parse(&words("list")), Ok(Action::List(ListFormat::Text)));
        assert_eq!(
            parse(&words("list --format json")),
            Ok(Action::List(ListFormat::Json))
        );
        assert_eq!(
            parse(&words("list --format text")),
            Ok(Action::List(ListFormat::Text))
        );
    }

    #[test]
    fn list_rejects_other_arguments_and_repeated_formats() {
        for bad in [
            "list --format",
            "list --format yaml",
            "list --format json --format text",
            "list --json",
            "list extra",
        ] {
            assert!(is_usage(parse(&words(bad))), "{bad}");
        }
    }

    #[test]
    fn install_takes_one_directory_and_an_optional_replace() {
        assert_eq!(
            parse(&words("install ./pack")),
            Ok(Action::Install {
                source: PathBuf::from("./pack"),
                replace: false
            })
        );
        assert_eq!(
            parse(&words("install --replace ./pack")),
            Ok(Action::Install {
                source: PathBuf::from("./pack"),
                replace: true
            })
        );
        assert_eq!(
            parse(&words("install ./pack --replace")),
            parse(&words("install --replace ./pack"))
        );
    }

    #[test]
    fn install_rejects_a_missing_second_or_unknown_operand() {
        for bad in [
            "install",
            "install --replace",
            "install a b",
            "install --replace --replace a",
            "install --force a",
        ] {
            assert!(is_usage(parse(&words(bad))), "{bad}");
        }
    }

    #[test]
    fn install_keeps_a_directory_with_multi_byte_characters() {
        let args = vec![
            "install".to_owned(),
            "r\u{f3}\u{17c}ne/\u{1f600}".to_owned(),
        ];
        assert_eq!(
            parse(&args),
            Ok(Action::Install {
                source: PathBuf::from("r\u{f3}\u{17c}ne/\u{1f600}"),
                replace: false
            })
        );
    }

    #[test]
    fn remove_takes_exactly_one_name_that_is_not_an_option() {
        assert_eq!(
            parse(&words("remove tools")),
            Ok(Action::Remove("tools".to_owned()))
        );
        for bad in ["remove", "remove a b", "remove --all"] {
            assert!(is_usage(parse(&words(bad))), "{bad}");
        }
    }

    #[test]
    fn an_operation_is_required_and_must_be_known() {
        assert!(is_usage(parse(&[])));
        assert!(is_usage(parse(&words("update"))));
        assert!(is_usage(parse(&words("List"))));
    }

    #[test]
    fn run_rejects_invalid_arguments_before_it_touches_the_store() {
        assert!(matches!(run(&[]), Err(CliError::Usage(_))));
        assert!(matches!(run(&words("remove")), Err(CliError::Usage(_))));
    }
}
