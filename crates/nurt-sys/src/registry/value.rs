// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::registry::value
// Responsibility:   Encoding and decoding of registry values.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Encoding and decoding of registry values.
//!
//! The registry stores a value as a type code and a byte string. Decoding is
//! tolerant: text without a terminating NUL is accepted, and data that does
//! not fit its declared type is reported as `other` with the raw bytes. The
//! raw form ([`RawValue`]) is kept unchanged wherever an exact copy matters,
//! so that preserved and restored values are identical byte for byte.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::ProviderValue;
use nurt_contracts::record::RegistryType;
use nurt_contracts::value::{Bytes, Value};

/// Type code `REG_NONE`.
pub const REG_NONE: u32 = 0;
/// Type code `REG_SZ`.
pub const REG_SZ: u32 = 1;
/// Type code `REG_EXPAND_SZ`.
pub const REG_EXPAND_SZ: u32 = 2;
/// Type code `REG_BINARY`.
pub const REG_BINARY: u32 = 3;
/// Type code `REG_DWORD` (little-endian).
pub const REG_DWORD: u32 = 4;
/// Type code `REG_DWORD_BIG_ENDIAN`.
pub const REG_DWORD_BIG_ENDIAN: u32 = 5;
/// Type code `REG_LINK`, the target of a symbolic link key.
pub const REG_LINK: u32 = 6;
/// Type code `REG_MULTI_SZ`.
pub const REG_MULTI_SZ: u32 = 7;
/// Type code `REG_QWORD` (little-endian).
pub const REG_QWORD: u32 = 11;

const DWORD_LEN: usize = 4;
const QWORD_LEN: usize = 8;
const UTF16_UNIT_LEN: usize = 2;
const NUL: u16 = 0;

/// Value as stored by the registry: type code and bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawValue {
    /// Type code (`REG_SZ`, `REG_DWORD`, …).
    pub raw_type: u32,
    /// Data exactly as stored.
    pub bytes: Vec<u8>,
}

impl RawValue {
    /// Value of type `raw_type` with `bytes` as data.
    pub fn new(raw_type: u32, bytes: Vec<u8>) -> Self {
        Self { raw_type, bytes }
    }

    /// `REG_SZ` value with a terminating NUL.
    pub fn string(text: &str) -> Self {
        Self::new(REG_SZ, encode_utf16z(text))
    }

    /// `REG_DWORD` value.
    pub fn dword(number: u32) -> Self {
        Self::new(REG_DWORD, number.to_le_bytes().to_vec())
    }

    /// `REG_QWORD` value.
    pub fn qword(number: u64) -> Self {
        Self::new(REG_QWORD, number.to_le_bytes().to_vec())
    }

    /// `REG_MULTI_SZ` value: every text followed by a NUL, and a final NUL.
    pub fn multi_string<S: AsRef<str>>(texts: &[S]) -> Self {
        let mut units = Vec::new();
        for text in texts {
            units.extend(text.as_ref().encode_utf16());
            units.push(NUL);
        }
        units.push(NUL);
        Self::new(REG_MULTI_SZ, units_to_bytes(&units))
    }

    /// Size of the data in bytes.
    pub fn size(&self) -> u64 {
        u64::try_from(self.bytes.len()).unwrap_or(u64::MAX)
    }

    /// Type of the value as shown in `registry_value` records.
    ///
    /// Data whose length does not match a numeric type is reported as
    /// `other`.
    pub fn registry_type(&self) -> RegistryType {
        match self.decode() {
            Decoded::Text(_) => RegistryType::String,
            Decoded::ExpandableText(_) => RegistryType::ExpandString,
            Decoded::TextList(_) => RegistryType::MultiString,
            Decoded::U32(_) => RegistryType::Dword,
            Decoded::U64(_) => RegistryType::Qword,
            Decoded::Binary => RegistryType::Binary,
            Decoded::None => RegistryType::None,
            Decoded::Other => RegistryType::Other,
        }
    }

    /// Typed value of the provider interface.
    ///
    /// Types without a typed form, and data that does not fit its type, are
    /// returned as [`ProviderValue::Binary`] with the type code.
    pub fn to_provider_value(&self) -> ProviderValue {
        match self.decode() {
            Decoded::Text(text) => ProviderValue::Text(text),
            Decoded::ExpandableText(text) => ProviderValue::ExpandableText(text),
            Decoded::TextList(texts) => ProviderValue::TextList(texts),
            Decoded::U32(number) => ProviderValue::U32(number),
            Decoded::U64(number) => ProviderValue::U64(number),
            Decoded::Binary | Decoded::None | Decoded::Other => self.to_exact_provider_value(),
        }
    }

    /// Provider value that keeps type code and bytes unchanged, used for
    /// preservation.
    pub fn to_exact_provider_value(&self) -> ProviderValue {
        ProviderValue::Binary {
            raw_type: self.raw_type,
            bytes: self.bytes.clone(),
        }
    }

    /// Raw form of a provider value. Text gets a terminating NUL;
    /// [`ProviderValue::Binary`] is taken unchanged.
    pub fn from_provider_value(value: &ProviderValue) -> Self {
        match value {
            ProviderValue::Text(text) => Self::string(text),
            ProviderValue::ExpandableText(text) => Self::new(REG_EXPAND_SZ, encode_utf16z(text)),
            ProviderValue::TextList(texts) => Self::multi_string(texts),
            ProviderValue::U32(number) => Self::dword(*number),
            ProviderValue::U64(number) => Self::qword(*number),
            ProviderValue::Binary { raw_type, bytes } => Self::new(*raw_type, bytes.clone()),
            _ => Self::new(REG_NONE, Vec::new()),
        }
    }

    /// Value of the language: text, integer, list of texts or bytes.
    ///
    /// A `REG_QWORD` above the range of `int` is returned as a big integer.
    pub fn to_language_value(&self) -> Value {
        match self.decode() {
            Decoded::Text(text) | Decoded::ExpandableText(text) => Value::Text(text),
            Decoded::TextList(texts) => Value::List(texts.into_iter().map(Value::Text).collect()),
            Decoded::U32(number) => Value::Int(i64::from(number)),
            Decoded::U64(number) => {
                i64::try_from(number).map_or_else(|_| Value::BigInt(number.to_string()), Value::Int)
            }
            Decoded::Binary | Decoded::None | Decoded::Other => {
                Value::Bytes(Bytes::new(self.bytes.clone()))
            }
        }
    }

    fn decode(&self) -> Decoded {
        match self.raw_type {
            REG_SZ => Decoded::Text(decode_string(&self.bytes)),
            REG_EXPAND_SZ => Decoded::ExpandableText(decode_string(&self.bytes)),
            REG_MULTI_SZ => Decoded::TextList(decode_multi_string(&self.bytes)),
            REG_DWORD => <[u8; DWORD_LEN]>::try_from(self.bytes.as_slice())
                .map_or(Decoded::Other, |bytes| {
                    Decoded::U32(u32::from_le_bytes(bytes))
                }),
            REG_QWORD => <[u8; QWORD_LEN]>::try_from(self.bytes.as_slice())
                .map_or(Decoded::Other, |bytes| {
                    Decoded::U64(u64::from_le_bytes(bytes))
                }),
            REG_BINARY => Decoded::Binary,
            REG_NONE => Decoded::None,
            _ => Decoded::Other,
        }
    }
}

enum Decoded {
    Text(String),
    ExpandableText(String),
    TextList(Vec<String>),
    U32(u32),
    U64(u64),
    Binary,
    None,
    Other,
}

/// Decodes `REG_SZ` or `REG_EXPAND_SZ` data: UTF-16LE up to the first NUL.
///
/// A missing terminator and an odd final byte are tolerated; references in
/// `REG_EXPAND_SZ` data are not expanded.
pub fn decode_string(bytes: &[u8]) -> String {
    let units = bytes_to_units(bytes);
    let end = units
        .iter()
        .position(|unit| *unit == NUL)
        .unwrap_or(units.len());
    String::from_utf16_lossy(units.get(..end).unwrap_or_default())
}

/// Decodes `REG_MULTI_SZ` data: NUL-separated texts ended by an empty text.
///
/// A missing final terminator is tolerated; the list ends at the first empty
/// text, as in the registry editor.
pub fn decode_multi_string(bytes: &[u8]) -> Vec<String> {
    bytes_to_units(bytes)
        .split(|unit| *unit == NUL)
        .take_while(|text| !text.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// Encodes a language value as a value of type `kind` for `write … value_type:`.
///
/// Accepted values: text for `string` and `expand_string`, a non-negative
/// integer for `dword` and `qword`, a list of texts for `multi_string` and
/// bytes for `binary`.
///
/// ```
/// use nurt_contracts::record::RegistryType;
/// use nurt_contracts::value::Value;
/// use nurt_sys::registry::value::{REG_DWORD, encode};
///
/// let raw = encode(&Value::Int(1), RegistryType::Dword)?;
/// assert_eq!(raw.raw_type, REG_DWORD);
/// assert_eq!(raw.bytes, [1, 0, 0, 0]);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
///
/// # Errors
///
/// Fails with `E_TYPE` when the value has the wrong type, `E_INVALID_VALUE` when a
/// number is out of range or a text contains a NUL character, and
/// `E_INVALID_VALUE` for the types `none` and `other`, which cannot be
/// written.
pub fn encode(value: &Value, kind: RegistryType) -> Result<RawValue, Error> {
    match kind {
        RegistryType::String => Ok(RawValue::string(text_of(value, kind)?)),
        RegistryType::ExpandString => Ok(RawValue::new(
            REG_EXPAND_SZ,
            encode_utf16z(text_of(value, kind)?),
        )),
        RegistryType::MultiString => encode_multi_string(value),
        RegistryType::Dword => {
            let number = unsigned_of(value, kind)?;
            u32::try_from(number)
                .map(RawValue::dword)
                .map_err(|_| out_of_range(kind, u64::from(u32::MAX)))
        }
        RegistryType::Qword => unsigned_of(value, kind).map(RawValue::qword),
        RegistryType::Binary => match value {
            Value::Bytes(bytes) => Ok(RawValue::new(REG_BINARY, bytes.as_slice().to_vec())),
            other => Err(type_error(kind, "bytes", other)),
        },
        _ => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("values of type `{kind}` cannot be written"),
        )
        .with_expected(serde_json::json!(WRITABLE_TYPES))),
    }
}

/// Types accepted by `write … value_type:`.
pub const WRITABLE_TYPES: [&str; 6] = [
    "string",
    "dword",
    "qword",
    "multi_string",
    "expand_string",
    "binary",
];

/// Type chosen by `write` without `value_type:`: the type of the existing
/// value, otherwise the type that matches the language value.
///
/// ```
/// use nurt_contracts::record::RegistryType;
/// use nurt_contracts::value::Value;
/// use nurt_sys::registry::value::infer_type;
///
/// assert_eq!(infer_type(None, &Value::Int(7)), Some(RegistryType::Dword));
/// assert_eq!(
///     infer_type(Some(RegistryType::ExpandString), &Value::Text("%TEMP%".into())),
///     Some(RegistryType::ExpandString)
/// );
/// ```
pub fn infer_type(existing: Option<RegistryType>, value: &Value) -> Option<RegistryType> {
    let writable =
        existing.filter(|kind| !matches!(kind, RegistryType::None | RegistryType::Other));
    if writable.is_some() {
        return writable;
    }
    match value {
        Value::Text(_) => Some(RegistryType::String),
        Value::Int(number) if u32::try_from(*number).is_ok() => Some(RegistryType::Dword),
        Value::Int(_) | Value::BigInt(_) => Some(RegistryType::Qword),
        Value::List(_) => Some(RegistryType::MultiString),
        Value::Bytes(_) => Some(RegistryType::Binary),
        _ => None,
    }
}

fn text_of(value: &Value, kind: RegistryType) -> Result<&str, Error> {
    match value {
        Value::Text(text) => {
            if text.contains('\0') {
                Err(nul_error(kind))
            } else {
                Ok(text)
            }
        }
        other => Err(type_error(kind, "text", other)),
    }
}

fn unsigned_of(value: &Value, kind: RegistryType) -> Result<u64, Error> {
    let limit = if kind == RegistryType::Dword {
        u64::from(u32::MAX)
    } else {
        u64::MAX
    };
    match value {
        Value::Int(number) => u64::try_from(*number).map_err(|_| out_of_range(kind, limit)),
        Value::BigInt(digits) => digits.parse::<u64>().map_err(|_| out_of_range(kind, limit)),
        other => Err(type_error(kind, "int", other)),
    }
}

fn encode_multi_string(value: &Value) -> Result<RawValue, Error> {
    let kind = RegistryType::MultiString;
    let Value::List(items) = value else {
        return Err(type_error(kind, "list<text>", value));
    };
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        let text = text_of(item, kind).map_err(|_| type_error(kind, "list<text>", item))?;
        if text.is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "a multi_string value cannot contain an empty text; remove the empty item",
            ));
        }
        texts.push(text);
    }
    Ok(RawValue::multi_string(&texts))
}

fn type_error(kind: RegistryType, expected: &str, found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "a `{kind}` value requires {expected}, found {}",
            found.type_name()
        ),
    )
    .with_found(found.type_name())
    .with_expected(serde_json::json!([expected]))
}

fn out_of_range(kind: RegistryType, limit: u64) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("a `{kind}` value must be between 0 and {limit}"),
    )
    .with_expected(serde_json::json!({ "min": 0, "max": limit }))
}

fn nul_error(kind: RegistryType) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("a `{kind}` value cannot contain a NUL character; remove it"),
    )
}

fn encode_utf16z(text: &str) -> Vec<u8> {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(NUL);
    units_to_bytes(&units)
}

fn units_to_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

fn bytes_to_units(bytes: &[u8]) -> Vec<u16> {
    let (pairs, _) = bytes.as_chunks::<UTF16_UNIT_LEN>();
    pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        units_to_bytes(&text.encode_utf16().collect::<Vec<_>>())
    }

    #[test]
    fn registry_value_decode_sz_without_terminator() {
        let without = RawValue::new(REG_SZ, utf16("code"));
        assert_eq!(
            without.to_provider_value(),
            ProviderValue::Text("code".into())
        );
        assert_eq!(without.registry_type(), RegistryType::String);
        let mut odd = utf16("ab");
        odd.push(0x41);
        assert_eq!(decode_string(&odd), "ab");
        let terminated = RawValue::string("code");
        assert_eq!(terminated.bytes.len(), without.bytes.len() + 2);
        assert_eq!(terminated.to_language_value(), Value::Text("code".into()));
        let expand = RawValue::new(REG_EXPAND_SZ, utf16("%TEMP%\\x"));
        assert_eq!(
            expand.to_provider_value(),
            ProviderValue::ExpandableText("%TEMP%\\x".into())
        );
        assert_eq!(expand.registry_type(), RegistryType::ExpandString);
        let exact = without.to_exact_provider_value();
        assert_eq!(RawValue::from_provider_value(&exact), without);
    }

    #[test]
    fn registry_value_decode_multi_sz_double_nul() {
        let raw = RawValue::multi_string(&["a", "bc"]);
        assert_eq!(raw.bytes, utf16("a\0bc\0\0"));
        assert_eq!(
            raw.to_provider_value(),
            ProviderValue::TextList(vec!["a".into(), "bc".into()])
        );
        assert_eq!(decode_multi_string(&utf16("a\0bc")), ["a", "bc"]);
        assert_eq!(decode_multi_string(&utf16("a\0\0ignored\0\0")), ["a"]);
        assert!(decode_multi_string(&utf16("\0")).is_empty());
        assert_eq!(
            raw.to_language_value(),
            Value::List(vec![Value::Text("a".into()), Value::Text("bc".into())])
        );
        assert_eq!(raw.registry_type(), RegistryType::MultiString);
    }

    #[test]
    fn registry_value_encode_dword_qword_range_errors() {
        let dword = encode(&Value::Int(i64::from(u32::MAX)), RegistryType::Dword).unwrap();
        assert_eq!(dword, RawValue::dword(u32::MAX));
        let too_big = encode(&Value::Int(i64::from(u32::MAX) + 1), RegistryType::Dword);
        assert_eq!(too_big.unwrap_err().code(), ErrorCode::InvalidValue);
        let negative = encode(&Value::Int(-1), RegistryType::Qword);
        assert_eq!(negative.unwrap_err().code(), ErrorCode::InvalidValue);
        let max = encode(&Value::BigInt(u64::MAX.to_string()), RegistryType::Qword).unwrap();
        assert_eq!(max, RawValue::qword(u64::MAX));
        assert_eq!(
            max.to_language_value(),
            Value::BigInt("18446744073709551615".into())
        );
        let over = encode(
            &Value::BigInt("18446744073709551616".into()),
            RegistryType::Qword,
        );
        assert_eq!(over.unwrap_err().code(), ErrorCode::InvalidValue);
        let text = encode(&Value::Text("1".into()), RegistryType::Dword);
        assert_eq!(text.unwrap_err().code(), ErrorCode::Type);
        assert_eq!(dword.to_language_value(), Value::Int(i64::from(u32::MAX)));
    }

    #[test]
    fn encode_checks_text_list_and_bytes() {
        let list = Value::List(vec![Value::Text("a".into()), Value::Text("b".into())]);
        assert_eq!(
            encode(&list, RegistryType::MultiString).unwrap(),
            RawValue::multi_string(&["a", "b"])
        );
        let mixed = Value::List(vec![Value::Text("a".into()), Value::Int(1)]);
        assert_eq!(
            encode(&mixed, RegistryType::MultiString)
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
        let empty_item = Value::List(vec![Value::Text(String::new())]);
        assert_eq!(
            encode(&empty_item, RegistryType::MultiString)
                .unwrap_err()
                .code(),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            encode(&Value::Text("x".into()), RegistryType::MultiString)
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
        let nul = encode(&Value::Text("a\0b".into()), RegistryType::String);
        assert_eq!(nul.unwrap_err().code(), ErrorCode::InvalidValue);
        let bytes = encode(&Value::Bytes(Bytes::new(vec![1, 2])), RegistryType::Binary).unwrap();
        assert_eq!(bytes, RawValue::new(REG_BINARY, vec![1, 2]));
        assert_eq!(
            encode(&Value::Int(1), RegistryType::Binary)
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
        assert_eq!(
            encode(&Value::Int(1), RegistryType::None)
                .unwrap_err()
                .code(),
            ErrorCode::InvalidValue
        );
        let expand = encode(&Value::Text("%PATH%".into()), RegistryType::ExpandString).unwrap();
        assert_eq!(expand.raw_type, REG_EXPAND_SZ);
    }

    #[test]
    fn other_types_keep_raw_bytes() {
        let big_endian = RawValue::new(REG_DWORD_BIG_ENDIAN, vec![0, 0, 0, 1]);
        assert_eq!(big_endian.registry_type(), RegistryType::Other);
        assert_eq!(
            big_endian.to_provider_value(),
            ProviderValue::Binary {
                raw_type: REG_DWORD_BIG_ENDIAN,
                bytes: vec![0, 0, 0, 1]
            }
        );
        let short_dword = RawValue::new(REG_DWORD, vec![1, 0]);
        assert_eq!(short_dword.registry_type(), RegistryType::Other);
        assert_eq!(
            short_dword.to_language_value(),
            Value::Bytes(Bytes::new(vec![1, 0]))
        );
        assert_eq!(
            RawValue::new(REG_QWORD, vec![1]).registry_type(),
            RegistryType::Other
        );
        assert_eq!(
            RawValue::new(REG_NONE, vec![]).registry_type(),
            RegistryType::None
        );
        assert_eq!(
            RawValue::new(REG_BINARY, vec![]).registry_type(),
            RegistryType::Binary
        );
        assert_eq!(RawValue::new(REG_BINARY, vec![9]).size(), 1);
    }

    #[test]
    fn provider_values_round_trip() {
        let values = [
            ProviderValue::Text("a".into()),
            ProviderValue::ExpandableText("%A%".into()),
            ProviderValue::TextList(vec!["x".into(), "y".into()]),
            ProviderValue::U32(7),
            ProviderValue::U64(u64::MAX),
            ProviderValue::Binary {
                raw_type: REG_BINARY,
                bytes: vec![1, 2, 3],
            },
        ];
        for value in values {
            let raw = RawValue::from_provider_value(&value);
            assert_eq!(raw.to_provider_value(), value);
        }
    }

    #[test]
    fn infer_type_prefers_existing_type() {
        assert_eq!(
            infer_type(Some(RegistryType::Qword), &Value::Int(1)),
            Some(RegistryType::Qword)
        );
        assert_eq!(
            infer_type(Some(RegistryType::Other), &Value::Text("x".into())),
            Some(RegistryType::String)
        );
        assert_eq!(
            infer_type(None, &Value::Int(i64::from(u32::MAX) + 1)),
            Some(RegistryType::Qword)
        );
        assert_eq!(
            infer_type(None, &Value::List(vec![])),
            Some(RegistryType::MultiString)
        );
        assert_eq!(
            infer_type(None, &Value::Bytes(Bytes::new(vec![]))),
            Some(RegistryType::Binary)
        );
        assert_eq!(infer_type(None, &Value::Bool(true)), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
