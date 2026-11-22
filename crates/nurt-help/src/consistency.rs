// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Consistency of program texts with the catalog.
//!
//! Examples in the help, in the description layers, in guides and in command
//! pages name commands, parameters and enumeration values. This module
//! checks those names against the catalog without parsing the whole language:
//! it splits a program into stages and words and compares the command of each
//! stage and every `name:` argument with the catalog.

use crate::catalog::Catalog;

/// Words that are valid values of an enumeration parameter in any command.
const LITERAL_WORDS: &[&str] = &["true", "false", "null"];

/// Checks the programs in `text`, one program per line, and returns one
/// message for each name the catalog does not know.
///
/// Lines that start with `|>` continue the previous line and are checked as
/// stages. Words that follow `@name` references and expressions are not
/// checked.
///
/// # Examples
///
/// ```
/// # use nurt_help::catalog::Catalog;
/// # use nurt_help::docs::Docs;
/// # fn main() -> Result<(), nurt_help::catalog::CatalogError> {
/// # let index = "format = \"nurt-catalog-index/1\"\ntotal = 1\n[[file]]\nname = \"core\"\nfile = \"core.toml\"\nsummary = \"Core.\"\ncommands = 1\n";
/// # let core = "format = \"nurt-catalog/1\"\ngroup = \"core\"\nsummary = \"Core.\"\n[[command]]\nname = \"count\"\ncategory = \"transform\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"any\"]\noutput = [\"int\"]\nsummary = \"Count.\"\n";
/// # let catalog = Catalog::from_sources(index, &[("core.toml", core)], Docs::empty())?;
/// use nurt_help::consistency::check_program;
///
/// assert!(check_program(&catalog, "count").is_empty());
/// assert_eq!(check_program(&catalog, "cuont").len(), 1);
/// # Ok(())
/// # }
/// ```
#[must_use]
pub fn check_program(catalog: &Catalog, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        for stage in split_stages(line) {
            check_stage(catalog, &stage, &mut problems);
        }
    }
    problems
}

fn check_stage(catalog: &Catalog, stage: &str, problems: &mut Vec<String>) {
    let words = split_words(stage);
    let mut words = words.as_slice();
    if words.len() >= 3 && words[0] == "let" && words[2] == "=" {
        words = &words[3..];
    }
    let Some(first) = words.first() else {
        return;
    };
    if first.starts_with('@') || first.starts_with("...") {
        return;
    }
    let Some(spec) = catalog.command(first) else {
        problems.push(format!("unknown command `{first}`"));
        return;
    };
    if spec.name == "select" {
        return;
    }
    for (position, word) in words.iter().enumerate().skip(1) {
        let Some(name) = word
            .strip_suffix(':')
            .filter(|name| is_parameter_name(name))
        else {
            continue;
        };
        let Some(param) = spec.param(name) else {
            problems.push(format!("command `{}` has no parameter `{name}`", spec.name));
            continue;
        };
        let value = words.get(position + 1).map(String::as_str);
        if let Some(value) = value.filter(|value| is_bare_word(value))
            && !param.values.is_empty()
            && !param.values.iter().any(|allowed| allowed == value)
            && !LITERAL_WORDS.contains(&value)
        {
            problems.push(format!(
                "`{value}` is not a value of `{}` parameter `{name}`",
                spec.name
            ));
        }
    }
}

fn is_parameter_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_bare_word(text: &str) -> bool {
    is_parameter_name(text)
}

/// Position state while scanning a program.
#[derive(Default)]
struct Scan {
    in_string: bool,
    in_regex: bool,
    escaped: bool,
    depth: usize,
}

impl Scan {
    /// Advances over `ch` and tells whether it is at the top level, outside
    /// strings, regular expressions and brackets.
    fn step(&mut self, previous: Option<char>, ch: char, next: Option<char>) -> bool {
        if self.escaped {
            self.escaped = false;
            return false;
        }
        if self.in_string || self.in_regex {
            match ch {
                '\\' => self.escaped = true,
                '"' if self.in_string => self.in_string = false,
                '/' if self.in_regex => self.in_regex = false,
                _ => {}
            }
            return false;
        }
        match ch {
            '"' => {
                self.in_string = true;
                false
            }
            '/' if starts_regex(previous, next) => {
                self.in_regex = true;
                false
            }
            '(' | '[' | '{' => {
                self.depth += 1;
                false
            }
            ')' | ']' | '}' => {
                self.depth = self.depth.saturating_sub(1);
                false
            }
            _ => self.depth == 0,
        }
    }
}

fn starts_regex(previous: Option<char>, next: Option<char>) -> bool {
    let after_space = previous.is_none_or(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | ','));
    after_space && next.is_some_and(|ch| !ch.is_whitespace())
}

fn split_stages(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut scan = Scan::default();
    let mut stages = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    while index < chars.len() {
        let previous = index.checked_sub(1).map(|at| chars[at]);
        let next = chars.get(index + 1).copied();
        let top = scan.step(previous, chars[index], next);
        if top && chars[index] == '|' && next == Some('>') {
            stages.push(std::mem::take(&mut current));
            index += 2;
            continue;
        }
        current.push(chars[index]);
        index += 1;
    }
    stages.push(current);
    stages
        .into_iter()
        .map(|stage| stage.trim().to_owned())
        .filter(|stage| !stage.is_empty())
        .collect()
}

fn split_words(stage: &str) -> Vec<String> {
    let chars: Vec<char> = stage.chars().collect();
    let mut scan = Scan::default();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, ch) in chars.iter().enumerate() {
        let previous = index.checked_sub(1).map(|at| chars[at]);
        let next = chars.get(index + 1).copied();
        let top = scan.step(previous, *ch, next);
        if top && ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(*ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_split_at_top_level_pipes_only() {
        let stages =
            split_stages(r#"find "/a|>b" glob: "*.go" |> verify (run ["x"] |> count) |> count"#);
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[2], "count");
    }

    #[test]
    fn words_keep_strings_lists_and_regular_expressions_whole() {
        let words = split_words(r#"search /a b\/c/i "/x y" glob: ["a", "b"]"#);
        assert_eq!(
            words,
            [
                "search",
                r"/a b\/c/i",
                r#""/x y""#,
                "glob:",
                r#"["a", "b"]"#
            ]
        );
    }

    #[test]
    fn division_is_not_a_regular_expression() {
        let words = split_words("filter size / 2 > 1");
        assert_eq!(words.len(), 6);
    }

    #[test]
    fn parameter_names_are_lowercase_identifiers() {
        assert!(is_parameter_name("exit_code"));
        assert!(!is_parameter_name("Exit"));
        assert!(!is_parameter_name("1x"));
        assert!(!is_parameter_name(""));
    }
}
