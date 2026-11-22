// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository access of the `git.merge` command: opening without external
//! programs, revisions, identities, the index and content filters.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_object::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::ExecCtx;

use super::message::{DEFAULT_SUPPRESSED_DESTINATIONS, MergeSource};
use super::trees::{io_error, revision_error};
use crate::state::quote;

/// Configuration values that keep `gix` from starting programs for
/// transports.
const SAFE_OVERRIDES: &[&str] = &[
    "protocol.file.allow=never",
    "protocol.ssh.allow=never",
    "protocol.ext.allow=never",
];

pub(crate) struct Repo {
    pub(crate) repo: gix::Repository,
    pub(crate) workdir: PathBuf,
    pub(crate) git_dir: PathBuf,
    pub(crate) given: String,
}

impl Repo {
    /// Opens the repository containing `path` without running programs.
    pub(crate) fn open(path: &str) -> Result<Self> {
        let (location, _trust) = gix::discover::upwards(Path::new(path)).map_err(|error| {
            Error::new(
                ErrorCode::GitNotRepository,
                format!("{path} is not inside a Git repository: {error}; give the path of a repository"),
            )
            .with_found(path.to_owned())
        })?;
        let (git_dir, _) = location.into_repository_and_work_tree_directories();
        let mut permissions = gix::open::Permissions::default();
        permissions.config.git_binary = false;
        let options = gix::open::Options::default()
            .permissions(permissions)
            .config_overrides(SAFE_OVERRIDES.iter().copied());
        let repo = gix::open_opts(&git_dir, options).map_err(|error| {
            Error::new(
                ErrorCode::GitNotRepository,
                format!("cannot open the repository at {}: {error}", git_dir.display()),
            )
        })?;
        let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
            return Err(Error::new(
                ErrorCode::GitUnsupported,
                "the repository has no working tree; merge in a repository with a working tree",
            )
            .with_problem("bare_repository"));
        };
        let git_dir = repo.git_dir().to_path_buf();
        Ok(Self {
            repo,
            workdir,
            git_dir,
            given: path.to_owned(),
        })
    }

    pub(crate) fn quoted(&self) -> String {
        quote(&self.given)
    }

    pub(crate) fn hash_kind(&self) -> gix_hash::Kind {
        self.repo.object_hash()
    }

    pub(crate) fn head(&self) -> Result<Head> {
        let head = self.repo.head().map_err(io_error("read HEAD"))?;
        let reference = head.referent_name().map(|name| name.as_bstr().to_str_lossy().into_owned());
        let commit = head.id().map(gix::Id::detach).ok_or_else(|| {
            Error::new(
                ErrorCode::GitRef,
                "HEAD has no commit yet; create the first commit before merging",
            )
            .with_problem("unborn_branch")
        })?;
        let tree = self.commit_tree(commit)?;
        let branch = reference
            .as_deref()
            .and_then(|name| name.strip_prefix("refs/heads/"))
            .map(str::to_owned);
        Ok(Head {
            commit,
            tree,
            reference,
            branch,
        })
    }

    pub(crate) fn commit_tree(&self, commit: ObjectId) -> Result<ObjectId> {
        let commit = self
            .repo
            .find_commit(commit)
            .map_err(revision_error("read a commit"))?;
        commit
            .tree_id()
            .map(gix::Id::detach)
            .map_err(revision_error("read the tree of a commit"))
    }

    /// Resolves the revision to merge and describes it as `fmt-merge-msg`
    /// does.
    pub(crate) fn merge_source(&self, text: &str) -> Result<Theirs> {
        let commit = self
            .repo
            .rev_parse_single(text)
            .and_then(|id| Ok(id.object()?.peel_to_commit()?.id))
            .map_err(|error| {
                Error::new(
                    ErrorCode::GitRevision,
                    format!("{text} does not name a commit: {error}; give a branch, tag or commit"),
                )
                .with_found(text.to_owned())
                .with_next_step(format!("git.branch {}", self.quoted()))
            })?;
        let source = self.describe(text)?;
        Ok(Theirs {
            commit,
            source,
            label: text.to_owned(),
        })
    }

    fn describe(&self, text: &str) -> Result<MergeSource> {
        if self.has_ref(&format!("refs/heads/{text}"))? {
            return Ok(MergeSource::Branch(text.to_owned()));
        }
        if self.has_ref(&format!("refs/remotes/{text}"))? {
            return Ok(MergeSource::RemoteBranch(text.to_owned()));
        }
        if self.has_ref(&format!("refs/tags/{text}"))? {
            return Ok(MergeSource::Tag {
                name: text.to_owned(),
                message: self.tag_message(text)?,
            });
        }
        Ok(MergeSource::Commit(text.to_owned()))
    }

    fn has_ref(&self, name: &str) -> Result<bool> {
        self.repo
            .try_find_reference(name)
            .map(|found| found.is_some())
            .map_err(io_error("read references"))
    }

    fn tag_message(&self, name: &str) -> Result<Option<String>> {
        let Some(reference) = self
            .repo
            .try_find_reference(&format!("refs/tags/{name}"))
            .map_err(io_error("read references"))?
        else {
            return Ok(None);
        };
        let Some(id) = reference.target().try_id().map(ToOwned::to_owned) else {
            return Ok(None);
        };
        let object = self.repo.find_object(id).map_err(revision_error("read a tag"))?;
        if object.kind != gix_object::Kind::Tag {
            return Ok(None);
        }
        let tag = object.into_tag();
        let decoded = tag.decode().map_err(revision_error("decode a tag"))?;
        Ok(Some(decoded.message.to_str_lossy().into_owned()))
    }

    pub(crate) fn read_index(&self) -> Result<gix_index::State> {
        let path = self.git_dir.join("index");
        if !path.exists() {
            return Ok(gix_index::State::new(self.hash_kind()));
        }
        let file = gix_index::File::at(&path, self.hash_kind(), false, Default::default())
            .map_err(io_error("read the index"))?;
        Ok(file.into_parts().0)
    }

    pub(crate) fn tree_index(&self, tree: ObjectId) -> Result<gix_index::State> {
        gix_index::State::from_tree(&tree, &self.repo.objects, Default::default())
            .map_err(revision_error("read a tree"))
    }

    pub(crate) fn blob(&self, id: ObjectId) -> Result<Vec<u8>> {
        Ok(self
            .repo
            .find_object(id)
            .map_err(revision_error("read a blob"))?
            .detach()
            .data)
    }

    pub(crate) fn signature(&self, ctx: &ExecCtx, role: Role) -> Result<gix::actor::Signature> {
        let (prefix, section) = match role {
            Role::Author => ("GIT_AUTHOR", "author"),
            Role::Committer => ("GIT_COMMITTER", "committer"),
        };
        let config = self.repo.config_snapshot();
        let lookup = |field: &str| -> Option<String> {
            env(ctx, &format!("{prefix}_{}", field.to_ascii_uppercase()))
                .or_else(|| config.string(&format!("{section}.{field}")).map(|v| v.to_str_lossy().into_owned()))
                .or_else(|| config.string(&format!("user.{field}")).map(|v| v.to_str_lossy().into_owned()))
        };
        let (Some(name), Some(email)) = (lookup("name"), lookup("email")) else {
            return Err(Error::new(
                ErrorCode::Config,
                format!("{section} identity unknown; set user.name and user.email in the Git configuration"),
            )
            .with_problem("identity_unknown"));
        };
        let time = match env(ctx, &format!("{prefix}_DATE")) {
            Some(text) => gix::date::parse(&text, Some(system_now(ctx))).map_err(|error| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("{prefix}_DATE has an invalid date {text}: {error}; use a Git date format"),
                )
            })?,
            None => local_now(ctx),
        };
        Ok(gix::actor::Signature {
            name: name.into(),
            email: email.into(),
            time,
        })
    }

    pub(crate) fn comment_char(&self) -> char {
        self.repo
            .config_snapshot()
            .string("core.commentChar")
            .and_then(|value| value.to_str().ok().and_then(|text| text.chars().next()))
            .filter(|c| *c != 'a')
            .unwrap_or('#')
    }

    pub(crate) fn suppressed_destinations(&self) -> Vec<String> {
        let config = self.repo.config_snapshot();
        let values: Vec<String> = config
            .plumbing()
            .strings("merge.suppressDest")
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.to_str_lossy().into_owned())
            .collect();
        if values.is_empty() {
            return DEFAULT_SUPPRESSED_DESTINATIONS.iter().map(|s| (*s).to_owned()).collect();
        }
        let start = values.iter().rposition(String::is_empty).map_or(0, |at| at + 1);
        values[start..].to_vec()
    }

    pub(crate) fn file_mode_enabled(&self) -> bool {
        self.repo.config_snapshot().boolean("core.fileMode").unwrap_or(true)
    }

    pub(crate) fn write_commit(
        &self,
        tree: ObjectId,
        parents: &[ObjectId],
        message: &str,
        author: gix::actor::Signature,
        committer: gix::actor::Signature,
    ) -> Result<ObjectId> {
        let commit = gix_object::Commit {
            tree,
            parents: parents.iter().copied().collect(),
            author,
            committer,
            encoding: None,
            message: message.into(),
            extra_headers: Vec::new(),
        };
        self.repo
            .write_object(&commit)
            .map(gix::Id::detach)
            .map_err(io_error("write the commit"))
    }

    /// Tree of the stage 0 entries of `index` (`git write-tree`).
    pub(crate) fn index_to_tree(&self, index: &gix_index::State) -> Result<ObjectId> {
        let mut editor = gix_object::tree::Editor::new(gix_object::Tree::empty(), &self.repo.objects, self.hash_kind());
        for entry in index.entries() {
            if entry.stage() != gix_index::entry::Stage::Unconflicted {
                continue;
            }
            let Some(kind) = entry_kind(entry.mode) else {
                continue;
            };
            let path = entry.path(index);
            editor
                .upsert(path.split(|byte| *byte == b'/'), kind, entry.id)
                .map_err(io_error("build the tree of the index"))?;
        }
        editor
            .write(|tree| self.repo.write_object(tree).map(gix::Id::detach))
            .map_err(io_error("write the tree of the index"))
    }
}

pub(crate) fn entry_kind(mode: gix_index::entry::Mode) -> Option<EntryKind> {
    if mode == gix_index::entry::Mode::FILE {
        Some(EntryKind::Blob)
    } else if mode == gix_index::entry::Mode::FILE_EXECUTABLE {
        Some(EntryKind::BlobExecutable)
    } else if mode == gix_index::entry::Mode::SYMLINK {
        Some(EntryKind::Link)
    } else if mode == gix_index::entry::Mode::COMMIT {
        Some(EntryKind::Commit)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Role {
    Author,
    Committer,
}

pub(crate) struct Head {
    pub(crate) commit: ObjectId,
    pub(crate) tree: ObjectId,
    pub(crate) reference: Option<String>,
    pub(crate) branch: Option<String>,
}

pub(crate) struct Theirs {
    pub(crate) commit: ObjectId,
    pub(crate) source: MergeSource,
    pub(crate) label: String,
}

fn env(ctx: &ExecCtx, name: &str) -> Option<String> {
    ctx.host()
        .env_var(name)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

fn now_seconds(ctx: &ExecCtx) -> i64 {
    let nanos = ctx.clock().now().unix_nanos();
    i64::try_from(nanos.div_euclid(1_000_000_000)).unwrap_or(i64::MAX)
}

fn system_now(ctx: &ExecCtx) -> SystemTime {
    let seconds = u64::try_from(now_seconds(ctx)).unwrap_or(0);
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn local_now(ctx: &ExecCtx) -> gix::date::Time {
    let seconds = now_seconds(ctx);
    let offset = jiff::Timestamp::from_second(seconds)
        .map(|stamp| jiff::tz::TimeZone::system().to_offset(stamp).seconds())
        .unwrap_or(0);
    gix::date::Time::new(seconds, offset)
}

/// Reflog line of an update of a reference from `old` to `new`.
pub(crate) fn reflog_line(old: ObjectId, new: ObjectId, who: &gix::actor::Signature, message: &str) -> Vec<u8> {
    let offset = who.time.offset;
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.unsigned_abs() / 60;
    format!(
        "{} {} {} <{}> {} {sign}{:02}{:02}\t{}\n",
        old.to_hex(),
        new.to_hex(),
        who.name,
        who.email,
        who.time.seconds,
        minutes / 60,
        minutes % 60,
        message.lines().next().unwrap_or_default()
    )
    .into_bytes()
}

/// Content filters between the working tree and the object database, built
/// without filter programs.
pub(crate) struct Filters<'r> {
    pipeline: gix_filter::Pipeline,
    attributes: gix::AttributeStack<'r>,
}

impl<'r> Filters<'r> {
    pub(crate) fn new(repo: &'r Repo, index: &gix_index::State) -> Result<Self> {
        let attributes = repo
            .repo
            .attributes_only(index, gix_worktree::stack::state::attributes::Source::WorktreeThenIdMapping)
            .map_err(io_error("read attributes"))?;
        let mut options = gix::filter::Pipeline::options(&repo.repo).map_err(io_error("read filter settings"))?;
        options.drivers.clear();
        let context = repo.repo.command_context().map_err(io_error("read the repository paths"))?;
        Ok(Self {
            pipeline: gix_filter::Pipeline::new(context, repo.hash_kind(), options),
            attributes,
        })
    }

    pub(crate) fn to_git(&mut self, data: &[u8], path: &BStr) -> Result<Vec<u8>> {
        let rela = path.to_path().map_err(io_error("convert a path"))?;
        let attributes = &mut self.attributes;
        let outcome = self
            .pipeline
            .convert_to_git(
                data,
                rela,
                &mut |p, out| {
                    if let Ok(platform) = attributes.at_entry(p, None) {
                        platform.matching_attributes(out);
                    }
                },
                &mut |_| Ok(None),
            )
            .map_err(io_error("convert a working tree file"))?;
        let mut out = Vec::new();
        match outcome {
            gix_filter::pipeline::convert::ToGitOutcome::Unchanged(mut reader) => {
                reader.read_to_end(&mut out).map_err(io_error("read a working tree file"))?;
            }
            gix_filter::pipeline::convert::ToGitOutcome::Buffer(buffer) => out.extend_from_slice(buffer),
            gix_filter::pipeline::convert::ToGitOutcome::Process(mut reader) => {
                reader.read_to_end(&mut out).map_err(io_error("read a working tree file"))?;
            }
        }
        Ok(out)
    }

    pub(crate) fn to_worktree(&mut self, data: &[u8], path: &BStr) -> Result<Vec<u8>> {
        let attributes = &mut self.attributes;
        let outcome = self
            .pipeline
            .convert_to_worktree(
                data,
                path,
                &mut |p, out| {
                    if let Ok(platform) = attributes.at_entry(p, None) {
                        platform.matching_attributes(out);
                    }
                },
                gix_filter::driver::apply::Delay::Forbid.into(),
            )
            .map_err(io_error("convert a blob for the working tree"))?;
        match outcome {
            gix_filter::pipeline::convert::ToWorktreeOutcome::Unchanged(bytes) => Ok(bytes.to_vec()),
            gix_filter::pipeline::convert::ToWorktreeOutcome::Buffer(bytes) => Ok(bytes.to_vec()),
            gix_filter::pipeline::convert::ToWorktreeOutcome::Process(_) => Err(Error::new(
                ErrorCode::GitUnsupported,
                format!("a filter program is configured for {path}; it is not run"),
            )),
        }
    }
}

pub(crate) fn blob_id(kind: gix_hash::Kind, data: &[u8]) -> Result<ObjectId> {
    gix_object::compute_hash(kind, gix_object::Kind::Blob, data).map_err(io_error("hash a working tree file"))
}

pub(crate) fn path_string(path: &BString) -> String {
    path.to_str_lossy().into_owned()
}
