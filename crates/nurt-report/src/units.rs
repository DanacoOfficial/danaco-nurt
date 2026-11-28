// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::units
// Responsibility:   Textual notation of sizes, durations and counts in rendered reports.
// Dependencies:     nurt_contracts, nurt_types
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Textual notation of sizes, durations and counts in rendered reports.

use nurt_contracts::value::{Duration, Size};
use nurt_types::{display_duration, display_size};

const DIGITS_PER_GROUP: usize = 3;

/// Formats a size in bytes in the short notation of reports.
pub(crate) fn format_size(bytes: u64) -> String {
    display_size(Size::from_bytes(bytes))
}

/// Formats a duration given in milliseconds in the short notation of reports.
pub(crate) fn format_duration(millis: i64) -> String {
    display_duration(Duration::from_millis(millis))
}

/// Formats a count with a space between groups of three digits.
pub(crate) fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / DIGITS_PER_GROUP);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(DIGITS_PER_GROUP) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    grouped
}

/// Writes a count followed by the singular or the plural form of a noun.
pub(crate) fn plural(count: u64, singular: &str, many: &str) -> String {
    let word = if count == 1 { singular } else { many };
    format!("{} {word}", format_count(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_sizes_and_durations_in_the_short_notation() {
        assert_eq!(format_size(12), "12b");
        assert_eq!(format_size(42_189), "41.2kib");
        assert_eq!(format_duration(12_400), "12.4s");
        assert_eq!(format_duration(250), "250ms");
    }

    #[test]
    fn groups_counts_by_three_digits() {
        assert_eq!(format_count(7), "7");
        assert_eq!(format_count(1207), "1 207");
        assert_eq!(format_count(18_402), "18 402");
        assert_eq!(format_count(1_234_567), "1 234 567");
    }

    #[test]
    fn chooses_the_plural_form() {
        assert_eq!(plural(1, "file", "files"), "1 file");
        assert_eq!(plural(1207, "dir", "dirs"), "1 207 dirs");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
