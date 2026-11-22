// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Opening repositories without external programs, repository-relative paths
//! and the errors of the history commands.

use std::fmt::Display;
use std::path::{Component, Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode};

use super::graph::CommitCache;

/// Bytes of decoded objects kept in memory while a command runs.
const OBJECT_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// A repository opened for one command.
pub(crate) struct OpenRepo {
    /// The repository.
    pub(crate) repo: gix::Repository,
    /// Parents, times and trees of the commits read so far.
    pub(crate) commits: CommitCache,
    workdir: Option<PathBuf>,
    label: String,
}

impl OpenRepo {
    /// Opens the repository that contains `path`, a directory or a file that
    /// may no longer exist.
    ///
    /// Configuration of the installation, of the user and of the environment
    /// is not read, and no program is started.
    pub(crate) fn discover(path: &str) -> Result<Self, Error> {
        let given = Path::new(path);
        let start = existing_directory(given).ok_or_else(|| not_repository(path))?;
        let mapping = gix::sec::trust::Mapping {
            full: gix::open::Options::isolated(),
            reduced: gix::open::Options::isolated(),
        };
        let repo = gix::ThreadSafeRepository::discover_opts(
            &start,
            gix::discover::upwards::Options::default(),
            mapping,
        )
        .map_err(|_| not_repository(path))?
        .to_thread_local();
        let mut repo = repo;
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
        let workdir = repo
            .workdir()
            .map(|dir| gix::path::realpath(dir).unwrap_or_else(|_| dir.to_path_buf()));
        let label = workdir
            .as_deref()
            .map_or_else(|| display_path(repo.git_dir()), display_path);
        let commits = CommitCache::new(&repo);
        Ok(Self {
            repo,
            commits,
            workdir,
            label,
        })
    }

    /// Root of the working tree, or of the repository directory when there is
    /// none, as written in suggested programs.
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    /// Root of the working tree.
    pub(crate) fn workdir(&self) -> Option<&Path> {
        self.workdir.as_deref()
    }

    /// Path relative to the root of the working tree, with `/` separators;
    /// the root itself is the empty path.
    pub(crate) fn relative(&self, path: &str) -> Result<BString, Error> {
        let workdir = self.workdir.as_deref().ok_or_else(|| {
            Error::new(
                ErrorCode::GitUnsupported,
                format!("the repository `{}` has no working tree, so `{path}` cannot be resolved", self.label),
            )
            .with_problem("bare_repository")
            .with_next_step(format!("git.log {}", quote(&self.label)))
        })?;
        let absolute = lenient_canonical(Path::new(path));
        let rest = absolute.strip_prefix(workdir).map_err(|_| {
            Error::new(
                ErrorCode::GitNotRepository,
                format!("`{path}` is outside the repository `{}`", self.label),
            )
            .with_problem("path_outside_repo")
            .with_fix(format!("git.log {}", quote(&self.label)))
        })?;
        let mut out = BString::default();
        for component in rest.components() {
            if let Component::Normal(name) = component {
                if !out.is_empty() {
                    out.push(b'/');
                }
                out.extend_from_slice(gix::path::into_bstr(Path::new(name)).as_ref());
            }
        }
        Ok(out)
    }

    /// Absolute path of a repository-relative path, in the canonical form of
    /// the language.
    pub(crate) fn absolute(&self, relative: &BStr) -> String {
        let root = self.workdir.as_deref().unwrap_or_else(|| self.repo.git_dir());
        let mut text = display_path(root);
        if !relative.is_empty() {
            if !text.ends_with('/') {
                text.push('/');
            }
            text.push_str(&relative.to_str_lossy());
        }
        text
    }

    /// Error for an object or reference that cannot be read.
    pub(crate) fn read_error(&self, what: impl Display, err: impl Display) -> Error {
        Error::new(ErrorCode::Io, format!("cannot read {what} in `{}`: {err}", self.label))
            .with_next_step(format!("git.status {}", quote(&self.label)))
    }

    /// Error for a revision that does not name a commit.
    pub(crate) fn revision_error(&self, spec: &str, problem: RevisionProblem) -> Error {
        let message = match problem {
            RevisionProblem::NotFound => format!("unknown revision `{spec}`"),
            RevisionProblem::Ambiguous => format!("the revision `{spec}` is ambiguous, give more characters of the identifier"),
            RevisionProblem::InvalidSyntax => format!("`{spec}` is not a valid revision"),
            RevisionProblem::NotACommit => format!("`{spec}` does not name a commit"),
            RevisionProblem::Range => format!("`{spec}` is a range, give a single revision"),
        };
        Error::new(ErrorCode::GitRevision, message)
            .with_problem(problem.as_str())
            .with_found(spec)
            .with_next_step(format!("git.branch {}", quote(&self.label)))
            .with_next_step(format!("git.log {} count: 10", quote(&self.label)))
    }

    /// Resolves one revision to an object identifier.
    pub(crate) fn resolve_object(&self, spec: &str) -> Result<ObjectId, Error> {
        match self.repo.rev_parse(spec) {
            Ok(parsed) => parsed
                .single()
                .map(gix::Id::detach)
                .ok_or_else(|| self.revision_error(spec, RevisionProblem::Range)),
            Err(err) => Err(self.revision_error(spec, RevisionProblem::classify(&err.to_string()))),
        }
    }

    /// Resolves one revision to a commit, peeling tags.
    pub(crate) fn resolve_commit(&self, spec: &str) -> Result<ObjectId, Error> {
        let id = self.resolve_object(spec)?;
        self.peel_to_commit(id)
            .ok_or_else(|| self.revision_error(spec, RevisionProblem::NotACommit))
    }

    /// The commit an object leads to through tags, if any.
    pub(crate) fn peel_to_commit(&self, id: ObjectId) -> Option<ObjectId> {
        let object = self.repo.find_object(id).ok()?;
        object.peel_to_commit().ok().map(|commit| commit.id)
    }

    /// The tree of a commit.
    pub(crate) fn tree_of(&self, commit: ObjectId) -> Result<ObjectId, Error> {
        self.commits.info(&self.repo, commit).map(|info| info.tree).map_err(|err| self.read_error(format_args!("commit {commit}"), err))
    }

    /// Content of a blob.
    pub(crate) fn blob(&self, id: ObjectId) -> Result<Vec<u8>, Error> {
        self.repo
            .find_object(id)
            .map(|mut object| object.take_data())
            .map_err(|err| self.read_error(format_args!("object {id}"), err))
    }

    /// Abbreviated identifier, unique in the repository.
    pub(crate) fn short_id(&self, id: ObjectId) -> String {
        id.attach(&self.repo).shorten_or_id().to_string()
    }
}

/// Why a revision could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevisionProblem {
    /// Nothing has that name.
    NotFound,
    /// Several objects match an abbreviated identifier.
    Ambiguous,
    /// The text is not a revision.
    InvalidSyntax,
    /// The object is not a commit and does not lead to one.
    NotACommit,
    /// A range was given where a single revision is needed.
    Range,
}

impl RevisionProblem {
    fn classify(message: &str) -> Self {
        let lower = message.to_ascii_lowercase();
        if lower.contains("ambiguous") {
            Self::Ambiguous
        } else if lower.contains("unconsumed") || lower.contains("syntax") || lower.contains("parse") {
            Self::InvalidSyntax
        } else {
            Self::NotFound
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Ambiguous => "ambiguous",
            Self::InvalidSyntax => "invalid_syntax",
            Self::NotACommit => "not_a_commit",
            Self::Range => "range",
        }
    }
}

/// Error for a path outside every repository.
pub(crate) fn not_repository(path: &str) -> Error {
    let dir = Path::new(path)
        .parent()
        .map_or_else(|| path.to_owned(), display_path);
    Error::new(ErrorCode::GitNotRepository, format!("`{path}` is not inside a git repository"))
        .with_problem("not_a_repository")
        .with_next_step(format!("list {}", quote(&dir)))
}

/// Text in double quotes, as written in a program of the language.
pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Path text with `/` separators and without the Windows verbatim prefix.
pub(crate) fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("//?/").map_or(text.clone(), str::to_owned)
}

fn existing_directory(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.is_dir() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

/// Canonical form of a path whose last components may not exist: the
/// longest existing prefix is resolved, the rest is appended as written.
fn lenient_canonical(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            let mut resolved = gix::path::realpath(&current).unwrap_or(current);
            for name in missing.iter().rev() {
                resolved.push(name);
            }
            return resolved;
        }
        match (current.file_name().map(ToOwned::to_owned), current.parent()) {
            (Some(name), Some(parent)) => {
                missing.push(name);
                current = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::Fixture;

    #[test]
    fn quote_escapes_quotes_and_backslashes() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn display_path_uses_forward_slashes_without_verbatim_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\repo")), "C:/repo");
        assert_eq!(display_path(Path::new("/home/user/repo")), "/home/user/repo");
    }

    #[test]
    fn classifies_revision_failures() {
        assert_eq!(RevisionProblem::classify("The short hash 12 is ambiguous"), RevisionProblem::Ambiguous);
        assert_eq!(RevisionProblem::classify("unconsumed input: ~x"), RevisionProblem::InvalidSyntax);
        assert_eq!(RevisionProblem::classify("reference not found"), RevisionProblem::NotFound);
    }

    #[test]
    fn rejects_directory_without_repository() {
        let dir = tempfile::tempdir().unwrap();
        let path = display_path(dir.path());
        let error = OpenRepo::discover(&path).err().unwrap();
        assert_eq!(error.code(), ErrorCode::GitNotRepository);
    }

    #[test]
    fn resolves_paths_relative_to_the_working_tree() {
        let fixture = Fixture::new();
        let repo = fixture.open();
        let root = fixture.root();
        assert_eq!(repo.relative(&root).unwrap(), "");
        assert_eq!(repo.relative(&format!("{root}/src/gone/file.rs")).unwrap(), "src/gone/file.rs");
        assert_eq!(repo.absolute("src/a.rs".into()), format!("{root}/src/a.rs"));
        let outside = repo.relative("/").err().unwrap();
        assert_eq!(outside.code(), ErrorCode::GitNotRepository);
    }

    #[test]
    fn reports_unknown_and_non_commit_revisions() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("a\n"))]);
        let repo = fixture.open();
        assert_eq!(repo.resolve_commit("HEAD").unwrap(), first);
        let unknown = repo.resolve_commit("no-such-branch").err().unwrap();
        assert_eq!(unknown.code(), ErrorCode::GitRevision);
        let tree = repo.resolve_commit("HEAD^{tree}").err().unwrap();
        assert_eq!(tree.code(), ErrorCode::GitRevision);
        let range = repo.resolve_object("HEAD..HEAD").err().unwrap();
        assert_eq!(range.code(), ErrorCode::GitRevision);
        assert_eq!(repo.short_id(first).len(), 7);
    }
}
