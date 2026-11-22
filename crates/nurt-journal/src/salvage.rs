// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repair of corrupt entries.
//!
//! [`list`] reports the entries of a volume journal that need manual repair:
//! entries marked corrupt, entries whose state file or manifest cannot be
//! verified and entries whose item state file fails its checksum. For each
//! one it reports what can still be read: number, time, kind, targets read
//! from the manifest member by member, and the preserved data with sizes,
//! hashes and metadata.
//!
//! [`extract`] copies the preserved data of one entry into a new directory.
//! Writing that directory changes user data, so it goes through a mutation
//! transaction supplied by the caller, which commits it. Neither function
//! changes user data in place or removes data of the journal; a repaired
//! entry is released with `journal.purge`.
//!
//! [`verify_chain`] checks the hash chain of the account index. The chain is
//! evidence for repair, not a validity condition of single entries: a broken
//! or forked link never marks an entry corrupt.

use std::collections::BTreeSet;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, ContentHash, ExtendedMeta, FileKind, Follow, Fs, FsErrorKind, LinkKind,
};
use nurt_contracts::journal::{
    CloneMode, EntryState, Expect, Mutation, MutationTx, OpId, TargetAction, TargetIndex,
    TargetPath, TargetSpec,
};
use serde::Serialize;
use serde_json::Value;

use crate::integrity::{self, ChainEntry, ManifestStatus, RecordedLink};
pub use crate::integrity::{ChainFork, ChainReport, LinkStatus};
use crate::layout::{self, EntryLayout, JournalLayout, StateLayout};
use crate::manifest::{self, Manifest};

/// Name of the description written into the extraction directory.
pub const DESCRIPTION_FILE: &str = "salvage.json";
/// Directory of the extraction holding data whose target path is unknown.
pub const RAW_DIR: &str = "blobs";
/// Permission bits of directories created by an extraction.
const DIRECTORY_MODE: u32 = 0o700;
/// Suffix of preserved content files.
const BLOB_SUFFIX: &str = ".blob";
/// Suffix of preserved metadata files.
const META_SUFFIX: &str = ".meta";

/// Why an entry needs manual repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Damage {
    /// The entry is in the state `corrupt`.
    Corrupt,
    /// The state file is missing or holds no known state.
    StateUnreadable,
    /// The manifest is missing, malformed or fails its digest.
    Manifest {
        /// Short name of the problem (`digest_mismatch`, `not_json`,
        /// `missing`).
        problem: String,
    },
    /// The item state file fails its checksum; every item is treated as
    /// unavailable.
    ItemsDamaged,
}

/// Target of an entry as far as the manifest can be read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SalvageTarget {
    /// Target index.
    pub index: Option<u32>,
    /// Address in language form; provider keys in their JSON form.
    pub path: Option<String>,
    /// Preserve mode.
    pub store: Option<String>,
    /// Preserved data relative to the entry directory.
    pub blob: Option<String>,
}

/// Preserved data found in the entry directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlobInfo {
    /// File name in `blobs/`.
    pub name: String,
    /// Kind of the preserved entry.
    pub kind: Option<FileKind>,
    /// Logical size.
    pub size: Option<u64>,
    /// Content hash of a regular file, when it can be read.
    pub hash: Option<ContentHash>,
    /// Preserved metadata, when the metadata file can be read.
    pub meta: Option<ExtendedMeta>,
}

/// Entry that needs manual repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DamagedEntry {
    /// Operation number.
    pub op: OpId,
    /// State, when the state file can be read.
    pub state: Option<EntryState>,
    /// Problems found.
    pub damage: Vec<Damage>,
    /// Time of the operation, when the manifest records it.
    pub time: Option<WallTime>,
    /// Command of the operation, when the manifest records it.
    pub command: Option<String>,
    /// Targets read from the manifest.
    pub targets: Vec<SalvageTarget>,
    /// Preserved data.
    pub blobs: Vec<BlobInfo>,
}

/// Outcome of one preserved item of an extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    /// Copied into the extraction directory.
    Extracted,
    /// Not copied: special files (devices, pipes, sockets) cannot be copied.
    Skipped,
    /// Not copied: the data cannot be read.
    Unreadable,
}

/// One preserved item of an extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtractedItem {
    /// Path of the data relative to the entry directory.
    pub source: String,
    /// Path relative to the extraction directory, `/`-separated.
    pub path: String,
    /// Outcome.
    pub status: ItemStatus,
}

/// Result of an extraction; also written as [`DESCRIPTION_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Extraction {
    /// Entry.
    pub op: OpId,
    /// Journal directory of the volume.
    pub journal: AbsPath,
    /// Extraction directory.
    pub destination: AbsPath,
    /// Items in extraction order.
    pub items: Vec<ExtractedItem>,
}

/// Lists the entries of the volume journal `journal_dir` that need manual
/// repair, in ascending order of number.
///
/// Every entry of `ops/` is checked: its state, its manifest digest and its
/// item state checksum. Damaged content of any of these files never causes a
/// panic.
///
/// # Errors
///
/// Fails with the file system error (`E_IO`, `E_PERMISSION`) when the journal
/// directory or a file of an entry cannot be read; a missing journal yields
/// an empty list.
pub fn list(fs: &dyn Fs, journal_dir: &AbsPath) -> Result<Vec<DamagedEntry>> {
    let journal = JournalLayout::new(journal_dir.clone());
    let mut damaged = Vec::new();
    for op in layout::entries(fs, &journal)? {
        if let Some(entry) = inspect(fs, &journal.entry(op)?, op)? {
            damaged.push(entry);
        }
    }
    Ok(damaged)
}

/// Verifies the hash chain of the account index in `state_dir`.
///
/// Every row of the index is linked to the manifest digest of the row that
/// precedes it, as recorded in the manifest of its entry. A link that cannot
/// be read because the entry no longer exists or its manifest fails
/// verification is reported as unverifiable; a fork left by a crash between
/// a commit and the append of its index row is reported without marking an
/// entry corrupt. Nothing is changed.
///
/// # Errors
///
/// Fails with the file system error (`E_IO`, `E_PERMISSION`) when the index
/// or a manifest cannot be read, and with `E_JOURNAL_CORRUPT` when a complete
/// line of the index is malformed.
pub fn verify_chain(fs: &dyn Fs, state_dir: &AbsPath) -> Result<ChainReport> {
    let index = StateLayout::new(state_dir.clone()).index()?;
    let mut entries = Vec::new();
    for row in manifest::read_index(fs, &index)? {
        let (Some(digest), Some(part)) = (row.digest(), row.parts.first()) else {
            continue;
        };
        entries.push(ChainEntry {
            op: row.op,
            digest,
            link: recorded_link(fs, &part.dir, row.op)?,
        });
    }
    Ok(integrity::verify_chain(&entries))
}

/// Link recorded by the committed manifest of entry `op`.
fn recorded_link(fs: &dyn Fs, journal_dir: &AbsPath, op: OpId) -> Result<RecordedLink> {
    let entry = JournalLayout::new(journal_dir.clone()).entry(op)?;
    let bytes = match manifest::read_bytes(fs, &entry.manifest()?) {
        Ok(bytes) => bytes,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(RecordedLink::Unavailable),
        Err(error) => return Err(error.into()),
    };
    if !integrity::verify_manifest(&bytes).is_intact() {
        return Ok(RecordedLink::Unavailable);
    }
    Ok(match Manifest::decode(&bytes) {
        Ok(manifest) => manifest
            .integrity
            .chain
            .map_or(RecordedLink::First, RecordedLink::After),
        Err(_) => RecordedLink::Unavailable,
    })
}

fn inspect(fs: &dyn Fs, entry: &EntryLayout, op: OpId) -> Result<Option<DamagedEntry>> {
    let check = integrity::check_entry(fs, entry)?;
    if !check.is_corrupt() && check.items_intact {
        return Ok(None);
    }
    let mut damage = Vec::new();
    match check.state {
        None => damage.push(Damage::StateUnreadable),
        Some(record) if record.state == EntryState::Corrupt => damage.push(Damage::Corrupt),
        Some(_) => {}
    }
    if let Some(problem) = check.manifest.problem() {
        damage.push(Damage::Manifest {
            problem: problem.to_owned(),
        });
    }
    if !check.items_intact {
        damage.push(Damage::ItemsDamaged);
    }
    if damage.is_empty() {
        return Ok(None);
    }
    let manifest = tolerant_manifest(fs, entry, check.manifest)?;
    Ok(Some(DamagedEntry {
        op,
        state: check.state.map(|record| record.state),
        damage,
        time: manifest
            .as_ref()
            .and_then(|value| serde_json::from_value(value.get("time")?.clone()).ok()),
        command: manifest
            .as_ref()
            .and_then(|value| Some(value.get("kind")?.get("command")?.as_str()?.to_owned())),
        targets: manifest.as_ref().map(targets_of).unwrap_or_default(),
        blobs: blobs_of(fs, entry)?,
    }))
}

/// Manifest as a JSON value, whatever its digest; `None` when it is not JSON.
fn tolerant_manifest(
    fs: &dyn Fs,
    entry: &EntryLayout,
    status: ManifestStatus,
) -> Result<Option<Value>> {
    if status == ManifestStatus::Missing {
        return Ok(None);
    }
    match manifest::read_bytes(fs, &entry.manifest()?) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Targets read member by member; unreadable members stay `None`. The
/// preserved destination of a rename is reported as a target of its own.
fn targets_of(manifest: &Value) -> Vec<SalvageTarget> {
    let Some(targets) = manifest.get("targets").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for target in targets {
        let index = target
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| u32::try_from(index).ok());
        let path = target
            .get("path")
            .and_then(|path| path.get("at"))
            .map(|at| {
                at.as_str()
                    .map_or_else(|| at.to_string(), ToOwned::to_owned)
            });
        found.push(SalvageTarget {
            index,
            path,
            store: text_at(target, &["before", "store"]),
            blob: text_at(target, &["before", "blob"]),
        });
        if target.get("replaced").is_some_and(Value::is_object) {
            found.push(SalvageTarget {
                index,
                path: text_at(target, &["action", "to"]),
                store: text_at(target, &["replaced", "store"]),
                blob: text_at(target, &["replaced", "blob"]),
            });
        }
    }
    found
}

fn text_at(value: &Value, members: &[&str]) -> Option<String> {
    let mut current = value;
    for member in members {
        current = current.get(member)?;
    }
    current.as_str().map(ToOwned::to_owned)
}

/// Preserved data of the entry, ordered by name.
fn blobs_of(fs: &dyn Fs, entry: &EntryLayout) -> Result<Vec<BlobInfo>> {
    let blobs = entry.blobs()?;
    let mut names = Vec::new();
    match fs.read_dir(&blobs) {
        Ok(listing) => {
            for child in listing {
                if let Some(name) = child?.name.to_str() {
                    names.push(name.to_owned());
                }
            }
        }
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    }
    names.sort();
    let mut found = Vec::new();
    for name in names {
        let Some(stem) = name.strip_suffix(BLOB_SUFFIX) else {
            continue;
        };
        let path = blobs.join_name(&name)?;
        let meta = fs.metadata(&path, Follow::NoFollow).ok();
        let kind = meta.as_ref().map(|meta| meta.kind);
        let hash = if kind == Some(FileKind::File) {
            fs.content_hash(&path).ok()
        } else {
            None
        };
        let preserved_meta = blobs
            .join_name(&format!("{stem}{META_SUFFIX}"))
            .ok()
            .and_then(|path| manifest::read_meta(fs, &path).ok());
        found.push(BlobInfo {
            name,
            kind,
            size: meta.map(|meta| meta.size),
            hash,
            meta: preserved_meta,
        });
    }
    Ok(found)
}

/// One entry created by an extraction.
enum Planned {
    Directory,
    Copy(AbsPath),
    Link(std::path::PathBuf, LinkKind),
    Bytes(Vec<u8>),
}

/// Copies the preserved data of entry `op` of the journal `journal_dir` into
/// the new directory `destination`, through `tx`.
///
/// Data of a target whose path the manifest records is placed under the
/// path of the target relative to its root (`/home/user/a.txt` becomes
/// `home/user/a.txt`); other data is placed under [`RAW_DIR`] with its block
/// name. A directory tree preserved as a whole is copied entry by entry.
/// Items that cannot be read are reported and left out; the description
/// [`DESCRIPTION_FILE`] lists every item. The journal is only read.
///
/// The caller opens `tx` for the operation `journal.salvage`, and commits or
/// rolls it back after this function returns.
///
/// # Errors
///
/// Fails with `E_EXISTS` when `destination` exists, with `E_NOT_FOUND` when
/// the entry does not exist, with the file system error when the entry
/// cannot be listed, and with the error of `tx` when the extraction cannot
/// be written (the transaction is then rolled back).
pub fn extract(
    tx: &mut dyn MutationTx,
    fs: &dyn Fs,
    journal_dir: &AbsPath,
    op: OpId,
    destination: &AbsPath,
) -> Result<Extraction> {
    if fs.metadata(destination, Follow::NoFollow).is_ok() {
        return Err(Error::new(
            ErrorCode::Exists,
            format!("{destination} exists; choose a directory that does not exist"),
        ));
    }
    let journal = JournalLayout::new(journal_dir.clone());
    let entry = journal.entry(op)?;
    if fs.metadata(entry.dir(), Follow::NoFollow).is_err() {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!("journal entry {} does not exist in {journal_dir}", op.0),
        ));
    }
    let manifest = tolerant_manifest(fs, &entry, ManifestStatus::Mismatch)?;
    let targets = manifest.as_ref().map(targets_of).unwrap_or_default();
    let mut plan = Plan::new(destination.clone());
    for blob in blobs_of(fs, &entry)? {
        let source = format!("{}/{}", layout::BLOBS_DIR, blob.name);
        let place = place_of(&targets, &source, &blob.name);
        let path = entry.resolve(&source)?;
        plan.add_tree(fs, &source, &path, &place)?;
    }
    let mut extraction = Extraction {
        op,
        journal: journal_dir.clone(),
        destination: destination.clone(),
        items: plan.items,
    };
    let description = serde_json::to_vec_pretty(&extraction)
        .map_err(|error| Error::new(ErrorCode::Io, format!("description not encoded: {error}")))?;
    plan.entries.push((
        destination.join_name(DESCRIPTION_FILE)?,
        Planned::Bytes(description),
    ));
    write_plan(tx, plan.entries)?;
    extraction
        .items
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(extraction)
}

/// Relative place of preserved data: the recorded target path, or
/// [`RAW_DIR`] with the block name.
fn place_of(targets: &[SalvageTarget], source: &str, name: &str) -> Vec<String> {
    let recorded = targets
        .iter()
        .find(|target| target.blob.as_deref() == Some(source))
        .and_then(|target| target.path.as_deref())
        .map(relative_components)
        .filter(|components| !components.is_empty());
    recorded.unwrap_or_else(|| vec![RAW_DIR.to_owned(), name.to_owned()])
}

/// Components of an absolute path in language form, without the root; a
/// drive letter becomes a directory without its colon.
fn relative_components(path: &str) -> Vec<String> {
    path.split('/')
        .map(|component| component.strip_suffix(':').unwrap_or(component))
        .filter(|component| !component.is_empty() && *component != "." && *component != "..")
        .map(ToOwned::to_owned)
        .collect()
}

/// Entries to create, in creation order, with the report of every item.
struct Plan {
    destination: AbsPath,
    entries: Vec<(AbsPath, Planned)>,
    directories: BTreeSet<AbsPath>,
    taken: BTreeSet<AbsPath>,
    items: Vec<ExtractedItem>,
}

impl Plan {
    fn new(destination: AbsPath) -> Self {
        Self {
            entries: vec![(destination.clone(), Planned::Directory)],
            directories: BTreeSet::from([destination.clone()]),
            taken: BTreeSet::new(),
            destination,
            items: Vec::new(),
        }
    }

    /// Path under the destination for `components`, creating the parent
    /// directories in the plan; a path already used gets a numbered suffix.
    fn place(&mut self, components: &[String]) -> Result<AbsPath> {
        let Some((last, parents)) = components.split_last() else {
            return Ok(self.destination.clone());
        };
        let mut dir = self.destination.clone();
        for component in parents {
            dir = dir.join_name(component)?;
            if self.directories.insert(dir.clone()) {
                self.entries.push((dir.clone(), Planned::Directory));
            }
        }
        let mut candidate = dir.join_name(last)?;
        let mut attempt: u64 = 1;
        while self.taken.contains(&candidate) || self.directories.contains(&candidate) {
            candidate = dir.join_name(&format!("{last}.{attempt}"))?;
            attempt += 1;
        }
        self.taken.insert(candidate.clone());
        Ok(candidate)
    }

    fn relative(&self, path: &AbsPath) -> String {
        path.as_path()
            .strip_prefix(self.destination.as_path())
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }

    /// Adds preserved data at `path` (a file, link or directory tree).
    fn add_tree(
        &mut self,
        fs: &dyn Fs,
        source: &str,
        path: &AbsPath,
        components: &[String],
    ) -> Result<()> {
        let mut pending = vec![(path.clone(), source.to_owned(), components.to_vec())];
        while let Some((current, source, components)) = pending.pop() {
            let Ok(meta) = fs.metadata(&current, Follow::NoFollow) else {
                self.report(source, &components, ItemStatus::Unreadable);
                continue;
            };
            match meta.kind {
                FileKind::Directory => {
                    let Ok(listing) = fs.read_dir(&current) else {
                        self.report(source, &components, ItemStatus::Unreadable);
                        continue;
                    };
                    let place = self.place(&components)?;
                    self.directories.insert(place.clone());
                    self.entries.push((place, Planned::Directory));
                    let mut children: Vec<String> = Vec::new();
                    for child in listing.flatten() {
                        children.push(child.name.to_string_lossy().into_owned());
                    }
                    children.sort();
                    for child in children.into_iter().rev() {
                        let mut nested = components.clone();
                        nested.push(child.clone());
                        pending.push((
                            current.join_name(&child)?,
                            format!("{source}/{child}"),
                            nested,
                        ));
                    }
                }
                FileKind::File if fs.open_read(&current).is_ok() => {
                    let place = self.place(&components)?;
                    self.items.push(ExtractedItem {
                        source,
                        path: self.relative(&place),
                        status: ItemStatus::Extracted,
                    });
                    self.entries.push((place, Planned::Copy(current)));
                }
                FileKind::Symlink | FileKind::Junction => {
                    let Ok(target) = fs.read_link(&current) else {
                        self.report(source, &components, ItemStatus::Unreadable);
                        continue;
                    };
                    let kind = if meta.kind == FileKind::Junction {
                        LinkKind::Junction
                    } else {
                        LinkKind::SymlinkFile
                    };
                    let place = self.place(&components)?;
                    self.items.push(ExtractedItem {
                        source,
                        path: self.relative(&place),
                        status: ItemStatus::Extracted,
                    });
                    self.entries.push((place, Planned::Link(target, kind)));
                }
                FileKind::File => self.report(source, &components, ItemStatus::Unreadable),
                _ => self.report(source, &components, ItemStatus::Skipped),
            }
        }
        Ok(())
    }

    fn report(&mut self, source: String, components: &[String], status: ItemStatus) {
        let path = components.join("/");
        self.items.push(ExtractedItem {
            source,
            path,
            status,
        });
    }
}

/// Declares every planned entry, then creates them in order.
fn write_plan(tx: &mut dyn MutationTx, entries: Vec<(AbsPath, Planned)>) -> Result<()> {
    let specs: Vec<TargetSpec> = entries
        .iter()
        .map(|(path, _)| TargetSpec {
            path: TargetPath::Fs(path.clone()),
            action: TargetAction::Create,
            expect: Expect::Absent,
            size_hint: None,
        })
        .collect();
    tx.preserve(&specs)?;
    for (position, (_, planned)) in entries.into_iter().enumerate() {
        let index = TargetIndex(u32::try_from(position).map_err(|_| {
            Error::new(
                ErrorCode::Io,
                "too many items for one extraction; extract the entry in parts",
            )
        })?);
        let mutation = match planned {
            Planned::Directory => Mutation::CreateDir {
                mode: DIRECTORY_MODE,
            },
            Planned::Copy(source) => Mutation::CopyFrom {
                source,
                clone: CloneMode::Auto,
            },
            Planned::Link(target, kind) => Mutation::CreateLink { target, kind },
            Planned::Bytes(bytes) => Mutation::WriteBytes(bytes),
        };
        tx.apply(index, mutation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;

    use nurt_contracts::fs::{FsWrite, RenameMode, ScopePurpose};
    use nurt_contracts::journal::{
        AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, OpHandle, OpKind,
        PreserveMode, Preserved, RollbackReason, RollbackReport,
    };
    use nurt_contracts::report::IrreversibleEntry;
    use nurt_fs::mem::{MemFs, VolumeSpec};
    use proptest::prelude::*;

    use crate::layout::BlobId;

    use super::*;
    use crate::manifest::{Items, StateRecord};
    use crate::store;

    fn abs(text: &str) -> AbsPath {
        AbsPath::new(text).unwrap()
    }

    fn mem() -> Arc<MemFs> {
        Arc::new(
            MemFs::builder()
                .volume(VolumeSpec::posix(abs("/v")))
                .build()
                .unwrap(),
        )
    }

    /// Transaction that applies creations directly, for extraction tests.
    struct DirectTx {
        fs: Arc<MemFs>,
        declared: Vec<TargetSpec>,
    }

    impl DirectTx {
        fn new(fs: &Arc<MemFs>) -> Self {
            Self {
                fs: fs.clone(),
                declared: Vec::new(),
            }
        }
    }

    impl MutationTx for DirectTx {
        fn op(&self) -> Option<OpId> {
            Some(OpId(99))
        }

        fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
            let mut preserved = Vec::new();
            for spec in targets {
                let TargetPath::Fs(path) = &spec.path else {
                    return Err(Error::new(ErrorCode::Io, "file system targets only"));
                };
                if self.fs.metadata(path, Follow::NoFollow).is_ok() {
                    return Err(Error::new(ErrorCode::Exists, "target exists"));
                }
                preserved.push(Preserved {
                    index: TargetIndex(u32::try_from(self.declared.len()).unwrap()),
                    volume: None,
                    mode: PreserveMode::Created,
                    bytes_held: 0,
                    before: BeforeState::default(),
                    irreversible: None,
                    notable: Vec::new(),
                });
                self.declared.push(spec.clone());
            }
            Ok(preserved)
        }

        fn stage(
            &mut self,
            _target: TargetIndex,
        ) -> Result<Box<dyn nurt_contracts::fs::WriteHandle>> {
            Err(Error::new(
                ErrorCode::Io,
                "staging is not used by extractions",
            ))
        }

        fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
            let TargetPath::Fs(path) = &self.declared[target.0 as usize].path else {
                return Err(Error::new(ErrorCode::Io, "file system targets only"));
            };
            let scope = store::scope(OpId(99), ScopePurpose::Operation);
            match mutation {
                Mutation::CreateDir { mode } => self.fs.create_dir(&scope, path, mode)?,
                Mutation::CopyFrom { source, .. } => {
                    self.fs.copy_file(&scope, &source, path)?;
                }
                Mutation::WriteBytes(bytes) => {
                    let mut handle = self.fs.create_new(&scope, path)?;
                    handle.write_all(&bytes).unwrap();
                }
                Mutation::CreateLink { target: link, kind } => {
                    self.fs.create_link(&scope, &link, path, kind)?;
                }
                _ => return Err(Error::new(ErrorCode::Io, "unexpected mutation")),
            }
            Ok(Applied {
                index: target,
                after: AfterState::default(),
            })
        }

        fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
            Ok(Vec::new())
        }

        fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {}

        fn record_exempt(&mut self, _write: ExemptWrite) {}

        fn commit(self: Box<Self>) -> Result<OpHandle> {
            Ok(OpHandle {
                op: Some(OpId(99)),
                kind: OpKind::command("journal.salvage"),
                bulk: true,
                targets: Vec::new(),
                irreversible: Vec::new(),
                notable: Vec::new(),
                exempt: Vec::new(),
            })
        }

        fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
            Ok(RollbackReport {
                op: Some(OpId(99)),
                restored: Vec::new(),
                failed: Vec::new(),
            })
        }
    }

    const JOURNAL: &str = "/v/.nurt-journal";

    fn entry(op: u64) -> EntryLayout {
        JournalLayout::new(abs(JOURNAL)).entry(OpId(op)).unwrap()
    }

    fn blob(op: u64, index: u32) -> AbsPath {
        entry(op)
            .blob(BlobId::new(TargetIndex(index), layout::BlobRole::Previous))
            .unwrap()
    }

    /// Manifest text of a corrupt entry: a deleted file and a deleted tree,
    /// with a recorded digest that does not match.
    fn manifest_text(op: u64) -> String {
        let file = BlobId::new(TargetIndex(0), layout::BlobRole::Previous).content_name();
        let tree = BlobId::new(TargetIndex(1), layout::BlobRole::Previous).content_name();
        serde_json::json!({
            "v": 1,
            "op": op,
            "time": 1_700_000_000_000_000_000_i64,
            "kind": { "command": "remove" },
            "targets": [
                { "index": 0, "path": { "kind": "fs", "at": "/v/home/user/a.txt" },
                  "before": { "store": "moved", "blob": file } },
                { "index": 1, "path": { "kind": "fs", "at": "/v/home/user/tree" },
                  "before": { "store": "moved", "blob": tree } }
            ],
            "integrity": { "algo": "sha256", "manifest_sha256": format!("sha256:{}", "0".repeat(64)), "chain": null }
        })
        .to_string()
    }

    /// Corrupt entry 1 with a file and a tree, a damaged-items entry 2 and a
    /// healthy entry 3 without manifest checks failing.
    fn journal(fs: &MemFs) {
        let corrupt = entry(1);
        fs.seed_file(
            &corrupt.state().unwrap(),
            &StateRecord::new(EntryState::Corrupt).encode(),
        )
        .unwrap();
        fs.seed_file(&corrupt.manifest().unwrap(), manifest_text(1).as_bytes())
            .unwrap();
        fs.seed_file(&blob(1, 0), b"file content").unwrap();
        fs.seed_file(&blob(1, 1).join_name("x.txt").unwrap(), b"x")
            .unwrap();
        fs.seed_file(
            &blob(1, 1)
                .join_name("sub")
                .unwrap()
                .join_name("y.txt")
                .unwrap(),
            b"y",
        )
        .unwrap();
        let damaged = entry(2);
        fs.seed_file(
            &damaged.state().unwrap(),
            &StateRecord::new(EntryState::Committed).encode(),
        )
        .unwrap();
        fs.seed_file(
            &damaged.items().unwrap(),
            b"{\"items\":[]}\ncrc32 00000000\n",
        )
        .unwrap();
    }

    #[test]
    fn lists_damaged_entries_with_readable_members() {
        let fs = mem();
        journal(&fs);
        let damaged = list(&*fs, &abs(JOURNAL)).unwrap();
        assert_eq!(damaged.len(), 2);
        let first = &damaged[0];
        assert_eq!(first.op, OpId(1));
        assert!(first.damage.contains(&Damage::Corrupt));
        assert!(first.damage.contains(&Damage::Manifest {
            problem: "digest_mismatch".to_owned()
        }));
        assert_eq!(first.command.as_deref(), Some("remove"));
        assert_eq!(first.targets.len(), 2);
        assert_eq!(first.targets[0].path.as_deref(), Some("/v/home/user/a.txt"));
        assert_eq!(first.blobs.len(), 2);
        assert_eq!(first.blobs[0].size, Some(12));
        assert_eq!(
            first.blobs[0].hash,
            Some(manifest::hash_bytes(b"file content"))
        );
        assert_eq!(first.blobs[1].kind, Some(FileKind::Directory));
        let second = &damaged[1];
        assert!(second.damage.contains(&Damage::ItemsDamaged));
        assert!(second.damage.contains(&Damage::Manifest {
            problem: "missing".to_owned()
        }));
    }

    #[test]
    fn extracts_preserved_data_under_recorded_paths_without_changing_the_journal() {
        let fs = mem();
        journal(&fs);
        let before = fs.snapshot(&abs(JOURNAL)).unwrap();
        let mut tx = DirectTx::new(&fs);
        let extraction = extract(&mut tx, &*fs, &abs(JOURNAL), OpId(1), &abs("/v/rescue")).unwrap();
        assert_eq!(fs.snapshot(&abs(JOURNAL)).unwrap(), before);
        let read = |path: &str| manifest::read_bytes(&*fs, &abs(path)).unwrap();
        assert_eq!(read("/v/rescue/v/home/user/a.txt"), b"file content");
        assert_eq!(read("/v/rescue/v/home/user/tree/x.txt"), b"x");
        assert_eq!(read("/v/rescue/v/home/user/tree/sub/y.txt"), b"y");
        let description: Value = serde_json::from_slice(&read("/v/rescue/salvage.json")).unwrap();
        assert_eq!(description["op"], 1);
        assert_eq!(extraction.items.len(), 3);
        assert!(
            extraction
                .items
                .iter()
                .all(|item| item.status == ItemStatus::Extracted)
        );
    }

    #[test]
    fn refuses_an_existing_destination_and_a_missing_entry() {
        let fs = mem();
        journal(&fs);
        fs.seed_dir(&abs("/v/rescue")).unwrap();
        let mut tx = DirectTx::new(&fs);
        let error = extract(&mut tx, &*fs, &abs(JOURNAL), OpId(1), &abs("/v/rescue")).unwrap_err();
        assert_eq!(error.code, ErrorCode::Exists);
        assert!(tx.declared.is_empty());
        let error = extract(&mut tx, &*fs, &abs(JOURNAL), OpId(7), &abs("/v/other")).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn extracts_what_is_readable_and_reports_the_rest() {
        let fs = mem();
        journal(&fs);
        fs.seed_file(&entry(1).manifest().unwrap(), b"").ok();
        let manifest_path = entry(1).manifest().unwrap();
        let scope = store::scope(OpId(1), ScopePurpose::Maintenance);
        let mut handle = fs.open_in_place(&scope, &manifest_path).unwrap();
        handle.set_length(0).unwrap();
        handle.write_all(b"\x00not json").unwrap();
        drop(handle);
        fs.deny_access(&blob(1, 0)).unwrap();
        let mut tx = DirectTx::new(&fs);
        let extraction = extract(&mut tx, &*fs, &abs(JOURNAL), OpId(1), &abs("/v/rescue")).unwrap();
        let unreadable: Vec<_> = extraction
            .items
            .iter()
            .filter(|item| item.status == ItemStatus::Unreadable)
            .collect();
        assert_eq!(unreadable.len(), 1);
        let raw = format!(
            "/v/rescue/{RAW_DIR}/{}.blob",
            BlobId::new(TargetIndex(1), layout::BlobRole::Previous)
        );
        assert_eq!(
            manifest::read_bytes(&*fs, &abs(&format!("{raw}/x.txt"))).unwrap(),
            b"x"
        );
        assert!(fs.metadata(&blob(1, 0), Follow::NoFollow).is_err());
        fs.clear_faults();
        assert_eq!(
            manifest::read_bytes(&*fs, &blob(1, 0)).unwrap(),
            b"file content"
        );
    }

    #[test]
    fn duplicate_places_get_numbered_names() {
        let mut plan = Plan::new(abs("/v/rescue"));
        let components = vec!["a".to_owned(), "b".to_owned()];
        let first = plan.place(&components).unwrap();
        let second = plan.place(&components).unwrap();
        assert_eq!(first, abs("/v/rescue/a/b"));
        assert_eq!(second, abs("/v/rescue/a/b.1"));
        assert_eq!(
            relative_components("C:/Users/user/a.txt"),
            vec!["C", "Users", "user", "a.txt"]
        );
    }

    #[test]
    fn healthy_entries_are_not_listed() {
        let fs = mem();
        let healthy = entry(4);
        fs.seed_file(
            &healthy.state().unwrap(),
            &StateRecord::new(EntryState::Committed).encode(),
        )
        .unwrap();
        let items = Items::default().encode().unwrap();
        fs.seed_file(&healthy.items().unwrap(), &items).unwrap();
        let mut manifest: Value = serde_json::from_str(&manifest_text(4)).unwrap();
        let digest = integrity::manifest_sha256(&manifest).unwrap();
        manifest["integrity"]["manifest_sha256"] = Value::String(digest.to_string());
        fs.seed_file(
            &healthy.manifest().unwrap(),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(list(&*fs, &abs(JOURNAL)).unwrap().is_empty());
        let scope = store::scope(OpId(4), ScopePurpose::Maintenance);
        fs.rename(
            &scope,
            &healthy.items().unwrap(),
            &abs("/v/items-moved"),
            RenameMode::NoReplace,
        )
        .unwrap();
        assert!(list(&*fs, &abs(JOURNAL)).unwrap().is_empty());
    }

    const STATE_DIR: &str = "/v/state";

    /// Seeds the committed manifest of entry `op` linked to `chain` and its
    /// index row; returns the digest of the manifest.
    fn seed_linked_entry(
        fs: &MemFs,
        op: u64,
        chain: Option<nurt_contracts::fs::ContentHash>,
    ) -> nurt_contracts::fs::ContentHash {
        let mut linked = manifest::tests::sample();
        linked.op = OpId(op);
        linked.integrity.chain = chain;
        let digest = linked.seal().unwrap();
        let bytes = linked.encode().unwrap();
        fs.seed_file(&entry(op).manifest().unwrap(), &bytes)
            .unwrap();
        append_row(fs, op, digest);
        digest
    }

    fn append_row(fs: &MemFs, op: u64, digest: nurt_contracts::fs::ContentHash) {
        let row = manifest::IndexRow {
            op: OpId(op),
            time: WallTime(1),
            kind: nurt_contracts::journal::OpKind::command("write"),
            session: None,
            bulk: false,
            counts_as_step: true,
            parts: vec![manifest::IndexPart {
                vol: nurt_contracts::fs::VolumeId {
                    dev: 1,
                    uuid: None,
                    subvolume: 0,
                },
                dir: abs(JOURNAL),
                manifest_sha256: digest,
            }],
        };
        let index = StateLayout::new(abs(STATE_DIR)).index().unwrap();
        let mut content = manifest::read_bytes(fs, &index).unwrap_or_default();
        content.extend_from_slice(&row.encode().unwrap());
        let state_dir = abs(STATE_DIR);
        fs.seed_dir(&state_dir).ok();
        let scope = store::scope(OpId(0), ScopePurpose::Maintenance);
        manifest::write_atomic(fs, &scope, &state_dir, &index, &content).unwrap();
    }

    #[test]
    fn verifies_the_chain_of_the_account_index() {
        let fs = mem();
        let first = seed_linked_entry(&fs, 1, None);
        let second = seed_linked_entry(&fs, 2, Some(first));
        seed_linked_entry(&fs, 3, Some(second));
        let report = verify_chain(&*fs, &abs(STATE_DIR)).unwrap();
        assert!(report.is_unbroken());
        assert!(report.forks.is_empty());
        assert!(
            report
                .links
                .iter()
                .all(|(_, status)| *status == LinkStatus::Verified)
        );
    }

    #[test]
    fn reports_forks_and_unverifiable_links_without_failing() {
        let fs = mem();
        let first = seed_linked_entry(&fs, 1, None);
        seed_linked_entry(&fs, 2, Some(first));
        seed_linked_entry(&fs, 3, Some(first));
        append_row(&fs, 4, manifest::hash_bytes(b"expired"));
        let report = verify_chain(&*fs, &abs(STATE_DIR)).unwrap();
        assert_eq!(
            report.links[2],
            (
                OpId(3),
                LinkStatus::Forked {
                    predecessor: OpId(1)
                }
            )
        );
        assert_eq!(report.links[3], (OpId(4), LinkStatus::Unverifiable));
        assert_eq!(report.forks.len(), 1);
        assert!(report.is_unbroken());
    }

    #[test]
    fn missing_index_yields_an_empty_report() {
        let fs = mem();
        let report = verify_chain(&*fs, &abs(STATE_DIR)).unwrap();
        assert!(report.links.is_empty());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn listing_never_panics_on_arbitrary_entry_files(
            state in proptest::collection::vec(any::<u8>(), 0..32),
            manifest_bytes in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let fs = mem();
            let damaged = entry(5);
            fs.seed_file(&damaged.state().unwrap(), &state).unwrap();
            fs.seed_file(&damaged.manifest().unwrap(), &manifest_bytes).unwrap();
            let listed = list(&*fs, &abs(JOURNAL)).unwrap();
            prop_assert_eq!(listed.len(), 1);
        }
    }
}
