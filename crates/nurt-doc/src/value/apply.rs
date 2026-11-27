// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::value::apply
// Responsibility:   Concrete step of a path in a value.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::value::{Fields, Value};

use crate::model::{FormatSemantics, key_conflict, key_not_found};

/// Concrete step of a path in a value.
enum Step {
    Name(String),
    Index(i64),
    Append,
}

/// Sets `value` at the concrete `key` in the document value `root`,
/// creating missing maps and lists on the way (`a.b` creates a map, `a[0]`
/// a list); `[+]` and an index equal to the length append to a list.
///
/// A key with a document selector addresses an element of `root`, which is
/// then the list of the documents of a stream. In flat formats consecutive
/// names are joined with `.` into one member name.
///
/// # Errors
///
/// Fails with `E_KEY_CONFLICT` when the key passes through a value of
/// another kind or is not concrete (wildcards, recursion, filters), and with
/// `E_KEY_NOT_FOUND` for an index beyond the end of a list.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::model::MapSemantics;
/// use nurt_doc::value::apply_set;
///
/// let mut root = Value::Null;
/// apply_set(&mut root, &KeyPath::parse("a.b[0]")?, Value::Int(1), &MapSemantics::LAST_WINS)?;
/// let expected = serde_json::json!({"a": {"b": [1]}});
/// assert_eq!(root, Value::from_plain_json(expected));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn apply_set(
    root: &mut Value,
    key: &KeyPath,
    value: Value,
    semantics: &dyn FormatSemantics,
) -> Result<()> {
    let steps = steps(key, semantics)?;
    let Some((last, path)) = steps.split_last() else {
        *document(root, key)? = value;
        return Ok(());
    };
    let mut current = document(root, key)?;
    for (position, step) in path.iter().enumerate() {
        let next = steps.get(position + 1);
        current = child(current, step, next, key)?;
    }
    put(current, last, value, key)
}

/// Removes the member or element at the concrete `key` from the document
/// value `root`.
///
/// # Errors
///
/// Fails with `E_KEY_NOT_FOUND` when the key does not exist, and with
/// `E_KEY_CONFLICT` when it passes through a value of another kind or is not
/// concrete.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::model::MapSemantics;
/// use nurt_doc::value::apply_remove;
///
/// let mut root = Value::from_plain_json(serde_json::json!({"a": [1, 2]}));
/// apply_remove(&mut root, &KeyPath::parse("a[0]")?, &MapSemantics::LAST_WINS)?;
/// assert_eq!(root, Value::from_plain_json(serde_json::json!({"a": [2]})));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn apply_remove(
    root: &mut Value,
    key: &KeyPath,
    semantics: &dyn FormatSemantics,
) -> Result<()> {
    let steps = steps(key, semantics)?;
    let Some((last, path)) = steps.split_last() else {
        *document(root, key)? = Value::Null;
        return Ok(());
    };
    let mut current = document(root, key)?;
    for step in path {
        current = existing(current, step, key)?;
    }
    let removed = match (current, last) {
        (Value::Map(fields), Step::Name(name)) => fields.shift_remove(name).is_some(),
        (Value::List(items), Step::Index(index)) => match position(items.len(), *index, false) {
            Some(at) => {
                items.remove(at);
                true
            }
            None => false,
        },
        (Value::List(items), Step::Name(name)) => match name.parse::<usize>() {
            Ok(at) if at < items.len() => {
                items.remove(at);
                true
            }
            _ => false,
        },
        (other, _) => {
            return Err(conflict(key, other));
        }
    };
    if removed {
        Ok(())
    } else {
        Err(key_not_found(key, &[]))
    }
}

fn steps(key: &KeyPath, semantics: &dyn FormatSemantics) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for segment in key.segments() {
        if let KeySegment::Key(name) = segment {
            if semantics.flat_keys() {
                names.push(name);
                continue;
            }
            steps.push(Step::Name(name.clone()));
            continue;
        }
        if !names.is_empty() {
            steps.push(Step::Name(names.join(".")));
            names.clear();
        }
        match segment {
            KeySegment::Index(index) => steps.push(Step::Index(*index)),
            KeySegment::Append => steps.push(Step::Append),
            _ => {
                return Err(key_conflict(
                    key,
                    "pattern",
                    "is not concrete; a change needs names and indexes only",
                ));
            }
        }
    }
    if !names.is_empty() {
        steps.push(Step::Name(names.join(".")));
    }
    Ok(steps)
}

fn document<'v>(root: &'v mut Value, key: &KeyPath) -> Result<&'v mut Value> {
    match key.document() {
        None => Ok(root),
        Some(DocSelector::Index(index)) => {
            let at = usize::try_from(index).unwrap_or(usize::MAX);
            match root {
                Value::List(documents) => {
                    documents.get_mut(at).ok_or_else(|| key_not_found(key, &[]))
                }
                other => Err(conflict(key, other)),
            }
        }
        Some(DocSelector::All) => Err(key_conflict(
            key,
            "pattern",
            "selects every document; a change needs one document",
        )),
    }
}

/// Member or element for the step, created when missing; `next` decides
/// whether a missing container is a map or a list.
fn child<'v>(
    current: &'v mut Value,
    step: &Step,
    next: Option<&Step>,
    key: &KeyPath,
) -> Result<&'v mut Value> {
    if current.is_null() {
        *current = match step {
            Step::Name(_) => Value::Map(Fields::new()),
            Step::Index(_) | Step::Append => Value::List(Vec::new()),
        };
    }
    let fresh = || match next {
        Some(Step::Index(_) | Step::Append) => Value::List(Vec::new()),
        _ => Value::Map(Fields::new()),
    };
    match (current, step) {
        (Value::Map(fields), Step::Name(name)) => {
            Ok(fields.entry(name.clone()).or_insert_with(fresh))
        }
        (Value::List(items), Step::Index(index)) => {
            let at = position(items.len(), *index, true).ok_or_else(|| key_not_found(key, &[]))?;
            if at == items.len() {
                items.push(fresh());
            }
            items.get_mut(at).ok_or_else(|| key_not_found(key, &[]))
        }
        (Value::List(items), Step::Append) => {
            items.push(fresh());
            items.last_mut().ok_or_else(|| key_not_found(key, &[]))
        }
        (Value::List(items), Step::Name(name)) => {
            let at: usize = name
                .parse()
                .map_err(|_| conflict(key, &Value::List(Vec::new())))?;
            items.get_mut(at).ok_or_else(|| key_not_found(key, &[]))
        }
        (other, _) => Err(conflict(key, other)),
    }
}

fn existing<'v>(current: &'v mut Value, step: &Step, key: &KeyPath) -> Result<&'v mut Value> {
    let found = match (current, step) {
        (Value::Map(fields), Step::Name(name)) => fields.get_mut(name),
        (Value::List(items), Step::Index(index)) => {
            let at = position(items.len(), *index, false);
            at.and_then(|at| items.get_mut(at))
        }
        (Value::List(items), Step::Name(name)) => {
            name.parse::<usize>().ok().and_then(|at| items.get_mut(at))
        }
        (other, _) => return Err(conflict(key, other)),
    };
    found.ok_or_else(|| key_not_found(key, &[]))
}

fn put(current: &mut Value, last: &Step, value: Value, key: &KeyPath) -> Result<()> {
    if current.is_null() {
        *current = match last {
            Step::Name(_) => Value::Map(Fields::new()),
            Step::Index(_) | Step::Append => Value::List(Vec::new()),
        };
    }
    match (current, last) {
        (Value::Map(fields), Step::Name(name)) => {
            fields.insert(name.clone(), value);
        }
        (Value::List(items), Step::Append) => items.push(value),
        (Value::List(items), Step::Index(index)) => {
            let at = position(items.len(), *index, true).ok_or_else(|| key_not_found(key, &[]))?;
            if at == items.len() {
                items.push(value);
            } else {
                items[at] = value;
            }
        }
        (Value::List(items), Step::Name(name)) => {
            let at: usize = name
                .parse()
                .map_err(|_| conflict(key, &Value::List(Vec::new())))?;
            let slot = items.get_mut(at).ok_or_else(|| key_not_found(key, &[]))?;
            *slot = value;
        }
        (other, _) => return Err(conflict(key, other)),
    }
    Ok(())
}

/// Position of `index` in a list of `len` elements; with `allow_end`, the
/// position after the last element is valid too.
fn position(len: usize, index: i64, allow_end: bool) -> Option<usize> {
    let at = if index < 0 {
        len.checked_sub(usize::try_from(index.unsigned_abs()).ok()?)?
    } else {
        usize::try_from(index).ok()?
    };
    (at < len || (allow_end && at == len)).then_some(at)
}

fn conflict(key: &KeyPath, found: &Value) -> nurt_contracts::error::Error {
    let kind = super::value_type(found);
    key_conflict(
        key,
        kind,
        format!("passes through a {kind}; the path needs a map or a list there"),
    )
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use serde_json::json;

    use super::*;
    use crate::model::{FlatSemantics, MapSemantics};

    fn value(json: serde_json::Value) -> Value {
        Value::from_plain_json(json)
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    fn set(root: &mut Value, path: &str, new: Value) -> Result<()> {
        apply_set(root, &key(path), new, &MapSemantics::LAST_WINS)
    }

    fn remove(root: &mut Value, path: &str) -> Result<()> {
        apply_remove(root, &key(path), &MapSemantics::LAST_WINS)
    }

    #[test]
    fn set_creates_maps_and_lists_on_the_way() {
        let mut root = Value::Null;
        set(&mut root, "a.b[0].c", Value::Int(1)).unwrap();
        assert_eq!(root, value(json!({"a": {"b": [{"c": 1}]}})));
    }

    #[test]
    fn set_appends_replaces_and_counts_from_the_end() {
        let mut root = value(json!({"l": [1]}));
        set(&mut root, "l[+]", Value::Int(2)).unwrap();
        set(&mut root, "l[2]", Value::Int(3)).unwrap();
        set(&mut root, "l[-1]", Value::Int(4)).unwrap();
        assert_eq!(root, value(json!({"l": [1, 2, 4]})));
    }

    #[test]
    fn set_beyond_the_end_of_a_list_is_not_found() {
        let mut root = value(json!({"l": [1]}));
        let error = set(&mut root, "l[5]", Value::Int(2)).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyNotFound);
        let error = set(&mut root, "l[-3]", Value::Int(2)).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyNotFound);
        assert_eq!(root, value(json!({"l": [1]})));
    }

    #[test]
    fn set_through_a_scalar_or_a_pattern_conflicts() {
        let mut root = value(json!({"a": 1}));
        let error = set(&mut root, "a.b", Value::Int(2)).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        for pattern in ["a[*]", "**", "a[?x == 1]"] {
            let error = set(&mut root, pattern, Value::Int(2)).unwrap_err();
            assert_eq!(error.code, ErrorCode::KeyConflict, "{pattern}");
        }
    }

    #[test]
    fn flat_semantics_joins_names_into_one_member() {
        let mut root = Value::Null;
        let path = key("server.port");
        apply_set(&mut root, &path, Value::Int(8080), &FlatSemantics).unwrap();
        assert_eq!(root, value(json!({"server.port": 8080})));
    }

    #[test]
    fn document_selector_addresses_an_element_of_the_stream() {
        let mut root = value(json!([{"a": 1}, {"a": 2}]));
        set(&mut root, "#1:a", Value::Int(9)).unwrap();
        assert_eq!(root, value(json!([{"a": 1}, {"a": 9}])));
        let error = set(&mut root, "#5:a", Value::Int(1)).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyNotFound);
        let error = set(&mut root, "#*:a", Value::Int(1)).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        let mut single = value(json!({"a": 1}));
        let error = set(&mut single, "#1:a", Value::Int(1)).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
    }

    #[test]
    fn setting_the_root_replaces_the_document() {
        let mut root = value(json!({"a": 1}));
        apply_set(
            &mut root,
            &KeyPath::root(),
            Value::Int(3),
            &MapSemantics::LAST_WINS,
        )
        .unwrap();
        assert_eq!(root, Value::Int(3));
    }

    #[test]
    fn remove_deletes_members_and_elements() {
        let mut root = value(json!({"a": [1, 2, 3, 4], "b": 1, "c": 2}));
        remove(&mut root, "a[-1]").unwrap();
        remove(&mut root, "a[0]").unwrap();
        remove(&mut root, "a.0").unwrap();
        remove(&mut root, "b").unwrap();
        assert_eq!(root, value(json!({"a": [3], "c": 2})));
    }

    #[test]
    fn remove_reports_missing_and_conflicting_keys() {
        let mut root = value(json!({"a": [1], "b": 1}));
        for path in ["zzz", "a[3]", "a.x", "a[-2]"] {
            let error = remove(&mut root, path).unwrap_err();
            assert_eq!(error.code, ErrorCode::KeyNotFound, "{path}");
        }
        let error = remove(&mut root, "b.c").unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        let error = remove(&mut root, "a[*]").unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        assert_eq!(root, value(json!({"a": [1], "b": 1})));
    }

    #[test]
    fn removing_the_root_leaves_null() {
        let mut root = value(json!({"a": 1}));
        apply_remove(&mut root, &KeyPath::root(), &MapSemantics::LAST_WINS).unwrap();
        assert_eq!(root, Value::Null);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
