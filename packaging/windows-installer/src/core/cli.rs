// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/cli.rs
// Responsibility:   Command line of the setup program: parsing of the arguments of the silent mode and of the internal switches.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Command line of the setup program.
//!
//! The options follow the table of section 11 of the design and the variables of `install.ps1`.
//! Both forms `--name value` and `--name=value` are accepted; `/S` and `/D=<dir>` are accepted
//! for compatibility with other setup programs.

use std::path::PathBuf;

use crate::model::{Options, Scope};

/// Parsed command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Cli {
    /// Silent mode: no window, no splash.
    pub(crate) silent: bool,
    /// Undoes the setup instead of installing.
    pub(crate) undo: bool,
    /// Installs the same version again over an installation (Repair).
    pub(crate) repair: bool,
    /// Scope of the installation.
    pub(crate) scope: Option<Scope>,
    /// Folder of `nurt.exe`.
    pub(crate) dir: Option<String>,
    /// Leaves `PATH` unchanged.
    pub(crate) no_path: bool,
    /// Leaves the documentation out.
    pub(crate) no_docs: bool,
    /// No Start menu shortcuts.
    pub(crate) no_start_menu: bool,
    /// Shortcut on the desktop.
    pub(crate) desktop: bool,
    /// Shortcut in the Quick Launch folder.
    pub(crate) quick_launch: bool,
    /// Profile of Windows Terminal.
    pub(crate) terminal_profile: bool,
    /// Makes the profile the default profile of Windows Terminal.
    pub(crate) terminal_default: bool,
    /// Tab completion in PowerShell.
    pub(crate) completions: bool,
    /// No first configuration.
    pub(crate) no_config: bool,
    /// Removes the configuration of the user when undoing.
    pub(crate) remove_config: bool,
    /// Version that the user expects; it must be the version of the setup.
    pub(crate) version: Option<String>,
    /// Opens the wizard without the splash.
    pub(crate) no_animation: bool,
    /// Log file.
    pub(crate) log: Option<PathBuf>,
    /// The process was started elevated by the wizard.
    pub(crate) elevated: bool,
    /// File with the state of the wizard that elevated itself.
    pub(crate) resume: Option<PathBuf>,
    /// Job file of the elevated worker.
    pub(crate) worker: Option<PathBuf>,
    /// Directory of the copy of the uninstaller that runs from the temporary directory.
    pub(crate) from_temp: Option<PathBuf>,
    /// Prints the usage.
    pub(crate) help: bool,
}

/// Usage text.
pub(crate) const USAGE: &str = "Danaco Nurt setup\n\
\n\
Usage: setup [/S] [options]\n\
\n\
  /S, --silent              no window, no splash; the exit code reports the result\n\
  --scope user|machine      for the current user (default) or for all users\n\
  --dir <path>, /D=<path>   folder of nurt.exe\n\
  --no-path                 leave PATH unchanged\n\
  --no-docs                 do not install the documentation\n\
  --no-start-menu           no Start menu shortcuts\n\
  --desktop                 shortcut on the desktop\n\
  --quick-launch            shortcut in the Quick Launch folder\n\
  --terminal-profile        add the Windows Terminal profile\n\
  --terminal-default        make the profile the default of Windows Terminal\n\
  --completions             tab completion in PowerShell\n\
  --no-config               do not write nurt.toml\n\
  --version <X.Y.Z>         version to install; the setup carries exactly one\n\
  --no-animation            open the wizard without the splash\n\
  --repair                  install the same version again over an installation\n\
  --undo                    reverse the journal of the setup\n\
  --remove-config           remove nurt.toml too when undoing\n\
  --log <file>              log file\n\
  -h, --help                this text\n\
\n\
Exit codes: 0 done, 1 failed and rolled back, 2 invalid arguments,\n\
3 requirements not met, 4 administrator approval declined.\n";

/// Parses the arguments after the program name.
pub(crate) fn parse(arguments: &[String]) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        index += 1;
        let (name, inline) = split_value(argument);
        let value = |index: &mut usize| -> Result<String, String> {
            if let Some(inline) = &inline {
                return Ok(inline.clone());
            }
            let next = arguments
                .get(*index)
                .ok_or_else(|| format!("{name} needs a value"))?;
            *index += 1;
            Ok(next.clone())
        };
        match name.to_ascii_lowercase().as_str() {
            "/s" | "--silent" | "--quiet" => cli.silent = true,
            "/d" => cli.dir = Some(inline.clone().ok_or("/D needs a value: /D=<path>")?),
            "--dir" => cli.dir = Some(value(&mut index)?),
            "--scope" => {
                cli.scope = Some(match value(&mut index)?.to_ascii_lowercase().as_str() {
                    "user" => Scope::User,
                    "machine" => Scope::Machine,
                    other => return Err(format!("invalid scope: {other}; use user or machine")),
                });
            }
            "--no-path" => cli.no_path = true,
            "--no-docs" => cli.no_docs = true,
            "--no-start-menu" => cli.no_start_menu = true,
            "--desktop" => cli.desktop = true,
            "--quick-launch" => cli.quick_launch = true,
            "--terminal-profile" => cli.terminal_profile = true,
            "--terminal-default" => cli.terminal_default = true,
            "--completions" => cli.completions = true,
            "--no-config" => cli.no_config = true,
            "--remove-config" => cli.remove_config = true,
            "--version" => {
                let version = value(&mut index)?;
                if !is_version(&version) {
                    return Err(format!("invalid version: {version}; use MAJOR.MINOR.PATCH"));
                }
                cli.version = Some(version);
            }
            "--no-animation" | "--noanim" => cli.no_animation = true,
            "--repair" => cli.repair = true,
            "--undo" | "--uninstall" | "/uninstall" => cli.undo = true,
            "--log" => cli.log = Some(PathBuf::from(value(&mut index)?)),
            "--elevated" => cli.elevated = true,
            "--resume" => cli.resume = Some(PathBuf::from(value(&mut index)?)),
            "--worker" => cli.worker = Some(PathBuf::from(value(&mut index)?)),
            "--from-temp" => cli.from_temp = Some(PathBuf::from(value(&mut index)?)),
            "-h" | "--help" | "/?" => cli.help = true,
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }
    Ok(cli)
}

/// `MAJOR.MINOR.PATCH` with an optional pre-release or build suffix.
pub(crate) fn is_version(text: &str) -> bool {
    let core = text.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
        && text[core.len()..].chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '+' | '.')
        })
}

/// The options that the command line chooses, starting from the defaults of the wizard.
pub(crate) fn options_of(cli: &Cli, base: Options) -> Options {
    Options {
        scope: cli.scope.unwrap_or(base.scope),
        dir: cli.dir.clone().unwrap_or(base.dir),
        path: !cli.no_path && base.path,
        docs: !cli.no_docs && base.docs,
        start_menu: !cli.no_start_menu && base.start_menu,
        desktop: cli.desktop || base.desktop,
        quick_launch: cli.quick_launch || base.quick_launch,
        terminal_profile: cli.terminal_profile || cli.terminal_default || base.terminal_profile,
        terminal_default: cli.terminal_default || base.terminal_default,
        completions: cli.completions || base.completions,
        write_config: !cli.no_config && base.write_config,
        config: base.config,
    }
}

/// Splits `--name=value` and `/D=value` at the first equals sign.
fn split_value(argument: &str) -> (String, Option<String>) {
    if (argument.starts_with("--") || argument.starts_with('/'))
        && let Some((name, value)) = argument.split_once('=')
    {
        return (name.to_string(), Some(value.trim_matches('"').to_string()));
    }
    (argument.to_string(), None)
}

/// Quotes an argument for a command line of Windows.
pub(crate) fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn silent_with_options() {
        let cli = parse(&args(&[
            "/S",
            "--scope",
            "machine",
            "--dir=C:\\Program Files\\Nurt\\bin",
            "--no-path",
            "--desktop",
        ]))
        .unwrap_or_default();
        assert!(cli.silent && cli.no_path && cli.desktop);
        assert_eq!(cli.scope, Some(Scope::Machine));
        assert_eq!(cli.dir.as_deref(), Some("C:\\Program Files\\Nurt\\bin"));
    }

    #[test]
    fn nsis_style_directory_and_undo() {
        let cli = parse(&args(&["/s", "/D=C:\\Tools\\Nurt\\bin", "--undo"])).unwrap_or_default();
        assert!(cli.silent && cli.undo);
        assert_eq!(cli.dir.as_deref(), Some("C:\\Tools\\Nurt\\bin"));
    }

    #[test]
    fn rejects_unknown_and_incomplete() {
        assert!(parse(&args(&["--bogus"])).is_err());
        assert!(parse(&args(&["--scope"])).is_err());
        assert!(parse(&args(&["--scope", "everyone"])).is_err());
        assert!(parse(&args(&["--version", "1.2"])).is_err());
    }

    #[test]
    fn versions_have_three_numbers() {
        assert!(is_version("0.1.2") && is_version("1.2.3-rc.1") && is_version("1.2.3+build.5"));
        assert!(!is_version("1.2") && !is_version("a.b.c") && !is_version("1.2.3."));
    }

    #[test]
    fn quoting_follows_the_rules_of_windows() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("C:\\a b\\"), "\"C:\\a b\\\\\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote(""), "\"\"");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
