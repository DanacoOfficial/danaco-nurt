// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Numbers written with a space before their unit.
//!
//! `10 mb` is two valid arguments, a number and a word, so the space is not
//! removed before parsing. The binder calls [`join_unit`] when a number stands
//! in a place of type `size` or `duration` and the next argument is a unit
//! word; the joined literal is then read by the type rules, which report the
//! correction.

/// Units of a size literal.
const SIZE_UNITS: [&str; 9] = ["b", "kb", "mb", "gb", "tb", "kib", "mib", "gib", "tib"];

/// Units of a duration literal.
const DURATION_UNITS: [&str; 6] = ["ms", "s", "m", "h", "d", "w"];

/// Type of the place of a number followed by a unit word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitPlace {
    /// Place of type `size`.
    Size,
    /// Place of type `duration`.
    Duration,
}

/// Joins a number and the unit word written after it into one literal.
///
/// `number` and `unit` are the two tokens as written. The unit is compared
/// without regard to letter case and must be a unit of the place; a size may
/// have a fractional part, a duration may not. Returns `None` when the two
/// tokens do not form a literal of the place, for example a one-letter size
/// unit such as `M`, which is ambiguous between a binary and a decimal unit.
///
/// # Examples
///
/// ```
/// use nurt_correct::lexical::spacing::{UnitPlace, join_unit};
///
/// assert_eq!(join_unit("10", "MB", UnitPlace::Size).as_deref(), Some("10mb"));
/// assert_eq!(join_unit("90", "m", UnitPlace::Duration).as_deref(), Some("90m"));
/// assert_eq!(join_unit("10", "M", UnitPlace::Size), None);
/// assert_eq!(join_unit("1.5", "h", UnitPlace::Duration), None);
/// ```
#[must_use]
pub fn join_unit(number: &str, unit: &str, place: UnitPlace) -> Option<String> {
    let unit = unit.to_ascii_lowercase();
    let (units, fraction_allowed): (&[&str], bool) = match place {
        UnitPlace::Size => (&SIZE_UNITS, true),
        UnitPlace::Duration => (&DURATION_UNITS, false),
    };
    let one_letter_size = place == UnitPlace::Size && unit.len() == 1 && unit != "b";
    if !units.contains(&unit.as_str()) || one_letter_size || !is_number(number, fraction_allowed) {
        return None;
    }
    Some(format!("{number}{unit}"))
}

/// Whether `text` is a decimal number literal: `10`, `1_000`, `1.5`.
fn is_number(text: &str, fraction_allowed: bool) -> bool {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) if fraction_allowed => (whole, Some(fraction)),
        Some(_) => return false,
        None => (text, None),
    };
    let digits = |part: &str| {
        part.starts_with(|c: char| c.is_ascii_digit())
            && part.chars().all(|c| c.is_ascii_digit() || c == '_')
    };
    digits(whole) && fraction.is_none_or(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_size_units_in_any_letter_case() {
        assert_eq!(
            join_unit("10", "mb", UnitPlace::Size).as_deref(),
            Some("10mb")
        );
        assert_eq!(
            join_unit("1.5", "GiB", UnitPlace::Size).as_deref(),
            Some("1.5gib")
        );
        assert_eq!(
            join_unit("512", "B", UnitPlace::Size).as_deref(),
            Some("512b")
        );
        assert_eq!(
            join_unit("1_000", "kb", UnitPlace::Size).as_deref(),
            Some("1_000kb")
        );
    }

    #[test]
    fn joins_duration_units_of_whole_numbers() {
        assert_eq!(
            join_unit("250", "ms", UnitPlace::Duration).as_deref(),
            Some("250ms")
        );
        assert_eq!(
            join_unit("2", "D", UnitPlace::Duration).as_deref(),
            Some("2d")
        );
        assert_eq!(join_unit("1.5", "h", UnitPlace::Duration), None);
    }

    #[test]
    fn rejects_units_of_another_place_and_ambiguous_letters() {
        assert_eq!(join_unit("10", "m", UnitPlace::Size), None);
        assert_eq!(join_unit("10", "mb", UnitPlace::Duration), None);
        assert_eq!(join_unit("10", "G", UnitPlace::Size), None);
        assert_eq!(join_unit("10", "megabytes", UnitPlace::Size), None);
    }

    #[test]
    fn rejects_words_that_are_not_numbers() {
        assert_eq!(join_unit("ten", "mb", UnitPlace::Size), None);
        assert_eq!(join_unit("0x10", "mb", UnitPlace::Size), None);
        assert_eq!(join_unit(".5", "mb", UnitPlace::Size), None);
        assert_eq!(join_unit("", "mb", UnitPlace::Size), None);
    }
}
