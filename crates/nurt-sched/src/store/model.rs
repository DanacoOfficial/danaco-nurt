// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sched::store::model
// Responsibility:   Content of the schedule store, format `nurt-schedules/1`.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Content of the schedule store, format `nurt-schedules/1`.

use std::collections::BTreeMap;

use nurt_contracts::bound::BoundPipeline;
use nurt_contracts::value::{Duration, Instant};
use serde::{Deserialize, Serialize};

use crate::trigger::TriggerDef;

/// Format name written into every file of the store.
pub const FORMAT: &str = "nurt-schedules/1";

macro_rules! names {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $text)] $variant,)+
        }

        impl $name {
            /// Stable name of the value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            /// Value of a stable name.
            #[must_use]
            pub fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

names! {
    /// Handling of a run time missed by more than the misfire tolerance.
    OnMissed {
        /// The run is skipped; the next run time counts from the present.
        Skip = "skip",
        /// Exactly one run is made, however many run times were missed.
        RunOnce = "run_once",
    }
}

names! {
    /// Handling of a run time reached while a run of the schedule executes.
    Overlap {
        /// The run time is skipped.
        Skip = "skip",
        /// One run waits and starts when the running one ends.
        Queue = "queue",
        /// The run starts as well, within `schedule.max_parallel`.
        Parallel = "parallel",
    }
}

names! {
    /// State of a schedule kept by the service.
    Status {
        /// The schedule runs at its run times.
        Active = "active",
        /// All runs are done.
        Completed = "completed",
        /// The schedule passed its expiry time.
        Expired = "expired",
    }
}

names! {
    /// What started a run.
    RunTrigger {
        /// A run time of the trigger.
        Schedule = "schedule",
        /// `schedule.run`.
        Manual = "manual",
        /// `run_now` of `schedule.create`.
        RunNow = "run_now",
        /// A run time that waited for the previous run (`overlap: queue`).
        Queued = "queued",
        /// A missed run time made up once (`on_missed: run_once`).
        Missed = "missed",
    }
}

names! {
    /// Outcome of a run.
    RunState {
        /// The action succeeded.
        Ok = "ok",
        /// The program of the action ended with a non-zero exit code.
        Exit = "exit",
        /// The run exceeded its time limit.
        Timeout = "timeout",
        /// The run was stopped.
        Stopped = "stopped",
        /// The run time was skipped; `reason` says why.
        Skipped = "skipped",
        /// The run was refused; `reason` says why.
        Rejected = "rejected",
        /// The action failed before or while it ran.
        Failed = "failed",
    }
}

names! {
    /// Why a run time was skipped or refused.
    SkipReason {
        /// Later than the misfire tolerance.
        Misfire = "misfire",
        /// A run of the schedule was executing.
        Overlap = "overlap",
        /// No free place among `schedule.max_parallel` runs within the misfire
        /// tolerance.
        Capacity = "capacity",
        /// No connected client provides the action kind.
        ActionUnavailable = "action_unavailable",
        /// The definition no longer matches its integrity hash.
        DefinitionChanged = "definition_changed",
    }
}

/// Definition of one schedule, file `definitions/<id>.json`.
///
/// Definitions are changed only by the schedule commands, through the
/// operation journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    /// Format name, [`FORMAT`].
    pub format: String,
    /// Identifier, for example `s3`.
    pub id: String,
    /// Label.
    pub label: String,
    /// Trigger.
    pub trigger: TriggerDef,
    /// Kind of action: `pipeline` or a kind registered by a `ScheduleAction`.
    pub action_kind: String,
    /// Bound pipeline of an action of kind `pipeline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<BoundPipeline>,
    /// Arguments of a registered action kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_args: Option<serde_json::Value>,
    /// Text of the action shown to the user.
    pub action: String,
    /// Integrity hash of the action, `sha256:<hex>`.
    pub ast_hash: String,
    /// Handling of missed run times.
    pub on_missed: OnMissed,
    /// Tolerance for a late start.
    pub misfire: Duration,
    /// Handling of overlapping runs.
    pub overlap: Overlap,
    /// Time limit of a run.
    pub timeout: Duration,
    /// Number of scheduled runs after which the schedule completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u64>,
    /// Expiry time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Instant>,
    /// The schedule is paused.
    #[serde(default)]
    pub paused: bool,
    /// Creation time.
    pub created: Instant,
    /// Time of the last change by a command; a new value makes the service
    /// recompute the next run time.
    pub changed: Instant,
    /// Session that created the schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Base directory of the creating call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Project directories of the creating call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project_roots: Vec<String>,
}

/// Run state of one schedule, kept by the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// State.
    pub status: Status,
    /// Next run time; `None` when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<Instant>,
    /// Runs started by run times, counted against `max_runs`.
    pub run_count: u64,
    /// Number of the last run record of the schedule.
    pub runs: u64,
    /// Run times skipped.
    pub skipped: u64,
    /// Start of the last run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<Instant>,
    /// Outcome of the last run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_state: Option<RunState>,
    /// Exit code of the last run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_exit_code: Option<i64>,
    /// One run waits for the running one (`overlap: queue`).
    #[serde(default)]
    pub queued: bool,
    /// Last time a due run found no free place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refused: Option<Instant>,
    /// Integrity hash of the action when the service first saw the schedule.
    pub ast_hash: String,
    /// `changed` of the definition last applied.
    pub changed: Instant,
    /// The definition was paused when last applied.
    #[serde(default)]
    pub paused: bool,
    /// Digest of the outcome of the last scheduled run, for `changed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_digest: Option<String>,
}

/// One run or skipped run time of a schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Schedule identifier.
    pub schedule: String,
    /// Number of the record within the schedule, from 1.
    pub run: u64,
    /// What started the run.
    pub trigger: RunTrigger,
    /// Run time of the trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<Instant>,
    /// Start time.
    pub started: Instant,
    /// Running time.
    pub elapsed: Duration,
    /// Outcome.
    pub state: RunState,
    /// Exit code of the program of the action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    /// Why the run was skipped or refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<SkipReason>,
    /// Journal operation of the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<u64>,
    /// Job of the program of the action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// File holding the full output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file: Option<String>,
    /// SHA-256 of the output, lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_sha256: Option<String>,
    /// The outcome differs from the previous scheduled run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<bool>,
    /// Short description of the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl RunRecord {
    /// Identifier of the run, `<schedule>#<run>`.
    #[must_use]
    pub fn id(&self) -> String {
        run_id(&self.schedule, self.run)
    }
}

/// Identifier of a run, `<schedule>#<run>`.
#[must_use]
pub fn run_id(schedule: &str, run: u64) -> String {
    format!("{schedule}#{run}")
}

/// Run state of all schedules, file `state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFile {
    /// Format name, [`FORMAT`].
    pub format: String,
    /// Run state by schedule identifier.
    #[serde(default)]
    pub schedules: BTreeMap<String, Entry>,
    /// Kept runs by schedule identifier, oldest first.
    #[serde(default)]
    pub history: BTreeMap<String, Vec<RunRecord>>,
}

impl Default for StateFile {
    fn default() -> Self {
        Self {
            format: FORMAT.to_owned(),
            schedules: BTreeMap::new(),
            history: BTreeMap::new(),
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
