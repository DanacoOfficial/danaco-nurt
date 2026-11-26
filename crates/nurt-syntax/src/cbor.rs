// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Deterministic CBOR encoding of syntax trees (RFC 8949, section 4.2).
//!
//! [`encode`] writes the deterministic encoding of RFC 8949, section 4.2.1:
//! definite lengths only, the shortest form of every integer and length, map
//! keys sorted by the bytewise lexicographic order of their encodings, every
//! floating-point number in the shortest of the 16-, 32- and 64-bit forms that
//! preserves its value, negative zero written as zero and no tags. A node of
//! the kind `num` stays a floating-point number even when its value is
//! integral, so decoding restores the node kind.
//!
//! The decoders accept well-formed CBOR of the JSON data model: maps with text
//! keys, arrays, text, integers within ±(2^53 - 1), finite floating-point
//! numbers, `true`, `false` and `null`. Tags, byte strings, `undefined`, other
//! simple values, keys other than text, duplicate keys, nesting deeper than
//! [`MAX_DEPTH`] and data after the document are rejected with `E_AST_SCHEMA`
//! and the offset of the offending byte. The decoded value then passes the
//! same checks as a JSON document ([`crate::json`]): format version,
//! structure and invariants. Length headers are compared with the remaining
//! input before any allocation, so no header can force a large allocation.

// `SyntaxError` is the error type shared by every module of the crate; its
// layout is defined with the parser diagnostics, not per module.
#![allow(clippy::result_large_err)]

use ciborium::value::Value as CborValue;
use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use serde::Serialize;
use serde_json::{Map, Number, Value};

use crate::error::SyntaxError;
use crate::json;
pub use crate::validate::MAX_DEPTH;
use crate::validate::{MAX_SAFE_INTEGER, violation};

/// Encodes a value as deterministic CBOR (RFC 8949, section 4.2.1).
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` when the value is outside the JSON data model of a
/// tree document: a non-finite number, an integer beyond ±(2^53 - 1), a byte
/// string, a tag, a key other than text, a duplicate key or nesting deeper
/// than [`MAX_DEPTH`].
///
/// # Examples
///
/// ```
/// use nurt_syntax::cbor::encode;
///
/// let value = serde_json::json!({"t": "num", "v": 3.0});
/// // Keys sorted by their encoding, 3.0 as a 16-bit float.
/// assert_eq!(
///     encode(&value).ok(),
///     Some(vec![0xa2, 0x61, b't', 0x63, b'n', b'u', b'm', 0x61, b'v', 0xf9, 0x42, 0x00]),
/// );
/// ```
pub fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, SyntaxError> {
    let value = CborValue::serialized(value)
        .map_err(|error| violation("", format!("the value cannot be encoded: {error}")))?;
    let value = normalize(value, "", 1)?;
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&value, &mut bytes)
        .map_err(|error| violation("", format!("the value cannot be encoded: {error}")))?;
    Ok(bytes)
}

/// Decodes a surface tree from CBOR.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` for input that is not well-formed CBOR of the JSON
/// data model or not a valid surface tree, and `E_AST_VERSION` for a document
/// of an unsupported format version.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::AstDocument;
/// use nurt_syntax::cbor::{decode_surface, encode};
///
/// let document: AstDocument = serde_json::from_str(
///     r#"{"nurt":"1.0","program":{"statements":[{"t":"pipeline",
///         "head":{"t":"stage","name":"count","args":[]},"stages":[]}]}}"#,
/// )?;
/// let bytes = encode(&document);
/// assert_eq!(bytes.and_then(|bytes| decode_surface(&bytes)).ok(), Some(document));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn decode_surface(bytes: &[u8]) -> Result<AstDocument, SyntaxError> {
    json::decode_surface_value(read(bytes)?)
}

/// Decodes a bound tree from CBOR.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` for input that is not well-formed CBOR of the JSON
/// data model or not a valid bound tree, and `E_AST_VERSION` for a document
/// of an unsupported format version.
pub fn decode_bound(bytes: &[u8]) -> Result<BoundDocument, SyntaxError> {
    json::decode_bound_value(read(bytes)?)
}

/// Decodes a surface tree from CBOR and requires the input to be exactly the
/// deterministic encoding of the decoded tree, as written by [`encode`].
///
/// # Errors
///
/// Returns the errors of [`decode_surface`], and `E_AST_SCHEMA` with the
/// offset of the first differing byte when the input is not the deterministic
/// encoding of the tree.
pub fn decode_surface_strict(bytes: &[u8]) -> Result<AstDocument, SyntaxError> {
    let document = decode_surface(bytes)?;
    ensure_deterministic(bytes, &encode(&document)?)?;
    Ok(document)
}

/// Decodes a bound tree from CBOR and requires the input to be exactly the
/// deterministic encoding of the decoded tree, as written by [`encode`].
///
/// # Errors
///
/// Returns the errors of [`decode_bound`], and `E_AST_SCHEMA` with the offset
/// of the first differing byte when the input is not the deterministic
/// encoding of the tree.
pub fn decode_bound_strict(bytes: &[u8]) -> Result<BoundDocument, SyntaxError> {
    let document = decode_bound(bytes)?;
    ensure_deterministic(bytes, &encode(&document)?)?;
    Ok(document)
}

/// Decodes well-formed CBOR of the JSON data model into a JSON value without
/// interpreting it as a tree.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` with the offset of the offending byte for the input
/// rejected as described in the module documentation.
///
/// # Examples
///
/// ```
/// use nurt_syntax::cbor::read;
///
/// assert_eq!(read(&[0x82, 0x01, 0xf5]).ok(), Some(serde_json::json!([1, true])));
/// assert!(read(&[0xc0, 0x01]).is_err());
/// ```
pub fn read(bytes: &[u8]) -> Result<Value, SyntaxError> {
    Reader::new(bytes).document()
}

fn ensure_deterministic(received: &[u8], deterministic: &[u8]) -> Result<(), SyntaxError> {
    match received
        .iter()
        .zip(deterministic)
        .position(|(left, right)| left != right)
    {
        Some(offset) => Err(at_byte(offset, NOT_DETERMINISTIC)),
        None if received.len() == deterministic.len() => Ok(()),
        None => Err(at_byte(
            received.len().min(deterministic.len()),
            NOT_DETERMINISTIC,
        )),
    }
}

const NOT_DETERMINISTIC: &str = "the input is not the deterministic encoding of the tree; re-encode it with the deterministic encoding";

/// Most elements reserved at once for a container; larger ones grow as their
/// elements are read.
const MAX_RESERVED_ELEMENTS: usize = 1024;
const DEPTH_MESSAGE: &str = "nesting deeper than 128 levels; flatten the document";

/// Returns a decoding error; [`SyntaxError::found`] holds the offset of the
/// offending byte, written `byte <offset>`.
fn at_byte(offset: usize, message: impl Into<String>) -> SyntaxError {
    SyntaxError::ast_schema(message).with_found(format!("byte {offset}"))
}

fn normalize(value: CborValue, pointer: &str, level: usize) -> Result<CborValue, SyntaxError> {
    if level > MAX_DEPTH {
        return Err(violation(pointer, DEPTH_MESSAGE));
    }
    match value {
        CborValue::Integer(integer) => {
            let magnitude = i128::from(integer).unsigned_abs();
            if magnitude > MAX_SAFE_INTEGER.unsigned_abs().into() {
                return Err(violation(pointer, INTEGER_MESSAGE));
            }
            Ok(CborValue::Integer(integer))
        }
        CborValue::Float(number) if !number.is_finite() => Err(violation(pointer, FLOAT_MESSAGE)),
        // The pattern also matches negative zero, which is written as zero.
        CborValue::Float(0.0) => Ok(CborValue::Float(0.0)),
        CborValue::Float(_) | CborValue::Text(_) | CborValue::Bool(_) | CborValue::Null => {
            Ok(value)
        }
        CborValue::Array(items) => items
            .into_iter()
            .enumerate()
            .map(|(index, item)| normalize(item, &format!("{pointer}/{index}"), level + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(CborValue::Array),
        CborValue::Map(entries) => normalize_map(entries, pointer, level),
        CborValue::Bytes(_) => Err(violation(pointer, BYTES_MESSAGE)),
        CborValue::Tag(..) => Err(violation(pointer, TAG_MESSAGE)),
        _ => Err(violation(
            pointer,
            "the value is outside the JSON data model",
        )),
    }
}

fn normalize_map(
    entries: Vec<(CborValue, CborValue)>,
    pointer: &str,
    level: usize,
) -> Result<CborValue, SyntaxError> {
    let mut keyed = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let CborValue::Text(name) = &key else {
            return Err(violation(pointer, KEY_MESSAGE));
        };
        let member = format!("{pointer}/{}", name.replace('~', "~0").replace('/', "~1"));
        let value = normalize(value, &member, level + 1)?;
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&key, &mut encoded)
            .map_err(|error| violation(&member, format!("the key cannot be encoded: {error}")))?;
        keyed.push((encoded, key, value));
    }
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    if let Some(pair) = keyed.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        let name = pair[0].1.as_text().unwrap_or_default();
        return Err(violation(
            pointer,
            format!("duplicate map key `{name}`; keep a single member"),
        ));
    }
    Ok(CborValue::Map(
        keyed
            .into_iter()
            .map(|(_, key, value)| (key, value))
            .collect(),
    ))
}

const INTEGER_MESSAGE: &str = "the integer must lie within -(2^53 - 1)..=2^53 - 1";

const FLOAT_MESSAGE: &str = "the number must be finite; NaN and infinities are not admissible";

const BYTES_MESSAGE: &str = "byte strings are not admissible; encode the value as text";

const TAG_MESSAGE: &str = "tags are not admissible; remove the tag";

const KEY_MESSAGE: &str = "map keys must be text";

/// Major types of RFC 8949, section 3.1.
mod major {
    pub(super) const UNSIGNED: u8 = 0;
    pub(super) const NEGATIVE: u8 = 1;
    pub(super) const BYTES: u8 = 2;
    pub(super) const TEXT: u8 = 3;
    pub(super) const ARRAY: u8 = 4;
    pub(super) const MAP: u8 = 5;
    pub(super) const TAG: u8 = 6;
    pub(super) const SIMPLE: u8 = 7;
}

/// Additional information values of RFC 8949, sections 3 and 3.3.
mod info {
    pub(super) const ONE_BYTE: u8 = 24;
    pub(super) const TWO_BYTES: u8 = 25;
    pub(super) const FOUR_BYTES: u8 = 26;
    pub(super) const EIGHT_BYTES: u8 = 27;
    pub(super) const INDEFINITE: u8 = 31;
    pub(super) const FALSE: u8 = 20;
    pub(super) const TRUE: u8 = 21;
    pub(super) const NULL: u8 = 22;
    pub(super) const UNDEFINED: u8 = 23;
}

/// Number of bits of the major type in an initial byte.
const MAJOR_SHIFT: u8 = 5;

/// Mask of the additional information in an initial byte.
const INFO_MASK: u8 = 0x1f;

/// Smallest encoded size of an array element.
const MIN_ELEMENT_SIZE: u64 = 1;

/// Smallest encoded size of a map entry: a key and a value.
const MIN_ENTRY_SIZE: u64 = 2;

/// Layout of an IEEE 754 binary16 number.
mod half {
    pub(super) const SIGN: u16 = 0x8000;
    pub(super) const EXPONENT_SHIFT: u16 = 10;
    pub(super) const EXPONENT_MASK: u16 = 0x1f;
    pub(super) const FRACTION_MASK: u16 = 0x03ff;
    pub(super) const EXPONENT_BIAS: i32 = 15;
    pub(super) const FRACTION_SCALE: f64 = 1024.0;
    pub(super) const SUBNORMAL_EXPONENT: i32 = -24;
}

fn half_to_f64(bits: u16) -> f64 {
    let exponent = (bits >> half::EXPONENT_SHIFT) & half::EXPONENT_MASK;
    let fraction = f64::from(bits & half::FRACTION_MASK);
    let magnitude = match exponent {
        0 => fraction * 2f64.powi(half::SUBNORMAL_EXPONENT),
        half::EXPONENT_MASK if fraction == 0.0 => f64::INFINITY,
        half::EXPONENT_MASK => f64::NAN,
        _ => {
            (1.0 + fraction / half::FRACTION_SCALE)
                * 2f64.powi(i32::from(exponent) - half::EXPONENT_BIAS)
        }
    };
    if bits & half::SIGN == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// Initial byte and argument of a data item.
#[derive(Debug, Clone, Copy)]
struct Head {
    offset: usize,
    major: u8,
    info: u8,
    /// Argument; `None` for an indefinite length.
    argument: Option<u64>,
}

/// Container being decoded.
enum Frame {
    Array {
        items: Vec<Value>,
        remaining: Option<u64>,
    },
    Map {
        members: Map<String, Value>,
        remaining: Option<u64>,
        key: Option<(String, usize)>,
    },
}

impl Frame {
    fn is_complete(&self) -> bool {
        match self {
            Self::Array { remaining, .. } | Self::Map { remaining, .. } => *remaining == Some(0),
        }
    }

    fn expects_key(&self) -> bool {
        matches!(self, Self::Map { key: None, .. })
    }

    fn into_value(self) -> Value {
        match self {
            Self::Array { items, .. } => Value::Array(items),
            Self::Map { members, .. } => Value::Object(members),
        }
    }
}

struct Reader<'a> {
    input: &'a [u8],
    offset: usize,
    stack: Vec<Frame>,
}

impl<'a> Reader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            offset: 0,
            stack: Vec::new(),
        }
    }

    fn document(mut self) -> Result<Value, SyntaxError> {
        let root = self.value()?;
        if self.offset < self.input.len() {
            return Err(at_byte(
                self.offset,
                "unexpected data after the document; remove the trailing bytes",
            ));
        }
        Ok(root)
    }

    fn remaining(&self) -> u64 {
        u64::try_from(self.input.len() - self.offset).unwrap_or(u64::MAX)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], SyntaxError> {
        let end = self
            .offset
            .checked_add(count)
            .filter(|end| *end <= self.input.len())
            .ok_or_else(|| at_byte(self.offset, TRUNCATED))?;
        let bytes = &self.input[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    fn head(&mut self) -> Result<Head, SyntaxError> {
        let offset = self.offset;
        let initial = *self
            .input
            .get(offset)
            .ok_or_else(|| at_byte(offset, TRUNCATED))?;
        self.offset += 1;
        let major = initial >> MAJOR_SHIFT;
        let info = initial & INFO_MASK;
        let argument = match info {
            0..info::ONE_BYTE => Some(u64::from(info)),
            info::ONE_BYTE => Some(u64::from(self.take(1)?[0])),
            info::TWO_BYTES => Some(u64::from(u16::from_be_bytes(self.array()?))),
            info::FOUR_BYTES => Some(u64::from(u32::from_be_bytes(self.array()?))),
            info::EIGHT_BYTES => Some(u64::from_be_bytes(self.array()?)),
            info::INDEFINITE => None,
            _ => {
                return Err(at_byte(
                    offset,
                    format!(
                        "reserved additional information {info}; the input is not well-formed CBOR"
                    ),
                ));
            }
        };
        Ok(Head {
            offset,
            major,
            info,
            argument,
        })
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SyntaxError> {
        let offset = self.offset;
        self.take(N)?
            .try_into()
            .map_err(|_| at_byte(offset, TRUNCATED))
    }

    /// Decodes one data item; containers are decoded with an explicit stack,
    /// so the depth of the input never deepens the call stack.
    fn value(&mut self) -> Result<Value, SyntaxError> {
        loop {
            if self.stack.last().is_some_and(Frame::is_complete) {
                if let Some(value) = self.close()? {
                    return Ok(value);
                }
                continue;
            }
            let head = self.head()?;
            if head.major == major::SIMPLE && head.info == info::INDEFINITE {
                self.end_indefinite(head.offset)?;
                if let Some(value) = self.close()? {
                    return Ok(value);
                }
                continue;
            }
            if self.stack.last().is_some_and(Frame::expects_key) {
                self.key(head)?;
                continue;
            }
            if self.stack.len() + 1 > MAX_DEPTH {
                return Err(at_byte(head.offset, DEPTH_MESSAGE));
            }
            if let Some(value) = self.item(head)?
                && let Some(root) = self.deliver(value, head.offset)?
            {
                return Ok(root);
            }
        }
    }

    /// Decodes a scalar or opens a container; returns the scalar.
    fn item(&mut self, head: Head) -> Result<Option<Value>, SyntaxError> {
        match head.major {
            major::UNSIGNED | major::NEGATIVE => integer(head).map(Some),
            major::TEXT => self.text(head).map(|text| Some(Value::String(text))),
            major::ARRAY => {
                let remaining = self.container_length(head, MIN_ELEMENT_SIZE)?;
                let capacity = Self::capacity(remaining, self.remaining());
                self.stack.push(Frame::Array {
                    items: Vec::with_capacity(capacity),
                    remaining,
                });
                Ok(None)
            }
            major::MAP => {
                let remaining = self.container_length(head, MIN_ENTRY_SIZE)?;
                self.stack.push(Frame::Map {
                    members: Map::new(),
                    remaining,
                    key: None,
                });
                Ok(None)
            }
            major::BYTES => Err(at_byte(head.offset, BYTES_MESSAGE)),
            major::TAG => Err(at_byte(head.offset, TAG_MESSAGE)),
            _ => self.simple(head).map(Some),
        }
    }

    /// Room reserved for a container of `remaining` elements, given `input_left`
    /// unread bytes. The count comes from the input, and a container that only
    /// claims many elements must not reserve memory for all of them at each of
    /// [`MAX_DEPTH`] levels of nesting: the share of the unread input for one
    /// level bounds the reservation, so the memory reserved by headers stays
    /// proportional to the input size.
    fn capacity(remaining: Option<u64>, input_left: u64) -> usize {
        let share = usize::try_from(input_left / MAX_DEPTH as u64).unwrap_or(usize::MAX);
        remaining
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default()
            .min(MAX_RESERVED_ELEMENTS)
            .min(share)
    }

    /// Returns the number of elements of a container; rejects a length that
    /// the remaining input cannot hold.
    fn container_length(&self, head: Head, min_size: u64) -> Result<Option<u64>, SyntaxError> {
        match head.argument {
            Some(count) if count.saturating_mul(min_size) > self.remaining() => {
                Err(at_byte(head.offset, length_message(count)))
            }
            argument => Ok(argument),
        }
    }

    fn simple(&mut self, head: Head) -> Result<Value, SyntaxError> {
        match head.info {
            info::FALSE => Ok(Value::Bool(false)),
            info::TRUE => Ok(Value::Bool(true)),
            info::NULL => Ok(Value::Null),
            info::UNDEFINED => Err(at_byte(
                head.offset,
                "`undefined` is not admissible; use null",
            )),
            info::TWO_BYTES | info::FOUR_BYTES | info::EIGHT_BYTES => {
                let bits = head.argument.unwrap_or_default();
                let number = match head.info {
                    info::TWO_BYTES => half_to_f64(u16::try_from(bits).unwrap_or_default()),
                    info::FOUR_BYTES => {
                        f64::from(f32::from_bits(u32::try_from(bits).unwrap_or_default()))
                    }
                    _ => f64::from_bits(bits),
                };
                Number::from_f64(number)
                    .map(Value::Number)
                    .ok_or_else(|| at_byte(head.offset, FLOAT_MESSAGE))
            }
            _ => Err(at_byte(
                head.offset,
                "simple values other than true, false and null are not admissible",
            )),
        }
    }

    fn text(&mut self, head: Head) -> Result<String, SyntaxError> {
        let Some(length) = head.argument else {
            return self.indefinite_text();
        };
        let length = usize::try_from(length)
            .ok()
            .filter(|_| length <= self.remaining())
            .ok_or_else(|| at_byte(head.offset, length_message(length)))?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| at_byte(head.offset, "text must be valid UTF-8"))
    }

    fn indefinite_text(&mut self) -> Result<String, SyntaxError> {
        let mut text = String::new();
        loop {
            let chunk = self.head()?;
            if chunk.major == major::SIMPLE && chunk.info == info::INDEFINITE {
                return Ok(text);
            }
            if chunk.major != major::TEXT || chunk.argument.is_none() {
                return Err(at_byte(
                    chunk.offset,
                    "a chunk of indefinite-length text must be definite-length text",
                ));
            }
            text.push_str(&self.text(chunk)?);
        }
    }

    fn key(&mut self, head: Head) -> Result<(), SyntaxError> {
        if head.major != major::TEXT {
            return Err(at_byte(head.offset, KEY_MESSAGE));
        }
        let name = self.text(head)?;
        if let Some(Frame::Map { members, key, .. }) = self.stack.last_mut() {
            if members.contains_key(&name) {
                return Err(at_byte(
                    head.offset,
                    format!("duplicate map key `{name}`; keep a single member"),
                ));
            }
            *key = Some((name, head.offset));
        }
        Ok(())
    }

    fn end_indefinite(&mut self, offset: usize) -> Result<(), SyntaxError> {
        match self.stack.last_mut() {
            Some(
                Frame::Array { remaining, .. }
                | Frame::Map {
                    remaining,
                    key: None,
                    ..
                },
            ) if remaining.is_none() => {
                *remaining = Some(0);
                Ok(())
            }
            _ => Err(at_byte(
                offset,
                "unexpected break code; the input is not well-formed CBOR",
            )),
        }
    }

    /// Closes the innermost container and delivers it to its parent; returns
    /// the document when the root container is closed.
    fn close(&mut self) -> Result<Option<Value>, SyntaxError> {
        match self.stack.pop() {
            Some(frame) => self.deliver(frame.into_value(), self.offset),
            None => Ok(None),
        }
    }

    /// Adds a decoded value to the innermost container; returns the value
    /// when it is the document.
    fn deliver(&mut self, value: Value, offset: usize) -> Result<Option<Value>, SyntaxError> {
        match self.stack.last_mut() {
            None => Ok(Some(value)),
            Some(Frame::Array { items, remaining }) => {
                items.push(value);
                *remaining = remaining.map(|count| count - 1);
                Ok(None)
            }
            Some(Frame::Map {
                members,
                remaining,
                key,
            }) => {
                let (name, _) = key.take().ok_or_else(|| at_byte(offset, KEY_MESSAGE))?;
                members.insert(name, value);
                *remaining = remaining.map(|count| count - 1);
                Ok(None)
            }
        }
    }
}

const TRUNCATED: &str = "unexpected end of input; the document is truncated";

const MALFORMED_INDEFINITE: &str = "an indefinite length is admissible only for text, arrays and maps; the input is not well-formed CBOR";

fn length_message(length: u64) -> String {
    format!("length {length} exceeds the remaining input; the document is truncated or corrupt")
}

/// Decodes an integer of the major type 0 or 1.
fn integer(head: Head) -> Result<Value, SyntaxError> {
    let argument = head
        .argument
        .ok_or_else(|| at_byte(head.offset, MALFORMED_INDEFINITE))?;
    let magnitude = i64::try_from(argument)
        .ok()
        .filter(|magnitude| *magnitude <= MAX_SAFE_INTEGER)
        .ok_or_else(|| at_byte(head.offset, INTEGER_MESSAGE))?;
    if head.major == major::UNSIGNED {
        Ok(Value::from(magnitude))
    } else if magnitude < MAX_SAFE_INTEGER {
        Ok(Value::from(-1 - magnitude))
    } else {
        Err(at_byte(head.offset, INTEGER_MESSAGE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_room_only_up_to_the_limit() {
        let plenty = u64::MAX;
        assert_eq!(Reader::capacity(None, plenty), 0);
        assert_eq!(Reader::capacity(Some(5), plenty), 5);
        assert_eq!(
            Reader::capacity(Some(MAX_RESERVED_ELEMENTS as u64 + 1), plenty),
            MAX_RESERVED_ELEMENTS
        );
        assert_eq!(
            Reader::capacity(Some(u64::MAX), plenty),
            MAX_RESERVED_ELEMENTS
        );
    }

    #[test]
    fn reservation_is_bounded_by_the_unread_input() {
        let depth = MAX_DEPTH as u64;
        assert_eq!(Reader::capacity(Some(1000), 0), 0);
        assert_eq!(Reader::capacity(Some(1000), depth - 1), 0);
        assert_eq!(Reader::capacity(Some(1000), depth * 7), 7);
        assert_eq!(Reader::capacity(Some(3), depth * 7), 3);
    }

    #[test]
    fn nested_headers_reserve_memory_proportional_to_the_input() {
        // Every level claims all remaining bytes as elements; the sum of the
        // reservations over the levels must stay within the input size.
        let mut input_left = 1024_u64;
        let mut reserved = 0_usize;
        for _ in 0..MAX_DEPTH {
            reserved += Reader::capacity(Some(input_left), input_left);
            input_left -= 1;
        }
        assert!(reserved as u64 <= 1024);
    }
}
