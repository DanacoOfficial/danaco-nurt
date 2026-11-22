// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Run decisions of schedules.
//!
//! [`Scheduler`] holds the definitions and the run state of all schedules
//! and decides, for a given instant, which runs start and which run times are
//! skipped. It does not read clocks, files or run actions: the schedule
//! module calls [`Scheduler::tick`] with the current time, executes the
//! returned [`Launch`]es and reports their end with [`Scheduler::finish`].
//!
//! Rules:
//!
//! * A run time is made when the present is at most `misfire` after it.
//!   Later, `on_missed: skip` records a skipped run (`reason: misfire`) and
//!   `on_missed: run_once` makes exactly one run (`trigger: missed`); in both
//!   cases the next run time counts from the present, so a long pause adds one
//!   skip, not one per missed run time. A single time (`at`) skipped this way
//!   makes the schedule expire.
//! * A run time reached while a run of the schedule executes is skipped
//!   (`overlap: skip`), waits in a queue of one place (`overlap: queue`) or
//!   runs as well (`overlap: parallel`).
//! * At most `schedule.max_parallel` runs execute at the same time; a due run
//!   without a free place waits within the misfire tolerance and is then
//!   skipped with `reason: capacity`.
//! * `max_runs` counts runs started by run times; manual runs and `run_now`
//!   do not count and do not move the next run time.
//! * A definition whose action no longer matches its integrity hash, or whose
//!   hash differs from the one seen when the schedule was created, is refused
//!   (`rejected`, `reason: definition_changed`).
//! * Resuming a schedule, and the first sight of a schedule restored by an
//!   undo, compute the next run time from the present.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nurt_contracts::error::Error;
use nurt_contracts::value::Instant;

use crate::action::{Outcome, definition_hash};
use crate::errors::{busy, failed, not_found};
use crate::settings::ScheduleSettings;
use crate::store::{
    Definition, Entry, OnMissed, Overlap, RunRecord, RunState, RunTrigger, SkipReason, StateFile,
    Status, run_id,
};
use crate::trigger::Trigger;
use crate::trigger::time::{self, std_duration, value_duration};

/// A run to execute.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Schedule identifier.
    pub schedule: String,
    /// Run number.
    pub run: u64,
    /// What started the run.
    pub trigger: RunTrigger,
    /// Run time of the trigger.
    pub scheduled_for: Option<Instant>,
    /// Start time.
    pub started: Instant,
    /// Definition of the schedule at the start.
    pub definition: Arc<Definition>,
}

impl Launch {
    /// Identifier of the run, `<schedule>#<run>`.
    #[must_use]
    pub fn id(&self) -> String {
        run_id(&self.schedule, self.run)
    }
}

/// Something that happened in the scheduler, published as an event.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// A run started (`schedule.fired`).
    Fired(Launch),
    /// A run ended or was refused (`schedule.finished`).
    Finished(RunRecord),
    /// A run time was skipped (`schedule.skipped`).
    Skipped(RunRecord),
}

#[derive(Debug, Clone)]
struct Slot {
    definition: Arc<Definition>,
    trigger: Option<Trigger>,
    hash_ok: bool,
}

/// Definitions, run state and running runs of all schedules.
#[derive(Debug)]
pub struct Scheduler {
    settings: ScheduleSettings,
    state: StateFile,
    slots: BTreeMap<String, Slot>,
    running: BTreeMap<String, Launch>,
    events: Vec<EngineEvent>,
    dirty: bool,
}

impl Scheduler {
    /// A scheduler starting from a saved run state.
    #[must_use]
    pub fn new(settings: ScheduleSettings, state: StateFile) -> Self {
        Self {
            settings,
            state,
            slots: BTreeMap::new(),
            running: BTreeMap::new(),
            events: Vec::new(),
            dirty: false,
        }
    }

    /// Replaces the limits and defaults.
    pub fn set_settings(&mut self, settings: ScheduleSettings) {
        self.settings = settings;
    }

    /// Applies the current definitions at `now`.
    ///
    /// The run state of removed schedules is dropped; their history stays.
    /// New schedules get their first run time; changed ones are paused or
    /// resumed. Returns the identifiers whose trigger could not be compiled.
    pub fn load(&mut self, definitions: Vec<Definition>, now: Instant) -> Vec<String> {
        let ids: BTreeSet<String> = definitions.iter().map(|d| d.id.clone()).collect();
        let before = self.state.schedules.len();
        self.state.schedules.retain(|id, _| ids.contains(id));
        self.slots.retain(|id, _| ids.contains(id));
        self.dirty |= before != self.state.schedules.len();
        let mut broken = Vec::new();
        for definition in definitions {
            let trigger = Trigger::compile(&definition.trigger).ok();
            if trigger.is_none() {
                broken.push(definition.id.clone());
            }
            let hash_ok =
                definition_hash(&definition).is_ok_and(|hash| hash == definition.ast_hash);
            self.apply_definition(&definition, trigger.as_ref(), now);
            self.slots.insert(
                definition.id.clone(),
                Slot {
                    definition: Arc::new(definition),
                    trigger,
                    hash_ok,
                },
            );
        }
        broken
    }

    fn apply_definition(
        &mut self,
        definition: &Definition,
        trigger: Option<&Trigger>,
        now: Instant,
    ) {
        match self.state.schedules.get_mut(&definition.id) {
            None => {
                let next_run = trigger.and_then(|t| t.first_run(definition.created, now));
                let runs = self
                    .state
                    .history
                    .get(&definition.id)
                    .and_then(|history| history.last())
                    .map_or(0, |record| record.run);
                self.state.schedules.insert(
                    definition.id.clone(),
                    Entry {
                        status: Status::Active,
                        next_run,
                        run_count: 0,
                        runs,
                        skipped: 0,
                        last_run: None,
                        last_state: None,
                        last_exit_code: None,
                        queued: false,
                        last_refused: None,
                        ast_hash: definition.ast_hash.clone(),
                        changed: definition.changed,
                        paused: definition.paused,
                        last_digest: None,
                    },
                );
                self.dirty = true;
            }
            Some(entry) if entry.changed != definition.changed => {
                if definition.paused {
                    entry.queued = false;
                } else if entry.paused {
                    entry.next_run = trigger.and_then(|t| resume_from(t, now));
                    entry.queued = false;
                    entry.last_refused = None;
                }
                entry.paused = definition.paused;
                entry.changed = definition.changed;
                self.dirty = true;
            }
            Some(_) => {}
        }
    }

    /// Decides about all schedules at `now` and returns the runs to start.
    pub fn tick(&mut self, now: Instant) -> Vec<Launch> {
        let ids: Vec<String> = self.slots.keys().cloned().collect();
        ids.iter().filter_map(|id| self.tick_one(id, now)).collect()
    }

    fn tick_one(&mut self, id: &str, now: Instant) -> Option<Launch> {
        let slot = self.slots.get(id)?.clone();
        let definition = &slot.definition;
        let entry = self.state.schedules.get_mut(id)?;
        if definition.paused || entry.status != Status::Active {
            return None;
        }
        if definition.expires.is_some_and(|expires| expires <= now) {
            entry.status = Status::Expired;
            entry.next_run = None;
            self.dirty = true;
            return None;
        }
        let trigger = slot.trigger.as_ref()?;
        let due = entry.next_run?;
        if due > now {
            return None;
        }
        if time::since(now, due) > std_duration(definition.misfire) {
            if entry.last_refused.take().is_some() {
                self.skip(id, Some(due), SkipReason::Capacity, now);
                self.advance(id, trigger, due, now, Status::Expired);
                return None;
            }
            if definition.on_missed == OnMissed::Skip {
                self.skip(id, Some(due), SkipReason::Misfire, now);
                self.advance(id, trigger, due, now, Status::Expired);
                return None;
            }
            return self.fire(&slot, trigger, due, RunTrigger::Missed, now);
        }
        self.fire(&slot, trigger, due, RunTrigger::Schedule, now)
    }

    fn fire(
        &mut self,
        slot: &Slot,
        trigger: &Trigger,
        due: Instant,
        kind: RunTrigger,
        now: Instant,
    ) -> Option<Launch> {
        let definition = &slot.definition;
        let id = definition.id.as_str();
        let recorded = self
            .state
            .schedules
            .get(id)
            .map(|entry| entry.ast_hash.clone());
        if !slot.hash_ok || recorded.as_deref() != Some(definition.ast_hash.as_str()) {
            self.reject(id, Some(due), kind, now);
            self.advance(id, trigger, due, now, Status::Expired);
            return None;
        }
        if self.running_of(id).is_some() {
            match definition.overlap {
                Overlap::Skip => {
                    self.skip(id, Some(due), SkipReason::Overlap, now);
                    self.advance(id, trigger, due, now, Status::Completed);
                    return None;
                }
                Overlap::Queue => {
                    let entry = self.state.schedules.get_mut(id)?;
                    if entry.queued {
                        self.skip(id, Some(due), SkipReason::Overlap, now);
                    } else {
                        entry.queued = true;
                        self.dirty = true;
                    }
                    self.advance(id, trigger, due, now, Status::Completed);
                    return None;
                }
                Overlap::Parallel => {}
            }
        }
        if self.at_capacity() {
            let entry = self.state.schedules.get_mut(id)?;
            entry.last_refused = Some(now);
            self.dirty = true;
            return None;
        }
        if let Some(entry) = self.state.schedules.get_mut(id) {
            entry.last_refused = None;
        }
        let launch = self.launch(definition, kind, Some(due), now, true);
        self.advance(id, trigger, due, now, Status::Completed);
        self.complete_if_done(id);
        Some(launch)
    }

    /// Starts a run outside the run times (`schedule.run`, `run_now`): the
    /// next run time and `max_runs` are not affected.
    ///
    /// # Errors
    ///
    /// Returns `E_NOT_FOUND` for an unknown schedule, `E_JOB_STATE` while a
    /// run of the schedule executes and `E_STAGE_FAILED` when the definition
    /// does not match its integrity hash.
    pub fn run_now(&mut self, id: &str, kind: RunTrigger, now: Instant) -> Result<Launch, Error> {
        let slot = self
            .slots
            .get(id)
            .cloned()
            .ok_or_else(|| not_found(&[id.to_owned()]))?;
        if let Some(running) = self.running_of(id) {
            return Err(busy(id, &running));
        }
        let recorded = self
            .state
            .schedules
            .get(id)
            .map(|entry| entry.ast_hash.clone());
        if !slot.hash_ok || recorded.as_deref() != Some(slot.definition.ast_hash.as_str()) {
            self.reject(id, None, kind, now);
            return Err(failed(format!(
                "the definition of schedule `{id}` was changed outside the schedule commands; \
                 remove it and create it again"
            )));
        }
        Ok(self.launch(&slot.definition, kind, None, now, false))
    }

    /// Records the end of the run `run` at `now`; returns the queued run that
    /// starts in its place.
    pub fn finish(&mut self, run: &str, outcome: &Outcome, now: Instant) -> Option<Launch> {
        let launch = self.running.remove(run)?;
        let id = launch.schedule.clone();
        let scheduled = matches!(
            launch.trigger,
            RunTrigger::Schedule | RunTrigger::Queued | RunTrigger::Missed
        );
        let digest = outcome.digest();
        let mut changed = None;
        if let Some(entry) = self.state.schedules.get_mut(&id) {
            if scheduled {
                changed = Some(entry.last_digest.as_deref() != Some(digest.as_str()));
                entry.last_digest = Some(digest);
            }
            entry.last_state = Some(outcome.state);
            entry.last_exit_code = outcome.exit_code;
        }
        let record = RunRecord {
            schedule: id.clone(),
            run: launch.run,
            trigger: launch.trigger,
            scheduled_for: launch.scheduled_for,
            started: launch.started,
            elapsed: value_duration(time::since(now, launch.started)),
            state: outcome.state,
            exit_code: outcome.exit_code,
            reason: (outcome.state == RunState::Skipped).then_some(SkipReason::ActionUnavailable),
            operation: outcome.operation,
            job: outcome.job.clone(),
            output_file: outcome.output_file.clone(),
            output_sha256: outcome.output_sha256.clone(),
            changed,
            message: outcome.message.clone(),
        };
        self.push_history(record.clone());
        self.events.push(EngineEvent::Finished(record));
        self.dirty = true;
        self.start_queued(&id, now)
    }

    fn start_queued(&mut self, id: &str, now: Instant) -> Option<Launch> {
        let slot = self.slots.get(id)?.clone();
        let entry = self.state.schedules.get_mut(id)?;
        if !entry.queued || slot.definition.paused || self.running_of(id).is_some() {
            return None;
        }
        let entry = self.state.schedules.get_mut(id)?;
        entry.queued = false;
        if entry.status != Status::Active {
            return None;
        }
        let launch = self.launch(&slot.definition, RunTrigger::Queued, None, now, true);
        self.complete_if_done(id);
        Some(launch)
    }

    /// Clears the queue of the schedule `id` and returns its running runs,
    /// which the caller cancels.
    pub fn stop(&mut self, id: &str) -> Vec<String> {
        if let Some(entry) = self.state.schedules.get_mut(id)
            && entry.queued
        {
            entry.queued = false;
            self.dirty = true;
        }
        self.running
            .values()
            .filter(|launch| launch.schedule == id)
            .map(Launch::id)
            .collect()
    }

    /// Earliest next run time of an active schedule.
    #[must_use]
    pub fn next_wake(&self) -> Option<Instant> {
        self.slots
            .iter()
            .filter(|(_, slot)| !slot.definition.paused && slot.trigger.is_some())
            .filter_map(|(id, _)| self.state.schedules.get(id))
            .filter(|entry| entry.status == Status::Active)
            .filter_map(|entry| entry.next_run)
            .min()
    }

    /// Run state.
    #[must_use]
    pub fn state(&self) -> &StateFile {
        &self.state
    }

    /// Run state of the schedule `id`.
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.state.schedules.get(id)
    }

    /// Definition of the schedule `id`.
    #[must_use]
    pub fn definition(&self, id: &str) -> Option<Arc<Definition>> {
        self.slots.get(id).map(|slot| Arc::clone(&slot.definition))
    }

    /// Kept runs of the schedule `id`, oldest first.
    #[must_use]
    pub fn history(&self, id: &str) -> &[RunRecord] {
        self.state.history.get(id).map_or(&[], Vec::as_slice)
    }

    /// Identifiers of the running runs.
    #[must_use]
    pub fn running(&self) -> Vec<String> {
        self.running.keys().cloned().collect()
    }

    /// Number of known schedules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Returns `true` when no schedule is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Returns `true` once since the run state changed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<EngineEvent> {
        std::mem::take(&mut self.events)
    }

    fn running_of(&self, id: &str) -> Option<String> {
        self.running
            .values()
            .find(|launch| launch.schedule == id)
            .map(Launch::id)
    }

    fn at_capacity(&self) -> bool {
        u32::try_from(self.running.len()).unwrap_or(u32::MAX) >= self.settings.max_parallel
    }

    fn launch(
        &mut self,
        definition: &Arc<Definition>,
        kind: RunTrigger,
        scheduled_for: Option<Instant>,
        now: Instant,
        counts: bool,
    ) -> Launch {
        let run = match self.state.schedules.get_mut(&definition.id) {
            Some(entry) => {
                entry.runs += 1;
                if counts {
                    entry.run_count += 1;
                }
                entry.last_run = Some(now);
                entry.runs
            }
            None => 1,
        };
        let launch = Launch {
            schedule: definition.id.clone(),
            run,
            trigger: kind,
            scheduled_for,
            started: now,
            definition: Arc::clone(definition),
        };
        self.running.insert(launch.id(), launch.clone());
        self.events.push(EngineEvent::Fired(launch.clone()));
        self.dirty = true;
        launch
    }

    fn advance(&mut self, id: &str, trigger: &Trigger, due: Instant, now: Instant, single: Status) {
        if let Some(entry) = self.state.schedules.get_mut(id) {
            entry.next_run = trigger.following(due, now);
            if entry.next_run.is_none() && entry.status == Status::Active {
                entry.status = if trigger.is_single() {
                    single
                } else {
                    Status::Expired
                };
            }
            self.dirty = true;
        }
    }

    fn complete_if_done(&mut self, id: &str) {
        let max_runs = self.slots.get(id).and_then(|slot| slot.definition.max_runs);
        if let (Some(max_runs), Some(entry)) = (max_runs, self.state.schedules.get_mut(id))
            && entry.run_count >= max_runs
        {
            entry.status = Status::Completed;
            entry.next_run = None;
            self.dirty = true;
        }
    }

    fn skip(&mut self, id: &str, scheduled_for: Option<Instant>, reason: SkipReason, now: Instant) {
        let Some(entry) = self.state.schedules.get_mut(id) else {
            return;
        };
        entry.skipped += 1;
        entry.runs += 1;
        let record = idle_record(
            id,
            entry.runs,
            RunTrigger::Schedule,
            scheduled_for,
            now,
            RunState::Skipped,
            reason,
        );
        self.push_history(record.clone());
        self.events.push(EngineEvent::Skipped(record));
        self.dirty = true;
    }

    fn reject(&mut self, id: &str, scheduled_for: Option<Instant>, kind: RunTrigger, now: Instant) {
        let Some(entry) = self.state.schedules.get_mut(id) else {
            return;
        };
        entry.runs += 1;
        entry.last_state = Some(RunState::Rejected);
        let record = idle_record(
            id,
            entry.runs,
            kind,
            scheduled_for,
            now,
            RunState::Rejected,
            SkipReason::DefinitionChanged,
        );
        self.push_history(record.clone());
        self.events.push(EngineEvent::Finished(record));
        self.dirty = true;
    }

    fn push_history(&mut self, record: RunRecord) {
        let keep = usize::try_from(self.settings.history_keep).unwrap_or(usize::MAX);
        let history = self
            .state
            .history
            .entry(record.schedule.clone())
            .or_default();
        history.push(record);
        if history.len() > keep {
            let excess = history.len() - keep;
            history.drain(..excess);
        }
    }
}

fn resume_from(trigger: &Trigger, now: Instant) -> Option<Instant> {
    match trigger {
        Trigger::At { at } => Some(*at),
        other => other.following(now, now),
    }
}

fn idle_record(
    id: &str,
    run: u64,
    trigger: RunTrigger,
    scheduled_for: Option<Instant>,
    now: Instant,
    state: RunState,
    reason: SkipReason,
) -> RunRecord {
    RunRecord {
        schedule: id.to_owned(),
        run,
        trigger,
        scheduled_for,
        started: now,
        elapsed: nurt_contracts::value::Duration::ZERO,
        state,
        exit_code: None,
        reason: Some(reason),
        operation: None,
        job: None,
        output_file: None,
        output_sha256: None,
        changed: None,
        message: Some(format!("{}: {}", state.as_str(), reason.as_str())),
    }
}
