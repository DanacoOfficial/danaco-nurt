// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Token rules applied to program text before parsing.
//!
//! [`rewrite`] applies, in one pass over the tokens of the text, the rules
//! that do not need the type of an argument place:
//!
//! * `L03`, `L04`: letter case and naming convention of names: command names,
//!   argument names, keywords, function names, `let` names and their
//!   references, and bare value words. A name joined by hyphens (`open-files`)
//!   is joined only in a command place when the joined name is a command, and
//!   in an argument name place (`case-sensitive:`).
//! * `L07`: text in single or typographic quotes becomes text in double
//!   quotes, or raw text when it contains a quote or a backslash.
//! * `L13`: `|`, `| >`, `|->` and `->` between a complete operand and a command
//!   name, outside brackets other than parentheses, become `|>`.
//! * `L16`: the separator of a named argument follows the syntax variant: `:`
//!   in word and call notation, `=` in named notation; map entries always use
//!   `:`. The name of a `let` statement is never an argument name.
//! * `L21`: an unquoted path at the start of an argument is quoted.
//!
//! Literals and comments are never changed: the operators `->` and `|` inside
//! a text, a raw text, a regular expression or a comment stay as written.

use std::ops::Range;

use nurt_contracts::kinds::SyntaxVariant;

use super::CatalogView;
use super::literal::text_literal;
use super::locate::Locator;
use super::paths::quote_bare_path;
use super::rules::{Rule, number_corrections};
use super::scan::{Piece, PieceKind, scan};
use crate::normalize::{Edit, PositionMap, apply_edits};
use crate::record::{Correction, CorrectionKind};

/// Keywords of the language.
const KEYWORDS: [&str; 8] = ["let", "true", "false", "null", "and", "or", "not", "in"];

/// Keywords that continue an expression, after which no argument starts.
const OPERATOR_KEYWORDS: [&str; 4] = ["and", "or", "not", "in"];

/// The canonical pipe operator.
const PIPE: &str = "|>";

/// What the token rules need to know besides the text.
#[derive(Clone, Copy)]
pub struct TokenContext<'a> {
    /// Syntax variant of the program.
    pub variant: SyntaxVariant,
    /// Command catalog.
    pub catalog: &'a dyn CatalogView,
    /// Whether a word is a habit of another shell, such as `grep` or `ls`.
    pub is_habit: &'a dyn Fn(&str) -> bool,
}

/// Program text after the token rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewritten {
    /// Corrected text.
    pub text: String,
    /// Corrections, with positions in the text passed to [`rewrite`].
    pub corrections: Vec<Correction>,
    /// Map from positions of [`Rewritten::text`] to the text passed to
    /// [`rewrite`].
    pub positions: PositionMap,
}

/// Applies the token rules to a program text.
///
/// The function is total and never changes a literal or a comment. Applied to
/// its own result it makes no further correction.
///
/// # Examples
///
/// ```
/// use nurt_contracts::kinds::SyntaxVariant;
/// use nurt_correct::lexical::tokens::{TokenContext, rewrite};
/// use nurt_correct::lexical::{ArgumentShape, CatalogView};
///
/// struct Commands;
///
/// impl CatalogView for Commands {
///     fn is_command(&self, name: &str) -> bool {
///         ["find", "search"].contains(&name)
///     }
///     fn command_names(&self) -> &[&str] {
///         &["find", "search"]
///     }
///     fn is_mutating(&self, _: &str) -> bool {
///         false
///     }
///     fn can_take_argument(&self, _: &str, _: usize, _: ArgumentShape) -> bool {
///         false
///     }
/// }
///
/// let context = TokenContext {
///     variant: SyntaxVariant::A,
///     catalog: &Commands,
///     is_habit: &|_| false,
/// };
/// let rewritten = rewrite("Find /srv/app glob: '*.go' | search /x/", &context);
/// assert_eq!(rewritten.text, "find \"/srv/app\" glob: \"*.go\" |> search /x/");
/// assert_eq!(rewritten.corrections.len(), 4);
/// ```
#[must_use]
pub fn rewrite(text: &str, context: &TokenContext<'_>) -> Rewritten {
    let locator = Locator::identity(text);
    let (text, corrections, positions) = rewrite_located(text, context, &locator);
    Rewritten {
        text,
        corrections,
        positions,
    }
}

/// Applies the token rules, locating corrections with `locator`.
pub(crate) fn rewrite_located(
    text: &str,
    context: &TokenContext<'_>,
    locator: &Locator<'_>,
) -> (String, Vec<Correction>, PositionMap) {
    let pieces = scan(text);
    let mut pass = Pass {
        text,
        significant: pieces
            .iter()
            .filter(|piece| !piece.kind.is_trivia())
            .cloned()
            .collect(),
        context,
        locator,
        changes: Vec::new(),
        brackets: Vec::new(),
        command_expected: true,
        let_name_expected: false,
    };
    pass.run();
    let mut changes = pass.changes;
    changes.sort_by_key(|(edit, _)| edit.range.start);
    let (edits, mut corrections): (Vec<Edit>, Vec<Correction>) = changes.into_iter().unzip();
    number_corrections(&mut corrections);
    let (corrected, positions) = apply_edits(text, &edits);
    (corrected, corrections, positions)
}

struct Pass<'a, 'c> {
    text: &'a str,
    significant: Vec<Piece>,
    context: &'c TokenContext<'c>,
    locator: &'c Locator<'c>,
    changes: Vec<(Edit, Correction)>,
    brackets: Vec<char>,
    command_expected: bool,
    let_name_expected: bool,
}

impl Pass<'_, '_> {
    fn run(&mut self) {
        let mut index = 0;
        while index < self.significant.len() {
            index = self.step(index);
        }
    }

    fn piece(&self, index: usize) -> Option<&Piece> {
        self.significant.get(index)
    }

    fn slice(&self, index: usize) -> &str {
        self.piece(index).map_or("", |piece| piece.text(self.text))
    }

    fn kind(&self, index: usize) -> Option<PieceKind> {
        self.piece(index).map(|piece| piece.kind)
    }

    /// Whether the significant pieces at `left` and `left + 1` touch.
    fn glued(&self, left: usize) -> bool {
        match (self.piece(left), self.piece(left + 1)) {
            (Some(a), Some(b)) => a.range.end == b.range.start,
            _ => false,
        }
    }

    fn change(&mut self, range: Range<usize>, rule: Rule, to: &str, kind: CorrectionKind) {
        let from = self.text.get(range.clone()).unwrap_or_default();
        let correction = rule.record(self.locator.at(range.clone()), from, to, kind);
        self.changes.push((Edit::new(range, to), correction));
    }

    /// Handles the significant piece at `index` and returns the index of the
    /// next piece to handle.
    fn step(&mut self, index: usize) -> usize {
        let Some(kind) = self.kind(index) else {
            return index + 1;
        };
        match kind {
            PieceKind::Newline => {
                if self.brackets.is_empty() {
                    self.command_expected = true;
                }
                index + 1
            }
            PieceKind::Word => self.word(index),
            PieceKind::Punct => self.punct(index),
            PieceKind::SingleQuoted | PieceKind::Typographic => {
                self.quotes(index);
                self.command_expected = false;
                index + 1
            }
            PieceKind::BarePath => {
                self.bare_path(index);
                self.command_expected = false;
                index + 1
            }
            _ => {
                self.command_expected = false;
                index + 1
            }
        }
    }

    fn punct(&mut self, index: usize) -> usize {
        let operator = self.slice(index).to_owned();
        match operator.as_str() {
            "(" | "[" | "{" => {
                self.brackets.push(operator.chars().next().unwrap_or('('));
                self.command_expected = operator == "(" && self.command_follows(index + 1);
                index + 1
            }
            ")" | "]" | "}" => {
                self.brackets.pop();
                self.command_expected = false;
                index + 1
            }
            PIPE => {
                self.command_expected = true;
                index + 1
            }
            "|" | "|->" | "->" => self.pipe(index),
            _ => {
                self.command_expected = false;
                index + 1
            }
        }
    }

    /// Rule `L13`.
    fn pipe(&mut self, index: usize) -> usize {
        let spaced_gt = self.slice(index) == "|" && self.slice(index + 1) == ">";
        let last = if spaced_gt { index + 1 } else { index };
        let between_stages =
            self.at_stage_level() && self.operand_before(index) && self.command_follows(last + 1);
        if !between_stages {
            self.command_expected = false;
            return index + 1;
        }
        let (Some(first), Some(end)) = (self.piece(index), self.piece(last)) else {
            return index + 1;
        };
        let range = first.range.start..end.range.end;
        self.change(range, Rule::L13, PIPE, CorrectionKind::Pipe);
        self.command_expected = true;
        last + 1
    }

    fn at_stage_level(&self) -> bool {
        self.brackets.iter().all(|&bracket| bracket == '(')
    }

    /// Whether a complete operand or a command name precedes the piece at
    /// `index`, possibly on the previous line.
    fn operand_before(&self, index: usize) -> bool {
        let mut before = index;
        while before > 0 {
            before -= 1;
            match self.kind(before) {
                Some(PieceKind::Newline) => {}
                Some(kind) if kind.ends_operand() => {
                    return !(kind == PieceKind::Word
                        && OPERATOR_KEYWORDS.contains(&fold(self.slice(before)).as_str()));
                }
                Some(PieceKind::Punct) => return matches!(self.slice(before), ")" | "]" | "}"),
                _ => return false,
            }
        }
        false
    }

    /// Whether a command name or a habit word starts at `index`.
    fn command_follows(&self, index: usize) -> bool {
        if self.kind(index) != Some(PieceKind::Word) {
            return false;
        }
        let (_, name) = self.name_chain(index, true);
        let word = self.slice(index);
        self.context.catalog.is_command(&fold(&name)) || (self.context.is_habit)(word)
    }

    /// Joins words connected by `.` or, when `hyphens` is set, by `-`, with no
    /// space between them. Returns the index of the last word and the text.
    fn name_chain(&self, index: usize, hyphens: bool) -> (usize, String) {
        let mut last = index;
        let mut name = self.slice(index).to_owned();
        while self.glued(last) && self.glued(last + 1) {
            let joiner = self.slice(last + 1);
            let joins = joiner == "." || (hyphens && joiner == "-");
            if !joins || self.kind(last + 2) != Some(PieceKind::Word) {
                break;
            }
            name.push_str(joiner);
            name.push_str(self.slice(last + 2));
            last += 2;
        }
        (last, name)
    }

    fn word(&mut self, index: usize) -> usize {
        if self.command_expected {
            return self.command_word(index);
        }
        if self.let_name_expected {
            self.let_name_expected = false;
            self.fix_name(index, index, CorrectionKind::Ref);
            if self.slice(index + 1) == "=" {
                self.command_expected = true;
                return index + 2;
            }
            return index + 1;
        }
        let (hyphen_last, _) = self.name_chain(index, true);
        if let Some(separator) = self.named_separator(hyphen_last) {
            // The key of a map entry is data, such as the name of an
            // environment variable, so its case is kept.
            if self.brackets.last() != Some(&'{') {
                self.fix_name(index, hyphen_last, CorrectionKind::Param);
            }
            self.separator(separator);
            return separator + 1;
        }
        let (last, _) = self.name_chain(index, false);
        if !self.is_flag_word(index) {
            let name_kind = self.value_word_kind(index, last);
            self.fix_name(index, last, name_kind);
        }
        last + 1
    }

    fn command_word(&mut self, index: usize) -> usize {
        self.command_expected = false;
        let word = fold(self.slice(index));
        if word == "let" {
            self.fix_name(index, index, CorrectionKind::Keyword);
            self.let_name_expected = true;
            return index + 1;
        }
        let (hyphen_last, hyphen_name) = self.name_chain(index, true);
        let (dot_last, _) = self.name_chain(index, false);
        if hyphen_last > dot_last {
            if self.context.catalog.is_command(&fold(&hyphen_name)) {
                self.fix_name(index, hyphen_last, CorrectionKind::Command);
            }
            return hyphen_last + 1;
        }
        self.fix_name(index, dot_last, CorrectionKind::Command);
        dot_last + 1
    }

    /// Whether the word at `index` is the name of a flag of another shell:
    /// `-Path`, `--name`, with the dashes glued to the word and a space before
    /// them.
    fn is_flag_word(&self, index: usize) -> bool {
        let Some(dash) = index.checked_sub(1) else {
            return false;
        };
        let dashes = matches!(self.slice(dash), "-" | "--");
        let spaced = match (
            dash.checked_sub(1).and_then(|before| self.piece(before)),
            self.piece(dash),
        ) {
            (Some(before), Some(dash_piece)) => before.range.end < dash_piece.range.start,
            _ => true,
        };
        dashes && self.glued(dash) && spaced
    }

    fn value_word_kind(&self, index: usize, last: usize) -> CorrectionKind {
        let after_at = index > 0 && self.slice(index - 1) == "@" && self.glued(index - 1);
        if after_at {
            return CorrectionKind::Ref;
        }
        if self.glued(last) && self.slice(last + 1) == "(" {
            return CorrectionKind::Function;
        }
        if KEYWORDS.contains(&fold(self.slice(index)).as_str()) {
            return CorrectionKind::Keyword;
        }
        CorrectionKind::Value
    }

    /// Index of a `:` or `=` glued to the name ending at `last`, when the name
    /// is the name of a named argument or a map key.
    fn named_separator(&self, last: usize) -> Option<usize> {
        let separator = last + 1;
        let is_separator = matches!(self.slice(separator), ":" | "=");
        (is_separator && self.glued(last)).then_some(separator)
    }

    /// Rules `L03` and `L04` on the name from piece `first` to piece `last`.
    fn fix_name(&mut self, first: usize, last: usize, name_kind: CorrectionKind) {
        let (Some(start), Some(end)) = (self.piece(first), self.piece(last)) else {
            return;
        };
        let range = start.range.start..end.range.end;
        let written = self.text.get(range.clone()).unwrap_or_default();
        let folded = fold(written);
        if folded == written {
            return;
        }
        let rule = if folded == written.to_ascii_lowercase() {
            Rule::L03
        } else {
            Rule::L04
        };
        self.change(range, rule, &folded, name_kind);
    }

    /// Rule `L16` on the separator of a named argument or map entry.
    fn separator(&mut self, index: usize) {
        let in_map = self.brackets.last() == Some(&'{');
        let wanted = match self.context.variant {
            SyntaxVariant::C if !in_map && self.brackets.last() != Some(&'[') => "=",
            _ => ":",
        };
        let written = self.slice(index).to_owned();
        let Some(range) = self.piece(index).map(|piece| piece.range.clone()) else {
            return;
        };
        if written == wanted {
            return;
        }
        let rest = self.text.get(range.end..).unwrap_or_default();
        let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let (edit_range, replacement) = if wanted == "=" {
            (range.start..range.end + spaces, "=")
        } else if spaces == 0 && !rest.is_empty() && !rest.starts_with(['\n', '\r']) {
            (range.clone(), ": ")
        } else {
            (range.clone(), ":")
        };
        let correction = Rule::L16.record(
            self.locator.at(range),
            written,
            wanted,
            CorrectionKind::Separator,
        );
        self.changes
            .push((Edit::new(edit_range, replacement), correction));
    }

    /// Rule `L07`.
    fn quotes(&mut self, index: usize) {
        let Some(range) = self.piece(index).map(|piece| piece.range.clone()) else {
            return;
        };
        let written = self.slice(index);
        let mut chars = written.chars();
        let (Some(open), Some(close)) = (chars.next(), chars.next_back()) else {
            return;
        };
        let content = written
            .get(open.len_utf8()..written.len() - close.len_utf8())
            .unwrap_or_default();
        let to = text_literal(content);
        self.change(range, Rule::L07, &to, CorrectionKind::Quote);
    }

    /// Rule `L21`.
    fn bare_path(&mut self, index: usize) {
        if !self.argument_starts(index) {
            return;
        }
        let Some(range) = self.piece(index).map(|piece| piece.range.clone()) else {
            return;
        };
        let written = self.slice(index);
        if let Some(fix) = quote_bare_path(written, self.locator.at(range.clone())) {
            let to = fix.correction.to.clone();
            self.changes.push((Edit::new(range, to), fix.correction));
        }
    }

    /// Whether an argument can start at `index`: after a command name, after
    /// a complete argument, after the separator of a named argument or after
    /// an opening bracket or a comma.
    fn argument_starts(&self, index: usize) -> bool {
        let Some(before) = index.checked_sub(1) else {
            return false;
        };
        match self.kind(before) {
            Some(PieceKind::Punct) => matches!(
                self.slice(before),
                ":" | "=" | "(" | "[" | "," | ")" | "]" | "}"
            ),
            Some(PieceKind::Word) => {
                !OPERATOR_KEYWORDS.contains(&fold(self.slice(before)).as_str())
            }
            Some(kind) => kind.ends_operand(),
            None => false,
        }
    }
}

/// Writes a name in lower-case `snake_case`, keeping dots: `caseSensitive`,
/// `case-sensitive` and `CaseSensitive` all give `case_sensitive`, and
/// `git.Status` gives `git.status`.
pub(crate) fn fold(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (index, &c) in chars.iter().enumerate() {
        if c == '-' {
            out.push('_');
            continue;
        }
        if c.is_ascii_uppercase() && index > 0 {
            let previous = chars.get(index - 1).copied().unwrap_or(' ');
            let next = chars.get(index + 1).copied().unwrap_or(' ');
            let word_start = previous.is_ascii_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next.is_ascii_lowercase());
            if word_start && !out.ends_with('_') && !out.ends_with('.') {
                out.push('_');
            }
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use nurt_contracts::report::{CorrectionClass, TextPosition};

    use super::*;
    use crate::lexical::ArgumentShape;

    const COMMANDS: [&str; 10] = [
        "find",
        "search",
        "count",
        "filter",
        "replace",
        "open_files",
        "run",
        "verify",
        "git.status",
        "list",
    ];

    struct Commands;

    impl CatalogView for Commands {
        fn is_command(&self, name: &str) -> bool {
            COMMANDS.contains(&name)
        }
        fn command_names(&self) -> &[&str] {
            &COMMANDS
        }
        fn is_mutating(&self, name: &str) -> bool {
            name == "replace"
        }
        fn can_take_argument(&self, _: &str, _: usize, _: ArgumentShape) -> bool {
            false
        }
    }

    fn habit(word: &str) -> bool {
        ["grep", "ls"].contains(&word)
    }

    fn run_in(variant: SyntaxVariant, text: &str) -> Rewritten {
        let context = TokenContext {
            variant,
            catalog: &Commands,
            is_habit: &habit,
        };
        rewrite(text, &context)
    }

    fn run(text: &str) -> Rewritten {
        run_in(SyntaxVariant::A, text)
    }

    fn rules(rewritten: &Rewritten) -> Vec<String> {
        rewritten
            .corrections
            .iter()
            .map(|correction| correction.rule.to_string())
            .collect()
    }

    #[test]
    fn folds_names_to_snake_case() {
        assert_eq!(fold("caseSensitive"), "case_sensitive");
        assert_eq!(fold("CaseSensitive"), "case_sensitive");
        assert_eq!(fold("case-sensitive"), "case_sensitive");
        assert_eq!(fold("GLOB"), "glob");
        assert_eq!(fold("HTTPRequest"), "http_request");
        assert_eq!(fold("git.Status"), "git.status");
        assert_eq!(fold("find"), "find");
    }

    #[test]
    fn keys_of_a_map_keep_their_case() {
        let text = "run [\"env\"] cwd: \"/srv\" env: {CI: \"1\", NODE_ENV: \"test\"}";
        let rewritten = run(text);
        assert_eq!(rewritten.text, text);
        assert!(rewritten.corrections.is_empty(), "{:?}", rules(&rewritten));
        let named = run("run [\"env\"] Cwd: \"/srv\"");
        assert_eq!(named.text, "run [\"env\"] cwd: \"/srv\"");
        assert_eq!(rules(&named), ["L03"]);
    }

    #[test]
    fn makes_no_correction_in_a_valid_program() {
        let text = "find \"/srv/app\" glob: \"**/*.go\"\n|> filter size > 1mb and modified > ago(24h)\n|> first 20";
        let rewritten = run(text);
        assert_eq!(rewritten.text, text);
        assert!(rewritten.corrections.is_empty());
    }

    #[test]
    fn writes_names_in_lower_case() {
        let rewritten = run("Find \"/srv/app\" Glob: \"*.go\" caseSensitive: false |> Count");
        assert_eq!(
            rewritten.text,
            "find \"/srv/app\" glob: \"*.go\" case_sensitive: false |> count"
        );
        assert_eq!(rules(&rewritten), ["L03", "L03", "L04", "L03"]);
        let kinds: Vec<_> = rewritten
            .corrections
            .iter()
            .map(|correction| correction.kind.as_str())
            .collect();
        assert_eq!(kinds, ["command", "param", "param", "command"]);
        assert_eq!(
            rewritten.corrections[2].at,
            Some(TextPosition {
                line: 1,
                column: 30,
                length: Some(13)
            })
        );
    }

    #[test]
    fn joins_hyphenated_argument_names_and_known_commands() {
        let rewritten = run("find \"/a\" case-sensitive: false\nopen-files \"/a\"");
        assert_eq!(
            rewritten.text,
            "find \"/a\" case_sensitive: false\nopen_files \"/a\""
        );
        assert_eq!(rules(&rewritten), ["L04", "L04"]);
    }

    #[test]
    fn keeps_subtraction_of_hyphenated_value_words() {
        let text = "filter open-files > 0";
        assert_eq!(run(text).text, text);
    }

    #[test]
    fn keeps_unknown_hyphenated_command_words() {
        let text = "Get-ChildItem -Path x";
        assert_eq!(run(text).text, text);
    }

    #[test]
    fn writes_quotes_as_double_quotes_or_raw_text() {
        let rewritten = run("replace 'newName(' „say \"hi\"” 'C:\\temp'");
        assert_eq!(
            rewritten.text,
            "replace \"newName(\" r#\"say \"hi\"\"# r\"C:\\temp\""
        );
        assert_eq!(rules(&rewritten), ["L07", "L07", "L07"]);
        assert_eq!(rewritten.corrections[0].class, CorrectionClass::Certain);
    }

    #[test]
    fn writes_every_pipe_notation_between_stages_as_pipe() {
        for pipe in ["|", "| >", "|->", "->"] {
            let rewritten = run(&format!("find \"/a\" {pipe} search /x/"));
            assert_eq!(rewritten.text, "find \"/a\" |> search /x/", "{pipe}");
            assert_eq!(rules(&rewritten), ["L13"]);
            assert_eq!(rewritten.corrections[0].from, pipe);
        }
    }

    #[test]
    fn keeps_arrows_and_bars_inside_literals_and_comments() {
        for text in [
            "replace \"a -> b\" \"c\"",
            "search /x|y/ \"/a\"",
            "search /a->b/ \"/a\"",
            "replace r#\"a | b\"# \"c\"",
            "find \"/a\" # find -> search",
        ] {
            let rewritten = run(text);
            assert_eq!(rewritten.text, text);
            assert!(rewritten.corrections.is_empty(), "{text}");
        }
    }

    #[test]
    fn keeps_pipes_not_followed_by_a_command() {
        for text in [
            "find \"/a\" | xyz",
            "filter size -> 10",
            "find \"/a\" || list \"/b\"",
            "filter [1 | count]",
        ] {
            assert_eq!(run(text).text, text);
        }
    }

    #[test]
    fn corrects_a_pipe_before_a_habit_word_and_at_line_start() {
        assert_eq!(run("find \"/a\" | grep x").text, "find \"/a\" |> grep x");
        assert_eq!(
            run("find \"/a\"\n| search /x/").text,
            "find \"/a\"\n|> search /x/"
        );
    }

    #[test]
    fn writes_named_argument_separators_per_variant() {
        assert_eq!(
            run("find \"/a\" glob=\"*.go\"").text,
            "find \"/a\" glob: \"*.go\""
        );
        assert_eq!(
            run_in(SyntaxVariant::C, "find path: \"/a\" glob=\"*.go\"").text,
            "find path=\"/a\" glob=\"*.go\""
        );
        assert_eq!(
            run_in(
                SyntaxVariant::C,
                "edit path=\"/a\" changes=[{replace: \"x\", with: \"y\"}]"
            )
            .text,
            "edit path=\"/a\" changes=[{replace: \"x\", with: \"y\"}]"
        );
        assert_eq!(run("let x = find \"/a\"").text, "let x = find \"/a\"");
        assert_eq!(run("filter a == b").text, "filter a == b");
    }

    #[test]
    fn quotes_bare_paths_at_argument_start_only() {
        assert_eq!(
            run("find /srv/app glob: \"*.go\"").text,
            "find \"/srv/app\" glob: \"*.go\""
        );
        assert_eq!(run("list C:\\proj").text, "list r\"C:\\proj\"");
        assert_eq!(run("find /etc/").text, "find /etc/");
        assert_eq!(run("filter path == /a/b").text, "filter path == /a/b");
    }

    #[test]
    fn lower_cases_keywords_values_functions_and_let_names() {
        let rewritten = run(
            "Let Recent = find \"/a\" type: File hidden: True |> filter modified > Ago(1h)\n@Recent |> count",
        );
        assert_eq!(
            rewritten.text,
            "let recent = find \"/a\" type: file hidden: true |> filter modified > ago(1h)\n@recent |> count"
        );
        let kinds: Vec<_> = rewritten
            .corrections
            .iter()
            .map(|correction| correction.kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            ["keyword", "ref", "value", "keyword", "function", "ref"]
        );
    }

    #[test]
    fn rewriting_twice_makes_no_further_correction() {
        let first = run("Find /srv/app glob: '*.go' | Filter Size > 1 |-> Count");
        let second = run(&first.text);
        assert!(second.corrections.is_empty());
        assert_eq!(second.text, first.text);
    }

    #[test]
    fn positions_point_into_the_text_as_written() {
        let rewritten = run("find \"αβγδεζ\" | search /x/");
        assert_eq!(
            rewritten.corrections[0].at,
            Some(TextPosition {
                line: 1,
                column: 15,
                length: Some(1)
            })
        );
        assert_eq!(rewritten.positions.to_original(rewritten.text.len()), 32);
    }

    #[test]
    fn treats_dotted_command_names_as_one_name() {
        assert_eq!(run("git.Status").text, "git.status");
        assert_eq!(
            run("find \"/a\" | git.status").text,
            "find \"/a\" |> git.status"
        );
    }

    #[test]
    fn sub_pipeline_in_parentheses_starts_with_a_command() {
        assert_eq!(
            run("verify (Run [\"go\"] cwd: \"/a\" | count)").text,
            "verify (run [\"go\"] cwd: \"/a\" |> count)"
        );
    }
}
