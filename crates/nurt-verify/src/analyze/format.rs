// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Preview of a formatter run: the changed lines of a file.

/// Longest sample of changed lines per file.
pub(crate) const SAMPLE_LINES_MAX: usize = 200;

/// Largest product of line counts compared by the sample diff.
const DIFF_CELLS_MAX: usize = 4_000_000;

/// Lines changed by a formatter, as `-old` and `+new` lines in file order,
/// without context; at most `limit` lines. Lines are split at `\n` only, so a
/// change of the line ending is a change of the line; a carriage return at
/// the end of a line is shown as the two characters `\r`.
pub(crate) fn changed_lines(before: &str, after: &str, limit: usize) -> Vec<String> {
    let old: Vec<&str> = before.split('\n').collect();
    let new: Vec<&str> = after.split('\n').collect();
    let (prefix, suffix) = common_edges(&old, &new);
    let old = &old[prefix..old.len() - suffix];
    let new = &new[prefix..new.len() - suffix];
    if old.len().saturating_mul(new.len()) > DIFF_CELLS_MAX {
        return replaced(old, new, limit);
    }
    diff(old, new).into_iter().take(limit).collect()
}

fn common_edges(old: &[&str], new: &[&str]) -> (usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let shorter = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(shorter)
        .take_while(|(a, b)| a == b)
        .count();
    (prefix, suffix)
}

fn replaced(old: &[&str], new: &[&str], limit: usize) -> Vec<String> {
    old.iter()
        .map(|line| format!("-{}", shown(line)))
        .chain(new.iter().map(|line| format!("+{}", shown(line))))
        .take(limit)
        .collect()
}

/// Line as shown in the preview: a final carriage return is made visible.
fn shown(line: &str) -> String {
    match line.strip_suffix('\r') {
        Some(text) => format!("{text}\\r"),
        None => line.to_owned(),
    }
}

/// Longest-common-subsequence diff of two short line lists.
fn diff(old: &[&str], new: &[&str]) -> Vec<String> {
    let width = new.len() + 1;
    let mut table = vec![0u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut lines = Vec::new();
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
        } else if j == new.len()
            || (i < old.len() && table[(i + 1) * width + j] >= table[i * width + j + 1])
        {
            lines.push(format!("-{}", shown(old[i])));
            i += 1;
        } else {
            lines.push(format!("+{}", shown(new[j])));
            j += 1;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::changed_lines;

    #[test]
    fn lists_removed_and_added_lines_in_order() {
        let lines = changed_lines("a\nb  \nc\n", "a\nb\nc\nd\n", 10);
        assert_eq!(lines, vec!["-b  ", "+b", "+d"]);
    }

    #[test]
    fn a_change_of_the_line_ending_alone_is_visible() {
        let lines = changed_lines("a\r\nb\r\n", "a\nb\n", 10);
        assert_eq!(lines, vec!["-a\\r", "-b\\r", "+a", "+b"]);
        assert_eq!(changed_lines("a\n", "a", 10), vec!["-"]);
    }

    #[test]
    fn identical_texts_have_no_changes() {
        assert!(changed_lines("a\nb\n", "a\nb\n", 10).is_empty());
    }

    #[test]
    fn limits_the_sample() {
        let lines = changed_lines("1\n2\n3\n", "x\ny\nz\n", 2);
        assert_eq!(lines.len(), 2);
    }
}
