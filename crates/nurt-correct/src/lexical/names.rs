// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Correction of names against the names expected in their place.
//!
//! [`match_name`] compares a written name with the set of names the binder
//! expects in its place (commands, parameters of one command, values of one
//! parameter, fields, functions, `let` names), together with the synonyms of
//! the synonym table and the habit words of other shells. The comparison
//! follows the acceptance rules of [`crate::distance::nearest`]: letter case
//! and naming convention first (`L03`, `L04`), then the synonym table
//! (`L06`), then a unique candidate within the edit distance threshold
//! (`L01` for commands, `L02` for parameters) and an unambiguous prefix of a
//! parameter or value (`L05`). A tie is never corrected.
//!
//! Fuzzy matches of values, fields, functions and `let` names belong to the
//! value rules `V01` to `V04` and are reported with those identifiers, as are
//! the synonyms of those kinds in the synonym table.

use nurt_contracts::report::CorrectionClass;

use super::rules::Rule;
use crate::distance::{EXPECTED_LIMIT, NameIndex, Nearest, Options, expected, nearest};
use crate::record::RuleId;

pub use crate::distance::NameKind;

/// Result of matching a written name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameMatch {
    /// The name is written exactly as expected.
    Exact,
    /// The name is corrected.
    Corrected {
        /// Name after correction.
        to: String,
        /// Rule of the correction, for example `L01` or `V01`.
        rule: RuleId,
        /// Certainty class of the correction.
        class: CorrectionClass,
        /// The corrected name is a command that changes something or runs a
        /// program; the binder applies the correction only when the whole
        /// corrected program binds and type-checks.
        requires_binding: bool,
    },
    /// Several names are equally close; nothing is corrected.
    Tie {
        /// The competing names, sorted.
        candidates: Vec<String>,
    },
    /// No name is close enough.
    Unknown {
        /// Up to eight nearest names, for the `expected` member of the error.
        nearest: Vec<String>,
    },
}

/// Matches a written name against the names of one kind.
///
/// The index holds the names expected in the place, their synonyms and the
/// habit words; the binder builds it from the catalog and the correction
/// tables. Commands are never completed from a prefix, and a fuzzy match of
/// a changing command is accepted only at distance 1 and with
/// `requires_binding` set.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::{NameEntry, NameIndex, Options};
/// use nurt_correct::lexical::names::{NameKind, NameMatch, match_name};
/// use nurt_correct::record::RuleId;
///
/// let index = NameIndex::new([
///     NameEntry::command("remove", true),
///     NameEntry::command("jobs", false),
///     NameEntry::synonym("work", NameKind::Command, "jobs"),
/// ]);
/// let options = Options::default();
/// let corrected = match_name("remvoe", NameKind::Command, &index, &options);
/// assert!(matches!(
///     corrected,
///     NameMatch::Corrected { rule: RuleId::L01, requires_binding: true, .. }
/// ));
/// let synonym = match_name("work", NameKind::Command, &index, &options);
/// assert!(matches!(synonym, NameMatch::Corrected { rule: RuleId::L06, .. }));
/// ```
#[must_use]
pub fn match_name(
    written: &str,
    kind: NameKind,
    index: &NameIndex,
    options: &Options,
) -> NameMatch {
    match nearest(written, index, kind, options) {
        Nearest::Exact { to } if to == written => NameMatch::Exact,
        Nearest::Exact { to } => {
            let rule = if to == written.to_ascii_lowercase() {
                Rule::L03
            } else {
                Rule::L04
            };
            corrected(to, rule.rule_id(), rule.class(), false)
        }
        Nearest::Synonym { to } => {
            corrected(to, synonym_rule(kind), CorrectionClass::Certain, false)
        }
        Nearest::Unique { to, effecting, .. } => {
            corrected(to, fuzzy_rule(kind), CorrectionClass::Fuzzy, effecting)
        }
        Nearest::Prefix { to } => corrected(to, Rule::L05.rule_id(), Rule::L05.class(), false),
        Nearest::Tie { names } => NameMatch::Tie { candidates: names },
        Nearest::None => NameMatch::Unknown {
            nearest: expected(written, index, kind, EXPECTED_LIMIT),
        },
    }
}

fn corrected(
    to: String,
    rule: RuleId,
    class: CorrectionClass,
    requires_binding: bool,
) -> NameMatch {
    NameMatch::Corrected {
        to,
        rule,
        class,
        requires_binding,
    }
}

/// Rule of a fuzzy match by kind of name.
fn fuzzy_rule(kind: NameKind) -> RuleId {
    match kind {
        NameKind::Command | NameKind::Group => RuleId::L01,
        NameKind::Param => RuleId::L02,
        NameKind::Value => RuleId::V01,
        NameKind::Field => RuleId::V02,
        NameKind::Function => RuleId::V03,
        NameKind::Let => RuleId::V04,
    }
}

/// Rule of a synonym by kind of name.
fn synonym_rule(kind: NameKind) -> RuleId {
    match kind {
        NameKind::Command | NameKind::Group | NameKind::Param => RuleId::L06,
        NameKind::Value => RuleId::V01,
        NameKind::Field => RuleId::V02,
        NameKind::Function => RuleId::V03,
        NameKind::Let => RuleId::V04,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distance::NameEntry;

    fn commands() -> NameIndex {
        let mut entries: Vec<NameEntry> = [
            ("find", false),
            ("search", false),
            ("list", false),
            ("last", false),
            ("jobs", false),
            ("remove", true),
            ("move", true),
            ("run", true),
            ("program.find", false),
        ]
        .into_iter()
        .map(|(name, effecting)| NameEntry::command(name, effecting))
        .collect();
        for (synonym, to) in [
            ("work", "jobs"),
            ("delete", "remove"),
            ("del", "remove"),
            ("erase", "remove"),
            ("dir", "list"),
            ("which", "program.find"),
        ] {
            entries.push(NameEntry::synonym(synonym, NameKind::Command, to));
        }
        entries.extend(["grep", "cat", "sed"].map(NameEntry::habit));
        NameIndex::new(entries)
    }

    fn params() -> NameIndex {
        let mut entries: Vec<NameEntry> = [
            "case_sensitive",
            "depth",
            "glob",
            "type",
            "hidden",
            "on_conflict",
        ]
        .map(|name| NameEntry::name(name, NameKind::Param))
        .into_iter()
        .collect();
        entries.push(NameEntry::synonym("max_depth", NameKind::Param, "depth"));
        NameIndex::new(entries)
    }

    fn rule_of(result: &NameMatch) -> Option<(String, String)> {
        match result {
            NameMatch::Corrected { to, rule, .. } => Some((to.clone(), rule.to_string())),
            _ => None,
        }
    }

    fn pair(to: &str, rule: &str) -> Option<(String, String)> {
        Some((to.to_owned(), rule.to_owned()))
    }

    fn command(written: &str) -> NameMatch {
        match_name(written, NameKind::Command, &commands(), &Options::default())
    }

    fn param(written: &str) -> NameMatch {
        match_name(written, NameKind::Param, &params(), &Options::default())
    }

    #[test]
    fn corrects_misspelled_commands_with_a_unique_candidate() {
        assert_eq!(rule_of(&command("fnid")), pair("find", "L01"));
        assert_eq!(rule_of(&command("serach")), pair("search", "L01"));
        assert!(matches!(
            command("remvoe"),
            NameMatch::Corrected {
                rule: RuleId::L01,
                class: CorrectionClass::Fuzzy,
                requires_binding: true,
                ..
            }
        ));
    }

    #[test]
    fn replaces_synonyms_including_work_by_jobs() {
        assert_eq!(rule_of(&command("work")), pair("jobs", "L06"));
        for synonym in ["delete", "del", "erase"] {
            assert_eq!(
                rule_of(&command(synonym)),
                pair("remove", "L06"),
                "{synonym}"
            );
        }
        assert_eq!(rule_of(&command("which")), pair("program.find", "L06"));
        assert_eq!(rule_of(&param("max_depth")), pair("depth", "L06"));
    }

    #[test]
    fn keeps_a_tie_between_list_and_last_uncorrected() {
        assert!(
            matches!(command("lest"), NameMatch::Tie { candidates } if candidates == ["last", "list"])
        );
    }

    #[test]
    fn writes_case_and_convention_as_letter_case_rules() {
        assert_eq!(rule_of(&command("Find")), pair("find", "L03"));
        assert_eq!(
            rule_of(&param("caseSensitive")),
            pair("case_sensitive", "L04")
        );
        assert_eq!(command("find"), NameMatch::Exact);
    }

    #[test]
    fn corrects_parameters_by_distance_and_prefix() {
        assert_eq!(
            rule_of(&param("case_sensitve")),
            pair("case_sensitive", "L02")
        );
        assert_eq!(rule_of(&param("depht")), pair("depth", "L02"));
        assert_eq!(rule_of(&param("on_conf")), pair("on_conflict", "L05"));
    }

    #[test]
    fn never_completes_a_command_from_a_prefix() {
        assert!(!matches!(command("sea"), NameMatch::Corrected { .. }));
        assert!(!matches!(command("rem"), NameMatch::Corrected { .. }));
    }

    #[test]
    fn short_names_are_corrected_only_through_the_table() {
        assert!(!matches!(command("rnu"), NameMatch::Corrected { .. }));
        assert!(!matches!(command("ru"), NameMatch::Corrected { .. }));
    }

    #[test]
    fn unknown_names_list_the_nearest_names() {
        let result = command("xyzzy");
        assert!(matches!(
            &result,
            NameMatch::Unknown { nearest } if !nearest.is_empty() && nearest.len() <= EXPECTED_LIMIT
        ));
    }

    #[test]
    fn fuzzy_and_synonym_rules_follow_the_kind_of_name() {
        assert_eq!(fuzzy_rule(NameKind::Value), RuleId::V01);
        assert_eq!(fuzzy_rule(NameKind::Let), RuleId::V04);
        assert_eq!(synonym_rule(NameKind::Field), RuleId::V02);
        assert_eq!(synonym_rule(NameKind::Function), RuleId::V03);
        let values = NameIndex::new([
            NameEntry::name("directory", NameKind::Value),
            NameEntry::synonym("dir", NameKind::Value, "directory"),
        ]);
        let result = match_name("dir", NameKind::Value, &values, &Options::default());
        assert_eq!(rule_of(&result), pair("directory", "V01"));
    }

    #[test]
    fn first_character_rule_follows_the_configuration() {
        let index = commands();
        assert!(!matches!(
            match_name("dind", NameKind::Command, &index, &Options::default()),
            NameMatch::Corrected { .. }
        ));
        let relaxed = Options {
            require_first_char: false,
        };
        assert_eq!(
            rule_of(&match_name("dind", NameKind::Command, &index, &relaxed)),
            pair("find", "L01")
        );
    }
}
