// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lossless parser of INI documents.

use std::ops::Range;

use nurt_contracts::error::Error;

use super::profile::{Continuation, IniProfile, Rules};
use super::support::errors::parse_error;
use super::support::text::{Line, lines};
use super::value::decode;

use super::BOM;

/// Name of the format in error messages.
const FORMAT: &str = "ini";

/// Key and value line, with its continuation lines.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub(crate) section: usize,
    /// Name as written, without the `[]` suffix of PHP list keys.
    pub(crate) name: String,
    /// The PHP list suffix `[]` follows the name.
    pub(crate) list_suffix: bool,
    pub(crate) first_line: usize,
    pub(crate) last_line: usize,
    pub(crate) name_range: Range<usize>,
    /// Raw value; empty at the end of the name when there is no separator.
    pub(crate) value_range: Range<usize>,
    /// Decoded value; `None` for a key without separator.
    pub(crate) value: Option<String>,
    /// Comment lines directly above the entry and the trailing comment.
    pub(crate) comment: Option<String>,
}

/// Section header with the entries that follow it; section 0 holds the
/// entries before the first header.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Section {
    /// Key path segments of the section: empty for the global section, the
    /// name, or for Git configuration the name and the subsection.
    pub(crate) path: Vec<String>,
    pub(crate) header_line: Option<usize>,
    pub(crate) entries: Vec<usize>,
    /// Comment lines directly above the header and its trailing comment.
    pub(crate) comment: Option<String>,
}

/// Parsed INI document.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parsed {
    pub(crate) lines: Vec<Line>,
    pub(crate) sections: Vec<Section>,
    pub(crate) entries: Vec<Entry>,
    /// Lines that hold only a comment.
    pub(crate) comment_lines: Vec<bool>,
}

struct Parser<'a> {
    text: &'a str,
    profile: IniProfile,
    rules: Rules,
    parsed: Parsed,
    pending_comment: Vec<String>,
}

/// Parses `text` with the rules of `profile`.
pub(crate) fn parse(text: &str, profile: IniProfile) -> Result<Parsed, Error> {
    let line_table = lines(text);
    let count = line_table.len();
    let mut parser = Parser {
        text,
        profile,
        rules: profile.rules(),
        parsed: Parsed {
            lines: line_table,
            sections: vec![Section {
                path: Vec::new(),
                header_line: None,
                entries: Vec::new(),
                comment: None,
            }],
            entries: Vec::new(),
            comment_lines: vec![false; count],
        },
        pending_comment: Vec::new(),
    };
    let mut index = 0;
    while index < count {
        index = parser.line(index)?;
    }
    Ok(parser.parsed)
}

impl Parser<'_> {
    /// Parses the line `index` and returns the index of the next line.
    fn line(&mut self, index: usize) -> Result<usize, Error> {
        let line = self.parsed.lines[index];
        let content = line.content(self.text);
        let indent = content.len() - content.trim_start_matches([' ', '\t', BOM]).len();
        let body = &content[indent..];
        let start = line.start + indent;
        match body.chars().next() {
            None => {
                self.pending_comment.clear();
                Ok(index + 1)
            }
            Some(c) if self.rules.comment_chars.contains(&c) => {
                self.parsed.comment_lines[index] = true;
                self.pending_comment.push(body[c.len_utf8()..].trim().to_owned());
                Ok(index + 1)
            }
            Some('[') => {
                self.header(index, start, body)?;
                Ok(index + 1)
            }
            Some(_) => self.entry(index, start, body),
        }
    }

    fn take_comment(&mut self, trailing: Option<String>) -> Option<String> {
        let mut parts = std::mem::take(&mut self.pending_comment);
        parts.extend(trailing);
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    fn header(&mut self, index: usize, start: usize, body: &str) -> Result<(), Error> {
        let close = self.header_close(body).ok_or_else(|| {
            parse_error(FORMAT, "section header has no closing `]`", self.text, start)
        })?;
        let after = body[close + 1..].trim_start_matches([' ', '\t']);
        let trailing = match after.chars().next() {
            None => None,
            Some(c) if self.rules.trailing_comment_chars.contains(&c) => {
                Some(after[c.len_utf8()..].trim().to_owned())
            }
            Some(_) => {
                return Err(parse_error(
                    FORMAT,
                    "unexpected text after the section header",
                    self.text,
                    start + close + 1,
                ));
            }
        };
        let name = &body[1..close];
        let path = self
            .section_path(name)
            .map_err(|message| parse_error(FORMAT, message, self.text, start + 1))?;
        let comment = self.take_comment(trailing);
        self.parsed.sections.push(Section {
            path,
            header_line: Some(index),
            entries: Vec::new(),
            comment,
        });
        Ok(())
    }

    /// Offset of the `]` that closes a header starting with `[`.
    fn header_close(&self, body: &str) -> Option<usize> {
        if self.rules.trailing_comment_chars.is_empty() {
            return body.rfind(']');
        }
        let mut quoted = false;
        let mut escaped = false;
        for (i, c) in body.char_indices().skip(1) {
            match c {
                _ if escaped => escaped = false,
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                ']' if !quoted => return Some(i),
                _ => {}
            }
        }
        None
    }

    fn section_path(&self, name: &str) -> Result<Vec<String>, &'static str> {
        let trimmed = name.trim_matches([' ', '\t']);
        if trimmed.is_empty() {
            return Err("section name is empty");
        }
        if self.profile != IniProfile::Gitconfig {
            return Ok(vec![trimmed.to_owned()]);
        }
        if let Some((section, rest)) = trimmed.split_once([' ', '\t']) {
            let sub = rest.trim_start_matches([' ', '\t']);
            let inner = sub
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or("subsection name must be quoted")?;
            return Ok(vec![section.to_ascii_lowercase(), unescape_subsection(inner)]);
        }
        Ok(match trimmed.split_once('.') {
            Some((section, sub)) => vec![section.to_ascii_lowercase(), sub.to_ascii_lowercase()],
            None => vec![trimmed.to_ascii_lowercase()],
        })
    }

    fn entry(&mut self, index: usize, start: usize, body: &str) -> Result<usize, Error> {
        let separator = body.find(self.rules.separators);
        let name_end = match separator {
            Some(position) => position,
            None => self.value_end(body, 0).0,
        };
        let raw_name = body[..name_end].trim_end_matches([' ', '\t']);
        if raw_name.is_empty() {
            return Err(parse_error(FORMAT, "key name is empty", self.text, start));
        }
        let (name, list_suffix) = match raw_name.strip_suffix("[]") {
            Some(base) if self.profile == IniProfile::Php => {
                (base.trim_end_matches([' ', '\t']).to_owned(), true)
            }
            _ => (raw_name.to_owned(), false),
        };
        let name_range = start..start + raw_name.len();
        let Some(separator) = separator else {
            let trailing = self.trailing_comment(body, raw_name.len());
            let comment = self.take_comment(trailing);
            self.push_entry(Entry {
                section: self.parsed.sections.len() - 1,
                name,
                list_suffix,
                first_line: index,
                last_line: index,
                name_range: name_range.clone(),
                value_range: name_range.end..name_range.end,
                value: None,
                comment,
            });
            return Ok(index + 1);
        };
        let after = &body[separator + 1..];
        let value_start = start + separator + 1 + (after.len() - after.trim_start_matches([' ', '\t']).len());
        let (last_line, value_end, trailing) = self.value_lines(index, value_start)?;
        let raw = &self.text[value_start..value_end];
        let value = decode(self.profile, raw)
            .map_err(|message| parse_error(FORMAT, message, self.text, value_start))?;
        let comment = self.take_comment(trailing);
        self.push_entry(Entry {
            section: self.parsed.sections.len() - 1,
            name,
            list_suffix,
            first_line: index,
            last_line,
            name_range,
            value_range: value_start..value_end,
            value: Some(value),
            comment,
        });
        Ok(last_line + 1)
    }

    fn push_entry(&mut self, entry: Entry) {
        let section = entry.section;
        self.parsed.entries.push(entry);
        let id = self.parsed.entries.len() - 1;
        self.parsed.sections[section].entries.push(id);
    }

    fn trailing_comment(&self, body: &str, from: usize) -> Option<String> {
        let rest = body[from..].trim_start_matches([' ', '\t']);
        let c = rest.chars().next()?;
        self.rules
            .trailing_comment_chars
            .contains(&c)
            .then(|| rest[c.len_utf8()..].trim().to_owned())
    }

    /// Finds the end of a value that starts at `value_start` on line
    /// `index`, following continuation lines; returns the last line, the end
    /// of the raw value and the trailing comment.
    fn value_lines(
        &self,
        mut index: usize,
        value_start: usize,
    ) -> Result<(usize, usize, Option<String>), Error> {
        let mut from = value_start;
        loop {
            let line = self.parsed.lines[index];
            let body = &self.text[from..line.content_end];
            let (end, comment_at) = self.value_end(body, 0);
            let raw = body[..end].trim_end_matches([' ', '\t']);
            let continued = comment_at.is_none()
                && self.rules.continuation != Continuation::None
                && ends_with_continuation(raw, self.profile)
                && index + 1 < self.parsed.lines.len();
            if !continued {
                if self.profile == IniProfile::Gitconfig && open_quote(&self.text[value_start..from + raw.len()]) {
                    return Err(parse_error(FORMAT, "quoted value is not terminated", self.text, from));
                }
                let trailing = comment_at.map(|at| body[at + 1..].trim().to_owned());
                return Ok((index, from + raw.len(), trailing));
            }
            index += 1;
            from = self.parsed.lines[index].start;
        }
    }

    /// End of the value part of `body` from `from`: the offset of a trailing
    /// comment marker outside quotes, or the length of `body`; the second
    /// element is the offset of the marker.
    fn value_end(&self, body: &str, from: usize) -> (usize, Option<usize>) {
        if self.rules.trailing_comment_chars.is_empty() {
            return (body.len(), None);
        }
        let quotes: &[char] = match self.profile {
            IniProfile::Php => &['"', '\''],
            _ => &['"'],
        };
        let mut quote = None;
        let mut escaped = false;
        for (i, c) in body.char_indices().skip_while(|(i, _)| *i < from) {
            match (quote, c) {
                _ if escaped => escaped = false,
                (Some('"'), '\\') | (None, '\\') if self.profile == IniProfile::Gitconfig => {
                    escaped = true;
                }
                (Some('"'), '\\') => escaped = true,
                (Some(q), _) if c == q => quote = None,
                (None, _) if quotes.contains(&c) => quote = Some(c),
                (None, _) if self.rules.trailing_comment_chars.contains(&c) => {
                    return (i, Some(i));
                }
                _ => {}
            }
        }
        (body.len(), None)
    }
}

/// Returns `true` when the raw value of one line ends with a continuation
/// backslash; Git requires the backslash itself to be unescaped.
fn ends_with_continuation(raw: &str, profile: IniProfile) -> bool {
    let backslashes = raw.len() - raw.trim_end_matches('\\').len();
    match profile {
        IniProfile::Gitconfig => backslashes % 2 == 1,
        _ => backslashes > 0,
    }
}

/// Returns `true` when a Git value leaves a quotation open.
fn open_quote(raw: &str) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    for c in raw.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            _ => {}
        }
    }
    quoted
}

/// Subsection names of Git configuration: `\"` and `\\` are escapes, a `\`
/// before another character is dropped.
fn unescape_subsection(inner: &str) -> String {
    let mut name = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            name.extend(chars.next());
        } else {
            name.push(c);
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(text: &str, profile: IniProfile) -> Vec<(Vec<String>, String, Option<String>)> {
        let parsed = parse(text, profile).unwrap();
        parsed
            .entries
            .iter()
            .map(|e| {
                (
                    parsed.sections[e.section].path.clone(),
                    e.name.clone(),
                    e.value.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn parses_global_keys_sections_and_comments() {
        let text = "\u{feff}top = 1\n; note\n[main]\nkey: value\n  # indented\nflag\n";
        let parsed = parse(text, IniProfile::Generic).unwrap();
        assert_eq!(parsed.sections.len(), 2);
        assert_eq!(parsed.entries[0].name, "top");
        assert_eq!(parsed.sections[1].comment.as_deref(), Some("note"));
        assert_eq!(parsed.entries[1].value.as_deref(), Some("value"));
        assert_eq!(parsed.entries[2].value, None);
        assert!(parsed.comment_lines[4]);
        assert_eq!(&text[parsed.entries[1].value_range.clone()], "value");
    }

    #[test]
    fn follows_continuation_lines() {
        let text = "[s]\na = one \\\n  two\nb = 3\n";
        let found = entries(text, IniProfile::Generic);
        assert_eq!(found[0].2.as_deref(), Some("one   two"));
        assert_eq!(found[1].1, "b");
        let parsed = parse(text, IniProfile::Generic).unwrap();
        assert_eq!(parsed.entries[0].last_line, 2);
    }

    #[test]
    fn reads_git_sections_subsections_and_trailing_comments() {
        let text = "[core]\n\tbare = false ; c\n[remote \"Or\\\"ig\"] # r\n\turl = x\n[Branch.Main]\n\tflag\n";
        let parsed = parse(text, IniProfile::Gitconfig).unwrap();
        let paths: Vec<_> = parsed.sections.iter().map(|s| s.path.clone()).collect();
        assert_eq!(paths[2], ["remote", "Or\"ig"]);
        assert_eq!(paths[3], ["branch", "main"]);
        assert_eq!(parsed.entries[0].value.as_deref(), Some("false"));
        assert_eq!(parsed.entries[0].comment.as_deref(), Some("c"));
        assert_eq!(parsed.sections[2].comment.as_deref(), Some("r"));
        assert_eq!(parsed.entries[2].value, None);
    }

    #[test]
    fn reads_php_lists_and_quoted_values() {
        let text = "[PHP]\nextension[] = a ; first\nextension[] = \"b;c\"\n";
        let found = entries(text, IniProfile::Php);
        assert_eq!(found[0].1, "extension");
        assert_eq!(found[1].2.as_deref(), Some("b;c"));
        assert!(parse(text, IniProfile::Php).unwrap().entries[0].list_suffix);
    }

    #[test]
    fn keeps_glob_section_names_of_editorconfig() {
        let text = "root = true\n[*.{js,py}]\nindent_style = space\n[lib/[ab].c]\nx = 1\n";
        let parsed = parse(text, IniProfile::Editorconfig).unwrap();
        assert_eq!(parsed.sections[1].path, ["*.{js,py}"]);
        assert_eq!(parsed.sections[2].path, ["lib/[ab].c"]);
    }

    #[test]
    fn rejects_malformed_lines() {
        let cases = [
            ("[open\n", IniProfile::Generic),
            ("[]\n", IniProfile::Generic),
            ("= value\n", IniProfile::Generic),
            ("[a] tail\n", IniProfile::Generic),
            ("[a]\nk = \"open\n", IniProfile::Gitconfig),
            ("[a \"b]\n", IniProfile::Gitconfig),
            ("[a b]\n", IniProfile::Gitconfig),
            ("[a]\nk = bad \\q\n", IniProfile::Gitconfig),
        ];
        for (text, profile) in cases {
            assert!(parse(text, profile).is_err(), "{text:?}");
        }
    }

    #[test]
    fn systemd_continuations_join_with_spaces() {
        let text = "[Service]\nExecStart=/bin/a \\\n    --b\nExecStart=\n";
        let found = entries(text, IniProfile::Systemd);
        assert_eq!(found[0].2.as_deref(), Some("/bin/a --b"));
        assert_eq!(found[1].2.as_deref(), Some(""));
    }
}
