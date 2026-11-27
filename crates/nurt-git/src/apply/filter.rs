// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Content conversion between the object database and the working tree.
//!
//! The built-in conversions of `gix-filter` apply: end-of-line handling
//! (`text`, `eol`, `core.autocrlf`, `core.eol`), `ident` and
//! `working-tree-encoding`. Filter drivers configured as `filter.<name>.*`
//! are never started here; a path whose `filter` attribute names a
//! configured driver is converted with the built-in conversions only and
//! reported as `filter_not_applied`.

use std::io::Read;

use bstr::{BStr, BString};
use gix_filter::pipeline::convert::{ToGitOutcome, ToWorktreeOutcome, to_worktree};
use gix_worktree::stack::state::attributes::Source;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::error::repo_error;
use super::plan::Notable;
use crate::repo::filter::PathAttributes;

/// Name of the attribute that selects a filter driver.
const FILTER_ATTRIBUTE: &str = "filter";

/// Direction of the attribute lookup, as `git` uses for check-in and
/// check-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Attributes of the working tree take precedence over the index.
    ToGit,
    /// Attributes of the index take precedence over the working tree.
    ToWorktree,
}

/// Conversion pipeline of one repository with filter drivers disabled.
pub(crate) struct Filters<'r> {
    repo: &'r gix::Repository,
    pipeline: gix_filter::Pipeline,
    stack: gix_worktree::Stack,
    drivers: Vec<BString>,
}

impl<'r> Filters<'r> {
    /// Pipeline whose attributes come from `index` and the working tree in
    /// the order given by `direction`.
    pub(crate) fn new(
        repo: &'r gix::Repository,
        index: &gix_index::State,
        direction: Direction,
    ) -> Result<Self> {
        let source = match direction {
            Direction::ToGit => Source::WorktreeThenIdMapping,
            Direction::ToWorktree => Source::IdMappingThenWorktree,
        };
        let stack = repo
            .attributes_only(index, source)
            .map_err(|err| repo_error(err, "read the attributes"))?
            .detach();
        let (mut pipeline, stack) = gix::filter::Pipeline::new(repo, stack)
            .map_err(|err| repo_error(err, "configure the content filters"))?
            .into_parts();
        let drivers = std::mem::take(&mut pipeline.options_mut().drivers)
            .into_iter()
            .map(|driver| driver.name)
            .collect();
        Ok(Self {
            repo,
            pipeline,
            stack,
            drivers,
        })
    }

    /// Converts working tree bytes of `path` to the form stored in the
    /// object database.
    pub(crate) fn clean(
        &mut self,
        path: &BStr,
        data: &[u8],
        index: &gix_index::State,
    ) -> Result<Vec<u8>> {
        let native = gix_path::from_bstr(path).map_err(|err| path_error(path, &err))?;
        let platform = self
            .stack
            .at_entry(path, None, &self.repo.objects)
            .map_err(|err| repo_error(err, "read the attributes"))?;
        let repo = self.repo;
        let outcome = self
            .pipeline
            .convert_to_git(
                data,
                &native,
                &mut |_, attrs| {
                    platform.matching_attributes(attrs);
                },
                &mut |buf| {
                    let Some(entry) = index.entry_by_path(path) else {
                        return Ok(None);
                    };
                    let object = gix_object::Find::try_find(&repo.objects, &entry.id, buf)?;
                    Ok(object
                        .filter(|object| object.kind == gix_object::Kind::Blob)
                        .map(|_| ()))
                },
            )
            .map_err(|err| convert_error(path, &err))?;
        let mut out = Vec::with_capacity(data.len());
        match outcome {
            ToGitOutcome::Unchanged(mut reader) => reader
                .read_to_end(&mut out)
                .map_err(|err| convert_error(path, &err))?,
            ToGitOutcome::Buffer(bytes) => {
                out.extend_from_slice(bytes);
                bytes.len()
            }
            ToGitOutcome::Process(mut reader) => reader
                .read_to_end(&mut out)
                .map_err(|err| convert_error(path, &err))?,
        };
        Ok(out)
    }

    /// Converts a blob of `path` to its working tree form.
    ///
    /// The notable effect is present when the path names a configured filter
    /// driver that was not applied.
    pub(crate) fn smudge(
        &mut self,
        path: &BStr,
        data: &[u8],
    ) -> Result<(Vec<u8>, Option<Notable>)> {
        let mut matches = (!self.drivers.is_empty())
            .then(|| self.stack.selected_attribute_matches([FILTER_ATTRIBUTE]));
        let platform = self
            .stack
            .at_entry(path, None, &self.repo.objects)
            .map_err(|err| repo_error(err, "read the attributes"))?;
        let skipped = matches.as_mut().and_then(|matches| {
            platform.matching_attributes(matches);
            filter_driver(matches)
                .filter(|name| self.drivers.iter().any(|driver| driver == name))
                .map(|_| Notable::at("filter_not_applied", path))
        });
        let outcome = self
            .pipeline
            .convert_to_worktree(
                data,
                path,
                &mut |_, attrs| {
                    platform.matching_attributes(attrs);
                },
                to_worktree::Options {
                    can_delay: gix_filter::driver::apply::Delay::Forbid,
                    unknown_encoding: to_worktree::UnknownEncoding::Fail,
                },
            )
            .map_err(|err| convert_error(path, &err))?;
        let bytes = match outcome {
            ToWorktreeOutcome::Unchanged(bytes) | ToWorktreeOutcome::Buffer(bytes) => {
                bytes.to_vec()
            }
            ToWorktreeOutcome::Process(_) => data.to_vec(),
        };
        let bytes = PathAttributes::read(&platform)
            .finish_worktree_content(bytes, self.repo.object_hash().len_in_hex());
        Ok((bytes, skipped))
    }
}

fn filter_driver(matches: &gix_attributes::search::Outcome) -> Option<BString> {
    matches
        .iter_selected()
        .find_map(|found| match found.assignment.state {
            gix_attributes::StateRef::Value(value) => Some(value.as_bstr().to_owned()),
            _ => None,
        })
}

fn path_error(path: &BStr, err: &dyn std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("the path '{path}' cannot be represented on this system: {err}"),
    )
    .with_found(path.to_string())
}

fn convert_error(path: &BStr, err: &dyn std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot convert the content of '{path}': {err}; check .gitattributes for the path"),
    )
    .with_found(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository(dir: &std::path::Path, attributes: &str) -> gix::Repository {
        let repo = crate::testutil::init_isolated(dir).expect("init");
        std::fs::write(dir.join(".gitattributes"), attributes).expect("attributes");
        repo
    }

    fn smudged(attributes: &str, data: &[u8]) -> Vec<u8> {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = repository(dir.path(), attributes);
        let index = gix_index::State::new(repo.object_hash());
        let mut filters = Filters::new(&repo, &index, Direction::ToWorktree).expect("filters");
        filters
            .smudge(b"f.txt".as_slice().into(), data)
            .expect("smudge")
            .0
    }

    #[test]
    fn smudge_completes_ident_marker_with_object_id() {
        let out = smudged("f.txt ident\n", b"$Id$\n");
        let text = String::from_utf8(out).expect("utf-8");
        assert!(
            text.starts_with("$Id: ") && text.ends_with(" $\n"),
            "{text}"
        );
    }

    #[test]
    fn smudge_encodes_utf16_working_tree_encoding() {
        let out = smudged("f.txt working-tree-encoding=UTF-16LE\n", b"a\n");
        assert_eq!(out, [b'a', 0, b'\n', 0]);
    }
}

#[cfg(test)]
mod clean_tests {
    use super::*;

    #[test]
    fn clean_converts_working_tree_line_endings_to_lf() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = crate::testutil::init_isolated(dir.path()).expect("init");
        std::fs::write(dir.path().join(".gitattributes"), "f.txt text eol=crlf\n")
            .expect("attributes");
        let index = gix_index::State::new(repo.object_hash());
        let mut filters = Filters::new(&repo, &index, Direction::ToGit).expect("filters");
        let out = filters
            .clean(b"f.txt".as_slice().into(), b"a\r\nb\r\n", &index)
            .expect("clean");
        assert_eq!(out, b"a\nb\n");
    }

    #[test]
    fn conversion_errors_name_the_path() {
        let error = convert_error(b"x.txt".as_slice().into(), &"bad");
        assert_eq!(error.code(), ErrorCode::Io);
        assert!(error.to_string().contains("x.txt"));
        assert!(
            path_error(b"y".as_slice().into(), &"bad")
                .to_string()
                .contains('y')
        );
    }
}
