// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Commit messages: cleanup, trailers and sign-off lines.
//!
//! The functions reproduce the message handling of `git commit -m` and of
//! `git interpret-trailers` with the default trailer configuration
//! (`trailer.where = end`, `trailer.ifExists = addIfDifferentNeighbor`,
//! `trailer.ifMissing = add`). Every function takes and returns complete
//! messages; a non-empty result always ends with a line feed.
//!
//! # Examples
//!
//! ```
//! use nurt_git::message::{
//!     Cleanup, Trailer, append_trailers, cleanup, reflog_subject, sign_off, trailers,
//! };
//!
//! let text = cleanup("feat: add parser  \n\n\n", Cleanup::Whitespace, '#');
//! let text = sign_off(&text, "Ada <ada@example.com>");
//! let refs = Trailer::parse("Refs=12").unwrap();
//! let text = append_trailers(&text, &[refs.clone()]);
//! assert_eq!(text, "feat: add parser\n\nSigned-off-by: Ada <ada@example.com>\nRefs: 12\n");
//! assert_eq!(append_trailers(&text, &[refs]), text);
//! assert_eq!(trailers(&text).get("Refs").map(String::as_str), Some("12"));
//! assert_eq!(reflog_subject(&text), "feat: add parser");
//! assert_eq!(Cleanup::from_config(Some("strip")), Ok(Cleanup::Strip));
//! assert_eq!(cleanup("title\n# note\n", Cleanup::Strip, '#'), "title\n");
//! ```

use std::fmt;

use indexmap::IndexMap;

/// Token of the trailer added by a sign-off.
pub const SIGN_OFF_TOKEN: &str = "Signed-off-by";

/// Prefixes that mark a paragraph as a trailer block even when it also
/// contains other lines.
const RECOGNIZED_PREFIXES: [&str; 2] = ["Signed-off-by: ", "(cherry picked from commit "];

/// A paragraph with more than this many non-trailer lines per trailer line is
/// not a trailer block, even with a recognized prefix (`git` accepts a block
/// in which at least 25% of the lines are trailers).
const NON_TRAILER_LINES_PER_TRAILER: usize = 3;

/// Cleanup mode of a commit message, as set by `commit.cleanup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cleanup {
    /// The message is used as given.
    Verbatim,
    /// Trailing white space of every line, leading and trailing blank lines
    /// and repeated blank lines are removed.
    Whitespace,
    /// As [`Cleanup::Whitespace`], and comment lines are removed.
    Strip,
}

impl Cleanup {
    /// Returns the mode for a message given on the command line, from the
    /// value of `commit.cleanup`.
    ///
    /// The values `default` and `scissors` select [`Cleanup::Whitespace`],
    /// because both behave so when no editor is used.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidCleanup`] for a value that `git` does not accept.
    pub fn from_config(value: Option<&str>) -> Result<Self, InvalidCleanup> {
        match value.map(str::to_ascii_lowercase).as_deref() {
            None | Some("default" | "scissors" | "whitespace") => Ok(Self::Whitespace),
            Some("strip") => Ok(Self::Strip),
            Some("verbatim") => Ok(Self::Verbatim),
            Some(other) => Err(InvalidCleanup {
                value: other.to_owned(),
            }),
        }
    }
}

/// Value of `commit.cleanup` that is not a cleanup mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidCleanup {
    /// The rejected value.
    pub value: String,
}

impl fmt::Display for InvalidCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "commit.cleanup has the invalid value `{}`; use strip, whitespace, verbatim, scissors or default",
            self.value
        )
    }
}

impl std::error::Error for InvalidCleanup {}

/// Cleans up a message as `git commit` does before it records the message.
///
/// The comment character (`core.commentChar`) is used only by
/// [`Cleanup::Strip`]. The result is empty when the message has no content.
#[must_use]
pub fn cleanup(message: &str, mode: Cleanup, comment: char) -> String {
    if mode == Cleanup::Verbatim {
        return message.to_owned();
    }
    let mut out = String::with_capacity(message.len());
    let mut pending_blank = false;
    for line in message.split('\n') {
        if mode == Cleanup::Strip && line.starts_with(comment) {
            continue;
        }
        let line = line.trim_end_matches(is_git_space);
        if line.is_empty() {
            pending_blank = !out.is_empty();
            continue;
        }
        if pending_blank {
            out.push('\n');
            pending_blank = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn is_git_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{0b}' | '\u{0c}')
}

/// Returns the first line of a message.
#[must_use]
pub fn subject(message: &str) -> &str {
    message.split('\n').next().unwrap_or_default()
}

/// Returns the text that follows `commit: ` in the reflog entry of a commit,
/// which is the first line without leading white space.
#[must_use]
pub fn reflog_subject(message: &str) -> &str {
    subject(message).trim_start_matches(is_git_space)
}

/// One trailer line, `Token: value`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Trailer {
    /// Token, for example `Signed-off-by`.
    pub token: String,
    /// Value after the separator, without surrounding white space.
    pub value: String,
}

impl Trailer {
    /// Parses a trailer written as `Token: value` or `Token=value`.
    ///
    /// The token consists of ASCII letters, digits and `-`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTrailer`] when the text has no separator, the token
    /// is empty or contains other characters, the value is empty, or the
    /// text spans several lines.
    pub fn parse(text: &str) -> Result<Self, InvalidTrailer> {
        let invalid = |reason: &'static str| InvalidTrailer {
            text: text.to_owned(),
            reason,
        };
        if text.contains('\n') {
            return Err(invalid("a trailer is one line"));
        }
        let position = text
            .find([':', '='])
            .ok_or_else(|| invalid("a trailer needs `:` between the token and the value"))?;
        let token = text[..position].trim_end();
        let value = text[position + 1..].trim_matches(is_git_space);
        if token.is_empty() || !token.bytes().all(is_token_byte) {
            return Err(invalid("the token may contain only letters, digits and `-`"));
        }
        if value.is_empty() {
            return Err(invalid("the trailer has no value"));
        }
        Ok(Self {
            token: token.to_owned(),
            value: value.to_owned(),
        })
    }

    fn same_as(&self, other: &Self) -> bool {
        self.token.eq_ignore_ascii_case(&other.token) && self.value.eq_ignore_ascii_case(&other.value)
    }
}

impl fmt::Display for Trailer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.token, self.value)
    }
}

/// Text that is not a trailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTrailer {
    /// The rejected text.
    pub text: String,
    /// What is wrong with it.
    pub reason: &'static str,
}

impl fmt::Display for InvalidTrailer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid trailer `{}`: {}", self.text, self.reason)
    }
}

impl std::error::Error for InvalidTrailer {}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

/// Token and value of a trailer line, when the line is one.
fn trailer_of_line(line: &str) -> Option<Trailer> {
    let position = line.find(':')?;
    let token = line[..position].trim_end();
    if token.is_empty() || !token.bytes().all(is_token_byte) {
        return None;
    }
    Some(Trailer {
        token: token.to_owned(),
        value: line[position + 1..].trim().to_owned(),
    })
}

/// Trailer block of a message.
struct Block {
    /// Index of the first line of the block.
    start: usize,
    /// Trailers of the block, in order.
    trailers: Vec<Trailer>,
}

/// Finds the trailer block: the last paragraph, when it is not the title and
/// consists of trailers, or contains a recognized trailer and at least 25%
/// trailer lines.
fn find_block(lines: &[&str]) -> Option<Block> {
    let end = lines.iter().rposition(|line| !line.trim().is_empty())? + 1;
    let start = lines[..end]
        .iter()
        .rposition(|line| line.trim().is_empty())
        .map(|blank| blank + 1)?;
    let mut trailers: Vec<Trailer> = Vec::new();
    let mut trailer_lines = 0_usize;
    let mut other_lines = 0_usize;
    let mut recognized = false;
    for line in &lines[start..end] {
        if line.starts_with([' ', '\t']) && !trailers.is_empty() {
            trailer_lines += 1;
            continue;
        }
        recognized |= RECOGNIZED_PREFIXES.iter().any(|prefix| line.starts_with(prefix));
        match trailer_of_line(line) {
            Some(trailer) => {
                trailer_lines += 1;
                trailers.push(trailer);
            }
            None => other_lines += 1,
        }
    }
    let is_block = trailer_lines > 0
        && (other_lines == 0
            || (recognized && trailer_lines * NON_TRAILER_LINES_PER_TRAILER >= other_lines));
    is_block.then_some(Block { start, trailers })
}

fn content_lines(message: &str) -> Vec<&str> {
    let body = message.strip_suffix('\n').unwrap_or(message);
    if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    }
}

/// Returns the trailers of the trailer block of a message, by token; values
/// of a token that occurs several times are joined with a line feed.
#[must_use]
pub fn trailers(message: &str) -> IndexMap<String, String> {
    let lines = content_lines(message);
    let mut found: IndexMap<String, String> = IndexMap::new();
    for trailer in find_block(&lines).map(|block| block.trailers).unwrap_or_default() {
        found
            .entry(trailer.token)
            .and_modify(|value| {
                value.push('\n');
                value.push_str(&trailer.value);
            })
            .or_insert(trailer.value);
    }
    found
}

/// Appends trailers to the trailer block of a message, or to a new block
/// after a blank line when the message has none.
///
/// A trailer equal to the last trailer of the block (token and value compared
/// without regard to case) is not added again.
#[must_use]
pub fn append_trailers(message: &str, trailers: &[Trailer]) -> String {
    if trailers.is_empty() {
        return message.to_owned();
    }
    let lines = content_lines(message);
    let block = find_block(&lines);
    let mut out = String::with_capacity(message.len() + trailers.len() * 32);
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    let mut last = block.and_then(|block| block.trailers.last().cloned());
    let mut separated = last.is_some() || lines.is_empty();
    for trailer in trailers {
        if last.as_ref().is_some_and(|neighbour| neighbour.same_as(trailer)) {
            continue;
        }
        if !separated {
            out.push('\n');
            separated = true;
        }
        out.push_str(&trailer.to_string());
        out.push('\n');
        last = Some(trailer.clone());
    }
    out
}

/// Adds `Signed-off-by: <identity>` as `git commit --signoff` does; the line
/// is not added when it is already the last line of the trailer block.
///
/// The identity is `Name <email>` of the committer.
#[must_use]
pub fn sign_off(message: &str, identity: &str) -> String {
    let line = format!("{SIGN_OFF_TOKEN}: {identity}");
    let lines = content_lines(message);
    let block = find_block(&lines);
    if block.is_some() && lines.last().is_some_and(|last| *last == line) {
        return message.to_owned();
    }
    let mut out = String::with_capacity(message.len() + line.len() + 2);
    for existing in &lines {
        out.push_str(existing);
        out.push('\n');
    }
    if block.is_none() && !lines.is_empty() {
        out.push('\n');
    }
    out.push_str(&line);
    out.push('\n');
    out
}

/// Returns the index of the first line of the trailer block, for renderers
/// that show a message without its trailers; `None` when there is no block.
#[must_use]
pub fn trailer_block_start(message: &str) -> Option<usize> {
    find_block(&content_lines(message)).map(|block| block.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trailer(text: &str) -> Trailer {
        Trailer::parse(text).unwrap()
    }

    #[test]
    fn whitespace_cleanup_keeps_leading_spaces_and_comments() {
        let cleaned = cleanup("  first   \n\n  body line  \n\n\n", Cleanup::Whitespace, '#');
        assert_eq!(cleaned, "  first\n\n  body line\n");
        assert_eq!(cleanup("a\n# c\n", Cleanup::Whitespace, '#'), "a\n# c\n");
        assert_eq!(cleanup(" \n\t\n", Cleanup::Whitespace, '#'), "");
        assert_eq!(cleanup("\n\ntitle\n\n\n\nbody\t\n", Cleanup::Whitespace, '#'), "title\n\nbody\n");
    }

    #[test]
    fn strip_cleanup_removes_lines_with_the_comment_character() {
        assert_eq!(cleanup("a\n; c\n\n; d\nb\n", Cleanup::Strip, ';'), "a\n\nb\n");
    }

    #[test]
    fn verbatim_cleanup_returns_the_message_unchanged() {
        assert_eq!(cleanup(" a \n\n\n", Cleanup::Verbatim, '#'), " a \n\n\n");
    }

    #[test]
    fn cleanup_mode_rejects_unknown_values_with_the_accepted_list() {
        let error = Cleanup::from_config(Some("Tidy")).unwrap_err();
        assert!(error.to_string().contains("strip, whitespace"));
        assert_eq!(Cleanup::from_config(Some("VERBATIM")), Ok(Cleanup::Verbatim));
        assert_eq!(Cleanup::from_config(Some("scissors")), Ok(Cleanup::Whitespace));
        assert_eq!(Cleanup::from_config(None), Ok(Cleanup::Whitespace));
    }

    #[test]
    fn title_paragraph_is_never_a_trailer_block() {
        assert_eq!(append_trailers("fix: subj\n", &[trailer("Refs:2")]), "fix: subj\n\nRefs: 2\n");
    }

    #[test]
    fn trailer_is_appended_after_a_blank_line_when_the_body_is_prose() {
        let out = append_trailers("subj\n\ntext here\n", &[trailer("Refs=3")]);
        assert_eq!(out, "subj\n\ntext here\n\nRefs: 3\n");
    }

    #[test]
    fn trailer_joins_an_existing_block_without_a_blank_line() {
        let out = append_trailers("docs: note\n\nRefs: 1\n", &[trailer("Co-Authored-By: A <a@x>")]);
        assert_eq!(out, "docs: note\n\nRefs: 1\nCo-Authored-By: A <a@x>\n");
    }

    #[test]
    fn duplicate_of_the_last_trailer_is_skipped_but_not_of_an_earlier_one() {
        assert_eq!(append_trailers("s\n\nRefs: 3\n", &[trailer("refs: 3")]), "s\n\nRefs: 3\n");
        let out = append_trailers("s\n\nRefs: 3\nOther: x\n", &[trailer("Refs: 3")]);
        assert_eq!(out, "s\n\nRefs: 3\nOther: x\nRefs: 3\n");
    }

    #[test]
    fn repeated_trailers_in_one_call_are_added_once() {
        let out = append_trailers("s\n", &[trailer("A: 1"), trailer("A: 1")]);
        assert_eq!(out, "s\n\nA: 1\n");
        assert_eq!(append_trailers("s\n", &[]), "s\n");
    }

    #[test]
    fn mixed_paragraph_with_sign_off_counts_as_a_block() {
        let message = "subj\n\nsome text\nRefs: 4\nSigned-off-by: A <a@b>\nmore text\n";
        let out = append_trailers(message, &[trailer("X: y")]);
        assert_eq!(out, format!("{message}X: y\n"));
    }

    #[test]
    fn mixed_paragraph_without_recognized_prefix_is_prose() {
        let message = "subj\n\nsome text\nRefs: 4\nmore text\n";
        let out = append_trailers(message, &[trailer("X: y")]);
        assert_eq!(out, format!("{message}\nX: y\n"));
    }

    #[test]
    fn continuation_lines_belong_to_the_block() {
        let message = "subj\n\nRefs: 5\n  continued\n";
        assert_eq!(append_trailers(message, &[trailer("X: y")]), format!("{message}X: y\n"));
    }

    #[test]
    fn sign_off_is_not_repeated_when_it_is_the_last_line() {
        let ada = "Tester <t@example.com>";
        let message = "x\n\nSigned-off-by: Tester <t@example.com>\n";
        assert_eq!(sign_off(message, ada), message);
        let other = "y\n\nSigned-off-by: Other <o@x>\n";
        assert_eq!(sign_off(other, ada), format!("{other}Signed-off-by: {ada}\n"));
        assert_eq!(sign_off("s\n", ada), format!("s\n\nSigned-off-by: {ada}\n"));
    }

    #[test]
    fn sign_off_precedes_trailers_given_with_the_call() {
        let signed = sign_off("feat: x\n\nRefs: 1\n", "Tester <t@example.com>");
        let out = append_trailers(&signed, &[trailer("Co-Authored-By: A <a@x>")]);
        assert_eq!(
            out,
            "feat: x\n\nRefs: 1\nSigned-off-by: Tester <t@example.com>\nCo-Authored-By: A <a@x>\n"
        );
    }

    #[test]
    fn trailer_parse_rejects_malformed_text() {
        assert!(Trailer::parse("key with space: v").is_err());
        assert!(Trailer::parse("Key:").is_err());
        assert!(Trailer::parse(": v").is_err());
        assert!(Trailer::parse("A: b\nC: d").is_err());
        let error = Trailer::parse("plain").unwrap_err();
        assert!(error.to_string().starts_with("invalid trailer `plain`"));
        assert_eq!(trailer("Co-Authored-By=Ada").to_string(), "Co-Authored-By: Ada");
    }

    #[test]
    fn reflog_subject_uses_only_the_first_line() {
        assert_eq!(reflog_subject("line one\nline two\n\nbody\n"), "line one");
        assert_eq!(reflog_subject("  first\n"), "first");
        assert_eq!(subject(""), "");
    }

    #[test]
    fn trailers_of_a_message_are_grouped_by_token() {
        assert!(trailers("subj\n\nbody text\n").is_empty());
        assert!(trailers("").is_empty());
        let found = trailers("fix: x\n\nRefs: 1\nRefs: 2\nAcked-by: Bo\n");
        assert_eq!(found.get("Refs").map(String::as_str), Some("1\n2"));
        assert_eq!(found.len(), 2);
        assert_eq!(trailer_block_start("s\n\nA: 1\n"), Some(2));
        assert_eq!(trailer_block_start("s\n\nbody\n"), None);
    }
}
