// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::jobs::state
// Responsibility:   Job states and their transitions.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Job states and their transitions.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{JobState, KilledBy};
use nurt_contracts::stage::JobEnd;

use crate::iface::ProcessEnd;

/// Whether a job in `state` still has a supervised process.
pub(crate) fn is_live(state: JobState) -> bool {
    matches!(state, JobState::Running | JobState::Suspended)
}

/// Final state of a job whose process ended with `end`.
pub(crate) fn final_state(end: &ProcessEnd) -> JobState {
    match end.killed_by {
        Some(KilledBy::Stop | KilledBy::Cancel) => JobState::Stopped,
        Some(KilledBy::Timeout) => JobState::TimedOut,
        Some(KilledBy::Idle) => JobState::Stalled,
        Some(KilledBy::Limit) => JobState::Limit,
        Some(_) => JobState::Failed,
        None if end.exit_code == Some(0) => JobState::Exited,
        None => JobState::Failed,
    }
}

/// State reported in the `job.finished` event, which distinguishes four
/// final states: a stalled job ended by its time limits, a job ended by a
/// resource limit or without a supervisor failed.
pub(crate) fn job_end(state: JobState) -> JobEnd {
    match state {
        JobState::Exited => JobEnd::Exited,
        JobState::Stopped => JobEnd::Stopped,
        JobState::TimedOut | JobState::Stalled => JobEnd::TimedOut,
        _ => JobEnd::Failed,
    }
}

/// Action requested on a job, as named in messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// `stop`.
    Stop,
    /// `suspend`.
    Suspend,
    /// `resume`.
    Resume,
    /// `remove`.
    Remove,
    /// `retry`.
    Retry,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Suspend => "suspend",
            Self::Resume => "resume",
            Self::Remove => "remove",
            Self::Retry => "retry",
        }
    }
}

/// Checks that `action` is allowed on a job in `state`.
///
/// # Errors
///
/// Fails with `E_JOB_STATE` and a ready program that reaches an allowed
/// state.
pub(crate) fn check_action(id: &str, state: JobState, action: Action) -> Result<(), Error> {
    let allowed = match action {
        Action::Stop => is_live(state),
        Action::Suspend => state == JobState::Running,
        Action::Resume => state == JobState::Suspended,
        Action::Remove | Action::Retry => !is_live(state),
    };
    if allowed {
        return Ok(());
    }
    let state_name = state.as_str();
    let error = Error::new(
        ErrorCode::JobState,
        format!(
            "job `{id}` is {state_name}; `{}` needs {}",
            action.name(),
            needs(action)
        ),
    )
    .with_found(state_name);
    let fix = match action {
        Action::Remove | Action::Retry => Some(format!("jobs \"{id}\" |> stop")),
        Action::Resume if state == JobState::Running => Some(format!("jobs \"{id}\" |> suspend")),
        Action::Suspend if state == JobState::Suspended => Some(format!("jobs \"{id}\" |> resume")),
        _ => None,
    };
    Err(match fix {
        Some(program) => error.with_fix(program),
        None => error,
    })
}

fn needs(action: Action) -> &'static str {
    match action {
        Action::Stop => "a running or suspended job",
        Action::Suspend => "a running job",
        Action::Resume => "a suspended job",
        Action::Remove | Action::Retry => "a finished job",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::env::WallTime;

    use super::*;
    use crate::iface::ResourceUsage;

    fn end(exit_code: Option<i32>, killed_by: Option<KilledBy>) -> ProcessEnd {
        ProcessEnd {
            exit_code,
            signal: None,
            signal_number: None,
            killed_by,
            limit_hit: None,
            finished: WallTime::UNIX_EPOCH,
            usage: ResourceUsage::default(),
            suspended_total: Duration::ZERO,
            stdin_closed_early: false,
        }
    }

    #[test]
    fn maps_process_ends_to_final_states() {
        assert_eq!(final_state(&end(Some(0), None)), JobState::Exited);
        assert_eq!(final_state(&end(Some(3), None)), JobState::Failed);
        assert_eq!(final_state(&end(None, None)), JobState::Failed);
        assert_eq!(
            final_state(&end(None, Some(KilledBy::Stop))),
            JobState::Stopped
        );
        assert_eq!(
            final_state(&end(None, Some(KilledBy::Cancel))),
            JobState::Stopped
        );
        assert_eq!(
            final_state(&end(None, Some(KilledBy::Timeout))),
            JobState::TimedOut
        );
        assert_eq!(
            final_state(&end(None, Some(KilledBy::Idle))),
            JobState::Stalled
        );
        assert_eq!(
            final_state(&end(None, Some(KilledBy::Limit))),
            JobState::Limit
        );
    }

    #[test]
    fn reports_four_final_states_in_events() {
        assert_eq!(job_end(JobState::Exited), JobEnd::Exited);
        assert_eq!(job_end(JobState::Failed), JobEnd::Failed);
        assert_eq!(job_end(JobState::Stopped), JobEnd::Stopped);
        assert_eq!(job_end(JobState::TimedOut), JobEnd::TimedOut);
        assert_eq!(job_end(JobState::Stalled), JobEnd::TimedOut);
        assert_eq!(job_end(JobState::Limit), JobEnd::Failed);
        assert_eq!(job_end(JobState::Lost), JobEnd::Failed);
    }

    #[test]
    fn allows_actions_only_in_matching_states() {
        assert!(check_action("j1", JobState::Running, Action::Stop).is_ok());
        assert!(check_action("j1", JobState::Suspended, Action::Stop).is_ok());
        assert!(check_action("j1", JobState::Running, Action::Suspend).is_ok());
        assert!(check_action("j1", JobState::Suspended, Action::Resume).is_ok());
        assert!(check_action("j1", JobState::Exited, Action::Remove).is_ok());
        assert!(check_action("j1", JobState::Lost, Action::Retry).is_ok());
    }

    #[test]
    fn rejects_actions_with_a_ready_fix() {
        let error = check_action("j4", JobState::Running, Action::Remove).unwrap_err();
        assert_eq!(error.code, ErrorCode::JobState);
        assert_eq!(error.fix, ["jobs \"j4\" |> stop"]);
        let error = check_action("j4", JobState::Running, Action::Resume).unwrap_err();
        assert_eq!(error.fix, ["jobs \"j4\" |> suspend"]);
        let error = check_action("j4", JobState::Suspended, Action::Suspend).unwrap_err();
        assert_eq!(error.fix, ["jobs \"j4\" |> resume"]);
        let error = check_action("j4", JobState::Exited, Action::Stop).unwrap_err();
        assert!(error.fix.is_empty());
        assert_eq!(error.found.as_deref(), Some("exited"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
