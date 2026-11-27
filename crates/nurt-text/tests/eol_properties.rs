// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/eol_properties.rs
// Responsibility:   Properties of line terminator detection, conversion, preservation and.
// Dependencies:     nurt_contracts, proptest
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Properties of line terminator detection, conversion, preservation and
//! positions, 10 000 cases each with a fixed seed.

use nurt_contracts::services::LineEnding;
use nurt_text::eol::{self, Terminator};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

const CASES: u32 = 10_000;
const SEED: u64 = 0x6e75_7274_656f_6c73;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: CASES,
        rng_seed: RngSeed::Fixed(SEED),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Lines with a random terminator each, including none on the last line.
fn lines_with_terminators() -> impl Strategy<Value = Vec<(String, Option<Terminator>)>> {
    let terminator = prop::sample::select(vec![
        Some(Terminator::Lf),
        Some(Terminator::CrLf),
        Some(Terminator::Cr),
    ]);
    let content = "[a-zA-Z\u{105}\u{107}\u{119}\u{142}\u{144}\u{f3}\u{15b}\u{17a}\u{17c} ]{0,12}";
    (
        prop::collection::vec((content, terminator), 0..24),
        prop::option::of(content),
    )
        .prop_map(|(mut lines, last)| {
            if let Some(last) = last {
                lines.push((last, None));
            }
            lines
        })
}

fn join(lines: &[(String, Option<Terminator>)]) -> String {
    lines
        .iter()
        .map(|(content, terminator)| {
            format!("{content}{}", terminator.map_or("", Terminator::as_str))
        })
        .collect()
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn conversion_changes_only_terminators(lines in lines_with_terminators(), target in prop::sample::select(vec![Terminator::Lf, Terminator::CrLf, Terminator::Cr])) {
        // A `\r` line followed by an empty `\n` line reads back as one
        // `\r\n` line, so the expected lines come from the joined text.
        let text = join(&lines);
        let converted = eol::convert(&text, target);
        let expected: Vec<_> = eol::lines(&text)
            .map(|line| (line.content, line.terminator.map(|_| target)))
            .collect();
        let actual: Vec<_> = eol::lines(&converted)
            .map(|line| (line.content, line.terminator))
            .collect();
        prop_assert_eq!(actual, expected);
        let detected = eol::detect(converted.as_bytes());
        prop_assert!(detected == LineEnding::from(target) || detected == LineEnding::None);
    }

    #[test]
    fn lines_reassemble_the_text(lines in lines_with_terminators()) {
        let text = join(&lines);
        let parsed: Vec<(String, Option<Terminator>)> = eol::lines(&text)
            .map(|line| (line.content.to_owned(), line.terminator))
            .collect();
        prop_assert_eq!(join(&parsed), text);
    }

    #[test]
    fn preserve_restores_terminators_of_unchanged_lines(lines in lines_with_terminators()) {
        let original = join(&lines);
        let normalized = eol::convert(&original, Terminator::Lf);
        prop_assert_eq!(eol::preserve(&original, &normalized), original.as_str());
    }

    #[test]
    fn preserve_keeps_edited_content_and_terminator_presence(
        original in lines_with_terminators(),
        edited in lines_with_terminators(),
    ) {
        let original = join(&original);
        let edited = join(&edited);
        let preserved = eol::preserve(&original, &edited);
        let shape = |text: &str| -> Vec<(String, bool)> {
            eol::lines(text)
                .map(|line| (line.content.to_owned(), line.terminator.is_some()))
                .collect()
        };
        prop_assert_eq!(shape(&preserved), shape(&edited));
    }

    #[test]
    fn line_index_positions_round_trip(lines in lines_with_terminators()) {
        let text = join(&lines);
        let index = eol::LineIndex::new(&text);
        for (offset, _) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
            // The `\n` of a `\r\n` pair lies inside a terminator and has no
            // column of its own.
            let (before, after) = text.as_bytes().split_at(offset);
            if before.ends_with(b"\r") && after.starts_with(b"\n") {
                continue;
            }
            let position = index.position(offset).unwrap();
            prop_assert_eq!(index.offset(position), Some(offset));
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
