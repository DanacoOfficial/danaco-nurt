// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Behaviour of the PowerShell translation that the corpus does not show:
//! call parameters, notes, script mode, `let` names, base of relative
//! paths, error details and the public lexer.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_correct::powershell::lex::{TokenKind, lex};
use nurt_correct::powershell::{
    Mode, TranslateContext, Translation, detect, translate, translate_with,
};

/// Result of a test whose helpers report a missing translation or error.
type Outcome = Result<(), Box<dyn std::error::Error>>;

fn live(input: &str) -> Result<Translation, String> {
    translate(input).ok_or_else(|| format!("no translation of {input:?}"))
}

fn error(input: &str, context: &TranslateContext<'_>) -> Result<Error, String> {
    match translate_with(input, context) {
        Some(Err(error)) => Ok(error),
        Some(Ok(translation)) => Err(format!("{input:?} became {:?}", translation.text())),
        None => Err(format!("{input:?} is not a PowerShell habit")),
    }
}

fn script() -> TranslateContext<'static> {
    TranslateContext {
        mode: Mode::Script,
        ..TranslateContext::default()
    }
}

#[test]
fn what_if_sets_the_preview_call_parameter() -> Outcome {
    let translation = live("Remove-Item C:\\tmp\\x -WhatIf")?;
    assert_eq!(translation.options.preview, Some(true));
    assert_eq!(translation.text(), "remove \"C:/tmp/x\"");
    let rules: Vec<&str> = translation
        .rules
        .iter()
        .map(|rule| rule.rule.as_str())
        .collect();
    assert_eq!(rules, ["P08", "L19", "P33"]);
    assert_eq!(translation.rules[2].to, "preview: true");
    Ok(())
}

#[test]
fn what_if_false_is_removed_without_preview() -> Outcome {
    let translation = live("Remove-Item /srv/x -WhatIf:$false")?;
    assert_eq!(translation.options.preview, None);
    assert_eq!(translation.text(), "remove \"/srv/x\"");
    Ok(())
}

#[test]
fn removed_parameters_are_explained_in_notes() -> Outcome {
    let translation = live("Remove-Item /srv/x -Recurse -Confirm:$false")?;
    let notes: Vec<(&str, &str)> = translation
        .notes
        .iter()
        .map(|note| (note.rule.as_str(), note.text.as_str()))
        .collect();
    assert!(notes.contains(&("P08", "remove deletes directories with their content")));
    assert!(notes.iter().any(|(rule, _)| *rule == "P34"));
    Ok(())
}

#[test]
fn corrections_are_numbered_with_positions() -> Outcome {
    let translation = live("Get-ChildItem C:\\proj")?;
    assert_eq!(translation.rules[0].id, "C1");
    assert_eq!(translation.rules[1].id, "C2");
    let path = translation.rules[1].at.ok_or("no position")?;
    assert_eq!((path.line, path.column, path.length), (1, 15, Some(7)));
    assert_eq!(translation.rules[1].from, "C:\\proj");
    assert_eq!(translation.rules[1].to, "\"C:/proj\"");
    Ok(())
}

#[test]
fn relative_paths_need_the_base() -> Outcome {
    let missing = error("Get-ChildItem src -Recurse", &TranslateContext::default())?;
    assert_eq!(missing.code, ErrorCode::PathRelative);
    let context = TranslateContext {
        base: Some("/srv/app"),
        ..TranslateContext::default()
    };
    let translation =
        translate_with("Get-ChildItem src -Recurse", &context).ok_or("no translation")??;
    assert!(translation.text().starts_with("find \"src\""));
    Ok(())
}

#[test]
fn the_current_directory_is_never_guessed() -> Outcome {
    let context = TranslateContext::default();
    let location = error("Set-Location C:\\proj", &context)?;
    assert_eq!(location.code, ErrorCode::Uncorrectable);
    assert_eq!(location.details["base"], "C:/proj");
    let combined = error("cd C:\\proj; gci", &context)?;
    assert_eq!(combined.problem.as_deref(), Some("no_current_directory"));
    let others = combined.details["errors"]
        .as_array()
        .ok_or("no further errors")?;
    assert_eq!(others.len(), 1);
    assert_eq!(others[0]["fix"][0], "list \"C:/proj\" hidden: false");
    Ok(())
}

#[test]
fn an_argument_text_is_not_split_silently() -> Outcome {
    let input = "Start-Process app.exe -ArgumentList \"a b\"";
    let ambiguous = error(input, &TranslateContext::default())?;
    assert_eq!(ambiguous.code, ErrorCode::Ambiguous);
    let programs: Vec<&str> = ambiguous
        .candidates
        .iter()
        .map(|candidate| candidate.program.as_str())
        .collect();
    assert_eq!(
        programs,
        [
            "run [\"app.exe\", \"a\", \"b\"]",
            "run [\"app.exe\", \"a b\"]"
        ]
    );
    assert_eq!(ambiguous.candidates[0].rank, 1);
    assert!(ambiguous.candidates[0].why.is_some());
    let applied = translate_with(input, &script()).ok_or("no translation")??;
    assert_eq!(applied.text(), "run [\"app.exe\", \"a\", \"b\"]");
    assert_eq!(applied.alternatives.len(), 1);
    assert_eq!(applied.assumptions[0].rule, "P23");
    Ok(())
}

#[test]
fn two_pipelines_in_a_live_call_are_next_steps() -> Outcome {
    let one_effect = error(
        "Remove-Item /srv/a; Remove-Item /srv/b",
        &TranslateContext::default(),
    )?;
    assert_eq!(one_effect.code, ErrorCode::OneEffect);
    assert_eq!(
        one_effect.next_steps,
        ["remove \"/srv/a\"", "remove \"/srv/b\""]
    );
    Ok(())
}

#[test]
fn script_mode_translates_every_statement() -> Outcome {
    let translation = translate_with("Remove-Item /srv/a\nRemove-Item /srv/b", &script())
        .ok_or("no translation")??;
    assert_eq!(translation.tree.program.statements.len(), 2);
    let unreadable = error("Remove-Item 'unterminated", &script())?;
    assert_eq!(unreadable.code, ErrorCode::Syntax);
    assert!(translate_with("find \"/srv\"", &script()).is_some());
    Ok(())
}

#[test]
fn variables_become_let_names_and_references() -> Outcome {
    let translation = live("$logs = Get-ChildItem /srv/logs; $logs | Measure-Object")?;
    assert_eq!(translation.tree.program.statements.len(), 2);
    assert!(translation.text().contains("@logs"));
    let lets = ["files".to_owned()];
    let context = TranslateContext {
        lets: &lets,
        ..TranslateContext::default()
    };
    let known = translate_with("$files | Measure-Object", &context).ok_or("no translation")??;
    assert_eq!(known.text(), "@files |> count");
    let properties = error("$files | Sort-Object Length", &context)?;
    assert_eq!(properties.problem.as_deref(), Some("unknown_record"));
    let unknown = error("$other | Measure-Object", &TranslateContext::default())?;
    assert_eq!(unknown.problem.as_deref(), Some("unknown_variable"));
    Ok(())
}

#[test]
fn a_changing_command_is_not_named_by_let() -> Outcome {
    let failure = error("$x = Remove-Item /srv/a", &TranslateContext::default())?;
    assert_eq!(failure.problem.as_deref(), Some("effect_in_let"));
    Ok(())
}

#[test]
fn size_multipliers_are_binary() -> Outcome {
    let mega = live("Get-ChildItem /srv | Where-Object { $_.Length -gt 1MB }")?;
    assert!(
        mega.text().ends_with("filter size > 1mib"),
        "{}",
        mega.text()
    );
    let kilo = live("Get-ChildItem /srv | Where-Object { $_.Length -le 10KB }")?;
    assert!(
        kilo.text().ends_with("filter size <= 10kib"),
        "{}",
        kilo.text()
    );
    Ok(())
}

#[test]
fn string_comparisons_ignore_case_as_in_powershell() -> Outcome {
    let equal = live("Get-Process | Where-Object { $_.Name -eq 'Chrome' }")?;
    assert!(
        equal
            .text()
            .ends_with("filter lower(name) == lower(\"Chrome\")"),
        "{}",
        equal.text()
    );
    let exact = live("Get-Process | Where-Object { $_.Name -ceq 'Chrome' }")?;
    assert!(
        exact.text().ends_with("filter name == \"Chrome\""),
        "{}",
        exact.text()
    );
    let pattern = live("Get-Process | Where-Object { $_.Name -match '^chr' }")?;
    assert!(
        pattern.text().ends_with("filter name ~ /^chr/i"),
        "{}",
        pattern.text()
    );
    let ordered = live("Get-Process | Where-Object { $_.Name -gt 'm' }")?;
    assert!(!ordered.assumptions.is_empty());
    Ok(())
}

#[test]
fn unknown_parameters_list_the_translated_ones() -> Outcome {
    let failure = error(
        "Get-ChildItem /srv -Colour red",
        &TranslateContext::default(),
    )?;
    assert_eq!(failure.code, ErrorCode::Uncorrectable);
    assert!(failure.message.contains("-Path"), "{}", failure.message);
    assert!(failure.message.contains("-Recurse"), "{}", failure.message);
    Ok(())
}

#[test]
fn unknown_cmdlets_are_reported() -> Outcome {
    let failure = error("Get-Widget -Name x", &TranslateContext::default())?;
    assert_eq!(failure.message, "Get-Widget has no Nurt translation");
    Ok(())
}

#[test]
fn the_public_lexer_reads_powershell_tokens() -> Outcome {
    let tokens = lex("& git --version -C:$false 2MB")?;
    assert_eq!(tokens[0].kind, TokenKind::Ampersand);
    assert_eq!(tokens[2].kind, TokenKind::Word("--version".to_owned()));
    assert!(matches!(&tokens[3].kind, TokenKind::Parameter { name, colon: true } if name == "C"));
    assert!(
        matches!(&tokens[5].kind, TokenKind::Number(number) if number.bytes() == Some(2 * 1024 * 1024))
    );
    Ok(())
}

#[test]
fn detection_keeps_posix_flags_with_bash() {
    assert!(detect("ls -Recurse /srv"));
    assert!(!detect("ls -l /srv"));
    assert!(!detect("rm -rf /srv/x"));
    assert!(!detect("copy \"/a\" to: \"/b\""));
}
