// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Short texts used in the `detail` field of notable entries.

/// Binary units above bytes, in increasing order.
const UNITS: [&str; 6] = ["kib", "mib", "gib", "tib", "pib", "eib"];

/// Ratio between consecutive binary units.
const STEP: f64 = 1024.0;

/// Bytes below which a size is written in bytes.
const STEP_BYTES: u64 = 1024;

/// Factor that converts a ratio to a percentage.
const PERCENT: f64 = 100.0;

/// Size in the shortest binary unit with one decimal digit, for example
/// `3.4gib`, `512b` or `1mib`.
pub(crate) fn format_size(bytes: u64) -> String {
    if bytes < STEP_BYTES {
        return format!("{bytes}b");
    }
    let mut value = bytes as f64 / STEP;
    let mut unit = 0;
    while round_tenth(value) >= STEP && unit + 1 < UNITS.len() {
        value /= STEP;
        unit += 1;
    }
    let text = format!("{:.1}", round_tenth(value));
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text}{}", UNITS[unit])
}

fn round_tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
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
        assert_eq!(format_size(1024), "1kib");
        assert_eq!(format_size(1536), "1.5kib");
        assert_eq!(format_size(1 << 20), "1mib");
        assert_eq!(format_size((1 << 20) - 1), "1mib");
        assert_eq!(format_size(3_650_722_201), "3.4gib");
        assert_eq!(format_size(u64::MAX), "16eib");
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
