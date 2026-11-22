// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text codec service of the text commands.
//!
//! [`TextCodec`] implements [`TextCodecService`] for the commands of other
//! groups that decode or encode files: detection follows
//! [`detect`](crate::detect), conversion follows [`codec`](crate::codec) and
//! line endings follow [`eol`](crate::eol).

use encoding_rs::{UTF_16BE, UTF_16LE};
use nurt_contracts::error::Result;
use nurt_contracts::services::{LineEnding, TextCodecService, TextDetection, TextEncoding};

use crate::codec::{self, Codec};
use crate::detect::{self, DetectOptions, Detection, LegacyEncoding};
use crate::eol;

/// Text codec with the detection rules of the text commands.
///
/// Decoding followed by encoding with the same [`TextEncoding`] returns the
/// original bytes; content for which that cannot hold fails with
/// `E_ENCODING`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::services::{LineEnding, TextCodecService};
/// use nurt_text::TextCodec;
///
/// let codec = TextCodec::default();
/// let detection = codec.detect(b"first\r\nsecond\r\n");
/// assert!(!detection.binary);
/// assert_eq!(detection.line_ending, LineEnding::CrLf);
///
/// let encoding = detection.encoding.ok_or("binary content")?;
/// let text = codec.decode(b"first\r\n", &encoding)?;
/// assert_eq!(codec.encode(&text, &encoding)?, b"first\r\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextCodec {
    options: DetectOptions,
}

impl TextCodec {
    /// Codec with the given detection settings.
    #[must_use]
    pub fn new(options: DetectOptions) -> Self {
        Self { options }
    }

    /// Codec that prefers the given legacy encodings, in order, as listed by
    /// the configuration key `text.legacy_encodings`.
    ///
    /// Labels that denote no legacy encoding (UTF-8, UTF-16 and unknown
    /// labels) are left out.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::TextCodec;
    ///
    /// let codec = TextCodec::with_legacy_encodings(["cp852", "utf-8", "latin2"]);
    /// let labels: Vec<String> = codec
    ///     .options()
    ///     .legacy_encodings()
    ///     .iter()
    ///     .map(|encoding| encoding.label())
    ///     .collect();
    /// assert_eq!(labels, ["ibm852", "iso-8859-2"]);
    /// ```
    #[must_use]
    pub fn with_legacy_encodings<I, S>(labels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let encodings = labels
            .into_iter()
            .filter_map(|label| LegacyEncoding::from_label(label.as_ref()));
        Self::new(DetectOptions::default().with_legacy_encodings(encodings))
    }

    /// Detection settings.
    #[must_use]
    pub fn options(&self) -> &DetectOptions {
        &self.options
    }
}

impl TextCodecService for TextCodec {
    fn detect(&self, bytes: &[u8]) -> TextDetection {
        match detect::detect(bytes, &self.options) {
            Detection::Text(encoding) => TextDetection {
                binary: false,
                line_ending: line_ending(bytes, &encoding),
                encoding: Some(encoding),
            },
            Detection::Binary => TextDetection {
                binary: true,
                encoding: None,
                line_ending: LineEnding::None,
            },
        }
    }

    fn decode(&self, bytes: &[u8], encoding: &TextEncoding) -> Result<String> {
        codec::decode(bytes, encoding).map_err(Into::into)
    }

    fn encode(&self, text: &str, encoding: &TextEncoding) -> Result<Vec<u8>> {
        codec::encode(text, encoding).map_err(Into::into)
    }
}

/// Line ending convention of content in a known encoding.
///
/// UTF-16 content is decoded first, with malformed code units replaced,
/// because its terminators are two bytes wide; content in other encodings is
/// examined as bytes.
///
/// # Examples
///
/// ```
/// use nurt_contracts::services::{LineEnding, TextEncoding};
/// use nurt_text::codec::Codec;
/// use nurt_text::codec_service::line_ending;
///
/// let utf16: Vec<u8> = "a\r\nb".encode_utf16().flat_map(u16::to_le_bytes).collect();
/// assert_eq!(line_ending(&utf16, &Codec::Utf16Le.text_encoding(false)), LineEnding::CrLf);
/// assert_eq!(line_ending(b"a\nb\n", &TextEncoding::utf8()), LineEnding::Lf);
/// ```
#[must_use]
pub fn line_ending(bytes: &[u8], encoding: &TextEncoding) -> LineEnding {
    let (codec, whatwg) = match Codec::from_label(&encoding.label) {
        Ok(codec @ Codec::Utf16Le) => (codec, UTF_16LE),
        Ok(codec @ Codec::Utf16Be) => (codec, UTF_16BE),
        _ => return eol::detect(bytes),
    };
    let body = match codec.bom() {
        Some(bom) if encoding.bom => bytes.strip_prefix(bom).unwrap_or(bytes),
        _ => bytes,
    };
    let (text, _) = whatwg.decode_without_bom_handling(body);
    eol::detect(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::services::Services;

    use super::*;

    /// Polish sentence with letters outside ASCII, for UTF-16 detection.
    const POLISH: &str = "Pchn\u{105}\u{107} w t\u{119} \u{142}\u{f3}d\u{17a} je\u{17c}a lub o\u{15b}m skrzy\u{144} fig.";

    fn utf16(text: &str, little: bool) -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|unit| {
                if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                }
            })
            .collect()
    }

    fn label(detection: &TextDetection) -> Option<&str> {
        detection
            .encoding
            .as_ref()
            .map(|encoding| encoding.label.as_str())
    }

    #[test]
    fn detects_utf8_with_line_feeds() {
        let detection = TextCodec::default().detect(b"fn main() {}\n");
        assert!(!detection.binary);
        assert_eq!(detection.encoding, Some(TextEncoding::utf8()));
        assert_eq!(detection.line_ending, LineEnding::Lf);
    }

    #[test]
    fn detects_legacy_encoding_with_crlf() {
        let detection = TextCodec::default()
            .detect(b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n");
        assert_eq!(label(&detection), Some("windows-1250"));
        assert_eq!(detection.line_ending, LineEnding::CrLf);
    }

    #[test]
    fn detects_utf16_with_byte_order_mark() {
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(utf16("one\r\ntwo\r\n", true));
        let detection = TextCodec::default().detect(&bytes);
        assert_eq!(label(&detection), Some("utf-16le"));
        assert_eq!(detection.encoding.map(|encoding| encoding.bom), Some(true));
        assert_eq!(detection.line_ending, LineEnding::CrLf);
    }

    #[test]
    fn detects_utf16_without_byte_order_mark() {
        let text = format!("{POLISH}\r{POLISH}\r");
        let detection = TextCodec::default().detect(&utf16(&text, false));
        assert_eq!(label(&detection), Some("utf-16be"));
        assert_eq!(detection.line_ending, LineEnding::Cr);
    }

    #[test]
    fn reports_mixed_and_missing_line_endings() {
        let codec = TextCodec::default();
        assert_eq!(codec.detect(b"a\nb\r\n").line_ending, LineEnding::Mixed);
        assert_eq!(codec.detect(b"single line").line_ending, LineEnding::None);
    }

    #[test]
    fn binary_content_has_no_encoding() {
        let detection = TextCodec::default().detect(b"\x7fELF\x02\x01\x01\x00\x00\x00\n");
        assert!(detection.binary);
        assert_eq!(detection.encoding, None);
        assert_eq!(detection.line_ending, LineEnding::None);
    }

    #[test]
    fn decoding_and_encoding_round_trip() {
        let codec = TextCodec::default();
        let encoding = Codec::Windows1250.text_encoding(false);
        let bytes = b"\xbf\xf3\xb3w\r\n";
        let text = codec.decode(bytes, &encoding).unwrap();
        assert_eq!(text, "\u{17c}\u{f3}\u{142}w\r\n");
        assert_eq!(codec.encode(&text, &encoding).unwrap(), bytes);
    }

    #[test]
    fn byte_order_mark_is_removed_and_restored() {
        let codec = TextCodec::default();
        let encoding = Codec::Utf8.text_encoding(true);
        let text = codec.decode(b"\xef\xbb\xbfa\n", &encoding).unwrap();
        assert_eq!(text, "a\n");
        assert_eq!(codec.encode(&text, &encoding).unwrap(), b"\xef\xbb\xbfa\n");
    }

    #[test]
    fn malformed_content_fails_with_encoding_error() {
        let error = TextCodec::default()
            .decode(b"ok \xff\xfe", &TextEncoding::utf8())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
    }

    #[test]
    fn unrepresentable_character_fails_with_encoding_error() {
        let error = TextCodec::default()
            .encode("\u{4e2d}", &Codec::Ibm852.text_encoding(false))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
    }

    #[test]
    fn unknown_label_fails_with_unknown_value() {
        let encoding = TextEncoding {
            label: "koi8-r".to_owned(),
            bom: false,
        };
        let codec = TextCodec::default();
        assert_eq!(
            codec.decode(b"a", &encoding).unwrap_err().code(),
            ErrorCode::UnknownValue
        );
        assert_eq!(
            codec.encode("a", &encoding).unwrap_err().code(),
            ErrorCode::UnknownValue
        );
    }

    #[test]
    fn legacy_preference_selects_the_encoding() {
        let text =
            "\u{179}r\u{f3}d\u{142}o: \u{17c}\u{f3}\u{142}w, g\u{119}\u{15b} i \u{142}\u{105}ka.\n";
        let bytes = codec::encode(text, &Codec::Ibm852.text_encoding(false)).unwrap();
        let preferred = TextCodec::with_legacy_encodings(["ibm852"]);
        assert_eq!(label(&preferred.detect(&bytes)), Some("ibm852"));
        let other = TextCodec::with_legacy_encodings(["windows-1250"]);
        assert_ne!(label(&other.detect(&bytes)), Some("ibm852"));
    }

    #[test]
    fn settings_are_kept() {
        let options = DetectOptions::default().with_legacy_encodings([]);
        let codec = TextCodec::new(options.clone());
        assert_eq!(codec.options(), &options);
        assert!(
            TextCodec::with_legacy_encodings(["utf-8", "no-such-label"])
                .options()
                .legacy_encodings()
                .is_empty()
        );
    }

    #[test]
    fn line_endings_of_utf16_ignore_the_byte_order_mark() {
        let mut bytes = vec![0xfe, 0xff];
        bytes.extend(utf16("a\nb\n", false));
        let encoding = Codec::Utf16Be.text_encoding(true);
        assert_eq!(line_ending(&bytes, &encoding), LineEnding::Lf);
        let mut odd = utf16("a\r\nb", true);
        odd.push(b'x');
        assert_eq!(
            line_ending(&odd, &Codec::Utf16Le.text_encoding(false)),
            LineEnding::CrLf
        );
    }

    #[test]
    fn line_endings_of_unknown_labels_are_read_from_bytes() {
        let encoding = TextEncoding {
            label: "unknown".to_owned(),
            bom: false,
        };
        assert_eq!(line_ending(b"a\r\n", &encoding), LineEnding::CrLf);
    }

    #[test]
    fn codec_serves_the_service_registry() {
        let services = Services::new().with_text_codec(Arc::new(TextCodec::default()));
        let codec = services.text_codec().unwrap();
        assert!(!codec.detect(b"text").binary);
    }
}
