// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! JSON codec of syntax trees.
//!
//! Decoding checks, in this order: JSON syntax, the nesting depth
//! ([`MAX_JSON_DEPTH`]), the format version, the members of every object of
//! a surface tree (of a bound tree through [`validate::check_members`]),
//! the structure of every node and the invariants of
//! [`validate::validate_surface`] and [`validate::validate_bound`]. A
//! document of a newer
//! minor version or of another major version is rejected with
//! `E_AST_VERSION`, naming the node kinds the reader does not know; every
//! other failure is `E_AST_SCHEMA`. Both carry the JSON Pointer (RFC 6901) of
//! the offending value in [`SyntaxError::found`]; the empty pointer denotes
//! the whole document.
//!
//! Encoding writes members in declaration order ([`encode`],
//! [`encode_pretty`]) or in the canonical form of RFC 8785
//! ([`encode_canonical`]), the input of `ast_hash` after
//! [`nurt_contracts::ast::strip_for_hash`].
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::error::ErrorCode;
//! use nurt_syntax::json;
//!
//! let text = r#"{"nurt": "1.0", "program": {"statements": [{"t": "pipeline",
//!     "head": {"t": "stage", "name": "count", "args": []}, "stages": []}]}}"#;
//! let document = json::decode_surface(text)?;
//! assert_eq!(
//!     json::encode(&document)?,
//!     r#"{"nurt":"1.0","program":{"statements":[{"t":"pipeline","head":{"t":"stage","name":"count","args":[]},"stages":[]}]}}"#
//! );
//!
//! let error = json::decode_surface(&text.replace("1.0", "1.9")).unwrap_err();
//! assert_eq!(error.code, ErrorCode::AstVersion);
//! assert_eq!(error.found.as_deref(), Some("/nurt"));
//! # Ok::<(), nurt_syntax::error::SyntaxError>(())
//! ```

use std::collections::BTreeSet;
use std::fmt;

use nurt_contracts::ast::{
    AST_VERSION, Arg, AstDocument, Branch, Expr, Head, KeySeg, MapEntry, Param, Pipeline, Program,
    SURFACE_KINDS, Stage, Statement,
};
use nurt_contracts::bound::{BOUND_KINDS, BOUND_VERSION, BoundDocument};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::SyntaxError;
use crate::validate;

/// Deepest nesting of JSON values accepted in a tree document; the root
/// value is at depth 1.
pub const MAX_JSON_DEPTH: usize = validate::MAX_DEPTH;

/// Decodes a surface tree from JSON text.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` for invalid JSON, nesting deeper than
/// [`MAX_JSON_DEPTH`], an unknown member or node kind and any structural
/// error, and `E_AST_VERSION` for an unsupported version, as described in
/// the module documentation.
pub fn decode_surface(text: &str) -> Result<AstDocument, SyntaxError> {
    decode_surface_value(parse_json(text)?)
}

/// Decodes a surface tree from a JSON value.
///
/// # Errors
///
/// As [`decode_surface`].
pub fn decode_surface_value(value: Value) -> Result<AstDocument, SyntaxError> {
    check_root(&value)?;
    check_version(&value, "nurt", AST_VERSION, &SURFACE_KINDS)?;
    let mut pointer = String::new();
    Walk::Members
        .root(&value, &mut pointer)
        .map_err(Fault::into_error)?;
    let document = AstDocument::deserialize(&value).map_err(|error| {
        let mut pointer = String::new();
        match Walk::Types.root(&value, &mut pointer) {
            Err(fault) => fault.into_error(),
            Ok(()) => Fault::new(String::new(), error.to_string()).into_error(),
        }
    })?;
    validate::validate_surface(&document)?;
    Ok(document)
}

/// Decodes a bound tree from JSON text.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` for invalid JSON, nesting deeper than
/// [`MAX_JSON_DEPTH`] and structural errors, and `E_AST_VERSION` when the
/// member `nurt` or `bound` has an unsupported version.
pub fn decode_bound(text: &str) -> Result<BoundDocument, SyntaxError> {
    decode_bound_value(parse_json(text)?)
}

/// Decodes a bound tree from a JSON value.
///
/// # Errors
///
/// As [`decode_bound`].
pub fn decode_bound_value(value: Value) -> Result<BoundDocument, SyntaxError> {
    check_root(&value)?;
    check_version(&value, "nurt", AST_VERSION, &BOUND_KINDS)?;
    check_version(&value, "bound", BOUND_VERSION, &BOUND_KINDS)?;
    let document = BoundDocument::deserialize(&value)
        .map_err(|error| Fault::new(String::new(), error.to_string()).into_error())?;
    let typed = serde_json::to_value(&document).map_err(encode_error)?;
    validate::check_members(&value, &typed)?;
    validate::validate_bound(&document)?;
    Ok(document)
}

/// Encodes a tree document, or any part of one, as compact JSON with
/// members in declaration order.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` when the value cannot be represented in JSON.
pub fn encode<T: Serialize + ?Sized>(tree: &T) -> Result<String, SyntaxError> {
    serde_json::to_string(tree).map_err(encode_error)
}

/// Encodes a tree document, or any part of one, as indented JSON for display
/// and golden files.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` when the value cannot be represented in JSON.
pub fn encode_pretty<T: Serialize + ?Sized>(tree: &T) -> Result<String, SyntaxError> {
    serde_json::to_string_pretty(tree).map_err(encode_error)
}

/// Encodes a value in the canonical JSON form of RFC 8785 (JCS).
///
/// Applied to the hash form of a document
/// ([`nurt_contracts::ast::strip_for_hash`]), the result is the input of
/// `ast_hash`.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` when the value cannot be represented in canonical
/// JSON, for example a number that is not finite.
///
/// # Examples
///
/// ```
/// use nurt_syntax::json;
///
/// let value = serde_json::json!({"t": "int", "v": 1, "span": [1, 1, 1, 2]});
/// assert_eq!(json::encode_canonical(&value)?, r#"{"span":[1,1,1,2],"t":"int","v":1}"#);
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub fn encode_canonical<T: Serialize>(tree: &T) -> Result<String, SyntaxError> {
    serde_json_canonicalizer::to_string(tree).map_err(encode_error)
}

fn encode_error(error: impl fmt::Display) -> SyntaxError {
    SyntaxError::ast_schema(format!("cannot encode the tree as JSON: {error}"))
        .with_problem("the tree holds a value without a JSON form; use finite numbers only")
}

/// A JSON value read without accepting an object that repeats a key: a
/// repeated key would be settled in favour of the last one, while the CBOR
/// decoder refuses it.
struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> serde::de::Visitor<'de> for StrictVisitor {
    type Value = Strict;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Strict, E> {
        Ok(Strict(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Strict, E> {
        Ok(Strict(Value::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Strict, E> {
        Ok(Strict(Value::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Strict, E> {
        Ok(Strict(Value::from(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Strict, E> {
        Ok(Strict(Value::String(value.to_owned())))
    }

    fn visit_unit<E>(self) -> Result<Strict, E> {
        Ok(Strict(Value::Null))
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
        let mut items = Vec::new();
        while let Some(Strict(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Strict(Value::Array(items)))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
        let mut members = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let Strict(value) = map.next_value()?;
            if members.insert(key.clone(), value).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "the key `{key}` is repeated in an object"
                )));
            }
        }
        Ok(Strict(Value::Object(members)))
    }
}

fn parse_json(text: &str) -> Result<Value, SyntaxError> {
    serde_json::from_str::<Strict>(text).map(|Strict(value)| value).map_err(|error| {
        Fault::new(
            String::new(),
            format!("invalid JSON: {error}; send one JSON object nested at most {MAX_JSON_DEPTH} levels deep"),
        )
        .into_error()
    })
}

/// Failure located by a JSON Pointer.
#[derive(Debug)]
struct Fault {
    pointer: String,
    message: String,
}

impl Fault {
    fn new(pointer: String, message: impl Into<String>) -> Self {
        Self {
            pointer,
            message: message.into(),
        }
    }

    fn into_error(self) -> SyntaxError {
        let at = if self.pointer.is_empty() {
            "the document root".to_owned()
        } else {
            format!("`{}`", self.pointer)
        };
        SyntaxError::ast_schema(format!("{} at {at}", self.message)).with_found(self.pointer)
    }
}

fn check_root(value: &Value) -> Result<(), SyntaxError> {
    if !value.is_object() {
        return Err(Fault::new(
            String::new(),
            "a tree document is a JSON object; wrap the tree in an object",
        )
        .into_error());
    }
    if depth(value) > MAX_JSON_DEPTH {
        return Err(Fault::new(
            String::new(),
            format!(
                "the document is nested deeper than {MAX_JSON_DEPTH} levels; split the program"
            ),
        )
        .into_error());
    }
    Ok(())
}

/// Nesting depth of a JSON value, the value itself at depth 1, computed
/// without recursion.
fn depth(value: &Value) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(value, 1_usize)];
    while let Some((current, level)) = pending.pop() {
        deepest = deepest.max(level);
        match current {
            Value::Object(members) => pending.extend(members.values().map(|v| (v, level + 1))),
            Value::Array(items) => pending.extend(items.iter().map(|v| (v, level + 1))),
            _ => {}
        }
    }
    deepest
}

/// Format version `MAJOR.MINOR` written without leading zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Version {
    major: u32,
    minor: u32,
}

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let (major, minor) = text.split_once('.')?;
        Some(Self {
            major: version_component(major)?,
            minor: version_component(minor)?,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

fn version_component(text: &str) -> Option<u32> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if canonical { text.parse().ok() } else { None }
}

fn check_version(
    value: &Value,
    member: &str,
    reader: &str,
    known: &[&str],
) -> Result<(), SyntaxError> {
    let pointer = format!("/{member}");
    let Some(text) = value.get(member).and_then(Value::as_str) else {
        return Err(Fault::new(
            pointer,
            format!(
                "the root member `{member}` is missing or not a string; write it as \"MAJOR.MINOR\""
            ),
        )
        .into_error());
    };
    let (Some(version), Some(reader)) = (Version::parse(text), Version::parse(reader)) else {
        return Err(Fault::new(
            pointer,
            format!("invalid format version `{text}`; write MAJOR.MINOR without leading zeros"),
        )
        .into_error());
    };
    let message = if version.major != reader.major {
        format!(
            "major version {} is not supported; write the tree at version {reader}",
            version.major
        )
    } else if version.minor > reader.minor {
        format!(
            "version {version} is newer than the supported version {reader}; upgrade the reader or write the tree at version {reader}"
        )
    } else {
        return Ok(());
    };
    let unknown = unknown_kinds(value, known);
    let problem = if unknown.is_empty() {
        "the document uses no unknown node kinds".to_owned()
    } else {
        format!("unknown node kinds: {}", unknown.join(", "))
    };
    Err(SyntaxError::ast_version(message)
        .with_found(pointer)
        .with_problem(problem)
        .with_expected(unknown))
}

/// Node kinds outside `known`, sorted, from every object outside the root
/// member `meta`.
fn unknown_kinds(root: &Value, known: &[&str]) -> Vec<String> {
    let mut found = BTreeSet::new();
    let mut pending: Vec<&Value> = match root {
        Value::Object(members) => members
            .iter()
            .filter(|(name, _)| name.as_str() != "meta")
            .map(|(_, member)| member)
            .collect(),
        _ => Vec::new(),
    };
    while let Some(current) = pending.pop() {
        match current {
            Value::Object(members) => {
                if let Some(kind) = members.get("t").and_then(Value::as_str)
                    && !known.contains(&kind)
                {
                    found.insert(kind.to_owned());
                }
                pending.extend(members.values());
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    found.into_iter().collect()
}

/// Place of a JSON value in a surface tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Program,
    Statement,
    Pipeline,
    Head,
    Stage,
    Arg,
    Expr,
    Entry,
    KeySeg,
    Branch,
    Param,
}

/// Values held by a member of a node: one value or an array of values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Child {
    One(Role),
    Many(Role),
}

/// Role of the value held by `member` of a node with the role `role` and
/// the kind `kind`; `None` for members that hold no nodes.
fn child_role(role: Role, kind: &str, member: &str) -> Option<Child> {
    use Child::{Many, One};
    let child = match (role, kind, member) {
        (Role::Program, _, "statements") => Many(Role::Statement),
        (Role::Statement, "let", "value") | (Role::Expr, "sub", "p") => One(Role::Pipeline),
        (Role::Statement, "pipeline", "head") | (Role::Pipeline, _, "head") => One(Role::Head),
        (Role::Statement, "pipeline", "stages") | (Role::Pipeline, _, "stages") => {
            Many(Role::Stage)
        }
        (Role::Head, "stage", "args") | (Role::Stage, _, "args") => Many(Role::Arg),
        (Role::Arg, _, "value") | (Role::Entry, _, "v") | (Role::KeySeg, _, "filter") => {
            One(Role::Expr)
        }
        (Role::Expr, "list", "items") | (Role::Expr, "call", "args") => Many(Role::Expr),
        (Role::Expr, "map", "entries") => Many(Role::Entry),
        (Role::Expr, "key", "segs") => Many(Role::KeySeg),
        (Role::Statement, "when", "branches") => Many(Role::Branch),
        (Role::Statement, "when", "otherwise")
        | (Role::Branch, _, "body")
        | (Role::Statement, "each" | "repeat" | "while" | "steps" | "define", "body") => {
            Many(Role::Statement)
        }
        (Role::Branch, _, "cond")
        | (Role::Statement, "each", "source")
        | (Role::Statement, "while", "cond")
        | (Role::Param, _, "default") => One(Role::Expr),
        (Role::Statement, "each" | "repeat" | "while", "options") => Many(Role::Arg),
        (Role::Statement, "define" | "params", "params") => Many(Role::Param),
        (Role::Expr, "spread" | "un", "x")
        | (Role::Expr, "field", "of")
        | (Role::Expr, "bin", "l" | "r") => One(Role::Expr),
        _ => return None,
    };
    Some(child)
}

/// Check applied to every value of a surface tree, children first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Walk {
    /// Rejects members and node kinds outside the definition of the tree.
    Members,
    /// Locates the innermost value that does not deserialize into its type.
    Types,
}

const ROOT_MEMBERS: [&str; 3] = ["nurt", "program", "meta"];

impl Walk {
    fn root(self, value: &Value, pointer: &mut String) -> Result<(), Fault> {
        if self == Self::Members
            && let Value::Object(members) = value
            && let Some(name) = members
                .keys()
                .find(|name| !ROOT_MEMBERS.contains(&name.as_str()))
        {
            return Err(unknown_member("", name, "the document root"));
        }
        match value.get("program") {
            Some(program) => self.child(program, "program", Role::Program, pointer),
            None => Ok(()),
        }
    }

    fn child(
        self,
        value: &Value,
        member: &str,
        role: Role,
        pointer: &mut String,
    ) -> Result<(), Fault> {
        let length = pointer.len();
        pointer.push('/');
        pointer.push_str(member);
        let result = self.node(value, role, pointer);
        pointer.truncate(length);
        result
    }

    fn items(
        self,
        value: &Value,
        member: &str,
        role: Role,
        pointer: &mut String,
    ) -> Result<(), Fault> {
        let Value::Array(items) = value else {
            return Ok(());
        };
        let length = pointer.len();
        pointer.push('/');
        pointer.push_str(member);
        let result = items
            .iter()
            .enumerate()
            .try_for_each(|(index, item)| self.child(item, &index.to_string(), role, pointer));
        pointer.truncate(length);
        result
    }

    fn node(self, value: &Value, role: Role, pointer: &mut String) -> Result<(), Fault> {
        if let Value::Object(members) = value {
            let kind = members.get("t").and_then(Value::as_str).unwrap_or_default();
            for (member, child) in members {
                match child_role(role, kind, member) {
                    Some(Child::One(child_role)) => {
                        self.child(child, member, child_role, pointer)?
                    }
                    Some(Child::Many(child_role)) => {
                        self.items(child, member, child_role, pointer)?
                    }
                    None => {}
                }
            }
        }
        match self {
            Self::Members => check_members(value, role, pointer),
            Self::Types => check_type(value, role, pointer),
        }
    }
}

fn unknown_member(pointer: &str, member: &str, node: &str) -> Fault {
    Fault::new(
        format!("{pointer}/{}", validate::escape_pointer_segment(member)),
        format!(
            "unknown member `{member}` in {node}; remove it or decode the document as a bound tree"
        ),
    )
}

/// Members of a node kind of the surface tree.
fn node_members(kind: &str) -> Option<&'static [&'static str]> {
    let members: &'static [&'static str] = match kind {
        "let" => &["t", "name", "value", "span"],
        "pipeline" => &["t", "head", "stages", "span"],
        "stage" => &["t", "name", "args", "span"],
        "ref" => &["t", "name", "fields", "span"],
        "text" | "ident" | "int" | "num" | "size" | "dur" | "bool" => &["t", "v", "span"],
        "regex" => &["t", "v", "flags", "span"],
        "null" => &["t", "span"],
        "key" => &["t", "segs", "span"],
        "list" => &["t", "items", "span"],
        "map" => &["t", "entries", "span"],
        "spread" => &["t", "x", "span"],
        "field" => &["t", "name", "of", "span"],
        "call" => &["t", "fn", "args", "span"],
        "un" => &["t", "op", "x", "span"],
        "bin" => &["t", "op", "l", "r", "span"],
        "range" => &["t", "lo", "hi", "span"],
        "sub" => &["t", "p", "span"],
        "when" => &["t", "branches", "otherwise", "span"],
        "each" => &["t", "source", "name", "options", "body", "span"],
        "repeat" => &["t", "options", "body", "span"],
        "while" => &["t", "cond", "options", "body", "span"],
        "break" | "continue" => &["t", "span"],
        "steps" => &["t", "body", "span"],
        "define" => &["t", "name", "params", "body", "span"],
        "params" => &["t", "params", "span"],
        _ => return None,
    };
    Some(members)
}

/// Members allowed in an object with the role `role` and the kind `kind`,
/// or `None` when the kind is not admissible in that role.
fn allowed_members(role: Role, kind: &str) -> Option<&'static [&'static str]> {
    match role {
        Role::Program => Some(&["statements"]),
        Role::Arg => Some(&["name", "value", "span"]),
        Role::Entry => Some(&["k", "v", "span"]),
        Role::KeySeg => Some(&["wild", "filter"]),
        Role::Branch => Some(&["cond", "body", "span"]),
        Role::Param => Some(&["name", "type", "default", "span"]),
        Role::Statement if is_statement_kind(kind) => node_members(kind),
        Role::Pipeline if kind == "pipeline" => node_members(kind),
        Role::Head if matches!(kind, "stage" | "ref") => node_members(kind),
        Role::Stage if kind == "stage" => node_members(kind),
        Role::Expr if !is_statement_kind(kind) && kind != "stage" => node_members(kind),
        _ => None,
    }
}

/// Whether `kind` names a statement of the surface tree.
fn is_statement_kind(kind: &str) -> bool {
    matches!(
        kind,
        "let"
            | "pipeline"
            | "when"
            | "each"
            | "repeat"
            | "while"
            | "break"
            | "continue"
            | "steps"
            | "define"
            | "params"
    )
}

fn check_members(value: &Value, role: Role, pointer: &str) -> Result<(), Fault> {
    let Value::Object(members) = value else {
        return Ok(());
    };
    let Some(kind) = members.get("t") else {
        return match allowed_members(role, "") {
            Some(allowed) => match members
                .keys()
                .find(|name| !allowed.contains(&name.as_str()))
            {
                Some(name) => Err(unknown_member(pointer, name, "an object")),
                None => Ok(()),
            },
            None => Err(Fault::new(
                pointer.to_owned(),
                "the node has no member `t`; add the node kind",
            )),
        };
    };
    let kind = kind.as_str().unwrap_or_default();
    let Some(allowed) = allowed_members(role, kind) else {
        return Err(Fault::new(
            format!("{pointer}/t"),
            format!("node kind `{kind}` is not admissible here; use a kind of the surface tree"),
        ));
    };
    match members
        .keys()
        .find(|name| !allowed.contains(&name.as_str()))
    {
        Some(name) => Err(unknown_member(pointer, name, &format!("a `{kind}` node"))),
        None => Ok(()),
    }
}

fn check_type(value: &Value, role: Role, pointer: &str) -> Result<(), Fault> {
    let result = match role {
        Role::Program => Program::deserialize(value).map(drop),
        Role::Statement => Statement::deserialize(value).map(drop),
        Role::Pipeline => Pipeline::deserialize(value).map(drop),
        Role::Head => Head::deserialize(value).map(drop),
        Role::Stage => Stage::deserialize(value).map(drop),
        Role::Arg => Arg::deserialize(value).map(drop),
        Role::Expr => Expr::deserialize(value).map(drop),
        Role::Entry => MapEntry::deserialize(value).map(drop),
        Role::KeySeg => KeySeg::deserialize(value).map(drop),
        Role::Branch => Branch::deserialize(value).map(drop),
        Role::Param => Param::deserialize(value).map(drop),
    };
    result.map_err(|error| Fault::new(pointer.to_owned(), error.to_string()))
}
