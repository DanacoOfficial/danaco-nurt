// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::ini::support::flat
// Responsibility:   Key handling of flat documents (property and environment files) and scalar text of values.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Key handling of flat documents (property and environment files) and
//! scalar text of values.

use std::ops::Range;

use nurt_contracts::error::Error;
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::Value;

use super::errors::key_conflict;
use super::text::{Line, Splice};
use super::tree::{Map, Node};

/// Key path of a flat document: consecutive names are joined with `.` into
/// one name, so `server.port` and `["server.port"]` address the same key.
pub(crate) fn flat_key(key: &KeyPath) -> Result<KeyPath, Error> {
    let mut segments: Vec<KeySegment> = Vec::new();
    for segment in key.segments() {
        match (segments.last_mut(), segment) {
            (Some(KeySegment::Key(previous)), KeySegment::Key(name)) => {
                previous.push('.');
                previous.push_str(name);
            }
            _ => segments.push(segment.clone()),
        }
    }
    KeyPath::new(key.document(), segments)
        .map_err(|error| key_conflict(&error.to_string(), "key path", &[]))
}

/// Name addressed by a flat key path that names exactly one key.
pub(crate) fn single_name(key: &KeyPath) -> Option<String> {
    match key.segments() {
        [KeySegment::Key(name)] => Some(name.clone()),
        _ => None,
    }
}

/// Key tree of a flat document: one member per name, referring to the last
/// entry with that name, in the order of first occurrence.
pub(crate) fn flat_tree<'a>(names: impl Iterator<Item = &'a str>) -> Node {
    let mut map = Map::new(false);
    for (id, name) in names.enumerate() {
        *map.entry(name, || Node::Leaf(id)) = Node::Leaf(id);
    }
    Node::Map(map)
}

/// Text written for a scalar value; `None` for lists, maps and records.
pub(crate) fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text)
        | Value::BigInt(text)
        | Value::Decimal(text)
        | Value::Key(text)
        | Value::Enum(text)
        | Value::Glob(text) => Some(text.clone()),
        Value::Int(int) => Some(int.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null => Some(String::new()),
        Value::Date(date) => Some(date.text.clone()),
        Value::Path(path) => Some(path.as_str().to_owned()),
        _ => None,
    }
}

/// Byte range removed with the lines `first..=last`: the lines and the
/// comment lines directly above them. At the end of a document without a
/// final line break the preceding line break is removed instead.
pub(crate) fn block_range(
    lines: &[Line],
    comment_lines: &[bool],
    mut first: usize,
    last: usize,
    floor: usize,
) -> Range<usize> {
    while first > 0 && comment_lines[first - 1] {
        first -= 1;
    }
    let end = lines[last];
    let mut start = lines[first].start.max(floor);
    if last + 1 == lines.len() && end.end == end.content_end && first > 0 {
        start = lines[first - 1].content_end;
    }
    start..end.end
}

/// Removal of the given line blocks, with overlapping blocks merged.
pub(crate) fn removals(mut ranges: Vec<Range<usize>>) -> Vec<Splice> {
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged.into_iter().map(|r| Splice::new(r, "")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::ini::support::text::lines;

    #[test]
    fn joins_names_into_one_flat_key() {
        let key = flat_key(&KeyPath::parse("server.port").unwrap()).unwrap();
        assert_eq!(single_name(&key).as_deref(), Some("server.port"));
        let quoted = flat_key(&KeyPath::parse("[\"a.b\"].c").unwrap()).unwrap();
        assert_eq!(single_name(&quoted).as_deref(), Some("a.b.c"));
        assert_eq!(
            single_name(&flat_key(&KeyPath::parse("a[*]").unwrap()).unwrap()),
            None
        );
    }

    #[test]
    fn the_last_duplicate_wins() {
        let Node::Map(map) = flat_tree(["a", "b", "a"].into_iter()) else {
            panic!("map expected");
        };
        assert_eq!(map.members.len(), 2);
        assert_eq!(map.get("a"), Some(&Node::Leaf(2)));
    }

    #[test]
    fn removes_blocks_with_their_comments() {
        let text = "# c\na=1\nb=2";
        let table = lines(text);
        let comments = [true, false, false];
        assert_eq!(block_range(&table, &comments, 1, 1, 0), 0..8);
        assert_eq!(block_range(&table, &comments, 2, 2, 0), 7..11);
        let merged = removals(vec![4..8, 0..5, 9..10]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].range, 0..8);
    }

    #[test]
    fn writes_scalars_as_text() {
        assert_eq!(scalar_text(&Value::Int(5)).as_deref(), Some("5"));
        assert_eq!(scalar_text(&Value::Bool(true)).as_deref(), Some("true"));
        assert_eq!(scalar_text(&Value::Null).as_deref(), Some(""));
        assert_eq!(scalar_text(&Value::List(Vec::new())), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
