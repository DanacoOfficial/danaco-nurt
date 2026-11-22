// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface of the bash translation: detection, the POSIX lexer, script
//! mode, assumptions and the translation table as data.

use nurt_contracts::error::ErrorCode;
use nurt_correct::bash::{
    Assumption, BashTable, Mode, Operator, PosixTokenKind, RedirectOp, TranslateContext, detect,
    lex, translate, translate_with, translate_with_table,
};

const BASH_TABLE: &str = include_str!("../tables/bash.toml");
const HABITS_TABLE: &str = include_str!("../tables/habits_bash.toml");

/// Script mode without `base` and without known `let` names.
const SCRIPT: TranslateContext<'static> = TranslateContext {
    base: None,
    lets: &[],
    mode: Mode::Script,
};

#[test]
fn detection_separates_habits_from_nurt_programs() {
    assert!(detect("find /a -name \"*.go\""));
    assert!(detect("cd /srv/app && make"));
    assert!(detect("awk '{print $1}' /a"));
    assert!(!detect("find \"/a\" glob: \"*.go\""));
    assert!(!detect("cat /a/b"));
    assert!(!detect("make && make install"));
    assert!(!detect("echo 'unterminated"));
}

#[test]
fn lexer_reports_operators_redirections_and_here_documents() {
    let tokens = lex("cat <<'EOF' > /a/f && ls\nx\nEOF\n").expect("valid input");
    let here_doc = tokens.iter().find_map(|token| match &token.kind {
        PosixTokenKind::Redirect(redirect) if redirect.op == RedirectOp::HereDoc => {
            redirect.here_doc.clone()
        }
        _ => None,
    });
    let here_doc = here_doc.expect("a here-document");
    assert_eq!(here_doc.body, "x\n");
    assert!(here_doc.quoted);
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == PosixTokenKind::Operator(Operator::And))
    );
    assert!(lex("echo \"open").is_err());
}

#[test]
fn live_translation_ignores_nurt_programs_and_hides_errors() {
    assert!(translate("remove \"/srv/app/build\"").is_none());
    assert!(translate("sudo rm -f /srv/x").is_none());
    let error = translate_with("sudo rm -f /srv/x", &TranslateContext::default())
        .expect("a bash habit")
        .expect_err("sudo is never translated");
    assert_eq!(error.code(), ErrorCode::Uncorrectable);
}

#[test]
fn script_mode_translates_every_statement_and_names_the_stop_rule() {
    let input = "mkdir -p /srv/out\ncp -a /srv/in/a.csv /srv/out/";
    let translation = translate_with(input, &SCRIPT)
        .expect("a script")
        .expect("a translation");
    assert_eq!(
        translation.text(),
        "create \"/srv/out\" type: directory\ncopy \"/srv/in/a.csv\" into: \"/srv/out\""
    );
    assert_eq!(translation.assumptions.len(), 1);
    assert_eq!(translation.assumptions[0].rule, "B22");
}

#[test]
fn script_mode_keeps_the_directory_of_cd_for_later_lines() {
    let input = "cd /srv/app\nmake\nmake install";
    let translation = translate_with(input, &SCRIPT)
        .expect("a script")
        .expect("a translation");
    assert_eq!(
        translation.text(),
        "run [\"make\"] cwd: \"/srv/app\"\nrun [\"make\", \"install\"] cwd: \"/srv/app\""
    );
}

#[test]
fn script_mode_applies_probable_translations_with_alternatives() {
    let input = "sed -i 's/a+b/c/' /srv/x.txt";
    let translation = translate_with(input, &SCRIPT)
        .expect("a script")
        .expect("a translation");
    assert_eq!(
        translation.text(),
        "replace \"a+b\" \"c\" in: [\"/srv/x.txt\"] occurrence: 1"
    );
    assert_eq!(translation.alternatives.len(), 2);
    assert_eq!(translation.rules[0].alternatives.len(), 2);
}

#[test]
fn working_directory_is_offered_from_base_and_never_chosen() {
    let context = TranslateContext {
        base: Some("/srv/app"),
        lets: &[],
        mode: Mode::Script,
    };
    let error = translate_with("go build ./...", &context)
        .expect("a script")
        .expect_err("no working directory");
    assert_eq!(error.code(), ErrorCode::Uncorrectable);
    assert_eq!(
        error.data().fix,
        vec!["run [\"go\", \"build\", \"./...\"] cwd: \"/srv/app\"".to_owned()]
    );
}

#[test]
fn probable_translations_name_their_assumption() {
    let translation = translate("grep -rl \"oldName(\" /srv/app/src").expect("a translation");
    assert_eq!(
        translation.assumptions,
        vec![Assumption {
            rule: "B15".to_owned(),
            text: "the pattern is read as literal text".to_owned()
        }]
    );
}

#[test]
fn flag_mappings_are_data_of_the_table() {
    let original = "{ flags = [\"-i\", \"--ignore-case\"], args = \"case_sensitive: false\" }";
    assert!(BASH_TABLE.contains(original));
    let changed = BASH_TABLE.replace(
        original,
        "{ flags = [\"-i\", \"--ignore-case\"], args = \"whole_word: true\" }",
    );
    let table = BashTable::parse(&changed, HABITS_TABLE).expect("a valid table");
    let context = TranslateContext::default();
    let input = "grep -i error /srv/a.log";
    let embedded = translate_with(input, &context)
        .expect("a habit")
        .expect("a translation");
    let modified = translate_with_table(input, &context, &table)
        .expect("a habit")
        .expect("a translation");
    assert_eq!(
        embedded.text(),
        "search /error/ \"/srv/a.log\" case_sensitive: false"
    );
    assert_eq!(
        modified.text(),
        "search /error/ \"/srv/a.log\" whole_word: true"
    );
}

#[test]
fn habit_words_are_data_of_the_table() {
    let habits = HABITS_TABLE.replacen("\"sleep\", ", "", 1);
    let table = BashTable::parse(BASH_TABLE, &habits).expect("a valid table");
    let context = TranslateContext::default();
    assert!(translate_with_table("sleep 10 &", &context, &table).is_none());
    assert!(translate_with("sleep 10 &", &context).is_some());
}

#[test]
fn invalid_tables_are_rejected() {
    assert!(
        BashTable::parse(
            "format = \"other\"\ntable = \"bash\"\nsummary = \"s\"\n",
            HABITS_TABLE
        )
        .is_err()
    );
    let unknown = format!("unknown_key = 1\n{BASH_TABLE}");
    assert!(BashTable::parse(&unknown, HABITS_TABLE).is_err());
}

#[test]
fn the_shared_loader_reads_the_bash_tables() {
    let tables = nurt_correct::tables::load_embedded().expect("valid tables");
    let bash = tables.table("bash").expect("the bash table");
    assert_eq!(
        bash.entries().len(),
        BASH_TABLE.matches("[[entry]]").count()
    );
    assert!(
        bash.entries()
            .iter()
            .all(|entry| entry.rule.to_string().starts_with('B'))
    );
    assert!(tables.habits().any(|word| word == "rm"));
}
