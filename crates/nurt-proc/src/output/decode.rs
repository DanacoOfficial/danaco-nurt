// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection and decoding of the text encoding of program output.
//!
//! Spool files keep the original bytes; decoding applies to the lines of a
//! response. Detection looks at the start of a stream: a byte order mark,
//! the distribution of zero bytes typical of UTF-16, NUL and control bytes
//! typical of binary data, UTF-8 validity, and finally the legacy encoding.
//! On Windows the legacy encoding is the code page the console gives to
//! child programs; elsewhere, and when that code page has no decoder, it is
//! guessed from the byte frequencies.

use encoding_rs::Encoding;
use nurt_contracts::error::{Error, ErrorCode, Result};

/// Bytes at the start of a stream used for detection.
pub(super) const DETECT_SAMPLE_BYTES: usize = 64 * 1024;

/// Share of zero bytes, in percent of the byte pairs, at the high byte
/// position of UTF-16 text written in a Latin script.
const UTF16_ZERO_PERCENT: usize = 30;

/// Share of zero bytes, in percent of the byte pairs, tolerated at the low
/// byte position of UTF-16 text.
const UTF16_STRAY_ZERO_PERCENT: usize = 5;

/// Share of control bytes, in percent, above which output is binary.
const BINARY_CONTROL_PERCENT: usize = 10;

/// Value of `output_encoding` that requests detection.
const AUTO: &str = "auto";

/// Name reported for binary output.
pub(super) const BINARY: &str = "binary";

/// Characters of code page 852 (IBM Latin-2) for the bytes 0x80 to 0xFF.
const CP852_HIGH: [u16; 128] = [
    0x00C7, 0x00FC, 0x00E9, 0x00E2, 0x00E4, 0x016F, 0x0107, 0x00E7, 0x0142, 0x00EB, 0x0150, 0x0151,
    0x00EE, 0x0179, 0x00C4, 0x0106, 0x00C9, 0x0139, 0x013A, 0x00F4, 0x00F6, 0x013D, 0x013E, 0x015A,
    0x015B, 0x00D6, 0x00DC, 0x0164, 0x0165, 0x0141, 0x00D7, 0x010D, 0x00E1, 0x00ED, 0x00F3, 0x00FA,
    0x0104, 0x0105, 0x017D, 0x017E, 0x0118, 0x0119, 0x00AC, 0x017A, 0x010C, 0x015F, 0x00AB, 0x00BB,
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x00C1, 0x00C2, 0x011A, 0x015E, 0x2563, 0x2551, 0x2557,
    0x255D, 0x017B, 0x017C, 0x2510, 0x2514, 0x2534, 0x252C, 0x251C, 0x2500, 0x253C, 0x0102, 0x0103,
    0x255A, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256C, 0x00A4, 0x0111, 0x0110, 0x010E, 0x00CB,
    0x010F, 0x0147, 0x00CD, 0x00CE, 0x011B, 0x2518, 0x250C, 0x2588, 0x2584, 0x0162, 0x016E, 0x2580,
    0x00D3, 0x00DF, 0x00D4, 0x0143, 0x0144, 0x0148, 0x0160, 0x0161, 0x0154, 0x00DA, 0x0155, 0x0170,
    0x00FD, 0x00DD, 0x0163, 0x00B4, 0x00AD, 0x02DD, 0x02DB, 0x02C7, 0x02D8, 0x00A7, 0x00F7, 0x00B8,
    0x00B0, 0x00A8, 0x02D9, 0x0171, 0x0158, 0x0159, 0x25A0, 0x00A0,
];

/// How lines of a stream end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineUnit {
    /// A line feed byte.
    Byte,
    /// A UTF-16 little-endian line feed at an even offset.
    Utf16Le,
    /// A UTF-16 big-endian line feed at an even offset.
    Utf16Be,
}

/// Text encoding of one output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextEncoding {
    /// UTF-8; invalid sequences decode to U+FFFD.
    Utf8,
    /// UTF-16 little-endian.
    Utf16Le,
    /// UTF-16 big-endian.
    Utf16Be,
    /// An encoding of the WHATWG Encoding Standard.
    Legacy(&'static Encoding),
    /// Code page 852, which the WHATWG Encoding Standard does not define.
    Cp852,
    /// Not text.
    Binary,
}

impl TextEncoding {
    /// Name reported in `decoded_as`.
    pub(super) fn name(self) -> String {
        match self {
            Self::Utf8 => "utf-8".to_owned(),
            Self::Utf16Le => "utf-16le".to_owned(),
            Self::Utf16Be => "utf-16be".to_owned(),
            Self::Legacy(encoding) => encoding.name().to_ascii_lowercase(),
            Self::Cp852 => "ibm852".to_owned(),
            Self::Binary => BINARY.to_owned(),
        }
    }

    /// How lines end in this encoding.
    pub(super) fn unit(self) -> LineUnit {
        match self {
            Self::Utf16Le => LineUnit::Utf16Le,
            Self::Utf16Be => LineUnit::Utf16Be,
            _ => LineUnit::Byte,
        }
    }

    /// Decodes one line; byte order marks are kept and removed by line
    /// cleaning.
    pub(super) fn decode(self, bytes: &[u8]) -> String {
        match self {
            Self::Utf8 | Self::Binary => String::from_utf8_lossy(bytes).into_owned(),
            Self::Utf16Le => decode_plain(encoding_rs::UTF_16LE, bytes),
            Self::Utf16Be => decode_plain(encoding_rs::UTF_16BE, bytes),
            Self::Legacy(encoding) => decode_plain(encoding, bytes),
            Self::Cp852 => bytes.iter().map(|byte| cp852_char(*byte)).collect(),
        }
    }
}

fn decode_plain(encoding: &'static Encoding, bytes: &[u8]) -> String {
    encoding
        .decode_without_bom_handling(bytes)
        .0
        .into_owned()
}

fn cp852_char(byte: u8) -> char {
    match byte.checked_sub(0x80) {
        None => char::from(byte),
        Some(high) => CP852_HIGH
            .get(usize::from(high))
            .and_then(|code| char::from_u32(u32::from(*code)))
            .unwrap_or(char::REPLACEMENT_CHARACTER),
    }
}

/// Encoding requested by `output_encoding`; `None` requests detection.
///
/// Accepted are `auto`, the labels of the WHATWG Encoding Standard (for
/// example `utf-8`, `utf-16le`, `cp1250`, `windows-1252`, `latin2`) and
/// `cp852`, `ibm852` or `852`.
///
/// # Errors
///
/// `E_INVALID_VALUE` for an unknown label.
pub(super) fn requested(label: Option<&str>) -> Result<Option<TextEncoding>> {
    let Some(label) = label.map(str::trim) else {
        return Ok(None);
    };
    if label.eq_ignore_ascii_case(AUTO) {
        return Ok(None);
    }
    if ["cp852", "ibm852", "852"]
        .iter()
        .any(|name| label.eq_ignore_ascii_case(name))
    {
        return Ok(Some(TextEncoding::Cp852));
    }
    match Encoding::for_label(label.as_bytes()) {
        Some(encoding) if encoding != encoding_rs::REPLACEMENT => {
            Ok(Some(from_encoding(encoding)))
        }
        _ => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("output_encoding: unknown encoding {label}"),
        )
        .with_found(label)
        .with_fix(r#"output_encoding: "auto""#)),
    }
}

fn from_encoding(encoding: &'static Encoding) -> TextEncoding {
    if encoding == encoding_rs::UTF_8 {
        TextEncoding::Utf8
    } else if encoding == encoding_rs::UTF_16LE {
        TextEncoding::Utf16Le
    } else if encoding == encoding_rs::UTF_16BE {
        TextEncoding::Utf16Be
    } else {
        TextEncoding::Legacy(encoding)
    }
}

/// Detects the encoding of a stream from the first bytes; `complete` tells
/// whether `sample` holds the whole stream.
pub(super) fn detect(sample: &[u8], complete: bool) -> TextEncoding {
    if let Some(encoding) = by_byte_order_mark(sample) {
        return encoding;
    }
    if let Some(encoding) = by_zero_bytes(sample) {
        return encoding;
    }
    if sample.contains(&0) || control_percent(sample) > BINARY_CONTROL_PERCENT {
        return TextEncoding::Binary;
    }
    if is_utf8(sample, complete) {
        return TextEncoding::Utf8;
    }
    legacy(sample, complete)
}

fn by_byte_order_mark(sample: &[u8]) -> Option<TextEncoding> {
    match sample {
        [0xef, 0xbb, 0xbf, ..] => Some(TextEncoding::Utf8),
        [0xff, 0xfe, ..] => Some(TextEncoding::Utf16Le),
        [0xfe, 0xff, ..] => Some(TextEncoding::Utf16Be),
        _ => None,
    }
}

fn by_zero_bytes(sample: &[u8]) -> Option<TextEncoding> {
    let pairs = sample.len() / 2;
    if pairs == 0 {
        return None;
    }
    let (mut even, mut odd) = (0, 0);
    for pair in sample.chunks_exact(2) {
        even += usize::from(pair[0] == 0);
        odd += usize::from(pair[1] == 0);
    }
    let dominant = |zeros: usize| zeros * 100 >= pairs * UTF16_ZERO_PERCENT;
    let rare = |zeros: usize| zeros * 100 <= pairs * UTF16_STRAY_ZERO_PERCENT;
    if dominant(odd) && rare(even) {
        Some(TextEncoding::Utf16Le)
    } else if dominant(even) && rare(odd) {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

/// Control bytes other than white space, backspace and escape, in percent.
fn control_percent(sample: &[u8]) -> usize {
    if sample.is_empty() {
        return 0;
    }
    let controls = sample
        .iter()
        .filter(|byte| {
            (**byte < 0x20 || **byte == 0x7f)
                && !matches!(**byte, b'\t' | b'\n' | b'\r' | 0x0b | 0x0c | 0x08 | 0x1b)
        })
        .count();
    controls * 100 / sample.len()
}

fn is_utf8(sample: &[u8], complete: bool) -> bool {
    match std::str::from_utf8(sample) {
        Ok(_) => true,
        Err(error) => !complete && error.error_len().is_none(),
    }
}

fn legacy(sample: &[u8], complete: bool) -> TextEncoding {
    if let Some(encoding) = console_encoding() {
        return encoding;
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(sample, complete);
    from_encoding(detector.guess(None, chardetng::Utf8Detection::Allow))
}

/// Encoding of the code page Windows gives to the output of console
/// programs started by this process.
///
/// A child attached to the console of this process writes in its output
/// code page. A child of a process without a console gets a new console
/// with the OEM code page of the system, so that code page applies when
/// this process has none. UTF-8 is excluded because detection only gets
/// here for output that is not valid UTF-8.
#[cfg(windows)]
fn console_encoding() -> Option<TextEncoding> {
    use windows_sys::Win32::Globalization::GetOEMCP;
    use windows_sys::Win32::System::Console::GetConsoleOutputCP;

    // SAFETY: GetConsoleOutputCP takes no arguments and reads the console
    // state of the calling process; it returns 0 when there is no console.
    let console = unsafe { GetConsoleOutputCP() };
    let page = if console == 0 {
        // SAFETY: GetOEMCP takes no arguments and reads a system setting.
        unsafe { GetOEMCP() }
    } else {
        console
    };
    code_page_encoding(page)
}

#[cfg(not(windows))]
fn console_encoding() -> Option<TextEncoding> {
    None
}

/// Decoder of a Windows code page, when one is available.
#[cfg(any(windows, test))]
fn code_page_encoding(page: u32) -> Option<TextEncoding> {
    let label = match page {
        852 => return Some(TextEncoding::Cp852),
        866 => "ibm866",
        874 => "windows-874",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        1250..=1258 => return windows_code_page(page),
        20866 => "koi8-r",
        21866 => "koi8-u",
        28592..=28606 => return iso_8859_code_page(page),
        54936 => "gb18030",
        _ => return None,
    };
    Encoding::for_label(label.as_bytes()).map(from_encoding)
}

#[cfg(any(windows, test))]
fn windows_code_page(page: u32) -> Option<TextEncoding> {
    Encoding::for_label(format!("windows-{page}").as_bytes()).map(from_encoding)
}

#[cfg(any(windows, test))]
fn iso_8859_code_page(page: u32) -> Option<TextEncoding> {
    const ISO_8859_BASE: u32 = 28590;
    let part = page - ISO_8859_BASE;
    Encoding::for_label(format!("iso-8859-{part}").as_bytes()).map(from_encoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLISH: &str = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}";

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

    #[test]
    fn detects_utf8_and_empty_output() {
        assert_eq!(detect(POLISH.as_bytes(), true), TextEncoding::Utf8);
        assert_eq!(detect(b"", true), TextEncoding::Utf8);
        assert_eq!(detect(b"\xef\xbb\xbfa", true), TextEncoding::Utf8);
    }

    #[test]
    fn accepts_sequence_cut_at_end_of_sample() {
        let bytes = "a\u{105}".as_bytes();
        let cut = &bytes[..bytes.len() - 1];
        assert_eq!(detect(cut, false), TextEncoding::Utf8);
        assert_ne!(detect(cut, true), TextEncoding::Utf8);
    }

    #[test]
    fn detects_utf16_by_byte_order_mark_and_zero_bytes() {
        let mut le = vec![0xff, 0xfe];
        le.extend(utf16("x", true));
        assert_eq!(detect(&le, true), TextEncoding::Utf16Le);
        assert_eq!(detect(&[0xfe, 0xff, 0, b'x'], true), TextEncoding::Utf16Be);
        assert_eq!(
            detect(&utf16("plain text\n", true), true),
            TextEncoding::Utf16Le
        );
        assert_eq!(
            detect(&utf16("plain text\n", false), true),
            TextEncoding::Utf16Be
        );
        assert_eq!(TextEncoding::Utf16Le.decode(&utf16(POLISH, true)), POLISH);
        assert_eq!(TextEncoding::Utf16Be.decode(&utf16(POLISH, false)), POLISH);
    }

    #[test]
    fn detects_binary_by_zero_and_control_bytes() {
        assert_eq!(detect(b"ab\0cd\0\0\0x", true), TextEncoding::Binary);
        let controls: Vec<u8> = (1u8..=6).cycle().take(64).collect();
        assert_eq!(detect(&controls, true), TextEncoding::Binary);
        assert_eq!(
            detect(b"\x1b[31mred\x1b[0m\tx\r\n", true),
            TextEncoding::Utf8
        );
    }

    #[test]
    fn decodes_cp852_and_cp1250() {
        let cp852 = [
            0x5a, 0x61, 0xbe, 0xa2, 0x88, 0x86, 0x20, 0x67, 0xa9, 0x98, 0x6c, 0xa5, 0x20, 0x6a,
            0x61, 0xab, 0xe4,
        ];
        assert_eq!(TextEncoding::Cp852.decode(&cp852), POLISH);
        let (cp1250, _, _) = encoding_rs::WINDOWS_1250.encode(POLISH);
        let requested = requested(Some("cp1250")).unwrap().unwrap();
        assert_eq!(requested.name(), "windows-1250");
        assert_eq!(requested.decode(&cp1250), POLISH);
    }

    #[test]
    fn guesses_legacy_encoding_for_invalid_utf8() {
        let text = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144} \
                    \u{15b}wi\u{119}ty \u{142}\u{105}ka \u{17c}\u{f3}\u{142}w";
        let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode(text);
        let detected = detect(&bytes, true);
        assert!(matches!(detected, TextEncoding::Legacy(_)), "{detected:?}");
        assert_eq!(detected.decode(&bytes), text);
    }

    #[test]
    fn reads_requested_labels() {
        assert_eq!(requested(None).unwrap(), None);
        assert_eq!(requested(Some("auto")).unwrap(), None);
        assert_eq!(requested(Some("AUTO")).unwrap(), None);
        assert_eq!(requested(Some("UTF-8")).unwrap(), Some(TextEncoding::Utf8));
        assert_eq!(
            requested(Some("utf-16be")).unwrap(),
            Some(TextEncoding::Utf16Be)
        );
        assert_eq!(requested(Some("ibm852")).unwrap(), Some(TextEncoding::Cp852));
        let error = requested(Some("klingon")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert!(requested(Some("iso-2022-kr")).is_err());
    }

    #[test]
    fn maps_windows_code_pages() {
        let name = |page| code_page_encoding(page).map(TextEncoding::name);
        assert_eq!(name(852).as_deref(), Some("ibm852"));
        assert_eq!(name(1250).as_deref(), Some("windows-1250"));
        assert_eq!(name(866).as_deref(), Some("ibm866"));
        assert_eq!(name(28592).as_deref(), Some("iso-8859-2"));
        assert_eq!(name(932).as_deref(), Some("shift_jis"));
        assert_eq!(name(437), None);
        assert_eq!(name(65001), None);
    }

    #[test]
    fn reports_line_units_and_names() {
        assert_eq!(TextEncoding::Utf16Le.unit(), LineUnit::Utf16Le);
        assert_eq!(TextEncoding::Utf16Be.unit(), LineUnit::Utf16Be);
        assert_eq!(TextEncoding::Cp852.unit(), LineUnit::Byte);
        assert_eq!(TextEncoding::Binary.name(), "binary");
        assert_eq!(TextEncoding::Utf8.decode(b"a\xffb"), "a\u{fffd}b");
    }
}
