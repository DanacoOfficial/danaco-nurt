// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Decoding and encoding of INI values by profile.

use super::profile::IniProfile;

/// Decodes the raw text of a value, which may include continuation line
/// breaks, into its value.
///
/// Returns a message describing the defect when the raw text is malformed.
pub(crate) fn decode(profile: IniProfile, raw: &str) -> Result<String, &'static str> {
    match profile {
        IniProfile::Systemd => Ok(join_continuations(raw, true)),
        IniProfile::Gitconfig => decode_git(raw),
        IniProfile::Php => Ok(decode_php(raw)),
        IniProfile::Desktop => Ok(decode_desktop(raw)),
        _ => Ok(join_continuations(raw, false)),
    }
}

/// Joins continuation lines: a final `\` and the line break are removed;
/// with `spaced`, the lines are trimmed and joined by one space.
fn join_continuations(raw: &str, spaced: bool) -> String {
    let parts = split_continuations(raw);
    if spaced {
        parts
            .iter()
            .map(|part| part.trim_matches([' ', '\t']))
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end()
            .to_owned()
    } else {
        parts.concat().trim_end_matches([' ', '\t']).to_owned()
    }
}

fn split_continuations(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = raw;
    while let Some(position) = rest.find('\n') {
        let line = &rest[..position];
        let line = line.strip_suffix('\r').unwrap_or(line);
        parts.push(line.strip_suffix('\\').unwrap_or(line));
        rest = &rest[position + 1..];
    }
    parts.push(rest);
    parts
}

/// Git value rules: whitespace outside quotes is kept inside the value as
/// spaces and dropped at both ends, `"` toggles quoting, `\` escapes `\`,
/// `"`, `n`, `t`, `b` and a line break, `#` and `;` outside quotes start a
/// comment.
fn decode_git(raw: &str) -> Result<String, &'static str> {
    let mut value = String::new();
    let mut quoted = false;
    let mut spaces = 0;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if !quoted && (c == ' ' || c == '\t') {
            if !value.is_empty() {
                spaces += 1;
            }
            continue;
        }
        if !quoted && (c == '#' || c == ';') {
            break;
        }
        if c == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        value.extend(std::iter::repeat_n(' ', spaces));
        spaces = 0;
        match c {
            '\\' => match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some('n') => value.push('\n'),
                Some('t') => value.push('\t'),
                Some('b') => value.push('\u{8}'),
                Some(escaped @ ('\\' | '"')) => value.push(escaped),
                _ => return Err("invalid escape sequence in value"),
            },
            '"' => quoted = !quoted,
            _ => value.push(c),
        }
    }
    if quoted {
        Err("quoted value is not terminated")
    } else {
        Ok(value)
    }
}

/// PHP value rules: `"…"` with `\"`, `'…'` taken literally, text outside
/// quotes trimmed; `;` outside quotes starts a comment.
fn decode_php(raw: &str) -> String {
    let mut value = String::new();
    let mut chars = raw.chars().peekable();
    let mut outside = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                value.push_str(outside.trim_matches([' ', '\t']));
                outside.clear();
                while let Some(q) = chars.next() {
                    if q == c {
                        break;
                    }
                    if c == '"' && q == '\\' && chars.peek() == Some(&'"') {
                        value.push('"');
                        chars.next();
                    } else {
                        value.push(q);
                    }
                }
            }
            ';' => break,
            _ => outside.push(c),
        }
    }
    value.push_str(outside.trim_matches([' ', '\t']));
    value
}

/// Desktop entry escapes: `\s`, `\n`, `\t`, `\r` and `\\`.
fn decode_desktop(raw: &str) -> String {
    let mut value = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            value.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => value.push(' '),
            Some('n') => value.push('\n'),
            Some('t') => value.push('\t'),
            Some('r') => value.push('\r'),
            Some('\\') => value.push('\\'),
            Some(other) => {
                value.push('\\');
                value.push(other);
            }
            None => value.push('\\'),
        }
    }
    value
}

/// Raw text written for `value` in `profile`; `old_raw` is the raw text of
/// the value being replaced, whose quoting style is kept when it can hold
/// the new value.
///
/// Returns `None` when the profile cannot represent the value on one line.
pub(crate) fn encode(profile: IniProfile, value: &str, old_raw: Option<&str>) -> Option<String> {
    match profile {
        IniProfile::Gitconfig => Some(encode_git(value, old_raw)),
        IniProfile::Php => encode_php(value, old_raw),
        IniProfile::Desktop => Some(encode_desktop(value)),
        _ => encode_plain(profile, value),
    }
}

/// Values of the plain profiles are written as they are; line breaks, edge
/// whitespace and a final `\` would be read back differently.
fn encode_plain(profile: IniProfile, value: &str) -> Option<String> {
    let edge_space = value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']);
    let continues = value.ends_with('\\') && profile != IniProfile::Editorconfig;
    if value.contains(['\n', '\r']) || edge_space || continues {
        None
    } else {
        Some(value.to_owned())
    }
}

fn encode_git(value: &str, old_raw: Option<&str>) -> String {
    let mut escaped = String::new();
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\u{8}' => escaped.push_str("\\b"),
            _ => escaped.push(c),
        }
    }
    let was_quoted = old_raw.is_some_and(|raw| raw.starts_with('"'));
    let needs_quotes = value.starts_with(' ')
        || value.ends_with(' ')
        || value.contains("  ")
        || value.contains(['#', ';']);
    if was_quoted || needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

/// Characters that end or change an unquoted PHP value.
const PHP_SPECIAL: [char; 15] = [
    ';', '"', '\'', '=', '{', '}', '|', '&', '~', '!', '[', ']', '(', ')', '^',
];

fn encode_php(value: &str, old_raw: Option<&str>) -> Option<String> {
    if value.contains(['\n', '\r']) {
        return None;
    }
    let old_quote = old_raw.and_then(|raw| raw.chars().next().filter(|c| *c == '"' || *c == '\''));
    if old_quote == Some('\'') && !value.contains('\'') {
        return Some(format!("'{value}'"));
    }
    let edge_space = value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']);
    if old_quote.is_some() || edge_space || value.contains(PHP_SPECIAL) {
        Some(format!("\"{}\"", value.replace('"', "\\\"")))
    } else {
        Some(value.to_owned())
    }
}

fn encode_desktop(value: &str) -> String {
    let mut escaped = String::new();
    for (i, c) in value.chars().enumerate() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            ' ' if i == 0 => escaped.push_str("\\s"),
            _ => escaped.push(c),
        }
    }
    if escaped.ends_with(' ') {
        escaped.pop();
        escaped.push_str("\\s");
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_git_quoting_escapes_and_comments() {
        let cases = [
            ("plain value", "plain value"),
            ("a  \t b ", "a    b"),
            ("\"  padded \" # note", "  padded "),
            ("tab\\there", "tab\there"),
            ("say \\\"hi\\\"", "say \"hi\""),
            ("first \\\n  second", "first   second"),
            ("x ; comment", "x"),
            ("\"in # quotes\"", "in # quotes"),
        ];
        for (raw, expected) in cases {
            assert_eq!(decode(IniProfile::Gitconfig, raw).as_deref(), Ok(expected), "{raw}");
        }
        assert!(decode(IniProfile::Gitconfig, "\"open").is_err());
        assert!(decode(IniProfile::Gitconfig, "bad \\q").is_err());
    }

    #[test]
    fn joins_continuation_lines_by_profile() {
        assert_eq!(decode(IniProfile::Generic, "a \\\n  b").as_deref(), Ok("a   b"));
        assert_eq!(
            decode(IniProfile::Systemd, "/bin/run \\\r\n    --flag").as_deref(),
            Ok("/bin/run --flag")
        );
    }

    #[test]
    fn decodes_php_and_desktop_values() {
        assert_eq!(decode_php("\"a;b\" ; note"), "a;b");
        assert_eq!(decode_php("'C:\\path'"), "C:\\path");
        assert_eq!(decode_php("say \"x\\\"y\""), "sayx\"y");
        assert_eq!(decode_desktop("\\sa\\tb\\\\c\\nd\\re\\q\\"), " a\tb\\c\nd\re\\q\\");
    }

    #[test]
    fn encodes_values_that_read_back_unchanged() {
        let samples = [
            "plain",
            " lead",
            "trail ",
            "a;b",
            "a#b",
            "quote\"d",
            "back\\slash",
            "two  spaces",
            "tab\tchar",
            "line\nbreak",
            "",
        ];
        for profile in [IniProfile::Gitconfig, IniProfile::Desktop, IniProfile::Php] {
            for sample in samples {
                let Some(raw) = encode(profile, sample, None) else {
                    assert_eq!(profile, IniProfile::Php);
                    continue;
                };
                assert_eq!(decode(profile, &raw).as_deref(), Ok(sample), "{profile} {raw}");
            }
        }
    }

    #[test]
    fn keeps_the_quoting_style_of_the_old_value() {
        assert_eq!(encode(IniProfile::Gitconfig, "x", Some("\"y\"")).as_deref(), Some("\"x\""));
        assert_eq!(encode(IniProfile::Php, "x", Some("'y'")).as_deref(), Some("'x'"));
        assert_eq!(encode(IniProfile::Php, "it's", Some("'y'")).as_deref(), Some("\"it's\""));
        assert_eq!(encode(IniProfile::Php, "x", Some("\"y\"")).as_deref(), Some("\"x\""));
    }

    #[test]
    fn rejects_values_plain_profiles_cannot_hold() {
        for value in ["a\nb", " a", "a ", "ends\\"] {
            assert_eq!(encode(IniProfile::Generic, value, None), None, "{value}");
        }
        assert_eq!(encode(IniProfile::Editorconfig, "x\\", None).as_deref(), Some("x\\"));
        assert_eq!(encode(IniProfile::Systemd, "a b", None).as_deref(), Some("a b"));
    }
}
