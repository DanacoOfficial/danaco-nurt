// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Comparison of normalized states and the text form of differences.

use std::fmt;

use crate::normalize::{RepoState, SectionState};
use crate::scenario::{IgnoreRule, StateSelector};

/// Longest number of lines of one value shown in a difference.
const SHOWN_LINES: usize = 12;

/// One key whose values differ between two states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// Section of the key.
    pub selector: StateSelector,
    /// Key within the section.
    pub key: String,
    /// Value on the left side (`git`, or the initial state); `None` when the
    /// key is absent.
    pub left: Option<String>,
    /// Value on the right side (Nurt); `None` when the key is absent.
    pub right: Option<String>,
}

/// Names of the two compared sides, for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sides {
    /// Name of the left side.
    pub left: &'static str,
    /// Name of the right side.
    pub right: &'static str,
}

impl Sides {
    /// Copy A after `git` against copy B after Nurt.
    pub const GIT_NURT: Self = Self {
        left: "git",
        right: "nurt",
    };

    /// Initial state against copy B after undo.
    pub const INITIAL_UNDONE: Self = Self {
        left: "initial",
        right: "undone",
    };
}

/// Compares the sections `selectors` of two states, without the keys
/// matched by `ignore`; differences are listed in section and key order.
///
/// ```
/// use nurt_git_difftest::compare::compare;
/// use nurt_git_difftest::normalize::{RepoState, SectionState};
/// use nurt_git_difftest::scenario::StateSelector;
///
/// let refs: StateSelector = "refs".parse()?;
/// let mut left = RepoState::default();
/// left.insert(refs.clone(), SectionState::from([("HEAD".into(), "a".into())]));
/// let mut right = RepoState::default();
/// right.insert(refs.clone(), SectionState::from([("HEAD".into(), "b".into())]));
/// let differences = compare(&left, &right, &[refs], &[]);
/// assert_eq!(differences.len(), 1);
/// assert_eq!(differences[0].key, "HEAD");
/// # Ok::<(), String>(())
/// ```
#[must_use]
pub fn compare(
    left: &RepoState,
    right: &RepoState,
    selectors: &[StateSelector],
    ignore: &[IgnoreRule],
) -> Vec<Difference> {
    let empty = SectionState::new();
    let mut differences = Vec::new();
    for selector in selectors {
        let left_section = left.section(selector).unwrap_or(&empty);
        let right_section = right.section(selector).unwrap_or(&empty);
        let keys = left_section
            .keys()
            .chain(right_section.keys())
            .collect::<std::collections::BTreeSet<_>>();
        for key in keys {
            if ignore.iter().any(|rule| rule.matches(selector, key)) {
                continue;
            }
            let left_value = left_section.get(key);
            let right_value = right_section.get(key);
            if left_value != right_value {
                differences.push(Difference {
                    selector: selector.clone(),
                    key: key.clone(),
                    left: left_value.cloned(),
                    right: right_value.cloned(),
                });
            }
        }
    }
    differences
}

/// Text form of differences with the values of both sides.
#[derive(Debug, Clone, Copy)]
pub struct DifferenceReport<'a> {
    /// Differences.
    pub differences: &'a [Difference],
    /// Names of the sides.
    pub sides: Sides,
}

impl fmt::Display for DifferenceReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for difference in self.differences {
            writeln!(f, "  [{}] {}", difference.selector, difference.key)?;
            write_value(f, self.sides.left, difference.left.as_deref())?;
            write_value(f, self.sides.right, difference.right.as_deref())?;
        }
        Ok(())
    }
}

fn write_value(f: &mut fmt::Formatter<'_>, side: &str, value: Option<&str>) -> fmt::Result {
    let Some(value) = value else {
        return writeln!(f, "    {side:>7}: (absent)");
    };
    let lines: Vec<&str> = value.lines().collect();
    let Some((first, rest)) = lines.split_first() else {
        return writeln!(f, "    {side:>7}: (empty)");
    };
    writeln!(f, "    {side:>7}: {first}")?;
    for line in rest.iter().take(SHOWN_LINES - 1) {
        writeln!(f, "             {line}")?;
    }
    if lines.len() > SHOWN_LINES {
        writeln!(f, "             ({} more lines)", lines.len() - SHOWN_LINES)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(selector: &StateSelector, pairs: &[(&str, &str)]) -> RepoState {
        let mut state = RepoState::default();
        state.insert(
            selector.clone(),
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        );
        state
    }

    #[test]
    fn reports_changed_added_and_removed_keys() {
        let worktree: StateSelector = "worktree".parse().unwrap();
        let left = state(&worktree, &[("a", "1"), ("b", "2")]);
        let right = state(&worktree, &[("a", "1"), ("b", "3"), ("c", "4")]);
        let differences = compare(&left, &right, std::slice::from_ref(&worktree), &[]);
        assert_eq!(differences.len(), 2);
        assert_eq!(differences[0].left.as_deref(), Some("2"));
        assert_eq!(differences[1].left, None);
        let back = compare(&right, &left, std::slice::from_ref(&worktree), &[]);
        assert_eq!(back[1].right, None);
    }

    #[test]
    fn ignores_keys_matched_by_rules() {
        let worktree: StateSelector = "worktree".parse().unwrap();
        let left = state(&worktree, &[("build/x", "1"), ("src/y", "1")]);
        let right = state(&worktree, &[("build/x", "2"), ("src/y", "2")]);
        let rule: IgnoreRule = "worktree build/*".parse().unwrap();
        let differences = compare(&left, &right, std::slice::from_ref(&worktree), &[rule]);
        assert_eq!(differences.len(), 1);
        assert_eq!(differences[0].key, "src/y");
        let whole: IgnoreRule = "worktree".parse().unwrap();
        assert!(compare(&left, &right, std::slice::from_ref(&worktree), &[whole]).is_empty());
    }

    #[test]
    fn compares_only_selected_sections() {
        let refs: StateSelector = "refs".parse().unwrap();
        let config: StateSelector = "config".parse().unwrap();
        let left = state(&refs, &[("HEAD", "x")]);
        let right = state(&refs, &[("HEAD", "y")]);
        assert!(compare(&left, &right, &[config], &[]).is_empty());
    }

    #[test]
    fn renders_values_of_both_sides_with_a_line_limit() {
        let long = (0..20)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let differences = [Difference {
            selector: "state_files".parse().unwrap(),
            key: "MERGE_MSG".into(),
            left: Some(long),
            right: None,
        }];
        let text = DifferenceReport {
            differences: &differences,
            sides: Sides::GIT_NURT,
        }
        .to_string();
        assert!(text.contains("[state_files] MERGE_MSG"));
        assert!(text.contains("    git: 0"));
        assert!(text.contains("(8 more lines)"));
        assert!(text.contains("nurt: (absent)"));
    }
}
