// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Run-time limit changes and retention status.
//!
//! `journal.limits` with `scope: session` keeps the limits of a session in a
//! file of the state directory (`journal-limits/<hash>.json`), never in the
//! memory of a process; the file is removed after `session.ttl` without
//! calls of the session or when the session ends. The limits of all live
//! sessions form the run-time level; when sessions set the same limit, the
//! most recent change wins. `scope: user` writes the `[journal]` section of
//! the user configuration file through a transaction that does not count as
//! a step.
//!
//! Lowering a limit expires entries at once; raising it never restores
//! expired entries. `journal.status` is the only report of retention state.

use std::collections::BTreeSet;

use nurt_contracts::config::{
    AgeLimit, ConfigLayer, ConfigLevel, Resolved, SizeLimit, StepLimit, resolve,
};
use nurt_contracts::env::{ClockStamp, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    EffectiveRetention, EntryState, JournalState, LimitsOutcome, LimitsRequest, LimitsScope,
    OldestEntry, OpId, SessionId, VolumeRetention,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, TableLike};

use super::ctx::CtxStore;
use super::expiry::{Snapshot, expirable, select_volumes};
use super::port::{Area, EntryFacts, RetentionStore, VolumeFacts};
use super::resolve::{LimitValues, RuntimeLimits, validate};
use super::tomb;
use crate::store::JournalCtx;

/// Directory of session limit files in the state directory.
pub(crate) const SESSION_DIR: &str = "journal-limits";

/// Suffix of session limit files.
const SESSION_SUFFIX: &str = ".json";

/// Limits of one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionFile {
    session: SessionId,
    /// Time of the last change of the limits.
    changed: WallTime,
    /// Time of the last call of the session.
    touched: WallTime,
    limits: RuntimeLimits,
}

/// Relative name of the limit file of a session: the identifier is hashed,
/// so any identifier gives a valid file name.
fn session_file(session: &SessionId) -> String {
    let digest = Sha256::digest(session.0.as_bytes());
    format!("{SESSION_DIR}/{}{SESSION_SUFFIX}", hex::encode(digest))
}

fn read_session(store: &dyn RetentionStore, name: &str) -> Result<Option<SessionFile>> {
    let Some(content) = store.read_file(Area::State, name)? else {
        return Ok(None);
    };
    Ok(serde_json::from_slice(&content).ok())
}

fn write_session(store: &dyn RetentionStore, file: &SessionFile) -> Result<()> {
    let content = serde_json::to_vec(file).map_err(|error| {
        Error::new(
            ErrorCode::Internal,
            format!(
                "cannot serialize the limits of session {}: {error}",
                file.session.0
            ),
        )
    })?;
    store.write_file(Area::State, &session_file(&file.session), &content)
}

/// Limits of the run-time level: the limits of every live session, the most
/// recent change last. Files of sessions without calls for `session.ttl`
/// and unreadable files are removed.
///
/// # Errors
///
/// Returns `E_IO` when the state directory cannot be read or a stale file
/// cannot be removed.
pub(crate) fn runtime_limits(
    store: &dyn RetentionStore,
    resolved: &Resolved,
    now: &ClockStamp,
) -> Result<Option<RuntimeLimits>> {
    let ttl = resolved.config.session.ttl.as_std();
    let mut live = Vec::new();
    for name in store.list_dir(Area::State, SESSION_DIR)? {
        if !name.ends_with(SESSION_SUFFIX) {
            continue;
        }
        let path = format!("{SESSION_DIR}/{name}");
        match read_session(store, &path)? {
            Some(file) if file.touched.saturating_add(ttl) > now.wall => live.push(file),
            _ => store.remove_file(Area::State, &path)?,
        }
    }
    Ok(merge_sessions(live))
}

/// Limits of the run-time level read from the contents of session files,
/// without evaluating their lifetime; unreadable contents are skipped.
pub(crate) fn sessions_from_contents(contents: &[Vec<u8>]) -> Option<RuntimeLimits> {
    merge_sessions(
        contents
            .iter()
            .filter_map(|content| serde_json::from_slice(content).ok())
            .collect(),
    )
}

/// Merges the limits of sessions; the most recent change wins.
fn merge_sessions(mut live: Vec<SessionFile>) -> Option<RuntimeLimits> {
    live.sort_by(|a, b| (a.changed, &a.session).cmp(&(b.changed, &b.session)));
    let mut merged = RuntimeLimits::default();
    for file in &live {
        merged.set(None, &file.limits.all);
        for (mount, values) in &file.limits.volumes {
            merged.set(Some(mount), values);
        }
    }
    (!merged.is_empty()).then_some(merged)
}

/// Records a call of `session`, which keeps its limits alive.
///
/// # Errors
///
/// Returns `E_IO` when the session file cannot be read or written.
pub(crate) fn touch_session(
    store: &dyn RetentionStore,
    session: &SessionId,
    now: &ClockStamp,
) -> Result<()> {
    if let Some(mut file) = read_session(store, &session_file(session))? {
        file.touched = now.wall;
        write_session(store, &file)?;
    }
    Ok(())
}

/// Removes the limits of a session that ended.
///
/// # Errors
///
/// Returns `E_IO` when the session file cannot be removed.
pub(crate) fn end_session_store(store: &dyn RetentionStore, session: &SessionId) -> Result<()> {
    store.remove_file(Area::State, &session_file(session))
}

/// Shows or changes limits (`journal.limits`).
///
/// Without values the effective limits of the selected volumes are
/// reported and nothing changes. With values the limits are validated,
/// stored at the requested scope and the expiry pass runs at once; with
/// `preview` the entries that would expire are reported and nothing
/// changes.
///
/// # Errors
///
/// Returns `E_NOT_FOUND` when no volume contains the requested path,
/// `E_INVALID_VALUE` for a value out of range, `E_CONFIG` for an invalid
/// user configuration file and `E_IO` when a file cannot be written or an
/// entry cannot be reclaimed.
pub(crate) fn limits_store(
    store: &dyn RetentionStore,
    request: &LimitsRequest,
) -> Result<LimitsOutcome> {
    let values = LimitValues {
        steps: request.steps,
        age: request.age,
        size: request.size,
    };
    let mut snapshot = Snapshot::load(store)?;
    let selected: Vec<VolumeFacts> = select_volumes(&snapshot.volumes, request.volume.as_ref())?
        .into_iter()
        .cloned()
        .collect();
    if values.is_empty() {
        if let LimitsScope::Session(session) = &request.scope {
            touch_session(store, session, &snapshot.now)?;
        }
        let limits = snapshot.limits();
        return Ok(LimitsOutcome {
            effective: effective_of(&selected, &limits),
            expiring: Vec::new(),
            expiring_bytes: 0,
            applied: false,
        });
    }
    for volume in &selected {
        validate(&values, volume.capacity)?;
    }
    let mount = match (&request.volume, selected.first()) {
        (Some(_), Some(volume)) => Some(volume.mount.to_language_string()),
        _ => None,
    };
    match &request.scope {
        LimitsScope::Session(session) => {
            apply_session(
                store,
                &mut snapshot,
                session,
                mount.as_deref(),
                &values,
                request.preview,
            )?;
        }
        LimitsScope::User => {
            apply_user(
                store,
                &mut snapshot,
                mount.as_deref(),
                &values,
                request.preview,
            )?;
        }
    }
    let limits = snapshot.limits();
    let plan = snapshot.plan(&limits);
    let expiring: Vec<OpId> = plan.iter().map(|expired| expired.op).collect();
    let expiring_bytes = bytes_of(&snapshot.entries, &expiring);
    if !request.preview {
        snapshot.execute(store, &plan)?;
    }
    Ok(LimitsOutcome {
        effective: effective_of(&selected, &limits),
        expiring,
        expiring_bytes,
        applied: !request.preview,
    })
}

fn apply_session(
    store: &dyn RetentionStore,
    snapshot: &mut Snapshot,
    session: &SessionId,
    mount: Option<&str>,
    values: &LimitValues,
    preview: bool,
) -> Result<()> {
    let mut runtime = snapshot.runtime.take().unwrap_or_default();
    runtime.set(mount, values);
    snapshot.runtime = Some(runtime);
    if preview {
        return Ok(());
    }
    let now = snapshot.now.wall;
    let mut file = read_session(store, &session_file(session))?.unwrap_or_else(|| SessionFile {
        session: session.clone(),
        changed: now,
        touched: now,
        limits: RuntimeLimits::default(),
    });
    file.limits.set(mount, values);
    file.changed = now;
    file.touched = now;
    write_session(store, &file)
}

fn apply_user(
    store: &dyn RetentionStore,
    snapshot: &mut Snapshot,
    mount: Option<&str>,
    values: &LimitValues,
    preview: bool,
) -> Result<()> {
    let edited = edit_user_config(store.read_user_config()?.as_deref(), mount, values)?;
    let layers = with_user_values(store.config_layers(), mount, values)?;
    snapshot.resolved = resolve(&layers, store.project_trusted())
        .map_err(|error| Error::new(ErrorCode::Config, error.to_string()))?;
    if !preview {
        store.write_user_config(&edited)?;
    }
    Ok(())
}

/// Sets limits in the `[journal]` section of the user configuration file,
/// or in `[journal.volume."<mount>"]`, keeping comments and layout.
///
/// # Errors
///
/// Returns `E_CONFIG` when the file is not valid TOML or a section is not a
/// table.
fn edit_user_config(
    current: Option<&str>,
    mount: Option<&str>,
    values: &LimitValues,
) -> Result<String> {
    let mut document: DocumentMut = current.unwrap_or_default().parse().map_err(|error| {
        Error::new(
            ErrorCode::Config,
            format!("the user configuration file is not valid TOML; correct it first: {error}"),
        )
    })?;
    let journal = child_table(document.as_table_mut(), "journal")?;
    let target = match mount {
        Some(mount) => child_table(child_table(journal, "volume")?, mount)?,
        None => journal,
    };
    for (key, value) in setting_texts(values) {
        let item = match value {
            Value::Number(number) => number.as_i64().map(toml_edit::value),
            Value::String(text) => Some(toml_edit::value(text)),
            _ => None,
        };
        if let Some(item) = item {
            target.insert(key, item);
        }
    }
    Ok(document.to_string())
}

fn child_table<'t>(parent: &'t mut dyn TableLike, key: &str) -> Result<&'t mut dyn TableLike> {
    parent
        .entry(key)
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_like_mut()
        .ok_or_else(|| {
            Error::new(
                ErrorCode::Config,
                format!("{key} in the user configuration file is not a table; correct it first"),
            )
        })
}

/// Keys and configuration values of the set limits.
fn setting_texts(values: &LimitValues) -> Vec<(&'static str, Value)> {
    let mut settings = Vec::new();
    if let Some(steps) = values.steps {
        settings.push((
            "max_ops",
            match steps {
                StepLimit::Limited(count) => Value::from(count),
                StepLimit::Unlimited => Value::from(steps.to_string()),
            },
        ));
    }
    if let Some(age) = values.age {
        settings.push(("max_age", Value::from(AgeLimit::to_string(&age))));
    }
    if let Some(size) = values.size {
        settings.push(("max_size", Value::from(SizeLimit::to_string(&size))));
    }
    settings
}

/// Configuration layers with the limits added to the user layer.
fn with_user_values(
    mut layers: Vec<ConfigLayer>,
    mount: Option<&str>,
    values: &LimitValues,
) -> Result<Vec<ConfigLayer>> {
    let index = match layers
        .iter()
        .rposition(|layer| layer.level == ConfigLevel::User)
    {
        Some(index) => index,
        None => {
            layers.push(ConfigLayer::new(ConfigLevel::User, "user", Map::new()));
            layers.len() - 1
        }
    };
    let mut path = vec!["journal".to_owned()];
    if let Some(mount) = mount {
        path.extend(["volume".to_owned(), mount.to_owned()]);
    }
    let mut table = &mut layers[index].values;
    for name in &path {
        let slot = table
            .entry(name.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        table = slot.as_object_mut().ok_or_else(|| {
            Error::new(
                ErrorCode::Config,
                format!("{name} in the user configuration is not a table; correct it first"),
            )
        })?;
    }
    for (key, value) in setting_texts(values) {
        table.insert(key.to_owned(), value);
    }
    Ok(layers)
}

fn effective_of(
    selected: &[VolumeFacts],
    limits: &std::collections::BTreeMap<AbsPath, EffectiveRetention>,
) -> Vec<VolumeRetention> {
    selected
        .iter()
        .filter_map(|volume| {
            limits.get(&volume.mount).map(|limits| VolumeRetention {
                volume: volume.mount.clone(),
                limits: limits.clone(),
            })
        })
        .collect()
}

fn bytes_of(entries: &[EntryFacts], ops: &[OpId]) -> u64 {
    entries
        .iter()
        .filter(|part| ops.contains(&part.op))
        .map(|part| part.bytes_held)
        .sum()
}

/// Retention state of the selected volumes (`journal.status`).
///
/// A volume that contains the requested path but has no journal yet reports
/// an empty journal.
///
/// # Errors
///
/// Returns `E_NOT_FOUND` when no volume contains the requested path,
/// `E_CONFIG` for an invalid configuration and `E_IO` when the store
/// cannot be read.
pub(crate) fn status_store(
    store: &dyn RetentionStore,
    volume: Option<&AbsPath>,
) -> Result<Vec<JournalState>> {
    let mut snapshot = Snapshot::load(store)?;
    let mut unused = None;
    if let Some(path) = volume
        && select_volumes(&snapshot.volumes, Some(path)).is_err()
        && let Some(facts) = store.volume_at(path)?
    {
        unused = Some(facts.mount.clone());
        snapshot.volumes.push(facts);
    }
    let selected = select_volumes(&snapshot.volumes, volume)?;
    let limits = snapshot.limits();
    let over: BTreeSet<OpId> = snapshot
        .plan(&limits)
        .iter()
        .map(|expired| expired.op)
        .collect();
    let resolver = snapshot.resolver();
    let mut states = Vec::new();
    for volume in selected {
        let Some(effective) = limits.get(&volume.mount).cloned() else {
            continue;
        };
        let parts: Vec<&EntryFacts> = snapshot
            .entries
            .iter()
            .filter(|part| part.volume == volume.mount && !part.rolled_back)
            .collect();
        let expired = if unused.as_ref() == Some(&volume.mount) {
            0
        } else {
            u64::try_from(tomb::list(store, &volume.mount)?.len()).unwrap_or(u64::MAX)
        };
        let available =
            |part: &&&EntryFacts| matches!(part.state, EntryState::Committed | EntryState::Partial);
        let floor = resolver.floor().map(|mut floor| {
            floor.raised_to_floor = (effective.max_ops.raised_to_floor
                && effective.max_ops.source == floor.source)
                || (effective.max_age.raised_to_floor && effective.max_age.source == floor.source);
            floor
        });
        states.push(JournalState {
            volume: volume.mount.clone(),
            entries_total: count(parts.len()).saturating_add(expired),
            entries_available: count(parts.iter().filter(available).count()),
            entries_expired: expired,
            bytes_held: held(&parts, |part| part.bytes_held),
            bytes_physical: held(&parts, |part| part.bytes_physical),
            oldest: parts
                .iter()
                .filter(available)
                .min_by_key(|part| part.op)
                .map(|part| OldestEntry {
                    op: part.op,
                    time: part.time,
                }),
            notes: resolver.notes(&effective),
            limits: effective,
            floor,
            inactive: volume.inactive,
            over_limit: count(
                parts
                    .iter()
                    .filter(|part| {
                        part.state == EntryState::Expiring
                            || (expirable(part.state) && over.contains(&part.op))
                    })
                    .count(),
            ),
        });
    }
    Ok(states)
}

/// Retention state of the journal store per volume (`journal.status`).
///
/// A volume that contains `volume` but has no journal yet reports an empty
/// journal.
///
/// # Errors
///
/// Returns `E_NOT_FOUND` when no volume contains `volume`,
/// `E_CONFIG` for an invalid configuration and `E_IO` when a journal
/// directory cannot be read.
pub(crate) fn status(ctx: &JournalCtx, volume: Option<&AbsPath>) -> Result<Vec<JournalState>> {
    status_store(&CtxStore(ctx), volume)
}

/// Removes the session limits of `session` from the journal store; the host
/// calls it when the session ends, instead of waiting for the session time
/// to live to pass.
///
/// # Errors
///
/// Returns `E_IO` when the session file cannot be removed.
pub(crate) fn end_session(ctx: &JournalCtx, session: &SessionId) -> Result<()> {
    end_session_store(&CtxStore(ctx), session)
}

/// Shows or changes the limits of the journal store (`journal.limits`).
///
/// `scope: user` writes the user configuration file set by
/// [`JournalBuilder::user_config`](crate::JournalBuilder::user_config)
/// through a transaction of kind `journal.limits` that does not count as a
/// step and can be undone.
///
/// # Errors
///
/// As [`limits_store`]; `scope: user` with values fails with
/// `E_UNSUPPORTED` when the journal has no user configuration file.
pub(crate) fn limits(ctx: &JournalCtx, request: &LimitsRequest) -> Result<LimitsOutcome> {
    limits_store(&CtxStore(ctx), request)
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn held(parts: &[&EntryFacts], bytes: impl Fn(&EntryFacts) -> u64) -> u64 {
    parts
        .iter()
        .filter(|part| part.state != EntryState::Expiring)
        .map(|part| bytes(part))
        .sum()
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::DurationSetting;
    use nurt_contracts::journal::{
        ExpiryReason, JournalNotable, PurgeOutcome, PurgeRequest, PurgeSelector,
    };
    use serde_json::json;

    use super::super::expiry::purge_store;
    use super::super::port::StepKind;
    use super::super::testing::{GIB, MIB, MemStore, entry, mount, stamp_at};
    use super::*;

    const HOUR: u64 = 3600;

    /// Store with the volume `/home` of 500 GiB, 200 GiB free.
    fn store() -> MemStore {
        let store = MemStore::new();
        store.volume("/home", 500 * GIB, 200 * GIB);
        store
    }

    fn session(name: &str) -> LimitsScope {
        LimitsScope::Session(SessionId(name.to_owned()))
    }

    fn steps(count: u32, scope: LimitsScope, preview: bool) -> LimitsRequest {
        LimitsRequest {
            steps: Some(StepLimit::Limited(count)),
            age: None,
            size: None,
            scope,
            volume: None,
            preview,
        }
    }

    fn report_only(volume: Option<AbsPath>) -> LimitsRequest {
        LimitsRequest {
            steps: None,
            age: None,
            size: None,
            scope: session("s1"),
            volume,
            preview: false,
        }
    }

    fn ops(list: &[OpId]) -> Vec<u64> {
        list.iter().map(|op| op.0).collect()
    }

    fn effective_steps(store: &MemStore) -> StepLimit {
        limits_store(store, &report_only(None)).unwrap().effective[0]
            .limits
            .max_ops
            .value
    }

    #[test]
    fn lowering_the_step_limit_expires_entries_at_once() {
        let store = store();
        store.commit("/home", 1..=25);
        let outcome = limits_store(&store, &steps(10, session("s1"), false)).unwrap();
        assert!(outcome.applied);
        assert_eq!(ops(&outcome.expiring), (1..=15).collect::<Vec<_>>());
        assert_eq!(outcome.expiring_bytes, 15 * MIB);
        assert_eq!(
            outcome.effective[0].limits.max_ops.value,
            StepLimit::Limited(10)
        );
        assert_eq!(
            outcome.effective[0].limits.max_ops.source,
            ConfigLevel::Runtime
        );
        assert_eq!(store.live("/home"), (16..=25).collect::<Vec<_>>());
        let tombstone = tomb::read(&store, &mount("/home"), OpId(15))
            .unwrap()
            .unwrap();
        assert_eq!(tombstone.reason, ExpiryReason::Ops);
    }

    #[test]
    fn raising_a_limit_does_not_restore_expired_entries() {
        let store = store();
        store.commit("/home", 1..=25);
        limits_store(&store, &steps(10, session("s1"), false)).unwrap();
        let outcome = limits_store(&store, &steps(30, session("s1"), false)).unwrap();
        assert!(outcome.expiring.is_empty());
        assert_eq!(store.live("/home"), (16..=25).collect::<Vec<_>>());
        assert_eq!(effective_steps(&store), StepLimit::Limited(30));
    }

    #[test]
    fn preview_reports_the_effect_without_changing_anything() {
        let store = store();
        store.commit("/home", 1..=25);
        let outcome = limits_store(&store, &steps(10, session("s1"), true)).unwrap();
        assert!(!outcome.applied);
        assert_eq!(outcome.expiring.len(), 15);
        assert_eq!(outcome.expiring_bytes, 15 * MIB);
        assert_eq!(store.live("/home").len(), 25);
        assert!(store.list_dir(Area::State, SESSION_DIR).unwrap().is_empty());
        assert_eq!(effective_steps(&store), StepLimit::Limited(30));
    }

    #[test]
    fn report_without_values_shows_the_volume_of_a_path() {
        let store = store();
        store.volume("/mnt/data", 100 * GIB, 50 * GIB);
        let outcome = limits_store(&store, &report_only(Some(mount("/mnt/data/project")))).unwrap();
        assert!(!outcome.applied);
        assert_eq!(outcome.effective.len(), 1);
        assert_eq!(outcome.effective[0].volume, mount("/mnt/data"));
        assert_eq!(
            outcome.effective[0].limits.max_ops.source,
            ConfigLevel::Builtin
        );
        let all = limits_store(&store, &report_only(None)).unwrap();
        assert_eq!(all.effective.len(), 2);
        let error = limits_store(&store, &report_only(Some(mount("/srv")))).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn values_out_of_range_change_nothing() {
        let store = store();
        store.commit("/home", 1..=5);
        let error = limits_store(&store, &steps(0, session("s1"), false)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        let mut too_large = report_only(None);
        too_large.size = Some(SizeLimit::Bytes(
            nurt_contracts::config::SizeSetting::from_gib(600),
        ));
        let error = limits_store(&store, &too_large).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(store.live("/home").len(), 5);
    }

    #[test]
    fn volume_limit_applies_to_that_volume_only() {
        let store = store();
        store.volume("/mnt/data", 100 * GIB, 50 * GIB);
        store.commit("/home", 1..=3);
        store.commit("/mnt/data", 4..=6);
        let mut request = steps(1, session("s1"), false);
        request.volume = Some(mount("/mnt/data"));
        let outcome = limits_store(&store, &request).unwrap();
        assert_eq!(ops(&outcome.expiring), [4, 5]);
        assert_eq!(store.live("/home"), [1, 2, 3]);
        assert_eq!(effective_steps(&store), StepLimit::Limited(30));
    }

    #[test]
    fn session_limits_expire_without_calls() {
        let store = store();
        limits_store(&store, &steps(20, session("s1"), false)).unwrap();
        assert_eq!(store.list_dir(Area::State, SESSION_DIR).unwrap().len(), 1);

        store.set_clock(23 * HOUR);
        limits_store(&store, &report_only(None)).unwrap();
        store.set_clock(30 * HOUR);
        assert_eq!(effective_steps(&store), StepLimit::Limited(20));

        store.set_clock(60 * HOUR);
        let resolved = expiry_config(&store);
        assert_eq!(
            runtime_limits(&store, &resolved, &stamp_at(60 * HOUR)).unwrap(),
            None
        );
        assert!(store.list_dir(Area::State, SESSION_DIR).unwrap().is_empty());
    }

    #[test]
    fn ended_session_removes_its_limits() {
        let store = store();
        limits_store(&store, &steps(20, session("s1"), false)).unwrap();
        end_session_store(&store, &SessionId("s1".to_owned())).unwrap();
        assert_eq!(effective_steps(&store), StepLimit::Limited(30));
        touch_session(&store, &SessionId("s1".to_owned()), &stamp_at(1)).unwrap();
        assert!(store.list_dir(Area::State, SESSION_DIR).unwrap().is_empty());
    }

    #[test]
    fn most_recent_session_change_wins() {
        let store = store();
        store.set_clock(10);
        limits_store(&store, &steps(20, session("s1"), false)).unwrap();
        store.set_clock(20);
        limits_store(&store, &steps(40, session("s2"), false)).unwrap();
        assert_eq!(effective_steps(&store), StepLimit::Limited(40));
        store.set_clock(30);
        limits_store(&store, &steps(25, session("s1"), false)).unwrap();
        assert_eq!(effective_steps(&store), StepLimit::Limited(25));
    }

    #[test]
    fn unreadable_session_file_is_removed() {
        let store = store();
        store
            .write_file(Area::State, &format!("{SESSION_DIR}/broken.json"), b"{")
            .unwrap();
        store
            .write_file(Area::State, &format!("{SESSION_DIR}/notes.txt"), b"x")
            .unwrap();
        let resolved = expiry_config(&store);
        assert_eq!(
            runtime_limits(&store, &resolved, &stamp_at(0)).unwrap(),
            None
        );
        assert_eq!(
            store.list_dir(Area::State, SESSION_DIR).unwrap(),
            ["notes.txt"]
        );
    }

    #[test]
    fn user_scope_writes_the_configuration_file_without_a_step() {
        let store = store();
        store.set_user_config("# limits of the account\n[journal]\nmax_age = \"1h\"\n");
        store.commit("/home", 1..=12);
        let outcome = limits_store(&store, &steps(10, LimitsScope::User, false)).unwrap();
        assert_eq!(ops(&outcome.expiring), [1, 2]);
        assert_eq!(
            outcome.effective[0].limits.max_ops.source,
            ConfigLevel::User
        );
        let written = store.user_config().unwrap();
        assert!(written.starts_with("# limits of the account\n[journal]\n"));
        assert!(written.contains("max_age = \"1h\""));
        assert!(written.contains("max_ops = 10"));
        let writes = store.user_writes();
        assert_eq!(writes.len(), 1);
        let mark = store
            .steps()
            .unwrap()
            .into_iter()
            .find(|mark| mark.op == writes[0])
            .unwrap();
        assert_eq!(mark.kind, StepKind::NotCounted);
    }

    #[test]
    fn user_scope_preview_writes_nothing() {
        let store = store();
        let mut request = steps(10, LimitsScope::User, true);
        request.age = Some(AgeLimit::Unlimited);
        request.size = Some(SizeLimit::Unlimited);
        let outcome = limits_store(&store, &request).unwrap();
        assert!(!outcome.applied);
        assert_eq!(
            outcome.effective[0].limits.max_age.value,
            AgeLimit::Unlimited
        );
        assert_eq!(store.user_config(), None);
        assert!(store.user_writes().is_empty());
    }

    #[test]
    fn user_scope_writes_volume_settings_in_their_table() {
        let store = store();
        let mut request = steps(15, LimitsScope::User, false);
        request.volume = Some(mount("/home"));
        request.age = Some(AgeLimit::Limited(DurationSetting::from_secs(2 * HOUR)));
        limits_store(&store, &request).unwrap();
        let written = store.user_config().unwrap();
        assert!(written.contains("[journal.volume.\"/home\"]"), "{written}");
        assert!(written.contains("max_ops = 15"));
        assert!(written.contains("max_age = \"2h\""));

        request.steps = Some(StepLimit::Unlimited);
        request.size = Some(SizeLimit::Unlimited);
        let outcome = limits_store(&store, &request).unwrap();
        let written = store.user_config().unwrap();
        assert!(written.contains("max_size = \"unlimited\""), "{written}");
        let home = &outcome.effective[0];
        assert_eq!(home.volume, mount("/home"));
        assert_eq!(home.limits.max_size.value, SizeLimit::Unlimited);
        assert_eq!(home.limits.max_size.source, ConfigLevel::User);
    }

    #[test]
    fn user_scope_refuses_an_invalid_configuration_file() {
        let store = store();
        store.set_user_config("[journal\n");
        let error = limits_store(&store, &steps(10, LimitsScope::User, false)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Config);
        store.set_user_config("journal = 5\n");
        let error = limits_store(&store, &steps(10, LimitsScope::User, false)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Config);
        assert!(store.user_writes().is_empty());
    }

    #[test]
    fn status_reports_entries_space_and_limits() {
        let store = store();
        store.layer(ConfigLevel::User, json!({ "journal": { "max_ops": 2 } }));
        store.commit("/home", [1]);
        let mut partial = entry(2, "/home");
        partial.state = EntryState::Partial;
        partial.bytes_physical = 0;
        store.add(partial, StepKind::Operation);
        let mut undone = entry(3, "/home");
        undone.state = EntryState::Undone;
        store.add(undone, StepKind::Undo);
        for op in [7, 8] {
            tomb::write(
                &store,
                &mount("/home"),
                &tomb::Tombstone::of(&entry(op, "/home"), ExpiryReason::Time),
            )
            .unwrap();
        }
        let states = status_store(&store, None).unwrap();
        assert_eq!(states.len(), 1);
        let state = &states[0];
        assert_eq!(state.volume, mount("/home"));
        assert_eq!(
            (
                state.entries_total,
                state.entries_available,
                state.entries_expired
            ),
            (5, 2, 2)
        );
        assert_eq!((state.bytes_held, state.bytes_physical), (3 * MIB, 2 * MIB));
        assert_eq!(state.oldest.as_ref().map(|oldest| oldest.op), Some(OpId(1)));
        assert_eq!(state.limits.max_ops.source, ConfigLevel::User);
        assert_eq!(state.over_limit, 1);
        assert_eq!(state.inactive, None);
        assert_eq!(state.floor, None);
        assert!(state.notes.is_empty());
        assert_eq!(store.live("/home"), [1, 2, 3]);
    }

    #[test]
    fn status_notes_ignored_project_settings_and_raised_limits() {
        let store = store();
        store.layer(
            ConfigLevel::System,
            json!({ "journal": { "floor": { "max_ops": 20 } } }),
        );
        store.layer(ConfigLevel::User, json!({ "journal": { "max_ops": 5 } }));
        store.layer(
            ConfigLevel::Project,
            json!({ "journal": { "max_ops": 1, "max_age": "10s" } }),
        );
        let state = status_store(&store, Some(&mount("/home/user")))
            .unwrap()
            .remove(0);
        assert_eq!(state.limits.max_ops.value, StepLimit::Limited(20));
        let floor = state.floor.unwrap();
        assert_eq!(floor.source, ConfigLevel::System);
        assert!(floor.raised_to_floor);
        assert_eq!(
            state.notes,
            [
                JournalNotable::JournalConfigIgnored {
                    file: "project.toml".to_owned()
                },
                JournalNotable::RaisedToFloor {
                    setting: "journal.max_ops".to_owned()
                },
            ]
        );
    }

    #[test]
    fn results_carry_no_expiry_time() {
        let store = store();
        store.commit("/home", 1..=3);
        let limits = limits_store(&store, &steps(1, session("s1"), false)).unwrap();
        let purge: PurgeOutcome = purge_store(
            &store,
            &PurgeRequest {
                volume: None,
                selector: PurgeSelector::All,
            },
        )
        .unwrap();
        for value in [
            serde_json::to_value(&limits).unwrap(),
            serde_json::to_value(&purge).unwrap(),
        ] {
            let text = value.to_string();
            for forbidden in ["expires", "deadline", "remaining", "until"] {
                assert!(!text.contains(forbidden), "{forbidden} in {text}");
            }
        }
        let mut keys: Vec<String> = serde_json::to_value(&limits)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(keys, ["applied", "effective", "expiring", "expiring_bytes"]);
    }

    #[test]
    fn session_limit_files_are_named_by_a_digest_of_the_identifier() {
        let mut names = BTreeSet::new();
        for id in ["s1", "", "../../etc/passwd", "sesja żółć", "a/b\\c\0d"] {
            let name = session_file(&SessionId(id.to_owned()));
            assert_eq!(name, session_file(&SessionId(id.to_owned())));
            let file = name.strip_prefix("journal-limits/").unwrap();
            let stem = file.strip_suffix(".json").unwrap();
            assert_eq!(stem.len(), 64, "{id:?}");
            assert!(stem.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
            names.insert(name);
        }
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn the_latest_change_wins_and_equal_times_are_ordered_by_session() {
        let content = |session: &str, steps: u32, changed: u64| {
            format!(
                r#"{{"session":"{session}","changed":{changed},"touched":0,"limits":{{"all":{{"steps":{steps}}}}}}}"#
            )
            .into_bytes()
        };
        let tie = sessions_from_contents(&[content("a", 5, 10), content("b", 6, 10)]).unwrap();
        assert_eq!(tie.all.steps, Some(StepLimit::Limited(6)));
        let reversed = sessions_from_contents(&[content("b", 6, 10), content("a", 5, 10)]).unwrap();
        assert_eq!(reversed.all.steps, Some(StepLimit::Limited(6)));
        let newer = sessions_from_contents(&[content("b", 6, 10), content("a", 5, 11)]).unwrap();
        assert_eq!(newer.all.steps, Some(StepLimit::Limited(5)));
    }

    #[test]
    fn unreadable_or_empty_session_contents_set_no_limits() {
        assert!(sessions_from_contents(&[]).is_none());
        assert!(sessions_from_contents(&[b"{}".to_vec(), b"\xff".to_vec(), Vec::new()]).is_none());
        let empty = br#"{"session":"a","changed":1,"touched":1,"limits":{}}"#.to_vec();
        assert!(sessions_from_contents(&[empty]).is_none());
    }

    #[test]
    fn user_configuration_edits_keep_comments_and_replace_values() {
        let values = LimitValues {
            steps: Some(StepLimit::Limited(10)),
            age: Some(AgeLimit::Limited(DurationSetting::from_secs(2 * HOUR))),
            size: Some(SizeLimit::Unlimited),
        };
        let fresh = edit_user_config(None, None, &values).unwrap();
        assert!(fresh.contains("[journal]"), "{fresh}");
        assert!(fresh.contains("max_ops = 10"), "{fresh}");
        assert!(fresh.contains("max_age = \"2h\""), "{fresh}");
        assert!(fresh.contains("max_size = \"unlimited\""), "{fresh}");

        let current = "# limits\n[journal]\nmax_ops = 5\nother = true\n";
        let edited = edit_user_config(Some(current), None, &values).unwrap();
        assert!(edited.starts_with("# limits\n"), "{edited}");
        assert!(edited.contains("other = true"), "{edited}");
        assert!(edited.contains("max_ops = 10"), "{edited}");
        assert!(!edited.contains("max_ops = 5"), "{edited}");

        let volume = edit_user_config(None, Some("/mnt/zażółć"), &values).unwrap();
        assert!(volume.contains("[journal.volume.\"/mnt/zażółć\"]"), "{volume}");

        let unlimited = LimitValues {
            steps: Some(StepLimit::Unlimited),
            ..LimitValues::default()
        };
        let text = edit_user_config(None, None, &unlimited).unwrap();
        assert!(text.contains("max_ops = \"unlimited\""), "{text}");
    }

    #[test]
    fn user_configuration_edits_refuse_files_that_are_not_tables() {
        let values = LimitValues {
            steps: Some(StepLimit::Limited(10)),
            ..LimitValues::default()
        };
        for current in ["[journal", "journal = 5\n", "[journal]\nvolume = 1\n"] {
            let error = edit_user_config(Some(current), Some("/mnt/data"), &values).unwrap_err();
            assert_eq!(error.code(), ErrorCode::Config, "{current:?}");
        }
    }

    #[test]
    fn limits_are_added_to_the_user_layer_in_their_tables() {
        let values = LimitValues {
            steps: Some(StepLimit::Limited(10)),
            ..LimitValues::default()
        };
        let layers = with_user_values(Vec::new(), None, &values).unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].level, ConfigLevel::User);
        assert_eq!(layers[0].values["journal"]["max_ops"], 10);
        let layers = with_user_values(layers, Some("/mnt/data"), &values).unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(
            layers[0].values["journal"]["volume"]["/mnt/data"]["max_ops"],
            10
        );
        let broken = ConfigLayer::new(
            ConfigLevel::User,
            "user",
            json!({ "journal": 5 }).as_object().cloned().unwrap(),
        );
        let error = with_user_values(vec![broken], None, &values).err().unwrap();
        assert_eq!(error.code(), ErrorCode::Config);
    }

    #[test]
    fn step_limits_are_accepted_from_one_to_one_hundred_thousand() {
        for count in [1, 100_000] {
            let store = store();
            assert!(limits_store(&store, &steps(count, session("s1"), false)).is_ok());
        }
        for count in [0, 100_001] {
            let store = store();
            let error = limits_store(&store, &steps(count, session("s1"), false))
                .err()
                .unwrap();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{count}");
        }
    }

    #[test]
    fn limits_for_an_unknown_volume_are_refused_without_storing_anything() {
        let store = store();
        let mut request = steps(5, session("s1"), false);
        request.volume = Some(mount("/srv"));
        let error = limits_store(&store, &request).err().unwrap();
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert!(store.list_dir(Area::State, SESSION_DIR).unwrap().is_empty());
    }

    fn expiry_config(store: &MemStore) -> Resolved {
        super::super::expiry::resolve_config(store).unwrap()
    }
}
