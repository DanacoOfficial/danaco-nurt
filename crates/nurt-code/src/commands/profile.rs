// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::commands::profile
// Responsibility:   The `code.profile` stage.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `code.profile` stage.
//!
//! The stage reads the profile of a project root and returns one
//! `profile_entry` record per entry, preceded by a header record with the
//! root, the revision, the number of files and whether the profile came from
//! the cache.

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{ProfileEntry, ProfileSection};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Instant, Path as PathValue, Record, RecordKind, Value};

use crate::profile::{self, ContextFiles, DEFAULT_DAYS, Entry, Env, Profile, Request, Section};
use crate::registry::Registry;

/// Longest history period accepted by `days`: ten years.
const DAYS_MAX: i64 = 3650;

/// Command name in the catalog.
const COMMAND: &str = "code.profile";

/// Stage of `code.profile`: build and test commands, entry points, change
/// hotspots, manifests, languages, modules and instruction files of a
/// project.
///
/// Manifests in JSON and TOML are read through the document service of the
/// execution context, the file tree through its file layer and listing
/// service, and the repository history through its repository query service.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodeProfile;

impl StageImpl for CodeProfile {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        options(args)?;
        let mut info = StaticInfo::default();
        if let Some(root) = args.value("root").and_then(Value::as_str) {
            info.paths.push(root.to_owned());
        }
        Ok(info)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for CodeProfile {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let request = request(args)?;
        let tree = ContextFiles::new(ctx.fs(), ctx.files());
        let env = Env {
            tree: &tree,
            doc: ctx.doc(),
            git: ctx.services().git_query(),
            state_dir: ctx.state_dir(),
            now_ns: i64::try_from(ctx.clock().now().unix_nanos()).unwrap_or(i64::MAX),
            registry: Registry::builtin(),
        };
        let profile = profile::profile(&request, &env)?;
        for reason in &profile.unavailable {
            let mut note = NotableEntry::new(Notable::SourceUnavailable);
            note.detail = Some(reason.clone());
            ctx.note(note);
        }
        let records = profile
            .entries
            .iter()
            .map(|entry| record(&request.root, entry))
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records))
            .with_header(header(&request.root, &profile)))
    }
}

/// Options of the call other than the root.
struct Options {
    sections: Vec<Section>,
    limit: Option<usize>,
    days: u32,
    refresh: bool,
}

fn options(args: &BoundArgs) -> Result<Options> {
    let sections = match args.value("sections") {
        None | Some(Value::Null) => Section::ALL.to_vec(),
        Some(value) => sections(value)?,
    };
    let limit = match args.int("limit")? {
        None => None,
        Some(limit) if limit >= 1 => usize::try_from(limit).ok(),
        Some(limit) => {
            return Err(invalid(
                "limit",
                format!("`limit` is {limit}; pass a number of entries of at least 1"),
            ));
        }
    };
    let days = match args.int("days")? {
        None => DEFAULT_DAYS,
        Some(days) if (1..=DAYS_MAX).contains(&days) => u32::try_from(days).unwrap_or(DEFAULT_DAYS),
        Some(days) => {
            return Err(invalid(
                "days",
                format!("`days` is {days}; pass a number of days from 1 to {DAYS_MAX}"),
            ));
        }
    };
    let refresh = args.bool("refresh")?.unwrap_or(false);
    Ok(Options {
        sections,
        limit,
        days,
        refresh,
    })
}

fn sections(value: &Value) -> Result<Vec<Section>> {
    let names: Vec<&Value> = match value {
        Value::List(items) => items.iter().collect(),
        single => vec![single],
    };
    if names.is_empty() {
        return Err(invalid(
            "sections",
            "`sections` is empty; omit it for every section or name at least one",
        ));
    }
    let mut sections = Vec::new();
    for name in names {
        let text = name.as_str().unwrap_or_default();
        let section = Section::parse(text).ok_or_else(|| {
            let known: Vec<&str> = Section::ALL.iter().map(|s| s.as_str()).collect();
            invalid(
                "sections",
                format!("unknown section `{text}`; use one of {}", known.join(", ")),
            )
        })?;
        if !sections.contains(&section) {
            sections.push(section);
        }
    }
    sections.sort();
    Ok(sections)
}

fn request(args: &BoundArgs) -> Result<Request> {
    let options = options(args)?;
    let root = match args.require("root")? {
        Value::Path(path) | Value::Text(path) => PathBuf::from(path),
        _ => {
            return Err(invalid(
                "root",
                "`root` is not a path; pass the root directory of a project",
            ));
        }
    };
    Ok(Request {
        root,
        sections: options.sections,
        limit: options.limit,
        days: options.days,
        refresh: options.refresh,
    })
}

fn invalid(param: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidValue, message).with_detail("param", serde_json::json!(param))
}

/// Path of a file given relative to the root, in canonical form.
fn canonical(root: &Path, relative: &str) -> String {
    let root = root.display().to_string().replace('\\', "/");
    let root = root.trim_end_matches('/');
    if relative.is_empty() || relative == "." {
        root.to_owned()
    } else {
        format!("{root}/{relative}")
    }
}

/// Record of an entry.
///
/// Sections outside [`ProfileSection`] (`languages`, `modules`,
/// `instructions`) keep the fields of `profile_entry` with their own
/// section name.
fn record(root: &Path, entry: &Entry) -> Record {
    let section = match entry.section {
        Section::Commands => ProfileSection::Commands,
        Section::EntryPoints => ProfileSection::EntryPoints,
        Section::Hotspots => ProfileSection::Hotspots,
        Section::Manifests | Section::Languages | Section::Modules | Section::Instructions => {
            ProfileSection::Manifests
        }
    };
    let typed = ProfileEntry {
        section,
        name: entry.name.clone(),
        path: entry
            .path
            .as_deref()
            .map(|path| PathValue::from(canonical(root, path))),
        command: entry.command.clone(),
        origin: entry.origin.clone(),
        changes: entry.changes,
        last_change: entry.last_change_ns.map(Instant::from_unix_nanos),
    };
    let mut record = Record::from(typed);
    if matches!(
        entry.section,
        Section::Languages | Section::Modules | Section::Instructions
    ) {
        record.set("section", Value::Enum(entry.section.as_str().to_owned()));
    }
    record
}

fn header(root: &Path, profile: &Profile) -> Record {
    let mut header = Record::new(RecordKind::Map);
    header.set("root", Value::Path(canonical(root, "")));
    header.set(
        "revision",
        profile.revision.clone().map_or(Value::Null, Value::Text),
    );
    header.set("repository", Value::Bool(profile.in_repository));
    header.set(
        "files",
        Value::Int(i64::try_from(profile.files).unwrap_or(i64::MAX)),
    );
    header.set("truncated", Value::Bool(profile.truncated));
    header.set("cached", Value::Bool(profile.cache_age_ns.is_some()));
    if let Some(age) = profile.cache_age_ns {
        const NANOS_PER_MILLI: i64 = 1_000_000;
        header.set("cache_age_ms", Value::Int(age / NANOS_PER_MILLI));
    }
    header
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;

    use super::*;

    fn args(pairs: &[(&str, Value)]) -> BoundArgs {
        pairs
            .iter()
            .fold(BoundArgs::new(COMMAND), |args, (name, value)| {
                args.with(*name, ArgValue::Value(value.clone()), false)
            })
    }

    #[test]
    fn options_default_to_every_section_and_ninety_days() {
        let options = options(&args(&[])).unwrap();
        assert_eq!(options.sections, Section::ALL.to_vec());
        assert_eq!(options.days, DEFAULT_DAYS);
        assert_eq!(options.limit, None);
        assert!(!options.refresh);
    }

    #[test]
    fn options_reject_out_of_range_values() {
        for (name, value) in [
            ("limit", Value::Int(0)),
            ("days", Value::Int(0)),
            ("days", Value::Int(DAYS_MAX + 1)),
            ("sections", Value::List(Vec::new())),
            (
                "sections",
                Value::List(vec![Value::Enum("tests".to_owned())]),
            ),
        ] {
            let error = options(&args(&[(name, value)])).err().unwrap();
            assert_eq!(error.code, ErrorCode::InvalidValue, "{name}");
        }
    }

    #[test]
    fn sections_are_deduplicated_in_output_order() {
        let value = Value::List(vec![
            Value::Enum("hotspots".to_owned()),
            Value::Enum("commands".to_owned()),
            Value::Enum("hotspots".to_owned()),
        ]);
        assert_eq!(
            sections(&value).unwrap(),
            [Section::Commands, Section::Hotspots]
        );
    }

    #[test]
    fn canonical_paths_join_root_and_relative_path() {
        assert_eq!(canonical(Path::new("/r/"), "a/b"), "/r/a/b");
        assert_eq!(canonical(Path::new("/r"), "."), "/r");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
