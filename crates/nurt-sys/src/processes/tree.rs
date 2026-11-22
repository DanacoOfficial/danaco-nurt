// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Ordering of processes as a tree.
//!
//! The selected processes are the roots; the output holds each root followed
//! by all its descendants in depth-first order, children ordered by
//! identifier. A selected process that descends from another selected process
//! appears once, under its ancestor. Parent links read at different moments
//! can form a cycle when an identifier is reused; the walk stops at the first
//! process it has already visited and counts the cut.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::model::{ProcessInfo, TreePosition};

/// Processes in tree order.
#[derive(Debug, Default)]
pub(crate) struct TreeOrder {
    /// Processes with their position, in output order.
    pub(crate) entries: Vec<(ProcessInfo, TreePosition)>,
    /// Number of parent links that closed a cycle and were cut.
    pub(crate) cycles: u64,
}

/// Orders `processes` as a tree whose roots are the processes for which
/// `selected` returns `true`.
pub(crate) fn order(
    processes: Vec<ProcessInfo>,
    selected: impl Fn(&ProcessInfo) -> bool,
) -> TreeOrder {
    let table = Table::new(processes, selected);
    let mut walk = Walk::default();
    for &pid in &table.roots() {
        walk.visit(&table, pid);
    }
    while let Some(pid) = table
        .selected
        .iter()
        .copied()
        .find(|pid| !walk.visited.contains(pid))
    {
        walk.visit(&table, pid);
    }
    let mut by_pid = table.processes;
    TreeOrder {
        entries: walk
            .order
            .into_iter()
            .filter_map(|(pid, position)| Some((by_pid.remove(&pid)?, position)))
            .collect(),
        cycles: walk.cycles,
    }
}

struct Table {
    processes: HashMap<u64, ProcessInfo>,
    children: BTreeMap<u64, Vec<u64>>,
    /// Selected processes in ascending order of identifier.
    selected: Vec<u64>,
}

impl Table {
    fn new(processes: Vec<ProcessInfo>, selected: impl Fn(&ProcessInfo) -> bool) -> Self {
        let mut children: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        let mut chosen = Vec::new();
        for process in &processes {
            if process.parent_pid != process.pid {
                children
                    .entry(process.parent_pid)
                    .or_default()
                    .push(process.pid);
            }
            if selected(process) {
                chosen.push(process.pid);
            }
        }
        for list in children.values_mut() {
            list.sort_unstable();
        }
        chosen.sort_unstable();
        Self {
            processes: processes.into_iter().map(|p| (p.pid, p)).collect(),
            children,
            selected: chosen,
        }
    }

    fn parent(&self, pid: u64) -> Option<u64> {
        let parent = self.processes.get(&pid)?.parent_pid;
        (parent != pid && self.processes.contains_key(&parent)).then_some(parent)
    }

    /// Selected processes without a selected ancestor.
    fn roots(&self) -> Vec<u64> {
        let selected: HashSet<u64> = self.selected.iter().copied().collect();
        self.selected
            .iter()
            .copied()
            .filter(|&pid| !self.has_selected_ancestor(pid, &selected))
            .collect()
    }

    fn has_selected_ancestor(&self, pid: u64, selected: &HashSet<u64>) -> bool {
        let mut seen = HashSet::from([pid]);
        let mut current = self.parent(pid);
        while let Some(ancestor) = current {
            if !seen.insert(ancestor) {
                return false;
            }
            if selected.contains(&ancestor) {
                return true;
            }
            current = self.parent(ancestor);
        }
        false
    }

    fn children(&self, pid: u64) -> &[u64] {
        self.children.get(&pid).map_or(&[], Vec::as_slice)
    }
}

#[derive(Default)]
struct Walk {
    visited: HashSet<u64>,
    order: Vec<(u64, TreePosition)>,
    cycles: u64,
}

impl Walk {
    fn visit(&mut self, table: &Table, root: u64) {
        let mut stack = vec![(root, 0_u64)];
        while let Some((pid, depth)) = stack.pop() {
            if !self.visited.insert(pid) {
                self.cycles += 1;
                continue;
            }
            let children = table.children(pid);
            self.order.push((
                pid,
                TreePosition {
                    depth,
                    children: children.to_vec(),
                },
            ));
            stack.extend(children.iter().rev().map(|&child| (child, depth + 1)));
        }
    }
}
