// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Consistency of generated texts, pages and guides with the catalog.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::envelope::Language;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::RecordKind;
use nurt_help::catalog::Catalog;
use nurt_help::consistency::check_program;
use nurt_help::describe::{self, Layer, Length, TARGET_LENGTHS};
use nurt_help::docs::CommandPage;
use nurt_help::help::{self, HelpFormat, HelpQuery};

/// Every text the crate generates for the catalog, with a label.
fn generated_texts(catalog: &Catalog) -> Vec<(String, String)> {
    let mut texts = Vec::new();
    for language in [Language::En, Language::Pl] {
        for target in TARGET_LENGTHS {
            let description =
                describe::layer(catalog, Layer::Core, "", language, Length::Tokens(target))
                    .unwrap();
            texts.push((format!("core {language:?} {target}"), description.text));
        }
        for file in catalog.groups() {
            let description = describe::layer(
                catalog,
                Layer::Group,
                &file.group,
                language,
                Length::Tokens(4000),
            )
            .unwrap();
            texts.push((
                format!("group {} {language:?}", file.group),
                description.text,
            ));
        }
        let overview = help::render(&HelpQuery::new().with_language(language), catalog).unwrap();
        texts.push((format!("overview {language:?}"), overview.body));
    }
    for guide in catalog.docs().guides() {
        texts.push((format!("guide {}", guide.name), guide.render()));
    }
    for topic in ["syntax", "corrections"] {
        let answer = help::render(&HelpQuery::topic(topic), catalog).unwrap();
        texts.push((topic.to_owned(), answer.body));
    }
    texts
}

/// Lines of a block that starts at the line with `start` and ends at the
/// next blank line.
fn block_after<'a>(text: &'a str, start: &str) -> Vec<&'a str> {
    text.lines()
        .skip_while(|line| !line.starts_with(start))
        .skip(1)
        .take_while(|line| !line.is_empty())
        .collect()
}

#[test]
fn core_command_lines_use_only_names_of_the_catalog() {
    let catalog = common::catalog();
    let text = describe::layer(
        &catalog,
        Layer::Core,
        "",
        Language::En,
        Length::Tokens(4000),
    )
    .unwrap()
    .text;
    let lines = block_after(&text, "CORE COMMANDS");
    let mut seen = 0;
    for line in lines.iter().filter(|line| !line.ends_with(':')) {
        let signature = line.split(" — ").next().unwrap();
        let mut words = signature.split(' ');
        let name = words.next().unwrap();
        let spec = catalog
            .command(name)
            .unwrap_or_else(|| panic!("unknown command {name}"));
        seen += 1;
        for word in words {
            if let Some((param, hint)) = word.split_once(':') {
                let param = spec
                    .param(param)
                    .unwrap_or_else(|| panic!("{name} has no parameter {param}"));
                if !param.values.is_empty() {
                    for item in hint
                        .trim_end_matches('*')
                        .trim_matches(['[', ']'])
                        .split('|')
                        .filter(|item| !item.is_empty())
                    {
                        assert!(
                            param.values.iter().any(|value| value == item)
                                || param.default.as_ref().is_some_and(|d| d.as_str() == item),
                            "{name}.{} has no value {item}",
                            param.name
                        );
                    }
                }
            } else {
                let label = word.trim_end_matches(['*', '…']).to_lowercase();
                let param = spec
                    .param(&label)
                    .unwrap_or_else(|| panic!("{name} has no positional {label}"));
                assert!(param.is_positional(), "{name}.{label}");
            }
        }
    }
    assert_eq!(seen, catalog.core_commands().len());
}

#[test]
fn group_command_lines_use_only_names_of_the_catalog() {
    let catalog = common::catalog();
    for file in catalog.groups().filter(|file| file.group != "core") {
        let text = describe::layer(
            &catalog,
            Layer::Group,
            &file.group,
            Language::En,
            Length::Tokens(4000),
        )
        .unwrap()
        .text;
        let mut listed = 0;
        for line in text.lines().take_while(|line| *line != "Examples:") {
            let signature = line.split(" — ").next().unwrap();
            let mut words = signature.split(' ');
            let Some(spec) = words.next().and_then(|name| file.command(name)) else {
                continue;
            };
            listed += 1;
            for word in words {
                match word.split_once(':') {
                    Some((param, _)) => {
                        assert!(spec.param(param).is_some(), "{}.{param}", spec.name)
                    }
                    None => {
                        let label = word.trim_end_matches(['*', '…']).to_lowercase();
                        assert!(spec.param(&label).is_some(), "{}.{label}", spec.name);
                    }
                }
            }
        }
        assert_eq!(listed, file.commands.len(), "{}", file.group);
    }
}

#[test]
fn examples_of_the_core_layer_name_only_known_commands_parameters_and_values() {
    let catalog = common::catalog();
    let text = describe::layer(
        &catalog,
        Layer::Core,
        "",
        Language::En,
        Length::Tokens(4000),
    )
    .unwrap()
    .text;
    let examples = block_after(&text, "EXAMPLES");
    assert!(examples.len() >= 7);
    let problems = check_program(&catalog, &examples.join("\n"));
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn examples_of_the_groups_name_only_known_commands_parameters_and_values() {
    let catalog = common::catalog();
    let mut count = 0;
    for file in catalog.groups().filter(|file| file.group != "core") {
        let examples = describe::group_examples(&file.group);
        assert!(!examples.is_empty(), "{} has no example", file.group);
        for program in examples {
            count += 1;
            let problems = check_program(&catalog, program);
            assert!(problems.is_empty(), "{program}: {problems:?}");
        }
    }
    assert!(count >= 20);
}

#[test]
fn checker_reports_unknown_commands_parameters_and_values() {
    let catalog = common::catalog();
    assert_eq!(check_program(&catalog, "remvoe \"/a\"").len(), 1);
    assert_eq!(check_program(&catalog, "list \"/a\" depht: 2").len(), 1);
    assert_eq!(check_program(&catalog, "list \"/a\" type: folder").len(), 1);
    assert!(check_program(&catalog, "list \"/a\" type: directory depth: 2").is_empty());
    assert!(check_program(&catalog, "let x = list \"/a\"\n@x |> count").is_empty());
}

#[test]
fn records_line_names_known_record_kinds() {
    let catalog = common::catalog();
    let text = describe::layer(
        &catalog,
        Layer::Core,
        "",
        Language::En,
        Length::Tokens(4000),
    )
    .unwrap()
    .text;
    let line = text
        .lines()
        .find(|line| line.starts_with("RECORDS"))
        .unwrap();
    let list = line.strip_prefix("RECORDS (fields): ").unwrap();
    let names: Vec<&str> = list
        .split(" · ")
        .map(|part| part.split('(').next().unwrap())
        .collect();
    assert_eq!(names.len(), 7);
    for name in names {
        assert!(name.parse::<RecordKind>().is_ok(), "{name}");
    }
}

#[test]
fn error_codes_in_generated_texts_exist() {
    let catalog = common::catalog();
    let mut checked = 0;
    for (label, text) in generated_texts(&catalog) {
        for word in text.split(|ch: char| !(ch.is_ascii_uppercase() || ch == '_')) {
            if word.starts_with("E_") && word.len() > 2 {
                assert!(word.parse::<ErrorCode>().is_ok(), "{label}: {word}");
                checked += 1;
            }
        }
    }
    assert!(checked > 10);
}

#[test]
fn error_codes_in_pages_exist() {
    let catalog = common::catalog();
    for page in catalog.docs().pages() {
        for note in &page.errors {
            assert!(
                note.code.parse::<ErrorCode>().is_ok(),
                "{}: {}",
                page.command,
                note.code
            );
        }
    }
}

#[test]
fn generated_texts_carry_no_decision_identifiers_or_working_notes() {
    let catalog = common::catalog();
    for (label, text) in generated_texts(&catalog) {
        let chars: Vec<char> = text.chars().collect();
        for (index, window) in chars.windows(3).enumerate() {
            let boundary = index == 0 || !chars[index - 1].is_alphanumeric();
            let identifier = window[0].is_ascii_uppercase()
                && window[1] == '-'
                && window[2].is_ascii_digit()
                && boundary;
            assert!(!identifier, "{label}: identifier at {index}");
        }
        for word in ["TODO", "FIXME", "proposal", "decision"] {
            assert!(!text.contains(word), "{label}: {word}");
        }
    }
}

#[test]
fn every_command_has_a_help_topic_with_all_its_parameters() {
    let catalog = common::catalog();
    for spec in catalog.commands() {
        let answer = help::render(&HelpQuery::topic(spec.name.as_str()), &catalog).unwrap();
        assert!(
            answer.body.starts_with(&format!("{} — ", spec.name)),
            "{}",
            spec.name
        );
        for param in &spec.params {
            let shown = if param.is_positional() {
                answer.body.contains(&param.name.to_ascii_uppercase())
            } else {
                answer.body.contains(&format!("{}:", param.name))
            };
            assert!(shown, "{}.{}", spec.name, param.name);
        }
        let json = help::render(
            &HelpQuery::topic(spec.name.as_str()).with_format(HelpFormat::Json),
            &catalog,
        )
        .unwrap();
        let document: serde_json::Value = serde_json::from_str(&json.body).unwrap();
        assert_eq!(
            document["params"].as_array().unwrap().len(),
            spec.params.len()
        );
    }
}

fn type_text(page_type: &str) -> String {
    page_type.replace("\\|", "|")
}

fn assert_page_matches_catalog(catalog: &Catalog, page: &CommandPage) {
    let spec = catalog.command(&page.command).unwrap();
    assert_eq!(page.group, spec.group, "{}", page.command);
    assert_eq!(page.summary, spec.summary, "{}", page.command);
    assert_eq!(page.parameters.len(), spec.params.len(), "{}", page.command);
    for row in &page.parameters {
        let param = spec
            .param(&row.name)
            .unwrap_or_else(|| panic!("{}.{}", page.command, row.name));
        assert_eq!(
            type_text(&row.ty),
            param.ty.to_string(),
            "{}.{}",
            page.command,
            row.name
        );
        let expected_default = match (&param.default, &param.default_note, param.required) {
            (Some(value), _, _) => value.as_str().to_owned(),
            (None, Some(note), _) => note.clone(),
            (None, None, true) => "required".to_owned(),
            (None, None, false) => "-".to_owned(),
        };
        assert_eq!(
            row.default, expected_default,
            "{}.{}",
            page.command, row.name
        );
        assert_eq!(
            row.description, param.summary,
            "{}.{}",
            page.command, row.name
        );
    }
}

#[test]
fn pages_agree_with_the_catalog() {
    let catalog = common::catalog();
    let pages: Vec<&CommandPage> = catalog.docs().pages().collect();
    let names: Vec<&str> = pages.iter().map(|page| page.command.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted, "pages are unique and ordered by name");
    for required in ["describe", "evaluate", "help"] {
        assert!(names.contains(&required), "missing page {required}");
    }
    for name in &names {
        assert!(
            catalog.command(name).is_some(),
            "page without command {name}"
        );
    }
    for page in pages {
        assert_page_matches_catalog(&catalog, page);
    }
}

#[test]
fn pages_have_at_least_three_examples_that_agree_with_the_catalog() {
    let catalog = common::catalog();
    for page in catalog.docs().pages() {
        assert!(
            page.examples.len() >= 3,
            "{} has {} examples",
            page.command,
            page.examples.len()
        );
        for example in &page.examples {
            assert!(
                !example.caption.is_empty(),
                "{}: {}",
                page.command,
                example.program
            );
            let problems = check_program(&catalog, &example.program);
            assert!(
                problems.is_empty(),
                "{}: {}: {problems:?}",
                page.command,
                example.program
            );
        }
    }
}

#[test]
fn page_synopsis_starts_with_the_command() {
    let catalog = common::catalog();
    for page in catalog.docs().pages() {
        assert!(!page.synopsis.is_empty(), "{}", page.command);
        for line in &page.synopsis {
            assert!(line.contains(&page.command), "{}: {line}", page.command);
        }
    }
}

#[test]
fn page_files_are_named_after_their_command() {
    for (name, text) in common::markdown_files("docs/commands") {
        let command = name.trim_end_matches(".md");
        assert!(text.contains(&format!("command: {command}\n")), "{name}");
    }
}

#[test]
fn guides_name_existing_commands_and_have_checked_examples() {
    let catalog = common::catalog();
    let mut examples = 0;
    for (file, text) in common::markdown_files("docs/guides") {
        let guide = catalog
            .docs()
            .guide(file.trim_end_matches(".md"))
            .unwrap_or_else(|| panic!("{file} is not loaded"));
        assert!(!guide.commands.is_empty(), "{file}");
        for command in &guide.commands {
            assert!(catalog.command(command).is_some(), "{file}: {command}");
        }
        assert!(guide.sections.len() >= 4, "{file}");
        let mut inside = false;
        let mut program = String::new();
        for line in text.lines() {
            if line.starts_with("```nurt") {
                inside = true;
                program.clear();
            } else if line.starts_with("```") && inside {
                inside = false;
                examples += 1;
                let problems = check_program(&catalog, &program);
                assert!(problems.is_empty(), "{file}: {program}: {problems:?}");
            } else if inside {
                program.push_str(line);
                program.push('\n');
            }
        }
    }
    assert!(examples >= 3);
}

#[test]
fn guide_sections_have_a_moderate_length() {
    let catalog = common::catalog();
    for guide in catalog.docs().guides() {
        for section in &guide.sections {
            let words = section.text.split_whitespace().count();
            assert!(
                (20..=250).contains(&words),
                "{}: {}: {words} words",
                guide.name,
                section.heading
            );
        }
    }
}

#[test]
fn catalog_keeps_the_names_skill_for_the_guide_topic() {
    let catalog = common::catalog();
    let help = catalog.command("help").unwrap();
    assert!(help.param("skill").is_some());
    assert!(
        help.param("kind")
            .unwrap()
            .values
            .iter()
            .any(|value| value == "skill")
    );
}

/// Spellings of the catalog that name the guide topic: the parameter and the
/// value of `kind`.
const GUIDE_TOPIC_SPELLINGS: [&str; 9] = [
    "kind: skill",
    "kind:skill",
    "skill:",
    "|skill|",
    "\\|skill\\|",
    "|skill)",
    "\\|skill)",
    "`skill`",
    "\"skill\"",
];

fn without_catalog_spellings(text: &str) -> String {
    let mut text = text.to_lowercase();
    for spelling in GUIDE_TOPIC_SPELLINGS {
        text = text.replace(spelling, "");
    }
    text
}

fn documentation_texts() -> Vec<(String, String)> {
    let mut texts = Vec::new();
    for directory in ["docs/commands", "docs/guides"] {
        for (name, text) in common::markdown_files(directory) {
            texts.push((format!("{directory}/{name}"), text));
        }
    }
    texts
}

#[test]
fn texts_use_guides_except_for_catalog_identifiers() {
    let catalog = common::catalog();
    let mut texts = generated_texts(&catalog);
    texts.extend(documentation_texts());
    for (label, text) in texts {
        let stripped = without_catalog_spellings(&text);
        assert!(!stripped.contains("skill"), "{label} uses the word skill");
    }
}

const STAGES_AFTER_THE_EFFECT_EN: &str =
    "assert, filter, select, first, last, skip, count and sort";
const STAGES_AFTER_THE_EFFECT_PL: &str = "assert, filter, select, first, last, skip, count i sort";

fn core_text(catalog: &Catalog, language: Language, target: u32) -> String {
    describe::layer(catalog, Layer::Core, "", language, Length::Tokens(target))
        .unwrap()
        .text
}

#[test]
fn core_layer_allows_stages_after_the_effect_in_every_length() {
    let catalog = common::catalog();
    for target in TARGET_LENGTHS {
        let english = core_text(&catalog, Language::En, target);
        assert!(
            english.contains(STAGES_AFTER_THE_EFFECT_EN),
            "en {target}: stages after the effect"
        );
        assert!(
            english.contains("only verify rolls back"),
            "en {target}: rollback by verify only"
        );
        let polish = core_text(&catalog, Language::Pl, target);
        assert!(
            polish.contains(STAGES_AFTER_THE_EFFECT_PL),
            "pl {target}: stages after the effect"
        );
        assert!(
            polish.contains("wycofuje wyłącznie verify"),
            "pl {target}: rollback by verify only"
        );
    }
}

#[test]
fn syntax_topic_states_the_stages_after_the_effect() {
    let catalog = common::catalog();
    for (language, stages) in [
        (Language::En, STAGES_AFTER_THE_EFFECT_EN),
        (Language::Pl, STAGES_AFTER_THE_EFFECT_PL),
    ] {
        let answer = help::render(
            &HelpQuery::topic("syntax").with_language(language),
            &catalog,
        )
        .unwrap();
        assert!(answer.body.contains(stages), "{language:?}");
    }
}

#[test]
fn core_layer_describes_partial_undo_and_selected_paths() {
    let catalog = common::catalog();
    for target in [2000, 3000, 4000] {
        for language in [Language::En, Language::Pl] {
            let text = core_text(&catalog, language, target);
            for phrase in [
                "history state: partial",
                "undo on_conflict: partial",
                "undo paths:",
            ] {
                assert!(
                    text.contains(phrase),
                    "{language:?} {target}: missing {phrase}"
                );
            }
        }
    }
}

#[test]
fn texts_do_not_mention_a_window_for_undoing() {
    let catalog = common::catalog();
    let mut texts = generated_texts(&catalog);
    texts.extend(documentation_texts());
    for (label, text) in texts {
        let lowered = text.to_lowercase();
        for phrase in [
            "undo window",
            "undo period",
            "window for undo",
            "window of undo",
            "okno cofania",
            "okno cofnięcia",
        ] {
            assert!(!lowered.contains(phrase), "{label} mentions {phrase}");
        }
    }
}
