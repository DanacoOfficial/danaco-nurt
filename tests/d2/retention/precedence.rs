// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/retention/precedence.rs
// Responsibility:   Precedence of the retention settings over the seven configuration levels.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Precedence of the retention settings over the seven configuration levels,
//! for the step, age and size limits: every pair of level and dimension, with
//! a distinct value set at that level and at every lower level, yields the
//! value of that level and names it as the source.

use nurt_contracts::config::{AgeLimit, ConfigLevel, SizeLimit, StepLimit};
use nurt_contracts::journal::EffectiveRetention;
use serde_json::Value;

use crate::harness::fixture::{Fixture, FixtureOptions, Setting};

/// Retention dimension with its configuration key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Steps,
    Age,
    Size,
}

impl Dimension {
    const ALL: [Self; 3] = [Self::Steps, Self::Age, Self::Size];

    fn key(self) -> &'static str {
        match self {
            Self::Steps => "journal.max_ops",
            Self::Age => "journal.max_age",
            Self::Size => "journal.max_size",
        }
    }

    /// Value set at a level: distinct per level and above every floor.
    fn value(self, level: ConfigLevel) -> String {
        let rank = ConfigLevel::ALL
            .iter()
            .position(|candidate| *candidate == level)
            .expect("level is listed");
        match self {
            Self::Steps => format!("{}", 40 + rank),
            Self::Age => format!("{}m", 40 + rank),
            Self::Size => format!("{}gib", 21 + rank),
        }
    }

    /// Value in the form of `journal.status`.
    fn parsed(self, text: &str) -> Value {
        let value = match self {
            Self::Steps => {
                serde_json::to_value(text.parse::<StepLimit>().expect("step limit is valid"))
            }
            Self::Age => {
                serde_json::to_value(text.parse::<AgeLimit>().expect("age limit is valid"))
            }
            Self::Size => {
                serde_json::to_value(text.parse::<SizeLimit>().expect("size limit is valid"))
            }
        };
        value.expect("limit serializes")
    }

    fn effective(self, limits: &EffectiveRetention) -> (Value, ConfigLevel) {
        let (value, source) = match self {
            Self::Steps => (
                serde_json::to_value(limits.max_ops.value),
                limits.max_ops.source,
            ),
            Self::Age => (
                serde_json::to_value(limits.max_age.value),
                limits.max_age.source,
            ),
            Self::Size => (
                serde_json::to_value(limits.max_size.value),
                limits.max_size.source,
            ),
        };
        (value.expect("limit serializes"), source)
    }
}

/// One case: a dimension and the highest level that sets it.
#[derive(Debug, Clone, Copy)]
struct Case {
    dimension: Dimension,
    level: ConfigLevel,
}

fn cases() -> Vec<Case> {
    Dimension::ALL
        .iter()
        .flat_map(|dimension| {
            ConfigLevel::ALL.iter().map(|level| Case {
                dimension: *dimension,
                level: *level,
            })
        })
        .collect()
}

/// Settings of every level from the lowest written one up to the level of
/// the case; the built-in level is never written.
fn settings(case: Case) -> Vec<Setting> {
    ConfigLevel::ALL
        .iter()
        .take_while(|level| **level != case.level)
        .chain(std::iter::once(&case.level))
        .filter(|level| **level != ConfigLevel::Builtin)
        .map(|level| Setting {
            level: *level,
            key: case.dimension.key(),
            value: case.dimension.value(*level),
        })
        .collect()
}

/// Cases: seven levels for each of three dimensions.
const CASES: usize = 21;

#[test]
fn precedence_table_covers_every_level_and_dimension() {
    let all = cases();
    assert_eq!(all.len(), CASES);
    for case in &all {
        let written = settings(*case);
        // The built-in level is never written, so a case at the level of rank
        // `n` writes the levels of ranks 1 to `n`.
        let rank = ConfigLevel::ALL
            .iter()
            .position(|level| *level == case.level)
            .expect("level is listed");
        assert_eq!(
            written.len(),
            rank,
            "levels written for {} at {}",
            case.dimension.key(),
            case.level.as_str()
        );
        if let Some(top) = written.last() {
            assert_eq!(
                top.level, case.level,
                "the level of the case is the highest one written"
            );
        }
        let distinct: std::collections::BTreeSet<&String> =
            written.iter().map(|setting| &setting.value).collect();
        assert_eq!(
            distinct.len(),
            written.len(),
            "every level writes its own value"
        );
    }
}

#[test]
fn retention_settings_follow_configuration_level_precedence() {
    let builtin = FixtureOptions::default().journal;
    let mut failures = Vec::new();
    for case in cases() {
        let options = FixtureOptions {
            settings: settings(case),
            ..FixtureOptions::default()
        };
        let fixture = Fixture::open(options);
        let (value, source) = case.dimension.effective(&fixture.status().limits);
        let expected = if case.level == ConfigLevel::Builtin {
            match case.dimension {
                Dimension::Steps => serde_json::to_value(builtin.max_ops),
                Dimension::Age => serde_json::to_value(builtin.max_age),
                Dimension::Size => serde_json::to_value(builtin.max_size),
            }
            .expect("limit serializes")
        } else {
            case.dimension.parsed(&case.dimension.value(case.level))
        };
        if value != expected || source != case.level {
            failures.push(format!(
                "{} at {}: expected {expected} from {}, found {value} from {}",
                case.dimension.key(),
                case.level.as_str(),
                case.level.as_str(),
                source.as_str()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "precedence differs:\n{}",
        failures.join("\n")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
