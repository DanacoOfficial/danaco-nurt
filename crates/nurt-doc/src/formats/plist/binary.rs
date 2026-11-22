// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binary property lists (`bplist00`).
//!
//! A binary property list has no layout to preserve: object offsets and the
//! object table are derived from the values. It is read and written with the
//! `plist` crate; every change writes the whole file again, with the same
//! bytes for the same values, and the change carries the flag
//! `binary_rewritten`.

use std::io::Cursor;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::KeySegment;
use nurt_contracts::value::Value;

use super::value::{self, Hint, UID_TAG};
use crate::model::{DocTree, NodeId, NodeKind, NodeSpec, Span, TreeBuilder};

/// Signature at the start of every binary property list.
pub(crate) const SIGNATURE_PREFIX: &[u8] = b"bplist";

/// Signature of the binary format version that is read and written.
pub(crate) const SIGNATURE: &[u8] = b"bplist00";

/// Deepest nesting of arrays and dictionaries accepted.
const MAX_DEPTH: usize = 512;

/// Reads a binary property list.
///
/// # Errors
///
/// Returns `E_UNSUPPORTED_CONSTRUCT` for binary versions other than `00` and
/// `E_DOC_PARSE` for a damaged file.
pub(crate) fn read(bytes: &[u8]) -> Result<plist::Value, Error> {
    if !bytes.starts_with(SIGNATURE) {
        let version = bytes
            .get(SIGNATURE_PREFIX.len()..SIGNATURE.len())
            .unwrap_or_default();
        return Err(crate::constructs::unsupported(
            crate::constructs::PLIST_BINARY_LAYOUT,
            format!(
                "binary property list version {} cannot be read; convert the file to bplist00 or XML",
                String::from_utf8_lossy(version)
            ),
        ));
    }
    plist::Value::from_reader(Cursor::new(bytes)).map_err(|error| {
        Error::new(
            ErrorCode::DocParse,
            format!("invalid plist document: {error}; restore the binary property list or convert it again"),
        )
    })
}

/// Writes `value` as a binary property list.
///
/// # Errors
///
/// Returns `E_INVALID_VALUE` when the value cannot be encoded.
pub(crate) fn write(value: &plist::Value) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    value.to_writer_binary(&mut bytes).map_err(|error| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("value cannot be written as a binary property list: {error}; pass a value of a property list type"),
        )
    })?;
    Ok(bytes)
}

/// Node tree of a binary property list; nodes have empty byte ranges at the
/// start of the file because the format has no text positions.
///
/// # Errors
///
/// Returns `E_DOC_PARSE` for nesting deeper than 512 levels.
pub(crate) fn tree(bytes: &[u8], root: &plist::Value) -> Result<DocTree, Error> {
    let mut builder = TreeBuilder::new(bytes);
    add(&mut builder, None, None, root, 0)?;
    Ok(builder.finish())
}

fn add(
    builder: &mut TreeBuilder<'_>,
    parent: Option<NodeId>,
    name: Option<&str>,
    value: &plist::Value,
    depth: usize,
) -> Result<(), Error> {
    if depth > MAX_DEPTH {
        return Err(Error::new(
            ErrorCode::DocParse,
            "invalid plist document: arrays and dictionaries are nested deeper than 512 levels; flatten the document",
        ));
    }
    let kind = match value {
        plist::Value::Array(_) => NodeKind::List,
        plist::Value::Dictionary(_) => NodeKind::Map,
        _ => NodeKind::Scalar,
    };
    let mut spec = NodeSpec::new(kind, Span::new(0, 0));
    if let Some(name) = name {
        spec = spec.with_name(name);
    }
    if kind == NodeKind::Scalar {
        spec = spec.with_value(value::to_value(value));
    }
    if matches!(value, plist::Value::Uid(_)) {
        spec = spec.with_tag(UID_TAG);
    }
    let node = builder.node(parent, spec);
    match value {
        plist::Value::Array(items) => {
            for item in items {
                add(builder, Some(node), None, item, depth + 1)?;
            }
        }
        plist::Value::Dictionary(dictionary) => {
            for (key, item) in dictionary {
                add(builder, Some(node), Some(key), item, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Mutable value at a concrete path of keys and indexes.
pub(crate) fn get_mut<'a>(
    root: &'a mut plist::Value,
    segments: &[KeySegment],
) -> Option<&'a mut plist::Value> {
    segments
        .iter()
        .try_fold(root, |value, segment| match (value, segment) {
            (plist::Value::Dictionary(dictionary), KeySegment::Key(name)) => {
                dictionary.get_mut(name)
            }
            (plist::Value::Array(items), KeySegment::Index(index)) => usize::try_from(*index)
                .ok()
                .and_then(|index| items.get_mut(index)),
            _ => None,
        })
}

/// Replaces the value at `segments`, keeping a `real` or `UID` type.
///
/// # Errors
///
/// Returns the errors of [`value::from_value`] and `E_KEY_NOT_FOUND` for a
/// path that does not exist.
pub(crate) fn replace(
    root: &mut plist::Value,
    segments: &[KeySegment],
    value: &Value,
) -> Result<(), Error> {
    let slot = get_mut(root, segments).ok_or_else(missing)?;
    *slot = value::from_value(value, Hint::of(slot))?;
    Ok(())
}

/// Adds `name` to the dictionary at `segments`.
///
/// # Errors
///
/// As for [`replace`].
pub(crate) fn insert(
    root: &mut plist::Value,
    segments: &[KeySegment],
    name: &str,
    value: &Value,
) -> Result<(), Error> {
    let new = value::from_value(value, Hint::Other)?;
    match get_mut(root, segments) {
        Some(plist::Value::Dictionary(dictionary)) => {
            dictionary.insert(name.to_owned(), new);
            Ok(())
        }
        _ => Err(missing()),
    }
}

/// Appends to the array at `segments`.
///
/// # Errors
///
/// As for [`replace`].
pub(crate) fn append(
    root: &mut plist::Value,
    segments: &[KeySegment],
    value: &Value,
) -> Result<(), Error> {
    let new = value::from_value(value, Hint::Other)?;
    match get_mut(root, segments) {
        Some(plist::Value::Array(items)) => {
            items.push(new);
            Ok(())
        }
        _ => Err(missing()),
    }
}

/// Removes the value at `segments`.
///
/// # Errors
///
/// Returns `E_KEY_NOT_FOUND` for a path that does not exist.
pub(crate) fn remove(root: &mut plist::Value, segments: &[KeySegment]) -> Result<(), Error> {
    let Some((last, parent)) = segments.split_last() else {
        return Err(missing());
    };
    match (get_mut(root, parent), last) {
        (Some(plist::Value::Dictionary(dictionary)), KeySegment::Key(name)) => {
            dictionary.remove(name).map(drop).ok_or_else(missing)
        }
        (Some(plist::Value::Array(items)), KeySegment::Index(index)) => {
            let index = usize::try_from(*index)
                .ok()
                .filter(|index| *index < items.len())
                .ok_or_else(missing)?;
            items.remove(index);
            Ok(())
        }
        _ => Err(missing()),
    }
}

fn missing() -> Error {
    Error::new(
        ErrorCode::KeyNotFound,
        "key not found in the binary property list; read the document to list its keys",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::{Dictionary, Uid};

    fn sample() -> plist::Value {
        let mut inner = Dictionary::new();
        inner.insert("uid".into(), plist::Value::Uid(Uid::new(7)));
        inner.insert("real".into(), plist::Value::Real(2.0));
        let mut root = Dictionary::new();
        root.insert("name".into(), plist::Value::String("app".into()));
        root.insert(
            "items".into(),
            plist::Value::Array(vec![plist::Value::Dictionary(inner)]),
        );
        plist::Value::Dictionary(root)
    }

    fn path(text: &str) -> Vec<KeySegment> {
        nurt_contracts::keypath::KeyPath::parse(text)
            .unwrap()
            .segments()
            .to_vec()
    }

    #[test]
    fn writing_is_deterministic_and_round_trips() {
        let value = sample();
        let first = write(&value).unwrap();
        assert!(first.starts_with(SIGNATURE));
        assert_eq!(write(&value).unwrap(), first);
        assert_eq!(read(&first).unwrap(), value);
    }

    #[test]
    fn changes_keep_real_and_uid_types() {
        let mut value = sample();
        replace(&mut value, &path("items[0].uid"), &Value::Int(8)).unwrap();
        replace(&mut value, &path("items[0].real"), &Value::Int(3)).unwrap();
        insert(&mut value, &path("items[0]"), "new", &Value::Bool(true)).unwrap();
        append(&mut value, &path("items"), &Value::Text("x".into())).unwrap();
        remove(&mut value, &path("name")).unwrap();
        let items = get_mut(&mut value, &path("items")).unwrap().clone();
        let plist::Value::Array(items) = items else {
            panic!("array expected")
        };
        let first = items[0].as_dictionary().unwrap();
        assert_eq!(first.get("uid"), Some(&plist::Value::Uid(Uid::new(8))));
        assert_eq!(first.get("real"), Some(&plist::Value::Real(3.0)));
        assert_eq!(first.get("new"), Some(&plist::Value::Boolean(true)));
        assert_eq!(items.len(), 2);
        assert!(remove(&mut value, &path("name")).is_err());
        assert!(remove(&mut value, &path("items[5]")).is_err());
        assert!(insert(&mut value, &path("missing"), "a", &Value::Int(1)).is_err());
        assert!(append(&mut value, &path("missing"), &Value::Int(1)).is_err());
    }

    #[test]
    fn tree_names_members_and_tags_uids() {
        let bytes = write(&sample()).unwrap();
        let tree = tree(&bytes, &sample()).unwrap();
        let root = tree.roots()[0];
        assert_eq!(tree[root].children.len(), 2);
        let found = tree
            .resolve(
                &nurt_contracts::keypath::KeyPath::parse("items[0].uid").unwrap(),
                &crate::model::MapSemantics::LAST_WINS,
            )
            .unwrap();
        assert_eq!(tree[found[0].node].tag.as_deref(), Some(UID_TAG));
    }

    #[test]
    fn rejects_other_versions_and_damaged_files() {
        let error = read(b"bplist15\0\0").unwrap_err();
        assert_eq!(error.code().as_str(), "E_UNSUPPORTED_CONSTRUCT");
        let error = read(b"bplist00garbage").unwrap_err();
        assert_eq!(error.code().as_str(), "E_DOC_PARSE");
    }
}
