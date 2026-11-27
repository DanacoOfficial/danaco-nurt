// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::values
// Responsibility:   Correction of names in typed places: commands, parameters, enumeration.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Correction of names in typed places: commands, parameters, enumeration
//! values (`V01`), record fields (`V02`), expression functions (`V03`) and
//! references to `let` names (`V04`).
//!
//! Every function takes the set of names valid in the place as data: the
//! caller derives it from the catalog, the input record or the program. The
//! same written word can be a synonym of different names in different kinds
//! of places (`length` is the field `size` and the function `len`); the kind
//! of the place decides.

use crate::distance::{NameEntry, NameIndex, NameKind, Nearest, Options, nearest};
use crate::policy::Class;
use crate::record::{Correction, CorrectionKind, RuleId};
use crate::tables::Tables;

/// Functions of the expression language, used when the caller has no other
/// list.
pub const FUNCTIONS: [&str; 26] = [
    "lower",
    "upper",
    "trim",
    "len",
    "abs",
    "round",
    "min",
    "max",
    "contains",
    "starts_with",
    "ends_with",
    "replace_text",
    "regex",
    "time",
    "now",
    "ago",
    "from_now",
    "parent",
    "join_path",
    "base64_encode",
    "base64_decode",
    "idna_encode",
    "idna_decode",
    "hex",
    "uuid",
    "random",
];

/// A name correction found by [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCorrection {
    /// Canonical name.
    pub to: String,
    /// Rule that applies.
    pub rule: RuleId,
    /// Certainty class.
    pub class: Class,
    /// Kind of the corrected element.
    pub kind: CorrectionKind,
    /// Edit distance after folding; 0 for notation and synonyms.
    pub distance: usize,
    /// The name is a command that changes the world or runs a program.
    pub effecting: bool,
}

impl NameCorrection {
    /// The correction record of this correction of the written text `from`.
    #[must_use]
    pub fn correction(&self, from: &str) -> Correction {
        Correction::new(self.rule, self.class, from, self.to.clone(), self.kind)
    }
}

/// Result of resolving a written name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// The name is written exactly as the canonical name.
    Same,
    /// The name is corrected.
    Corrected(NameCorrection),
    /// Two or more names are equally close; nothing is corrected.
    Tie(Vec<String>),
    /// No name is close enough.
    Unknown,
}

/// Builds the comparison set of a place: the given names, the synonyms of
/// the same kind whose target is among them, and, for commands, the habit
/// words.
#[must_use]
pub fn index(
    kind: NameKind,
    names: impl IntoIterator<Item = NameEntry>,
    tables: &Tables,
) -> NameIndex {
    let mut index = NameIndex::new(names);
    let targets: Vec<String> = index.entries().map(|entry| entry.text.clone()).collect();
    for entry in tables.name_entries() {
        let relevant = match &entry.role {
            crate::distance::NameRole::Synonym { to } => entry.kind == kind && targets.contains(to),
            crate::distance::NameRole::Habit => kind == NameKind::Command,
            crate::distance::NameRole::Name => false,
        };
        if relevant {
            index.insert(entry);
        }
    }
    index
}

/// Resolves the written name `written` in a place of `kind`.
///
/// Notation differences give `L03` (case) or `L04` (naming convention),
/// synonyms give the rule of their table entry, fuzzy matches give `L01`
/// (command), `L02` (parameter), `V01` (value), `V02` (field), `V03`
/// (function) or `V04` (`let` name), and prefixes give `L05`.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::{NameKind, Options};
/// use nurt_correct::tables::load_embedded;
/// use nurt_correct::values::{Match, resolve_in};
///
/// let tables = load_embedded().unwrap();
/// let options = Options::default();
/// let Match::Corrected(found) = resolve_in("dir", NameKind::Value, &["file", "directory", "link", "any"], &tables, &options) else {
///     panic!("expected a correction");
/// };
/// assert_eq!((found.to.as_str(), found.rule.to_string()), ("directory", "V01".to_owned()));
/// ```
#[must_use]
pub fn resolve(
    written: &str,
    kind: NameKind,
    index: &NameIndex,
    tables: &Tables,
    options: &Options,
) -> Match {
    let correction_kind = correction_kind(kind);
    match nearest(written, index, kind, options) {
        Nearest::Exact { to } if to == written => Match::Same,
        Nearest::Exact { to } => {
            let rule = if written.to_lowercase() == to.to_lowercase() {
                RuleId::L03
            } else {
                RuleId::L04
            };
            Match::Corrected(NameCorrection {
                to,
                rule,
                class: Class::Certain,
                kind: correction_kind,
                distance: 0,
                effecting: false,
            })
        }
        Nearest::Synonym { to } => {
            let entry = tables.synonym_entry(kind, written);
            Match::Corrected(NameCorrection {
                to,
                rule: entry.map_or(RuleId::L06, |entry| entry.rule),
                class: entry.map_or(Class::Certain, |entry| entry.class),
                kind: correction_kind,
                distance: 0,
                effecting: false,
            })
        }
        Nearest::Unique {
            to,
            distance,
            effecting,
        } => Match::Corrected(NameCorrection {
            to,
            rule: fuzzy_rule(kind),
            class: Class::Fuzzy,
            kind: correction_kind,
            distance,
            effecting,
        }),
        Nearest::Prefix { to } => Match::Corrected(NameCorrection {
            to,
            rule: RuleId::L05,
            class: Class::Fuzzy,
            kind: correction_kind,
            distance: 0,
            effecting: false,
        }),
        Nearest::Tie { names } => Match::Tie(names),
        Nearest::None => Match::Unknown,
    }
}

/// Resolves `written` among `names` of `kind`, with the synonyms of the
/// tables.
#[must_use]
pub fn resolve_in(
    written: &str,
    kind: NameKind,
    names: &[&str],
    tables: &Tables,
    options: &Options,
) -> Match {
    let index = index(
        kind,
        names.iter().map(|name| NameEntry::name(*name, kind)),
        tables,
    );
    resolve(written, kind, &index, tables, options)
}

/// `V01`: resolves an enumeration value among the values of one parameter.
#[must_use]
pub fn value(written: &str, values: &[&str], tables: &Tables, options: &Options) -> Match {
    resolve_in(written, NameKind::Value, values, tables, options)
}

/// `V01` for a parameter that selects the mode of an effect (`mode`,
/// `on_conflict`, `recursive`, `type` of `create`): like [`value`], but a
/// prefix never selects a mode.
#[must_use]
pub fn mode_value(written: &str, values: &[&str], tables: &Tables, options: &Options) -> Match {
    match value(written, values, tables, options) {
        Match::Corrected(found) if found.rule == RuleId::L05 => Match::Unknown,
        other => other,
    }
}

/// `V02`: resolves a field among the fields of the input record.
#[must_use]
pub fn field(written: &str, fields: &[&str], tables: &Tables, options: &Options) -> Match {
    resolve_in(written, NameKind::Field, fields, tables, options)
}

/// `V03`: resolves an expression function.
#[must_use]
pub fn function(written: &str, functions: &[&str], tables: &Tables, options: &Options) -> Match {
    resolve_in(written, NameKind::Function, functions, tables, options)
}

/// `V04`: resolves a reference `@x` among the `let` names of the program,
/// at distance 1 or less whatever the length of the name.
#[must_use]
pub fn let_name(written: &str, lets: &[&str], options: &Options) -> Match {
    let index = NameIndex::new(
        lets.iter()
            .map(|name| NameEntry::name(*name, NameKind::Let)),
    );
    let corrected = |to, rule, class, distance| {
        Match::Corrected(NameCorrection {
            to,
            rule,
            class,
            kind: CorrectionKind::Ref,
            distance,
            effecting: false,
        })
    };
    match nearest(written, &index, NameKind::Let, options) {
        Nearest::Exact { to } if to == written => Match::Same,
        Nearest::Exact { to } => corrected(to, RuleId::L03, Class::Certain, 0),
        Nearest::Unique { to, distance, .. } => corrected(to, RuleId::V04, Class::Fuzzy, distance),
        Nearest::Tie { names } => Match::Tie(names),
        _ => Match::Unknown,
    }
}

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

fn correction_kind(kind: NameKind) -> CorrectionKind {
    match kind {
        NameKind::Command | NameKind::Group => CorrectionKind::Command,
        NameKind::Param => CorrectionKind::Param,
        NameKind::Value => CorrectionKind::Value,
        NameKind::Field => CorrectionKind::Field,
        NameKind::Function => CorrectionKind::Function,
        NameKind::Let => CorrectionKind::Ref,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::load_embedded;

    const ENTRY_FIELDS: [&str; 9] = [
        "path",
        "name",
        "stem",
        "extension",
        "parent",
        "type",
        "size",
        "modified",
        "created",
    ];
    const MODES: [&str; 4] = ["overwrite", "append", "prepend", "create"];
    const SIGNALS: [&str; 4] = ["hangup", "interrupt", "kill", "terminate"];

    fn corrected(found: Match) -> (String, String, Class) {
        match found {
            Match::Corrected(correction) => {
                (correction.to, correction.rule.to_string(), correction.class)
            }
            other => panic!("expected a correction, found {other:?}"),
        }
    }

    #[test]
    fn corrects_enumeration_values() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        let types = ["file", "directory", "link", "any"];
        assert_eq!(
            corrected(value("dir", &types, &tables, &options)),
            ("directory".into(), "V01".into(), Class::Certain)
        );
        assert_eq!(
            corrected(value("File", &types, &tables, &options)),
            ("file".into(), "L03".into(), Class::Certain)
        );
        assert_eq!(
            corrected(value("directry", &types, &tables, &options)),
            ("directory".into(), "V01".into(), Class::Fuzzy)
        );
        assert_eq!(
            corrected(value("direc", &types, &tables, &options)),
            ("directory".into(), "L05".into(), Class::Fuzzy)
        );
        assert_eq!(
            corrected(value("ascending", &["asc", "desc"], &tables, &options)),
            ("asc".into(), "V01".into(), Class::Certain)
        );
        assert_eq!(
            corrected(value("add", &MODES, &tables, &options)),
            ("append".into(), "T09".into(), Class::Certain)
        );
        assert_eq!(
            corrected(value("truncate", &MODES, &tables, &options)),
            ("overwrite".into(), "T09".into(), Class::Certain)
        );
        assert_eq!(
            corrected(value("term", &SIGNALS, &tables, &options)),
            ("terminate".into(), "V01".into(), Class::Certain)
        );
        assert_eq!(value("file", &types, &tables, &options), Match::Same);
        assert_eq!(value("socket", &types, &tables, &options), Match::Unknown);
    }

    #[test]
    fn ignores_synonyms_whose_target_is_not_allowed() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        assert_eq!(
            value("add", &["merge", "rebase"], &tables, &options),
            Match::Unknown
        );
    }

    #[test]
    fn mode_values_never_come_from_a_prefix() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        assert_eq!(mode_value("app", &MODES, &tables, &options), Match::Unknown);
        assert_eq!(mode_value("append", &MODES, &tables, &options), Match::Same);
        assert_eq!(
            corrected(mode_value("truncate", &MODES, &tables, &options)),
            ("overwrite".into(), "T09".into(), Class::Certain)
        );
    }

    #[test]
    fn resolves_names_with_multibyte_characters() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        let names = ["żółw", "żółć"];
        assert_eq!(
            resolve_in("ŻÓŁW", NameKind::Field, &names, &tables, &options),
            Match::Corrected(NameCorrection {
                to: "żółw".to_owned(),
                rule: RuleId::L03,
                class: Class::Certain,
                kind: CorrectionKind::Field,
                distance: 0,
                effecting: false,
            })
        );
        assert_eq!(
            resolve_in("żółx", NameKind::Field, &names, &tables, &options),
            Match::Tie(vec!["żółw".to_owned(), "żółć".to_owned()])
        );
        assert_eq!(
            resolve_in("", NameKind::Field, &names, &tables, &options),
            Match::Unknown
        );
    }

    #[test]
    fn corrects_record_fields() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        for (written, expected) in [
            ("mtime", "modified"),
            ("modtime", "modified"),
            ("LastWriteTime", "modified"),
            ("filename", "name"),
            ("bytes", "size"),
            ("length", "size"),
            ("ext", "extension"),
            ("kind", "type"),
        ] {
            let (to, rule, class) = corrected(field(written, &ENTRY_FIELDS, &tables, &options));
            assert_eq!(
                (to.as_str(), rule.as_str(), class),
                (expected, "V02", Class::Certain),
                "{written}"
            );
        }
        assert_eq!(
            corrected(field("modifed", &ENTRY_FIELDS, &tables, &options)),
            ("modified".into(), "V02".into(), Class::Fuzzy)
        );
        assert_eq!(
            corrected(field(
                "line_number",
                &["path", "no", "text"],
                &tables,
                &options
            ))
            .0,
            "no"
        );
    }

    #[test]
    fn corrects_functions_by_the_kind_of_place() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        assert_eq!(
            corrected(function("lowercase", &FUNCTIONS, &tables, &options)),
            ("lower".into(), "V03".into(), Class::Certain)
        );
        assert_eq!(
            corrected(function("length", &FUNCTIONS, &tables, &options)).0,
            "len"
        );
        assert_eq!(
            corrected(function("startsWith", &FUNCTIONS, &tables, &options)),
            ("starts_with".into(), "L04".into(), Class::Certain)
        );
        assert_eq!(
            corrected(function("contans", &FUNCTIONS, &tables, &options)),
            ("contains".into(), "V03".into(), Class::Fuzzy)
        );
    }

    #[test]
    fn corrects_let_names_at_distance_one() {
        let options = Options::default();
        let found = let_name("chnaged", &["changed", "files"], &options);
        let Match::Corrected(correction) = found else {
            panic!("expected a correction");
        };
        assert_eq!(
            (correction.to.as_str(), correction.kind),
            ("changed", CorrectionKind::Ref)
        );
        assert_eq!(
            correction.correction("@chnaged").to_string(),
            "V04  @chnaged → changed  ref"
        );
        assert_eq!(let_name("changed", &["changed"], &options), Match::Same);
        assert_eq!(let_name("chngd", &["changed"], &options), Match::Unknown);
        assert_eq!(
            let_name(
                "ab",
                &["aa", "bb"],
                &Options {
                    require_first_char: false
                }
            ),
            Match::Tie(vec!["aa".into(), "bb".into()])
        );
        assert!(matches!(
            let_name("Changed", &["changed"], &options),
            Match::Corrected(_)
        ));
    }

    #[test]
    fn resolves_commands_with_synonyms_and_habits() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        let names = [
            "remove", "list", "last", "read", "first", "jobs", "set", "find",
        ];
        let index = index(
            NameKind::Command,
            names.map(|name| NameEntry::command(name, name == "remove" || name == "set")),
            &tables,
        );
        let resolve_command =
            |written: &str| resolve(written, NameKind::Command, &index, &tables, &options);
        assert_eq!(
            corrected(resolve_command("work")),
            ("jobs".into(), "L06".into(), Class::Certain)
        );
        let Match::Corrected(remove) = resolve_command("remvoe") else {
            panic!("expected a correction");
        };
        assert_eq!(
            (remove.rule, remove.class, remove.effecting),
            (RuleId::L01, Class::Fuzzy, true)
        );
        assert_eq!(
            resolve_command("lest"),
            Match::Tie(vec!["last".into(), "list".into(), "test".into()])
        );
        assert_eq!(resolve_command("find"), Match::Same);
        assert_eq!(
            corrected(resolve_command("Read")),
            ("read".into(), "L03".into(), Class::Certain)
        );
        assert_eq!(resolve_command("rnu"), Match::Unknown);
    }

    #[test]
    fn corrects_parameter_names() {
        let tables = load_embedded().unwrap();
        let options = Options::default();
        let params = [
            "path",
            "glob",
            "type",
            "depth",
            "case_sensitive",
            "on_conflict",
        ];
        assert_eq!(
            corrected(resolve_in(
                "case-sensitive",
                NameKind::Param,
                &params,
                &tables,
                &options
            )),
            ("case_sensitive".into(), "L04".into(), Class::Certain)
        );
        assert_eq!(
            corrected(resolve_in(
                "depht",
                NameKind::Param,
                &params,
                &tables,
                &options
            )),
            ("depth".into(), "L02".into(), Class::Fuzzy)
        );
        assert_eq!(
            corrected(resolve_in(
                "on_conf",
                NameKind::Param,
                &params,
                &tables,
                &options
            )),
            ("on_conflict".into(), "L05".into(), Class::Fuzzy)
        );
        assert_eq!(
            corrected(resolve_in(
                "max_depth",
                NameKind::Param,
                &params,
                &tables,
                &options
            )),
            ("depth".into(), "L06".into(), Class::Certain)
        );
        assert_eq!(
            corrected(resolve_in(
                "Glob",
                NameKind::Param,
                &params,
                &tables,
                &options
            )),
            ("glob".into(), "L03".into(), Class::Certain)
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
