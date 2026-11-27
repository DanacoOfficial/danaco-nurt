// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::os::descendants
// Responsibility:   Descendants of a process that left its process group.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Descendants of a process that left its process group.
//!
//! A program that calls `setsid` or `setpgid` is no longer a member of the
//! group that `killpg` reaches, and a descendant whose parent ended is adopted
//! by the init process, so neither the group nor the parent chain names it
//! later. The processes of a tree are therefore remembered with their start
//! times while the tree runs, and a remembered process is signalled only when
//! its start time is unchanged, which excludes a process that received the
//! identifier after the first one ended.

use std::collections::{HashMap, HashSet};

/// Most processes visited in one walk of the tree; a limit against a process
/// table that changes faster than it is read.
pub(super) const WALK_LIMIT: usize = 8192;

/// `root` and every process below it, found with `children`; the order is
/// breadth first and every process appears once.
pub(super) fn tree_of<C>(root: i32, mut children: C) -> Vec<i32>
where
    C: FnMut(i32) -> Vec<i32>,
{
    let mut seen: HashSet<i32> = HashSet::from([root]);
    let mut order = vec![root];
    let mut next = 0;
    while let Some(&pid) = order.get(next) {
        next += 1;
        for child in children(pid) {
            if order.len() >= WALK_LIMIT {
                return order;
            }
            if child > 0 && seen.insert(child) {
                order.push(child);
            }
        }
    }
    order
}

/// Processes remembered for a tree, with their start times.
#[derive(Debug, Default)]
pub(super) struct Remembered {
    started: HashMap<i32, u64>,
}

impl Remembered {
    /// Remembers `pids`, each with the start time `start_of` reports; a
    /// process without a start time ended and is not remembered. Returns the
    /// number of processes remembered for the first time.
    pub(super) fn add<S>(&mut self, pids: &[i32], mut start_of: S) -> usize
    where
        S: FnMut(i32) -> Option<u64>,
    {
        let mut added = 0;
        for &pid in pids {
            if self.started.contains_key(&pid) {
                continue;
            }
            if let Some(started) = start_of(pid) {
                self.started.insert(pid, started);
                added += 1;
            }
        }
        added
    }

    /// Remembered processes that still run with the remembered start time.
    pub(super) fn alive<S>(&self, mut start_of: S) -> Vec<i32>
    where
        S: FnMut(i32) -> Option<u64>,
    {
        let mut alive: Vec<i32> = self
            .started
            .iter()
            .filter(|&(&pid, &started)| start_of(pid) == Some(started))
            .map(|(&pid, _)| pid)
            .collect();
        alive.sort_unstable();
        alive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(edges: &[(i32, &[i32])]) -> impl FnMut(i32) -> Vec<i32> + use<> {
        let map: HashMap<i32, Vec<i32>> = edges
            .iter()
            .map(|(parent, children)| (*parent, children.to_vec()))
            .collect();
        move |pid| map.get(&pid).cloned().unwrap_or_default()
    }

    #[test]
    fn the_tree_lists_every_descendant_once() {
        let children = table(&[(10, &[11, 12]), (11, &[13]), (12, &[13, 14]), (14, &[10])]);
        assert_eq!(tree_of(10, children), [10, 11, 12, 13, 14]);
    }

    #[test]
    fn a_process_without_children_is_its_own_tree() {
        assert_eq!(tree_of(7, table(&[])), [7]);
    }

    #[test]
    fn invalid_identifiers_are_not_part_of_the_tree() {
        assert_eq!(tree_of(7, table(&[(7, &[0, -1, 8])])), [7, 8]);
    }

    #[test]
    fn the_walk_stops_at_its_limit() {
        let wide = |pid: i32| -> Vec<i32> {
            if pid == 1 {
                (2..=i32::try_from(WALK_LIMIT * 2).unwrap()).collect()
            } else {
                Vec::new()
            }
        };
        assert_eq!(tree_of(1, wide).len(), WALK_LIMIT);
    }

    #[test]
    fn a_remembered_process_is_found_again_only_with_its_start_time() {
        let mut remembered = Remembered::default();
        let times = HashMap::from([(5, 100_u64), (6, 200), (7, 300)]);
        assert_eq!(
            remembered.add(&[5, 6, 7, 8], |pid| times.get(&pid).copied()),
            3
        );
        // 5 ended, 6 runs as before, 7's identifier now names another
        // process, 8 never existed.
        let later = HashMap::from([(6, 200_u64), (7, 999)]);
        assert_eq!(remembered.alive(|pid| later.get(&pid).copied()), [6]);
    }

    #[test]
    fn a_process_is_counted_once() {
        let mut remembered = Remembered::default();
        assert_eq!(remembered.add(&[5, 5], |_| Some(1)), 1);
        assert_eq!(remembered.add(&[5], |_| Some(2)), 0);
        assert_eq!(remembered.alive(|_| Some(1)), [5]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
