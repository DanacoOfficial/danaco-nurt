// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Canonical notation of text and regular expression literals.

/// Writes a text value in canonical notation.
///
/// The raw form `r#"…"#` with the fewest `#` is used when the text contains
/// `"` or `\` and no control character other than tab and line feed;
/// otherwise the text is quoted with escapes.
pub(crate) fn text_literal(value: &str) -> String {
    let needs_raw = value.contains(['"', '\\']);
    let raw_allowed = value
        .chars()
        .all(|c| !c.is_control() || c == '\t' || c == '\n');
    if needs_raw && raw_allowed {
        let hashes = raw_hashes(value);
        let fence = "#".repeat(hashes);
        return format!("r{fence}\"{value}\"{fence}");
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            _ if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Smallest number of `#` such that `"` followed by that many `#` does not
/// occur in the text.
fn raw_hashes(value: &str) -> usize {
    (0..)
        .find(|&hashes| !value.contains(&format!("\"{}", "#".repeat(hashes))))
        .unwrap_or(0)
}

/// Writes a regular expression in canonical notation `/v/flags`.
pub(crate) fn regex_literal(pattern: &str, flags: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + flags.len() + 2);
    out.push('/');
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push('\\');
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            '/' => out.push_str("\\/"),
            _ => out.push(c),
        }
    }
    out.push('/');
    out.extend("imsx".chars().filter(|flag| flags.contains(*flag)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_quoted() {
        assert_eq!(text_literal("*.go"), "\"*.go\"");
        assert_eq!(text_literal("a\u{1}b"), "\"a\\u{1}b\"");
    }

    #[test]
    fn text_with_backslash_or_quote_is_raw() {
        assert_eq!(text_literal(r"C:\proj"), r#"r"C:\proj""#);
        assert_eq!(text_literal(r#"say "hi""#), r##"r#"say "hi""#"##);
        assert_eq!(text_literal("a\"#b"), "r##\"a\"#b\"##");
    }

    #[test]
    fn text_with_control_characters_keeps_escapes() {
        assert_eq!(text_literal("a\"\rb"), "\"a\\\"\\rb\"");
    }

    #[test]
    fn regex_escapes_slashes_and_orders_flags() {
        assert_eq!(regex_literal("a/b", "xi"), "/a\\/b/ix");
        assert_eq!(regex_literal("\\d+", ""), "/\\d+/");
    }
}
