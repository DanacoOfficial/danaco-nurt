// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `journal` subcommand: maintenance of the journal of a volume.
//!
//! The only form is `journal salvage <volume>`. It reports the entries of the
//! journal on the volume that need manual repair and the state of the hash
//! chain of the account index. Nothing is changed: a repaired entry is
//! released with `journal.purge`, and its preserved data is copied out with
//! `journal.salvage`. The exit code is 0 when no entry needs repair and the
//! chain has no broken link, and 1 otherwise.

use std::io::Write;
use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{AbsPath, Fs};
use nurt_core::config::{ConfigLoader, ConfigSources};
use nurt_core::system::SystemHost;
use nurt_journal::salvage::{self, Damage, DamagedEntry, LinkStatus};
use nurt_journal::volumes::{Registry, state_dir};
use serde_json::Value;

use crate::app::CliError;
use crate::exit;

/// Parses the arguments of `journal` and returns the volume path.
///
/// # Errors
///
/// Returns a usage error unless the arguments are `salvage` and one path.
pub fn parse(args: &[String]) -> Result<PathBuf, CliError> {
    let Some((form, rest)) = args.split_first() else {
        return Err(CliError::Usage(
            "`journal` needs `salvage <volume>`".to_owned(),
        ));
    };
    if form != "salvage" {
        return Err(CliError::Usage(format!(
            "unknown form `{form}` of `journal`; use `salvage <volume>`"
        )));
    }
    match rest {
        [volume] => Ok(PathBuf::from(volume)),
        [] => Err(CliError::Usage(
            "`journal salvage` needs a volume: `journal salvage <volume>`".to_owned(),
        )),
        [_, extra, ..] => Err(CliError::Usage(format!(
            "unknown argument `{extra}` of `journal salvage`"
        ))),
    }
}

fn damage_text(damage: &Damage) -> String {
    let value = serde_json::to_value(damage).unwrap_or(Value::Null);
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match value.get("problem").and_then(Value::as_str) {
        Some(problem) => format!("{kind} ({problem})"),
        None => kind.to_owned(),
    }
}

fn state_text(entry: &DamagedEntry) -> String {
    entry
        .state
        .and_then(|state| serde_json::to_value(state).ok())
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unreadable".to_owned())
}

fn write_entry(out: &mut dyn Write, entry: &DamagedEntry) -> std::io::Result<()> {
    let damage: Vec<String> = entry.damage.iter().map(damage_text).collect();
    write!(
        out,
        "entry: {}  state: {}  damage: [{}]",
        entry.op.0,
        state_text(entry),
        damage.join(", ")
    )?;
    if let Some(command) = &entry.command {
        write!(out, "  command: {command}")?;
    }
    writeln!(out)?;
    for target in &entry.targets {
        if let Some(path) = &target.path {
            writeln!(out, "  target: {path}")?;
        }
    }
    for blob in &entry.blobs {
        let size = blob
            .size
            .map_or_else(|| "unknown".to_owned(), |size| size.to_string());
        writeln!(out, "  data: {}  size: {size}", blob.name)?;
    }
    Ok(())
}

fn journal_error(message: String) -> CliError {
    CliError::from(Error::new(ErrorCode::NotFound, message))
}

/// Runs `nurt journal` and writes the report to `out`.
///
/// # Errors
///
/// Returns a usage error for invalid arguments, the error of the file system
/// or of the configuration when the volume or the registry cannot be read,
/// and `E_NOT_FOUND` when the volume has no journal.
pub fn execute(args: &[String], out: &mut dyn Write) -> Result<u8, CliError> {
    let volume_path = parse(args)?;
    let absolute = std::path::absolute(&volume_path).map_err(|error| {
        CliError::Failure(format!(
            "cannot resolve `{}`: {error}",
            volume_path.display()
        ))
    })?;
    let target = AbsPath::new(absolute).map_err(Error::from)?;
    let fs = nurt_fs::system_fs();
    let reader: &dyn Fs = &*fs;
    let volume = reader.volume(&target).map_err(Error::from)?;

    let host = SystemHost::detect();
    let loader =
        ConfigLoader::new(&ConfigSources::new(), &host).map_err(|error| error.to_error())?;
    let loaded = loader.load(None).map_err(|error| error.to_error())?;
    let state = state_dir(&host, &loaded.config().state).map_err(Error::from)?;
    let record = Registry::new(state.clone())
        .load(reader)
        .map_err(Error::from)?
        .into_iter()
        .find(|record| record.id == volume.id)
        .ok_or_else(|| {
            journal_error(format!(
                "the volume at {} has no journal; run a changing command on it first",
                volume.mount
            ))
        })?;

    let damaged = salvage::list(reader, &record.journal)?;
    let chain = salvage::verify_chain(reader, &state)?;
    let unverifiable = chain
        .links
        .iter()
        .filter(|(_, status)| matches!(status, LinkStatus::Unverifiable))
        .count();
    writeln!(
        out,
        "volume: {}  journal: {}  damaged: {}  chain: {}  forks: {}  unverifiable: {unverifiable}",
        volume.mount,
        record.journal,
        damaged.len(),
        if chain.is_unbroken() {
            "unbroken"
        } else {
            "broken"
        },
        chain.forks.len(),
    )?;
    for entry in &damaged {
        write_entry(out, entry)?;
    }
    if damaged.is_empty() && chain.is_unbroken() {
        Ok(exit::SUCCESS)
    } else {
        Ok(exit::FAILURE)
    }
}

/// Runs `nurt journal` on the standard output of the process.
///
/// # Errors
///
/// Returns the errors of [`execute`].
pub fn run(args: &[String]) -> Result<u8, CliError> {
    execute(args, &mut std::io::stdout().lock())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn salvage_takes_one_volume() {
        assert_eq!(parse(&words("salvage /")), Ok(PathBuf::from("/")));
        assert_eq!(
            parse(&words("salvage /mnt/dane")),
            Ok(PathBuf::from("/mnt/dane"))
        );
        let args = vec!["salvage".to_owned(), "/mnt/r\u{f3}\u{17c}ne \u{1f600}".to_owned()];
        assert_eq!(
            parse(&args),
            Ok(PathBuf::from("/mnt/r\u{f3}\u{17c}ne \u{1f600}"))
        );
    }

    #[test]
    fn a_form_is_required_and_must_be_salvage() {
        for bad in ["", "purge /", "Salvage /"] {
            assert!(
                matches!(parse(&words(bad)), Err(CliError::Usage(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn salvage_without_a_volume_or_with_more_than_one_is_a_usage_error() {
        assert!(matches!(
            parse(&words("salvage")),
            Err(CliError::Usage(message)) if message.contains("needs a volume")
        ));
        assert!(matches!(
            parse(&words("salvage / /tmp")),
            Err(CliError::Usage(message)) if message.contains("`/tmp`")
        ));
    }

    #[test]
    fn invalid_arguments_are_rejected_before_the_volume_is_opened() {
        let mut out = Vec::new();
        let result = execute(&words("salvage"), &mut out);
        assert!(matches!(result, Err(CliError::Usage(_))));
        assert!(out.is_empty());
    }

    #[test]
    fn a_volume_without_a_journal_is_not_found() {
        let error = journal_error("the volume has no journal".to_owned());
        assert!(matches!(
            error,
            CliError::Language(error) if error.code == ErrorCode::NotFound
        ));
    }
}
