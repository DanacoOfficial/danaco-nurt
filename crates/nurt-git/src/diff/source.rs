// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository access for differences and status: opening without external
//! programs, file contents in the form stored by Git, attributes of the
//! difference and the file lists of the compared sides.
//!
//! Filter drivers (`filter.<name>.clean`, `smudge`, `process`) and text
//! conversion programs (`diff.<name>.textconv`) are not run: worktree files
//! are compared after the built-in conversions only (`core.autocrlf`,
//! `core.eol`, `text`, `eol`, `ident`, `working-tree-encoding`).

use std::collections::BTreeMap;
use std::path::{Path as StdPath, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::{Notable, NotableEntry};

/// Bytes inspected for a NUL byte when deciding whether content is binary.
pub const BINARY_PROBE: usize = 8000;

/// Mode of a regular file.
pub const MODE_FILE: u32 = 0o100_644;
/// Mode of an executable file.
pub const MODE_EXECUTABLE: u32 = 0o100_755;
/// Mode of a symbolic link.
pub const MODE_LINK: u32 = 0o120_000;
/// Mode of a submodule commit.
pub const MODE_GITLINK: u32 = 0o160_000;

/// Configuration keys of filter drivers that name programs.
const FILTER_PROGRAM_KEYS: [&str; 3] = ["clean", "smudge", "process"];

/// A repository opened for reading, with the notes that the opening raised.
pub struct Opened {
    /// The repository.
    pub repo: gix::Repository,
    /// Settings that are not applied because they name programs.
    pub notes: Vec<NotableEntry>,
}

/// Opens the repository that contains `path`, searching upwards.
///
/// The configuration of the Git installation is not looked up through the
/// `git` program, the `file`, `ssh` and `ext` transports are disabled and
/// filter driver sections are removed from the in-memory configuration.
///
/// # Errors
///
/// `E_NOT_FOUND` when `path` does not exist, `E_GIT_NOT_REPOSITORY` when no
/// repository contains it.
pub fn open_repository(path: &StdPath) -> Result<Opened, Error> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err(Error::new(ErrorCode::NotFound, format!("`{}` does not exist", path.display()))
            .with_problem("not_found"));
    }
    let mut permissions = gix::open::Permissions::default();
    permissions.config.git_binary = false;
    let options = gix::open::Options::default()
        .permissions(permissions)
        .config_overrides(["protocol.file.allow=never", "protocol.ssh.allow=never", "protocol.ext.allow=never"]);
    let mapping = gix::sec::trust::Mapping {
        full: options.clone(),
        reduced: options,
    };
    let repo = gix::ThreadSafeRepository::discover_opts(path, gix::discover::upwards::Options::default(), mapping)
        .map_err(|_| not_a_repository(path))?;
    let mut repo = repo.to_thread_local();
    let mut notes = Vec::new();
    strip_filter_drivers(&mut repo, &mut notes)?;
    let fsmonitor = repo.config_snapshot().string("core.fsmonitor");
    if fsmonitor.is_some_and(|value| !matches!(value.to_str_lossy().as_ref(), "" | "false" | "0" | "no" | "off")) {
        let mut note = NotableEntry::new(Notable::FsmonitorIgnored);
        note.detail = Some("core.fsmonitor is not used, files are checked directly".to_owned());
        notes.push(note);
    }
    Ok(Opened { repo, notes })
}

fn not_a_repository(path: &StdPath) -> Error {
    Error::new(
        ErrorCode::GitNotRepository,
        format!("`{}` is not inside a git repository", path.display()),
    )
    .with_problem("not_a_repository")
    .with_next_step(format!("git.status \"{}\"", path.display()))
}

fn strip_filter_drivers(repo: &mut gix::Repository, notes: &mut Vec<NotableEntry>) -> Result<(), Error> {
    let mut drivers = Vec::new();
    let mut snapshot = repo.config_snapshot_mut();
    let ids: Vec<_> = snapshot
        .sections_and_ids_by_name("filter")
        .map(|sections| {
            sections
                .map(|(section, id)| {
                    let names_program = FILTER_PROGRAM_KEYS
                        .iter()
                        .any(|key| section.value(key).is_some());
                    if names_program {
                        if let Some(name) = section.header().subsection_name() {
                            drivers.push(name.to_str_lossy().into_owned());
                        }
                    }
                    id
                })
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        snapshot.remove_section_by_id(id);
    }
    snapshot.commit().map_err(|error| read_error("configuration", &error))?;
    drivers.sort();
    drivers.dedup();
    if !drivers.is_empty() {
        let mut note = NotableEntry::new(Notable::FilterNotApplied);
        note.count = Some(drivers.len() as u64);
        note.detail = Some(format!("filter drivers not run: {}", drivers.join(", ")));
        notes.push(note);
    }
    Ok(())
}

/// Error of a failed read of repository data.
pub(crate) fn read_error(what: &str, error: &dyn std::fmt::Display) -> Error {
    Error::new(ErrorCode::StageFailed, format!("cannot read {what}: {error}")).with_problem("read_failed")
}

/// An entry of one side of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Mode in the Git encoding, for example `0o100644`.
    pub mode: u32,
    /// Object of the content; `None` when the content is a worktree file that
    /// must be read.
    pub id: Option<ObjectId>,
}

impl Entry {
    /// The entry is a regular or executable file.
    #[must_use]
    pub fn is_file(&self) -> bool {
        self.mode & 0o170_000 == 0o100_000
    }

    /// The entry is a symbolic link.
    #[must_use]
    pub fn is_link(&self) -> bool {
        self.mode == MODE_LINK
    }
}

/// The mode in six octal digits.
#[must_use]
pub fn mode_text(mode: u32) -> String {
    format!("{mode:06o}")
}

/// Mode of a tree entry.
pub(crate) fn tree_mode(mode: gix::object::tree::EntryMode) -> u32 {
    u32::from(mode.value())
}

/// Mode of an index entry.
pub(crate) fn index_mode(mode: gix::index::entry::Mode) -> u32 {
    mode.bits()
}

/// Files of a tree, sorted by path.
pub(crate) fn tree_files(repo: &gix::Repository, tree: ObjectId) -> Result<BTreeMap<BString, Entry>, Error> {
    let mut files = BTreeMap::new();
    if tree == ObjectId::empty_tree(repo.object_hash()) {
        return Ok(files);
    }
    let tree = repo.find_tree(tree).map_err(|error| read_error("tree", &error))?;
    let mut recorder = gix::traverse::tree::Recorder::default();
    tree.traverse()
        .breadthfirst(&mut recorder)
        .map_err(|error| read_error("tree", &error))?;
    for record in recorder.records {
        if record.mode.is_tree() {
            continue;
        }
        files.insert(
            record.filepath,
            Entry {
                mode: tree_mode(record.mode),
                id: Some(record.oid),
            },
        );
    }
    Ok(files)
}

/// Stage-0 files of the index, sorted by path, and the paths with conflict
/// stages.
pub(crate) fn index_files(index: &gix::index::State) -> (BTreeMap<BString, Entry>, BTreeMap<BString, [Option<Entry>; 3]>) {
    let mut files = BTreeMap::new();
    let mut conflicts: BTreeMap<BString, [Option<Entry>; 3]> = BTreeMap::new();
    for entry in index.entries() {
        let path = entry.path(index).to_owned();
        let side = Entry {
            mode: index_mode(entry.mode),
            id: Some(entry.id),
        };
        match entry.stage_raw() {
            0 => {
                if !entry.flags.contains(gix::index::entry::Flags::INTENT_TO_ADD) {
                    files.insert(path, side);
                }
            }
            stage => {
                let slot = conflicts.entry(path).or_default();
                if let Some(cell) = slot.get_mut(stage as usize - 1) {
                    *cell = Some(side);
                }
            }
        }
    }
    (files, conflicts)
}

/// The tree of the commit named by `rev`, or of `HEAD` when `rev` is
/// `None`; the empty tree for an unborn `HEAD`.
///
/// # Errors
///
/// `E_GIT_REVISION` with problem `invalid_syntax`, `not_found` or
/// `not_a_commit`.
pub(crate) fn resolve_tree(repo: &gix::Repository, rev: Option<&str>) -> Result<ObjectId, Error> {
    let Some(rev) = rev else {
        return repo
            .head_tree_id_or_empty()
            .map(gix::Id::detach)
            .map_err(|error| read_error("HEAD", &error));
    };
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
        return Err(revision_error(rev, "invalid_syntax", "is not a valid revision"));
    }
    let id = repo
        .rev_parse_single(rev)
        .map_err(|_| revision_error(rev, "not_found", "does not name a revision"))?;
    let object = id
        .object()
        .map_err(|_| revision_error(rev, "not_found", "does not name a revision"))?;
    let tree = object
        .peel_to_tree()
        .map_err(|_| revision_error(rev, "not_a_commit", "does not name a commit or a tree"))?;
    Ok(tree.id)
}

fn revision_error(rev: &str, problem: &str, what: &str) -> Error {
    Error::new(ErrorCode::GitRevision, format!("`{rev}` {what}, use a branch, a tag or a commit"))
        .with_problem(problem)
        .with_found(rev)
}

/// Attribute `diff` of a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffAttr {
    /// Not specified: binary when the content contains a NUL byte.
    Auto,
    /// `-diff` or `binary`: always binary.
    Binary,
    /// `diff`: always text.
    Text,
    /// `diff=<name>`: the driver `diff.<name>.*`.
    Driver(String),
}

/// Reader of file contents and attributes.
pub struct Reader<'r> {
    repo: &'r gix::Repository,
    workdir: Option<PathBuf>,
    pipeline: Option<gix::filter::Pipeline<'r>>,
    index: gix::worktree::IndexPersistedOrInMemory,
    attributes: Option<gix::AttributeStack<'r>>,
    attribute_out: Option<gix::attrs::search::Outcome>,
    filemode: bool,
}

impl<'r> Reader<'r> {
    /// Prepares a reader for `repo`.
    ///
    /// # Errors
    ///
    /// Fails when the index or the attribute files cannot be read.
    pub fn new(repo: &'r gix::Repository) -> Result<Self, Error> {
        let workdir = repo.workdir().map(StdPath::to_path_buf);
        let (pipeline, index) = if workdir.is_some() {
            let (pipeline, index) = repo
                .filter_pipeline(None)
                .map_err(|error| read_error("attributes", &error))?;
            (Some(pipeline), index)
        } else {
            let index = repo.index_or_empty().map_err(|error| read_error("index", &error))?;
            (None, gix::worktree::IndexPersistedOrInMemory::Persisted(index))
        };
        let source = if workdir.is_some() {
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping
        } else {
            gix::worktree::stack::state::attributes::Source::IdMapping
        };
        let attributes = repo.attributes_only(&index, source).ok();
        let attribute_out = attributes
            .as_ref()
            .map(|stack| stack.selected_attribute_matches(["diff"]));
        let filemode = repo
            .config_snapshot()
            .boolean("core.fileMode")
            .unwrap_or(!cfg!(windows));
        Ok(Self {
            repo,
            workdir,
            pipeline,
            index,
            attributes,
            attribute_out,
            filemode,
        })
    }

    /// The index the reader uses.
    #[must_use]
    pub fn index(&self) -> &gix::index::State {
        &self.index
    }

    /// Content of a blob.
    ///
    /// # Errors
    ///
    /// Fails when the object is missing.
    pub fn blob(&self, id: ObjectId) -> Result<Vec<u8>, Error> {
        if id.is_null() {
            return Ok(Vec::new());
        }
        let object = self.repo.find_object(id).map_err(|error| read_error("object", &error))?;
        Ok(object.detach().data)
    }

    /// Content of an entry; a submodule commit reads as
    /// `Subproject commit <id>`.
    ///
    /// # Errors
    ///
    /// Fails when the object or the file cannot be read.
    pub fn content(&mut self, path: &BStr, entry: &Entry) -> Result<Vec<u8>, Error> {
        if entry.mode == MODE_GITLINK {
            let id = entry.id.unwrap_or_else(|| ObjectId::null(self.repo.object_hash()));
            return Ok(format!("Subproject commit {id}\n").into_bytes());
        }
        match entry.id {
            Some(id) => self.blob(id),
            None => Ok(self.worktree(path, None)?.map(|(data, _)| data).unwrap_or_default()),
        }
    }

    /// A worktree file in the form stored by Git with its mode; `None` when
    /// the file does not exist or is a directory. `index_mode` gives the
    /// executable bit when `core.fileMode` is off.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or converted.
    pub fn worktree(&mut self, path: &BStr, index_mode: Option<u32>) -> Result<Option<(Vec<u8>, u32)>, Error> {
        let Some(root) = &self.workdir else {
            return Ok(None);
        };
        let relative = gix::path::from_bstr(path).map_err(|error| read_error("path", &error))?;
        let full = root.join(&relative);
        let Ok(metadata) = std::fs::symlink_metadata(&full) else {
            return Ok(None);
        };
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&full).map_err(|error| read_error("link", &error))?;
            let bytes = gix::path::into_bstr(target)
                .map_err(|error| read_error("link", &error))?
                .into_owned();
            return Ok(Some((bytes.into(), MODE_LINK)));
        }
        if metadata.is_dir() {
            return Ok(None);
        }
        let mode = self.file_mode(&metadata, index_mode);
        let file = std::fs::File::open(&full).map_err(|error| read_error("file", &error))?;
        let Some(pipeline) = self.pipeline.as_mut() else {
            return Ok(None);
        };
        let outcome = pipeline
            .convert_to_git(file, &relative, &self.index)
            .map_err(|error| read_error("file", &error))?;
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut { outcome }, &mut data).map_err(|error| read_error("file", &error))?;
        Ok(Some((data, mode)))
    }

    fn file_mode(&self, metadata: &std::fs::Metadata, index_mode: Option<u32>) -> u32 {
        if !self.filemode {
            return match index_mode {
                Some(MODE_EXECUTABLE) => MODE_EXECUTABLE,
                _ => MODE_FILE,
            };
        }
        if is_executable(metadata) { MODE_EXECUTABLE } else { MODE_FILE }
    }

    /// The `diff` attribute of `path`.
    pub fn diff_attr(&mut self, path: &BStr) -> DiffAttr {
        let (Some(stack), Some(out)) = (self.attributes.as_mut(), self.attribute_out.as_mut()) else {
            return DiffAttr::Auto;
        };
        let Ok(platform) = stack.at_entry(path, None) else {
            return DiffAttr::Auto;
        };
        out.reset();
        platform.matching_attributes(out);
        let Some(found) = out.iter_selected().next() else {
            return DiffAttr::Auto;
        };
        match found.assignment.state {
            gix::attrs::StateRef::Set => DiffAttr::Text,
            gix::attrs::StateRef::Unset => DiffAttr::Binary,
            gix::attrs::StateRef::Value(value) => DiffAttr::Driver(value.as_bstr().to_str_lossy().into_owned()),
            gix::attrs::StateRef::Unspecified => DiffAttr::Auto,
        }
    }

    /// The repository.
    #[must_use]
    pub fn repo(&self) -> &'r gix::Repository {
        self.repo
    }
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Content contains a NUL byte among its first [`BINARY_PROBE`] bytes.
#[must_use]
pub fn looks_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE)].contains(&0)
}
