// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Opening repositories without external programs, repository-relative paths
//! and the errors of the history commands.

use std::fmt::Display;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix::prelude::ObjectIdExt as _;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::ExecCtx;
use nurt_git_transport::sign::{BuiltinSigner, Signer, VerifyConfig};

use super::graph::CommitCache;
use crate::repo::GitRepo;

/// Bytes of decoded objects kept in memory while a command runs.
const OBJECT_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Shortest abbreviation `git` accepts in `core.abbrev`.
const MIN_ABBREVIATION: usize = 4;

/// Longest abbreviation of a SHA-1 identifier.
const MAX_ABBREVIATION: usize = 40;

/// Abbreviation length that repositories of up to 16384 objects get.
const AUTO_ABBREVIATION: usize = 7;

/// Number of abbreviations asked from the object database before the
/// identifiers of all objects are sorted.
const SORTED_AFTER: usize = 512;

/// A repository opened for one command.
pub(crate) struct OpenRepo {
    /// The repository.
    pub(crate) repo: gix::Repository,
    /// Parents, times and trees of the commits read so far.
    pub(crate) commits: CommitCache,
    /// Signer that verifies signatures of commits and tags.
    pub(crate) signer: Arc<dyn Signer>,
    /// Settings of signature verification for this repository.
    pub(crate) verify: VerifyConfig,
    workdir: Option<PathBuf>,
    label: String,
    abbrev: OnceLock<usize>,
    lookups: AtomicUsize,
    sorted: OnceLock<Option<Vec<ObjectId>>>,
}

impl OpenRepo {
    /// Opens the repository that contains `path`, a directory or a file that
    /// may no longer exist.
    ///
    /// The search starts at the deepest existing directory of `path`; the
    /// configuration and environment are read as [`GitRepo::discover`] reads
    /// them, and no program is started.
    pub(crate) fn discover(path: &str, ctx: &ExecCtx) -> Result<Self, Error> {
        let start = existing_directory(Path::new(path)).ok_or_else(|| not_repository(path))?;
        let mut repo = GitRepo::discover(&start, ctx)?.gix().clone();
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
        let workdir = repo
            .workdir()
            .map(|dir| gix::path::realpath(dir).unwrap_or_else(|_| dir.to_path_buf()));
        let label = workdir
            .as_deref()
            .map_or_else(|| display_path(repo.git_dir()), display_path);
        let commits = CommitCache::new(&repo);
        let mut verify = VerifyConfig::resolve(repo.config_snapshot().plumbing(), ctx);
        if let Some(dir) = workdir.as_deref() {
            verify = verify.in_work_dir(dir);
        }
        let signer: Arc<dyn Signer> = ctx
            .extensions()
            .get::<dyn Signer>()
            .unwrap_or_else(|| Arc::new(BuiltinSigner));
        Ok(Self {
            repo,
            commits,
            signer,
            verify,
            workdir,
            label,
            abbrev: OnceLock::new(),
            lookups: AtomicUsize::new(0),
            sorted: OnceLock::new(),
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
                format!(
                    "the repository `{}` has no working tree, so `{path}` cannot be resolved",
                    self.label
                ),
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
                let bytes = gix::path::into_bstr(Path::new(name)).map_err(|_| {
                    Error::new(
                        ErrorCode::Encoding,
                        format!("`{path}` is not valid Unicode, give a path in Unicode"),
                    )
                })?;
                out.extend_from_slice(bytes.as_ref());
            }
        }
        Ok(out)
    }

    /// Absolute path of a repository-relative path, in the canonical form of
    /// the language.
    pub(crate) fn absolute(&self, relative: &BStr) -> String {
        let root = self
            .workdir
            .as_deref()
            .unwrap_or_else(|| self.repo.git_dir());
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
        Error::new(
            ErrorCode::Io,
            format!("cannot read {what} in `{}`: {err}", self.label),
        )
        .with_next_step(format!("git.status {}", quote(&self.label)))
    }

    /// Error for a revision that does not name a commit.
    pub(crate) fn revision_error(&self, spec: &str, problem: RevisionProblem) -> Error {
        let message = match problem {
            RevisionProblem::NotFound => format!("unknown revision `{spec}`"),
            RevisionProblem::Ambiguous => format!(
                "the revision `{spec}` is ambiguous, give more characters of the identifier"
            ),
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
        self.commits
            .info(&self.repo, commit)
            .map(|info| info.tree)
            .map_err(|err| self.read_error(format_args!("commit {commit}"), err))
    }

    /// Content of a blob.
    pub(crate) fn blob(&self, id: ObjectId) -> Result<Vec<u8>, Error> {
        self.repo
            .find_object(id)
            .map(|object| object.detach().data)
            .map_err(|err| self.read_error(format_args!("object {id}"), err))
    }

    /// Abbreviated identifier, unique in the repository.
    ///
    /// The first lookups ask the object database. After `SORTED_AFTER`
    /// lookups the identifiers of all objects are sorted once and each
    /// abbreviation is extended past the longest prefix shared with a
    /// neighbour.
    pub(crate) fn short_id(&self, id: ObjectId) -> String {
        let length = *self.abbrev.get_or_init(|| self.abbreviation_length());
        if self.lookups.fetch_add(1, Ordering::Relaxed) >= SORTED_AFTER
            && let Some(sorted) = self.sorted.get_or_init(|| self.sorted_ids()).as_ref()
            && let Ok(position) = sorted.binary_search(&id)
        {
            let shared = [position.checked_sub(1), position.checked_add(1)]
                .into_iter()
                .flatten()
                .filter_map(|neighbour| sorted.get(neighbour))
                .map(|other| shared_hex_digits(&id, other))
                .max()
                .unwrap_or(0);
            return id.to_hex_with_len((shared + 1).max(length)).to_string();
        }
        gix::odb::store::prefix::disambiguate::Candidate::new(id, length)
            .ok()
            .and_then(|candidate| {
                self.repo
                    .objects
                    .disambiguate_prefix(candidate)
                    .ok()
                    .flatten()
            })
            .map_or_else(
                || id.attach(&self.repo).shorten_or_id().to_string(),
                |prefix| prefix.to_string(),
            )
    }

    fn sorted_ids(&self) -> Option<Vec<ObjectId>> {
        let mut ids: Vec<ObjectId> = self
            .repo
            .objects
            .iter()
            .ok()?
            .collect::<Result<_, _>>()
            .ok()?;
        ids.sort_unstable();
        ids.dedup();
        Some(ids)
    }

    /// Length of abbreviated identifiers before they are extended to be
    /// unique: `core.abbrev`, or from the number of objects as `git` chooses
    /// it.
    fn abbreviation_length(&self) -> usize {
        let configured = self
            .repo
            .config_snapshot()
            .integer("core.abbrev")
            .and_then(|length| usize::try_from(length).ok())
            .filter(|length| (MIN_ABBREVIATION..=MAX_ABBREVIATION).contains(length));
        configured.unwrap_or_else(|| {
            let objects = self.repo.objects.packed_object_count().unwrap_or(0);
            let bits = u64::BITS - objects.leading_zeros();
            usize::try_from(bits.div_ceil(2))
                .map_or(AUTO_ABBREVIATION, |length| length.max(AUTO_ABBREVIATION))
        })
    }
}

/// Leading hexadecimal digits that two identifiers share.
fn shared_hex_digits(a: &ObjectId, b: &ObjectId) -> usize {
    let mut shared = 0;
    for (x, y) in a.as_bytes().iter().zip(b.as_bytes()) {
        if x == y {
            shared += 2;
        } else {
            if x >> 4 == y >> 4 {
                shared += 1;
            }
            break;
        }
    }
    shared
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
        } else if lower.contains("unconsumed")
            || lower.contains("syntax")
            || lower.contains("parse")
        {
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
    Error::new(
        ErrorCode::GitNotRepository,
        format!("`{path}` is not inside a git repository"),
    )
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
    text.strip_prefix("//?/")
        .map_or(text.clone(), str::to_owned)
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
    use crate::revwalk::fixture::{Fixture, exec_ctx};

    #[test]
    fn quote_escapes_quotes_and_backslashes() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn display_path_uses_forward_slashes_without_verbatim_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\repo")), "C:/repo");
        assert_eq!(
            display_path(Path::new("/home/user/repo")),
            "/home/user/repo"
        );
    }

    #[test]
    fn classifies_revision_failures() {
        assert_eq!(
            RevisionProblem::classify("The short hash 12 is ambiguous"),
            RevisionProblem::Ambiguous
        );
        assert_eq!(
            RevisionProblem::classify("unconsumed input: ~x"),
            RevisionProblem::InvalidSyntax
        );
        assert_eq!(
            RevisionProblem::classify("reference not found"),
            RevisionProblem::NotFound
        );
    }

    #[test]
    fn rejects_directory_without_repository() {
        let dir = tempfile::tempdir().unwrap();
        let path = display_path(dir.path());
        let error = OpenRepo::discover(&path, &exec_ctx(None)).err().unwrap();
        assert_eq!(error.code(), ErrorCode::GitNotRepository);
    }

    #[test]
    fn resolves_paths_relative_to_the_working_tree() {
        let fixture = Fixture::new();
        let repo = fixture.open();
        let root = fixture.root();
        assert_eq!(repo.relative(&root).unwrap(), "");
        assert_eq!(
            repo.relative(&format!("{root}/src/gone/file.rs")).unwrap(),
            "src/gone/file.rs"
        );
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

    #[test]
    fn abbreviations_agree_before_and_after_the_identifiers_are_sorted() {
        let mut fixture = Fixture::new();
        let ids: Vec<ObjectId> = (0..40)
            .map(|number| {
                fixture.commit(
                    &format!("commit {number}"),
                    &[("a.txt", Some(&format!("{number}\n")))],
                )
            })
            .collect();
        let repo = fixture.open();
        let before: Vec<String> = ids.iter().map(|id| repo.short_id(*id)).collect();
        for _ in 0..SORTED_AFTER {
            repo.short_id(ids[0]);
        }
        let after: Vec<String> = ids.iter().map(|id| repo.short_id(*id)).collect();
        assert!(repo.sorted.get().is_some_and(Option::is_some));
        assert_eq!(before, after);
        assert!(before.iter().all(|short| short.len() >= MIN_ABBREVIATION));
    }

    #[test]
    fn counts_the_digits_that_identifiers_share() {
        let id = |text: &str| ObjectId::from_hex(text.as_bytes()).unwrap();
        let base = id("abcdef0000000000000000000000000000000000");
        assert_eq!(
            shared_hex_digits(&base, &id("abcdef1000000000000000000000000000000000")),
            6
        );
        assert_eq!(
            shared_hex_digits(&base, &id("abcd0f0000000000000000000000000000000000")),
            4
        );
        assert_eq!(
            shared_hex_digits(&base, &id("abc9ef0000000000000000000000000000000000")),
            3
        );
    }
}
