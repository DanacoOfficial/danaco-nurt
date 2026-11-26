// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Guides of a catalog: front matter, sections of the body and material.
//!
//! The front matter is a subset of YAML 1.2: one mapping of keys to plain,
//! quoted or block scalars, flow or block sequences of scalars and one level
//! of nested mappings. Every scalar is text. Anchors, tags and flow mappings
//! are rejected, so no YAML library is needed and nothing in the file can
//! change how it is read.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::catalog::paths::Root;
use crate::catalog::types::{GUIDE_EXTENSION, Limits, is_valid_language, is_valid_name};
use crate::text::{fold, query_words, strip_controls};

/// Line that opens and closes the front matter.
const DELIMITER: &str = "---";
/// Prefix of a section heading in the body.
const SECTION_PREFIX: &str = "## ";
/// Weight of a query word found in a section title, relative to the text.
const TITLE_WEIGHT: usize = 3;
/// Number of hexadecimal digits of an etag.
const ETAG_DIGITS: usize = 8;
/// Longest `description` of the front matter.
const MAX_DESCRIPTION: usize = 1024;
/// Longest `version` of the front matter.
const MAX_VERSION: usize = 60;
/// Longest `license` of the front matter.
const MAX_LICENSE: usize = 200;
/// Most programs listed in the front matter.
const MAX_PROGRAMS: usize = 64;
/// Shortest fence of a code block.
const MIN_FENCE: usize = 3;

/// Value of a key of the front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontValue {
    /// A scalar.
    Text(String),
    /// A sequence of scalars.
    List(Vec<String>),
    /// A nested mapping of scalars, in file order.
    Map(Vec<(String, String)>),
}

/// Violation of the front matter syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatterError {
    /// Line in the guide file, counted from 1.
    pub line: u64,
    /// What is wrong and how to correct it.
    pub message: String,
}

impl fmt::Display for FrontMatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for FrontMatterError {}

/// Line of a guide file on which the text of the front matter starts.
pub const FRONT_FIRST_LINE: u64 = 2;

/// A guide file split into its front matter and its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitGuide<'a> {
    /// Text between the delimiter lines, starting on line
    /// [`FRONT_FIRST_LINE`].
    pub front: &'a str,
    /// Text after the closing delimiter line.
    pub body: &'a str,
}

/// Splits a guide into front matter and body. The opening delimiter is the
/// first line, after an optional byte order mark; the closing delimiter must
/// end within `limit` bytes.
///
/// # Errors
///
/// Fails when the file does not start with front matter or the front matter
/// does not end within `limit` bytes.
///
/// # Examples
///
/// ```
/// use nurt_programs::catalog::guide::split_guide;
///
/// let guide = split_guide("---\nname: tar\n---\n## Create\n", 8192).unwrap();
/// assert_eq!(guide.front, "name: tar\n");
/// assert_eq!(guide.body, "## Create\n");
/// ```
pub fn split_guide(text: &str, limit: usize) -> Result<SplitGuide<'_>, FrontMatterError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let missing = || FrontMatterError {
        line: 1,
        message: "guide must start with front matter between two `---` lines".to_owned(),
    };
    let first_end = text.find('\n').ok_or_else(missing)?;
    if trim_line(&text[..first_end]) != DELIMITER {
        return Err(missing());
    }
    let front_start = first_end + 1;
    let mut offset = front_start;
    while offset < text.len() && offset <= limit {
        let end = text[offset..]
            .find('\n')
            .map_or(text.len(), |at| offset + at);
        if trim_line(&text[offset..end]) == DELIMITER && end <= limit {
            let body_start = (end + 1).min(text.len());
            return Ok(SplitGuide {
                front: &text[front_start..offset],
                body: &text[body_start..],
            });
        }
        offset = end + 1;
    }
    Err(FrontMatterError {
        line: 1,
        message: format!("front matter must end with a `---` line within the first {limit} bytes"),
    })
}

fn trim_line(line: &str) -> &str {
    line.trim_end_matches(['\r', ' ', '\t'])
}

/// Parses front matter text into keys and values in file order.
/// `first_line` is the line of the first line of `front` in the guide file.
///
/// # Errors
///
/// Fails on the first line that is outside the permitted subset of YAML or
/// repeats a key.
///
/// # Examples
///
/// ```
/// use nurt_programs::catalog::guide::{parse_front_matter, FrontValue};
///
/// let entries = parse_front_matter("name: tar\nprograms: [tar, zstd]\n", 2).unwrap();
/// assert_eq!(entries[0], ("name".to_owned(), FrontValue::Text("tar".to_owned())));
/// assert_eq!(
///     entries[1].1,
///     FrontValue::List(vec!["tar".to_owned(), "zstd".to_owned()])
/// );
/// ```
pub fn parse_front_matter(
    front: &str,
    first_line: u64,
) -> Result<Vec<(String, FrontValue)>, FrontMatterError> {
    let lines: Vec<&str> = front
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut parser = FrontParser {
        lines: &lines,
        first_line,
        index: 0,
    };
    let mut entries: Vec<(String, FrontValue)> = Vec::new();
    while let Some((key, value)) = parser.next_entry()? {
        if entries.iter().any(|(known, _)| *known == key) {
            return Err(parser.error_at(
                parser.index.saturating_sub(1),
                format!("key `{key}` is repeated; keep one"),
            ));
        }
        entries.push((key, value));
    }
    Ok(entries)
}

struct FrontParser<'a> {
    lines: &'a [&'a str],
    first_line: u64,
    index: usize,
}

impl<'a> FrontParser<'a> {
    fn error_at(&self, index: usize, message: impl Into<String>) -> FrontMatterError {
        FrontMatterError {
            line: self.first_line + u64::try_from(index).unwrap_or(u64::MAX - self.first_line),
            message: message.into(),
        }
    }

    fn next_entry(&mut self) -> Result<Option<(String, FrontValue)>, FrontMatterError> {
        while let Some(line) = self.lines.get(self.index) {
            let at = self.index;
            self.index += 1;
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with([' ', '\t']) {
                return Err(
                    self.error_at(at, "unexpected indentation; start keys in the first column")
                );
            }
            let (key, rest) =
                split_key(line).ok_or_else(|| self.error_at(at, "expected `key: value`"))?;
            let value = self.value(at, rest.trim())?;
            return Ok(Some((key.to_owned(), value)));
        }
        Ok(None)
    }

    fn value(&mut self, at: usize, rest: &str) -> Result<FrontValue, FrontMatterError> {
        if rest.is_empty() {
            return self.nested();
        }
        if let Some(style) = BlockStyle::parse(rest) {
            return Ok(FrontValue::Text(self.block_scalar(style)));
        }
        if rest.starts_with('[') {
            return parse_flow_sequence(rest)
                .map(FrontValue::List)
                .map_err(|message| self.error_at(at, message));
        }
        let mut text = parse_scalar(rest).map_err(|message| self.error_at(at, message))?;
        if !rest.starts_with(['"', '\'']) {
            for line in self.indented_lines() {
                text.push(' ');
                text.push_str(&strip_comment(line.trim()));
            }
        }
        Ok(FrontValue::Text(text))
    }

    /// Following lines that are indented and not empty.
    fn indented_lines(&mut self) -> Vec<&'a str> {
        let mut out = Vec::new();
        while let Some(line) = self.lines.get(self.index) {
            if line.trim().is_empty() || !line.starts_with([' ', '\t']) {
                break;
            }
            out.push(*line);
            self.index += 1;
        }
        out
    }

    fn nested(&mut self) -> Result<FrontValue, FrontMatterError> {
        let start = self.index;
        let lines = self.indented_lines();
        let Some(first) = lines.first() else {
            return Ok(FrontValue::Text(String::new()));
        };
        if first.trim_start().starts_with('-') {
            let mut items = Vec::new();
            for (offset, line) in lines.iter().enumerate() {
                let item = line
                    .trim_start()
                    .strip_prefix('-')
                    .filter(|item| item.is_empty() || item.starts_with(' '))
                    .ok_or_else(|| {
                        self.error_at(start + offset, "expected a sequence item `- value`")
                    })?;
                items.push(
                    parse_scalar(item.trim())
                        .map_err(|message| self.error_at(start + offset, message))?,
                );
            }
            return Ok(FrontValue::List(items));
        }
        let mut map: Vec<(String, String)> = Vec::new();
        for (offset, line) in lines.iter().enumerate() {
            let (key, rest) = split_key(line.trim_start()).ok_or_else(|| {
                self.error_at(start + offset, "expected `key: value` in a nested mapping")
            })?;
            if rest.trim().is_empty() {
                return Err(self.error_at(
                    start + offset,
                    "nesting deeper than one level is not permitted",
                ));
            }
            let value = parse_scalar(rest.trim())
                .map_err(|message| self.error_at(start + offset, message))?;
            map.push((key.to_owned(), value));
        }
        Ok(FrontValue::Map(map))
    }

    fn block_scalar(&mut self, style: BlockStyle) -> String {
        let mut raw: Vec<&str> = Vec::new();
        while let Some(line) = self.lines.get(self.index) {
            if !line.trim().is_empty() && !line.starts_with([' ', '\t']) {
                break;
            }
            raw.push(line);
            self.index += 1;
        }
        let indent = raw
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.len() - line.trim_start().len())
            .min()
            .unwrap_or(0);
        let content: Vec<&str> = raw
            .iter()
            .map(|line| line.get(indent..).unwrap_or(""))
            .collect();
        style.assemble(&content)
    }
}

/// Indicator of a block scalar.
#[derive(Debug, Clone, Copy)]
struct BlockStyle {
    folded: bool,
    chomp: Chomp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chomp {
    Clip,
    Strip,
    Keep,
}

impl BlockStyle {
    fn parse(indicator: &str) -> Option<Self> {
        let indicator = strip_comment(indicator);
        let mut chars = indicator.chars();
        let folded = match chars.next()? {
            '|' => false,
            '>' => true,
            _ => return None,
        };
        let chomp = match chars.as_str() {
            "" => Chomp::Clip,
            "-" => Chomp::Strip,
            "+" => Chomp::Keep,
            _ => return None,
        };
        Some(Self { folded, chomp })
    }

    /// Joins the lines of the scalar: a literal scalar keeps line breaks; a
    /// folded scalar joins adjacent lines with a space and turns each blank
    /// line into a line break.
    fn assemble(self, lines: &[&str]) -> String {
        let text = if self.folded {
            fold_lines(lines)
        } else {
            lines.join("\n")
        };
        let body = text.trim_end_matches('\n').trim_start_matches('\n');
        let trailing = text.len() - text.trim_end_matches('\n').len();
        match self.chomp {
            Chomp::Strip => body.to_owned(),
            Chomp::Clip if body.is_empty() => String::new(),
            Chomp::Clip => format!("{body}\n"),
            Chomp::Keep => format!("{body}\n{}", "\n".repeat(trailing)),
        }
    }
}

fn fold_lines(lines: &[&str]) -> String {
    let mut text = String::new();
    let mut previous_blank = false;
    for line in lines {
        if line.trim().is_empty() {
            text.push('\n');
            previous_blank = true;
            continue;
        }
        if !text.is_empty() && !previous_blank {
            text.push(' ');
        }
        text.push_str(line);
        previous_blank = false;
    }
    text
}

fn split_key(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once(':')?;
    let valid_key = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    (valid_key && (rest.is_empty() || rest.starts_with([' ', '\t']))).then_some((key, rest))
}

/// Removes a comment introduced by ` #` from a plain scalar.
fn strip_comment(text: &str) -> String {
    let mut out = text;
    if let Some(at) = text.find(" #") {
        out = &text[..at];
    }
    if let Some(at) = out.find("\t#") {
        out = &out[..at];
    }
    out.trim_end().to_owned()
}

/// Parses a scalar written on one line: plain, in double quotes or in single
/// quotes.
fn parse_scalar(text: &str) -> Result<String, String> {
    if let Some(rest) = text.strip_prefix('"') {
        let (value, after) = parse_double_quoted(rest)?;
        return finish_quoted(value, after);
    }
    if let Some(rest) = text.strip_prefix('\'') {
        let (value, after) = parse_single_quoted(rest)?;
        return finish_quoted(value, after);
    }
    if text.starts_with(['&', '*', '!', '{', '[', '|', '>', '%', '@', '`']) {
        return Err("anchors, aliases, tags, flow mappings and reserved indicators are not permitted; quote the value".to_owned());
    }
    Ok(strip_comment(text))
}

fn finish_quoted(value: String, after: &str) -> Result<String, String> {
    let after = after.trim();
    if after.is_empty() || after.starts_with('#') {
        Ok(value)
    } else {
        Err("unexpected text after the closing quote".to_owned())
    }
}

fn parse_double_quoted(text: &str) -> Result<(String, &str), String> {
    let mut value = String::new();
    let mut chars = text.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => return Ok((value, &text[at + 1..])),
            '\\' => {
                let (_, escape) = chars.next().ok_or_else(unterminated)?;
                match escape {
                    '"' | '\\' | '/' => value.push(escape),
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    'u' => {
                        let digits: String = chars.by_ref().take(4).map(|(_, c)| c).collect();
                        let code = u32::from_str_radix(&digits, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or_else(|| {
                                format!("escape `\\u{digits}` is not a valid character")
                            })?;
                        value.push(code);
                    }
                    other => return Err(format!("escape `\\{other}` is not permitted")),
                }
            }
            other => value.push(other),
        }
    }
    Err(unterminated())
}

fn parse_single_quoted(text: &str) -> Result<(String, &str), String> {
    let mut value = String::new();
    let mut rest = text;
    loop {
        let at = rest.find('\'').ok_or_else(unterminated)?;
        value.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        if let Some(after) = rest.strip_prefix('\'') {
            value.push('\'');
            rest = after;
        } else {
            return Ok((value, rest));
        }
    }
}

fn unterminated() -> String {
    "quoted value is not closed on the same line; add the closing quote".to_owned()
}

fn parse_flow_sequence(text: &str) -> Result<Vec<String>, String> {
    let mut rest = text
        .strip_prefix('[')
        .ok_or_else(|| "expected `[`".to_owned())?
        .trim_start();
    let mut items = Vec::new();
    if let Some(after) = rest.strip_prefix(']') {
        return finish_quoted(String::new(), after).map(|_| items);
    }
    loop {
        let (item, after) = if let Some(quoted) = rest.strip_prefix('"') {
            parse_double_quoted(quoted)?
        } else if let Some(quoted) = rest.strip_prefix('\'') {
            parse_single_quoted(quoted)?
        } else {
            let end = rest
                .find([',', ']'])
                .ok_or_else(|| "sequence is not closed; add `]`".to_owned())?;
            let plain = rest[..end].trim();
            if plain.is_empty() || plain.starts_with(['[', '{', '&', '*', '!']) {
                return Err("sequence items must be scalars".to_owned());
            }
            (plain.to_owned(), &rest[end..])
        };
        items.push(item);
        let after = after.trim_start();
        if let Some(next) = after.strip_prefix(',') {
            rest = next.trim_start();
        } else if let Some(end) = after.strip_prefix(']') {
            return finish_quoted(String::new(), end).map(|_| items);
        } else {
            return Err("expected `,` or `]` in the sequence".to_owned());
        }
    }
}

/// Typed front matter of a guide.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrontMatter {
    /// Name of the guide.
    pub name: Option<String>,
    /// When and for what to use the guide.
    pub description: Option<String>,
    /// Version of the guide or of the described program.
    pub version: Option<String>,
    /// Programs the guide describes.
    pub programs: Vec<String>,
    /// Language of the guide.
    pub language: Option<String>,
    /// License of the guide text.
    pub license: Option<String>,
    /// Data of the producer.
    pub metadata: BTreeMap<String, String>,
}

/// Violation of the rules of one key of the front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontFault {
    /// Key.
    pub key: String,
    /// What is wrong and how to correct it.
    pub message: String,
}

impl FrontMatter {
    /// Reads the keys defined by the format from parsed entries; other keys
    /// are ignored. A missing `version` is not a fault: the caller reports
    /// it separately.
    #[must_use]
    pub fn from_entries(entries: Vec<(String, FrontValue)>) -> (Self, Vec<FrontFault>) {
        let mut front = Self::default();
        let mut faults = Vec::new();
        for (key, value) in entries {
            if let Err(message) = front.set(&key, value) {
                faults.push(FrontFault { key, message });
            }
        }
        let required = [
            ("name", front.name.is_none()),
            ("description", front.description.is_none()),
        ];
        for (key, missing) in required {
            if missing && !faults.iter().any(|item| item.key == key) {
                faults.push(FrontFault {
                    key: key.to_owned(),
                    message: "is required; add it to the front matter".to_owned(),
                });
            }
        }
        (front, faults)
    }

    fn set(&mut self, key: &str, value: FrontValue) -> Result<(), String> {
        match (key, value) {
            ("name", FrontValue::Text(name)) if is_valid_name(&name) => self.name = Some(name),
            ("name", _) => {
                return Err(
                    "must be a guide name of letters, digits, `.`, `_`, `+` or `-`".to_owned(),
                );
            }
            ("description", FrontValue::Text(text)) => {
                self.description = Some(checked_text(&text, MAX_DESCRIPTION)?);
            }
            ("version", FrontValue::Text(text)) => {
                self.version = Some(checked_text(&text, MAX_VERSION)?)
            }
            ("license", FrontValue::Text(text)) => {
                self.license = Some(checked_text(&text, MAX_LICENSE)?)
            }
            ("language", FrontValue::Text(tag)) if is_valid_language(&tag) => {
                self.language = Some(tag)
            }
            ("language", _) => {
                return Err("must be a language tag such as `en` or `pt-BR`".to_owned());
            }
            ("programs", FrontValue::List(names)) => self.programs = checked_programs(names)?,
            ("programs", _) => return Err("must be a sequence of program names".to_owned()),
            ("metadata", FrontValue::Map(pairs)) => self.metadata = pairs.into_iter().collect(),
            ("metadata", _) => return Err("must be a mapping of keys to texts".to_owned()),
            ("description" | "version" | "license", _) => return Err("must be text".to_owned()),
            _ => {}
        }
        Ok(())
    }
}

fn checked_text(text: &str, max: usize) -> Result<String, String> {
    let text = text.trim();
    let count = text.chars().count();
    if count == 0 {
        Err("must not be empty".to_owned())
    } else if count > max {
        Err(format!("is longer than {max} characters; shorten it"))
    } else {
        Ok(text.to_owned())
    }
}

fn checked_programs(names: Vec<String>) -> Result<Vec<String>, String> {
    if names.len() > MAX_PROGRAMS {
        return Err(format!("lists more than {MAX_PROGRAMS} programs"));
    }
    let mut out: Vec<String> = Vec::new();
    for name in names {
        if !is_valid_name(&name) {
            return Err(format!("`{name}` is not a valid program name"));
        }
        if out.contains(&name) {
            return Err(format!("`{name}` is listed twice"));
        }
        out.push(name);
    }
    Ok(out)
}

/// Section of the body of a guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuideSection<'a> {
    /// Title, without `## ` and trailing `#` characters.
    pub title: &'a str,
    /// Text of the section, from its heading line to the next heading.
    pub text: &'a str,
}

/// Sections of a guide body, introduced by lines that start with `## `
/// outside fenced code blocks.
///
/// # Examples
///
/// ```
/// use nurt_programs::catalog::guide::sections;
///
/// let body = "Intro\n## Create\n```\n## not a heading\n```\n## Extract ##\nx\n";
/// let titles: Vec<&str> = sections(body).iter().map(|s| s.title).collect();
/// assert_eq!(titles, ["Create", "Extract"]);
/// ```
#[must_use]
pub fn sections(body: &str) -> Vec<GuideSection<'_>> {
    let starts = heading_starts(body);
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(body.len());
            let text = &body[*start..end];
            let heading = text.lines().next().unwrap_or_default();
            let title = heading
                .strip_prefix(SECTION_PREFIX)
                .unwrap_or_default()
                .trim()
                .trim_end_matches('#')
                .trim_end();
            GuideSection { title, text }
        })
        .collect()
}

/// Text before the first section.
#[must_use]
pub fn introduction(body: &str) -> &str {
    let end = heading_starts(body).first().copied().unwrap_or(body.len());
    &body[..end]
}

fn heading_starts(body: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        match fence {
            Some((mark, length)) => {
                if fence_of(content).is_some_and(|(close, count, rest)| {
                    close == mark && count >= length && rest.trim().is_empty()
                }) {
                    fence = None;
                }
            }
            None => {
                if let Some((mark, count, _)) = fence_of(content) {
                    fence = Some((mark, count));
                } else if content.starts_with(SECTION_PREFIX) {
                    starts.push(offset);
                }
            }
        }
        offset += line.len();
    }
    starts
}

/// Fence character, its count and the rest of a line that opens or closes a
/// fenced code block (at most three spaces of indentation).
fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > MIN_FENCE {
        return None;
    }
    let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let count = trimmed.chars().take_while(|c| *c == mark).count();
    (count >= MIN_FENCE).then(|| (mark, count, &trimmed[count..]))
}

/// Section whose title equals `wanted`, compared after folding, or else the
/// first section whose title contains it. Leading `#` characters of `wanted`
/// are ignored.
///
/// # Examples
///
/// ```
/// use nurt_programs::catalog::guide::find_section;
///
/// let body = "## Create archives\na\n## Pitfalls\nb\n";
/// assert_eq!(find_section(body, "## pitfalls").unwrap().title, "Pitfalls");
/// assert_eq!(find_section(body, "create").unwrap().title, "Create archives");
/// assert!(find_section(body, "missing").is_none());
/// ```
#[must_use]
pub fn find_section<'a>(body: &'a str, wanted: &str) -> Option<GuideSection<'a>> {
    let wanted = fold(wanted.trim_start_matches('#').trim());
    if wanted.is_empty() {
        return None;
    }
    let all = sections(body);
    all.iter()
        .find(|section| fold(section.title) == wanted)
        .or_else(|| {
            all.iter()
                .find(|section| fold(section.title).contains(&wanted))
        })
        .copied()
}

/// Sections that match the words of `query`, best first, at most `limit`.
/// A word in the title counts three times as much as a word in the text;
/// equal scores keep the order of the guide.
///
/// # Examples
///
/// ```
/// use nurt_programs::catalog::guide::search_sections;
///
/// let body = "## Create\ntar cf\n## Extract\ntar xf, create nothing\n## Other\nx\n";
/// let found: Vec<&str> = search_sections(body, "create", 3).iter().map(|s| s.title).collect();
/// assert_eq!(found, ["Create", "Extract"]);
/// ```
#[must_use]
pub fn search_sections<'a>(body: &'a str, query: &str, limit: usize) -> Vec<GuideSection<'a>> {
    let words = query_words(query, true);
    let mut scored: Vec<(usize, usize, GuideSection<'a>)> = sections(body)
        .into_iter()
        .enumerate()
        .filter_map(|(position, section)| {
            let title = fold(section.title);
            let text = fold(section.text);
            let score: usize = words
                .iter()
                .map(|word| {
                    let in_title = if title.contains(word.as_str()) {
                        TITLE_WEIGHT
                    } else {
                        0
                    };
                    in_title + usize::from(text.contains(word.as_str()))
                })
                .sum();
            (score > 0).then_some((score, position, section))
        })
        .collect();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, section)| section)
        .collect()
}

/// Guide file read in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideFile {
    /// Front matter.
    pub front: FrontMatter,
    /// Violations of the rules of front matter keys.
    pub faults: Vec<FrontFault>,
    /// Body without the front matter.
    pub body: String,
    /// Etag of the whole file.
    pub etag: String,
}

/// Reads the front matter of the guide file at `path`, reading at most
/// `limits.front_matter_bytes` of it.
pub(crate) fn read_head(
    path: &Path,
    limits: &Limits,
) -> Result<(FrontMatter, Vec<FrontFault>), String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("guide cannot be read: {error}"))?
        .len();
    if size > limits.guide_bytes {
        return Err(too_large(limits.guide_bytes));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(
                u64::try_from(limits.front_matter_bytes)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            )
            .read_to_end(&mut bytes)
        })
        .map_err(|error| format!("guide cannot be read: {error}"))?;
    let text = utf8_prefix(&bytes)?;
    parse_head(text, limits.front_matter_bytes)
}

/// Reads the guide file at `path` in full.
pub(crate) fn read_full(path: &Path, limits: &Limits) -> Result<GuideFile, String> {
    let file = File::open(path).map_err(|error| format!("guide cannot be read: {error}"))?;
    let mut bytes = Vec::new();
    file.take(limits.guide_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("guide cannot be read: {error}"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits.guide_bytes {
        return Err(too_large(limits.guide_bytes));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "guide is not valid UTF-8; save it as UTF-8".to_owned())?;
    let split = split_guide(text, limits.front_matter_bytes).map_err(|error| error.to_string())?;
    let entries =
        parse_front_matter(split.front, FRONT_FIRST_LINE).map_err(|error| error.to_string())?;
    let (front, faults) = FrontMatter::from_entries(entries);
    Ok(GuideFile {
        front,
        faults,
        body: strip_controls(split.body),
        etag: etag(&bytes),
    })
}

fn parse_head(text: &str, limit: usize) -> Result<(FrontMatter, Vec<FrontFault>), String> {
    let split = split_guide(text, limit).map_err(|error| error.to_string())?;
    let entries =
        parse_front_matter(split.front, FRONT_FIRST_LINE).map_err(|error| error.to_string())?;
    Ok(FrontMatter::from_entries(entries))
}

/// Valid UTF-8 prefix of bytes read up to a limit; a character cut by the
/// limit is dropped.
fn utf8_prefix(bytes: &[u8]) -> Result<&str, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).map_err(|_| not_utf8())
        }
        Err(_) => Err(not_utf8()),
    }
}

fn not_utf8() -> String {
    "guide is not valid UTF-8; save it as UTF-8".to_owned()
}

fn too_large(limit: u64) -> String {
    format!("guide is larger than {limit} bytes; split it into several guides")
}

/// Files of the material directory of the guide at `guide` (the path without
/// `.md`), relative to the root, sorted. Only regular files inside the root
/// are listed, to the depth and count of `limits`.
pub(crate) fn material(root: &Root, guide: &str, limits: &Limits) -> Vec<String> {
    let Some(dir) = guide.strip_suffix(GUIDE_EXTENSION) else {
        return Vec::new();
    };
    let Ok(start) = root.resolve(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect_material(root, &start, 1, limits, &mut files);
    files.sort();
    files
}

/// Adds the files below `dir` to `files` in the order of their names, and
/// stops as soon as `limits.guide_files` files are listed.
fn collect_material(
    root: &Root,
    dir: &Path,
    depth: usize,
    limits: &Limits,
    files: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if files.len() >= limits.guide_files {
            return;
        }
        let Some(relative) = root.relative(&path) else {
            continue;
        };
        let Ok(resolved) = root.resolve(&relative) else {
            continue;
        };
        if resolved.is_dir() {
            if depth < limits.guide_files_depth {
                collect_material(root, &resolved, depth + 1, limits, files);
            }
        } else if resolved.is_file() {
            files.push(relative);
        }
    }
}

/// Etag of a guide file: the first eight hexadecimal digits of the SHA-256
/// of its bytes.
///
/// # Examples
///
/// ```
/// use nurt_programs::catalog::guide::etag;
///
/// assert_eq!(etag(b""), "e3b0c442");
/// ```
#[must_use]
pub fn etag(bytes: &[u8]) -> String {
    let mut digest = hex::encode(Sha256::digest(bytes));
    digest.truncate(ETAG_DIGITS);
    digest
}
