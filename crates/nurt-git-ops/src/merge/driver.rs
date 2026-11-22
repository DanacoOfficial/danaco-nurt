// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Content merge drivers: the built-in drivers of `gix-merge` and the
//! detection of drivers configured as programs (`merge.<name>.driver`).
//!
//! Programs configured in a repository never run: the platform of
//! `gix-merge` is built without configured drivers and without filter
//! programs, so every path is merged by a built-in driver, and each path whose
//! `merge` attribute names a configured program is reported as
//! `filter_not_applied`.

use bstr::{BStr, BString, ByteSlice};
use gix_merge::tree::{Conflict, Resolution};
use nurt_contracts::error::Result;
use nurt_contracts::report::{Notable, NotableEntry};

use super::trees::{MergeOptions, io_error};

/// A merge driver configured as a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDriver {
    /// Name used by the `merge` attribute.
    pub name: String,
    /// Command line of the driver; recorded but never run.
    pub command: String,
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

pub(super) fn unrun_notes(
    repo: &gix::Repository,
    outcome: &gix_merge::tree::Outcome<'_>,
    options: &MergeOptions,
) -> Result<Vec<NotableEntry>> {
    let mut notes = Vec::new();
    if options.drivers.is_empty() {
        return Ok(notes);
    }
    let mut lookup = DriverLookup::new(repo, options)?;
    for conflict in &outcome.conflicts {
        let Some(path) = content_merge_path(conflict) else {
            continue;
        };
        if let Some(driver) = lookup.driver(path.as_bstr())? {
            notes.push(not_run(driver, path.as_bstr()));
        }
    }
    Ok(notes)
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

fn not_run(driver: &ExternalDriver, path: &BStr) -> NotableEntry {
    let mut note = NotableEntry::new(Notable::FilterNotApplied);
    note.path = Some(path.to_str_lossy().into_owned());
    note.detail = Some(format!(
        "merge driver {} is not run; the built-in text driver merged the file",
        driver.name
    ));
    note
}
