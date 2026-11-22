// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the character set of a body and decoding to text.

use encoding_rs::Encoding;
use nurt_contracts::error::{Error, ErrorCode, Result};

/// Bytes at the start of an HTML document searched for a `<meta>` charset
/// declaration, as in the HTML standard prescan.
const META_PRESCAN: usize = 1024;
/// Bytes inspected when deciding whether a body without a media type is
/// binary.
const BINARY_SNIFF: usize = 1024;

/// Text decoded from a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// Text.
    pub text: String,
    /// Name of the character set used, in lower case.
    pub charset: String,
}

/// Decodes a body to text.
///
/// The character set is taken from the first source that names one:
///
/// - `forced`, unless it is `auto`;
/// - A byte order mark;
/// - The `charset` parameter of the response;
/// - A `<meta>` declaration in the first 1024 bytes of an HTML body;
/// - UTF-8, when the body is valid UTF-8;
/// - The guess of `chardetng`.
///
/// # Errors
///
/// `E_ARG` when `forced` names an unknown character set.
///
/// ```
/// use nurt_net::html::decode;
///
/// let bytes = b"<meta charset=\"windows-1250\"><p>\xb9\xe6";
/// let decoded = decode(bytes, None, None, true).unwrap();
/// assert_eq!(decoded.charset, "windows-1250");
/// assert!(decoded.text.ends_with("\u{105}\u{107}"));
/// ```
pub fn decode(
    bytes: &[u8],
    declared: Option<&str>,
    forced: Option<&str>,
    html: bool,
) -> Result<Decoded> {
    if let Some(label) = forced.filter(|label| !label.eq_ignore_ascii_case("auto")) {
        let encoding = Encoding::for_label(label.trim().as_bytes()).ok_or_else(|| {
            Error::new(
                ErrorCode::Arg,
                format!(
                    "`{label}` is not a known character set; use for example utf-8 or iso-8859-2"
                ),
            )
        })?;
        return Ok(with(encoding, bytes));
    }
    if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        return Ok(with(encoding, bytes));
    }
    let labelled = declared
        .and_then(|label| Encoding::for_label(label.trim().as_bytes()))
        .or_else(|| if html { meta_charset(bytes) } else { None });
    if let Some(encoding) = labelled {
        return Ok(with(encoding, bytes));
    }
    if std::str::from_utf8(bytes).is_ok() {
        return Ok(with(encoding_rs::UTF_8, bytes));
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    Ok(with(
        detector.guess(None, chardetng::Utf8Detection::Allow),
        bytes,
    ))
}

fn with(encoding: &'static Encoding, bytes: &[u8]) -> Decoded {
    let (text, used, _) = encoding.decode(bytes);
    Decoded {
        text: text.into_owned(),
        charset: used.name().to_ascii_lowercase(),
    }
}

fn meta_charset(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = &bytes[..bytes.len().min(META_PRESCAN)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let mut rest = text.as_str();
    while let Some(position) = rest.find("charset") {
        rest = &rest[position + "charset".len()..];
        let value = rest.trim_start().strip_prefix('=')?.trim_start();
        let value = value.trim_start_matches(['"', '\'']);
        let end = value
            .find(|c: char| matches!(c, '"' | '\'' | ';' | '>' | '/') || c.is_whitespace())
            .unwrap_or(value.len());
        if let Some(encoding) = Encoding::for_label(&value.as_bytes()[..end]) {
            return Some(encoding);
        }
    }
    None
}

/// Whether a body of `media_type` should be read as text.
///
/// Without a media type, a body is text when its first 1024 bytes hold no
/// NUL byte.
#[must_use]
pub fn is_textual(media_type: Option<&str>, bytes: &[u8]) -> bool {
    match media_type {
        Some(media) => {
            media.starts_with("text/")
                || media.ends_with("+json")
                || media.ends_with("+xml")
                || matches!(
                    media,
                    "application/json"
                        | "application/xml"
                        | "application/javascript"
                        | "application/ecmascript"
                        | "application/x-www-form-urlencoded"
                        | "application/yaml"
                        | "application/x-yaml"
                        | "application/toml"
                        | "application/x-sh"
                )
        }
        None => !bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_forced_then_bom_then_declared() {
        let latin2 = b"\xb1\xe6";
        assert_eq!(
            decode(latin2, None, Some("iso-8859-2"), false)
                .unwrap()
                .text,
            "\u{105}\u{107}"
        );
        let bom = b"\xef\xbb\xbfa\xc5\xbc";
        let decoded = decode(bom, Some("iso-8859-2"), None, false).unwrap();
        assert_eq!(decoded.charset, "utf-8");
        assert!(decoded.text.ends_with("a\u{17c}"));
        assert_eq!(
            decode(b"\xb9", Some("windows-1250"), None, false)
                .unwrap()
                .text,
            "\u{105}"
        );
    }

    #[test]
    fn rejects_unknown_forced_charset() {
        let error = decode(b"x", None, Some("klingon"), false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg);
    }

    #[test]
    fn guesses_legacy_text_without_declaration() {
        let text = "Letters \u{105}\u{107}\u{119}\u{142}\u{144}\u{f3}\u{15b}\u{17a}\u{17c} \
                    in a line of text without a declared character set.";
        let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode(text);
        let decoded = decode(&bytes, None, None, true).unwrap();
        assert_ne!(decoded.charset, "utf-8");
        assert!(!decoded.text.contains(char::REPLACEMENT_CHARACTER));
        assert_eq!(decoded.text.chars().count(), text.chars().count());
    }

    #[test]
    fn classifies_media_types() {
        assert!(is_textual(Some("text/html"), b""));
        assert!(is_textual(Some("application/ld+json"), b""));
        assert!(!is_textual(Some("image/png"), b""));
        assert!(!is_textual(None, b"\x89PNG\x00"));
        assert!(is_textual(None, b"plain"));
    }
}
