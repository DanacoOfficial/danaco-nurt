// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
    std::fs::read_to_string(path)
        .map_err(|error| CliError::Failure(format!("cannot read `{}`: {error}", path.display())))
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
