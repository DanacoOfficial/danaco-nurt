// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository access used by `git.commit` and `git.review`: opening without
//! external programs, layered configuration, `HEAD`, the index, trees, the
//! working tree with content filters, untracked files and path selection.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use bstr::{BStr, BString, ByteSlice, ByteVec};
use gix::ObjectId;
use gix::objs::tree::{EntryKind, EntryMode};
use nurt_contracts::env::Host;
use nurt_contracts::error::{Error, ErrorCode, Result};

/// Bytes inspected for a NUL byte when deciding whether content is binary,
/// as `git` does.
pub(crate) const BINARY_PROBE_BYTES: usize = 8000;

/// Value that switches the global configuration off when given in
/// `GIT_CONFIG_GLOBAL`.
const NULL_DEVICE: &str = "/dev/null";

/// Builds an error with a code, a problem identifier and a message.
pub(crate) fn fail(code: ErrorCode, problem: &str, message: impl Into<String>) -> Error {
    Error::new(code, message).with_problem(problem)
}

/// Maps an input or output error on `path` to the error codes of the file
/// layer.
pub(crate) fn io_error(path: &Path, error: &std::io::Error) -> Error {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::Permission,
        _ => ErrorCode::Io,
    };
    Error::new(code, format!("cannot access {}: {error}", path.display()))
}

/// Maps a failure of the repository library to `E_IO`.
pub(crate) fn lib_error(what: &str, error: impl Display) -> Error {
    Error::new(ErrorCode::Io, format!("{what}: {error}"))
}

/// Configuration of a repository in the order of precedence of `git`:
/// `GIT_CONFIG_COUNT` variables, worktree configuration, repository
/// configuration, `~/.gitconfig`, the XDG configuration file and the system
/// file.
pub(crate) struct ConfigView {
    overrides: Vec<(String, String)>,
    files: Vec<gix::config::File<'static>>,
}

impl ConfigView {
    fn load(git_dir: &Path, common_dir: &Path, host: &dyn Host) -> Result<Self> {
        let overrides = env_overrides(host);
        let mut lower = Vec::new();
        for (path, source) in global_files(host) {
            if let Some(file) = read_config(&path, source)? {
                lower.push(file);
            }
        }
        let mut files = Vec::new();
        let local = read_config(&common_dir.join("config"), gix::config::Source::Local)?;
        let worktree_config = local
            .as_ref()
            .and_then(|file| file.string("extensions.worktreeConfig"))
            .is_some_and(|value| parse_bool(&value.to_str_lossy()) == Some(true));
        if worktree_config {
            let path = git_dir.join("config.worktree");
            if let Some(file) = read_config(&path, gix::config::Source::Worktree)? {
                files.push(file);
            }
        }
        files.extend(local);
        lower.reverse();
        files.extend(lower);
        Ok(Self { overrides, files })
    }

    /// Value of a key, written `section.key` or `section.subsection.key`.
    pub(crate) fn get(&self, key: &str) -> Option<String> {
        if let Some((_, value)) = self.overrides.iter().rev().find(|(name, _)| same_key(name, key)) {
            return Some(value.clone());
        }
        self.files
            .iter()
            .find_map(|file| file.string(key))
            .map(|value| value.to_str_lossy().into_owned())
    }

    /// Boolean value of a key.
    pub(crate) fn flag(&self, key: &str) -> Result<Option<bool>> {
        match self.get(key) {
            None => Ok(None),
            Some(value) => parse_bool(&value).map(Some).ok_or_else(|| {
                fail(
                    ErrorCode::Config,
                    "invalid_value",
                    format!("{key} has the value `{value}`; use true or false"),
                )
            }),
        }
    }
}

fn read_config(path: &Path, source: gix::config::Source) -> Result<Option<gix::config::File<'static>>> {
    if !path.is_file() {
        return Ok(None);
    }
    gix::config::File::from_path_no_includes(path.to_path_buf(), source)
        .map(Some)
        .map_err(|error| {
            fail(
                ErrorCode::Config,
                "unreadable_config",
                format!("cannot read the configuration file {}: {error}", path.display()),
            )
        })
}

fn host_var(host: &dyn Host, name: &str) -> Option<String> {
    host.env_var(name)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

/// Global and system configuration files, lowest precedence first.
fn global_files(host: &dyn Host) -> Vec<(PathBuf, gix::config::Source)> {
    let mut files = Vec::new();
    if cfg!(unix) && host.env_var("GIT_CONFIG_NOSYSTEM").is_none() {
        files.push((PathBuf::from("/etc/gitconfig"), gix::config::Source::System));
    }
    if let Some(global) = host_var(host, "GIT_CONFIG_GLOBAL") {
        if global != NULL_DEVICE {
            files.push((PathBuf::from(global), gix::config::Source::User));
        }
        return files;
    }
    let home = host_var(host, "HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.info().home.clone());
    let xdg = host_var(host, "XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    files.push((xdg.join("git").join("config"), gix::config::Source::Git));
    files.push((home.join(".gitconfig"), gix::config::Source::User));
    files
}

fn env_overrides(host: &dyn Host) -> Vec<(String, String)> {
    let count = host_var(host, "GIT_CONFIG_COUNT")
        .and_then(|count| count.parse::<usize>().ok())
        .unwrap_or(0);
    (0..count)
        .filter_map(|n| {
            let key = host_var(host, &format!("GIT_CONFIG_KEY_{n}"))?;
            let value = host
                .env_var(&format!("GIT_CONFIG_VALUE_{n}"))
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            Some((key, value))
        })
        .collect()
}

/// Compares configuration keys: section and variable names without regard to
/// case, the subsection exactly.
fn same_key(left: &str, right: &str) -> bool {
    let split = |key: &str| -> Option<(String, String, String)> {
        let (section, rest) = key.split_once('.')?;
        let (subsection, name) = rest.rsplit_once('.').unwrap_or(("", rest));
        Some((section.to_ascii_lowercase(), subsection.to_owned(), name.to_ascii_lowercase()))
    };
    split(left).is_some_and(|left| Some(left) == split(right))
}

/// Parses a boolean of the configuration.
pub(crate) fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" | "" => Some(false),
        _ => None,
    }
}

/// Current commit and the file that records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Head {
    /// Full name of the checked out branch, `None` when `HEAD` is detached.
    pub(crate) branch: Option<String>,
    /// Current commit, `None` on an unborn branch.
    pub(crate) id: Option<ObjectId>,
    /// File written to move the branch: the loose reference, or `HEAD`
    /// itself when detached.
    pub(crate) ref_file: PathBuf,
    /// Content of that file before the commit, when it exists.
    pub(crate) ref_content: Option<Vec<u8>>,
}

impl Head {
    /// Short name of the branch, for example `main`.
    pub(crate) fn short_branch(&self) -> Option<&str> {
        self.branch.as_deref().map(|name| name.strip_prefix("refs/heads/").unwrap_or(name))
    }
}

/// Index of a repository as read before the operation.
pub(crate) struct IndexSnapshot {
    /// Decoded index.
    pub(crate) file: gix::index::File,
    /// Bytes of the index file; `None` when the repository has no index.
    pub(crate) raw: Option<Vec<u8>>,
    /// Path of the index file.
    pub(crate) path: PathBuf,
}

/// File of a tree: object and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TreeFile {
    /// Object identifier.
    pub(crate) id: ObjectId,
    /// Mode.
    pub(crate) kind: EntryKind,
}

/// Files of a tree by path relative to the root, in path order.
pub(crate) type FileMap = BTreeMap<BString, TreeFile>;

/// Parsed commit object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitInfo {
    /// Tree.
    pub(crate) tree: ObjectId,
    /// Parents.
    pub(crate) parents: Vec<ObjectId>,
    /// Author header value: `Name <email> seconds offset`.
    pub(crate) author: String,
    /// Message.
    pub(crate) message: String,
}

impl CommitInfo {
    fn parse(data: &[u8]) -> Option<Self> {
        let text = String::from_utf8_lossy(data);
        let (headers, message) = text.split_once("\n\n").unwrap_or((&text, ""));
        let mut tree = None;
        let mut parents = Vec::new();
        let mut author = None;
        for line in headers.lines() {
            if let Some(value) = line.strip_prefix("tree ") {
                tree = ObjectId::from_hex(value.as_bytes()).ok();
            } else if let Some(value) = line.strip_prefix("parent ") {
                parents.push(ObjectId::from_hex(value.as_bytes()).ok()?);
            } else if let Some(value) = line.strip_prefix("author ") {
                author = Some(value.to_owned());
            }
        }
        Some(Self {
            tree: tree?,
            parents,
            author: author?,
            message: message.to_owned(),
        })
    }
}

/// Opened repository.
pub(crate) struct Repo {
    /// Library handle, opened with repository configuration only.
    pub(crate) repo: gix::Repository,
    /// Repository directory of the working tree (`.git` or
    /// `.git/worktrees/<name>`).
    pub(crate) git_dir: PathBuf,
    /// Common repository directory with objects and references.
    pub(crate) common_dir: PathBuf,
    /// Root of the working tree; `None` for a bare repository.
    pub(crate) workdir: Option<PathBuf>,
    /// Layered configuration.
    pub(crate) config: ConfigView,
}

impl Repo {
    /// Opens the repository that contains `path`, searching upwards.
    ///
    /// The library is opened with repository configuration only, so it never
    /// starts a program; user configuration is read from the environment of
    /// the host.
    pub(crate) fn open(path: &Path, host: &dyn Host) -> Result<Self> {
        let mapping = gix::sec::trust::Mapping {
            full: gix::open::Options::isolated(),
            reduced: gix::open::Options::isolated(),
        };
        let repo = gix::ThreadSafeRepository::discover_opts(path, Default::default(), mapping)
            .map_err(|error| {
                fail(
                    ErrorCode::GitNotRepository,
                    "not_a_repository",
                    format!("{} is not inside a Git repository: {error}", path.display()),
                )
            })?
            .to_thread_local();
        let git_dir = repo.git_dir().to_path_buf();
        let common_dir = repo.common_dir().to_path_buf();
        let workdir = repo.workdir().map(Path::to_path_buf);
        let config = ConfigView::load(&git_dir, &common_dir, host)?;
        Ok(Self {
            repo,
            git_dir,
            common_dir,
            workdir,
            config,
        })
    }

    /// Root shown to the user: the working tree, or the repository directory
    /// of a bare repository.
    pub(crate) fn root(&self) -> &Path {
        self.workdir.as_deref().unwrap_or(&self.git_dir)
    }

    /// Root of the working tree.
    pub(crate) fn require_workdir(&self) -> Result<&Path> {
        self.workdir.as_deref().ok_or_else(|| {
            fail(
                ErrorCode::GitUnsupported,
                "bare_repository",
                format!("{} has no working tree; use `staged: true`", self.git_dir.display()),
            )
        })
    }

    /// Hash kind of the repository.
    pub(crate) fn hash_kind(&self) -> gix::hash::Kind {
        self.repo.object_hash()
    }

    /// Reads `HEAD` and the value of the branch it names.
    pub(crate) fn head(&self) -> Result<Head> {
        let head_path = self.git_dir.join("HEAD");
        let content = std::fs::read(&head_path).map_err(|error| io_error(&head_path, &error))?;
        let text = content.to_str_lossy();
        let text = text.trim_end();
        let Some(name) = text.strip_prefix("ref:").map(str::trim) else {
            let id = parse_id(text, &head_path)?;
            return Ok(Head {
                branch: None,
                id: Some(id),
                ref_file: head_path,
                ref_content: Some(content.clone()),
            });
        };
        let ref_file = self.ref_path(name);
        let ref_content = match std::fs::read(&ref_file) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_error(&ref_file, &error)),
        };
        let id = match &ref_content {
            Some(bytes) => Some(parse_id(bytes.to_str_lossy().trim_end(), &ref_file)?),
            None => self.packed_ref(name)?,
        };
        Ok(Head {
            branch: Some(name.to_owned()),
            id,
            ref_file,
            ref_content,
        })
    }

    /// Path of the loose file of a reference.
    pub(crate) fn ref_path(&self, name: &str) -> PathBuf {
        let base = if name.starts_with("refs/") { &self.common_dir } else { &self.git_dir };
        name.split('/').fold(base.clone(), |path, part| path.join(part))
    }

    /// Path of the reflog of a reference.
    pub(crate) fn log_path(&self, name: &str) -> PathBuf {
        let base = if name.starts_with("refs/") { &self.common_dir } else { &self.git_dir };
        name.split('/').fold(base.join("logs"), |path, part| path.join(part))
    }

    fn packed_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        let path = self.common_dir.join("packed-refs");
        let content = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(&path, &error)),
        };
        for line in content.lines() {
            if line.starts_with(b"#") || line.starts_with(b"^") {
                continue;
            }
            if let Some((hex, refname)) = line.split_once_str(" ") {
                if refname == name.as_bytes() {
                    return parse_id(&hex.to_str_lossy(), &path).map(Some);
                }
            }
        }
        Ok(None)
    }

    /// Reads the index; a repository without an index has an empty one.
    pub(crate) fn read_index(&self) -> Result<IndexSnapshot> {
        let path = self.git_dir.join("index");
        let raw = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_error(&path, &error)),
        };
        let file = if raw.is_some() {
            gix::index::File::at(&path, self.hash_kind(), false, Default::default())
                .map_err(|error| lib_error("cannot read the index", error))?
        } else {
            gix::index::File::from_state(gix::index::State::new(self.hash_kind()), path.clone())
        };
        if file.is_sparse() {
            return Err(fail(
                ErrorCode::GitUnsupported,
                "sparse_index",
                "the index is sparse; expand it with `git sparse-checkout disable` before this command",
            ));
        }
        Ok(IndexSnapshot { file, raw, path })
    }

    /// Reads and parses a commit.
    pub(crate) fn commit(&self, id: ObjectId) -> Result<CommitInfo> {
        let object = self
            .repo
            .find_object(id)
            .map_err(|error| lib_error(&format!("cannot read commit {id}"), error))?;
        CommitInfo::parse(&object.data).ok_or_else(|| {
            fail(ErrorCode::GitRevision, "not_a_commit", format!("object {id} is not a valid commit"))
        })
    }

    /// Resolves a revision to a commit.
    pub(crate) fn resolve_commit(&self, spec: &str) -> Result<ObjectId> {
        let not_found = |error: &dyn Display| {
            fail(
                ErrorCode::GitRevision,
                "not_found",
                format!("revision `{spec}` does not name a commit: {error}"),
            )
        };
        if spec.starts_with('-') {
            return Err(fail(
                ErrorCode::GitRevision,
                "invalid_syntax",
                format!("revision `{spec}` may not start with `-`"),
            ));
        }
        let id = self
            .repo
            .rev_parse_single(spec)
            .map_err(|error| not_found(&error))?;
        let commit = id
            .object()
            .map_err(|error| not_found(&error))?
            .peel_to_commit()
            .map_err(|error| not_found(&error))?;
        Ok(commit.id)
    }

    /// Reads a blob.
    pub(crate) fn blob(&self, id: ObjectId) -> Result<Vec<u8>> {
        let object = self
            .repo
            .find_object(id)
            .map_err(|error| lib_error(&format!("cannot read object {id}"), error))?;
        Ok(object.data.clone())
    }

    /// Writes a blob to the object store.
    pub(crate) fn write_blob(&self, data: &[u8]) -> Result<ObjectId> {
        self.repo
            .write_blob(data)
            .map(gix::Id::detach)
            .map_err(|error| lib_error("cannot write an object", error))
    }

    /// Files of a tree.
    pub(crate) fn flatten_tree(&self, tree: ObjectId) -> Result<FileMap> {
        let mut files = FileMap::new();
        self.flatten_into(tree, BStr::new(""), &mut files)?;
        Ok(files)
    }

    fn flatten_into(&self, tree: ObjectId, prefix: &BStr, files: &mut FileMap) -> Result<()> {
        let object = self
            .repo
            .find_tree(tree)
            .map_err(|error| lib_error(&format!("cannot read tree {tree}"), error))?;
        let entries: Vec<(BString, EntryMode, ObjectId)> = object
            .decode()
            .map_err(|error| lib_error(&format!("cannot decode tree {tree}"), error))?
            .entries
            .iter()
            .map(|entry| (entry.filename.to_owned(), entry.mode, entry.oid.to_owned()))
            .collect();
        for (name, mode, id) in entries {
            let path = join_path(prefix, name.as_ref());
            if mode.is_tree() {
                self.flatten_into(id, path.as_ref(), files)?;
            } else {
                files.insert(path, TreeFile { id, kind: mode.kind() });
            }
        }
        Ok(())
    }

    /// Writes the trees of a set of files and returns the root tree.
    pub(crate) fn write_tree(&self, files: &FileMap) -> Result<ObjectId> {
        let mut root = Dir::default();
        for (path, file) in files {
            root.insert(path.as_ref(), *file);
        }
        self.write_dir(&root)
    }

    fn write_dir(&self, dir: &Dir) -> Result<ObjectId> {
        let mut entries = Vec::with_capacity(dir.entries.len());
        for (name, node) in &dir.entries {
            let (mode, oid) = match node {
                Node::File(file) => (EntryMode::from(file.kind), file.id),
                Node::Dir(sub) => (EntryMode::from(EntryKind::Tree), self.write_dir(sub)?),
            };
            entries.push(gix::objs::tree::Entry {
                mode,
                filename: name.clone(),
                oid,
            });
        }
        entries.sort();
        self.repo
            .write_object(gix::objs::Tree { entries })
            .map(gix::Id::detach)
            .map_err(|error| lib_error("cannot write a tree", error))
    }

    /// Converts a path given by the user to a path relative to the root of
    /// the working tree; `""` is the whole tree.
    pub(crate) fn repo_relative(&self, raw: &str) -> Result<BString> {
        let root = self.root();
        let given = Path::new(raw);
        let relative = if given.is_absolute() {
            given.strip_prefix(root).map_err(|_| {
                fail(
                    ErrorCode::GitNotRepository,
                    "path_outside_repo",
                    format!("{raw} is outside the repository {}", root.display()),
                )
            })?
        } else {
            given
        };
        let mut out = BString::default();
        for component in relative.components() {
            let part = match component {
                Component::CurDir => continue,
                Component::Normal(part) => part.to_string_lossy(),
                _ => {
                    return Err(fail(
                        ErrorCode::InvalidValue,
                        "invalid_path",
                        format!("path `{raw}` must stay inside the repository"),
                    ));
                }
            };
            if part.eq_ignore_ascii_case(".git") {
                return Err(fail(
                    ErrorCode::InvalidValue,
                    "invalid_path",
                    format!("path `{raw}` names the repository directory"),
                ));
            }
            if !out.is_empty() {
                out.push(b'/');
            }
            out.push_str(part.as_bytes());
        }
        Ok(out)
    }
}

fn parse_id(text: &str, file: &Path) -> Result<ObjectId> {
    ObjectId::from_hex(text.as_bytes()).map_err(|_| {
        fail(
            ErrorCode::GitUnsupported,
            "invalid_reference",
            format!("{} does not hold an object identifier", file.display()),
        )
    })
}

/// Joins a directory and a name with `/`.
pub(crate) fn join_path(prefix: &BStr, name: &BStr) -> BString {
    let mut path = BString::from(prefix);
    if !path.is_empty() {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

#[derive(Default)]
struct Dir {
    entries: BTreeMap<BString, Node>,
}

enum Node {
    File(TreeFile),
    Dir(Dir),
}

impl Dir {
    fn insert(&mut self, path: &BStr, file: TreeFile) {
        match path.split_once_str("/") {
            None => {
                self.entries.insert(path.to_owned(), Node::File(file));
            }
            Some((head, rest)) => {
                let node = self
                    .entries
                    .entry(head.into())
                    .or_insert_with(|| Node::Dir(Dir::default()));
                if let Node::File(_) = node {
                    *node = Node::Dir(Dir::default());
                }
                if let Node::Dir(dir) = node {
                    dir.insert(rest.as_bstr(), file);
                }
            }
        }
    }
}

/// Tree mode of an index entry; `None` for entries that are not files.
pub(crate) fn kind_of_index_mode(mode: gix::index::entry::Mode) -> Option<EntryKind> {
    mode.to_tree_entry_mode().map(|mode| mode.kind())
}

/// Index mode of a tree file.
pub(crate) fn index_mode_of(kind: EntryKind) -> gix::index::entry::Mode {
    match kind {
        EntryKind::BlobExecutable => gix::index::entry::Mode::FILE_EXECUTABLE,
        EntryKind::Link => gix::index::entry::Mode::SYMLINK,
        EntryKind::Commit => gix::index::entry::Mode::COMMIT,
        EntryKind::Tree => gix::index::entry::Mode::DIR,
        EntryKind::Blob => gix::index::entry::Mode::FILE,
    }
}

/// Files of the index at stage 0, without entries marked intent-to-add;
/// returns the paths with conflict stages as the error list.
pub(crate) fn index_files(state: &gix::index::State) -> std::result::Result<FileMap, Vec<String>> {
    let mut files = FileMap::new();
    let mut unmerged: Vec<String> = Vec::new();
    for entry in state.entries() {
        let path = entry.path(state);
        if entry.stage_raw() != 0 {
            let path = path.to_str_lossy().into_owned();
            if unmerged.last() != Some(&path) {
                unmerged.push(path);
            }
            continue;
        }
        if entry.flags.contains(gix::index::entry::Flags::INTENT_TO_ADD) {
            continue;
        }
        if let Some(kind) = kind_of_index_mode(entry.mode) {
            files.insert(path.to_owned(), TreeFile { id: entry.id, kind });
        }
    }
    if unmerged.is_empty() { Ok(files) } else { Err(unmerged) }
}

/// Serializes an index for writing; the cached trees are dropped because
/// they are written without being recomputed.
pub(crate) fn encode_index(file: &mut gix::index::File) -> Result<Vec<u8>> {
    file.remove_tree();
    let mut bytes = Vec::new();
    file.write_to(&mut bytes, gix::index::write::Options::default())
        .map_err(|error| lib_error("cannot encode the index", error))?;
    Ok(bytes)
}

/// Content of a working tree file, converted for storage.
pub(crate) struct WorkFile {
    /// Content after the conversion to the repository form.
    pub(crate) data: Vec<u8>,
    /// Mode.
    pub(crate) kind: EntryKind,
    /// File status for the index entry.
    pub(crate) stat: gix::index::entry::Stat,
}

/// Reader of working tree files with the content filters of the repository.
///
/// Filter drivers (`filter.<name>.clean`) are programs and are not run; the
/// content of such files is stored unconverted and
/// [`Worktree::dropped_drivers`] reports that drivers were configured.
pub(crate) struct Worktree<'r> {
    repo: &'r Repo,
    root: PathBuf,
    pipeline: gix_filter::Pipeline,
    stack: gix_worktree::Stack,
    filemode: bool,
    symlinks: bool,
    /// Filter drivers were configured and left out.
    pub(crate) dropped_drivers: bool,
}

impl<'r> Worktree<'r> {
    /// Prepares the reader.
    pub(crate) fn new(repo: &'r Repo) -> Result<Self> {
        let root = repo.require_workdir()?.to_path_buf();
        let (pipeline, _) = repo
            .repo
            .filter_pipeline(None)
            .map_err(|error| lib_error("cannot prepare the content filters", error))?;
        let (mut pipeline, stack) = pipeline.into_parts();
        let options = pipeline.options_mut();
        let dropped_drivers = !options.drivers.is_empty();
        options.drivers.clear();
        let filemode = repo.config.flag("core.filemode")?.unwrap_or(cfg!(unix));
        let symlinks = repo.config.flag("core.symlinks")?.unwrap_or(cfg!(unix));
        Ok(Self {
            repo,
            root,
            pipeline,
            stack,
            filemode,
            symlinks,
            dropped_drivers,
        })
    }

    /// Absolute path of a repository path.
    pub(crate) fn absolute(&self, path: &BStr) -> PathBuf {
        path.split_str("/")
            .fold(self.root.clone(), |out, part| out.join(part.to_str_lossy().as_ref()))
    }

    /// Reads a file; `None` when it is missing or is a directory.
    ///
    /// `tracked` is the mode recorded in the index, kept for the executable
    /// bit when `core.filemode` is false and for links when `core.symlinks`
    /// is false.
    pub(crate) fn read(&mut self, path: &BStr, tracked: Option<EntryKind>) -> Result<Option<WorkFile>> {
        let absolute = self.absolute(path);
        let metadata = match std::fs::symlink_metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(&absolute, &error)),
        };
        if metadata.is_dir() {
            return Ok(None);
        }
        let (data, kind) = if metadata.file_type().is_symlink() && self.symlinks {
            let target = std::fs::read_link(&absolute).map_err(|error| io_error(&absolute, &error))?;
            let target = target.to_string_lossy().replace('\\', "/");
            (target.into_bytes(), EntryKind::Link)
        } else {
            let raw = std::fs::read(&absolute).map_err(|error| io_error(&absolute, &error))?;
            let kind = self.file_kind(&metadata, tracked);
            let data = if kind == EntryKind::Link { raw } else { self.clean(path, raw)? };
            (data, kind)
        };
        let fs_metadata = gix::index::fs::Metadata::from_path_no_follow(&absolute)
            .map_err(|error| io_error(&absolute, &error))?;
        let stat = gix::index::entry::Stat::from_fs(&fs_metadata)
            .map_err(|error| lib_error("cannot read the file time", error))?;
        Ok(Some(WorkFile { data, kind, stat }))
    }

    fn file_kind(&self, metadata: &std::fs::Metadata, tracked: Option<EntryKind>) -> EntryKind {
        if !self.symlinks && tracked == Some(EntryKind::Link) {
            return EntryKind::Link;
        }
        if !self.filemode {
            return match tracked {
                Some(EntryKind::BlobExecutable) => EntryKind::BlobExecutable,
                _ => EntryKind::Blob,
            };
        }
        if is_executable(metadata) { EntryKind::BlobExecutable } else { EntryKind::Blob }
    }

    fn clean(&mut self, path: &BStr, raw: Vec<u8>) -> Result<Vec<u8>> {
        use gix_filter::pipeline::convert::ToGitOutcome;
        let relative = gix::path::from_bstr(path).into_owned();
        let platform = self
            .stack
            .at_path(&relative, Some(gix::index::entry::Mode::FILE), &self.repo.repo.objects)
            .map_err(|error| lib_error("cannot read the attributes", error))?;
        let outcome = self
            .pipeline
            .convert_to_git(
                raw.as_slice(),
                &relative,
                &mut |_, attributes| {
                    platform.matching_attributes(attributes);
                },
                &mut |_buffer: &mut Vec<u8>| Ok(None),
            )
            .map_err(|error| lib_error("cannot convert the file content", error))?;
        match outcome {
            ToGitOutcome::Unchanged(_) => Ok(raw),
            ToGitOutcome::Buffer(buffer) => Ok(buffer.to_vec()),
            ToGitOutcome::Process(mut reader) => {
                let mut data = Vec::new();
                reader
                    .read_to_end(&mut data)
                    .map_err(|error| lib_error("cannot convert the file content", error))?;
                Ok(data)
            }
        }
    }

    /// Untracked files that are not ignored, in path order.
    pub(crate) fn untracked(&self, index: &gix::index::State) -> Result<Vec<BString>> {
        let mut excludes = self
            .repo
            .repo
            .excludes(
                index,
                None,
                gix_worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(|error| lib_error("cannot read the ignore rules", error))?;
        let mut found = Vec::new();
        let mut pending = vec![BString::default()];
        while let Some(dir) = pending.pop() {
            let absolute = self.absolute(dir.as_ref());
            let mut names: Vec<(String, std::fs::FileType)> = std::fs::read_dir(&absolute)
                .map_err(|error| io_error(&absolute, &error))?
                .filter_map(std::result::Result::ok)
                .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.file_type().ok()?)))
                .collect();
            names.sort();
            for (name, file_type) in names.into_iter().rev() {
                if name == ".git" {
                    continue;
                }
                let path = join_path(dir.as_ref(), BStr::new(name.as_bytes()));
                let relative = gix::path::from_bstr(path.as_bstr()).into_owned();
                if file_type.is_dir() {
                    let mode = Some(gix::index::entry::Mode::DIR);
                    let excluded = excludes
                        .at_path(&relative, mode)
                        .map_err(|error| lib_error("cannot read the ignore rules", error))?
                        .is_excluded();
                    if !excluded && !self.absolute(path.as_ref()).join(".git").exists() {
                        pending.push(path);
                    }
                    continue;
                }
                if index.entry_range(path.as_ref()).is_some() {
                    continue;
                }
                let excluded = excludes
                    .at_path(&relative, Some(gix::index::entry::Mode::FILE))
                    .map_err(|error| lib_error("cannot read the ignore rules", error))?
                    .is_excluded();
                if !excluded {
                    found.push(path);
                }
            }
        }
        found.sort();
        Ok(found)
    }
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const OWNER_EXECUTE: u32 = 0o100;
    metadata.permissions().mode() & OWNER_EXECUTE != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Returns `true` when `path` equals one of the selected paths or lies below
/// one; an empty selection or the path `""` selects everything.
pub(crate) fn selected(path: &BStr, selection: &[BString]) -> bool {
    selection.is_empty()
        || selection.iter().any(|spec| {
            spec.is_empty()
                || path == spec.as_bstr()
                || (path.starts_with(spec.as_slice()) && path.get(spec.len()) == Some(&b'/'))
        })
}

/// Paths left out by `exclude` patterns.
///
/// A pattern with `/` is matched against the whole path, a pattern without
/// it against the whole path and the file name; `*` does not match `/`, `**`
/// does.
#[derive(Debug, Clone, Default)]
pub(crate) struct Exclude {
    patterns: Vec<BString>,
}

impl Exclude {
    /// Creates the filter from patterns.
    pub(crate) fn new(patterns: &[String]) -> Self {
        Self {
            patterns: patterns.iter().map(|pattern| BString::from(pattern.as_str())).collect(),
        }
    }

    /// Returns `true` when the path is excluded.
    pub(crate) fn matches(&self, path: &BStr) -> bool {
        let mode = gix::glob::wildmatch::Mode::NO_MATCH_SLASH_LITERAL;
        let name = path.rsplit_str("/").next().unwrap_or(path.as_bytes()).as_bstr();
        self.patterns.iter().any(|pattern| {
            gix::glob::wildmatch(pattern.as_bstr(), path, mode)
                || (!pattern.contains(&b'/') && gix::glob::wildmatch(pattern.as_bstr(), name, mode))
        })
    }

    /// Returns `true` when no pattern is set.
    pub(crate) fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

/// Returns `true` when content is binary by the rule of `git`.
pub(crate) fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE_BYTES)].contains(&0)
}

/// Line hunks between two contents, as `git diff` computes them with the
/// Myers algorithm and the indent heuristic.
pub(crate) fn line_hunks(old: &[u8], new: &[u8]) -> Vec<gix_imara_diff::Hunk> {
    let input = gix_imara_diff::InternedInput::new(
        gix_imara_diff::sources::byte_lines(old),
        gix_imara_diff::sources::byte_lines(new),
    );
    let mut diff = gix_imara_diff::Diff::compute(gix_imara_diff::Algorithm::Myers, &input);
    diff.postprocess_lines(&input);
    diff.hunks().collect()
}

/// Lines added and removed between two contents; `None` for binary content.
pub(crate) fn line_counts(old: &[u8], new: &[u8]) -> Option<(u64, u64)> {
    if is_binary(old) || is_binary(new) {
        return None;
    }
    let (added, removed) = line_hunks(old, new).iter().fold((0_u64, 0_u64), |(added, removed), hunk| {
        (
            added + u64::from(hunk.after.end - hunk.after.start),
            removed + u64::from(hunk.before.end - hunk.before.start),
        )
    });
    Some((added, removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_keys_compare_subsections_exactly() {
        assert!(same_key("User.Name", "user.name"));
        assert!(same_key("branch.Main.protected", "branch.Main.PROTECTED"));
        assert!(!same_key("branch.Main.protected", "branch.main.protected"));
        assert!(!same_key("nodot", "nodot"));
    }

    #[test]
    fn booleans_follow_git_spelling() {
        assert_eq!(parse_bool("Yes"), Some(true));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn selection_matches_files_and_directories() {
        let selection = vec![BString::from("src"), BString::from("README.md")];
        assert!(selected(BStr::new("src/a.rs"), &selection));
        assert!(selected(BStr::new("README.md"), &selection));
        assert!(!selected(BStr::new("srcx/a.rs"), &selection));
        assert!(selected(BStr::new("any"), &[BString::default()]));
        assert!(selected(BStr::new("any"), &[]));
    }

    #[test]
    fn exclude_patterns_match_names_and_paths() {
        let exclude = Exclude::new(&["**/secrets.*".to_owned(), ".env".to_owned(), "build/*".to_owned()]);
        assert!(exclude.matches(BStr::new("config/secrets.json")));
        assert!(exclude.matches(BStr::new("app/.env")));
        assert!(exclude.matches(BStr::new("build/out.bin")));
        assert!(!exclude.matches(BStr::new("build/sub/out.bin")));
        assert!(!exclude.matches(BStr::new("src/main.rs")));
        assert!(Exclude::default().is_empty());
    }

    #[test]
    fn binary_detection_and_line_counts() {
        assert!(is_binary(b"a\0b"));
        assert!(!is_binary(b"text\n"));
        assert_eq!(line_counts(b"a\nb\nc\n", b"a\nB\nc\nd\n"), Some((2, 1)));
        assert_eq!(line_counts(b"", b"\0"), None);
    }

    #[test]
    fn commit_objects_are_parsed_for_tree_parents_author_and_message() {
        let data = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nparent 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nsubject\n";
        let info = CommitInfo::parse(data).unwrap();
        assert_eq!(info.parents.len(), 1);
        assert_eq!(info.author, "A <a@x> 1 +0000");
        assert_eq!(info.message, "subject\n");
        assert!(CommitInfo::parse(b"garbage").is_none());
    }
}
