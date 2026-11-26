// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Three-way merge of trees and commits.

use std::fmt::Display;
use std::num::NonZeroU8;

use bstr::{BStr, ByteSlice};
use gix::prelude::ObjectIdExt;
use gix_hash::ObjectId;
use gix_merge::blob::builtin_driver::binary::ResolveWith;
use gix_merge::blob::builtin_driver::text;
use gix_merge::tree::TreatAsUnresolved;
use gix_merge::tree::apply_index_entries::RemovalMode;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::report::NotableEntry;

use super::conflict::{self, MergeConflict};
use super::driver::{self, ExternalDriver};
use super::layout;

/// Marker size of Git when neither the `conflict-marker-size` attribute
/// nor the caller sets one.
pub const DEFAULT_MARKER_SIZE: u8 = 7;

/// Rename limit of `merge-ort` when neither `merge.renameLimit` nor
/// `diff.renameLimit` is set.
pub const DEFAULT_RENAME_LIMIT: usize = 7000;

/// How conflicting hunks are written into a file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ConflictStyle {
    /// Ours and theirs (`merge.conflictStyle = merge`).
    #[default]
    Merge,
    /// Ours, base and theirs (`diff3`).
    Diff3,
    /// Like `diff3`, with lines common to both sides moved out of the
    /// conflict (`zdiff3`).
    Zdiff3,
}

impl ConflictStyle {
    /// Parses a value of `merge.conflictStyle` or of the `conflict_style`
    /// parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::merge::ConflictStyle;
    ///
    /// assert_eq!(ConflictStyle::parse("zdiff3"), Some(ConflictStyle::Zdiff3));
    /// assert_eq!(ConflictStyle::parse("other"), None);
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "merge" => Some(Self::Merge),
            "diff3" => Some(Self::Diff3),
            "zdiff3" => Some(Self::Zdiff3),
            _ => None,
        }
    }

    /// Stable name of the style.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Diff3 => "diff3",
            Self::Zdiff3 => "zdiff3",
        }
    }

    const fn to_gix(self) -> text::ConflictStyle {
        match self {
            Self::Merge => text::ConflictStyle::Merge,
            Self::Diff3 => text::ConflictStyle::Diff3,
            Self::Zdiff3 => text::ConflictStyle::ZealousDiff3,
        }
    }
}

/// Side that wins conflicting hunks (`git merge -X ours|theirs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Favor {
    /// The current side wins.
    Ours,
    /// The merged side wins.
    Theirs,
}

impl Favor {
    /// Parses a value of the `strategy_option` parameter.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ours" => Some(Self::Ours),
            "theirs" => Some(Self::Theirs),
            _ => None,
        }
    }
}

/// Names written after the conflict markers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeLabels {
    /// Label of the base (`|||||||`); `None` lets [`merge_commits`] use the
    /// abbreviated base commit or `merged common ancestors`.
    pub base: Option<String>,
    /// Label of the current side (`<<<<<<<`), usually `HEAD`.
    pub ours: String,
    /// Label of the merged side (`>>>>>>>`), usually the revision as given.
    pub theirs: String,
}

impl MergeLabels {
    /// Labels `HEAD` and `theirs`, with the base chosen by the merge.
    #[must_use]
    pub fn head_and(theirs: impl Into<String>) -> Self {
        Self {
            base: None,
            ours: "HEAD".to_owned(),
            theirs: theirs.into(),
        }
    }
}

/// Settings of a three-way merge.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeOptions {
    /// Style of conflict hunks.
    pub conflict_style: ConflictStyle,
    /// Length of conflict markers, unless the `conflict-marker-size`
    /// attribute of a path sets another.
    pub marker_size: NonZeroU8,
    /// Side that wins conflicting hunks; `None` keeps conflicts.
    pub favor: Option<Favor>,
    /// Rename detection; `None` disables it (`merge.renames = false`).
    pub renames: Option<gix_diff::Rewrites>,
    /// Normalize line endings of all three sides before merging
    /// (`merge.renormalize`).
    pub renormalize: bool,
    /// Driver of paths without a `merge` attribute (`merge.default`).
    pub default_driver: Option<String>,
    /// Merge drivers configured as programs (`merge.<name>.driver`).
    pub drivers: Vec<ExternalDriver>,
    /// Merge commits without a common ancestor against the empty tree.
    pub allow_unrelated: bool,
    /// Names written after the conflict markers.
    pub labels: MergeLabels,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            conflict_style: ConflictStyle::Merge,
            marker_size: NonZeroU8::MIN.saturating_add(DEFAULT_MARKER_SIZE - 1),
            favor: None,
            renames: Some(gix_diff::Rewrites {
                limit: DEFAULT_RENAME_LIMIT,
                ..gix_diff::Rewrites::default()
            }),
            renormalize: false,
            default_driver: None,
            drivers: Vec::new(),
            allow_unrelated: false,
            labels: MergeLabels::head_and("theirs"),
        }
    }
}

impl MergeOptions {
    /// Settings read from the configuration of `repo`: `merge.conflictStyle`,
    /// `merge.renames`, `merge.renameLimit` (falling back to `diff.renames`
    /// and `diff.renameLimit`), `merge.renormalize`, `merge.default` and
    /// `merge.<name>.driver`.
    ///
    /// # Errors
    ///
    /// Fails with `E_CONFIG` when a value is invalid.
    pub fn from_repo(repo: &gix::Repository) -> Result<Self> {
        let config = repo.config_snapshot();
        let mut options = Self::default();
        if let Some(style) = config.string("merge.conflictStyle") {
            let text = style.to_str_lossy();
            options.conflict_style = ConflictStyle::parse(&text).ok_or_else(|| {
                Error::new(
                    ErrorCode::Config,
                    format!(
                        "merge.conflictStyle has unknown value {text}; use merge, diff3 or zdiff3"
                    ),
                )
                .with_found(text.into_owned())
            })?;
        }
        let tree_options: gix_merge::tree::Options = repo
            .tree_merge_options()
            .map_err(config_error("merge.renames"))?
            .into();
        options.renames = tree_options.renames_with_default_limit(&config);
        options.renormalize = config.boolean("merge.renormalize").unwrap_or(false);
        options.default_driver = config
            .string("merge.default")
            .map(|name| name.to_str_lossy().into_owned());
        options.drivers = driver::from_config(&config);
        Ok(options)
    }

    fn tree_options(&self) -> gix_merge::tree::Options {
        let conflict = match self.favor {
            Some(Favor::Ours) => text::Conflict::ResolveWithOurs,
            Some(Favor::Theirs) => text::Conflict::ResolveWithTheirs,
            None => text::Conflict::Keep {
                style: self.conflict_style.to_gix(),
                marker_size: self.marker_size,
            },
        };
        let binary = self.favor.map(|favor| match favor {
            Favor::Ours => ResolveWith::Ours,
            Favor::Theirs => ResolveWith::Theirs,
        });
        gix_merge::tree::Options {
            rewrites: self.renames,
            blob_merge: gix_merge::blob::platform::merge::Options {
                is_virtual_ancestor: false,
                resolve_binary_with: binary,
                text: text::Options {
                    diff_algorithm: gix_diff::blob::Algorithm::Histogram,
                    conflict,
                },
            },
            blob_merge_command_ctx: Default::default(),
            fail_on_conflict: None,
            marker_size_multiplier: 0,
            symlink_conflicts: binary,
            tree_conflicts: None,
        }
    }
}

trait RenameLimit {
    fn renames_with_default_limit(
        self,
        config: &gix::config::Snapshot<'_>,
    ) -> Option<gix_diff::Rewrites>;
}

impl RenameLimit for gix_merge::tree::Options {
    fn renames_with_default_limit(
        self,
        config: &gix::config::Snapshot<'_>,
    ) -> Option<gix_diff::Rewrites> {
        let configured = config.integer("merge.renameLimit").is_some()
            || config.integer("diff.renameLimit").is_some();
        self.rewrites.map(|mut rewrites| {
            if !configured {
                rewrites.limit = DEFAULT_RENAME_LIMIT;
            }
            rewrites
        })
    }
}

/// Result of a three-way merge.
///
/// The result tree holds every cleanly merged change and, for conflicts,
/// the content Git leaves in the working tree: text with conflict markers,
/// the current side of binary files and of modify/delete conflicts, and
/// both sides of rename conflicts. It is the tree Git records in
/// `AUTO_MERGE`.
#[derive(Debug, Clone)]
pub struct MergeOutcome {
    /// Result tree, written to the object database.
    pub tree: ObjectId,
    /// Index of the result: the entries of [`Self::tree`] with conflicting
    /// paths replaced by their stages 1, 2 and 3. File system fields of the
    /// entries are zero.
    pub index: gix_index::State,
    /// Unresolved conflicts, sorted by path.
    pub conflicts: Vec<MergeConflict>,
    /// Tree used as the base.
    pub base_tree: ObjectId,
    /// Best common ancestors of the merged commits; empty for tree merges
    /// and unrelated histories.
    pub merge_bases: Vec<ObjectId>,
    /// Commits created to merge several bases into one, the last being the
    /// base used.
    pub virtual_bases: Vec<ObjectId>,
    /// Unusual effects, for example a configured merge driver that is not run.
    pub notes: Vec<NotableEntry>,
}

impl MergeOutcome {
    /// Returns `true` when no conflict is left to resolve.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }

    /// Conflicting paths in index order, as listed in the `# Conflicts:`
    /// section of `MERGE_MSG`.
    #[must_use]
    pub fn conflicted_paths(&self) -> Vec<&BStr> {
        self.conflicts.iter().map(|c| c.path.as_bstr()).collect()
    }
}

/// Merges `ours` and `theirs` against `base` (all three trees).
///
/// Changes are detected with renames according to `options.renames`; text is
/// merged with the histogram algorithm, as by `merge-ort`. Merged blobs and
/// the result tree are written to the object database of `repo`; the
/// working tree, the index and the references are not touched. Merge
/// drivers configured as programs are not run; their paths are reported as
/// `filter_not_applied`.
///
/// # Errors
///
/// Fails with `E_GIT_REVISION` when a tree cannot be read and with `E_IO`
/// when an object cannot be written.
pub fn merge_trees(
    repo: &gix::Repository,
    base: &gix_hash::oid,
    ours: &gix_hash::oid,
    theirs: &gix_hash::oid,
    options: &MergeOptions,
) -> Result<MergeOutcome> {
    let mut diff_cache = repo
        .diff_resource_cache_for_tree_diff()
        .map_err(io_error("prepare the merge"))?;
    let mut platform = driver::blob_platform(repo, options)?;
    let labels = &options.labels;
    let outcome = gix_merge::tree(
        base,
        ours,
        theirs,
        text::Labels {
            ancestor: labels
                .base
                .as_deref()
                .map(|label| label.as_bytes().as_bstr()),
            current: Some(labels.ours.as_bytes().as_bstr()),
            other: Some(labels.theirs.as_bytes().as_bstr()),
        },
        repo,
        |buf| gix_object::Write::write_buf(repo, gix_object::Kind::Blob, buf),
        &mut gix_diff::tree::State::default(),
        &mut diff_cache,
        &mut platform,
        options.tree_options(),
    )
    .map_err(revision_error("merge the trees"))?;
    let bases = Bases {
        tree: base.to_owned(),
        commits: Vec::new(),
        virtual_commits: Vec::new(),
    };
    finish(repo, outcome, bases, options)
}

/// Merges the commits `ours` and `theirs` (`git merge` and `ort`).
///
/// The base is the best common ancestor; several best ancestors are merged
/// recursively into a virtual base, labelled `merged common ancestors`, with
/// conflict markers of the inner merges longer by two per level. Without a
/// common ancestor the merge fails unless `options.allow_unrelated` is set,
/// in which case the empty tree is the base.
///
/// # Errors
///
/// Fails with `E_GIT_NON_FAST_FORWARD` (problem `unrelated_histories`) when
/// the commits share no history and unrelated histories are not allowed,
/// with `E_GIT_REVISION` when a commit cannot be read and with `E_IO` when an
/// object cannot be written.
pub fn merge_commits(
    repo: &gix::Repository,
    ours: ObjectId,
    theirs: ObjectId,
    options: &MergeOptions,
) -> Result<MergeOutcome> {
    let bases = merge_bases(repo, ours, theirs)?;
    if bases.is_empty() && !options.allow_unrelated {
        return Err(unrelated_error());
    }
    let mut diff_cache = repo
        .diff_resource_cache_for_tree_diff()
        .map_err(io_error("prepare the merge"))?;
    let mut platform = driver::blob_platform(repo, options)?;
    let commit_graph = repo
        .commit_graph_if_enabled()
        .map_err(io_error("read the commit graph"))?;
    let mut graph = repo.revision_graph(commit_graph.as_ref());
    let labels = &options.labels;
    let mut abbreviate =
        |id: &gix_hash::oid| id.to_owned().attach(repo).shorten_or_id().to_string();
    let outcome = gix_merge::commit(
        ours,
        theirs,
        text::Labels {
            ancestor: labels
                .base
                .as_deref()
                .map(|label| label.as_bytes().as_bstr()),
            current: Some(labels.ours.as_bytes().as_bstr()),
            other: Some(labels.theirs.as_bytes().as_bstr()),
        },
        &mut graph,
        &mut diff_cache,
        &mut platform,
        repo,
        &mut abbreviate,
        gix_merge::commit::Options {
            allow_missing_merge_base: options.allow_unrelated,
            tree_merge: options.tree_options(),
            use_first_merge_base: false,
        },
    )
    .map_err(revision_error("merge the commits"))?;
    let bases = Bases {
        tree: outcome.merge_base_tree_id,
        commits: outcome.merge_bases.map(Vec::from).unwrap_or_default(),
        virtual_commits: outcome.virtual_merge_bases,
    };
    finish(repo, outcome.tree_merge, bases, options)
}

/// Best common ancestors of `ours` and `theirs`; empty when the histories
/// are unrelated.
///
/// # Errors
///
/// Fails with `E_GIT_REVISION` when a commit cannot be read.
pub fn merge_bases(
    repo: &gix::Repository,
    ours: ObjectId,
    theirs: ObjectId,
) -> Result<Vec<ObjectId>> {
    let commit_graph = repo
        .commit_graph_if_enabled()
        .map_err(io_error("read the commit graph"))?;
    let mut graph = repo.revision_graph(commit_graph.as_ref());
    let bases = gix_revision::merge_base(ours, &[theirs], &mut graph)
        .map_err(revision_error("find the merge base"))?;
    Ok(bases.map(Vec::from).unwrap_or_default())
}

/// Tree of the virtual base of `bases`: the bases merged recursively into
/// one, as `merge-ort` does for criss-cross histories; the tree of the only
/// base when there is one.
///
/// # Errors
///
/// Fails with `E_GIT_REVISION` when `bases` is empty or a commit cannot be
/// read.
pub fn merge_base_virtual(
    repo: &gix::Repository,
    bases: &[ObjectId],
    options: &MergeOptions,
) -> Result<ObjectId> {
    let outcome = repo
        .virtual_merge_base(bases.iter().copied(), options.tree_options().into())
        .map_err(revision_error("merge the merge bases"))?;
    Ok(outcome.tree_id.detach())
}

/// `E_GIT_NON_FAST_FORWARD` for histories without a common ancestor.
#[must_use]
pub fn unrelated_error() -> Error {
    Error::new(
        ErrorCode::GitNonFastForward,
        "refusing to merge unrelated histories; the commits share no common ancestor, so merge a branch that descends from the current one",
    )
    .with_problem("unrelated_histories")
}

struct Bases {
    tree: ObjectId,
    commits: Vec<ObjectId>,
    virtual_commits: Vec<ObjectId>,
}

fn finish(
    repo: &gix::Repository,
    mut outcome: gix_merge::tree::Outcome<'_>,
    bases: Bases,
    options: &MergeOptions,
) -> Result<MergeOutcome> {
    let notes = driver::unrun_notes(repo, &outcome, options)?;
    let fixes = layout::plan(
        &outcome.conflicts,
        &outcome.tree,
        &options.labels.ours,
        &options.labels.theirs,
    );
    layout::apply_to_tree(&mut outcome.tree, &fixes)?;
    let tree = outcome
        .tree
        .write(|tree| repo.write_object(tree).map(gix::Id::detach))
        .map_err(io_error("write the merged tree"))?;
    let mut index = gix_index::State::from_tree(&tree, &repo.objects, Default::default())
        .map_err(revision_error("read the merged tree"))?;
    gix_merge::tree::apply_index_entries(
        &outcome.conflicts,
        TreatAsUnresolved::git(),
        &mut index,
        RemovalMode::Prune,
    );
    layout::apply_to_index(&mut index, &fixes);
    let conflicts = conflict::collect(&index, &outcome.conflicts);
    Ok(MergeOutcome {
        tree,
        index,
        conflicts,
        base_tree: bases.tree,
        merge_bases: bases.commits,
        virtual_bases: bases.virtual_commits,
        notes,
    })
}

pub(crate) fn io_error<E: Display>(what: &'static str) -> impl FnOnce(E) -> Error {
    move |error| Error::new(ErrorCode::Io, format!("cannot {what}: {error}"))
}

pub(crate) fn revision_error<E: Display>(what: &'static str) -> impl FnOnce(E) -> Error {
    move |error| {
        Error::new(
            ErrorCode::GitRevision,
            format!("cannot {what}: {error}; check that the repository objects are complete"),
        )
    }
}

fn config_error<E: Display>(key: &'static str) -> impl FnOnce(E) -> Error {
    move |error| {
        Error::new(
            ErrorCode::Config,
            format!("invalid value of {key}: {error}; correct the repository configuration"),
        )
    }
}
