// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registry of the lexical correction rules.

use nurt_contracts::report::{CorrectionClass, TextPosition};

use crate::record::{CORRECTION_ID_PREFIX, Correction, CorrectionKind, RuleFamily, RuleId};

use CorrectionClass::{Certain, Fuzzy, Probable, Structural};

/// Description of a lexical correction rule, as listed by `help corrections`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleInfo {
    /// Rule identifier, `L01` to `L24`.
    pub id: &'static str,
    /// Certainty class of every correction made by the rule.
    pub class: CorrectionClass,
    /// One sentence describing the rule.
    pub summary: &'static str,
    /// Example of notation that the rule corrects.
    pub example_from: &'static str,
    /// The example after correction.
    pub example_to: &'static str,
}

/// Lexical correction rule.
///
/// The discriminant is the index of the rule in [`RULES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(usize)]
pub enum Rule {
    /// Misspelled command name.
    L01,
    /// Misspelled parameter name.
    L02,
    /// Letter case of a name.
    L03,
    /// Naming convention of a name.
    L04,
    /// Unambiguous prefix of a parameter or value name.
    L05,
    /// Synonym from the synonym table.
    L06,
    /// Single or typographic quotes.
    L07,
    /// Missing closing quote at the end of a line.
    L08,
    /// Extra quote at the end of a text.
    L09,
    /// Unclosed bracket at the end of the program.
    L10,
    /// Unclosed bracket before a pipe in call notation.
    L11,
    /// Extra closing bracket at the end of the program.
    L12,
    /// Pipe written in another notation.
    L13,
    /// Missing pipe between stages on one line.
    L14,
    /// Commas between arguments.
    L15,
    /// Colon or equals sign of a named argument.
    L16,
    /// White space that the language does not have.
    L17,
    /// Positional argument after a named argument.
    L18,
    /// Separators and drive letter of a path.
    L19,
    /// Notation of another syntax variant.
    L20,
    /// Unquoted absolute path.
    L21,
    /// Regular expression in a path place.
    L22,
    /// Relative path resolved against an explicit base.
    L23,
    /// Text with unknown escapes in a regular expression or path place.
    L24,
}

impl Rule {
    /// Every rule, in identifier order.
    pub const ALL: [Rule; 24] = [
        Rule::L01,
        Rule::L02,
        Rule::L03,
        Rule::L04,
        Rule::L05,
        Rule::L06,
        Rule::L07,
        Rule::L08,
        Rule::L09,
        Rule::L10,
        Rule::L11,
        Rule::L12,
        Rule::L13,
        Rule::L14,
        Rule::L15,
        Rule::L16,
        Rule::L17,
        Rule::L18,
        Rule::L19,
        Rule::L20,
        Rule::L21,
        Rule::L22,
        Rule::L23,
        Rule::L24,
    ];

    /// Returns the description of the rule.
    #[must_use]
    pub fn info(self) -> &'static RuleInfo {
        &RULES[self as usize]
    }

    /// Returns the identifier of the rule, for example `L13`.
    #[must_use]
    pub fn id(self) -> &'static str {
        self.info().id
    }

    /// Returns the certainty class of the rule.
    #[must_use]
    pub fn class(self) -> CorrectionClass {
        self.info().class
    }

    /// Returns the typed identifier of the rule.
    #[must_use]
    pub fn rule_id(self) -> RuleId {
        let number = u8::try_from(self as usize + 1).unwrap_or(u8::MAX);
        RuleId::new(RuleFamily::Lexical, number).unwrap_or(RuleId::L01)
    }

    /// Builds the record of one correction made by the rule.
    ///
    /// The identifier of the record is empty; lists of corrections are
    /// numbered by [`number_corrections`].
    #[must_use]
    pub fn record(
        self,
        at: TextPosition,
        from: impl Into<String>,
        to: impl Into<String>,
        kind: CorrectionKind,
    ) -> Correction {
        Correction::new(self.rule_id(), self.class(), from, to, kind).with_at(at)
    }
}

/// Numbers corrections `C1`, `C2`, … in list order.
pub fn number_corrections(corrections: &mut [Correction]) {
    for (index, correction) in corrections.iter_mut().enumerate() {
        correction.id = format!("{CORRECTION_ID_PREFIX}{}", index + 1);
    }
}

const fn rule(
    id: &'static str,
    class: CorrectionClass,
    summary: &'static str,
    example_from: &'static str,
    example_to: &'static str,
) -> RuleInfo {
    RuleInfo {
        id,
        class,
        summary,
        example_from,
        example_to,
    }
}

/// Lexical correction rules `L01` to `L24`, in identifier order.
pub static RULES: [RuleInfo; 24] = [
    rule(
        "L01",
        Fuzzy,
        "A misspelled command name is replaced by the only command within the edit distance threshold.",
        "remvoe \"/srv/app/build/old.log\"",
        "remove \"/srv/app/build/old.log\"",
    ),
    rule(
        "L02",
        Fuzzy,
        "A misspelled parameter name is replaced by the only parameter of the command within the edit distance threshold.",
        "find \"/srv\" depht: 2",
        "find \"/srv\" depth: 2",
    ),
    rule(
        "L03",
        Certain,
        "Names of commands, parameters, values, functions and keywords are written in lower case; literals keep their case.",
        "Find \"/srv/app\" Glob: \"*.go\"",
        "find \"/srv/app\" glob: \"*.go\"",
    ),
    rule(
        "L04",
        Certain,
        "Names written in kebab-case, camelCase or PascalCase are written in snake_case.",
        "find \"/srv\" caseSensitive: false",
        "find \"/srv\" case_sensitive: false",
    ),
    rule(
        "L05",
        Fuzzy,
        "An unambiguous prefix of at least three characters of a parameter or value name is completed; command names are never completed.",
        "find \"/srv\" type: dir",
        "find \"/srv\" type: directory",
    ),
    rule(
        "L06",
        Certain,
        "A name from the synonym table is replaced by the name it stands for.",
        "delete \"/srv/x\"",
        "remove \"/srv/x\"",
    ),
    rule(
        "L07",
        Certain,
        "Text in single or typographic quotes is written in double quotes, or as raw text when it contains a quote or a backslash; its characters are kept.",
        "replace 'newName('",
        "replace \"newName(\"",
    ),
    rule(
        "L08",
        Probable,
        "A text that is not closed on its line is closed at the end of the line.",
        "replace \"x\" \"newName(",
        "replace \"x\" \"newName(\"",
    ),
    rule(
        "L09",
        Probable,
        "An extra quote directly after a text is removed when the line then parses.",
        "find \"/srv\" glob: \"*.go\"\"",
        "find \"/srv\" glob: \"*.go\"",
    ),
    rule(
        "L10",
        Structural,
        "Brackets left open at the end of the program are closed in reverse order.",
        "verify (run [\"go\", \"build\"] cwd: \"/a\"",
        "verify (run [\"go\", \"build\"] cwd: \"/a\")",
    ),
    rule(
        "L11",
        Structural,
        "In call notation, a bracket left open before a pipe is closed before the pipe.",
        "find(\"/a\" |> count()",
        "find(\"/a\") |> count()",
    ),
    rule(
        "L12",
        Structural,
        "An extra closing bracket at the end of the program is removed.",
        "find \"/a\" |> count())",
        "find \"/a\" |> count()",
    ),
    rule(
        "L13",
        Certain,
        "A pipe written as `|`, `| >`, `|->` or `->` between stages is written as `|>`.",
        "find \"/a\" | search /x/",
        "find \"/a\" |> search /x/",
    ),
    rule(
        "L14",
        Structural,
        "A pipe is inserted before a command name that cannot be an argument of the preceding stage.",
        "find \"/a\" glob: \"*.go\" search /x/",
        "find \"/a\" glob: \"*.go\" |> search /x/",
    ),
    rule(
        "L15",
        Structural,
        "Commas between arguments are removed in word and named notation and inserted in call notation.",
        "find \"/a\", glob: \"*.go\"",
        "find \"/a\" glob: \"*.go\"",
    ),
    rule(
        "L16",
        Certain,
        "A named argument uses the colon in word and call notation and the equals sign in named notation.",
        "find \"/a\" glob=\"*.go\"",
        "find \"/a\" glob: \"*.go\"",
    ),
    rule(
        "L17",
        Certain,
        "No-break spaces, zero-width characters and other Unicode spaces outside literals are written as spaces or removed.",
        "find\u{a0}\"/a\"",
        "find \"/a\"",
    ),
    rule(
        "L18",
        Certain,
        "Positional arguments written after named arguments are moved before them, keeping their order.",
        "find glob: \"*.go\" \"/srv/app\"",
        "find \"/srv/app\" glob: \"*.go\"",
    ),
    rule(
        "L19",
        Certain,
        "A path uses `/` as separator, an upper-case drive letter, no doubled separators and no trailing separator; the letter case of its segments is kept.",
        "list \"C:\\\\proj\\\\src\\\\\"",
        "list \"C:/proj/src\"",
    ),
    rule(
        "L20",
        Certain,
        "A statement written in another syntax variant is written in the configured variant.",
        "find(\"/srv\", glob: \"*.go\")",
        "find \"/srv\" glob: \"*.go\"",
    ),
    rule(
        "L21",
        Certain,
        "An unquoted path is quoted.",
        "find /srv/app glob: \"*.go\"",
        "find \"/srv/app\" glob: \"*.go\"",
    ),
    rule(
        "L22",
        Structural,
        "A regular expression in a path place becomes the path written between its slashes.",
        "list /etc/",
        "list \"/etc\"",
    ),
    rule(
        "L23",
        Certain,
        "A relative path is resolved against the explicit base of the call.",
        "find \"./src\"",
        "find \"/srv/app/src\"",
    ),
    rule(
        "L24",
        Probable,
        "A text with unknown escapes in a regular expression or path place keeps its characters as a regular expression or path.",
        "search \"\\d+\" \"/srv\"",
        "search /\\d+/ \"/srv\"",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_entries_are_built_from_their_fields() {
        for info in &RULES {
            let built = rule(
                info.id,
                info.class,
                info.summary,
                info.example_from,
                info.example_to,
            );
            assert_eq!(
                (
                    built.id,
                    built.class,
                    built.summary,
                    built.example_from,
                    built.example_to
                ),
                (
                    info.id,
                    info.class,
                    info.summary,
                    info.example_from,
                    info.example_to
                )
            );
        }
    }

    #[test]
    fn rules_are_listed_in_identifier_order() {
        for (index, rule) in Rule::ALL.iter().enumerate() {
            assert_eq!(rule.id(), format!("L{:02}", index + 1));
            assert_eq!(*rule as usize, index);
        }
    }

    #[test]
    fn class_counts_match_the_rule_table() {
        let count = |class| RULES.iter().filter(|info| info.class == class).count();
        assert_eq!(count(Certain), 12);
        assert_eq!(count(Structural), 6);
        assert_eq!(count(Fuzzy), 3);
        assert_eq!(count(Probable), 3);
    }

    #[test]
    fn classes_follow_the_rule_table() {
        let expected = "FFCCFCCPPSSSCSSCCCCCCSCP";
        let actual: String = Rule::ALL.iter().map(|rule| rule.class().as_str()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn records_are_numbered_in_order() {
        let at = TextPosition {
            line: 1,
            column: 1,
            length: Some(1),
        };
        let mut corrections = vec![
            Rule::L13.record(at, "|", "|>", CorrectionKind::Pipe),
            Rule::L07.record(at, "'a'", "\"a\"", CorrectionKind::Quote),
        ];
        number_corrections(&mut corrections);
        assert_eq!(corrections[0].id, "C1");
        assert_eq!(corrections[1].id, "C2");
        assert_eq!(corrections[1].class, Certain);
        assert_eq!(corrections[0].rule.to_string(), "L13");
        assert!(
            Rule::ALL
                .iter()
                .all(|rule| rule.rule_id().to_string() == rule.id())
        );
    }
}
