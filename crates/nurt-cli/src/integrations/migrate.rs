// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::integrations::migrate
// Responsibility:   The `migrate` subcommand: translation of a shell script into a program.
// Dependencies:     nurt_bind, nurt_catalog, nurt_migrate
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `migrate` subcommand: translation of a shell script into a program.
//!
//! The command line takes one script file and the options
//! `--format text|json`, `--output <file>`, `--script <path>` and
//! `--language bash|powershell`. The translated program is written to the
//! standard output, or to the file of `--output`. The migration report is
//! written to the standard error output, or to the standard output when the
//! program goes to a file. The exit code is 0 when every statement is
//! translated and 1 when at least one statement is not.

use std::io::Write;
use std::path::{Path, PathBuf};

use nurt_migrate::{
    Catalog, MigrateOptions, MigrationReport, ScriptLanguage, TableTranslator, migrate_with,
};

use crate::app::{CliError, Io};
use crate::exit;

/// Form of the migration report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReportFormat {
    /// Text for a person.
    #[default]
    Text,
    /// JSON document of the format `nurt-migrate-report/1`.
    Json,
}

impl ReportFormat {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Arguments of `migrate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    /// Script to translate.
    pub file: PathBuf,
    /// Form of the report.
    pub format: ReportFormat,
    /// File that receives the translated program; the standard output when
    /// absent.
    pub output: Option<PathBuf>,
    /// Script path written into the fallback program; the absolute path of
    /// the script when absent.
    pub script: Option<String>,
    /// Language of the script; detected from the file when absent.
    pub language: Option<ScriptLanguage>,
}

/// Parses the arguments of `migrate`.
///
/// # Errors
///
/// Returns a usage error unless the arguments contain exactly one script
/// file and options of the forms above, each at most once.
pub fn parse(args: &[String]) -> Result<Arguments, CliError> {
    let mut file = None;
    let mut format = None;
    let mut output = None;
    let mut script = None;
    let mut language = None;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--format" => {
                let name = value(&mut words, "--format")?;
                let chosen = ReportFormat::from_name(name).ok_or_else(|| {
                    CliError::Usage(format!("unknown format `{name}`; use `text` or `json`"))
                })?;
                set(&mut format, chosen, "--format")?;
            }
            "--output" => {
                let path = value(&mut words, "--output")?;
                set(&mut output, PathBuf::from(path), "--output")?;
            }
            "--script" => {
                let path = value(&mut words, "--script")?;
                set(&mut script, path.to_owned(), "--script")?;
            }
            "--language" => {
                let name = value(&mut words, "--language")?;
                let chosen = match name {
                    "bash" => ScriptLanguage::Bash,
                    "powershell" => ScriptLanguage::PowerShell,
                    other => {
                        return Err(CliError::Usage(format!(
                            "unknown language `{other}`; use `bash` or `powershell`"
                        )));
                    }
                };
                set(&mut language, chosen, "--language")?;
            }
            option if option.starts_with("--") => {
                return Err(CliError::Usage(format!(
                    "unknown option `{option}` of `migrate`"
                )));
            }
            path => set(&mut file, PathBuf::from(path), "the script file")?,
        }
    }
    let file = file.ok_or_else(|| {
        CliError::Usage("`migrate` needs a script file: `migrate <file.sh | file.ps1>`".to_owned())
    })?;
    Ok(Arguments {
        file,
        format: format.unwrap_or_default(),
        output,
        script,
        language,
    })
}

fn set<T>(slot: &mut Option<T>, value: T, what: &str) -> Result<(), CliError> {
    if slot.replace(value).is_some() {
        return Err(CliError::Usage(format!("`migrate` takes {what} only once")));
    }
    Ok(())
}

fn value<'a>(words: &mut std::slice::Iter<'a, String>, option: &str) -> Result<&'a str, CliError> {
    words
        .next()
        .map(String::as_str)
        .ok_or_else(|| CliError::Usage(format!("`{option}` needs a value")))
}

fn translator() -> Result<TableTranslator, CliError> {
    let (index, files) = nurt_catalog::embedded_sources();
    let texts: Vec<&str> = files.iter().map(|(_, text)| *text).collect();
    let catalog = Catalog::from_toml(index, &texts)
        .map_err(|error| CliError::Failure(format!("cannot load the embedded catalog: {error}")))?;
    Ok(TableTranslator::new().with_catalog(catalog, nurt_bind::host_os())?)
}

fn script_path(arguments: &Arguments) -> Result<String, CliError> {
    if let Some(script) = &arguments.script {
        return Ok(script.clone());
    }
    let absolute = std::path::absolute(&arguments.file).map_err(|error| {
        CliError::Failure(format!(
            "cannot resolve `{}`: {error}",
            arguments.file.display()
        ))
    })?;
    Ok(absolute.to_string_lossy().into_owned())
}

fn read_script(path: &Path) -> Result<String, CliError> {
    let bytes = std::fs::read(path).map_err(|error| unreadable(path, &error.to_string()))?;
    crate::text::decode_file(&bytes).map_err(|reason| unreadable(path, &reason))
}

fn unreadable(path: &Path, reason: &str) -> CliError {
    CliError::Failure(format!("cannot read `{}`: {reason}", path.display()))
}

fn render(report: &MigrationReport, format: ReportFormat) -> String {
    let mut text = match format {
        ReportFormat::Text => report.to_text(),
        ReportFormat::Json => report.to_json(),
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Translates the script named by the arguments and writes the program and
/// the report.
///
/// # Errors
///
/// Returns a usage error for invalid arguments, a failure when the script or
/// the output file cannot be accessed or the output stream cannot be written,
/// and the error of the translation.
pub fn execute(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<u8, CliError> {
    let arguments = parse(args)?;
    let source = read_script(&arguments.file)?;
    let language = match arguments.language {
        Some(language) => language,
        None => ScriptLanguage::detect(&arguments.file.to_string_lossy(), &source)?,
    };
    let options = MigrateOptions::new().with_script(script_path(&arguments)?);
    let migration = migrate_with(&source, language, &options, &translator()?)?;
    let report = render(&migration.report, arguments.format);
    match &arguments.output {
        Some(path) => {
            std::fs::write(path, &migration.nurt).map_err(|error| {
                CliError::Failure(format!("cannot write `{}`: {error}", path.display()))
            })?;
            out.write_all(report.as_bytes())?;
        }
        None => {
            out.write_all(migration.nurt.as_bytes())?;
            err.write_all(report.as_bytes())?;
        }
    }
    let summary = migration.report.summary();
    if summary.impossible + summary.error == 0 {
        Ok(exit::SUCCESS)
    } else {
        Ok(exit::FAILURE)
    }
}

/// Runs `nurt migrate` on the streams of the command line.
///
/// # Errors
///
/// Returns the errors of [`execute`].
pub fn start(args: &[String], io: &mut Io<'_>) -> Result<u8, CliError> {
    execute(args, io.out, io.err)
}

/// Runs `nurt migrate` on the standard streams of the process.
///
/// # Errors
///
/// Returns the errors of [`execute`].
pub fn run(args: &[String]) -> Result<u8, CliError> {
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    execute(args, &mut out, &mut err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn usage(text: &str) -> bool {
        matches!(parse(&words(text)), Err(CliError::Usage(_)))
    }

    #[test]
    fn a_script_file_alone_gives_the_defaults() {
        assert_eq!(
            parse(&words("deploy.sh")).unwrap(),
            Arguments {
                file: PathBuf::from("deploy.sh"),
                format: ReportFormat::Text,
                output: None,
                script: None,
                language: None,
            }
        );
    }

    #[test]
    fn every_option_is_read_in_any_order() {
        let arguments = parse(&words(
            "--language powershell --output out.nurt a.ps1 --script /opt/a.ps1 --format json",
        ))
        .unwrap();
        assert_eq!(arguments.file, PathBuf::from("a.ps1"));
        assert_eq!(arguments.format, ReportFormat::Json);
        assert_eq!(arguments.output, Some(PathBuf::from("out.nurt")));
        assert_eq!(arguments.script.as_deref(), Some("/opt/a.ps1"));
        assert_eq!(arguments.language, Some(ScriptLanguage::PowerShell));
        let bash = parse(&words("a.sh --language bash")).unwrap();
        assert_eq!(bash.language, Some(ScriptLanguage::Bash));
    }

    #[test]
    fn a_file_name_with_multi_byte_characters_is_kept() {
        let args = vec!["migrate-\u{142}\u{f3}d\u{17a}.sh".to_owned()];
        assert_eq!(
            parse(&args).unwrap().file,
            PathBuf::from("migrate-\u{142}\u{f3}d\u{17a}.sh")
        );
    }

    #[test]
    fn options_given_twice_are_rejected() {
        for bad in [
            "a.sh --format text --format json",
            "a.sh --output x --output y",
            "a.sh --script x --script y",
            "a.sh --language bash --language powershell",
            "a.sh b.sh",
        ] {
            assert!(usage(bad), "{bad}");
        }
    }

    #[test]
    fn missing_files_values_and_unknown_names_are_rejected() {
        for bad in [
            "",
            "--format json",
            "a.sh --format",
            "a.sh --output",
            "a.sh --script",
            "a.sh --language",
            "a.sh --format yaml",
            "a.sh --language zsh",
            "a.sh --help",
            "a.sh --unknown",
        ] {
            assert!(usage(bad), "{bad}");
        }
    }

    #[test]
    fn a_script_in_utf16_is_read() {
        let bytes: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("echo \u{105}".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let dir = std::env::temp_dir().join(format!("nurt-migrate-utf16-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.ps1");
        std::fs::write(&file, bytes).unwrap();
        assert_eq!(read_script(&file).unwrap(), "echo \u{105}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_unknown_option_is_named_in_the_message() {
        assert_eq!(
            parse(&words("a.sh --help")),
            Err(CliError::Usage(
                "unknown option `--help` of `migrate`".to_owned()
            ))
        );
    }

    #[test]
    fn the_script_path_is_the_given_one_or_the_absolute_path_of_the_file() {
        let mut arguments = parse(&words("dir/deploy.sh")).unwrap();
        let derived = script_path(&arguments).unwrap();
        assert!(Path::new(&derived).is_absolute(), "{derived}");
        assert!(derived.ends_with("deploy.sh"), "{derived}");
        arguments.script = Some("/opt/deploy.sh".to_owned());
        assert_eq!(script_path(&arguments).unwrap(), "/opt/deploy.sh");
    }

    #[test]
    fn a_missing_script_is_a_failure_naming_the_file() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = execute(&words("/nonexistent/nurt/deploy.sh"), &mut out, &mut err);
        assert!(matches!(
            result,
            Err(CliError::Failure(message)) if message.starts_with("cannot read `/nonexistent/nurt/deploy.sh`")
        ));
        assert!(out.is_empty() && err.is_empty());
    }

    #[test]
    fn invalid_arguments_are_rejected_before_the_file_is_read() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = execute(&words("--format"), &mut out, &mut err);
        assert!(matches!(result, Err(CliError::Usage(_))));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
