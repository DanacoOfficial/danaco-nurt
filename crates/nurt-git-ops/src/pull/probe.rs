// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Read-only view of a repository for the checks of a pull.
//!
//! The repository is opened through [`GitRepo`]: no program runs, and the
//! system and global configuration come from the execution context.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use gix::ObjectId;
use gix::bstr::ByteSlice;
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::stage::ExecCtx;
use nurt_git::repo::GitRepo;

use super::settings::ConfigSource;

/// Mode of a submodule entry; `git pull` leaves submodules out of the checks
/// for uncommitted changes.
const GITLINK_MODE: u32 = 0o160_000;

/// State of `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HeadState {
    /// `HEAD` names a branch with commits.
    Branch {
        /// Short branch name.
        name: String,
        /// Commit of the branch.
        id: ObjectId,
    },
    /// `HEAD` names a branch without commits.
    Unborn {
        /// Short branch name.
        name: String,
    },
    /// `HEAD` points directly to a commit.
    Detached {
        /// Commit.
        id: ObjectId,
    },
}

impl HeadState {
    /// Short name of the current branch.
    pub(crate) fn branch(&self) -> Option<&str> {
        match self {
            Self::Branch { name, .. } | Self::Unborn { name } => Some(name),
            Self::Detached { .. } => None,
        }
    }

    /// Commit of `HEAD`.
    pub(crate) fn id(&self) -> Option<ObjectId> {
        match self {
            Self::Branch { id, .. } | Self::Detached { id } => Some(*id),
            Self::Unborn { .. } => None,
        }
    }
}

/// Repository with a working tree, opened for reading.
pub(crate) struct Repo {
    repo: GitRepo,
    root: PathBuf,
}

fn read_error(what: &str, error: impl Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read {what} of the repository: {error}"),
    )
    .with_stage(Stage::Read)
}

/// Opens the repository that contains `path`.
///
/// # Errors
///
/// The errors of [`GitRepo::discover`], and `E_GIT_UNSUPPORTED` for a bare
/// repository.
pub(crate) fn open(path: &Path, ctx: &ExecCtx) -> Result<Repo> {
    let repo = GitRepo::discover(path, ctx)?;
    let Some(root) = repo.workdir().map(Path::to_path_buf) else {
        return Err(Error::new(
            ErrorCode::GitUnsupported,
            "a bare repository has no working tree to pull into; give a repository with a working tree",
        )
        .with_stage(Stage::Read)
        .with_problem("bare_repository"));
    };
    Ok(Repo { repo, root })
}

impl ConfigSource for Repo {
    fn get(&self, key: &str) -> Option<String> {
        let config = self.repo.config();
        let truth = config.boolean(key);
        match config.string(key) {
            Some(value) if value.is_empty() && truth == Some(true) => Some("true".to_owned()),
            Some(value) => Some(value.to_str_lossy().into_owned()),
            None if truth == Some(true) => Some("true".to_owned()),
            None => None,
        }
    }

    fn get_all(&self, key: &str) -> Vec<String> {
        self.repo
            .config()
            .plumbing()
            .strings(key)
            .unwrap_or_default()
            .iter()
            .map(|value| value.to_str_lossy().into_owned())
            .collect()
    }
}

impl Repo {
    /// The opened repository.
    pub(crate) fn git(&self) -> &GitRepo {
        &self.repo
    }

    fn gix(&self) -> &gix::Repository {
        self.repo.gix()
    }

    /// Root of the working tree.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// State of `HEAD`.
    pub(crate) fn head(&self) -> Result<HeadState> {
        let head = self
            .gix()
            .head()
            .map_err(|error| read_error("HEAD", error))?;
        let branch = head
            .referent_name()
            .map(|name| name.shorten().to_str_lossy().into_owned());
        match (branch, head.id()) {
            (Some(name), Some(id)) => Ok(HeadState::Branch {
                name,
                id: id.detach(),
            }),
            (Some(name), None) => Ok(HeadState::Unborn { name }),
            (None, Some(id)) => Ok(HeadState::Detached { id: id.detach() }),
            (None, None) => Err(read_error("HEAD", "HEAD names no commit and no branch")),
        }
    }

    fn git_file_exists(&self, name: &str) -> bool {
        self.repo.git_dir().join(name).exists()
    }

    /// `MERGE_HEAD` exists.
    pub(crate) fn merge_in_progress(&self) -> bool {
        self.git_file_exists("MERGE_HEAD")
    }

    /// A rebase directory exists.
    pub(crate) fn rebase_in_progress(&self) -> bool {
        self.git_file_exists("rebase-merge") || self.git_file_exists("rebase-apply")
    }

    fn index(&self) -> Result<gix::index::File> {
        self.repo.index_file()
    }

    /// Paths with unmerged index entries, sorted and without repetitions.
    pub(crate) fn unmerged_paths(&self) -> Result<Vec<String>> {
        let index = self.index()?;
        let mut paths: Vec<String> = index
            .entries()
            .iter()
            .filter(|entry| entry.stage() != gix::index::entry::Stage::Unconflicted)
            .map(|entry| entry.path(&index).to_str_lossy().into_owned())
            .collect();
        paths.dedup();
        Ok(paths)
    }

    fn head_entries(&self, head: Option<ObjectId>) -> Result<BTreeMap<String, (u32, ObjectId)>> {
        let mut entries = BTreeMap::new();
        let Some(head) = head else {
            return Ok(entries);
        };
        let tree = self
            .gix()
            .find_commit(head)
            .and_then(|commit| commit.tree())
            .map_err(|error| read_error("the tree of HEAD", error))?;
        let mut recorder = gix::traverse::tree::Recorder::default();
        tree.traverse()
            .breadthfirst(&mut recorder)
            .map_err(|error| read_error("the tree of HEAD", error))?;
        for record in recorder.records {
            if !record.mode.is_tree() && u32::from(record.mode.value()) != GITLINK_MODE {
                let path = record.filepath.to_str_lossy().into_owned();
                entries.insert(path, (u32::from(record.mode.value()), record.oid));
            }
        }
        Ok(entries)
    }

    /// Paths whose index entry differs from the tree of `head`.
    pub(crate) fn staged_paths(&self, head: Option<ObjectId>) -> Result<Vec<String>> {
        let mut tree = self.head_entries(head)?;
        let index = self.index()?;
        let mut paths = Vec::new();
        for entry in index.entries() {
            if entry.mode.bits() == GITLINK_MODE {
                continue;
            }
            let path = entry.path(&index).to_str_lossy().into_owned();
            let current = (entry.mode.bits(), entry.id);
            if tree.remove(&path) != Some(current) {
                paths.push(path);
            }
        }
        paths.extend(tree.into_keys());
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// Paths whose working tree file differs from the index entry.
    ///
    /// A file whose size and modification time match the index entry and
    /// that was modified before the index was written is unchanged; any
    /// other file is compared by its blob identifier, as `git` does after
    /// refreshing the index.
    pub(crate) fn unstaged_paths(&self) -> Result<Vec<String>> {
        let index = self.index()?;
        let index_time = std::fs::metadata(self.repo.index_path())
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
        let filemode = self.get("core.filemode").as_deref() != Some("false");
        let mut paths = Vec::new();
        for entry in index.entries() {
            let skip = entry
                .flags
                .contains(gix::index::entry::Flags::SKIP_WORKTREE)
                || entry.mode.is_sparse()
                || entry.mode.bits() == GITLINK_MODE
                || entry.stage() != gix::index::entry::Stage::Unconflicted;
            if skip {
                continue;
            }
            let raw = entry.path(&index);
            if self.worktree_differs(entry, raw, index_time, filemode)? {
                paths.push(raw.to_str_lossy().into_owned());
            }
        }
        Ok(paths)
    }

    /// Whether the file of `entry` differs from it; the file is found by the
    /// bytes of `raw`, which need not be UTF-8.
    fn worktree_differs(
        &self,
        entry: &gix::index::Entry,
        raw: &gix::bstr::BStr,
        index_time: Option<std::time::Duration>,
        filemode: bool,
    ) -> Result<bool> {
        let shown = raw.to_str_lossy();
        let path: &str = &shown;
        let relative = gix::path::from_bstr(raw).map_err(|error| read_error(path, error))?;
        let file = self.root.join(relative);
        let Ok(metadata) = std::fs::symlink_metadata(&file) else {
            return Ok(true);
        };
        let is_link = entry.mode == gix::index::entry::Mode::SYMLINK;
        if metadata.file_type().is_symlink() != is_link || metadata.is_dir() {
            return Ok(true);
        }
        let entry_executable = entry.mode == gix::index::entry::Mode::FILE_EXECUTABLE;
        if !is_link && filemode && executable(&metadata) != entry_executable {
            return Ok(true);
        }
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
        if let (Some(modified), Some(index_time)) = (modified, index_time) {
            let same_stat = u64::from(entry.stat.size) == (metadata.len() & u64::from(u32::MAX))
                && u64::from(entry.stat.mtime.secs) == modified.as_secs()
                && entry.stat.mtime.nsecs == modified.subsec_nanos();
            if same_stat && modified < index_time {
                return Ok(false);
            }
        }
        let content = if is_link {
            let target = std::fs::read_link(&file).map_err(|error| read_error(path, error))?;
            gix::path::into_bstr(target)
                .map_err(|error| read_error(path, error))?
                .into_owned()
                .into()
        } else {
            std::fs::read(&file).map_err(|error| read_error(path, error))?
        };
        let id = gix::objs::compute_hash(self.gix().object_hash(), gix::objs::Kind::Blob, &content)
            .map_err(|error| read_error(path, error))?;
        Ok(id != entry.id)
    }

    /// Whether `ancestor` is reachable from `descendant` or equal to it.
    pub(crate) fn is_ancestor(&self, ancestor: ObjectId, descendant: ObjectId) -> Result<bool> {
        if ancestor == descendant {
            return Ok(true);
        }
        let base = self
            .gix()
            .merge_base(ancestor, descendant)
            .map_err(|error| read_error("the history", error))?;
        Ok(base.is_some_and(|base| base.detach() == ancestor))
    }

    /// Best merge base of two commits.
    pub(crate) fn merge_base(&self, one: ObjectId, two: ObjectId) -> Result<Option<ObjectId>> {
        self.gix()
            .merge_base(one, two)
            .map(|base| base.map(gix::Id::detach))
            .map_err(|error| read_error("the history", error))
    }

    fn reflog_ids(&self, reference: &str) -> Result<Vec<ObjectId>> {
        let path = self.repo.common_dir().join("logs").join(reference);
        let text = match std::fs::read(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(read_error("a reflog", error)),
        };
        let mut ids: Vec<ObjectId> = Vec::new();
        for line in text.lines() {
            let mut fields = line.split_str(" ");
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                continue;
            };
            let candidates = if ids.is_empty() {
                vec![old, new]
            } else {
                vec![new]
            };
            for hex in candidates {
                if let Ok(id) = ObjectId::from_hex(hex)
                    && !id.is_null()
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }

    /// Fork point of `head` with the remote-tracking reference, as
    /// `git merge-base --fork-point` computes it: the single merge base of
    /// `head` and the values recorded in the reflog of the reference, when it
    /// is one of those values.
    pub(crate) fn fork_point(&self, reference: &str, head: ObjectId) -> Result<Option<ObjectId>> {
        let mut candidates = self.reflog_ids(reference)?;
        if candidates.is_empty() {
            match self.gix().try_find_reference(reference) {
                Ok(Some(mut found)) => {
                    if let Ok(id) = found.peel_to_id() {
                        candidates.push(id.detach());
                    }
                }
                Ok(None) => return Ok(None),
                Err(error) => return Err(read_error("a reference", error)),
            }
        }
        let bases = self
            .gix()
            .merge_bases_many(head, &candidates)
            .map_err(|error| read_error("the history", error))?;
        match bases.as_slice() {
            [base] if candidates.contains(&base.detach()) => Ok(Some(base.detach())),
            _ => Ok(None),
        }
    }
}

#[cfg(unix)]
fn executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    const EXECUTE_BY_OWNER: u32 = 0o100;
    metadata.permissions().mode() & EXECUTE_BY_OWNER != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use gix::objs::tree::{Entry, EntryKind};
    use nurt_contracts::journal::MutationTx;
    use nurt_git::apply::testjournal::RecordingJournal;
    use nurt_git::apply::{ApplyContext, apply_plan, plan_new_tree};
    use nurt_git::testutil::test_ctx;

    use super::*;

    /// Repository built with `gix` alone: commits, references and a working
    /// tree checked out through the working tree engine.
    pub(crate) struct Fixture {
        pub(crate) dir: tempfile::TempDir,
        pub(crate) repo: gix::Repository,
        time: i64,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let repo = gix::init(dir.path()).unwrap();
            fs::write(repo.git_dir().join("HEAD"), "ref: refs/heads/main\n").unwrap();
            Self {
                dir,
                repo,
                time: 1_767_268_800,
            }
        }

        pub(crate) fn root(&self) -> &Path {
            self.dir.path()
        }

        pub(crate) fn git_dir(&self) -> PathBuf {
            self.repo.git_dir().to_path_buf()
        }

        pub(crate) fn tree(&self, files: &[(&str, &str)]) -> ObjectId {
            let mut entries: Vec<Entry> = files
                .iter()
                .map(|(name, content)| Entry {
                    mode: EntryKind::Blob.into(),
                    filename: (*name).into(),
                    oid: self.repo.write_blob(content.as_bytes()).unwrap().detach(),
                })
                .collect();
            entries.sort_by(|a, b| a.filename.cmp(&b.filename));
            self.repo
                .write_object(&gix::objs::Tree { entries })
                .unwrap()
                .detach()
        }

        pub(crate) fn commit(
            &mut self,
            files: &[(&str, &str)],
            parents: &[ObjectId],
            message: &str,
        ) -> ObjectId {
            self.time += 60;
            let signature = gix::actor::Signature {
                name: "A".into(),
                email: "a@example.com".into(),
                time: gix::date::Time::new(self.time, 0),
            };
            let commit = gix::objs::Commit {
                tree: self.tree(files),
                parents: parents.iter().copied().collect(),
                author: signature.clone(),
                committer: signature,
                encoding: None,
                message: format!("{message}\n").into(),
                extra_headers: Vec::new(),
            };
            self.repo.write_object(&commit).unwrap().detach()
        }

        pub(crate) fn set_ref(&self, name: &str, id: ObjectId) {
            let path = self.repo.git_dir().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("{id}\n")).unwrap();
        }

        pub(crate) fn append_reflog(&self, name: &str, old: ObjectId, new: ObjectId) {
            let path = self.repo.git_dir().join("logs").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut text = fs::read_to_string(&path).unwrap_or_default();
            text.push_str(&format!(
                "{old} {new} A <a@example.com> 1767268800 +0000\tfetch: test\n"
            ));
            fs::write(path, text).unwrap();
        }

        /// Appends `text` to the repository configuration.
        pub(crate) fn config(&self, text: &str) {
            let path = self.repo.git_dir().join("config");
            let mut config = fs::read_to_string(&path).unwrap();
            config.push_str(text);
            fs::write(path, config).unwrap();
        }

        /// Writes the index and the working tree of `commit` into the
        /// repository without an index.
        pub(crate) fn checkout(&self, commit: ObjectId) {
            let tree = self.repo.find_commit(commit).unwrap().tree_id().unwrap();
            let ctx = ApplyContext::open(&self.repo).unwrap();
            let plan = plan_new_tree(&ctx, &tree).unwrap();
            let journal = RecordingJournal::new();
            let mut tx = journal.begin();
            apply_plan(&mut tx, &ctx, plan).unwrap();
            Box::new(tx).commit().unwrap();
        }

        pub(crate) fn open(&self) -> Repo {
            open(self.root(), &test_ctx()).unwrap()
        }
    }

    #[test]
    fn reports_the_branch_and_its_commit() {
        let mut fixture = Fixture::new();
        let repo = fixture.open();
        assert_eq!(
            repo.head().unwrap(),
            HeadState::Unborn {
                name: "main".into()
            }
        );
        let first = fixture.commit(&[("a.txt", "a\n")], &[], "init");
        fixture.set_ref("refs/heads/main", first);
        let head = fixture.open().head().unwrap();
        assert_eq!(
            head,
            HeadState::Branch {
                name: "main".into(),
                id: first
            }
        );
        assert_eq!(head.branch(), Some("main"));
        assert_eq!(head.id(), Some(first));
        fs::write(fixture.git_dir().join("HEAD"), format!("{first}\n")).unwrap();
        let detached = fixture.open().head().unwrap();
        assert_eq!(detached, HeadState::Detached { id: first });
        assert_eq!(detached.branch(), None);
    }

    #[test]
    fn rejects_paths_outside_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let error = open(dir.path(), &test_ctx()).err().unwrap();
        assert_eq!(error.code, ErrorCode::GitNotRepository);
    }

    #[test]
    fn rejects_a_bare_repository() {
        let dir = tempfile::tempdir().unwrap();
        gix::init_bare(dir.path()).unwrap();
        let error = open(dir.path(), &test_ctx()).err().unwrap();
        assert_eq!(error.code, ErrorCode::GitUnsupported);
        assert_eq!(error.problem.as_deref(), Some("bare_repository"));
    }

    #[test]
    fn reads_configuration_values() {
        let fixture = Fixture::new();
        fixture.config("[pull]\n\trebase\n\tff = \n[branch \"main\"]\n\tmerge = refs/heads/a\n\tmerge = refs/heads/b\n[rebase]\n\tautoStash = yes\n");
        let repo = fixture.open();
        assert_eq!(repo.get("pull.rebase").as_deref(), Some("true"));
        assert_eq!(repo.get("pull.ff").as_deref(), Some(""));
        assert_eq!(repo.get("rebase.autoStash").as_deref(), Some("yes"));
        assert_eq!(repo.get("pull.missing"), None);
        assert_eq!(
            repo.get_all("branch.main.merge"),
            ["refs/heads/a", "refs/heads/b"]
        );
        assert!(repo.get_all("branch.other.merge").is_empty());
        assert_eq!(
            fs::canonicalize(repo.root()).unwrap(),
            fs::canonicalize(fixture.root()).unwrap()
        );
        assert!(repo.git().workdir().is_some());
    }

    #[test]
    fn detects_unfinished_operations() {
        let fixture = Fixture::new();
        let repo = fixture.open();
        assert!(!repo.merge_in_progress());
        assert!(!repo.rebase_in_progress());
        fs::write(fixture.git_dir().join("MERGE_HEAD"), "x\n").unwrap();
        fs::create_dir(fixture.git_dir().join("rebase-merge")).unwrap();
        assert!(repo.merge_in_progress());
        assert!(repo.rebase_in_progress());
    }

    #[test]
    fn finds_staged_and_unstaged_changes() {
        let mut fixture = Fixture::new();
        let files = [("a.txt", "a\n"), ("b.txt", "b\n")];
        let first = fixture.commit(&files, &[], "init");
        fixture.set_ref("refs/heads/main", first);
        fixture.checkout(first);
        let repo = fixture.open();
        assert!(repo.staged_paths(Some(first)).unwrap().is_empty());
        assert!(repo.unstaged_paths().unwrap().is_empty());
        assert!(repo.unmerged_paths().unwrap().is_empty());
        assert_eq!(repo.staged_paths(None).unwrap(), ["a.txt", "b.txt"]);

        fs::write(fixture.root().join("a.txt"), "changed\n").unwrap();
        fs::remove_file(fixture.root().join("b.txt")).unwrap();
        fs::write(fixture.root().join("untracked.txt"), "u\n").unwrap();
        assert_eq!(repo.unstaged_paths().unwrap(), ["a.txt", "b.txt"]);

        let second = fixture.commit(&[("a.txt", "changed\n")], &[first], "second");
        assert_eq!(repo.staged_paths(Some(second)).unwrap(), ["a.txt", "b.txt"]);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture adds conflict stages by writing the index directly, outside a transaction"
    )]
    fn lists_unmerged_paths_once() {
        let mut fixture = Fixture::new();
        let first = fixture.commit(&[("a.txt", "a\n")], &[], "init");
        fixture.checkout(first);
        let mut index = fixture.repo.open_index().unwrap();
        let blob = fixture.repo.write_blob(b"theirs\n").unwrap().detach();
        for stage in [2_u32, 3] {
            let flags = gix::index::entry::Flags::from_bits_retain(stage << 12);
            index.dangerously_push_entry(
                Default::default(),
                blob,
                flags,
                gix::index::entry::Mode::FILE,
                "c.txt".into(),
            );
        }
        index.sort_entries();
        index.write(Default::default(), Default::default()).unwrap();
        assert_eq!(fixture.open().unmerged_paths().unwrap(), ["c.txt"]);
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture adds a submodule entry by writing the index directly, outside a transaction"
    )]
    fn leaves_submodules_out_of_the_checks_for_changes() {
        let mut fixture = Fixture::new();
        let first = fixture.commit(&[("a.txt", "a\n")], &[], "init");
        fixture.set_ref("refs/heads/main", first);
        fixture.checkout(first);
        let mut index = fixture.repo.open_index().unwrap();
        index.dangerously_push_entry(
            Default::default(),
            first,
            gix::index::entry::Flags::empty(),
            gix::index::entry::Mode::COMMIT,
            "sub".into(),
        );
        index.sort_entries();
        index.write(Default::default(), Default::default()).unwrap();
        fs::create_dir(fixture.root().join("sub")).unwrap();
        let repo = fixture.open();
        assert!(repo.unstaged_paths().unwrap().is_empty());
        assert!(repo.staged_paths(Some(first)).unwrap().is_empty());
    }

    #[test]
    #[cfg(unix)]
    #[allow(
        clippy::disallowed_methods,
        reason = "the fixture adds an entry with a raw path by writing the index directly, outside a transaction"
    )]
    fn finds_unstaged_changes_in_a_path_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let mut fixture = Fixture::new();
        let first = fixture.commit(&[("a.txt", "a\n")], &[], "init");
        fixture.set_ref("refs/heads/main", first);
        fixture.checkout(first);
        let name: &[u8] = b"bad\xff.txt";
        let file = fixture.root().join(std::ffi::OsStr::from_bytes(name));
        fs::write(&file, "one\n").unwrap();
        let blob = fixture.repo.write_blob(b"one\n").unwrap().detach();
        let mut index = fixture.repo.open_index().unwrap();
        index.dangerously_push_entry(
            Default::default(),
            blob,
            gix::index::entry::Flags::empty(),
            gix::index::entry::Mode::FILE,
            name.into(),
        );
        index.sort_entries();
        index.write(Default::default()).unwrap();
        assert!(fixture.open().unstaged_paths().unwrap().is_empty());
        fs::write(&file, "two\n").unwrap();
        assert_eq!(
            fixture.open().unstaged_paths().unwrap(),
            ["bad\u{fffd}.txt"]
        );
    }

    #[test]
    fn computes_ancestry_and_merge_bases() {
        let mut fixture = Fixture::new();
        let base = fixture.commit(&[("a.txt", "a\n")], &[], "base");
        let left = fixture.commit(&[("a.txt", "l\n")], &[base], "left");
        let right = fixture.commit(&[("a.txt", "r\n")], &[base], "right");
        let repo = fixture.open();
        assert!(repo.is_ancestor(base, left).unwrap());
        assert!(repo.is_ancestor(left, left).unwrap());
        assert!(!repo.is_ancestor(left, right).unwrap());
        assert_eq!(repo.merge_base(left, right).unwrap(), Some(base));
    }

    #[test]
    fn finds_the_fork_point_of_a_rewritten_upstream() {
        let mut fixture = Fixture::new();
        let base = fixture.commit(&[("a.txt", "a\n")], &[], "base");
        let old_upstream = fixture.commit(&[("a.txt", "b\n")], &[base], "second");
        let local = fixture.commit(
            &[("a.txt", "b\n"), ("l.txt", "l\n")],
            &[old_upstream],
            "local",
        );
        let rewritten = fixture.commit(&[("a.txt", "B\n")], &[base], "second rewritten");
        let tracking = "refs/remotes/origin/main";
        fixture.set_ref(tracking, old_upstream);
        let repo = fixture.open();
        assert_eq!(
            repo.fork_point(tracking, local).unwrap(),
            Some(old_upstream)
        );
        fixture.append_reflog(tracking, ObjectId::null(gix::hash::Kind::Sha1), base);
        fixture.append_reflog(tracking, base, old_upstream);
        fixture.append_reflog(tracking, old_upstream, rewritten);
        assert_eq!(
            repo.fork_point(tracking, local).unwrap(),
            Some(old_upstream)
        );
        assert_eq!(
            repo.fork_point("refs/remotes/origin/none", local).unwrap(),
            None
        );
        let unrelated = fixture.commit(&[("z.txt", "z\n")], &[], "unrelated");
        assert_eq!(repo.fork_point(tracking, unrelated).unwrap(), None);
    }
}
