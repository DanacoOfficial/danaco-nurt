// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Internationalised domain names: conversion between Unicode labels and
//! their ASCII form `xn--…` (RFC 5891, Punycode of RFC 3492).

/// Prefix of a label in ASCII compatible encoding.
const ACE_PREFIX: &str = "xn--";

const BASE: u32 = 36;
const T_MIN: u32 = 1;
const T_MAX: u32 = 26;
const SKEW: u32 = 38;
const DAMP: u32 = 700;
const INITIAL_BIAS: u32 = 72;
const INITIAL_N: u32 = 0x80;
const DIGIT_LETTERS: u32 = 26;

/// ASCII form of a domain name with the UTS #46 mapping; `None` when the
/// text is not a valid domain name.
///
/// An address is accepted only in its literal form: four decimal numbers
/// separated by periods, or an IPv6 address in brackets. The parser of URL
/// hosts reads other all-numeric names (`1`, `0x7f.1`) as IPv4 addresses and
/// would change them into one.
pub(crate) fn to_ascii(domain: &str) -> Option<String> {
    match url::Host::parse(domain).ok()? {
        url::Host::Domain(name) => Some(name),
        url::Host::Ipv4(address) => is_dotted_quad(domain).then(|| address.to_string()),
        url::Host::Ipv6(address) => {
            domain.starts_with('[').then(|| format!("[{address}]"))
        }
    }
}

fn is_dotted_quad(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Unicode form of a domain name: every label with the prefix `xn--` is
/// decoded; `None` when such a label is not valid Punycode.
pub(crate) fn to_unicode(domain: &str) -> Option<String> {
    let labels = domain
        .split('.')
        .map(|label| match ace_payload(label) {
            Some(encoded) => decode_label(encoded),
            None => Some(label.to_owned()),
        })
        .collect::<Option<Vec<_>>>()?;
    Some(labels.join("."))
}

fn ace_payload(label: &str) -> Option<&str> {
    let prefix = label.get(..ACE_PREFIX.len())?;
    prefix
        .eq_ignore_ascii_case(ACE_PREFIX)
        .then(|| label.get(ACE_PREFIX.len()..))
        .flatten()
}

/// Decodes one Punycode label (RFC 3492, section 6.2).
fn decode_label(input: &str) -> Option<String> {
    let (basic, extended) = match input.rfind('-') {
        Some(index) => (input.get(..index)?, input.get(index + 1..)?),
        None => ("", input),
    };
    if !basic.is_ascii() {
        return None;
    }
    let mut output: Vec<char> = basic.chars().collect();
    let mut code_point = INITIAL_N;
    let mut bias = INITIAL_BIAS;
    let mut index: u32 = 0;
    let mut digits = extended.bytes().peekable();
    while digits.peek().is_some() {
        let previous = index;
        let mut weight: u32 = 1;
        let mut k = BASE;
        loop {
            let digit = digit_value(digits.next()?)?;
            index = index.checked_add(digit.checked_mul(weight)?)?;
            let threshold = threshold(k, bias);
            if digit < threshold {
                break;
            }
            weight = weight.checked_mul(BASE - threshold)?;
            k = k.checked_add(BASE)?;
        }
        let length = u32::try_from(output.len()).ok()?.checked_add(1)?;
        bias = adapt(index - previous, length, previous == 0);
        code_point = code_point.checked_add(index / length)?;
        index %= length;
        let position = usize::try_from(index).ok()?;
        output.insert(position, char::from_u32(code_point)?);
        index += 1;
    }
    Some(output.into_iter().collect())
}

const fn threshold(k: u32, bias: u32) -> u32 {
    if k <= bias {
        T_MIN
    } else if k >= bias + T_MAX {
        T_MAX
    } else {
        k - bias
    }
}

fn digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'a'..=b'z' => Some(u32::from(byte - b'a')),
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'0'..=b'9' => Some(u32::from(byte - b'0') + DIGIT_LETTERS),
        _ => None,
    }
}

fn adapt(delta: u32, points: u32, first: bool) -> u32 {
    let mut delta = if first { delta / DAMP } else { delta / 2 };
    delta += delta / points;
    let mut k = 0;
    while delta > ((BASE - T_MIN) * T_MAX) / 2 {
        delta /= BASE - T_MIN;
        k += BASE;
    }
    k + (BASE - T_MIN + 1) * delta / (delta + SKEW)
}

#[cfg(test)]
mod tests {
    use super::{to_ascii, to_unicode};

    #[test]
    fn decodes_labels_of_rfc_3492_samples() {
        assert_eq!(to_unicode("xn--bcher-kva.de").as_deref(), Some("bücher.de"));
        assert_eq!(
            to_unicode("xn--mnchen-3ya.de").as_deref(),
            Some("münchen.de")
        );
        assert_eq!(
            to_unicode("xn--wgv71a119e.jp").as_deref(),
            Some("日本語.jp")
        );
        assert_eq!(to_unicode("example.com").as_deref(), Some("example.com"));
    }

    #[test]
    fn rejects_malformed_punycode() {
        assert_eq!(to_unicode("xn--bcher-kv!.de"), None);
        assert_eq!(to_unicode("xn--99999999999.de"), None);
    }

    #[test]
    fn encodes_with_uts_46_mapping() {
        assert_eq!(to_ascii("Bücher.de").as_deref(), Some("xn--bcher-kva.de"));
        assert_eq!(to_ascii("example.com").as_deref(), Some("example.com"));
        assert_eq!(to_ascii("bad host"), None);
        assert_eq!(to_ascii("1"), None);
        assert_eq!(to_ascii("0x7f.1"), None);
        assert_eq!(to_ascii("192.0.2.7").as_deref(), Some("192.0.2.7"));
        assert_eq!(to_ascii("[::1]").as_deref(), Some("[::1]"));
    }
}
