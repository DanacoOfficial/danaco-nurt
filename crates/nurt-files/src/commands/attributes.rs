// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `attributes` command: permissions, account and group, times,
//! read-only and hidden attributes, extended attributes and access control
//! of entries.
//!
//! The plan step reads every target, computes the metadata change from the
//! current state and leaves out targets whose metadata already has the
//! requested values; they are reported as `unchanged`. The apply step makes
//! every change through the transaction as one bulk operation; the journal
//! keeps the previous metadata, so the change is undone as a whole.
//!
//! The same parameters act differently on the three systems:
//!
//! | Parameter | Linux | macOS | Windows |
//! |---|---|---|---|
//! | `mode` | permission bits | permission bits | read-only attribute from the write bit of the account; flag `acl_unchanged` |
//! | `owner`, `group` | account and group ids | account and group ids | `E_UNSUPPORTED` |
//! | `acl` | POSIX ACL | `E_UNSUPPORTED` | `E_UNSUPPORTED` |
//! | `readonly` | write bits | write bits | read-only attribute |
//! | `hidden` | name only; flag `hidden_unsupported` | name only; flag `hidden_unsupported` | hidden attribute |
//! | `created` | flag `created_unsupported` | birth time | creation time |
//! | `xattrs` | extended attributes | extended attributes | `E_UNSUPPORTED` |
//!
//! A flag in the `flags` field of a change, or in the detail of an
//! `unchanged` entry, names a requested element that the system could not
//! apply. With `recursive`, the entries below a directory are changed as
//! well; links below it are not followed and not changed.

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::sync::Arc;

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Etag, FileKind, FileMeta, Follow, Fs, FsError, FsErrorKind, MetaChange, Xattr,
};
use nurt_contracts::journal::{Expect, FileOp, MutationTx, TargetAction};
use nurt_contracts::record::{Effect, EffectStatus};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Instant, Record, RecordKind, Value};
use serde_json::Value as Json;

use crate::common::etag::{check_if_match, observe, parse_etag};
use crate::common::mutation::{apply_changes, fs_target, planned_change};
use crate::common::notable::{at_path, counted};
use crate::common::path::{path_list_arg, record_path};
use crate::details::accounts::{Accounts, SUPERUSER_UID};
use crate::details::acl::{self, ACCESS_XATTR, AclEntry};
use crate::details::mode::{ModeSpec, PERMISSION_BITS};
use crate::walk::{KindFilter, WalkItem, WalkOptions, Walker};

const COMMAND: &str = "attributes";

const PATH: &str = "path";
const MODE: &str = "mode";
const OWNER: &str = "owner";
const GROUP: &str = "group";
const ACL: &str = "acl";
const READONLY: &str = "readonly";
const HIDDEN: &str = "hidden";
const MODIFIED: &str = "modified";
const ACCESSED: &str = "accessed";
const CREATED: &str = "created";
const XATTRS: &str = "xattrs";
const RECURSIVE: &str = "recursive";

/// Parameters that request a change.
const SETTINGS: [&str; 11] = [
    MODE, OWNER, GROUP, ACL, READONLY, HIDDEN, MODIFIED, ACCESSED, CREATED, XATTRS, RECURSIVE,
];

/// Field of input records holding the entity tag observed by the producer.
const ETAG_FIELD: &str = "etag";

/// `FILE_ATTRIBUTE_READONLY` of Windows.
const WINDOWS_READONLY: u32 = 0x1;

/// `FILE_ATTRIBUTE_HIDDEN` of Windows.
const WINDOWS_HIDDEN: u32 = 0x2;

/// Write permission of every class.
const WRITE_BITS: u32 = 0o222;

/// Write permission of the account of the file.
const OWNER_WRITE: u32 = 0o200;

/// Names starting with this character are hidden on Unix systems.
const HIDDEN_PREFIX: char = '.';

/// Flag of a mode applied on Windows only as the read-only attribute.
const FLAG_ACL_UNCHANGED: &str = "acl_unchanged";

/// Flag of a hidden state that the system ties to the name.
const FLAG_HIDDEN_UNSUPPORTED: &str = "hidden_unsupported";

/// Flag of a creation time the system does not let set.
const FLAG_CREATED_UNSUPPORTED: &str = "created_unsupported";

/// Separator of the elements and flags in a notable detail.
const LIST_SEPARATOR: &str = ", ";

/// Stage of `attributes`.
pub(crate) struct Attributes;

impl StageImpl for Attributes {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let settings = Settings::parse(args)?;
        settings.require_change()?;
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Attributes {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let settings = Settings::parse(args)?;
        settings.require_change()?;
        let os = ctx.host().info().os;
        settings.check_platform(os)?;
        let roots = roots(args, input, ctx)?;
        let fs = ctx.require_fs()?;
        let accounts =
            if settings.owner.is_some() || settings.group.is_some() || settings.acl.is_some() {
                Accounts::load(fs, os)
            } else {
                Accounts::default()
            };
        let acl = settings
            .acl
            .as_ref()
            .map(|value| acl::parse(value, &accounts))
            .transpose()?;
        let mut planner = Planner {
            ctx,
            fs,
            os,
            engine_uid: accounts.resolve_user(&ctx.host().info().user),
            settings: &settings,
            accounts,
            acl,
            planned: Vec::new(),
            visited: HashSet::new(),
            notable: Vec::new(),
            unchanged_below: 0,
            denied: 0,
        };
        for root in &roots {
            ctx.checkpoint()?;
            planner.plan_root(root)?;
        }
        Ok(planner.finish(roots.is_empty()))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan.take_state::<PlanState>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "the plan of `attributes` carries no targets",
            )
        })?;
        let flags: Vec<Vec<&'static str>> = state
            .targets
            .iter()
            .map(|target| target.flags.clone())
            .collect();
        let ops: Vec<FileOp> = state
            .targets
            .into_iter()
            .map(|target| FileOp::set_meta(target.path, target.change).expect(target.expect))
            .collect();
        let mut changes = if ops.is_empty() {
            Vec::new()
        } else {
            apply_changes(tx, ops)?
        };
        for (change, flags) in changes.iter_mut().zip(flags) {
            change.flags = flags_field(&flags);
        }
        let mut codes: Vec<String> = Vec::new();
        for entry in &plan.notable {
            let code = entry.code.as_str().to_owned();
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
        let effect = Effect {
            status: EffectStatus::Ok,
            kind: COMMAND.to_owned(),
            op: tx.op().map(|op| op.0),
            targets: Some(u64::try_from(changes.len()).unwrap_or(u64::MAX)),
            changes: Some(changes.clone()),
            notable: (!codes.is_empty()).then_some(codes),
            ..Effect::default()
        };
        Ok(EffectOutcome {
            changes,
            ..EffectOutcome::new(StageOutput::records(RecordStream::once(Record::from(
                effect,
            ))))
        })
    }
}

/// Stages of this module, for the registry.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Attributes)]
}

/// Requested changes.
#[derive(Debug, Default)]
struct Settings {
    mode: Option<ModeSpec>,
    owner: Option<String>,
    group: Option<String>,
    acl: Option<Value>,
    readonly: Option<bool>,
    hidden: Option<bool>,
    modified: Option<i64>,
    accessed: Option<i64>,
    created: Option<i64>,
    xattrs: Vec<(String, Option<Vec<u8>>)>,
    recursive: bool,
}

impl Settings {
    fn parse(args: &BoundArgs) -> Result<Self> {
        Ok(Self {
            mode: args.text(MODE)?.map(ModeSpec::parse).transpose()?,
            owner: args.text(OWNER)?.map(str::to_owned),
            group: args.text(GROUP)?.map(str::to_owned),
            acl: args
                .value(ACL)
                .filter(|value| **value != Value::Null)
                .cloned(),
            readonly: args.bool(READONLY)?,
            hidden: args.bool(HIDDEN)?,
            modified: instant_arg(args, MODIFIED)?,
            accessed: instant_arg(args, ACCESSED)?,
            created: instant_arg(args, CREATED)?,
            xattrs: xattrs_arg(args)?,
            recursive: args.bool(RECURSIVE)?.unwrap_or(false),
        })
    }

    /// Fails with `E_MISSING_PARAM` when no change is requested.
    fn require_change(&self) -> Result<()> {
        let requested = self.mode.is_some()
            || self.owner.is_some()
            || self.group.is_some()
            || self.acl.is_some()
            || self.readonly.is_some()
            || self.hidden.is_some()
            || self.modified.is_some()
            || self.accessed.is_some()
            || self.created.is_some()
            || !self.xattrs.is_empty();
        if requested {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::MissingParam,
                format!(
                    "`{COMMAND}` needs a change: give at least one of {}",
                    SETTINGS[..SETTINGS.len() - 1].join(", ")
                ),
            ))
        }
    }

    /// Fails with `E_UNSUPPORTED` for elements the system has no form for.
    fn check_platform(&self, os: Os) -> Result<()> {
        let refusal = if os == Os::Windows {
            if self.owner.is_some() || self.group.is_some() {
                Some(
                    "changing the account or group of a file is not supported on Windows: leave `owner` and `group` out",
                )
            } else if self.acl.is_some() {
                Some("`acl` is not supported on Windows: protect files with `readonly`")
            } else if !self.xattrs.is_empty() {
                Some("extended attributes are not supported on Windows: leave `xattrs` out")
            } else {
                None
            }
        } else if os != Os::Linux && self.acl.is_some() {
            Some("`acl` is supported on Linux only: change permissions with `mode`")
        } else {
            None
        };
        match refusal {
            Some(message) => Err(Error::new(ErrorCode::Unsupported, message)),
            None => Ok(()),
        }
    }
}

fn instant_arg(args: &BoundArgs, name: &str) -> Result<Option<i64>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Instant(instant)) => Ok(Some(instant.as_unix_nanos())),
        Some(Value::Text(text)) => text
            .parse::<Instant>()
            .map(|instant| Some(instant.as_unix_nanos()))
            .map_err(|_| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{text}` is not a time: give an RFC 3339 time such as \"2026-01-31T12:00:00Z\""),
                )
                .with_found(text.clone())
            }),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!("`{COMMAND}` expects an instant for `{name}`"),
        )),
    }
}

/// Extended attributes to set; a `null` value removes the attribute.
fn xattrs_arg(args: &BoundArgs) -> Result<Vec<(String, Option<Vec<u8>>)>> {
    let fields = match args.value(XATTRS) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Map(fields)) => fields,
        Some(_) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!("`{COMMAND}` expects a map from names to texts for `{XATTRS}`"),
            ));
        }
    };
    fields
        .iter()
        .map(|(name, value)| {
            if name.is_empty() {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    "an extended attribute needs a name, such as \"user.comment\"",
                ));
            }
            match value {
                Value::Text(text) => Ok((name.clone(), Some(text.as_bytes().to_vec()))),
                Value::Null => Ok((name.clone(), None)),
                _ => Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "the value of extended attribute `{name}` is a text, or null to remove it"
                    ),
                )),
            }
        })
        .collect()
}

/// An entry named by the program or by an input record.
#[derive(Debug)]
struct Root {
    path: AbsPath,
    expected: Option<Etag>,
}

fn roots(args: &BoundArgs, input: StageInput<'_>, ctx: &ExecCtx) -> Result<Vec<Root>> {
    if input.is_none() {
        return Ok(path_list_arg(args, PATH, ctx)?
            .into_iter()
            .map(|path| Root {
                path,
                expected: None,
            })
            .collect());
    }
    input
        .into_records()?
        .map(|item| {
            let record = item?;
            let expected = match record.get(ETAG_FIELD) {
                Some(Value::Text(tag)) => Some(parse_etag(tag)?),
                _ => None,
            };
            Ok(Root {
                path: record_path(&record, ctx)?,
                expected,
            })
        })
        .collect()
}

/// Change of one target, kept between the plan and the apply step.
#[derive(Debug)]
struct Planned {
    path: AbsPath,
    change: MetaChange,
    expect: Expect,
    flags: Vec<&'static str>,
}

/// Private state of the plan.
#[derive(Debug)]
struct PlanState {
    targets: Vec<Planned>,
}

/// Change of one target while it is computed.
#[derive(Debug, Default)]
struct Draft {
    change: MetaChange,
    flags: Vec<&'static str>,
    elements: Vec<&'static str>,
}

struct Planner<'a> {
    ctx: &'a ExecCtx,
    fs: &'a dyn Fs,
    os: Os,
    engine_uid: Option<u32>,
    settings: &'a Settings,
    accounts: Accounts,
    acl: Option<Vec<AclEntry>>,
    planned: Vec<Planned>,
    visited: HashSet<AbsPath>,
    notable: Vec<NotableEntry>,
    unchanged_below: u64,
    denied: u64,
}

impl Planner<'_> {
    fn plan_root(&mut self, root: &Root) -> Result<()> {
        let observed = observe(self.fs, &root.path)?;
        let Some(meta) = observed.meta() else {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!(
                    "`{}` does not exist: give an existing file or directory",
                    root.path
                ),
            )
            .with_detail("path", Json::from(root.path.to_language_string())));
        };
        check_if_match(&root.path, &observed, root.expected.as_ref())?;
        if !self.plan_entry(&root.path, meta)? {
            let flags = self.unchanged_flags(&root.path, meta).unwrap_or_default();
            self.notable.push(NotableEntry {
                detail: (!flags.is_empty()).then(|| flags.join(LIST_SEPARATOR)),
                ..at_path(Notable::Unchanged, &root.path)
            });
        }
        if self.settings.recursive && meta.kind == FileKind::Directory {
            self.plan_tree(&root.path)?;
        }
        Ok(())
    }

    /// Flags of an unchanged entry, computed again for its note.
    fn unchanged_flags(&self, path: &AbsPath, meta: &FileMeta) -> Result<Vec<&'static str>> {
        let current = self.current(path, meta)?;
        let mut draft = Draft::default();
        self.plan_permissions(&current, &mut draft);
        self.plan_hidden(path, &current, &mut draft);
        self.plan_times(&current, &mut draft);
        Ok(draft.flags)
    }

    fn plan_tree(&mut self, root: &AbsPath) -> Result<()> {
        let threads = usize::try_from(self.ctx.limits().threads)
            .unwrap_or(1)
            .max(1);
        let walker = Walker::new(WalkOptions {
            min_depth: 1,
            kind: KindFilter::Any,
            hidden: true,
            follow_links: false,
            metadata: true,
            threads,
            ..WalkOptions::default()
        })?;
        let mut found = Vec::new();
        let mut failure: Option<FsError> = None;
        let ctx = self.ctx;
        let summary = walker.run(
            self.fs,
            std::slice::from_ref(root),
            &|| ctx.checkpoint(),
            |item| match item {
                WalkItem::Entry(entry) => {
                    if let Some(meta) = entry.meta {
                        found.push((entry.path, meta));
                    }
                    ControlFlow::Continue(())
                }
                WalkItem::Unreadable(unreadable)
                    if unreadable.error.kind == FsErrorKind::PermissionDenied =>
                {
                    ControlFlow::Continue(())
                }
                WalkItem::Unreadable(unreadable) => {
                    failure = Some(unreadable.error);
                    ControlFlow::Break(())
                }
            },
        )?;
        if let Some(error) = failure {
            return Err(error.into());
        }
        self.denied += summary.permission_denied;
        for (path, meta) in found {
            if matches!(meta.kind, FileKind::Symlink | FileKind::Junction) {
                continue;
            }
            self.ctx.checkpoint()?;
            if !self.plan_entry(&path, &meta)? {
                self.unchanged_below += 1;
            }
        }
        Ok(())
    }

    /// Metadata the change is computed from: the entry itself, or the entry
    /// a link resolves to.
    fn current(&self, path: &AbsPath, meta: &FileMeta) -> Result<FileMeta> {
        if matches!(meta.kind, FileKind::Symlink | FileKind::Junction) {
            Ok(self.fs.metadata(path, Follow::Follow)?)
        } else {
            Ok(meta.clone())
        }
    }

    /// Plans the change of one entry; returns `false` when the entry
    /// already has the requested metadata.
    ///
    /// An entry reached twice, as two roots or as a root and an entry below
    /// a recursive root, is planned once: two targets with one path cannot
    /// be preserved in one pass.
    fn plan_entry(&mut self, path: &AbsPath, meta: &FileMeta) -> Result<bool> {
        if !self.visited.insert(path.clone()) {
            return Ok(true);
        }
        let current = self.current(path, meta)?;
        let mut draft = Draft::default();
        self.plan_permissions(&current, &mut draft);
        self.plan_hidden(path, &current, &mut draft);
        self.plan_times(&current, &mut draft);
        self.plan_accounts(path, &current, &mut draft)?;
        self.plan_extended(path, &mut draft)?;
        if draft.change == MetaChange::default() {
            return Ok(false);
        }
        self.notable.push(NotableEntry {
            detail: Some(draft.elements.join(LIST_SEPARATOR)),
            ..at_path(Notable::AttributesChanged, path)
        });
        self.planned.push(Planned {
            path: path.clone(),
            change: draft.change,
            expect: Expect::Etag(meta.etag()),
            flags: draft.flags,
        });
        Ok(true)
    }

    fn plan_permissions(&self, current: &FileMeta, draft: &mut Draft) {
        let settings = self.settings;
        if settings.mode.is_none() && settings.readonly.is_none() {
            return;
        }
        let directory = current.kind == FileKind::Directory;
        let bits = current.mode & PERMISSION_BITS;
        let mut wanted = settings
            .mode
            .as_ref()
            .map_or(bits, |mode| mode.apply(bits, directory));
        match settings.readonly {
            Some(true) => wanted &= !WRITE_BITS,
            Some(false) => wanted |= OWNER_WRITE,
            None => {}
        }
        if self.os == Os::Windows {
            if settings.mode.is_some() {
                draft.flags.push(FLAG_ACL_UNCHANGED);
            }
            let readonly = wanted & OWNER_WRITE == 0;
            if readonly != (current.windows_attributes & WINDOWS_READONLY != 0) {
                if readonly {
                    draft.change.set_windows_attributes |= WINDOWS_READONLY;
                } else {
                    draft.change.clear_windows_attributes |= WINDOWS_READONLY;
                }
                draft.elements.push(READONLY);
            }
        } else if wanted != bits {
            draft.change.mode = Some(wanted);
            draft.elements.push(MODE);
        }
    }

    fn plan_hidden(&self, path: &AbsPath, current: &FileMeta, draft: &mut Draft) {
        let Some(hidden) = self.settings.hidden else {
            return;
        };
        if self.os == Os::Windows {
            if hidden != (current.windows_attributes & WINDOWS_HIDDEN != 0) {
                if hidden {
                    draft.change.set_windows_attributes |= WINDOWS_HIDDEN;
                } else {
                    draft.change.clear_windows_attributes |= WINDOWS_HIDDEN;
                }
                draft.elements.push(HIDDEN);
            }
            return;
        }
        let dotted = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(HIDDEN_PREFIX));
        if hidden != dotted {
            draft.flags.push(FLAG_HIDDEN_UNSUPPORTED);
        }
    }

    fn plan_times(&self, current: &FileMeta, draft: &mut Draft) {
        let settings = self.settings;
        if let Some(modified) = settings.modified
            && modified != current.modified_ns
        {
            draft.change.modified_ns = Some(modified);
            draft.elements.push(MODIFIED);
        }
        if let Some(accessed) = settings.accessed
            && current.accessed_ns != Some(accessed)
        {
            draft.change.accessed_ns = Some(accessed);
            draft.elements.push(ACCESSED);
        }
        if let Some(created) = settings.created
            && current.created_ns != Some(created)
        {
            if self.os == Os::Linux {
                draft.flags.push(FLAG_CREATED_UNSUPPORTED);
            } else {
                draft.change.created_ns = Some(created);
                draft.elements.push(CREATED);
            }
        }
    }

    fn plan_accounts(&self, path: &AbsPath, current: &FileMeta, draft: &mut Draft) -> Result<()> {
        if let Some(owner) = &self.settings.owner {
            let uid = self
                .accounts
                .resolve_user(owner)
                .ok_or_else(|| unknown_account(OWNER, owner))?;
            if current.uid != Some(uid) {
                if self
                    .engine_uid
                    .is_some_and(|engine| engine != SUPERUSER_UID)
                {
                    return Err(Error::new(
                        ErrorCode::Privilege,
                        format!(
                            "changing the account of `{path}` needs administrator rights: run as root or leave `owner` out"
                        ),
                    )
                    .with_detail("path", Json::from(path.to_language_string())));
                }
                draft.change.uid = Some(uid);
                draft.elements.push(OWNER);
            }
        }
        if let Some(group) = &self.settings.group {
            let gid = self
                .accounts
                .resolve_group(group)
                .ok_or_else(|| unknown_account(GROUP, group))?;
            if current.gid != Some(gid) {
                draft.change.gid = Some(gid);
                draft.elements.push(GROUP);
            }
        }
        Ok(())
    }

    fn plan_extended(&self, path: &AbsPath, draft: &mut Draft) -> Result<()> {
        let settings = self.settings;
        if self.acl.is_none() && settings.xattrs.is_empty() {
            return Ok(());
        }
        let features = self.fs.volume(path)?.features;
        if !features.xattr || (self.acl.is_some() && !features.acl) {
            let element = if self.acl.is_some() && !features.acl {
                "access control lists"
            } else {
                "extended attributes"
            };
            return Err(Error::new(
                ErrorCode::Unsupported,
                format!("the volume of `{path}` does not support {element}"),
            )
            .with_detail("path", Json::from(path.to_language_string())));
        }
        let extended = self.fs.extended_metadata(path, Follow::Follow)?;
        let existing = |name: &str| {
            extended
                .xattrs
                .iter()
                .find(|xattr| xattr.name == name)
                .map(|xattr| xattr.value.as_slice())
        };
        if let Some(entries) = &self.acl {
            let encoded = acl::encode(entries);
            if existing(ACCESS_XATTR) != Some(encoded.as_slice()) {
                draft.change.set_xattrs.push(Xattr {
                    name: ACCESS_XATTR.into(),
                    value: encoded,
                });
                draft.elements.push(ACL);
            }
        }
        let mut changed = false;
        for (name, value) in &settings.xattrs {
            match (value, existing(name)) {
                (Some(value), current) if current != Some(value.as_slice()) => {
                    draft.change.set_xattrs.push(Xattr {
                        name: name.into(),
                        value: value.clone(),
                    });
                    changed = true;
                }
                (None, Some(_)) => {
                    draft.change.remove_xattrs.push(name.into());
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            draft.elements.push(XATTRS);
        }
        Ok(())
    }

    fn finish(self, no_roots: bool) -> EffectPlan {
        let mut plan = EffectPlan::new();
        plan.changes = self
            .planned
            .iter()
            .map(|target| {
                let declared = fs_target(
                    &target.path,
                    TargetAction::Attributes,
                    target.expect.clone(),
                );
                let mut change = planned_change(&declared);
                change.flags = flags_field(&target.flags);
                change
            })
            .collect();
        plan.notable = self.notable;
        if self.unchanged_below > 0 {
            plan.notable
                .push(counted(Notable::Unchanged, self.unchanged_below));
        }
        if self.denied > 0 {
            plan.notable
                .push(counted(Notable::PermissionDeniedSkipped, self.denied));
        }
        if no_roots {
            plan.notable
                .push(NotableEntry::new(Notable::NothingMatched));
        }
        plan.bulk = self.planned.len() > 1;
        plan.with_state(PlanState {
            targets: self.planned,
        })
    }
}

fn flags_field(flags: &[&'static str]) -> Option<Vec<String>> {
    (!flags.is_empty()).then(|| flags.iter().map(|flag| (*flag).to_owned()).collect())
}

fn unknown_account(parameter: &str, name: &str) -> Error {
    let kind = if parameter == OWNER {
        "account"
    } else {
        "group"
    };
    Error::new(
        ErrorCode::InvalidValue,
        format!("no {kind} is named `{name}`: give a name from the {kind} table or a numeric id"),
    )
    .with_found(name)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::VolumeKind;
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::record::{Change, ChangeAction};
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::Fields;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::host::TestHost;
    use nurt_testkit::journal::JournalCall;
    use nurt_testkit::vfs::{VolumeConfig, test_path};

    use super::*;
    use crate::common::etag::etag_text;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn args(paths: &[&str], params: &[(&str, Value)]) -> BoundArgs {
        let list = paths
            .iter()
            .map(|path| Value::Path(abs(path).to_language_string()))
            .collect();
        let mut args = BoundArgs::new(COMMAND)
            .with(PATH, ArgValue::Value(Value::List(list)), false)
            .with(RECURSIVE, ArgValue::Value(Value::Bool(false)), true);
        for (name, value) in params {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    struct Outcome {
        planned: Vec<Change>,
        notable: Vec<NotableEntry>,
        applied: Vec<Change>,
        effect: Effect,
    }

    fn run(test: &TestContext, args: &BoundArgs, input: StageInput<'_>) -> Result<Outcome> {
        let ctx = test.ctx();
        let plan = Attributes.plan(args, input, &ctx)?;
        let planned = plan.changes.clone();
        let notable = plan.notable.clone();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = Attributes.apply(args, plan, tx.as_mut(), &ctx)?;
        let StageData::Records(mut records) = outcome.output.data else {
            panic!("`attributes` returns an effect record");
        };
        let effect = Effect::try_from(records.next().unwrap()?).map_err(Error::from)?;
        let applied = outcome.changes;
        tx.commit()?;
        Ok(Outcome {
            planned,
            notable,
            applied,
            effect,
        })
    }

    fn meta(test: &TestContext, path: &str) -> FileMeta {
        test.fs().metadata(&abs(path), Follow::NoFollow).unwrap()
    }

    fn file(test: &TestContext, path: &str) {
        test.fs().write_file(&abs(path), b"content").unwrap();
    }

    fn codes(notable: &[NotableEntry]) -> Vec<Notable> {
        notable.iter().map(|entry| entry.code).collect()
    }

    #[test]
    fn every_change_goes_through_the_transaction() {
        let test = TestContext::new();
        file(&test, "/data/a.txt");
        file(&test, "/data/b.txt");
        let call = args(&["/data/a.txt", "/data/b.txt"], &[(MODE, text("0600"))]);
        let outcome = run(&test, &call, StageInput::none()).unwrap();
        let declared: Vec<_> = test
            .journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Preserve { targets, .. } => Some(targets),
                _ => None,
            })
            .flatten()
            .collect();
        let applied: Vec<_> = test
            .journal()
            .calls()
            .into_iter()
            .filter(|call| {
                matches!(
                    call,
                    JournalCall::Apply {
                        ok: true,
                        mutation: "set_meta",
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(declared.len(), 2);
        assert_eq!(applied.len(), declared.len());
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o600);
        assert_eq!(meta(&test, "/data/b.txt").mode, 0o600);
        assert_eq!(outcome.applied.len(), 2);
        assert!(
            outcome
                .applied
                .iter()
                .all(|change| change.action == ChangeAction::Attributes)
        );
        assert_eq!(outcome.planned.len(), 2);
        assert_eq!(outcome.effect.kind, COMMAND);
        assert_eq!(outcome.effect.targets, Some(2));
        assert_eq!(
            outcome.effect.notable,
            Some(vec!["attributes_changed".to_owned()])
        );
    }

    #[test]
    fn values_equal_to_the_current_ones_change_nothing() {
        let test = TestContext::new();
        file(&test, "/data/a.txt");
        let call = args(&["/data/a.txt"], &[(MODE, text("644"))]);
        let outcome = run(&test, &call, StageInput::none()).unwrap();
        assert!(outcome.applied.is_empty());
        assert_eq!(codes(&outcome.notable), [Notable::Unchanged]);
        assert!(
            !test
                .journal()
                .calls()
                .iter()
                .any(|call| matches!(call, JournalCall::Apply { .. }))
        );
    }

    #[test]
    fn a_request_without_changes_is_rejected() {
        let test = TestContext::new();
        file(&test, "/data/a.txt");
        let error = run(&test, &args(&["/data/a.txt"], &[]), StageInput::none())
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        let error = run(
            &test,
            &args(&["/data/a.txt"], &[(MODE, text("u+q"))]),
            StageInput::none(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn missing_targets_fail_before_any_change() {
        let test = TestContext::new();
        file(&test, "/data/a.txt");
        let call = args(&["/data/a.txt", "/data/gone.txt"], &[(MODE, text("0600"))]);
        let error = run(&test, &call, StageInput::none()).err().unwrap();
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o644);
    }

    #[test]
    fn stale_input_entries_fail_with_stale_and_change_nothing() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"one").unwrap();
        let tag = etag_text(&test.fs().etag(&path, Follow::NoFollow).unwrap());
        test.fs().write_file(&path, b"changed").unwrap();
        let record = Record::new(RecordKind::Entry)
            .with("path", Value::Path(path.to_language_string()))
            .with(ETAG_FIELD, Value::Text(tag));
        let call = BoundArgs::new(COMMAND).with(MODE, ArgValue::Value(text("0600")), false);
        let error = run(
            &test,
            &call,
            StageInput::records(RecordStream::once(record)),
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o644);
    }

    #[test]
    fn current_input_entries_are_changed() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"one").unwrap();
        let tag = etag_text(&test.fs().etag(&path, Follow::NoFollow).unwrap());
        let record = Record::new(RecordKind::Entry)
            .with("path", Value::Path(path.to_language_string()))
            .with(ETAG_FIELD, Value::Text(tag));
        let call = BoundArgs::new(COMMAND).with(MODE, ArgValue::Value(text("u+x")), false);
        run(
            &test,
            &call,
            StageInput::records(RecordStream::once(record)),
        )
        .unwrap();
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o744);
    }

    #[test]
    fn recursive_changes_reach_entries_below_and_skip_links() {
        let test = TestContext::new();
        file(&test, "/site/index.html");
        file(&test, "/site/css/main.css");
        test.fs()
            .symlink(abs("/outside.txt").as_path(), &abs("/site/link"))
            .unwrap();
        file(&test, "/outside.txt");
        let call = args(
            &["/site"],
            &[(MODE, text("go-rwx")), (RECURSIVE, Value::Bool(true))],
        );
        let outcome = run(&test, &call, StageInput::none()).unwrap();
        assert_eq!(meta(&test, "/site").mode, 0o700);
        assert_eq!(meta(&test, "/site/css").mode, 0o700);
        assert_eq!(meta(&test, "/site/css/main.css").mode, 0o600);
        assert_eq!(meta(&test, "/site/index.html").mode, 0o600);
        assert_eq!(meta(&test, "/outside.txt").mode, 0o644);
        assert_eq!(outcome.applied.len(), 4);
        assert_eq!(
            outcome.applied[0].path.as_str(),
            abs("/site").to_language_string()
        );
    }

    #[test]
    fn recursive_on_a_file_changes_the_file_only() {
        let test = TestContext::new();
        file(&test, "/data/a.txt");
        let call = args(
            &["/data/a.txt"],
            &[(MODE, text("0600")), (RECURSIVE, Value::Bool(true))],
        );
        let outcome = run(&test, &call, StageInput::none()).unwrap();
        assert_eq!(outcome.applied.len(), 1);
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o600);
    }

    #[test]
    fn an_entry_reached_twice_is_one_target() {
        let test = TestContext::new();
        file(&test, "/site/index.html");
        let twice = args(
            &["/site/index.html", "/site/index.html"],
            &[(MODE, text("0600"))],
        );
        let outcome = run(&test, &twice, StageInput::none()).unwrap();
        assert_eq!(outcome.applied.len(), 1);
        assert_eq!(outcome.effect.targets, Some(1));
        let below = args(
            &["/site", "/site/index.html"],
            &[(MODE, text("0640")), (RECURSIVE, Value::Bool(true))],
        );
        let outcome = run(&test, &below, StageInput::none()).unwrap();
        let mut paths: Vec<_> = outcome.applied.iter().map(|c| c.path.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), outcome.applied.len());
        assert_eq!(meta(&test, "/site/index.html").mode, 0o640);
    }

    #[test]
    fn entries_below_that_already_match_are_counted_once() {
        let test = TestContext::new();
        file(&test, "/site/index.html");
        file(&test, "/site/css/main.css");
        let call = args(
            &["/site"],
            &[(MODE, text("go-rwx")), (RECURSIVE, Value::Bool(true))],
        );
        run(&test, &call, StageInput::none()).unwrap();
        let again = run(&test, &call, StageInput::none()).unwrap();
        assert!(again.applied.is_empty());
        let counted = again
            .notable
            .iter()
            .rev()
            .find(|entry| entry.code == Notable::Unchanged)
            .unwrap();
        assert_eq!(counted.count, Some(3));
    }

    #[test]
    fn read_only_and_mode_combine_for_one_entry() {
        let test = TestContext::with_host(TestHost::linux());
        file(&test, "/data/a.txt");
        let call = args(
            &["/data/a.txt"],
            &[(MODE, text("0664")), (READONLY, Value::Bool(true))],
        );
        run(&test, &call, StageInput::none()).unwrap();
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o444);
    }

    #[test]
    fn times_are_set_and_creation_time_is_partial_on_linux() {
        let test = TestContext::with_host(TestHost::linux());
        file(&test, "/data/a.txt");
        let instant: Instant = "2026-01-31T12:00:00Z".parse().unwrap();
        let call = args(
            &["/data/a.txt"],
            &[
                (MODIFIED, Value::Instant(instant)),
                (ACCESSED, text("2026-01-30T08:00:00Z")),
                (CREATED, Value::Instant(instant)),
            ],
        );
        let outcome = run(&test, &call, StageInput::none()).unwrap();
        let after = meta(&test, "/data/a.txt");
        assert_eq!(after.modified_ns, instant.as_unix_nanos());
        assert_ne!(after.created_ns, Some(instant.as_unix_nanos()));
        assert_eq!(
            outcome.applied[0].flags,
            Some(vec![FLAG_CREATED_UNSUPPORTED.to_owned()])
        );
        let wrong = args(&["/data/a.txt"], &[(MODIFIED, text("yesterday"))]);
        let error = run(&test, &wrong, StageInput::none()).err().unwrap();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn macos_sets_the_creation_time_and_keeps_hidden_in_the_name() {
        let test = TestContext::with_host(TestHost::macos());
        file(&test, "/data/a.txt");
        let instant: Instant = "2020-05-01T00:00:00Z".parse().unwrap();
        let call = args(
            &["/data/a.txt"],
            &[
                (CREATED, Value::Instant(instant)),
                (HIDDEN, Value::Bool(true)),
            ],
        );
        let outcome = run(&test, &call, StageInput::none()).unwrap();
        assert_eq!(
            meta(&test, "/data/a.txt").created_ns,
            Some(instant.as_unix_nanos())
        );
        assert_eq!(
            outcome.applied[0].flags,
            Some(vec![FLAG_HIDDEN_UNSUPPORTED.to_owned()])
        );

        let hidden_only = args(&["/data/a.txt"], &[(HIDDEN, Value::Bool(true))]);
        let outcome = run(&test, &hidden_only, StageInput::none()).unwrap();
        assert!(outcome.applied.is_empty());
        assert_eq!(
            outcome.notable[0].detail.as_deref(),
            Some(FLAG_HIDDEN_UNSUPPORTED)
        );
    }

    #[test]
    fn windows_mode_sets_the_read_only_attribute_only() {
        let test = TestContext::with_host(TestHost::windows());
        file(&test, "/data/a.txt");
        let call = args(
            &["/data/a.txt"],
            &[(MODE, text("0444")), (HIDDEN, Value::Bool(true))],
        );
        let outcome = run(&test, &call, StageInput::none()).unwrap();
        let after = meta(&test, "/data/a.txt");
        assert_eq!(
            after.windows_attributes & WINDOWS_READONLY,
            WINDOWS_READONLY
        );
        assert_eq!(after.windows_attributes & WINDOWS_HIDDEN, WINDOWS_HIDDEN);
        assert_eq!(after.mode, 0o644);
        assert_eq!(
            outcome.applied[0].flags,
            Some(vec![FLAG_ACL_UNCHANGED.to_owned()])
        );
        assert_eq!(
            outcome.notable[0].detail.as_deref(),
            Some("readonly, hidden")
        );

        let writable = args(&["/data/a.txt"], &[(READONLY, Value::Bool(false))]);
        run(&test, &writable, StageInput::none()).unwrap();
        assert_eq!(
            meta(&test, "/data/a.txt").windows_attributes & WINDOWS_READONLY,
            0
        );
    }

    #[test]
    fn windows_refuses_accounts_acl_and_extended_attributes() {
        let test = TestContext::with_host(TestHost::windows());
        file(&test, "/data/a.txt");
        let mut xattrs = Fields::new();
        xattrs.insert("user.a".to_owned(), text("1"));
        for param in [
            (OWNER, text("user")),
            (XATTRS, Value::Map(xattrs)),
            (ACL, Value::List(Vec::new())),
        ] {
            let error = run(&test, &args(&["/data/a.txt"], &[param]), StageInput::none())
                .err()
                .unwrap();
            assert_eq!(error.code(), ErrorCode::Unsupported, "{}", error.message);
        }
        let mac = TestContext::with_host(TestHost::macos());
        file(&mac, "/data/a.txt");
        let error = run(
            &mac,
            &args(&["/data/a.txt"], &[(ACL, Value::List(Vec::new()))]),
            StageInput::none(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), ErrorCode::Unsupported);
    }

    #[test]
    fn read_only_clears_and_restores_write_bits_on_unix() {
        let test = TestContext::with_host(TestHost::linux());
        file(&test, "/data/a.txt");
        run(
            &test,
            &args(&["/data/a.txt"], &[(READONLY, Value::Bool(true))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o444);
        run(
            &test,
            &args(&["/data/a.txt"], &[(READONLY, Value::Bool(false))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o644);
    }

    #[test]
    fn accounts_resolve_names_and_need_privilege() {
        let test = TestContext::with_host(TestHost::linux());
        if cfg!(windows) {
            return;
        }
        test.fs()
            .write_file(
                &abs("/etc/passwd"),
                b"root:x:0:0::/root:/bin/sh\nuser:x:1000:1000::/home/user:/bin/sh\n",
            )
            .unwrap();
        test.fs()
            .write_file(&abs("/etc/group"), b"staff:x:50:user\n")
            .unwrap();
        file(&test, "/data/a.txt");
        run(
            &test,
            &args(&["/data/a.txt"], &[(GROUP, text("staff"))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(meta(&test, "/data/a.txt").gid, Some(50));

        let error = run(
            &test,
            &args(&["/data/a.txt"], &[(OWNER, text("root"))]),
            StageInput::none(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), ErrorCode::Privilege);
        let error = run(
            &test,
            &args(&["/data/a.txt"], &[(OWNER, text("nobody"))]),
            StageInput::none(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        let same = run(
            &test,
            &args(&["/data/a.txt"], &[(OWNER, text("user"))]),
            StageInput::none(),
        )
        .unwrap();
        assert!(same.applied.is_empty());
    }

    #[test]
    fn superuser_changes_the_account() {
        let test = TestContext::with_host(TestHost::linux());
        if cfg!(windows) {
            return;
        }
        test.fs()
            .write_file(&abs("/etc/passwd"), b"user:x:0:0::/root:/bin/sh\n")
            .unwrap();
        file(&test, "/data/a.txt");
        run(
            &test,
            &args(&["/data/a.txt"], &[(OWNER, text("4242"))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(meta(&test, "/data/a.txt").uid, Some(4242));
    }

    #[test]
    fn extended_attributes_are_set_and_removed() {
        let test = TestContext::with_host(TestHost::linux());
        file(&test, "/data/a.txt");
        let mut set = Fields::new();
        set.insert("user.origin".to_owned(), text("camera"));
        run(
            &test,
            &args(&["/data/a.txt"], &[(XATTRS, Value::Map(set.clone()))]),
            StageInput::none(),
        )
        .unwrap();
        let read = |test: &TestContext| {
            test.fs()
                .extended_metadata(&abs("/data/a.txt"), Follow::NoFollow)
                .unwrap()
                .xattrs
        };
        assert_eq!(read(&test).len(), 1);
        let again = run(
            &test,
            &args(&["/data/a.txt"], &[(XATTRS, Value::Map(set))]),
            StageInput::none(),
        )
        .unwrap();
        assert!(again.applied.is_empty());
        let mut remove = Fields::new();
        remove.insert("user.origin".to_owned(), Value::Null);
        run(
            &test,
            &args(&["/data/a.txt"], &[(XATTRS, Value::Map(remove))]),
            StageInput::none(),
        )
        .unwrap();
        assert!(read(&test).is_empty());
    }

    #[test]
    fn acl_is_written_to_volumes_that_support_it() {
        let test = TestContext::with_host(TestHost::linux());
        let mount = abs("/acl");
        test.fs().create_dir_all(&mount).unwrap();
        let mut features = VolumeConfig::default_features();
        features.acl = true;
        test.fs()
            .add_volume(
                &mount,
                VolumeConfig {
                    fs_type: "ext4".to_owned(),
                    kind: VolumeKind::Fixed,
                    features,
                    ..VolumeConfig::default()
                },
            )
            .unwrap();
        file(&test, "/acl/a.txt");
        file(&test, "/data/a.txt");
        let entry = |tag: &str, perms: &str| {
            let mut fields = Fields::new();
            fields.insert("tag".to_owned(), text(tag));
            fields.insert("perms".to_owned(), text(perms));
            Value::Map(fields)
        };
        let list = Value::List(vec![
            entry("user", "rw"),
            entry("group", "r"),
            entry("other", ""),
        ]);
        run(
            &test,
            &args(&["/acl/a.txt"], &[(ACL, list.clone())]),
            StageInput::none(),
        )
        .unwrap();
        let extended = test
            .fs()
            .extended_metadata(&abs("/acl/a.txt"), Follow::NoFollow)
            .unwrap();
        let stored = extended
            .xattrs
            .iter()
            .find(|xattr| xattr.name == ACCESS_XATTR)
            .unwrap();
        assert_eq!(
            acl::decode(&stored.value).map(|entries| entries.len()),
            Some(3)
        );
        let error = run(
            &test,
            &args(&["/data/a.txt"], &[(ACL, list)]),
            StageInput::none(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), ErrorCode::Unsupported);
    }

    #[test]
    fn metadata_round_trips_through_a_second_change() {
        let test = TestContext::with_host(TestHost::linux());
        file(&test, "/data/a.txt");
        let path = abs("/data/a.txt");
        let before = test
            .fs()
            .extended_metadata(&path, Follow::NoFollow)
            .unwrap();
        let mut set = Fields::new();
        set.insert("user.tag".to_owned(), text("x"));
        let call = args(
            &["/data/a.txt"],
            &[
                (MODE, text("0600")),
                (MODIFIED, text("2001-01-01T00:00:00Z")),
                (XATTRS, Value::Map(set)),
            ],
        );
        run(&test, &call, StageInput::none()).unwrap();
        let mut remove = Fields::new();
        remove.insert("user.tag".to_owned(), Value::Null);
        let restore = args(
            &["/data/a.txt"],
            &[
                (
                    MODE,
                    text(&format!("{:o}", before.basic.mode & PERMISSION_BITS)),
                ),
                (
                    MODIFIED,
                    Value::Instant(Instant::from_unix_nanos(before.basic.modified_ns)),
                ),
                (XATTRS, Value::Map(remove)),
            ],
        );
        run(&test, &restore, StageInput::none()).unwrap();
        let after = test
            .fs()
            .extended_metadata(&path, Follow::NoFollow)
            .unwrap();
        assert_eq!(after.basic.mode, before.basic.mode);
        assert_eq!(after.basic.modified_ns, before.basic.modified_ns);
        assert_eq!(after.basic.uid, before.basic.uid);
        assert_eq!(after.xattrs, before.xattrs);
        assert_eq!(after.acl, before.acl);
    }

    #[test]
    fn links_change_the_entry_they_resolve_to() {
        let test = TestContext::new();
        file(&test, "/data/a.txt");
        test.fs()
            .symlink(abs("/data/a.txt").as_path(), &abs("/data/link"))
            .unwrap();
        run(
            &test,
            &args(&["/data/link"], &[(MODE, text("0600"))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(meta(&test, "/data/a.txt").mode, 0o600);
        test.fs()
            .symlink(abs("/data/gone").as_path(), &abs("/data/dangling"))
            .unwrap();
        let error = run(
            &test,
            &args(&["/data/dangling"], &[(MODE, text("0600"))]),
            StageInput::none(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn an_empty_input_matches_nothing() {
        let test = TestContext::new();
        let call = BoundArgs::new(COMMAND).with(MODE, ArgValue::Value(text("0600")), false);
        let outcome = run(&test, &call, StageInput::records(RecordStream::empty())).unwrap();
        assert!(outcome.applied.is_empty());
        assert_eq!(codes(&outcome.notable), [Notable::NothingMatched]);
    }
}
