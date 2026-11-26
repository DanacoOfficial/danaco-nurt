// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `archive.extract`: extracts an archive into a directory.
//!
//! The plan step reads the headers of the archive once
//! ([`nurt_archive::extract::plan`]): members whose path or link target
//! would resolve outside `into` are rejected and reported with the notable
//! `archive_path_escape`, unless `allow_outside` is set. The plan then
//! checks every destination on disk: a destination below an existing
//! symbolic link inside `into` is rejected the same way, and an existing
//! entry is handled by `on_conflict`.
//!
//! The apply step declares every target before the first change, creates
//! the directories (parents first), streams the content of the regular
//! files from a second pass over the archive into staged files of the
//! transaction, and creates hard links and symbolic links last, so that no
//! file is written through a link created by the archive. Content is never
//! held in memory beyond one copy buffer.
//!
//! Regular files are created with the permissions stored in the archive
//! (`preserve_permissions: true`) without the set-id bits, which only a
//! privileged extraction that states the option keeps, or with those
//! permissions without the set-id, sticky and group and other write bits. Directories are created
//! with their permissions and always with full access for the account, so
//! that their content can be written. Modification times are not restored:
//! a new file has the time it was written.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use nurt_archive::extract::{ExtractPlan, Limits, PlannedEntry, plan, stream};
use nurt_archive::list::{ArchiveError, MemberKind};
use nurt_archive::safety::{Rules, Violation};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Etag, ExtendedMeta, FileKind, Follow, Fs, FsErrorKind, LinkKind,
};
use nurt_contracts::journal::{
    Applied, Expect, Mutation, MutationTx, Preserved, TargetAction, TargetIndex, TargetSpec,
};
use nurt_contracts::record::Change;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageDescriptor,
    StageImpl, StageInput,
};
use nurt_contracts::value::Size;
use nurt_contracts::value::Value;
use serde_json::Value as Json;

use super::{
    Carried, PATH, choice, count, effect_output, flag, member_path, missing_state, path_filter,
    required_path, unknown_value,
};
use crate::common::etag::{Observed, check_if_match, observe};
use crate::common::mutation::{applied_change, fs_target, planned_change};
use crate::common::notable::counted;

const COMMAND: &str = "archive.extract";
const INTO: &str = "into";
const ONLY: &str = "only";
const STRIP_COMPONENTS: &str = "strip_components";
const ON_CONFLICT: &str = "on_conflict";
const PRESERVE_PERMISSIONS: &str = "preserve_permissions";
const ALLOW_OUTSIDE: &str = "allow_outside";
const ON_CONFLICT_VALUES: [&str; 3] = ["overwrite", "skip", "error"];

/// Permissions of a regular file whose member records none.
const DEFAULT_FILE_MODE: u32 = 0o644;
/// Permissions of a directory whose member records none, and of the
/// directories created for `into` and for parents without a member.
const DEFAULT_DIR_MODE: u32 = 0o755;
/// Permission bits applied, including the set-id and sticky bits.
const PERMISSION_BITS: u32 = 0o7777;
/// Set-user-id and set-group-id bits, applied only to a privileged
/// extraction that asks for the stored permissions explicitly.
const SET_ID_BITS: u32 = 0o6000;
/// Name of the account that may extract set-id bits.
const ROOT_ACCOUNT: &str = "root";
/// Bits kept with `preserve_permissions: false`: no set-id or sticky bits
/// and no write access for the group and others.
const UNPRESERVED_BITS: u32 = 0o755;
/// Read, write and search access of the account, always given to created
/// directories.
const ACCOUNT_ACCESS: u32 = 0o700;
/// Size of the buffer that copies member content into a staged file.
const COPY_BUFFER: usize = 64 * 1024;

/// Stage of `archive.extract`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ArchiveExtract;

impl StageImpl for ArchiveExtract {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// Value of `on_conflict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnConflict {
    Overwrite,
    Skip,
    Error,
}

/// Settings of one call.
#[derive(Debug, Clone, Copy)]
struct Settings {
    on_conflict: OnConflict,
    preserve_permissions: bool,
    /// Whether the set-user-id and set-group-id bits of the archive are applied.
    keep_set_id: bool,
    allow_outside: bool,
    strip_components: usize,
}

impl Settings {
    /// Reads the parameters; `privileged` tells whether the process runs as
    /// `root`.
    fn read(args: &BoundArgs, privileged: bool) -> Result<Self> {
        let on_conflict = match choice(args, ON_CONFLICT)? {
            None | Some("overwrite") => OnConflict::Overwrite,
            Some("skip") => OnConflict::Skip,
            Some("error") => OnConflict::Error,
            Some(other) => return Err(unknown_value(ON_CONFLICT, other, &ON_CONFLICT_VALUES)),
        };
        let strip = args.int(STRIP_COMPONENTS)?.unwrap_or(0);
        let strip_components = usize::try_from(strip).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{STRIP_COMPONENTS}` must be 0 or more, not {strip}"),
            )
            .with_found(strip.to_string())
        })?;
        let preserve_permissions = flag(args, PRESERVE_PERMISSIONS, true)?;
        let stated = matches!(args.value(PRESERVE_PERMISSIONS), Some(Value::Bool(true)));
        Ok(Self {
            on_conflict,
            preserve_permissions,
            keep_set_id: preserve_permissions && stated && privileged,
            allow_outside: flag(args, ALLOW_OUTSIDE, false)?,
            strip_components,
        })
    }

    /// Bits of a stored mode that are applied.
    fn kept_bits(self) -> u32 {
        if !self.preserve_permissions {
            UNPRESERVED_BITS
        } else if self.keep_set_id {
            PERMISSION_BITS
        } else {
            PERMISSION_BITS & !SET_ID_BITS
        }
    }

    fn file_mode(self, stored: Option<u32>) -> u32 {
        stored.unwrap_or(DEFAULT_FILE_MODE) & self.kept_bits()
    }

    fn dir_mode(self, stored: Option<u32>) -> u32 {
        (stored.unwrap_or(DEFAULT_DIR_MODE) & self.kept_bits()) | ACCOUNT_ACCESS
    }
}

/// Change applied to one declared target.
#[derive(Debug, Clone)]
enum Step {
    /// Create a directory.
    Directory { mode: u32 },
    /// Write the content of the member with this ordinal; `mode` is applied
    /// to a new file, an overwritten file keeps its own metadata.
    File { ordinal: u64, mode: Option<u32> },
    /// Create a hard link to an extracted or existing file.
    HardLink { existing: AbsPath },
    /// Create a symbolic link.
    Symlink { target: PathBuf, kind: LinkKind },
}

/// Counters of the `effect` record.
#[derive(Debug, Clone, Copy, Default)]
struct Summary {
    directories: u64,
    files: u64,
    links: u64,
    bytes: u64,
    overwritten: u64,
    unchanged: u64,
    skipped: u64,
    rejected: u64,
    special: u64,
}

impl Summary {
    fn pairs(self) -> [(&'static str, u64); 9] {
        [
            ("directories", self.directories),
            ("files", self.files),
            ("links", self.links),
            ("bytes", self.bytes),
            ("overwritten", self.overwritten),
            ("unchanged", self.unchanged),
            ("skipped", self.skipped),
            ("rejected", self.rejected),
            ("special", self.special),
        ]
    }
}

/// Private state of the plan.
#[derive(Debug)]
struct ExtractState {
    archive: AbsPath,
    archive_tag: Etag,
    /// The plan of the library with only the regular files to write.
    files: ExtractPlan,
    targets: Vec<TargetSpec>,
    steps: Vec<Step>,
    summary: Summary,
}

impl EffectStage for ArchiveExtract {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let archive = required_path(args, PATH, ctx)?;
        let into = required_path(args, INTO, ctx)?;
        let settings = Settings::read(args, ctx.host().info().user == ROOT_ACCOUNT)?;
        let filter = path_filter(args, ONLY)?;
        let select = filter
            .as_ref()
            .map(|filter| move |path: &str| filter.matches(path));
        let rules = Rules {
            only: select
                .as_ref()
                .map(|select| select as &dyn Fn(&str) -> bool),
            strip_components: settings.strip_components,
            allow_outside: settings.allow_outside,
        };
        let reader = fs.open_read(&archive)?;
        let archive_tag = reader.meta()?.etag();
        let library_plan = plan(reader, rules, &Limits::default())
            .map_err(|error| super::archive_error(&error, &archive))?;

        let mut planner = Planner::new(fs, archive, into, settings);
        planner.prepare_into()?;
        planner.report_library_plan(&library_plan);
        let ExtractPlan {
            format,
            entries,
            rejected,
            special,
            skipped,
            total_size,
        } = library_plan;
        planner.summary.skipped += skipped;
        let mut kept = Vec::new();
        for entry in entries {
            ctx.checkpoint()?;
            if let Some(entry) = planner.place(entry)? {
                kept.push(entry);
            }
        }
        let files = ExtractPlan {
            format,
            entries: kept,
            rejected,
            special,
            skipped,
            total_size,
        };
        Ok(planner.finish(archive_tag, files))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let fs = ctx.require_fs()?;
        let state = plan
            .take_state::<ExtractState>()
            .ok_or_else(|| missing_state(COMMAND))?;
        let reader = fs.open_read(&state.archive)?;
        let current = Observed::Present(reader.meta()?);
        check_if_match(&state.archive, &current, Some(&state.archive_tag))?;

        let preserved = tx.preserve(&state.targets)?;
        let mut applied: Vec<Option<Applied>> = vec![None; state.steps.len()];
        let mut by_ordinal = HashMap::new();
        for (slot, step) in state.steps.iter().enumerate() {
            match step {
                Step::Directory { mode } => {
                    ctx.checkpoint()?;
                    applied[slot] =
                        Some(tx.apply(preserved[slot].index, Mutation::CreateDir { mode: *mode })?);
                }
                Step::File { ordinal, mode } => {
                    by_ordinal.insert(*ordinal, (slot, *mode));
                }
                Step::HardLink { .. } | Step::Symlink { .. } => {}
            }
        }

        let mut carried = Carried::default();
        let mut buffer = vec![0_u8; COPY_BUFFER];
        let streamed = stream(reader, &state.files, |entry, content| {
            let Some(&(slot, mode)) = by_ordinal.get(&entry.ordinal) else {
                return Ok(());
            };
            let writer = FileWriter {
                fs,
                ctx,
                index: preserved[slot].index,
                mode,
            };
            match writer.write(&mut *tx, content, &mut buffer) {
                Ok(done) => {
                    applied[slot] = Some(done);
                    Ok(())
                }
                Err(Failure::Archive(error)) => Err(error),
                Err(Failure::Stage(error)) => Err(carried.stop(error)),
            }
        });
        streamed.map_err(|error| carried.resolve(&error, &state.archive))?;

        for (slot, step) in state.steps.iter().enumerate() {
            let mutation = match step {
                Step::HardLink { existing } => Mutation::CreateHardLink {
                    existing: existing.clone(),
                },
                Step::Symlink { target, kind } => Mutation::CreateLink {
                    target: target.clone(),
                    kind: *kind,
                },
                Step::Directory { .. } | Step::File { .. } => continue,
            };
            ctx.checkpoint()?;
            applied[slot] = Some(tx.apply(preserved[slot].index, mutation)?);
        }

        let changes = applied_changes(&state.targets, &preserved, applied, &state.archive)?;
        let output = effect_output(
            COMMAND,
            &*tx,
            &changes,
            &plan.notable,
            &state.summary.pairs(),
        );
        let mut outcome = EffectOutcome::new(output);
        outcome.changes = changes;
        Ok(outcome)
    }
}

/// `change` records of the applied targets, in declaration order.
fn applied_changes(
    targets: &[TargetSpec],
    preserved: &[Preserved],
    applied: Vec<Option<Applied>>,
    archive: &AbsPath,
) -> Result<Vec<Change>> {
    targets
        .iter()
        .zip(preserved)
        .zip(applied)
        .map(|((target, preserved), applied)| {
            applied
                .map(|applied| applied_change(target, preserved, &applied))
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::Stale,
                        format!(
                            "`{archive}` changed while it was extracted: run the command again"
                        ),
                    )
                    .with_detail("archive", Json::from(archive.to_language_string()))
                })
        })
        .collect()
}

/// Failure while one member is written: an error of the archive, which the
/// library classifies, or an error of the stage.
enum Failure {
    Archive(ArchiveError),
    Stage(Error),
}

/// Writer of the content of one regular file into its staged file.
struct FileWriter<'a> {
    fs: &'a dyn Fs,
    ctx: &'a ExecCtx,
    index: TargetIndex,
    mode: Option<u32>,
}

impl FileWriter<'_> {
    fn write(
        &self,
        tx: &mut dyn MutationTx,
        content: &mut dyn Read,
        buffer: &mut [u8],
    ) -> std::result::Result<Applied, Failure> {
        self.ctx.checkpoint().map_err(Failure::Stage)?;
        let mut handle = tx.stage(self.index).map_err(Failure::Stage)?;
        let staged = handle.path().clone();
        let write_error = |error: std::io::Error| {
            Failure::Stage(
                Error::new(
                    ErrorCode::Io,
                    format!("cannot write the staged file `{staged}`: {error}"),
                )
                .with_detail("path", Json::from(staged.to_language_string())),
            )
        };
        loop {
            let read = content
                .read(buffer)
                .map_err(|error| Failure::Archive(ArchiveError::Io(error)))?;
            if read == 0 {
                break;
            }
            handle.write_all(&buffer[..read]).map_err(write_error)?;
        }
        handle.flush().map_err(write_error)?;
        if let Some(mode) = self.mode {
            let mut basic = self
                .fs
                .metadata(&staged, Follow::NoFollow)
                .map_err(|error| Failure::Stage(error.into()))?;
            basic.mode = (basic.mode & !PERMISSION_BITS) | mode;
            let meta = ExtendedMeta {
                basic,
                xattrs: Vec::new(),
                acl: None,
                account_sid: None,
                flags: None,
                streams: Vec::new(),
                link_target: None,
            };
            handle
                .apply_meta(&meta)
                .map_err(|error| Failure::Stage(error.into()))?;
        }
        drop(handle);
        tx.apply(self.index, Mutation::CommitStaged)
            .map_err(Failure::Stage)
    }
}

/// What the plan knows about a directory on the way to a destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Known {
    /// An existing directory, or a link to one that may be passed.
    Directory,
    /// A directory the plan creates.
    Planned,
    /// An existing symbolic link inside `into`.
    Link,
    /// A file, link or special file the plan creates.
    Occupied,
}

/// Outcome of preparing the parents of a destination.
enum Parents {
    Ready,
    ThroughLink,
}

/// Destination checks and target list of one plan.
struct Planner<'a> {
    fs: &'a dyn Fs,
    archive: AbsPath,
    into: AbsPath,
    settings: Settings,
    known: HashMap<AbsPath, Known>,
    directories: BTreeMap<AbsPath, u32>,
    files: Vec<(TargetSpec, Step)>,
    hard_links: Vec<(TargetSpec, Step)>,
    symlinks: Vec<(TargetSpec, Step)>,
    unavailable: HashSet<String>,
    notable: Vec<NotableEntry>,
    summary: Summary,
}

impl<'a> Planner<'a> {
    fn new(fs: &'a dyn Fs, archive: AbsPath, into: AbsPath, settings: Settings) -> Self {
        Self {
            fs,
            archive,
            into,
            settings,
            known: HashMap::new(),
            directories: BTreeMap::new(),
            files: Vec::new(),
            hard_links: Vec::new(),
            symlinks: Vec::new(),
            unavailable: HashSet::new(),
            notable: Vec::new(),
            summary: Summary::default(),
        }
    }

    /// Checks `into` and plans it with its missing parents when it does not
    /// exist.
    fn prepare_into(&mut self) -> Result<()> {
        match self.fs.metadata(&self.into, Follow::Follow) {
            Ok(meta) if meta.kind == FileKind::Directory => {
                self.known.insert(self.into.clone(), Known::Directory);
                Ok(())
            }
            Ok(_) => Err(Error::new(
                ErrorCode::Exists,
                format!(
                    "`{}` is not a directory: give a directory or a new path for `{INTO}`",
                    self.into
                ),
            )
            .with_found(self.into.to_language_string())),
            Err(error) if error.kind == FsErrorKind::NotFound => {
                let into = self.into.clone();
                self.create_missing(&into)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Reports the members the library rejected or does not extract.
    fn report_library_plan(&mut self, plan: &ExtractPlan) {
        for rejected in &plan.rejected {
            self.escape(&rejected.path, rejected.violation, false);
        }
        for path in &plan.special {
            self.notable.push(NotableEntry {
                path: Some(member_path(&self.archive, path)),
                ..NotableEntry::new(Notable::SpecialFile)
            });
        }
        self.summary.special = count(plan.special.len());
        for entry in plan.entries.iter().filter(|entry| entry.waived.is_some()) {
            if let Some(violation) = entry.waived {
                self.escape(&entry.member.path, violation, true);
            }
        }
    }

    /// Records an `archive_path_escape` entry for a member.
    fn escape(&mut self, member: &str, violation: Violation, extracted: bool) {
        let detail = if extracted {
            format!("{}, extracted with {ALLOW_OUTSIDE}", violation.name())
        } else {
            self.summary.rejected += 1;
            violation.name().to_owned()
        };
        self.notable.push(NotableEntry {
            path: Some(member_path(&self.archive, member)),
            detail: Some(detail),
            ..NotableEntry::new(Notable::ArchivePathEscape)
        });
    }

    /// Plans one member; returns the entry when its content is streamed.
    fn place(&mut self, entry: PlannedEntry) -> Result<Option<PlannedEntry>> {
        let target = self.destination(&entry.dest)?;
        let waived = entry.waived.is_some();
        if entry
            .hard_link_to
            .as_ref()
            .is_some_and(|to| self.unavailable.contains(to))
        {
            self.reject(&entry, Violation::HardLinkOutside);
            return Ok(None);
        }
        if let Parents::ThroughLink = self.prepare_parents(&target, waived)? {
            self.reject(&entry, Violation::ThroughLink);
            return Ok(None);
        }
        if self.known.get(&target) == Some(&Known::Planned)
            && entry.member.kind != MemberKind::Directory
        {
            return Err(collision_error(&target));
        }
        let observed = observe(self.fs, &target)?;
        match entry.member.kind {
            MemberKind::Directory => {
                self.place_directory(&entry, target, &observed)?;
                Ok(None)
            }
            MemberKind::File => self.place_file(entry, target, &observed),
            MemberKind::Symlink => {
                self.place_symlink(&entry, target, &observed)?;
                Ok(None)
            }
            MemberKind::HardLink => {
                self.place_hard_link(&entry, target, &observed)?;
                Ok(None)
            }
            MemberKind::Other => Ok(None),
        }
    }

    /// Absolute destination of a planned member.
    fn destination(&self, dest: &str) -> Result<AbsPath> {
        let joined = if dest.starts_with(super::SEPARATOR) {
            PathBuf::from(dest)
        } else {
            self.into.as_path().join(dest)
        };
        AbsPath::new(joined).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("member `{dest}` leads above the root directory"),
            )
            .with_found(dest)
        })
    }

    fn reject(&mut self, entry: &PlannedEntry, violation: Violation) {
        self.unavailable.insert(entry.dest.clone());
        self.escape(&entry.member.path, violation, false);
    }

    /// Plans the missing parents of `target`; reports a parent that is an
    /// existing symbolic link inside `into`, unless `allow_outside` is set.
    fn prepare_parents(&mut self, target: &AbsPath, waived: bool) -> Result<Parents> {
        let mut current = target.parent();
        let mut missing = Vec::new();
        while let Some(dir) = current {
            match self.known.get(&dir) {
                Some(Known::Directory | Known::Planned) => break,
                Some(Known::Link) if !self.settings.allow_outside && !waived => {
                    return Ok(Parents::ThroughLink);
                }
                Some(Known::Occupied) => return Err(collision_error(&dir)),
                Some(Known::Link) | None => {}
            }
            match observe(self.fs, &dir)? {
                Observed::Absent => missing.push(dir.clone()),
                Observed::Present(meta) => {
                    if meta.kind == FileKind::Directory {
                        self.known.insert(dir, Known::Directory);
                        break;
                    }
                    if !matches!(meta.kind, FileKind::Symlink | FileKind::Junction) {
                        return Err(not_a_directory_error(&dir));
                    }
                    let inside = dir != self.into && dir.starts_with(&self.into);
                    if inside && !self.settings.allow_outside && !waived {
                        self.known.insert(dir, Known::Link);
                        return Ok(Parents::ThroughLink);
                    }
                    let followed = self.fs.metadata(&dir, Follow::Follow)?;
                    if followed.kind != FileKind::Directory {
                        return Err(not_a_directory_error(&dir));
                    }
                    self.known.insert(dir, Known::Directory);
                    break;
                }
            }
            current = dir.parent();
        }
        for dir in missing.into_iter().rev() {
            self.plan_directory(dir, DEFAULT_DIR_MODE);
        }
        Ok(Parents::Ready)
    }

    /// Plans `path` and its missing parents as new directories with the
    /// default permissions.
    fn create_missing(&mut self, path: &AbsPath) -> Result<()> {
        let mut missing = vec![path.clone()];
        let mut current = path.parent();
        while let Some(dir) = current {
            if matches!(
                self.known.get(&dir),
                Some(Known::Directory | Known::Planned)
            ) {
                break;
            }
            match self.fs.metadata(&dir, Follow::Follow) {
                Ok(meta) if meta.kind == FileKind::Directory => {
                    self.known.insert(dir, Known::Directory);
                    break;
                }
                Ok(_) => return Err(not_a_directory_error(&dir)),
                Err(error) if error.kind == FsErrorKind::NotFound => {
                    missing.push(dir.clone());
                }
                Err(error) => return Err(error.into()),
            }
            current = dir.parent();
        }
        for dir in missing.into_iter().rev() {
            self.plan_directory(dir, DEFAULT_DIR_MODE);
        }
        Ok(())
    }

    fn plan_directory(&mut self, path: AbsPath, mode: u32) {
        self.directories.insert(path.clone(), mode);
        self.known.insert(path, Known::Planned);
    }

    fn place_directory(
        &mut self,
        entry: &PlannedEntry,
        target: AbsPath,
        observed: &Observed,
    ) -> Result<()> {
        let mode = self.settings.dir_mode(entry.member.mode);
        if self.known.get(&target) == Some(&Known::Planned) {
            self.directories.insert(target, mode);
            return Ok(());
        }
        match observed.meta().map(|meta| meta.kind) {
            None => {
                self.plan_directory(target, mode);
                Ok(())
            }
            Some(FileKind::Directory) => {
                self.known.insert(target, Known::Directory);
                self.summary.unchanged += 1;
                Ok(())
            }
            Some(kind) => self.conflict(entry, &target, kind),
        }
    }

    fn place_file(
        &mut self,
        entry: PlannedEntry,
        target: AbsPath,
        observed: &Observed,
    ) -> Result<Option<PlannedEntry>> {
        let mode = self.settings.file_mode(entry.member.mode);
        let (spec, mode) = match observed {
            Observed::Absent => (
                fs_target(&target, TargetAction::Create, Expect::Absent),
                Some(mode),
            ),
            Observed::Present(meta) if meta.kind == FileKind::File => {
                match self.settings.on_conflict {
                    OnConflict::Overwrite => {
                        self.summary.overwritten += 1;
                        let mut spec = fs_target(&target, TargetAction::Modify, observed.expect());
                        spec.size_hint = Some(meta.size);
                        (spec, None)
                    }
                    OnConflict::Skip => {
                        self.summary.skipped += 1;
                        return Ok(None);
                    }
                    OnConflict::Error => return Err(exists_error(&target, None)),
                }
            }
            Observed::Present(meta) => {
                self.conflict(&entry, &target, meta.kind)?;
                return Ok(None);
            }
        };
        self.known.insert(target, Known::Occupied);
        self.summary.files += 1;
        self.summary.bytes = self.summary.bytes.saturating_add(entry.member.size);
        self.files.push((
            spec,
            Step::File {
                ordinal: entry.ordinal,
                mode,
            },
        ));
        Ok(Some(entry))
    }

    fn place_symlink(
        &mut self,
        entry: &PlannedEntry,
        target: AbsPath,
        observed: &Observed,
    ) -> Result<()> {
        let link_target = PathBuf::from(entry.member.link_target.clone().unwrap_or_default());
        if let Some(meta) = observed.meta() {
            let same = meta.kind == FileKind::Symlink
                && self
                    .fs
                    .read_link(&target)
                    .is_ok_and(|stored| stored == link_target);
            if same {
                self.summary.unchanged += 1;
                return Ok(());
            }
            return self.conflict(entry, &target, meta.kind);
        }
        let kind = if self.points_to_directory(&target, &link_target) {
            LinkKind::SymlinkDirectory
        } else {
            LinkKind::SymlinkFile
        };
        self.known.insert(target.clone(), Known::Occupied);
        self.summary.links += 1;
        self.symlinks.push((
            fs_target(&target, TargetAction::Link, Expect::Absent),
            Step::Symlink {
                target: link_target,
                kind,
            },
        ));
        Ok(())
    }

    fn place_hard_link(
        &mut self,
        entry: &PlannedEntry,
        target: AbsPath,
        observed: &Observed,
    ) -> Result<()> {
        if let Some(meta) = observed.meta() {
            return self.conflict(entry, &target, meta.kind);
        }
        let Some(to) = entry.hard_link_to.as_deref() else {
            self.reject(entry, Violation::HardLinkOutside);
            return Ok(());
        };
        let existing = self.destination(to)?;
        self.known.insert(target.clone(), Known::Occupied);
        self.summary.links += 1;
        self.hard_links.push((
            fs_target(&target, TargetAction::Link, Expect::Absent),
            Step::HardLink { existing },
        ));
        Ok(())
    }

    /// Whether a symbolic link at `link` with the stored target names a
    /// directory planned or existing; decides the kind of link on Windows.
    fn points_to_directory(&self, link: &AbsPath, target: &Path) -> bool {
        let Some(parent) = link.parent() else {
            return false;
        };
        let Ok(resolved) = AbsPath::new(parent.as_path().join(target)) else {
            return false;
        };
        self.directories.contains_key(&resolved)
            || self
                .fs
                .metadata(&resolved, Follow::Follow)
                .is_ok_and(|meta| meta.kind == FileKind::Directory)
    }

    /// Handles an existing entry of another kind than the member.
    fn conflict(&mut self, entry: &PlannedEntry, target: &AbsPath, kind: FileKind) -> Result<()> {
        match self.settings.on_conflict {
            OnConflict::Skip => {
                self.unavailable.insert(entry.dest.clone());
                self.summary.skipped += 1;
                Ok(())
            }
            OnConflict::Overwrite | OnConflict::Error => Err(exists_error(target, Some(kind))),
        }
    }

    fn finish(self, archive_tag: Etag, files: ExtractPlan) -> EffectPlan {
        let Self {
            archive,
            directories,
            files: file_steps,
            hard_links,
            symlinks,
            mut notable,
            mut summary,
            ..
        } = self;
        summary.directories = count(directories.len());
        let mut targets = Vec::new();
        let mut steps = Vec::new();
        for (path, mode) in directories {
            targets.push(fs_target(&path, TargetAction::Create, Expect::Absent));
            steps.push(Step::Directory { mode });
        }
        for (spec, step) in file_steps.into_iter().chain(hard_links).chain(symlinks) {
            targets.push(spec);
            steps.push(step);
        }
        if summary.overwritten > 0 {
            notable.push(counted(Notable::OverwroteExisting, summary.overwritten));
        }
        let changes = targets
            .iter()
            .zip(&steps)
            .map(|(target, step)| planned(target, step, &files))
            .collect();
        let mut plan = EffectPlan::new();
        plan.changes = changes;
        plan.notable = notable;
        plan.bulk = targets.len() > 1;
        plan.with_state(ExtractState {
            archive,
            archive_tag,
            files,
            targets,
            steps,
            summary,
        })
    }
}

/// Planned `change` record, with the size of the extracted content for
/// files.
fn planned(target: &TargetSpec, step: &Step, files: &ExtractPlan) -> Change {
    let mut change = planned_change(target);
    if let Step::File { ordinal, .. } = step {
        change.size_after = files
            .entries
            .iter()
            .find(|entry| entry.ordinal == *ordinal)
            .map(|entry| Size::from_bytes(entry.member.size));
    }
    change
}

fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::File => "file",
        FileKind::Directory => "directory",
        FileKind::Symlink | FileKind::Junction => "link",
        _ => "special file",
    }
}

/// `E_EXISTS` for a destination that exists; `kind` when it differs from
/// the member.
fn exists_error(target: &AbsPath, kind: Option<FileKind>) -> Error {
    let message = match kind {
        Some(kind) => format!(
            "`{target}` exists as a {}: remove it or set {ON_CONFLICT}: skip",
            kind_name(kind)
        ),
        None => format!("`{target}` exists: set {ON_CONFLICT}: overwrite or skip"),
    };
    Error::new(ErrorCode::Exists, message)
        .with_detail("path", Json::from(target.to_language_string()))
}

/// `E_EXISTS` for a path the archive uses both as a directory and as
/// another kind of entry.
fn collision_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!(
            "the archive uses `{path}` both as a directory and as a file or link: extract the members separately with `{ONLY}`"
        ),
    )
    .with_detail("path", Json::from(path.to_language_string()))
}

/// `E_EXISTS` for an existing entry that is needed as a directory.
fn not_a_directory_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!("`{path}` is not a directory: remove it or extract into another directory"),
    )
    .with_detail("path", Json::from(path.to_language_string()))
}

#[cfg(test)]
mod tests {
    use nurt_archive::create::{CreateOptions, Input, create};
    use nurt_archive::list::Format;
    use nurt_contracts::journal::{JournalStore, TargetPath};
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::JournalCall;

    use super::*;

    fn abs(path: &str) -> nurt_contracts::fs::AbsPath {
        nurt_testkit::vfs::test_path(path).unwrap()
    }

    fn zip(inputs: Vec<Input>, content: &'static [u8]) -> Vec<u8> {
        create(Vec::new(), &CreateOptions::new(Format::Zip), inputs, |_| {
            Ok(Box::new(content))
        })
        .unwrap()
    }

    fn args(given: &[(&str, Value)]) -> BoundArgs {
        let base = BoundArgs::new(COMMAND)
            .with(
                PATH,
                ArgValue::Value(Value::Path(abs("/in/a.zip").to_language_string())),
                false,
            )
            .with(
                INTO,
                ArgValue::Value(Value::Path(abs("/out").to_language_string())),
                false,
            );
        given.iter().fold(base, |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
    }

    fn extract(test: &TestContext, args: &BoundArgs) -> Result<Vec<Change>> {
        let ctx = test.ctx();
        let plan = ArchiveExtract.plan(args, StageInput::none(), &ctx)?;
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = ArchiveExtract.apply(args, plan, tx.as_mut(), &ctx)?;
        let changes = outcome.changes;
        tx.commit()?;
        Ok(changes)
    }

    #[test]
    fn settings_read_every_parameter() {
        let given = args(&[
            (ON_CONFLICT, Value::Enum("error".to_owned())),
            (STRIP_COMPONENTS, Value::Int(2)),
            (PRESERVE_PERMISSIONS, Value::Bool(false)),
            (ALLOW_OUTSIDE, Value::Bool(true)),
        ]);
        let settings = Settings::read(&given, false).unwrap();
        assert_eq!(settings.on_conflict, OnConflict::Error);
        assert_eq!(settings.strip_components, 2);
        assert!(!settings.preserve_permissions);
        assert!(settings.allow_outside);
        assert_eq!(settings.file_mode(Some(0o4777)), 0o755);
        assert_eq!(settings.dir_mode(Some(0o500)), 0o700);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let negative =
            Settings::read(&args(&[(STRIP_COMPONENTS, Value::Int(-1))]), false).unwrap_err();
        assert_eq!(negative.code(), ErrorCode::InvalidValue);
        let unknown = Settings::read(
            &args(&[(ON_CONFLICT, Value::Enum("merge".to_owned()))]),
            false,
        )
        .unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::UnknownValue);
    }

    #[test]
    fn set_id_bits_are_applied_only_to_a_privileged_extraction_that_asks_for_them() {
        let stated = args(&[(PRESERVE_PERMISSIONS, Value::Bool(true))]);
        let privileged = Settings::read(&stated, true).unwrap();
        assert_eq!(privileged.file_mode(Some(0o106_755)), 0o6755);
        assert_eq!(privileged.dir_mode(Some(0o103_755)), 0o3755);
        for settings in [
            Settings::read(&stated, false).unwrap(),
            Settings::read(&args(&[]), true).unwrap(),
            Settings::read(&args(&[]), false).unwrap(),
        ] {
            assert_eq!(settings.file_mode(Some(0o106_755)), 0o755);
            assert_eq!(settings.file_mode(Some(0o101_755)), 0o1755);
            assert_eq!(settings.dir_mode(Some(0o102_755)), 0o755);
            assert_eq!(settings.file_mode(None), DEFAULT_FILE_MODE);
            assert_eq!(settings.dir_mode(None), DEFAULT_DIR_MODE);
        }
    }

    #[test]
    fn extracts_files_with_parents_and_permissions() {
        let test = TestContext::new();
        let archive = zip(vec![Input::file("a/b/c.sh", 2).with_mode(0o750)], b"ok");
        test.fs().write_file(&abs("/in/a.zip"), &archive).unwrap();
        let changes = extract(&test, &args(&[])).unwrap();
        assert_eq!(changes.len(), 4);
        let file = abs("/out/a/b/c.sh");
        assert_eq!(test.fs().read_file(&file).unwrap(), b"ok");
        let meta = test.fs().metadata(&file, Follow::NoFollow).unwrap();
        assert_eq!(meta.mode & PERMISSION_BITS, 0o750);
    }

    #[test]
    fn existing_files_are_overwritten_skipped_or_refused() {
        let test = TestContext::new();
        let archive = zip(vec![Input::file("a.txt", 3)], b"new");
        test.fs().write_file(&abs("/in/a.zip"), &archive).unwrap();
        let existing = abs("/out/a.txt");
        test.fs().write_file(&existing, b"old").unwrap();

        let refused = extract(
            &test,
            &args(&[(ON_CONFLICT, Value::Enum("error".to_owned()))]),
        )
        .unwrap_err();
        assert_eq!(refused.code(), ErrorCode::Exists);
        let skipped = extract(
            &test,
            &args(&[(ON_CONFLICT, Value::Enum("skip".to_owned()))]),
        );
        assert!(skipped.unwrap().is_empty());
        assert_eq!(test.fs().read_file(&existing).unwrap(), b"old");

        let changes = extract(&test, &args(&[])).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(test.fs().read_file(&existing).unwrap(), b"new");
    }

    #[test]
    fn identical_links_and_directories_are_left_unchanged() {
        let test = TestContext::new();
        let archive = zip(vec![Input::directory("d"), Input::symlink("l", "d")], b"");
        test.fs().write_file(&abs("/in/a.zip"), &archive).unwrap();
        assert_eq!(extract(&test, &args(&[])).unwrap().len(), 3);
        assert!(extract(&test, &args(&[])).unwrap().is_empty());
    }

    #[test]
    fn into_must_be_a_directory() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/in/a.zip"), &zip(vec![Input::file("a", 1)], b"x"))
            .unwrap();
        test.fs().write_file(&abs("/out"), b"file").unwrap();
        let error = extract(&test, &args(&[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
    }

    #[test]
    fn plans_declare_the_targets_the_apply_step_changes() {
        let test = TestContext::new();
        let archive = zip(
            vec![
                Input::directory("d"),
                Input::file("d/f", 1),
                Input::symlink("d/l", "f"),
            ],
            b"x",
        );
        test.fs().write_file(&abs("/in/a.zip"), &archive).unwrap();
        extract(&test, &args(&[])).unwrap();
        let declared: Vec<TargetPath> = test
            .journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Preserve { targets, .. } => Some(targets),
                _ => None,
            })
            .flatten()
            .collect();
        let expected: Vec<TargetPath> = ["/out", "/out/d", "/out/d/f", "/out/d/l"]
            .iter()
            .map(|path| TargetPath::Fs(abs(path)))
            .collect();
        assert_eq!(declared, expected);
    }
}
