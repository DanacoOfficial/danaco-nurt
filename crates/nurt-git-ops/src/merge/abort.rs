// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Restoration of the state before a stopped merge, with the rules of
//! `git reset --merge` that `git merge --abort` uses.
//!
//! The index and the working tree return to the tree of `HEAD` for every
//! path the merge changed. Paths whose index entry still equals `HEAD` keep
//! their working tree content, so local changes made before the merge
//! survive the abort. A path the merge changed and the user edited again is
//! not up to date: the abort is refused before anything changes.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_index::entry::{Mode, Stage};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_git::apply::{ApplyContext, ApplyPlan, ResetOptions, plan_reset};

use super::trees::{io_error, revision_error};
use crate::state::quote;

/// Plans the return of the index and the working tree to `head_tree`,
/// keeping local changes of paths the merge did not change.
pub(super) fn plan_reset_merge(
    ctx: &ApplyContext<'_>,
    head_tree: &ObjectId,
    label: &str,
) -> Result<ApplyPlan> {
    let repo = ctx.repo();
    let head = gix_index::State::from_tree(head_tree, &repo.objects, Default::default())
        .map_err(revision_error("read the tree of HEAD"))?;
    let index = ctx.index();
    let mut checker = Checker::new(repo, ctx.workdir(), index)?;
    let mut keep: BTreeSet<BString> = BTreeSet::new();
    let mut blocked: Vec<String> = Vec::new();
    for entry in index.entries() {
        if entry.stage() != Stage::Unconflicted {
            continue;
        }
        let path = entry.path(index);
        let unchanged = head
            .entry_by_path_and_stage(path, Stage::Unconflicted)
            .is_some_and(|base| base.id == entry.id && base.mode == entry.mode);
        if unchanged {
            keep.insert(path.to_owned());
        } else if !checker.is_up_to_date(path, entry.id, entry.mode)? {
            blocked.push(path.to_str_lossy().into_owned());
        }
    }
    if !blocked.is_empty() {
        return Err(not_up_to_date(&blocked, label));
    }
    let mut plan = plan_reset(ctx, head_tree, ResetOptions::default())?;
    plan.entries.retain(|entry| !keep.contains(&entry.path));
    Ok(plan)
}

fn not_up_to_date(paths: &[String], label: &str) -> Error {
    let mut error = Error::new(
        ErrorCode::GitDirty,
        format!(
            "{} path(s) changed by the merge were edited afterwards; the abort would lose these edits, stash them first",
            paths.len()
        ),
    )
    .with_problem("local_changes");
    for path in paths {
        error = error.with_sample(path.clone());
    }
    error
        .with_fix(format!("git.stash {} action: push", quote(label)))
        .with_next_step(format!("git.status {}", quote(label)))
}

/// Comparison of working tree files with index entries through the
/// built-in content filters; configured filter programs are not run.
struct Checker<'r> {
    root: &'r Path,
    hash: gix_hash::Kind,
    pipeline: gix_filter::Pipeline,
    attributes: gix::AttributeStack<'r>,
}

impl<'r> Checker<'r> {
    fn new(repo: &'r gix::Repository, root: &'r Path, index: &gix_index::State) -> Result<Self> {
        let attributes = repo
            .attributes_only(
                index,
                gix_worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
            )
            .map_err(io_error("read attributes"))?;
        let mut options =
            gix::filter::Pipeline::options(repo).map_err(io_error("read filter settings"))?;
        options.drivers.clear();
        let context = repo
            .command_context()
            .map_err(io_error("read the repository paths"))?;
        Ok(Self {
            root,
            hash: repo.object_hash(),
            pipeline: gix_filter::Pipeline::new(context, repo.object_hash(), options),
            attributes,
        })
    }

    /// Whether the working tree entry of `path` matches `id` and `mode`. A
    /// missing entry counts as up to date, as in Git.
    fn is_up_to_date(&mut self, path: &BStr, id: ObjectId, mode: Mode) -> Result<bool> {
        let relative = path.to_path().map_err(io_error("convert a path"))?;
        let native = self.root.join(relative);
        let meta = match fs::symlink_metadata(&native) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(crate::state::io_error(&native, &error)),
        };
        let data = if meta.file_type().is_symlink() {
            if mode != Mode::SYMLINK {
                return Ok(false);
            }
            let target =
                fs::read_link(&native).map_err(|error| crate::state::io_error(&native, &error))?;
            target.to_string_lossy().into_owned().into_bytes()
        } else if meta.is_file() {
            if mode == Mode::COMMIT {
                return Ok(false);
            }
            let bytes =
                fs::read(&native).map_err(|error| crate::state::io_error(&native, &error))?;
            self.git_form(&bytes, path)?
        } else {
            return Ok(mode == Mode::COMMIT);
        };
        let actual = gix_object::compute_hash(self.hash, gix_object::Kind::Blob, &data)
            .map_err(io_error("hash a working tree file"))?;
        Ok(actual == id)
    }

    fn git_form(&mut self, data: &[u8], path: &BStr) -> Result<Vec<u8>> {
        let relative = path.to_path().map_err(io_error("convert a path"))?;
        let attributes = &mut self.attributes;
        let outcome = self
            .pipeline
            .convert_to_git(
                data,
                relative,
                &mut |entry, out| {
                    if let Ok(platform) = attributes.at_entry(entry, None) {
                        platform.matching_attributes(out);
                    }
                },
                &mut |_| Ok(None),
            )
            .map_err(io_error("convert a working tree file"))?;
        let mut out = Vec::new();
        match outcome {
            gix_filter::pipeline::convert::ToGitOutcome::Unchanged(mut reader) => {
                reader
                    .read_to_end(&mut out)
                    .map_err(io_error("read a working tree file"))?;
            }
            gix_filter::pipeline::convert::ToGitOutcome::Process(mut reader) => {
                reader
                    .read_to_end(&mut out)
                    .map_err(io_error("read a working tree file"))?;
            }
            gix_filter::pipeline::convert::ToGitOutcome::Buffer(buffer) => {
                out.extend_from_slice(buffer);
            }
        }
        Ok(out)
    }
}
