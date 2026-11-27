// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::rules::format
// Responsibility:   Short texts used in the `detail` field of notable entries.
// Dependencies:     nurt_contracts, nurt_types
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Short texts used in the `detail` field of notable entries.

use nurt_contracts::value::Size;
use nurt_types::display_size;

/// Factor that converts a ratio to a percentage.
const PERCENT: f64 = 100.0;

/// Size in the short report notation of `nurt_types`, for example `3.4gib`,
/// `512b` or `1.0mib`.
pub(crate) fn format_size(bytes: u64) -> String {
    display_size(Size::from_bytes(bytes))
}

/// Number followed by the singular or plural noun, for example `1 file` or
/// `3 files`.
pub(crate) fn count_noun(count: u64, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
}

/// Ratio between 0 and 1 as a whole percentage, rounded down.
pub(crate) fn percent(ratio: f64) -> u64 {
    (ratio.clamp(0.0, 1.0) * PERCENT).floor() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_the_shortest_binary_unit() {
        assert_eq!(format_size(0), "0b");
        assert_eq!(format_size(1023), "1023b");
        assert_eq!(format_size(1024), "1.0kib");
        assert_eq!(format_size(1536), "1.5kib");
        assert_eq!(format_size(1 << 20), "1.0mib");
        assert_eq!(format_size((1 << 20) - 1), "1.0mib");
        assert_eq!(format_size(3_650_722_201), "3.4gib");
        assert_eq!(format_size(u64::MAX), "16.0eib");
    }

    #[test]
    fn nouns_agree_with_the_count() {
        assert_eq!(count_noun(1, "file", "files"), "1 file");
        assert_eq!(count_noun(0, "file", "files"), "0 files");
        assert_eq!(count_noun(18_402, "file", "files"), "18402 files");
    }

    #[test]
    fn percentages_are_rounded_down_and_bounded() {
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(0.799), 79);
        assert_eq!(percent(1.5), 100);
        assert_eq!(percent(-0.5), 0);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
