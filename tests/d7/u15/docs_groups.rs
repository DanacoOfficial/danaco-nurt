// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Consistency of the pages of the command groups with the command catalog.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "helpers of an integration test fail the test on unexpected input"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

const GROUPS: [&str; 6] = [
    "system", "admin", "package", "network", "schedule", "program",
];

const COMMANDS_DOCUMENTED_ELSEWHERE: [&str; 2] = ["suspend", "resume"];

const EXPECTED_COMMANDS: usize = 46;

const MIN_EXAMPLES: usize = 3;

struct Command {
    group: String,
    table: toml::Table,
}

impl Command {
    fn params(&self) -> BTreeSet<String> {
        self.table
            .get("param")
            .and_then(toml::Value::as_array)
            .map(|params| {
                params
                    .iter()
                    .filter_map(|param| param.get("name")?.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn marker(&self) -> Option<char> {
        if self
            .table
            .get("implementation")
            .and_then(toml::Value::as_str)
            == Some("package_manager")
        {
            return Some('P');
        }
        let external = self.table.get("external")?;
        match external.get("kind")?.as_str()? {
            "system_program" => Some('X'),
            "user_program" => Some('U'),
            _ => None,
        }
    }
}

fn root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn load_catalog() -> BTreeMap<String, Command> {
    let mut commands = BTreeMap::new();
    let dir = root().join("catalog");
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let is_group_file = path.extension().is_some_and(|ext| ext == "toml")
            && path
                .file_name()
                .is_some_and(|name| !name.to_string_lossy().starts_with('_'));
        if !is_group_file {
            continue;
        }
        let table: toml::Table = fs::read_to_string(&path).unwrap().parse().unwrap();
        let group = table["group"].as_str().unwrap().to_owned();
        for command in table["command"].as_array().unwrap() {
            let command = command.as_table().unwrap().clone();
            let name = command["name"].as_str().unwrap().to_owned();
            commands.insert(
                name,
                Command {
                    group: group.clone(),
                    table: command,
                },
            );
        }
    }
    commands
}

fn group_commands(catalog: &BTreeMap<String, Command>) -> BTreeMap<&str, &Command> {
    catalog
        .iter()
        .filter(|(name, command)| {
            GROUPS.contains(&command.group.as_str())
                && !COMMANDS_DOCUMENTED_ELSEWHERE.contains(&name.as_str())
        })
        .map(|(name, command)| (name.as_str(), command))
        .collect()
}

fn group_pages() -> Vec<(&'static str, String)> {
    GROUPS
        .iter()
        .map(|group| {
            let path = root().join("docs/groups").join(format!("{group}.md"));
            (*group, fs::read_to_string(path).unwrap())
        })
        .collect()
}

fn section<'a>(page: &'a str, heading: &str) -> &'a str {
    let marker = format!("\n## {heading}\n");
    let start = page
        .find(&marker)
        .unwrap_or_else(|| panic!("missing section {heading}"));
    let body = &page[start + marker.len()..];
    body.find("\n## ").map_or(body, |end| &body[..end])
}

fn first_table(section: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut started = false;
    for line in section.lines() {
        if line.starts_with('|') {
            started = true;
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_owned())
                .collect();
            if cells.iter().all(|cell| cell.chars().all(|c| c == '-')) {
                continue;
            }
            rows.push(cells);
        } else if started {
            break;
        }
    }
    rows.into_iter().skip(1).collect()
}

fn unquote(cell: &str) -> &str {
    cell.trim_matches('`')
}

fn examples(page: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in page.lines() {
        match (&mut current, line) {
            (None, "```nurt example") => current = Some(String::new()),
            (Some(block), "```") => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            (Some(block), text) => {
                block.push_str(text);
                block.push('\n');
            }
            (None, _) => {}
        }
    }
    blocks
}

#[derive(Debug, PartialEq)]
enum Piece {
    Word(String),
    Group(String),
}

fn matching_close(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        _ => '}',
    }
}

fn split_top_level(text: &str, separator: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut parts = vec![String::new()];
    let mut stack: Vec<char> = Vec::new();
    let mut in_text = false;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if in_text {
            if c == '\\' {
                parts.last_mut().unwrap().push(c);
                index += 1;
                if index < chars.len() {
                    parts.last_mut().unwrap().push(chars[index]);
                }
            } else {
                in_text = c != '"';
                parts.last_mut().unwrap().push(c);
            }
            index += 1;
            continue;
        }
        let at_separator = stack.is_empty()
            && !separator.is_empty()
            && chars[index..]
                .iter()
                .take(separator.chars().count())
                .collect::<String>()
                == separator;
        if at_separator {
            parts.push(String::new());
            index += separator.chars().count();
            continue;
        }
        match c {
            '"' => in_text = true,
            '(' | '[' | '{' => stack.push(matching_close(c)),
            ')' | ']' | '}' if stack.pop() != Some(c) => {
                return Err(format!("unbalanced `{c}` in `{text}`"));
            }
            _ => {}
        }
        parts.last_mut().unwrap().push(c);
        index += 1;
    }
    if in_text || !stack.is_empty() {
        return Err(format!("unterminated text or group in `{text}`"));
    }
    Ok(parts)
}

fn pieces(stage: &str) -> Result<Vec<Piece>, String> {
    let chars: Vec<char> = stage.trim().chars().collect();
    let mut pieces = Vec::new();
    let mut word = String::new();
    let mut stack: Vec<char> = Vec::new();
    let mut in_text = false;
    let mut group_start = None;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if in_text {
            word.push(c);
            if c == '\\' {
                index += 1;
                if let Some(next) = chars.get(index) {
                    word.push(*next);
                }
            } else if c == '"' {
                in_text = false;
            }
            index += 1;
            continue;
        }
        match c {
            '"' => {
                in_text = true;
                word.push(c);
            }
            '(' | '[' | '{' => {
                if stack.is_empty() && c == '(' && word.is_empty() {
                    group_start = Some(index + 1);
                }
                stack.push(matching_close(c));
                word.push(c);
            }
            ')' | ']' | '}' => {
                if stack.pop() != Some(c) {
                    return Err(format!("unbalanced `{c}` in `{stage}`"));
                }
                word.push(c);
                if stack.is_empty()
                    && let Some(start) = group_start.take()
                {
                    pieces.push(Piece::Group(chars[start..index].iter().collect()));
                    word.clear();
                }
            }
            c if c.is_whitespace() && stack.is_empty() => {
                if !word.is_empty() {
                    pieces.push(Piece::Word(std::mem::take(&mut word)));
                }
            }
            _ => word.push(c),
        }
        index += 1;
    }
    if in_text || !stack.is_empty() {
        return Err(format!("unterminated text or group in `{stage}`"));
    }
    if !word.is_empty() {
        pieces.push(Piece::Word(word));
    }
    Ok(pieces)
}

fn check_program(text: &str, catalog: &BTreeMap<String, Command>) -> Result<usize, String> {
    let name_pattern = Regex::new(r"^[a-z_]+:$").unwrap();
    let mut stages_checked = 0;
    for statement in split_top_level(text, "\n")? {
        if statement.trim().is_empty() {
            continue;
        }
        for stage in split_top_level(&statement, "|>")? {
            let pieces = pieces(&stage)?;
            let Some(Piece::Word(command)) = pieces.first() else {
                return Err(format!("stage without a command: `{stage}`"));
            };
            let command_entry = catalog
                .get(command.as_str())
                .ok_or_else(|| format!("unknown command `{command}`"))?;
            let params = command_entry.params();
            stages_checked += 1;
            for piece in &pieces[1..] {
                match piece {
                    Piece::Word(word) if name_pattern.is_match(word) => {
                        let name = word.trim_end_matches(':');
                        if !params.contains(name) {
                            return Err(format!("unknown parameter `{name}` of `{command}`"));
                        }
                    }
                    Piece::Group(inner) => stages_checked += check_program(inner, catalog)?,
                    Piece::Word(_) => {}
                }
            }
        }
    }
    Ok(stages_checked)
}

#[test]
fn group_pages_list_every_command_exactly_once() {
    let catalog = load_catalog();
    let expected = group_commands(&catalog);
    assert_eq!(expected.len(), EXPECTED_COMMANDS);
    let mut documented: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (group, page) in group_pages() {
        for row in first_table(section(&page, "Commands")) {
            let name = unquote(&row[0]).to_owned();
            let entry = expected.get(name.as_str()).unwrap_or_else(|| {
                panic!("{group}: `{name}` is not a command of the documented groups")
            });
            assert_eq!(entry.group, group, "`{name}` is listed in the wrong group");
            documented.entry(name).or_default().push(group);
        }
    }
    for name in expected.keys() {
        let pages = documented.get(*name).map_or(0, Vec::len);
        assert_eq!(pages, 1, "`{name}` is listed {pages} times");
    }
}

#[test]
fn group_page_examples_use_catalog_commands_and_parameters() {
    let catalog = load_catalog();
    let mut pages = group_pages();
    pages.push((
        "sleep",
        fs::read_to_string(root().join("docs/commands/sleep.md")).unwrap(),
    ));
    for (name, page) in pages {
        let blocks = examples(&page);
        assert!(
            blocks.len() >= MIN_EXAMPLES,
            "{name}: {} examples, at least {MIN_EXAMPLES} required",
            blocks.len()
        );
        for block in blocks {
            if let Err(problem) = check_program(&block, &catalog) {
                panic!("{name}: {problem} in example\n{block}");
            }
        }
    }
}

#[test]
fn group_page_examples_use_a_command_of_the_group() {
    let catalog = load_catalog();
    for (group, page) in group_pages() {
        let used: Vec<String> = examples(&page)
            .iter()
            .flat_map(|block| split_top_level(block, "|>").unwrap_or_default())
            .flat_map(|stage| stage.split_whitespace().next().map(str::to_owned))
            .collect();
        assert!(
            used.iter()
                .any(|name| catalog.get(name).is_some_and(|c| c.group == group)),
            "{group}: no example uses a command of the group"
        );
    }
}

#[test]
fn group_page_links_resolve() {
    let catalog = load_catalog();
    let link = Regex::new(r"\]\(([^)#]+)(?:#[^)]*)?\)").unwrap();
    for (group, page) in group_pages() {
        let base = root().join("docs/groups");
        for capture in link.captures_iter(&page) {
            let target = &capture[1];
            if let Some(file) = target.strip_prefix("../commands/") {
                let name = file
                    .strip_suffix(".md")
                    .expect("command page link ends in .md");
                assert!(
                    catalog.contains_key(name),
                    "{group}: link to a page of unknown command `{name}`"
                );
            } else {
                assert!(
                    base.join(target).is_file(),
                    "{group}: link target `{target}` does not exist"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires the pages of all commands listed in the group pages"]
fn group_page_links_point_to_existing_command_pages() {
    let link = Regex::new(r"\]\(\.\./commands/([^)#]+)\)").unwrap();
    for (group, page) in group_pages() {
        for capture in link.captures_iter(&page) {
            let path = root().join("docs/commands").join(&capture[1]);
            assert!(path.is_file(), "{group}: missing page {}", &capture[1]);
        }
    }
}

#[test]
fn group_pages_contain_no_environment_specific_text() {
    let comment = Regex::new(r"(?s)<!--.*?-->").unwrap();
    let decision = Regex::new(r"\b[A-Z]{1,4}\d?-\d+\b").unwrap();
    let section_number = Regex::new(r"\b[A-R]\d?\.\d+\b").unwrap();
    let mut pages = group_pages();
    pages.push((
        "sleep",
        fs::read_to_string(root().join("docs/commands/sleep.md")).unwrap(),
    ));
    for (name, page) in pages {
        let text = comment.replace_all(&page, "");
        for forbidden in ["/root/", "/srv/", "/mnt/", "/opt/"] {
            assert!(!text.contains(forbidden), "{name}: contains `{forbidden}`");
        }
        if let Some(c) = text.chars().find(|c| !c.is_ascii()) {
            panic!("{name}: non-ASCII character `{c}`");
        }
        for (position, _) in text.match_indices("/home/") {
            assert!(
                text[position..].starts_with("/home/user"),
                "{name}: home directory other than /home/user"
            );
        }
        for (position, _) in text.match_indices(":/Users/") {
            assert!(
                text[position..].starts_with(":/Users/user"),
                "{name}: home directory other than C:/Users/user"
            );
        }
        assert!(!decision.is_match(&text), "{name}: decision identifier");
        assert!(!section_number.is_match(&text), "{name}: section number");
    }
}

#[test]
fn group_page_external_markers_match_catalog() {
    let catalog = load_catalog();
    let expected_markers: BTreeMap<&str, char> = group_commands(&catalog)
        .into_iter()
        .filter_map(|(name, command)| command.marker().map(|marker| (name, marker)))
        .collect();
    let mut documented: BTreeMap<String, char> = BTreeMap::new();
    for (group, page) in group_pages() {
        let rows = match page.contains("\n## External programs\n") {
            true => first_table(section(&page, "External programs")),
            false => panic!("{group}: no section External programs"),
        };
        for row in rows {
            if row.len() < 2 || !row[1].starts_with('[') {
                continue;
            }
            let marker = row[1]
                .trim_matches(|c| c == '[' || c == ']')
                .chars()
                .next()
                .unwrap();
            documented.insert(unquote(&row[0]).to_owned(), marker);
        }
    }
    let documented: BTreeMap<&str, char> = documented
        .iter()
        .map(|(name, marker)| (name.as_str(), *marker))
        .collect();
    assert_eq!(documented, expected_markers);
}
