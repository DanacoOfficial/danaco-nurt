// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Planning of the file changes of a reference transaction.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_ref::{Category, FullName, FullNameRef, Target};
use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::Expect;

use super::read::{Loose, Packed, is_missing, loose_content, parse_loose, read_loose};
use super::reflog::format_reflog_entry;
use super::{
    EditOp, PreviousValue, RefChangeKind, RefEdit, RefLayout, RefOutcome, RefTx, abs_path,
    io_error, ref_error,
};

mod apply;

pub(super) use apply::Step;

/// Longest chain of symbolic references followed, as in Git.
const MAX_SYMREF_DEPTH: usize = 5;

/// Directories `refs/<category>` and `logs/refs/<category>` are kept when
/// they become empty; deeper directories are removed.
const KEPT_REF_DEPTH: usize = 2;
const KEPT_LOG_DEPTH: usize = 3;

/// File state verified again immediately before the first change.
#[derive(Debug, Clone)]
pub(super) struct Guard {
    pub(super) path: AbsPath,
    pub(super) expected: Loose,
}

/// Planned file changes and the outcome of every edit.
#[derive(Debug, Default)]
pub(super) struct Plan {
    pub(super) outcomes: Vec<RefOutcome>,
    pub(super) steps: Vec<Step>,
    pub(super) guards: Vec<Guard>,
    pub(super) declared: Option<Vec<apply::StepTargets>>,
}

/// One edit after symbolic references were followed.
#[derive(Debug, Clone)]
struct Expanded {
    name: FullName,
    op: EditOp,
    expected: PreviousValue,
    message: BString,
    log_only: bool,
    force_log: bool,
    /// Symbolic references whose reflog records the same change.
    mirrors: Vec<FullName>,
}

/// Pending reflog write of one file.
#[derive(Debug, Default)]
struct LogWrite {
    exists: bool,
    bytes: Vec<u8>,
}

pub(super) fn prepare(tx: RefTx) -> Result<super::PreparedRefTx> {
    let mut builder = Builder::new(&tx)?;
    let expanded = builder.expand(&tx.edits)?;
    check_duplicates(&expanded)?;
    builder.check_new_names(&expanded)?;
    for edit in &expanded {
        builder.plan_edit(edit)?;
    }
    let plan = builder.finish()?;
    Ok(super::PreparedRefTx { plan })
}

struct Builder<'a> {
    layout: &'a RefLayout,
    committer: &'a gix_actor::Signature,
    packed: Packed,
    packed_path: AbsPath,
    cache: HashMap<PathBuf, Loose>,
    head_name: FullName,
    head: Option<Target>,
    planned: HashMap<BString, Option<Target>>,
    outcomes: Vec<RefOutcome>,
    create_dirs: Vec<AbsPath>,
    clear_dirs: Vec<AbsPath>,
    writes: Vec<Step>,
    ref_deletes: Vec<(AbsPath, AbsPath, Vec<u8>)>,
    log_deletes: Vec<AbsPath>,
    logs: IndexMap<AbsPath, LogWrite>,
    guards: Vec<Guard>,
    created: Vec<AbsPath>,
    removed: Vec<AbsPath>,
    packed_removals: Vec<BString>,
}

impl<'a> Builder<'a> {
    fn new(tx: &'a RefTx) -> Result<Self> {
        let layout = &tx.layout;
        let packed_path = layout.packed_path()?;
        let packed = Packed::read(packed_path.as_path())?;
        let mut builder = Self {
            layout,
            committer: &tx.committer,
            packed,
            packed_path,
            cache: HashMap::new(),
            head_name: head_name()?,
            head: None,
            planned: HashMap::new(),
            outcomes: Vec::new(),
            create_dirs: Vec::new(),
            clear_dirs: Vec::new(),
            writes: Vec::new(),
            ref_deletes: Vec::new(),
            log_deletes: Vec::new(),
            logs: IndexMap::new(),
            guards: Vec::new(),
            created: Vec::new(),
            removed: Vec::new(),
            packed_removals: Vec::new(),
        };
        let head = builder.head_name.clone();
        builder.head = builder.value(head.as_ref())?;
        Ok(builder)
    }

    fn loose(&mut self, path: &AbsPath) -> Result<Loose> {
        if let Some(state) = self.cache.get(path.as_path()) {
            return Ok(state.clone());
        }
        let state = read_loose(path.as_path())?;
        self.cache
            .insert(path.as_path().to_path_buf(), state.clone());
        Ok(state)
    }

    /// Current value of `name`, from its loose file or `packed-refs`.
    fn value(&mut self, name: &FullNameRef) -> Result<Option<Target>> {
        let path = self.layout.loose_path(name)?;
        match self.loose(&path)? {
            Loose::File(content) => parse_loose(&content, name.as_bstr()).map(Some),
            Loose::Absent | Loose::Dir => Ok(self.packed.find(name.as_bstr()).map(Target::Object)),
        }
    }

    /// Object a target resolves to before the transaction.
    fn resolve_before(&mut self, target: Option<&Target>) -> Result<Option<ObjectId>> {
        let mut current = target.cloned();
        for _ in 0..MAX_SYMREF_DEPTH {
            match current {
                None => return Ok(None),
                Some(Target::Object(id)) => return Ok(Some(id)),
                Some(Target::Symbolic(name)) => current = self.value(name.as_ref())?,
            }
        }
        Err(symref_loop(target))
    }

    /// Object a target resolves to after the transaction.
    fn resolve_after(&mut self, target: &Target) -> Result<Option<ObjectId>> {
        let mut current = Some(target.clone());
        for _ in 0..MAX_SYMREF_DEPTH {
            match current {
                None => return Ok(None),
                Some(Target::Object(id)) => return Ok(Some(id)),
                Some(Target::Symbolic(name)) => {
                    current = match self.planned.get(name.as_bstr()) {
                        Some(planned) => planned.clone(),
                        None => self.value(name.as_ref())?,
                    };
                }
            }
        }
        Err(symref_loop(Some(target)))
    }

    fn expand(&mut self, edits: &[RefEdit]) -> Result<Vec<Expanded>> {
        let explicit_head = edits.iter().any(|edit| edit.name.as_bstr() == "HEAD");
        let mut out = Vec::with_capacity(edits.len());
        for edit in edits {
            check_supported(edit.name.as_ref())?;
            if let EditOp::Rename { to } = &edit.op {
                check_supported(to.as_ref())?;
            }
            let mut expanded = Expanded {
                name: edit.name.clone(),
                op: edit.op.clone(),
                expected: edit.expected.clone(),
                message: edit.message.clone(),
                log_only: edit.log_only,
                force_log: edit.force_create_reflog,
                mirrors: Vec::new(),
            };
            if edit.deref && !edit.log_only {
                let referent = self.referent(edit.name.as_ref())?;
                if referent != edit.name {
                    expanded.mirrors.push(edit.name.clone());
                    expanded.name = referent;
                }
            }
            let mirrors_head = expanded.mirrors.iter().any(|m| m.as_bstr() == "HEAD");
            if !explicit_head && !mirrors_head && self.head_points_to(&expanded) {
                expanded.mirrors.push(self.head_name.clone());
            }
            out.push(expanded);
        }
        Ok(out)
    }

    /// Last reference of the symbolic chain starting at `name`.
    fn referent(&mut self, name: &FullNameRef) -> Result<FullName> {
        let mut current = name.to_owned();
        for _ in 0..MAX_SYMREF_DEPTH {
            match self.value(current.as_ref())? {
                Some(Target::Symbolic(next)) => current = next,
                Some(Target::Object(_)) | None => return Ok(current),
            }
        }
        Err(symref_loop(Some(&Target::Symbolic(name.to_owned()))))
    }

    fn head_points_to(&self, edit: &Expanded) -> bool {
        let applies = !edit.log_only && !matches!(edit.op, EditOp::Rename { .. });
        applies
            && edit.name.as_bstr() != "HEAD"
            && matches!(&self.head, Some(Target::Symbolic(target)) if *target == edit.name)
    }

    /// Rejects names that would become a directory of an existing reference
    /// or that already have references below them.
    fn check_new_names(&mut self, edits: &[Expanded]) -> Result<()> {
        let mut new_names: Vec<&FullName> = Vec::new();
        for edit in edits {
            match &edit.op {
                EditOp::Set(_) if !edit.log_only => {
                    if self.value(edit.name.as_ref())?.is_none() {
                        new_names.push(&edit.name);
                    }
                }
                EditOp::Rename { to } => new_names.push(to),
                _ => {}
            }
        }
        for (position, name) in new_names.iter().enumerate() {
            self.check_prefixes(name)?;
            self.check_below(name)?;
            let clash = new_names.iter().enumerate().any(|(other, candidate)| {
                other != position && is_dir_prefix(name.as_bstr(), candidate.as_bstr())
            });
            if clash {
                return Err(name_conflict(name.as_bstr()));
            }
        }
        Ok(())
    }

    fn check_prefixes(&mut self, name: &FullName) -> Result<()> {
        let bytes = name.as_bstr();
        let slashes = bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'/')
            .map(|(position, _)| position);
        for position in slashes.skip(1) {
            let prefix = bytes[..position].as_bstr();
            let Ok(prefix_name) = FullName::try_from(prefix) else {
                continue;
            };
            let path = self.layout.loose_path(prefix_name.as_ref())?;
            let loose_file = matches!(self.loose(&path)?, Loose::File(_));
            if loose_file || self.packed.find(prefix).is_some() {
                return Err(name_conflict(bytes));
            }
        }
        Ok(())
    }

    fn check_below(&mut self, name: &FullName) -> Result<()> {
        let bytes = name.as_bstr();
        let packed_below = self
            .packed
            .entries
            .iter()
            .any(|entry| is_dir_prefix(bytes, entry.name.as_bstr()));
        if packed_below {
            return Err(name_conflict(bytes));
        }
        let path = self.layout.loose_path(name.as_ref())?;
        self.clear_if_empty_dir(&path, bytes)
    }

    /// Schedules removal of an empty directory tree at `path`, or fails when
    /// files exist below it.
    fn clear_if_empty_dir(&mut self, path: &AbsPath, name: &BStr) -> Result<()> {
        if self.loose(path)? != Loose::Dir {
            return Ok(());
        }
        if has_files(path.as_path())? {
            return Err(name_conflict(name));
        }
        if !self.clear_dirs.contains(path) {
            self.clear_dirs.push(path.clone());
        }
        Ok(())
    }

    fn plan_edit(&mut self, edit: &Expanded) -> Result<()> {
        match &edit.op {
            EditOp::Set(new) if edit.log_only => self.plan_log_only(edit, new),
            EditOp::Set(new) => self.plan_set(edit, new),
            EditOp::Delete => self.plan_delete(edit),
            EditOp::Rename { to } => self.plan_rename(edit, to),
        }
    }

    fn plan_set(&mut self, edit: &Expanded, new: &Target) -> Result<()> {
        let current = self.value(edit.name.as_ref())?;
        check_expected(&edit.name, &edit.expected, current.as_ref())?;
        let kind = match &current {
            None => RefChangeKind::Created,
            Some(value) if value == new => RefChangeKind::Unchanged,
            Some(_) => RefChangeKind::Updated,
        };
        self.planned
            .insert(edit.name.as_bstr().to_owned(), Some(new.clone()));
        if kind != RefChangeKind::Unchanged {
            let path = self.layout.loose_path(edit.name.as_ref())?;
            self.schedule_write(path, loose_content(new))?;
        }
        let old_id = self.resolve_before(current.as_ref())?;
        let new_id = self.resolve_after(new)?;
        if let Some(new_id) = new_id {
            let old_id = old_id.unwrap_or_else(|| self.layout.object_hash.null());
            let changed = kind != RefChangeKind::Unchanged || matches!(new, Target::Symbolic(_));
            if changed {
                self.log(
                    &edit.name,
                    edit.force_log,
                    &old_id,
                    &new_id,
                    edit.message.as_bstr(),
                )?;
            }
            for mirror in &edit.mirrors {
                self.log(mirror, false, &old_id, &new_id, edit.message.as_bstr())?;
            }
        }
        self.outcomes.push(RefOutcome {
            name: edit.name.clone(),
            previous: current,
            new: Some(new.clone()),
            kind,
        });
        Ok(())
    }

    fn plan_log_only(&mut self, edit: &Expanded, new: &Target) -> Result<()> {
        let current = self.value(edit.name.as_ref())?;
        check_expected(&edit.name, &edit.expected, current.as_ref())?;
        let old_id = self
            .resolve_before(current.as_ref())?
            .unwrap_or_else(|| self.layout.object_hash.null());
        if let Some(new_id) = self.resolve_after(new)? {
            self.log(
                &edit.name,
                edit.force_log,
                &old_id,
                &new_id,
                edit.message.as_bstr(),
            )?;
        }
        Ok(())
    }

    fn plan_delete(&mut self, edit: &Expanded) -> Result<()> {
        let current = self.value(edit.name.as_ref())?;
        check_expected(&edit.name, &edit.expected, current.as_ref())?;
        let Some(previous) = current else {
            self.outcomes.push(RefOutcome {
                name: edit.name.clone(),
                previous: None,
                new: None,
                kind: RefChangeKind::Unchanged,
            });
            return Ok(());
        };
        self.planned.insert(edit.name.as_bstr().to_owned(), None);
        let old_id = self
            .resolve_before(Some(&previous))?
            .unwrap_or_else(|| self.layout.object_hash.null());
        self.remove_ref(&edit.name)?;
        let null = self.layout.object_hash.null();
        for mirror in &edit.mirrors {
            self.log(mirror, false, &old_id, &null, edit.message.as_bstr())?;
        }
        self.outcomes.push(RefOutcome {
            name: edit.name.clone(),
            previous: Some(previous),
            new: None,
            kind: RefChangeKind::Deleted,
        });
        Ok(())
    }

    /// Schedules removal of the loose file, the packed entry and the reflog
    /// of `name`; returns the previous reflog content.
    fn remove_ref(&mut self, name: &FullName) -> Result<Option<Vec<u8>>> {
        let path = self.layout.loose_path(name.as_ref())?;
        if let Loose::File(content) = self.loose(&path)? {
            let lock = lock_path(&path)?;
            self.guards.push(Guard {
                path: path.clone(),
                expected: Loose::File(content.clone()),
            });
            self.ref_deletes.push((lock, path.clone(), content));
            self.removed.push(path);
        }
        if self.packed.find(name.as_bstr()).is_some() {
            self.packed_removals.push(name.as_bstr().to_owned());
        }
        let log_path = self.layout.log_path(name.as_ref())?;
        let Loose::File(log) = self.loose(&log_path)? else {
            return Ok(None);
        };
        self.log_deletes.push(log_path.clone());
        self.removed.push(log_path);
        Ok(Some(log))
    }

    fn plan_rename(&mut self, edit: &Expanded, to: &FullName) -> Result<()> {
        let current = self.value(edit.name.as_ref())?;
        let Some(Target::Object(id)) = current else {
            return Err(match current {
                None => missing(&edit.name),
                Some(_) => ref_error(
                    "symbolic",
                    edit.name.as_bstr(),
                    format!(
                        "{} is a symbolic reference; rename the reference it points to",
                        edit.name.as_bstr()
                    ),
                ),
            });
        };
        if self.value(to.as_ref())?.is_some() {
            return Err(exists(to));
        }
        self.planned.insert(edit.name.as_bstr().to_owned(), None);
        self.planned
            .insert(to.as_bstr().to_owned(), Some(Target::Object(id)));
        let old_log = self.remove_ref(&edit.name)?;
        let path = self.layout.loose_path(to.as_ref())?;
        self.schedule_write(path, loose_content(&Target::Object(id)))?;
        let entry = format_reflog_entry(&id, &id, self.committer, edit.message.as_bstr());
        let log_path = self.layout.log_path(to.as_ref())?;
        match old_log {
            Some(mut log) => {
                log.extend_from_slice(&entry);
                self.schedule_log_file(log_path, log)?;
            }
            None if self.layout.log_all.creates_log_for(to.as_ref()) => {
                self.schedule_log_file(log_path, entry.into())?;
            }
            None => {}
        }
        self.move_heads(&edit.name, to, &id, edit.message.as_bstr())?;
        self.outcomes.push(RefOutcome {
            name: edit.name.clone(),
            previous: Some(Target::Object(id)),
            new: None,
            kind: RefChangeKind::Deleted,
        });
        self.outcomes.push(RefOutcome {
            name: to.clone(),
            previous: None,
            new: Some(Target::Object(id)),
            kind: RefChangeKind::Created,
        });
        Ok(())
    }

    /// Points every `HEAD` of the repository that names `from` to `to`.
    ///
    /// Git records two entries in the reflog of each moved `HEAD`: the
    /// deletion of `from` and the update of the symbolic reference.
    fn move_heads(
        &mut self,
        from: &FullName,
        to: &FullName,
        id: &ObjectId,
        message: &BStr,
    ) -> Result<()> {
        let null = self.layout.object_hash.null();
        let content = loose_content(&Target::Symbolic(from.clone()));
        for dir in self.head_dirs()? {
            let head = dir.as_path().join("HEAD");
            let head = abs_path(&head)?;
            let Loose::File(current) = self.loose(&head)? else {
                continue;
            };
            if current.trim() != content.trim() {
                continue;
            }
            self.schedule_write(head, loose_content(&Target::Symbolic(to.clone())))?;
            let log_path = abs_path(&dir.as_path().join("logs").join("HEAD"))?;
            let log_exists = matches!(self.loose(&log_path)?, Loose::File(_));
            if log_exists || self.layout.log_all.creates_log_for(self.head_name.as_ref()) {
                let mut bytes = format_reflog_entry(id, &null, self.committer, message);
                bytes.extend_from_slice(&format_reflog_entry(&null, id, self.committer, message));
                self.append_log(log_path, log_exists, &bytes)?;
            }
        }
        Ok(())
    }

    /// Repository directories of all working trees: the common directory and
    /// every registered linked working tree.
    fn head_dirs(&self) -> Result<Vec<AbsPath>> {
        let mut dirs = vec![self.layout.common_dir.clone()];
        let worktrees = self.layout.common_dir.as_path().join("worktrees");
        let entries = match std::fs::read_dir(&worktrees) {
            Ok(entries) => entries,
            Err(error) if is_missing(&error) => return Ok(dirs),
            Err(error) => return Err(io_error(&worktrees, &error)),
        };
        let mut linked = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&worktrees, &error))?;
            linked.push(abs_path(&entry.path())?);
        }
        linked.sort();
        dirs.extend(linked);
        Ok(dirs)
    }

    /// Schedules `content` to be written to `path`; the journal checks the
    /// previous content read now atomically with the switch.
    fn schedule_write(&mut self, path: AbsPath, content: Vec<u8>) -> Result<()> {
        let state = self.loose(&path)?;
        if state == Loose::Dir {
            let name = BString::from(path.to_language_string());
            self.clear_if_empty_dir(&path, name.as_ref())?;
        } else {
            self.ensure_parents(&path)?;
        }
        self.created.push(path.clone());
        self.push_write(path, content, state)
    }

    /// Records the write of `path`, whose current state is `state`.
    fn push_write(&mut self, path: AbsPath, content: Vec<u8>, state: Loose) -> Result<()> {
        let expect = match &state {
            Loose::File(old) => Expect::Hash(apply::content_hash(old)),
            Loose::Absent => Expect::Absent,
            Loose::Dir => Expect::Unchecked,
        };
        self.guards.push(Guard {
            path: path.clone(),
            expected: state,
        });
        let lock = lock_path(&path)?;
        self.writes.push(Step::Write {
            lock,
            path,
            content,
            expect,
        });
        Ok(())
    }

    /// Appends a reflog entry for `name` when Git would.
    fn log(
        &mut self,
        name: &FullName,
        force: bool,
        old: &ObjectId,
        new: &ObjectId,
        message: &BStr,
    ) -> Result<()> {
        let path = self.layout.log_path(name.as_ref())?;
        let exists = matches!(self.loose(&path)?, Loose::File(_));
        let pending = self.logs.contains_key(&path);
        if !(exists || pending || force || self.layout.log_all.creates_log_for(name.as_ref())) {
            return Ok(());
        }
        let entry = format_reflog_entry(old, new, self.committer, message);
        self.append_log(path, exists, &entry)
    }

    fn append_log(&mut self, path: AbsPath, exists: bool, bytes: &[u8]) -> Result<()> {
        if !exists && !self.logs.contains_key(&path) {
            let name = BString::from(path.to_language_string());
            self.clear_if_empty_dir(&path, name.as_ref())?;
            self.ensure_parents(&path)?;
            self.created.push(path.clone());
        }
        let write = self.logs.entry(path).or_default();
        write.exists = exists;
        write.bytes.extend_from_slice(bytes);
        Ok(())
    }

    /// Schedules a new reflog file with the complete content `bytes`.
    fn schedule_log_file(&mut self, path: AbsPath, bytes: Vec<u8>) -> Result<()> {
        self.append_log(path, false, &bytes)
    }

    /// Schedules creation of the missing parent directories of `path`.
    fn ensure_parents(&mut self, path: &AbsPath) -> Result<()> {
        let mut missing = Vec::new();
        let mut current = path.parent();
        while let Some(dir) = current {
            if self.create_dirs.contains(&dir) {
                break;
            }
            match self.loose(&dir)? {
                Loose::Dir => break,
                Loose::File(_) => {
                    let name = BString::from(path.to_language_string());
                    return Err(name_conflict(name.as_ref()));
                }
                Loose::Absent => {
                    missing.push(dir.clone());
                    current = dir.parent();
                }
            }
        }
        missing.reverse();
        self.create_dirs.extend(missing);
        Ok(())
    }

    fn finish(mut self) -> Result<Plan> {
        if !self.packed_removals.is_empty() {
            let content = self.packed.without(&self.packed_removals);
            let state = self.packed.raw.clone().map_or(Loose::Absent, Loose::File);
            let path = self.packed_path.clone();
            self.push_write(path, content, state)?;
        }
        let remove_dirs = self.empty_dirs()?;
        let mut steps = Vec::new();
        steps.extend(self.create_dirs.into_iter().map(Step::CreateDir));
        steps.extend(self.clear_dirs.into_iter().map(Step::ClearDir));
        steps.extend(self.writes);
        steps.extend(
            self.ref_deletes
                .into_iter()
                .map(|(lock, path, old)| Step::DeleteRef { lock, path, old }),
        );
        steps.extend(self.log_deletes.into_iter().map(Step::DeleteLog));
        steps.extend(self.logs.into_iter().map(|(path, write)| {
            if write.exists {
                Step::AppendLog {
                    path,
                    bytes: write.bytes,
                }
            } else {
                Step::CreateLog {
                    path,
                    bytes: write.bytes,
                }
            }
        }));
        steps.extend(remove_dirs.into_iter().map(Step::RemoveDir));
        apply::check_locks(&steps)?;
        Ok(Plan {
            outcomes: self.outcomes,
            steps,
            guards: self.guards,
            declared: None,
        })
    }

    /// Directories left empty by the removed files, deepest first.
    fn empty_dirs(&self) -> Result<Vec<AbsPath>> {
        let mut candidates: BTreeSet<(std::cmp::Reverse<usize>, AbsPath)> = BTreeSet::new();
        for path in &self.removed {
            let mut current = path.parent();
            while let Some(dir) = current {
                if !self.removable_depth(&dir) {
                    break;
                }
                candidates.insert((
                    std::cmp::Reverse(dir.as_path().components().count()),
                    dir.clone(),
                ));
                current = dir.parent();
            }
        }
        let mut gone: HashSet<PathBuf> = self
            .removed
            .iter()
            .map(|path| path.as_path().to_path_buf())
            .collect();
        let mut out = Vec::new();
        for (_, dir) in candidates {
            let receives_file = self.created.iter().any(|path| path.starts_with(&dir));
            if !receives_file && all_entries_gone(dir.as_path(), &gone)? {
                gone.insert(dir.as_path().to_path_buf());
                out.push(dir);
            }
        }
        Ok(out)
    }

    /// Whether `dir` lies below `refs/<category>/` or `logs/refs/<category>/`.
    fn removable_depth(&self, dir: &AbsPath) -> bool {
        let relative = [&self.layout.git_dir, &self.layout.common_dir]
            .into_iter()
            .find_map(|base| dir.as_path().strip_prefix(base.as_path()).ok());
        let Some(relative) = relative else {
            return false;
        };
        let mut components = relative.components();
        let kept = match components.next() {
            Some(first) if first.as_os_str() == "logs" => KEPT_LOG_DEPTH,
            Some(first) if first.as_os_str() == "refs" => KEPT_REF_DEPTH,
            _ => return false,
        };
        relative.components().count() > kept
    }
}

fn head_name() -> Result<FullName> {
    FullName::try_from("HEAD")
        .map_err(|error| Error::new(ErrorCode::Internal, format!("invalid name HEAD: {error}")))
}

fn check_supported(name: &FullNameRef) -> Result<()> {
    let other_worktree = matches!(
        name.category(),
        Some(
            Category::MainPseudoRef
                | Category::MainRef
                | Category::LinkedPseudoRef { .. }
                | Category::LinkedRef { .. }
        )
    );
    if other_worktree {
        return Err(Error::new(
            ErrorCode::GitUnsupported,
            format!(
                "reference {} belongs to another working tree; run the command in that working tree",
                name.as_bstr()
            ),
        )
        .with_problem("other_worktree")
        .with_found(name.as_bstr().to_str_lossy().into_owned()));
    }
    Ok(())
}

fn check_duplicates(edits: &[Expanded]) -> Result<()> {
    let mut seen: HashSet<&BStr> = HashSet::new();
    for edit in edits.iter().filter(|edit| !edit.log_only) {
        let mut names = vec![edit.name.as_bstr()];
        if let EditOp::Rename { to } = &edit.op {
            names.push(to.as_bstr());
        }
        for name in names {
            if !seen.insert(name) {
                return Err(ref_error(
                    "duplicate",
                    name,
                    format!("reference {name} is changed twice in one operation; change it once"),
                ));
            }
        }
    }
    Ok(())
}

fn check_expected(
    name: &FullName,
    expected: &PreviousValue,
    current: Option<&Target>,
) -> Result<()> {
    match (expected, current) {
        (PreviousValue::Any, _)
        | (PreviousValue::MustExist, Some(_))
        | (PreviousValue::MustNotExist, None)
        | (PreviousValue::ExistingMustMatch(_), None) => Ok(()),
        (PreviousValue::MustExist | PreviousValue::MustExistAndMatch(_), None) => {
            Err(missing(name))
        }
        (PreviousValue::MustNotExist, Some(_)) => Err(exists(name)),
        (
            PreviousValue::MustExistAndMatch(want) | PreviousValue::ExistingMustMatch(want),
            Some(have),
        ) => {
            if want == have {
                Ok(())
            } else {
                Err(ref_error(
                    "changed",
                    name.as_bstr(),
                    format!(
                        "reference {} is at {} but {} was expected; read its current value and retry",
                        name.as_bstr(),
                        describe(have),
                        describe(want)
                    ),
                )
                .with_expected(serde_json::Value::String(describe(want)))
                .with_actual(serde_json::Value::String(describe(have))))
            }
        }
    }
}

fn describe(target: &Target) -> String {
    match target {
        Target::Object(id) => id.to_hex().to_string(),
        Target::Symbolic(name) => format!("ref: {}", name.as_bstr()),
    }
}

fn missing(name: &FullName) -> Error {
    ref_error(
        "missing",
        name.as_bstr(),
        format!(
            "reference {} does not exist; check the name",
            name.as_bstr()
        ),
    )
}

fn exists(name: &FullName) -> Error {
    ref_error(
        "exists",
        name.as_bstr(),
        format!(
            "reference {} already exists; choose another name",
            name.as_bstr()
        ),
    )
}

fn name_conflict(name: &BStr) -> Error {
    ref_error(
        "name_conflict",
        name,
        format!(
            "{name} conflicts with an existing reference used as a directory or below it; choose another name"
        ),
    )
}

fn symref_loop(target: Option<&Target>) -> Error {
    let name = target.map(describe).unwrap_or_default();
    ref_error(
        "symref_loop",
        name.as_str().into(),
        format!(
            "symbolic reference chain from {name} is longer than {MAX_SYMREF_DEPTH}; repair the symbolic references"
        ),
    )
}

/// Whether `prefix` names a directory containing `name`.
fn is_dir_prefix(prefix: &BStr, name: &BStr) -> bool {
    name.len() > prefix.len() && name.starts_with(prefix) && name[prefix.len()] == b'/'
}

/// `<path>.lock`.
fn lock_path(path: &AbsPath) -> Result<AbsPath> {
    let mut name = path.as_path().as_os_str().to_owned();
    name.push(".lock");
    abs_path(Path::new(&name))
}

/// Whether a regular file or link exists anywhere below `dir`.
fn has_files(dir: &Path) -> Result<bool> {
    let entries = std::fs::read_dir(dir).map_err(|error| io_error(dir, &error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error(dir, &error))?;
        let kind = entry.file_type().map_err(|error| io_error(dir, &error))?;
        if !kind.is_dir() || has_files(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether every entry of `dir` is in `gone`.
fn all_entries_gone(dir: &Path, gone: &HashSet<PathBuf>) -> Result<bool> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if is_missing(&error) => return Ok(false),
        Err(error) => return Err(io_error(dir, &error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| io_error(dir, &error))?;
        if !gone.contains(&entry.path()) {
            return Ok(false);
        }
    }
    Ok(true)
}
