// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-help::lookup
// Responsibility:   Name lookup helpers shared by the help and the description layers.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Name lookup helpers shared by the help and the description layers.

use nurt_contracts::error::{Error, ErrorCode};

/// Largest number of names listed in `expected`.
pub(crate) const MAX_EXPECTED: usize = 8;

/// Smallest distance a name may be from the unknown one and still be offered.
const MIN_OFFERED_DISTANCE: usize = 2;

/// Optimal string alignment distance: insertions, deletions, substitutions
/// and transpositions of adjacent characters.
pub(crate) fn distance(left: &str, right: &str) -> usize {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    let mut rows = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in rows[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = best;
        }
    }
    rows[a.len()][b.len()]
}

/// Up to `limit` candidates nearest to `name`, nearest first, ties in
/// alphabetical order. Candidates that start with `name` are always near.
pub(crate) fn nearest(name: &str, candidates: &[String], limit: usize) -> Vec<String> {
    let lower = name.to_lowercase();
    let reach = (lower.chars().count() / 2).max(MIN_OFFERED_DISTANCE);
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .filter_map(|candidate| {
            let folded = candidate.to_lowercase();
            let near = distance(&lower, &folded);
            (near <= reach || folded.starts_with(&lower)).then_some((near, candidate))
        })
        .collect();
    scored.sort();
    scored.dedup();
    scored
        .into_iter()
        .take(limit)
        .map(|(_, candidate)| candidate.clone())
        .collect()
}

/// Error for a name that is not known: `E_UNKNOWN_NAME` with the nearest
/// names in `expected`.
pub(crate) fn unknown(what: &str, name: &str, candidates: &[String]) -> Error {
    let expected = nearest(name, candidates, MAX_EXPECTED);
    Error::new(
        ErrorCode::UnknownName,
        format!("no {what} named {name}; choose a name from expected or run help for the list"),
    )
    .with_found(name)
    .with_expected(serde_json::json!(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn transposition_costs_one_edit() {
        assert_eq!(distance("remvoe", "remove"), 1);
        assert_eq!(distance("find", "fnid"), 1);
    }

    #[test]
    fn distance_of_equal_names_is_zero() {
        assert_eq!(distance("list", "list"), 0);
        assert_eq!(distance("", "abc"), 3);
    }

    #[test]
    fn nearest_orders_by_distance_then_name() {
        let found = nearest("serach", &names(&["search", "sort", "info"]), 8);
        assert_eq!(found, ["search"]);
    }

    #[test]
    fn nearest_is_limited() {
        let candidates = names(&["aa1", "aa2", "aa3", "aa4"]);
        assert_eq!(nearest("aa", &candidates, 2).len(), 2);
    }

    #[test]
    fn prefix_of_a_name_is_near() {
        let found = nearest("git.s", &names(&["git.status", "git.stash", "list"]), 8);
        assert_eq!(found, ["git.stash", "git.status"]);
    }

    #[test]
    fn unknown_name_error_lists_expected() {
        let error = unknown("group", "documnts", &names(&["documents", "text"]));
        assert_eq!(error.code(), ErrorCode::UnknownName);
        assert_eq!(error.expected, Some(serde_json::json!(["documents"])));
        assert_eq!(error.found.as_deref(), Some("documnts"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
