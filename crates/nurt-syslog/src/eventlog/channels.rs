// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Channels of the Windows event log.
//!
//! Channel names do not distinguish letter case, so names and patterns of
//! the parameter `unit` are matched without regard to case.

/// Channels read when `unit` is absent.
///
/// `Security` is not among them because reading it requires the privilege
/// `SeSecurityPrivilege`.
pub const DEFAULT_CHANNELS: [&str; 2] = ["System", "Application"];

/// Largest number of channel names offered for an unknown channel.
pub const MAX_CANDIDATES: usize = 8;

/// Up to [`MAX_CANDIDATES`] channel names nearest to an unknown name.
///
/// Names that contain the unknown name come first, then names by increasing
/// edit distance. Ties are ordered by name, and letter case is ignored.
///
/// # Examples
///
/// ```
/// use nurt_syslog::eventlog::channels::nearest;
/// let available = ["Application", "Microsoft-Windows-Sysmon/Operational", "Setup", "System"]
///     .map(String::from);
/// assert_eq!(nearest("Sytem", &available)[0], "System");
/// assert_eq!(nearest("sysmon", &available)[0], "Microsoft-Windows-Sysmon/Operational");
/// ```
#[must_use]
pub fn nearest(name: &str, available: &[String]) -> Vec<String> {
    let wanted_text = name.to_lowercase();
    let wanted: Vec<char> = wanted_text.chars().collect();
    let mut ranked: Vec<(bool, usize, &String)> = available
        .iter()
        .map(|channel| {
            let lower = channel.to_lowercase();
            let candidate: Vec<char> = lower.chars().collect();
            let distance = edit_distance(&wanted, &candidate);
            (!lower.contains(&wanted_text), distance, channel)
        })
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, _, channel)| channel.clone())
        .collect()
}

/// Levenshtein distance between two sequences of characters.
fn edit_distance(left: &[char], right: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (row, left_char) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != right_char);
            let deletion = previous[column + 1] + 1;
            let insertion = current[column] + 1;
            current[column + 1] = substitution.min(deletion).min(insertion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}
