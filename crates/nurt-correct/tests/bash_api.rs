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

/// The reaction of a live translation without `base` and `let` names: the
/// canonical text, the error code, or `unchanged` for a Nurt program.
fn live_reaction(input: &str) -> String {
    match translate_with(input, &TranslateContext::default()) {
        None => "unchanged".to_owned(),
        Some(Ok(translation)) => translation.text(),
        Some(Err(error)) => error.code().as_str().to_owned(),
    }
}

fn assert_reactions(cases: &[(&str, &str)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(input, expected)| {
            let actual = live_reaction(input);
            (actual != *expected)
                .then(|| format!("{input:?}\n  expected {expected:?}\n  actual   {actual:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} failing cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn redirections_without_an_equivalent_are_uncorrectable() {
    assert_reactions(&[
        ("ls -l /srv >&2", "E_UNCORRECTABLE"),
        ("ls -l /srv 3> /srv/out.txt", "E_UNCORRECTABLE"),
        ("ls /srv | wc -l < /srv/a.txt", "E_UNCORRECTABLE"),
        ("rm -f /srv/a > /srv/log.txt", "E_UNCORRECTABLE"),
        ("wc -l <<EOF\nx\nEOF\n", "E_UNCORRECTABLE"),
        ("ls -l /srv &> /dev/null", "E_UNCORRECTABLE"),
    ]);
}

#[test]
fn redirections_with_an_equivalent_are_translated() {
    assert_reactions(&[
        ("wc -l < /srv/a.txt", "lines \"/srv/a.txt\"\n|> count"),
        (
            "ls -l /srv &> /srv/out.txt",
            "write \"/srv/out.txt\" from: (list \"/srv\" hidden: false) mode: overwrite",
        ),
    ]);
}

#[test]
fn flags_and_operands_outside_the_entry_are_uncorrectable() {
    assert_reactions(&[
        ("rm -f", "E_UNCORRECTABLE"),
        ("head -n", "E_UNCORRECTABLE"),
        ("ls --frobnicate /srv", "E_UNCORRECTABLE"),
        ("cat /srv/a | mkdir -p", "E_UNCORRECTABLE"),
        ("rm /srv/**.tmp", "E_UNCORRECTABLE"),
        ("rm *.tmp", "E_PATH_RELATIVE"),
        ("curl -H 'X' https://a.example.com/x", "E_UNCORRECTABLE"),
        ("service nginx > /dev/null", "E_UNCORRECTABLE"),
        ("service nginx frobnicate > /dev/null", "E_UNCORRECTABLE"),
        ("git -C", "E_UNCORRECTABLE"),
        ("git -C /srv/app commit -m \"$MSG\"", "E_UNCORRECTABLE"),
    ]);
}

#[test]
fn long_flags_name_parameters_of_the_target_command() {
    assert_reactions(&[
        ("rm -f -- /srv/-a", "remove \"/srv/-a\""),
        (
            "head --lines=5 /srv/a.txt",
            "read \"/srv/a.txt\" lines: ..5",
        ),
        ("ls --depth=2 /srv", "list \"/srv\" hidden: false depth: 2"),
        ("ls --hidden=true /srv", "list \"/srv\" hidden: true"),
        (
            "ls --type=file /srv",
            "list \"/srv\" hidden: false type: file",
        ),
        (
            "ls --sort='a b' /srv",
            "list \"/srv\" hidden: false sort: \"a b\"",
        ),
        ("ls --dep /srv", "list \"/srv\" hidden: false depth: true"),
    ]);
}

#[test]
fn patterns_archives_and_git_programs_are_translated() {
    assert_reactions(&[
        (
            "rm /*.tmp",
            "find \"/\" glob: \"*.tmp\" depth: 1 hidden: false\n|> remove on_missing: error",
        ),
        (
            "rm /srv/'a*'*.tmp",
            "find \"/srv\" glob: \"a[*]*.tmp\" depth: 1 hidden: false\n|> remove on_missing: error",
        ),
        (
            "zip -r /srv/a.zip /srv/x",
            "find \"/srv/x\" type: any\n|> archive.create \"/srv/a.zip\" format: zip base: \"/\"",
        ),
        (
            "git -C /srv/app frobnicate --x",
            "run [\"git\", \"frobnicate\", \"--x\"] cwd: \"/srv/app\"",
        ),
    ]);
}

#[test]
fn text_without_a_file_is_evaluated_and_unsupported_formats_are_uncorrectable() {
    assert_reactions(&[
        ("echo -n hello", "evaluate \"hello\""),
        ("printf > /srv/a.txt", "E_UNCORRECTABLE"),
        ("printf '%d' 5 > /srv/a.txt", "E_UNCORRECTABLE"),
        ("printf 'a' b > /srv/a.txt", "E_UNCORRECTABLE"),
    ]);
}

#[test]
fn command_substitutions_assigned_to_a_name_become_results() {
    let text = |input: &str| {
        translate_with(input, &SCRIPT)
            .expect("a bash habit")
            .expect("a translation")
            .text()
    };
    assert_eq!(
        text("FILES=$(find /srv/app -name \"*.go\")"),
        "let files = find \"/srv/app\" glob: \"**/*.go\" type: any"
    );
    assert_eq!(text("NOW=$(date)"), "let now = evaluate now()");
    assert_eq!(
        text("FILES=$(find /srv/app -name \"*.go\")\nwc -l $FILES"),
        "let files = find \"/srv/app\" glob: \"**/*.go\" type: any\nlines @files\n|> count"
    );
    for input in [
        "X=$(rm /a)",
        "X=$(date) Y=$(date)",
        "Bad-Name=$(date)",
        "X=$(date)\nX=$(date)",
    ] {
        assert!(
            matches!(translate_with(input, &SCRIPT), Some(Err(_))),
            "{input}"
        );
    }
}

#[test]
fn text_without_a_file_is_the_value_of_evaluate() {
    let text = |input: &str| {
        translate_with(input, &SCRIPT)
            .expect("a bash habit")
            .expect("a translation")
            .text()
    };
    assert_eq!(text("echo \"hello\""), "evaluate \"hello\"");
    assert_eq!(text("printf \"%s\\n\" \"hello\""), "evaluate \"hello\"");
    assert_eq!(text("echo -n abc"), "evaluate \"abc\"");
}

#[test]
fn text_written_to_a_file_keeps_its_bytes() {
    assert_reactions(&[
        (
            "echo -E 'a\\tb' > /srv/f.txt",
            "write \"/srv/f.txt\" r\"a\\tb\n\" mode: overwrite",
        ),
        (
            "printf '100%%\\n' > /srv/a.txt",
            "write \"/srv/a.txt\" \"100%\\n\" mode: overwrite",
        ),
    ]);
}

#[test]
fn tests_outside_the_table_are_uncorrectable() {
    assert_reactions(&[
        ("[ -f /srv/a", "E_UNCORRECTABLE"),
        ("[ -f ]", "E_UNCORRECTABLE"),
        ("[ -q /srv/a ]", "E_UNCORRECTABLE"),
        ("[ -d /srv/a ] && make", "E_UNCORRECTABLE"),
        (
            "[ ! -f /srv/a ]",
            "info \"/srv/a\"\n|> assert all: not type == file",
        ),
    ]);
}

#[test]
fn substitutions_of_several_or_changing_commands_are_uncorrectable() {
    assert_reactions(&[
        ("cat $(ls /srv; ls /tmp)", "E_UNCORRECTABLE"),
        ("cat $(ls /srv && ls /tmp)", "E_UNCORRECTABLE"),
        ("cat $(rm -f /srv/a)", "E_UNCORRECTABLE"),
    ]);
}

#[test]
fn entries_reach_commands_through_xargs_tee_and_at() {
    assert_reactions(&[
        ("find /srv -name '*.tmp' | xargs -n 1 rm", "E_UNCORRECTABLE"),
        ("find /srv -name '*.tmp' | xargs mv", "E_UNCORRECTABLE"),
        ("cat /srv/a.txt | xargs rm -f", "E_UNCORRECTABLE"),
        ("find /srv -name '*.tmp' | xargs", "E_UNCORRECTABLE"),
        ("find /srv -name '*.go' | xargs wc -l", "E_UNCORRECTABLE"),
        ("find /srv -name '*.go' | xargs gofmt -l", "E_UNCORRECTABLE"),
        (
            "cd /srv && find /srv -name '*.go' | xargs gofmt -l",
            "E_AMBIGUOUS",
        ),
        ("ls -l /srv | tee -x /srv/out.txt", "E_UNCORRECTABLE"),
        ("ls -l /srv | tee /srv/a /srv/b", "E_UNCORRECTABLE"),
        ("at 10:00 rm -f /srv/a", "E_UNCORRECTABLE"),
        ("at 2026-12-01T10:00 > /dev/null", "E_UNCORRECTABLE"),
    ]);
}

#[test]
fn sed_scripts_outside_the_interpreter_are_uncorrectable() {
    assert_reactions(&[
        ("ls /srv | sed -n 1p", "E_UNCORRECTABLE"),
        ("sed -i", "E_UNCORRECTABLE"),
        ("sed -i 's/a/b/'", "E_UNCORRECTABLE"),
        ("sed -i \"$X\" /srv/f", "E_UNCORRECTABLE"),
        ("sed -i 's/a/b/w' /srv/f", "E_UNCORRECTABLE"),
        ("sed -E -i 's/a(/b/' /srv/f", "E_UNCORRECTABLE"),
        ("sed -n 1p /srv/a /srv/b", "E_UNCORRECTABLE"),
        ("sed '1p' /srv/a", "E_UNCORRECTABLE"),
    ]);
}

#[test]
fn sed_scripts_with_flags_and_addresses_are_translated() {
    assert_reactions(&[
        ("sed -n 5p /srv/a", "read \"/srv/a\" lines: 5"),
        ("sed -i '/a(/d' /srv/f", "E_AMBIGUOUS"),
        (
            "sed -i 's/a/b/gI' /srv/f",
            "replace /a/ \"b\" in: [\"/srv/f\"] case_sensitive: false",
        ),
        (
            "sed -E -i 's/a+/b/' /srv/f",
            "replace /a+/ \"b\" in: [\"/srv/f\"] occurrence: 1",
        ),
        (
            "sed -i -e 's/a/b/' /srv/f",
            "replace /a/ \"b\" in: [\"/srv/f\"] occurrence: 1",
        ),
    ]);
}

#[test]
fn find_predicates_outside_the_table_are_uncorrectable() {
    assert_reactions(&[
        ("ls /srv | find /srv -type f", "E_UNCORRECTABLE"),
        ("find /srv /tmp -name '*.go'", "E_UNCORRECTABLE"),
        ("find /srv -name", "E_UNCORRECTABLE"),
        ("find /srv -exec rm {}", "E_UNCORRECTABLE"),
        ("find /srv -exec rm \\;", "E_UNCORRECTABLE"),
        ("find /srv -name 'a/b'", "E_UNCORRECTABLE"),
        ("find /srv -type q", "E_UNCORRECTABLE"),
        (
            "find /srv -mmin -5",
            "find \"/srv\" type: any\n|> filter modified > ago(5m)",
        ),
    ]);
}

#[test]
fn grep_patterns_and_operands_follow_the_dialect() {
    assert_reactions(&[
        ("grep -E 'a(' /srv/a", "E_UNCORRECTABLE"),
        ("ls /srv | grep -i a /srv/b", "E_UNCORRECTABLE"),
        ("grep -i a", "E_UNCORRECTABLE"),
        ("grep -i", "E_UNCORRECTABLE"),
        ("grep -e a -e b /srv/a", "E_UNCORRECTABLE"),
        ("grep -r foo", "E_PATH_RELATIVE"),
        (
            "grep -r --include '*.go' foo /srv",
            "search /foo/ \"/srv\" glob: \"**/*.go\"",
        ),
        ("grep -e foo /srv/a", "search /foo/ \"/srv/a\""),
        ("grep -P '\\d+' /srv/a", "search /\\d+/ \"/srv/a\""),
    ]);
}

#[test]
fn shell_constructs_without_an_equivalent_are_uncorrectable() {
    assert_reactions(&[
        ("export FOO && make", "E_UNCORRECTABLE"),
        ("! grep -q a /srv/a", "unchanged"),
        (
            "cd /srv/app && for f in *; do echo $f; done",
            "E_UNCORRECTABLE",
        ),
        ("env A=1 > /dev/null", "E_UNCORRECTABLE"),
        ("head -n x /srv/a", "E_UNCORRECTABLE"),
        ("timeout 5x make", "E_UNCORRECTABLE"),
    ]);
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
