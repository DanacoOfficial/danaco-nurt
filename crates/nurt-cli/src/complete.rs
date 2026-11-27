// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::complete
// Responsibility:   Vocabulary of the interactive session: completion and highlighting of a.
// Dependencies:     nurt_core, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Vocabulary of the interactive session: completion and highlighting of a
//! program line.
//!
//! The vocabulary is read from the command catalog of the engine: the names
//! of the commands and the names of their parameters. The functions of this
//! module work on plain text; [`crate::repl`] adapts them to the line editor.

use nurt_core::Engine;
use serde_json::Value as Json;

/// Separator of the stages of a pipeline.
const PIPE: &str = "|>";

/// Words that the interactive session itself understands.
pub const BUILTINS: &[&str] = &["exit", "quit", "help", "jobs", "bg", "fg", "kill"];

/// A command and the names of its parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    /// Name of the command.
    pub name: String,
    /// Names of its parameters, in the order of the catalog.
    pub params: Vec<String>,
}

/// Role of a part of a program line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Name of a known command at the start of a stage.
    Command,
    /// Text in double quotes.
    Text,
    /// The stage separator.
    Pipe,
    /// White space.
    Space,
    /// Any other word.
    Word,
}

/// Names that complete a program line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vocabulary {
    commands: Vec<CommandInfo>,
}

fn param_names(value: &Json, names: &mut Vec<String>) {
    match value {
        Json::Object(map) => {
            if let Some(Json::Array(params)) = map.get("params") {
                names.extend(
                    params
                        .iter()
                        .filter_map(|param| param.get("name").and_then(Json::as_str))
                        .map(str::to_owned),
                );
            }
            map.iter()
                .filter(|(key, _)| key.as_str() != "params")
                .for_each(|(_, child)| param_names(child, names));
        }
        Json::Array(items) => items.iter().for_each(|item| param_names(item, names)),
        _ => {}
    }
}

fn collect(value: &Json, commands: &mut Vec<CommandInfo>) {
    match value {
        Json::Object(map) => {
            let described = map.contains_key("params")
                || map.contains_key("forms")
                || map.contains_key("summary");
            match map.get("name").and_then(Json::as_str) {
                Some(name) if described && !commands.iter().any(|known| known.name == name) => {
                    let mut params = Vec::new();
                    param_names(value, &mut params);
                    params.dedup();
                    commands.push(CommandInfo {
                        name: name.to_owned(),
                        params,
                    });
                }
                _ => map.values().for_each(|child| collect(child, commands)),
            }
        }
        Json::Array(items) => items.iter().for_each(|item| collect(item, commands)),
        _ => {}
    }
}

fn group_names(catalog: &Json) -> Vec<String> {
    catalog
        .get("groups")
        .and_then(Json::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| group.get("name").and_then(Json::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl Vocabulary {
    /// Vocabulary from catalog documents: every object with a name and
    /// parameters is a command.
    #[must_use]
    pub fn from_documents(documents: &[Json]) -> Self {
        let mut commands = Vec::new();
        documents
            .iter()
            .for_each(|document| collect(document, &mut commands));
        Self { commands }
    }

    /// Vocabulary of an engine. An engine without a catalog gives an empty
    /// vocabulary.
    #[must_use]
    pub fn from_engine(engine: &Engine) -> Self {
        let Ok(catalog) = engine.catalog() else {
            return Self::default();
        };
        let mut documents = vec![catalog.clone()];
        documents.extend(
            group_names(&catalog)
                .iter()
                .filter_map(|group| engine.describe(group, None).ok()),
        );
        Self::from_documents(&documents)
    }

    fn command(&self, name: &str) -> Option<&CommandInfo> {
        self.commands.iter().find(|command| command.name == name)
    }

    /// Whether `name` is a command of the catalog.
    #[must_use]
    pub fn knows(&self, name: &str) -> bool {
        self.command(name).is_some()
    }

    /// Completions of the word that ends at byte `pos` of `line`.
    ///
    /// Returns the start of that word and the candidates, sorted. The first
    /// word of a stage completes to command names (and, at the start of the
    /// line, to the words of the session); a later word completes to the
    /// parameters of the command, written with a trailing colon.
    #[must_use]
    pub fn complete(&self, line: &str, pos: usize) -> (usize, Vec<String>) {
        let pos = floor_boundary(line, pos);
        let head = &line[..pos];
        let after_space = head
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(index, c)| index + c.len_utf8());
        let stage_start = head.rfind(PIPE).map_or(0, |index| index + PIPE.len());
        let start = after_space.max(stage_start);
        let prefix = &head[start..];
        let stage = head[stage_start..start]
            .split_whitespace()
            .collect::<Vec<_>>();
        let mut found: Vec<String> = match stage.first() {
            None => {
                let builtins = if stage_start == 0 { BUILTINS } else { &[] };
                self.commands
                    .iter()
                    .map(|command| command.name.as_str())
                    .chain(builtins.iter().copied())
                    .filter(|name| name.starts_with(prefix))
                    .map(str::to_owned)
                    .collect()
            }
            Some(name) => self
                .command(name)
                .map(|command| {
                    command
                        .params
                        .iter()
                        .map(|param| format!("{param}:"))
                        .filter(|param| param.starts_with(prefix))
                        .collect()
                })
                .unwrap_or_default(),
        };
        found.sort();
        found.dedup();
        (start, found)
    }

    /// Splits `line` into parts with their roles; the parts together are the
    /// line.
    #[must_use]
    pub fn tokens(&self, line: &str) -> Vec<(TokenKind, String)> {
        let mut tokens: Vec<(TokenKind, String)> = Vec::new();
        let mut at_stage_start = true;
        let mut rest = line;
        while let Some(first) = rest.chars().next() {
            let length = if first.is_whitespace() {
                run_length(rest, char::is_whitespace)
            } else if first == '"' {
                string_length(rest)
            } else if rest.starts_with(PIPE) {
                PIPE.len()
            } else {
                run_length(rest, |c| !c.is_whitespace() && c != '"')
                    .min(rest.find(PIPE).unwrap_or(rest.len()).max(first.len_utf8()))
            };
            let (part, tail) = rest.split_at(length);
            let kind = if first.is_whitespace() {
                TokenKind::Space
            } else if first == '"' {
                at_stage_start = false;
                TokenKind::Text
            } else if part == PIPE {
                at_stage_start = true;
                TokenKind::Pipe
            } else if at_stage_start && self.knows(part) {
                at_stage_start = false;
                TokenKind::Command
            } else {
                at_stage_start = false;
                TokenKind::Word
            };
            tokens.push((kind, part.to_owned()));
            rest = tail;
        }
        tokens
    }
}

fn floor_boundary(text: &str, pos: usize) -> usize {
    let mut pos = pos.min(text.len());
    while !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn run_length(text: &str, accept: impl Fn(char) -> bool) -> usize {
    text.char_indices()
        .find(|(_, c)| !accept(*c))
        .map_or(text.len(), |(index, _)| index)
}

/// Length of the text literal at the start of `text`, including both quotes;
/// an unterminated literal runs to the end.
fn string_length(text: &str) -> usize {
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return index + 1,
            _ => escaped = false,
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn vocabulary() -> Vocabulary {
        Vocabulary::from_documents(&[json!({
            "format": "nurt-catalog-result/1",
            "commands": [
                {"name": "find", "summary": "list", "forms": [{"params": [{"name": "path"}, {"name": "name"}]}]},
                {"name": "remove", "params": [{"name": "path"}, {"name": "recursive"}]}
            ]
        })])
    }

    #[test]
    fn commands_and_parameters_are_read_from_documents() {
        let vocabulary = vocabulary();
        assert!(vocabulary.knows("find"));
        assert_eq!(
            vocabulary.command("find").map(|c| c.params.clone()),
            Some(vec!["path".to_owned(), "name".to_owned()])
        );
        assert!(!vocabulary.knows("copy"));
        assert_eq!(
            Vocabulary::from_documents(&[Json::Null]),
            Vocabulary::default()
        );
    }

    #[test]
    fn first_word_completes_to_commands_and_session_words() {
        let (start, found) = vocabulary().complete("re", 2);
        assert_eq!(start, 0);
        assert_eq!(found, vec!["remove".to_owned()]);
        let (_, all) = vocabulary().complete("", 0);
        assert!(all.contains(&"find".to_owned()) && all.contains(&"exit".to_owned()));
    }

    #[test]
    fn later_words_complete_to_parameters() {
        let (start, found) = vocabulary().complete("remove pa", 9);
        assert_eq!(start, 7);
        assert_eq!(found, vec!["path:".to_owned()]);
        assert!(vocabulary().complete("unknown pa", 10).1.is_empty());
    }

    #[test]
    fn a_stage_after_a_pipe_completes_to_commands_only() {
        let line = "find \"/srv\" |> re";
        let (start, found) = vocabulary().complete(line, line.len());
        assert_eq!(&line[start..], "re");
        assert_eq!(found, vec!["remove".to_owned()]);
        let (_, none) = vocabulary().complete("find |> ex", 10);
        assert!(none.is_empty());
    }

    #[test]
    fn completion_position_inside_a_character_is_rounded_down() {
        let (start, _) = vocabulary().complete("\u{142}a", 1);
        assert_eq!(start, 0);
    }

    #[test]
    fn tokens_cover_the_whole_line() {
        let line = "find \"/srv/a b\" |> remove path: \"x\\\"y\"  ";
        let tokens = vocabulary().tokens(line);
        let joined: String = tokens.iter().map(|(_, part)| part.as_str()).collect();
        assert_eq!(joined, line);
        let kinds: Vec<TokenKind> = tokens.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds[0], TokenKind::Command);
        assert!(kinds.contains(&TokenKind::Pipe));
        assert_eq!(
            tokens
                .iter()
                .filter(|(kind, _)| *kind == TokenKind::Text)
                .count(),
            2
        );
    }

    #[test]
    fn unterminated_text_runs_to_the_end() {
        let tokens = vocabulary().tokens("find \"/srv");
        assert_eq!(tokens.last().map(|(kind, _)| *kind), Some(TokenKind::Text));
    }

    #[test]
    fn position_beyond_the_line_is_the_end_of_the_line() {
        let (start, found) = vocabulary().complete("fi", 99);
        assert_eq!((start, found), (0, vec!["find".to_owned()]));
        let (start, found) = vocabulary().complete("", 5);
        assert_eq!(start, 0);
        assert!(found.contains(&"remove".to_owned()));
    }

    #[test]
    fn multi_byte_words_are_completed_at_byte_offsets() {
        let line = "\u{142}\u{f3}d\u{17a} |> re";
        let (start, found) = vocabulary().complete(line, line.len());
        assert_eq!(&line[start..], "re");
        assert_eq!(found, vec!["remove".to_owned()]);
        let (start, found) = vocabulary().complete("remove \u{142}a", "remove \u{142}a".len());
        assert_eq!(start, 7);
        assert!(found.is_empty());
    }

    #[test]
    fn parameters_are_listed_once_and_sorted() {
        let documents = [json!({"commands": [
            {"name": "sort", "params": [{"name": "order"}, {"name": "by"}, {"name": "by"}]}
        ]})];
        let vocabulary = Vocabulary::from_documents(&documents);
        let (_, found) = vocabulary.complete("sort ", 5);
        assert_eq!(found, vec!["by:".to_owned(), "order:".to_owned()]);
    }

    #[test]
    fn commands_found_twice_in_the_documents_are_kept_once() {
        let command = json!({"name": "find", "summary": "list", "params": [{"name": "path"}]});
        let vocabulary = Vocabulary::from_documents(&[command.clone(), command]);
        let (_, found) = vocabulary.complete("fi", 2);
        assert_eq!(found, vec!["find".to_owned()]);
    }

    #[test]
    fn commands_are_found_in_nested_documents_and_session_words_only_at_the_start() {
        let documents = [json!({"group": {"items": [{"name": "git.status", "forms": []}]}})];
        let vocabulary = Vocabulary::from_documents(&documents);
        assert!(vocabulary.knows("git.status"));
        let (_, at_start) = vocabulary.complete("ex", 2);
        assert_eq!(at_start, vec!["exit".to_owned()]);
        let (_, after_pipe) = vocabulary.complete("git.status |> ex", 16);
        assert!(after_pipe.is_empty());
    }

    #[test]
    fn the_stage_after_a_pipe_looks_up_its_own_command() {
        let line = "find \"/srv\" |> remove pa";
        let (start, found) = vocabulary().complete(line, line.len());
        assert_eq!(&line[start..], "pa");
        assert_eq!(found, vec!["path:".to_owned()]);
    }

    #[test]
    fn tokens_cover_multi_byte_text_and_crlf() {
        let line = "find \"\u{1f600}\" |>\r\n  remove";
        let tokens = vocabulary().tokens(line);
        let joined: String = tokens.iter().map(|(_, part)| part.as_str()).collect();
        assert_eq!(joined, line);
        assert!(tokens.contains(&(TokenKind::Text, "\"\u{1f600}\"".to_owned())));
        assert!(tokens.contains(&(TokenKind::Space, "\r\n  ".to_owned())));
    }

    #[test]
    fn a_name_that_is_not_a_command_is_a_word() {
        let tokens = vocabulary().tokens("frobnicate find");
        let kinds: Vec<TokenKind> = tokens.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            vec![TokenKind::Word, TokenKind::Space, TokenKind::Word]
        );
        assert!(vocabulary().tokens("").is_empty());
    }

    #[test]
    fn a_text_with_an_escaped_quote_is_one_token() {
        let tokens = vocabulary().tokens("\"a\\\"b\" c");
        assert_eq!(tokens[0], (TokenKind::Text, "\"a\\\"b\"".to_owned()));
        assert_eq!(string_length("\"a\\\\\" x"), 5);
    }

    #[test]
    fn words_that_touch_a_pipe_are_split() {
        let tokens = vocabulary().tokens("find|>remove");
        let kinds: Vec<TokenKind> = tokens.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            vec![TokenKind::Command, TokenKind::Pipe, TokenKind::Command]
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
