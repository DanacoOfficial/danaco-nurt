// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Escapes of property files as defined by `java.util.Properties`.

/// Number of hexadecimal digits of a `\uHHHH` escape.
const UNICODE_DIGITS: usize = 4;

/// Highest character of ISO-8859-1.
const LATIN1_MAX: char = '\u{ff}';

/// Decodes the escapes of a key or value: `\t`, `\n`, `\r`, `\f`,
/// `\uHHHH` (with UTF-16 surrogate pairs) and `\c` for any other `c`.
///
/// Returns a message describing the defect for a malformed `\u` escape or
/// an unpaired surrogate.
pub(crate) fn unescape(raw: &[char]) -> Result<String, &'static str> {
    let mut out = String::with_capacity(raw.len());
    let mut pending_high: Option<u16> = None;
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        i += 1;
        if c != '\\' {
            if pending_high.is_some() {
                return Err("unpaired surrogate in a \\u escape");
            }
            out.push(c);
            continue;
        }
        let Some(&next) = raw.get(i) else {
            break;
        };
        i += 1;
        let unit = match next {
            'u' => {
                let digits: String = raw
                    .get(i..i + UNICODE_DIGITS)
                    .ok_or(MALFORMED)?
                    .iter()
                    .collect();
                i += UNICODE_DIGITS;
                if !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
                    return Err(MALFORMED);
                }
                u16::from_str_radix(&digits, 16).map_err(|_| MALFORMED)?
            }
            other => {
                if pending_high.is_some() {
                    return Err("unpaired surrogate in a \\u escape");
                }
                out.push(match other {
                    't' => '\t',
                    'n' => '\n',
                    'r' => '\r',
                    'f' => '\u{c}',
                    _ => other,
                });
                continue;
            }
        };
        match (pending_high.take(), unit) {
            (None, 0xD800..=0xDBFF) => pending_high = Some(unit),
            (Some(high), 0xDC00..=0xDFFF) => {
                let c = char::decode_utf16([high, unit])
                    .next()
                    .and_then(Result::ok)
                    .ok_or("unpaired surrogate in a \\u escape")?;
                out.push(c);
            }
            (None, _) => out
                .push(char::from_u32(u32::from(unit)).ok_or("unpaired surrogate in a \\u escape")?),
            (Some(_), _) => return Err("unpaired surrogate in a \\u escape"),
        }
    }
    if pending_high.is_some() {
        return Err("unpaired surrogate in a \\u escape");
    }
    Ok(out)
}

const MALFORMED: &str = "malformed \\uHHHH escape; write four hexadecimal digits";

/// Part of a key or value written by [`escape`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    Key,
    /// First line of a value: a leading space is escaped.
    Value,
    /// Continuation line of a value: leading whitespace is escaped as well,
    /// since the reader skips it.
    Continuation,
}

/// Escapes `text` for a property file; characters outside ISO-8859-1 are
/// written as `\uHHHH` when `latin1` is set, control characters always.
pub(crate) fn escape(text: &str, part: Part, latin1: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            ' ' if part == Part::Key || i == 0 => out.push_str("\\ "),
            '=' | ':' if part == Part::Key => {
                out.push('\\');
                out.push(c);
            }
            '#' | '!' if i == 0 && part == Part::Key => {
                out.push('\\');
                out.push(c);
            }
            _ if c.is_control() || (latin1 && c > LATIN1_MAX) => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04X}"));
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    #[test]
    fn decodes_the_escapes_of_java_properties() {
        let cases = [
            (r"a\tb", "a\tb"),
            (r"\:\=\ \#x", ":= #x"),
            (r"\u00e9t\u00E9", "été"),
            (r"\uD83D\uDE00", "\u{1F600}"),
            (r"c:\\dir", "c:\\dir"),
            (r"\q", "q"),
            ("end\\", "end"),
        ];
        for (raw, expected) in cases {
            assert_eq!(unescape(&chars(raw)).as_deref(), Ok(expected), "{raw}");
        }
    }

    #[test]
    fn rejects_malformed_unicode_escapes() {
        for raw in [
            r"\u12", r"\uXYZW", r"\uD83Dx", r"\uDE00", r"\uD83D", r"\u+123", r"\u-123", r"\u 123",
        ] {
            assert!(unescape(&chars(raw)).is_err(), "{raw}");
        }
    }

    #[test]
    fn escapes_values_that_read_back_unchanged() {
        let samples = [
            " lead",
            "a=b:c",
            "tab\tnl\n",
            "#hash",
            "λ€üé",
            "\u{1F600}",
            "\\",
        ];
        for latin1 in [true, false] {
            for part in [Part::Key, Part::Value, Part::Continuation] {
                for sample in samples {
                    let escaped = escape(sample, part, latin1);
                    assert_eq!(unescape(&chars(&escaped)).as_deref(), Ok(sample));
                    if latin1 {
                        assert!(escaped.chars().all(|c| c <= LATIN1_MAX), "{escaped}");
                    }
                }
            }
        }
        assert_eq!(escape("a b", Part::Key, false), "a\\ b");
        assert_eq!(escape("a b", Part::Value, false), "a b");
        assert_eq!(escape("€", Part::Value, true), "\\u20AC");
    }
}
