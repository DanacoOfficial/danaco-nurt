// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Notable effects derived from the changes of an operation, and their merge
//! with the codes emitted by commands.
//!
//! | Code | Condition | Severity |
//! |---|---|---|
//! | `outside_project` | a path lies outside every `scope.project_roots` entry (rule active when the list is not empty) | `alert` |
//! | `system_path` | a path lies in a system directory of the host | `alert` |
//! | `home_config` | a path is a configuration entry of the home directory | `warning` |
//! | `mass_delete` | deleted files reach `mass_files`, their bytes reach `mass_bytes`, or the deleted share of a scope directory reaches `mass_ratio` | `warning`; `alert` at ten times `mass_files` or `mass_bytes` |
//! | `delete_tree` | a non-empty directory is deleted recursively | `notice` |
//! | `overwrote_existing` | a write, move or copy replaced an existing file | `warning` |
//! | `file_emptied` | a file with content has 0 bytes after the change | `warning` |
//! | `size_shrunk` | a file of at least 1 KiB shrank by at least `shrink_ratio` | `warning` |
//! | `many_per_file` | one file received at least `per_file_repl` replacements | `notice` |
//! | `target_count_high` | the number of targets reaches `targets_high` | `notice` |
//!
//! Every comparison with a threshold is inclusive: a value equal to the
//! threshold meets the rule.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use nurt_contracts::config::NotableThresholds;
use nurt_contracts::report::{ChangeAction, Notable, NotableEntry, Severity};
use nurt_contracts::value::{Path, Size};

use super::format::{count_noun, format_size, percent};
use super::hits::{Collected, Hits, combine_counts, many_targets};
use super::paths::{PathContext, is_fs_path, parent, reportable_ancestor, shrink_to_common};
use crate::assemble::{EntryKind, ReportContext, ScopeDirectory, TargetChange};

/// Smallest size before a change, in bytes, for which `size_shrunk` is
/// considered.
pub const SHRINK_MIN_BYTES: u64 = 1024;

/// Multiple of the `mass_files` or `mass_bytes` threshold from which a mass
/// deletion is an alert.
pub const ALERT_FACTOR: u64 = 10;

/// Derives the notable effects of an operation from its changes.
///
/// `targets` is the number of targets of the operation and drives
/// `target_count_high`. `scope_directories` are the directories whose
/// deleted share is measured for `mass_delete`. Entries are returned in no
/// particular order; [`merge_notable`] orders them.
///
/// A code met by more than [`ENTRIES_PER_CODE`](super::ENTRIES_PER_CODE)
/// targets is reported as one entry whose `count` is the number of targets
/// and whose `path` is their common directory.
#[must_use]
pub fn derive_notable(
    ctx: &ReportContext,
    changes: &[TargetChange],
    targets: u64,
    scope_directories: &[ScopeDirectory],
) -> Vec<NotableEntry> {
    let thresholds = &ctx.config().notable;
    let mut per_target = PerTarget::default();
    let mut deletions = Deletions::default();
    for target in changes {
        per_target.observe(ctx, target);
        deletions.observe(target);
    }
    let mut entries = per_target.finish();
    entries.extend(deletions.mass_delete(ctx.paths(), thresholds, scope_directories));
    entries.extend(deletions.delete_trees());
    entries.extend(target_count_high(thresholds, targets));
    entries
}

/// Merges derived entries with the entries emitted by commands and orders
/// the result.
///
/// A derived entry takes precedence over an emitted entry with the same code
/// and path. Entries with the same code, path and detail are merged into one
/// with the higher severity and the sum of their counts. The result is in
/// descending order of severity; entries of equal severity follow the
/// declaration order of the codes, then the path, then the detail, so the
/// order does not depend on the order in which entries were emitted.
#[must_use]
pub fn merge_notable(
    derived: Vec<NotableEntry>,
    emitted: impl IntoIterator<Item = NotableEntry>,
) -> Vec<NotableEntry> {
    let kept: Vec<NotableEntry> = {
        let known: HashSet<(Notable, Option<&str>)> = derived
            .iter()
            .map(|entry| (entry.code, entry.path.as_deref()))
            .collect();
        emitted
            .into_iter()
            .filter(|entry| !known.contains(&(entry.code, entry.path.as_deref())))
            .collect()
    };
    let mut entries = derived;
    entries.extend(kept);
    entries.sort_by(|left, right| merge_key(left).cmp(&merge_key(right)));
    entries.dedup_by(|later, earlier| {
        if merge_key(later) != merge_key(earlier) {
            return false;
        }
        earlier.severity = earlier.severity.max(later.severity);
        earlier.count = combine_counts(earlier.count, later.count);
        true
    });
    entries.sort_by_key(|entry| Reverse(entry.severity));
    entries
}

fn merge_key(entry: &NotableEntry) -> (usize, Option<&str>, Option<&str>) {
    (
        code_rank(entry.code),
        entry.path.as_deref(),
        entry.detail.as_deref(),
    )
}

fn code_rank(code: Notable) -> usize {
    Notable::ALL
        .iter()
        .position(|candidate| *candidate == code)
        .unwrap_or(Notable::ALL.len())
}

/// Converts the targets collected for `code` into entries.
fn entries_for(code: Notable, hits: Hits) -> Vec<NotableEntry> {
    match hits.collect() {
        Collected::Each(list) => list
            .into_iter()
            .map(|hit| NotableEntry {
                path: Some(hit.path),
                count: hit.count,
                detail: hit.detail,
                ..NotableEntry::new(code)
            })
            .collect(),
        Collected::Many { total, ancestor } => vec![NotableEntry {
            path: ancestor,
            count: Some(total),
            detail: Some(many_targets(total)),
            ..NotableEntry::new(code)
        }],
    }
}

/// Rules evaluated for each change on its own.
#[derive(Debug, Default)]
struct PerTarget {
    outside_project: Hits,
    system_path: Hits,
    home_config: Hits,
    overwrote_existing: Hits,
    file_emptied: Hits,
    size_shrunk: Hits,
    many_per_file: Hits,
}

impl PerTarget {
    fn observe(&mut self, ctx: &ReportContext, target: &TargetChange) {
        self.observe_locations(ctx, target);
        if target.change.action != ChangeAction::Delete {
            self.observe_content(&ctx.config().notable, target);
        }
    }

    fn observe_locations(&mut self, ctx: &ReportContext, target: &TargetChange) {
        let change = &target.change;
        let paths = || {
            std::iter::once(change.path.as_str())
                .chain(change.from.as_ref().map(Path::as_str))
                .chain(change.to.as_ref().map(Path::as_str))
                .filter(|path| is_fs_path(path))
        };
        let host = ctx.paths();
        let roots = ctx.project_roots();
        if !roots.is_empty()
            && let Some(path) =
                paths().find(|path| !roots.iter().any(|root| host.is_within(path, root)))
        {
            self.outside_project.push(path, None, || None);
        }
        if let Some(path) = paths().find(|path| host.is_system_path(path)) {
            self.system_path.push(path, None, || None);
        }
        if let Some(path) = paths().find(|path| host.is_home_config(path)) {
            self.home_config.push(path, None, || None);
        }
    }

    fn observe_content(&mut self, thresholds: &NotableThresholds, target: &TargetChange) {
        let change = &target.change;
        let path = change.path.as_str();
        let before = change.size_before.map(Size::bytes);
        let after = change.size_after.map(Size::bytes);
        match (before, after) {
            (Some(before), Some(0)) if before > 0 => {
                self.file_emptied
                    .push(path, None, || Some(format!("was {}", format_size(before))));
            }
            (Some(before), Some(after)) if shrunk(thresholds.shrink_ratio, before, after) => {
                self.size_shrunk.push(path, None, || {
                    Some(format!(
                        "from {} to {}",
                        format_size(before),
                        format_size(after)
                    ))
                });
            }
            _ => {}
        }
        if let Some(replacements) = change.replacements
            && replacements > 0
            && replacements >= u64::from(thresholds.per_file_repl)
        {
            self.many_per_file.push(path, Some(replacements), || None);
        }
        if target.replaced_existing {
            self.overwrote_existing.push(path, None, || None);
        }
    }

    fn finish(self) -> Vec<NotableEntry> {
        [
            (Notable::OutsideProject, self.outside_project),
            (Notable::SystemPath, self.system_path),
            (Notable::HomeConfig, self.home_config),
            (Notable::OverwroteExisting, self.overwrote_existing),
            (Notable::FileEmptied, self.file_emptied),
            (Notable::SizeShrunk, self.size_shrunk),
            (Notable::ManyPerFile, self.many_per_file),
        ]
        .into_iter()
        .filter(|(_, hits)| !hits.is_empty())
        .flat_map(|(code, hits)| entries_for(code, hits))
        .collect()
    }
}

/// Whether a file of `before` bytes that has `after` bytes shrank by at
/// least `ratio` of its size. Files below [`SHRINK_MIN_BYTES`] and emptied
/// files are not considered.
fn shrunk(ratio: f64, before: u64, after: u64) -> bool {
    before >= SHRINK_MIN_BYTES
        && after > 0
        && after < before
        && (before - after) as f64 >= ratio * before as f64
}

/// Deletions of an operation, for `mass_delete` and `delete_tree`.
#[derive(Debug, Default)]
struct Deletions<'a> {
    files: u64,
    bytes: u64,
    sized: bool,
    file_paths: Vec<&'a str>,
    ancestor: Option<String>,
    directories: Vec<&'a TargetChange>,
    all_paths: Vec<&'a str>,
}

impl<'a> Deletions<'a> {
    fn observe(&mut self, target: &'a TargetChange) {
        if target.change.action != ChangeAction::Delete {
            return;
        }
        let path = target.change.path.as_str();
        self.all_paths.push(path);
        if target.entry == EntryKind::Directory {
            self.directories.push(target);
            return;
        }
        if !target.entry.is_file() {
            return;
        }
        self.files += 1;
        if let Some(size) = target.change.size_before {
            self.bytes = self.bytes.saturating_add(size.bytes());
            self.sized = true;
        }
        self.file_paths.push(path);
        match &mut self.ancestor {
            Some(prefix) => shrink_to_common(prefix, path),
            None => self.ancestor = Some(path.to_owned()),
        }
    }

    fn mass_delete(
        &self,
        host: &PathContext,
        thresholds: &NotableThresholds,
        scope_directories: &[ScopeDirectory],
    ) -> Option<NotableEntry> {
        if self.files == 0 {
            return None;
        }
        let mass_files = u64::from(thresholds.mass_files);
        let mass_bytes = thresholds.mass_bytes.bytes();
        let share = self.largest_share(host, scope_directories);
        let by_share = share.is_some_and(|(ratio, _)| ratio >= thresholds.mass_ratio);
        if !(self.files >= mass_files || self.bytes >= mass_bytes || by_share) {
            return None;
        }
        let alert = reaches_alert(self.files, mass_files) || reaches_alert(self.bytes, mass_bytes);
        let path = match share {
            Some((_, directory)) if by_share => Some(directory.to_owned()),
            _ => self.ancestor.clone().and_then(reportable_ancestor),
        };
        Some(NotableEntry {
            severity: if alert {
                Severity::Alert
            } else {
                Severity::Warning
            },
            path,
            count: Some(self.files),
            detail: Some(self.mass_detail(share)),
            ..NotableEntry::new(Notable::MassDelete)
        })
    }

    fn mass_detail(&self, share: Option<(f64, &str)>) -> String {
        let mut detail = count_noun(self.files, "file", "files");
        if self.sized {
            detail.push_str(", ");
            detail.push_str(&format_size(self.bytes));
        }
        if let Some((ratio, directory)) = share {
            detail.push_str(&format!(", {}% of {directory}", percent(ratio)));
        }
        detail
    }

    /// The scope directory with the largest deleted share of its files, and
    /// that share; the first directory wins a tie.
    fn largest_share<'s>(
        &self,
        host: &PathContext,
        scope_directories: &'s [ScopeDirectory],
    ) -> Option<(f64, &'s str)> {
        let mut largest: Option<(f64, &'s str)> = None;
        for directory in scope_directories.iter().filter(|dir| dir.files_before > 0) {
            let deleted = self
                .file_paths
                .iter()
                .filter(|path| host.is_within(path, &directory.path))
                .count();
            if deleted == 0 {
                continue;
            }
            let ratio = deleted as f64 / directory.files_before as f64;
            if largest.is_none_or(|(best, _)| ratio > best) {
                largest = Some((ratio, directory.path.as_str()));
            }
        }
        largest
    }

    /// Topmost deleted directories that held entries, with the number of
    /// entries deleted below them.
    fn delete_trees(&self) -> Vec<NotableEntry> {
        if self.directories.is_empty() {
            return Vec::new();
        }
        let directories: HashSet<&str> = self
            .directories
            .iter()
            .map(|target| target.change.path.as_str())
            .collect();
        let mut below: HashMap<&str, u64> = HashMap::new();
        let mut nested: HashSet<&str> = HashSet::new();
        for &path in &self.all_paths {
            if let Some(top) = topmost_deleted_ancestor(path, &directories) {
                *below.entry(top).or_default() += 1;
                if directories.contains(path) {
                    nested.insert(path);
                }
            }
        }
        let mut hits = Hits::default();
        for target in &self.directories {
            let path = target.change.path.as_str();
            if nested.contains(path) {
                continue;
            }
            let listed = below.get(path).copied().unwrap_or(0);
            let entries = listed.max(target.entries_inside.unwrap_or(0));
            if entries > 0 {
                hits.push(path, Some(entries), || None);
            }
        }
        entries_for(Notable::DeleteTree, hits)
    }
}

/// The highest ancestor of `path` that is itself a deleted directory.
fn topmost_deleted_ancestor<'p>(path: &'p str, directories: &HashSet<&str>) -> Option<&'p str> {
    let mut current = path;
    let mut top = None;
    while let Some(dir) = parent(current) {
        if directories.contains(dir) {
            top = Some(dir);
        }
        current = dir;
    }
    top
}

/// Whether `value` reaches [`ALERT_FACTOR`] times a non-zero `threshold`.
fn reaches_alert(value: u64, threshold: u64) -> bool {
    threshold > 0 && value >= threshold.saturating_mul(ALERT_FACTOR)
}

fn target_count_high(thresholds: &NotableThresholds, targets: u64) -> Option<NotableEntry> {
    (targets > 0 && targets >= u64::from(thresholds.targets_high)).then(|| NotableEntry {
        count: Some(targets),
        ..NotableEntry::new(Notable::TargetCountHigh)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_requires_the_minimum_size_and_ratio() {
        assert!(shrunk(0.5, 2048, 1024));
        assert!(!shrunk(0.5, 2048, 1025));
        assert!(!shrunk(0.5, 1023, 1));
        assert!(!shrunk(0.5, 2048, 0));
        assert!(!shrunk(0.5, 2048, 4096));
    }

    #[test]
    fn alert_requires_a_non_zero_threshold() {
        assert!(reaches_alert(1000, 100));
        assert!(!reaches_alert(999, 100));
        assert!(!reaches_alert(5, 0));
        assert!(reaches_alert(u64::MAX, u64::MAX));
    }

    #[test]
    fn topmost_ancestor_skips_directories_that_were_kept() {
        let directories: HashSet<&str> = ["/srv/a", "/srv/a/b/c"].into_iter().collect();
        assert_eq!(
            topmost_deleted_ancestor("/srv/a/b/c/d", &directories),
            Some("/srv/a")
        );
        assert_eq!(topmost_deleted_ancestor("/srv/a", &directories), None);
        assert_eq!(topmost_deleted_ancestor("/srv/x/y", &directories), None);
    }
}
