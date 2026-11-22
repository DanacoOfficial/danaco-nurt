// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reference model of retention by steps and age, against which the journal
//! is compared after random sequences of operations.
//!
//! An entry expires when at least the step limit of later steps exist, or
//! when its effective age reaches the age limit. The effective age is the
//! larger of the wall clock difference and the running and suspended time
//! since the commit, so a wall clock step back never extends it. Expiry is
//! final: raising a limit restores nothing.

use std::time::Duration;

/// Step of a random sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Overwrite of the small file with the given index.
    Write(u8),
    /// Undo of the newest available entry; itself a step.
    UndoLast,
    /// New session step limit.
    Limit(u32),
    /// Running time in minutes.
    Advance(u16),
    /// Suspension in minutes.
    Suspend(u16),
    /// Wall clock step back in minutes.
    StepBack(u16),
    /// Wall clock step forward in minutes.
    StepForward(u16),
    /// Power off for the given minutes and a new journal instance.
    Restart(u16),
}

/// State of one entry in the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Position of the entry in commit order, from zero.
    pub(crate) index: usize,
    /// Wall clock at commit, in seconds.
    pub(crate) wall_at_commit: i64,
    /// Running and suspended time since the commit, in seconds.
    pub(crate) elapsed: i64,
    /// The entry was undone by a later entry.
    pub(crate) undone: bool,
    /// The entry expired.
    pub(crate) expired: bool,
}

/// Model of the journal of one session.
#[derive(Debug, Clone)]
pub(crate) struct Model {
    steps: u32,
    age: i64,
    wall: i64,
    /// Entries in commit order.
    pub(crate) entries: Vec<Entry>,
}

const SECONDS_PER_MINUTE: i64 = 60;

impl Model {
    /// Model with the given step limit and age limit.
    pub(crate) fn new(steps: u32, age: Duration) -> Self {
        Self {
            steps,
            age: i64::try_from(age.as_secs()).unwrap_or(i64::MAX),
            wall: 0,
            entries: Vec::new(),
        }
    }

    fn commit(&mut self) {
        let index = self.entries.len();
        self.entries.push(Entry {
            index,
            wall_at_commit: self.wall,
            elapsed: 0,
            undone: false,
            expired: false,
        });
    }

    fn pass(&mut self, minutes: u16, wall: bool) {
        let seconds = i64::from(minutes) * SECONDS_PER_MINUTE;
        if wall {
            self.wall += seconds;
        }
        for entry in &mut self.entries {
            entry.elapsed += seconds;
        }
    }

    fn expire(&mut self) {
        let total = self.entries.len();
        let steps = usize::try_from(self.steps).unwrap_or(usize::MAX);
        let (wall, age) = (self.wall, self.age);
        for entry in &mut self.entries {
            let later = total - entry.index - 1;
            let effective = entry.elapsed.max(wall - entry.wall_at_commit);
            if later >= steps || effective >= age {
                entry.expired = true;
            }
        }
    }

    /// Index of the newest entry that is neither undone nor expired, and is
    /// not itself an undo.
    pub(crate) fn newest_available(&self, undo_entries: &[usize]) -> Option<usize> {
        self.entries
            .iter()
            .rev()
            .find(|entry| !entry.undone && !entry.expired && !undo_entries.contains(&entry.index))
            .map(|entry| entry.index)
    }

    /// Applies one action; `undo_entries` lists the entries that are undo
    /// entries and is extended by [`Action::UndoLast`]. Returns the undone
    /// entry, if any.
    pub(crate) fn apply(&mut self, action: Action, undo_entries: &mut Vec<usize>) -> Option<usize> {
        let mut undone = None;
        match action {
            Action::Write(_) => self.commit(),
            Action::UndoLast => {
                if let Some(index) = self.newest_available(undo_entries) {
                    self.entries[index].undone = true;
                    undone = Some(index);
                    undo_entries.push(self.entries.len());
                    self.commit();
                }
            }
            Action::Limit(steps) => self.steps = steps,
            Action::Advance(minutes) | Action::Suspend(minutes) | Action::Restart(minutes) => {
                self.pass(minutes, true);
            }
            Action::StepBack(minutes) => self.wall -= i64::from(minutes) * SECONDS_PER_MINUTE,
            Action::StepForward(minutes) => self.wall += i64::from(minutes) * SECONDS_PER_MINUTE,
        }
        self.expire();
        undone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_expires_when_the_step_limit_of_later_steps_exists() {
        let mut model = Model::new(3, Duration::from_secs(1800));
        let mut undo = Vec::new();
        for index in 0..4 {
            model.apply(Action::Write(index), &mut undo);
        }
        assert!(model.entries[0].expired);
        assert!(!model.entries[1].expired);
    }

    #[test]
    fn wall_clock_step_back_does_not_extend_age() {
        let mut model = Model::new(30, Duration::from_secs(1800));
        let mut undo = Vec::new();
        model.apply(Action::Write(0), &mut undo);
        model.apply(Action::StepBack(60), &mut undo);
        model.apply(Action::Advance(31), &mut undo);
        assert!(model.entries[0].expired);
    }

    #[test]
    fn raising_a_limit_restores_nothing() {
        let mut model = Model::new(30, Duration::from_secs(1800));
        let mut undo = Vec::new();
        for index in 0..5 {
            model.apply(Action::Write(index), &mut undo);
        }
        model.apply(Action::Limit(1), &mut undo);
        model.apply(Action::Limit(30), &mut undo);
        assert_eq!(
            model.entries.iter().filter(|entry| entry.expired).count(),
            4
        );
    }
}
