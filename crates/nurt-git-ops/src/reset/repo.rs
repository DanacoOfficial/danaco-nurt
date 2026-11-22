// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Opening a repository without external programs, reading `HEAD`,
//! resolving revisions and the identity used for commits and reflogs.

use std::path::{Path, PathBuf};

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::refs::FullName;
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};

/// Configuration overrides that keep `gix` from starting transport programs.
const SAFE_OVERRIDES: [&str; 3] = [
    "protocol.file.allow=never",
    "protocol.ssh.allow=never",
    "protocol.ext.allow=never",
];

/// Abbreviated length of object names used when the repository has no
/// unique shorter prefix available.
const SHORT_ID_LEN: usize = 7;

/// An open repository with a working tree.
pub(crate) struct Repo {
    /// The `gix` handle.
    pub(crate) repo: gix::Repository,
    /// Root of the working tree.
    pub(crate) workdir: PathBuf,
    /// Repository directory of this working tree (`.git` or
    /// `.git/worktrees/<name>`).
    pub(crate) git_dir: PathBuf,
    /// Directory shared by all working trees.
    pub(crate) common_dir: PathBuf,
}

/// State of `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeadState {
    /// Branch `HEAD` points to; `None` when detached.
    pub(crate) branch: Option<FullName>,
    /// Commit of `HEAD`; `None` on an unborn branch.
    pub(crate) id: Option<ObjectId>,
}

impl HeadState {
    /// Short branch name as used in stash messages, `(no branch)` when
    /// detached.
    pub(crate) fn branch_label(&self) -> String {
        self.branch
            .as_ref()
            .map_or_else(|| "(no branch)".to_owned(), |name| name.shorten().to_string())
    }
}

/// Error with a code and the stage it belongs to.
pub(crate) fn error(code: ErrorCode, message: impl Into<String>) -> Error {
    let stage = match code {
        ErrorCode::GitNotRepository | ErrorCode::GitRevision => Stage::Read,
        ErrorCode::GitConflict | ErrorCode::Io | ErrorCode::Stale => Stage::Apply,
        _ => Stage::Plan,
    };
    Error::new(code, message).with_stage(stage)
}

/// Input or output error of the repository.
pub(crate) fn io_error(context: &str, cause: impl std::fmt::Display) -> Error {
    error(ErrorCode::Io, format!("{context}: {cause}"))
}

/// Quotes a path as a text literal of the language.
pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        if matches!(ch, '"' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

impl Repo {
    /// Opens the repository containing `path`.
    ///
    /// Only the repository configuration is read; the environment, the
    /// global and system configuration and the installation of the `git`
    /// program are not consulted.
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let options = gix::open::Options::isolated().config_overrides(SAFE_OVERRIDES);
        let trust = gix::sec::trust::Mapping {
            full: options.clone(),
            reduced: options,
        };
        let shared =
            gix::ThreadSafeRepository::discover_opts(path, Default::default(), trust).map_err(|cause| {
                error(
                    ErrorCode::GitNotRepository,
                    format!("not a git repository: {}: {cause}", path.display()),
                )
                .with_fix("git.status \".\"")
            })?;
        let repo = shared.to_thread_local();
        let workdir = repo.workdir().map(Path::to_path_buf).ok_or_else(|| {
            error(
                ErrorCode::GitUnsupported,
                format!("the repository at {} has no working tree", path.display()),
            )
            .with_problem("bare_repository")
        })?;
        Ok(Self {
            git_dir: repo.git_dir().to_path_buf(),
            common_dir: repo.common_dir().to_path_buf(),
            workdir,
            repo,
        })
    }

    /// Reads `HEAD`.
    pub(crate) fn head(&self) -> Result<HeadState> {
        let head = self
            .repo
            .head()
            .map_err(|cause| io_error("cannot read HEAD", cause))?;
        Ok(match head.kind {
            gix::head::Kind::Symbolic(reference) => HeadState {
                id: reference.target.try_id().map(ToOwned::to_owned),
                branch: Some(reference.name),
            },
            gix::head::Kind::Unborn(name) => HeadState {
                branch: Some(name),
                id: None,
            },
            gix::head::Kind::Detached { target, .. } => HeadState {
                branch: None,
                id: Some(target),
            },
        })
    }

    /// Resolves `spec` to a commit.
    pub(crate) fn resolve_commit(&self, spec: &str) -> Result<ObjectId> {
        let not_found = |cause: String| {
            error(
                ErrorCode::GitRevision,
                format!("unknown revision `{spec}`: {cause}"),
            )
            .with_problem("unknown")
        };
        let id = self
            .repo
            .rev_parse_single(spec)
            .map_err(|cause| not_found(cause.to_string()))?;
        let commit = id
            .object()
            .map_err(|cause| not_found(cause.to_string()))?
            .peel_to_commit()
            .map_err(|_| {
                error(
                    ErrorCode::GitRevision,
                    format!("revision `{spec}` does not name a commit"),
                )
                .with_problem("not_a_commit")
            })?;
        Ok(commit.id)
    }

    /// Tree of a commit.
    pub(crate) fn commit_tree(&self, commit: ObjectId) -> Result<ObjectId> {
        self.repo
            .find_commit(commit)
            .and_then(|commit| commit.tree_id().map(gix::Id::detach))
            .map_err(|cause| io_error("cannot read commit", cause))
    }

    /// Parents of a commit.
    pub(crate) fn commit_parents(&self, commit: ObjectId) -> Result<Vec<ObjectId>> {
        let commit = self
            .repo
            .find_commit(commit)
            .map_err(|cause| io_error("cannot read commit", cause))?;
        Ok(commit.parent_ids().map(gix::Id::detach).collect())
    }

    /// Subject of a commit as `git log --format=%s` prints it: the first
    /// paragraph with its lines joined by spaces.
    pub(crate) fn commit_subject(&self, commit: ObjectId) -> Result<String> {
        let commit = self
            .repo
            .find_commit(commit)
            .map_err(|cause| io_error("cannot read commit", cause))?;
        let message = commit
            .message_raw()
            .map_err(|cause| io_error("cannot read commit message", cause))?;
        Ok(subject_of(message))
    }

    /// Shortest unique abbreviation of an object name, at least seven
    /// digits.
    pub(crate) fn short_id(&self, id: ObjectId) -> String {
        let prefix = id.attach(&self.repo).shorten_or_id().to_string();
        if prefix.len() >= SHORT_ID_LEN {
            prefix
        } else {
            id.to_hex_with_len(SHORT_ID_LEN).to_string()
        }
    }

    /// Whether `ancestor` is reachable from `descendant`.
    pub(crate) fn is_ancestor(&self, ancestor: ObjectId, descendant: ObjectId) -> bool {
        ancestor == descendant
            || self
                .repo
                .merge_base(ancestor, descendant)
                .is_ok_and(|base| base.detach() == ancestor)
    }

    /// Path of a reference file or a state file: per-worktree names live in
    /// the repository directory of the working tree, the rest in the common
    /// directory.
    pub(crate) fn ref_path(&self, name: &str) -> PathBuf {
        let per_worktree = !name.starts_with("refs/")
            || name.starts_with("refs/bisect/")
            || name.starts_with("refs/worktree/")
            || name.starts_with("refs/rewritten/");
        let base = if per_worktree { &self.git_dir } else { &self.common_dir };
        join_slash(base, name)
    }

    /// Path of the reflog of a reference.
    pub(crate) fn reflog_path(&self, name: &str) -> PathBuf {
        let base = if name.starts_with("refs/") {
            &self.common_dir
        } else {
            &self.git_dir
        };
        join_slash(&base.join("logs"), name)
    }

    /// Path of the index file.
    pub(crate) fn index_path(&self) -> PathBuf {
        self.repo.index_path()
    }

    /// Absolute path of a path relative to the working tree root.
    pub(crate) fn worktree_path(&self, relative: &BStr) -> PathBuf {
        join_slash(&self.workdir, &relative.to_str_lossy())
    }

    /// Path relative to the working tree root, with `/` separators, for a
    /// path given absolute or relative to the root.
    pub(crate) fn relative_path(&self, path: &Path) -> Option<BString> {
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.workdir).ok()?
        } else {
            path
        };
        let mut out = BString::default();
        for component in relative.components() {
            match component {
                std::path::Component::Normal(part) => {
                    if !out.is_empty() {
                        out.push(b'/');
                    }
                    out.extend_from_slice(gix::path::os_str_into_bstr(part).ok()?);
                }
                std::path::Component::CurDir => {}
                _ => return None,
            }
        }
        Some(out)
    }
}

/// Joins a `/`-separated relative name to a directory.
pub(crate) fn join_slash(base: &Path, name: &str) -> PathBuf {
    name.split('/')
        .filter(|part| !part.is_empty())
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

/// First paragraph of a commit message with its lines joined by spaces.
pub(crate) fn subject_of(message: &BStr) -> String {
    let text = message.to_str_lossy();
    let mut parts = Vec::new();
    for line in text.lines().skip_while(|line| line.trim().is_empty()) {
        if line.trim().is_empty() {
            break;
        }
        parts.push(line.trim());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_joins_the_lines_of_the_first_paragraph() {
        assert_eq!(subject_of(b"first\nsecond\n\nbody\n".as_bstr()), "first second");
        assert_eq!(subject_of(b"\n\nonly\n".as_bstr()), "only");
        assert_eq!(subject_of(b"".as_bstr()), "");
    }

    #[test]
    fn quote_escapes_quotes_and_backslashes() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn join_slash_ignores_empty_components() {
        let joined = join_slash(Path::new("/r"), "refs//heads/main");
        assert_eq!(joined, Path::new("/r").join("refs").join("heads").join("main"));
    }
}
