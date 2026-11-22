// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Content merge drivers: the built-in drivers of `gix-merge` and drivers
//! configured as programs (`merge.<name>.driver`).
//!
//! Programs never run inside `gix-merge`: its platform is built without
//! configured drivers and without filter programs, so every path is first
//! merged by a built-in driver. Paths whose `merge` attribute names a
//! configured program are then merged again by that program, started
//! through the program launching service of the execution context with the
//! protocol of Git: `%O`, `%A` and `%B` are files holding the base, the
//! current and the other version, the result is read back from `%A`, and a
//! non-zero exit code leaves a conflict.

use std::fs;
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_merge::blob::Resolution as BlobResolution;
use gix_merge::tree::{Conflict, Resolution};
use gix_object::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::ProgramSpec;
use nurt_contracts::stage::ExecCtx;

use super::trees::{MergeOptions, io_error};

/// A merge driver configured as a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDriver {
    /// Name used by the `merge` attribute.
    pub name: String,
    /// Command line run by the shell, with `%O`, `%A`, `%B`, `%L`, `%P`,
    /// `%S`, `%X` and `%Y` placeholders.
    pub command: String,
}

/// Whether configured merge driver programs may run.
#[derive(Clone, Copy)]
pub enum DriverMode<'a> {
    /// Merge with built-in drivers only and list the paths that need a
    /// program in [`super::MergeOutcome::pending_drivers`]; used when
    /// computing a plan, which must not start programs.
    Defer,
    /// Run configured programs through the context.
    Run(&'a ExecCtx),
}

impl std::fmt::Debug for DriverMode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Defer => f.write_str("Defer"),
            Self::Run(_) => f.write_str("Run"),
        }
    }
}

#[derive(Default)]
pub(super) struct DriverReport {
    pub(super) pending: Vec<BString>,
    pub(super) ran: Vec<(String, BString)>,
    pub(super) notes: Vec<NotableEntry>,
}

pub(super) fn from_config(config: &gix::config::Snapshot<'_>) -> Vec<ExternalDriver> {
    let mut out: Vec<ExternalDriver> = Vec::new();
    let Some(sections) = config.plumbing().sections_by_name("merge") else {
        return out;
    };
    for section in sections {
        let Some(name) = section
            .header()
            .subsection_name()
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(command) = section.value("driver") else {
            continue;
        };
        let name = name.to_str_lossy().into_owned();
        let command = command.to_str_lossy().into_owned();
        match out.iter_mut().find(|driver| driver.name == name) {
            Some(existing) => existing.command = command,
            None => out.push(ExternalDriver { name, command }),
        }
    }
    out
}

pub(super) fn blob_platform(
    repo: &gix::Repository,
    options: &MergeOptions,
) -> Result<gix_merge::blob::Platform> {
    let index = repo
        .index_or_load_from_head_or_empty()
        .map_err(io_error("read the index"))?;
    let attributes = repo
        .attributes_only(&index, attribute_source(repo))
        .map_err(io_error("read attributes"))?
        .detach();
    let mut filter_options =
        gix::filter::Pipeline::options(repo).map_err(io_error("read filter settings"))?;
    filter_options.drivers.clear();
    let context = repo
        .command_context()
        .map_err(io_error("read the repository paths"))?;
    let filter = gix_filter::Pipeline::new(context, repo.object_hash(), filter_options);
    let threshold = repo
        .big_file_threshold()
        .map_err(io_error("read core.bigFileThreshold"))?;
    let pipeline = gix_merge::blob::Pipeline::new(
        Default::default(),
        filter,
        gix_merge::blob::pipeline::Options {
            large_file_threshold_bytes: threshold,
        },
    );
    let mode = if options.renormalize {
        gix_merge::blob::pipeline::Mode::Renormalize
    } else {
        gix_merge::blob::pipeline::Mode::ToGit
    };
    Ok(gix_merge::blob::Platform::new(
        pipeline,
        mode,
        attributes,
        Vec::new(),
        gix_merge::blob::platform::Options {
            default_driver: options.default_driver.as_deref().map(BString::from),
        },
    ))
}

fn attribute_source(repo: &gix::Repository) -> gix_worktree::stack::state::attributes::Source {
    if repo.workdir().is_some() {
        gix_worktree::stack::state::attributes::Source::WorktreeThenIdMapping
    } else {
        gix_worktree::stack::state::attributes::Source::IdMapping
    }
}

pub(super) fn run_external(
    repo: &gix::Repository,
    outcome: &mut gix_merge::tree::Outcome<'_>,
    options: &MergeOptions,
    mode: DriverMode<'_>,
) -> Result<DriverReport> {
    let mut report = DriverReport::default();
    if options.drivers.is_empty() {
        return Ok(report);
    }
    let mut lookup = DriverLookup::new(repo, options)?;
    let mut updates = Vec::new();
    for conflict in &mut outcome.conflicts {
        let Some(path) = content_merge_path(conflict) else {
            continue;
        };
        let Some(driver) = lookup.driver(path.as_bstr())? else {
            continue;
        };
        let ctx = match mode {
            DriverMode::Defer => {
                report.pending.push(path);
                continue;
            }
            DriverMode::Run(ctx) => ctx,
        };
        let Some(result) = run_one(
            repo,
            ctx,
            conflict,
            driver,
            path.as_bstr(),
            options,
            &mut report,
        )?
        else {
            continue;
        };
        report.ran.push((driver.name.clone(), path.clone()));
        updates.push((path, result));
    }
    for (path, (kind, id)) in updates {
        outcome
            .tree
            .upsert(path.split(|byte| *byte == b'/'), kind, id)
            .map_err(io_error("update the merged tree"))?;
    }
    Ok(report)
}

fn content_merge_path(conflict: &Conflict) -> Option<BString> {
    match &conflict.resolution {
        Ok(Resolution::OursModifiedTheirsModifiedThenBlobContentMerge { .. }) => {
            Some(conflict.ours.location().to_owned())
        }
        _ => None,
    }
}

struct DriverLookup<'a> {
    stack: gix::AttributeStack<'a>,
    selection: gix::attrs::search::Outcome,
    options: &'a MergeOptions,
}

impl<'a> DriverLookup<'a> {
    fn new(repo: &'a gix::Repository, options: &'a MergeOptions) -> Result<Self> {
        let index = repo
            .index_or_load_from_head_or_empty()
            .map_err(io_error("read the index"))?;
        let stack = repo
            .attributes_only(&index, attribute_source(repo))
            .map_err(io_error("read attributes"))?;
        let mut selection = gix::attrs::search::Outcome::default();
        selection.initialize_with_selection(&Default::default(), ["merge"]);
        Ok(Self {
            stack,
            selection,
            options,
        })
    }

    fn driver(&mut self, path: &BStr) -> Result<Option<&'a ExternalDriver>> {
        let platform = self
            .stack
            .at_entry(path, None)
            .map_err(io_error("read attributes"))?;
        self.selection.reset();
        platform.matching_attributes(&mut self.selection);
        let name = match self
            .selection
            .iter_selected()
            .next()
            .map(|m| m.assignment.state)
        {
            Some(gix::attrs::StateRef::Value(value)) => {
                Some(value.as_bstr().to_str_lossy().into_owned())
            }
            Some(gix::attrs::StateRef::Unspecified) | None => self.options.default_driver.clone(),
            Some(gix::attrs::StateRef::Set | gix::attrs::StateRef::Unset) => None,
        };
        let options = self.options;
        Ok(name.and_then(|name| options.drivers.iter().find(|driver| driver.name == name)))
    }
}

fn run_one(
    repo: &gix::Repository,
    ctx: &ExecCtx,
    conflict: &mut Conflict,
    driver: &ExternalDriver,
    path: &BStr,
    options: &MergeOptions,
    report: &mut DriverReport,
) -> Result<Option<(EntryKind, ObjectId)>> {
    let (Some(host), Some(state_dir)) = (ctx.process(), ctx.state_dir()) else {
        report.notes.push(not_run(
            driver,
            path,
            "the program launching service is not available",
        ));
        return Ok(None);
    };
    let entries = conflict.entries();
    let read = |index: usize| -> Result<Vec<u8>> {
        match entries[index] {
            Some(entry) => Ok(repo
                .find_object(entry.id)
                .map_err(io_error("read a blob to merge"))?
                .detach()
                .data),
            None => Ok(Vec::new()),
        }
    };
    let (base, ours, theirs) = (read(0)?, read(1)?, read(2)?);
    let kind = entries[1].map_or(EntryKind::Blob, |entry| entry.mode.kind());
    let dir = scratch_dir(state_dir, ctx)?;
    let files = Scratch::write(&dir, &base, &ours, &theirs)?;
    let command = expand(&driver.command, &files, path, options);
    let workdir = repo.workdir().unwrap_or(repo.git_dir()).to_path_buf();
    let spec = ProgramSpec::new("sh", vec!["-c".to_owned(), command], workdir);
    let outcome = host.run_program(&spec, ctx.cancel());
    let result = match outcome {
        Ok(outcome) => fs::read(&files.ours).map(|bytes| (outcome.exit_code == Some(0), bytes)),
        Err(_) => {
            remove_scratch(&dir);
            report
                .notes
                .push(not_run(driver, path, "the shell could not be started"));
            return Ok(None);
        }
    };
    remove_scratch(&dir);
    let (clean, merged) = result.map_err(io_error("read the merge driver result"))?;
    let id = repo
        .write_blob(&merged)
        .map_err(io_error("write the merged blob"))?
        .detach();
    if let Ok(Resolution::OursModifiedTheirsModifiedThenBlobContentMerge { merged_blob }) =
        &mut conflict.resolution
    {
        merged_blob.merged_blob_id = id;
        merged_blob.resolution = if clean {
            BlobResolution::Complete
        } else {
            BlobResolution::Conflict
        };
    }
    Ok(Some((kind, id)))
}

fn not_run(driver: &ExternalDriver, path: &BStr, why: &str) -> NotableEntry {
    let mut note = NotableEntry::new(Notable::FilterNotApplied);
    note.path = Some(path.to_str_lossy().into_owned());
    note.detail = Some(format!(
        "merge driver {} was not run because {why}; the built-in text driver merged the file",
        driver.name
    ));
    note
}

struct Scratch {
    base: PathBuf,
    ours: PathBuf,
    theirs: PathBuf,
}

impl Scratch {
    fn write(dir: &Path, base: &[u8], ours: &[u8], theirs: &[u8]) -> Result<Self> {
        let files = Self {
            base: dir.join("base"),
            ours: dir.join("ours"),
            theirs: dir.join("theirs"),
        };
        for (path, data) in [
            (&files.base, base),
            (&files.ours, ours),
            (&files.theirs, theirs),
        ] {
            fs::write(path, data).map_err(io_error("write a merge driver input"))?;
        }
        Ok(files)
    }
}

fn scratch_dir(state_dir: &Path, ctx: &ExecCtx) -> Result<PathBuf> {
    let stamp = ctx.clock().now().unix_nanos();
    let dir = state_dir
        .join("git-merge-driver")
        .join(format!("{stamp:x}"));
    fs::create_dir_all(&dir).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!(
                "cannot create {}: {error}; check the engine state directory",
                dir.display()
            ),
        )
    })?;
    Ok(dir)
}

fn remove_scratch(dir: &Path) {
    // The directory belongs to the engine state directory; a leftover after
    // a failed removal holds only copies of committed blobs.
    fs::remove_dir_all(dir).ok();
}

fn expand(command: &str, files: &Scratch, path: &BStr, options: &MergeOptions) -> String {
    let labels = &options.labels;
    let mut out = String::with_capacity(command.len() + 64);
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('O') => out.push_str(&shell_quote(&files.base.to_string_lossy())),
            Some('A') => out.push_str(&shell_quote(&files.ours.to_string_lossy())),
            Some('B') => out.push_str(&shell_quote(&files.theirs.to_string_lossy())),
            Some('L') => out.push_str(&options.marker_size.to_string()),
            Some('P') => out.push_str(&shell_quote(&path.to_str_lossy())),
            Some('S') => out.push_str(&shell_quote(labels.base.as_deref().unwrap_or_default())),
            Some('X') => out.push_str(&shell_quote(&labels.ours)),
            Some('Y') => out.push_str(&shell_quote(&labels.theirs)),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn shell_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_placeholders_like_git() {
        let files = Scratch {
            base: PathBuf::from("/s/base"),
            ours: PathBuf::from("/s/ours"),
            theirs: PathBuf::from("/s/theirs"),
        };
        let mut options = MergeOptions::default();
        options.labels.base = Some("base label".to_owned());
        options.labels.theirs = "it's".to_owned();
        let out = expand(
            "m %O %A %B %L %P %S %X %Y %% %q %",
            &files,
            BStr::new("d/f.txt"),
            &options,
        );
        assert_eq!(
            out,
            "m '/s/base' '/s/ours' '/s/theirs' 7 'd/f.txt' 'base label' 'HEAD' 'it'\\''s' % %q %"
        );
    }

    #[test]
    fn quotes_single_quotes_for_the_shell() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
