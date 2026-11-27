// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::spawn::windows_cmd
// Responsibility:   Command lines of batch files started through the Windows command.
// Dependencies:     serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Command lines of batch files started through the Windows command
//! interpreter.
//!
//! `cmd.exe` parses the command line of a batch file itself and expands
//! variables and metacharacters before the script sees its arguments, so the
//! quoting rules of `CreateProcessW` do not protect them (CVE-2024-24576). The
//! command line is built as the Rust standard library builds it since 1.77.2:
//!
//! * the interpreter runs with command extensions on, delayed expansion off
//!   (`!` stays literal), without AutoRun commands, and the whole command is
//!   enclosed in one more pair of quotes;
//! * an argument is quoted when it is empty, ends with `\`, or contains an
//!   ASCII character other than letters, digits and `#$*+-./:?@\_`, or a
//!   control character; inside quotes `& | < > ^ ( )` are literal;
//! * `"` is doubled, with the backslashes before it doubled as well;
//! * `%` is written as `%%cd:~,%`, an empty substring of the variable `cd`,
//!   which stops the expansion of `%NAME%`;
//! * an argument with a line break cannot be passed, because the interpreter
//!   ends the command at the line break.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// Interpreter and switches, followed by the opening quote of the whole
/// command.
const INTERPRETER_PREFIX: &str = "cmd.exe /e:ON /v:OFF /d /c \"";
/// Text written before every `%` of an argument.
const PERCENT_GUARD: &str = "%%cd:~,";
/// ASCII characters other than letters and digits that never need quotes.
const UNQUOTED: &str = r"#$*+-./:?@\_";
/// Characters the command interpreter treats specially outside quotes.
const METACHARACTERS: &[char] = &['&', '|', '<', '>', '^', '(', ')', '!', '"', '%'];
/// Windows directory used when `SystemRoot` is not set.
const DEFAULT_SYSTEM_ROOT: &str = r"C:\Windows";
/// Location of the interpreter below the Windows directory.
const INTERPRETER_PATH: &str = r"\System32\cmd.exe";

/// Argument that cannot be passed to a batch file without changing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArgProblem {
    /// Position in the command; 0 is the script.
    pub(crate) index: usize,
    /// Short name of the problem.
    pub(crate) problem: &'static str,
    /// What is wrong, as the end of a sentence.
    pub(crate) reason: &'static str,
}

/// The command interpreter in the Windows directory `system_root`, never
/// looked up in `PATH`.
pub(crate) fn interpreter(system_root: Option<&OsStr>) -> PathBuf {
    let root = system_root
        .map(|root| root.to_string_lossy().trim_end_matches('\\').to_owned())
        .filter(|root| !root.is_empty())
        .unwrap_or_else(|| DEFAULT_SYSTEM_ROOT.to_owned());
    let mut path = OsString::from(root);
    path.push(INTERPRETER_PATH);
    PathBuf::from(path)
}

/// Complete command line that runs `script` with `args`.
///
/// # Errors
///
/// Fails when the script path contains `"` or ends with `\`, or when an
/// argument contains a NUL character or a line break.
pub(crate) fn command_line(script: &str, args: &[String]) -> Result<String, ArgProblem> {
    if script.contains('"') || script.ends_with('\\') {
        return Err(ArgProblem {
            index: 0,
            problem: "unquotable_script_path",
            reason: "is a batch file path with `\"` or a final `\\`, which cannot be quoted",
        });
    }
    let mut line = String::from(INTERPRETER_PREFIX);
    line.push('"');
    line.push_str(&script.replace('%', &format!("{PERCENT_GUARD}%")));
    line.push('"');
    for (position, arg) in args.iter().enumerate() {
        line.push(' ');
        append_argument(&mut line, arg, position + 1)?;
    }
    line.push('"');
    Ok(line)
}

/// Appends one quoted argument to `line`.
///
/// # Errors
///
/// Fails when the argument contains a NUL character or a line break.
pub(crate) fn append_argument(
    line: &mut String,
    arg: &str,
    index: usize,
) -> Result<(), ArgProblem> {
    if arg.contains('\0') {
        return Err(ArgProblem {
            index,
            problem: "nul_in_argument",
            reason: "contains a NUL character, which no program can receive",
        });
    }
    if arg.contains(['\r', '\n']) {
        return Err(ArgProblem {
            index,
            problem: "line_break_in_batch_argument",
            reason: "contains a line break, which ends the command of a batch file",
        });
    }
    let quote = arg.is_empty() || arg.ends_with('\\') || arg.chars().any(needs_quotes);
    if quote {
        line.push('"');
    }
    let mut backslashes = 0;
    for character in arg.chars() {
        if character == '\\' {
            backslashes += 1;
        } else {
            if character == '"' {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push('"');
            } else if character == '%' {
                line.push_str(PERCENT_GUARD);
            }
            backslashes = 0;
        }
        line.push(character);
    }
    if quote {
        line.extend(std::iter::repeat_n('\\', backslashes));
        line.push('"');
    }
    Ok(())
}

fn needs_quotes(character: char) -> bool {
    (character.is_ascii() && !(character.is_ascii_alphanumeric() || UNQUOTED.contains(character)))
        || character.is_control()
}

/// Positions in `command` of the arguments that contain metacharacters of
/// the command interpreter; the program itself is not counted.
pub(crate) fn metacharacter_arguments(command: &[String]) -> Vec<usize> {
    command
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, arg)| arg.contains(METACHARACTERS))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{ArgProblem, append_argument, command_line, interpreter, metacharacter_arguments};

    /// Arguments that attempt command injection through a batch file, with
    /// the expected quoted form, or the expected problem when the argument
    /// cannot be passed.
    const CORPUS: &str = include_str!("../../tests/data/spawn/bat-injection.json");

    #[derive(Deserialize)]
    struct Case {
        arg: String,
        #[serde(default)]
        quoted: Option<String>,
        #[serde(default)]
        problem: Option<String>,
    }

    fn corpus() -> Vec<Case> {
        serde_json::from_str(CORPUS).unwrap_or_default()
    }

    #[test]
    fn corpus_has_at_least_sixty_cases() {
        assert!(corpus().len() >= 60);
    }

    #[test]
    fn quotes_every_corpus_argument_as_expected() {
        for case in corpus() {
            let mut line = String::new();
            let result = append_argument(&mut line, &case.arg, 1);
            match (&case.quoted, &case.problem) {
                (Some(quoted), None) => {
                    assert_eq!(result, Ok(()), "argument {:?}", case.arg);
                    assert_eq!(&line, quoted, "argument {:?}", case.arg);
                }
                (None, Some(problem)) => {
                    let error = result.err();
                    assert_eq!(
                        error.map(|error| error.problem),
                        Some(problem.as_str()),
                        "argument {:?}",
                        case.arg
                    );
                }
                _ => panic!("corpus case {:?} needs `quoted` or `problem`", case.arg),
            }
        }
    }

    #[test]
    fn builds_the_whole_command_line() {
        let line = command_line(
            r"C:\Tools\build.cmd",
            &["release".to_owned(), "a b".to_owned(), "x&y".to_owned()],
        );
        assert_eq!(
            line,
            Ok(
                r#"cmd.exe /e:ON /v:OFF /d /c ""C:\Tools\build.cmd" release "a b" "x&y"""#
                    .to_owned()
            )
        );
    }

    #[test]
    fn a_percent_sign_in_the_script_path_is_not_expanded() {
        assert_eq!(
            command_line(r"C:\100%\run.bat", &[]),
            Ok(r#"cmd.exe /e:ON /v:OFF /d /c ""C:\100%%cd:~,%\run.bat"""#.to_owned())
        );
    }

    #[test]
    fn rejects_script_paths_that_cannot_be_quoted() {
        for script in [r#"C:\a"b.bat"#, r"C:\dir\"] {
            assert_eq!(
                command_line(script, &[]).map_err(|problem| problem.index),
                Err(0)
            );
        }
    }

    #[test]
    fn reports_the_position_of_an_argument_with_a_line_break() {
        let result = command_line(r"C:\a.bat", &["ok".to_owned(), "two\nlines".to_owned()]);
        assert_eq!(
            result.map_err(|problem: ArgProblem| (problem.index, problem.problem)),
            Err((2, "line_break_in_batch_argument"))
        );
    }

    #[test]
    fn takes_the_interpreter_from_the_windows_directory() {
        assert_eq!(
            interpreter(Some(std::ffi::OsStr::new(r"D:\Win\"))).to_string_lossy(),
            r"D:\Win\System32\cmd.exe"
        );
        assert_eq!(
            interpreter(None).to_string_lossy(),
            r"C:\Windows\System32\cmd.exe"
        );
    }

    #[test]
    fn lists_arguments_with_metacharacters() {
        let command: Vec<String> = ["run.bat", "plain", "a&b", "100%", "(x)"]
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect();
        assert_eq!(metacharacter_arguments(&command), vec![2, 3, 4]);
    }

    #[cfg(windows)]
    #[test]
    fn batch_file_receives_every_corpus_argument_unchanged() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let fixture = crate::spawn::testing::fixture_path();
        let script = dir.path().join("forward.bat");
        std::fs::write(
            &script,
            format!("@echo off\r\n\"{}\" print-argv %*\r\n", fixture.display()),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        for case in corpus().into_iter().filter(|case| case.quoted.is_some()) {
            let script_text = script.to_string_lossy();
            let line = command_line(&script_text, std::slice::from_ref(&case.arg))
                .unwrap_or_else(|problem| panic!("{problem:?}"));
            let tail = line.strip_prefix("cmd.exe ").unwrap_or(&line);
            let output = Command::new(interpreter(std::env::var_os("SystemRoot").as_deref()))
                .raw_arg(tail)
                .output()
                .unwrap_or_else(|error| panic!("{error}"));
            let received: Vec<String> = serde_json::from_slice(&output.stdout).unwrap_or_default();
            assert_eq!(received, vec![case.arg.clone()], "argument {:?}", case.arg);
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
