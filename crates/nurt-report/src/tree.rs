// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Drawing of the tree view of records of kind `entry` that carry a depth.

use nurt_contracts::value::{Record, Value};

use crate::facts::{field_str, value_text};

const DEPTH_FIELD: &str = "depth";
const NAME_FIELD: &str = "name";
const PATH_FIELD: &str = "path";
const SIZE_FIELD: &str = "size";
const TYPE_FIELD: &str = "type";
const DIRECTORY: &str = "directory";
const BRANCH: &str = "|-- ";
const LAST_BRANCH: &str = "`-- ";
const GUIDE: &str = "|   ";
const BLANK_GUIDE: &str = "    ";

/// Depth of a row, when the row carries one.
fn depth_of(row: &Record) -> Option<usize> {
    match row.get(DEPTH_FIELD)? {
        Value::Int(depth) => usize::try_from(*depth).ok(),
        _ => None,
    }
}

/// Whether every row has a depth, so that the rows form a tree.
pub(crate) fn is_tree(rows: &[Record]) -> bool {
    !rows.is_empty() && rows.iter().all(|row| depth_of(row).is_some())
}

fn label(row: &Record) -> String {
    let name = field_str(row, NAME_FIELD)
        .or_else(|| field_str(row, PATH_FIELD))
        .unwrap_or_default();
    let mut text = name.to_owned();
    if field_str(row, TYPE_FIELD) == Some(DIRECTORY) {
        text.push('/');
    } else if let Some(size) = row.get(SIZE_FIELD) {
        text.push_str("  ");
        text.push_str(&value_text(size));
    }
    text
}

fn is_last_sibling(rows: &[Record], index: usize, depth: usize) -> bool {
    rows[index + 1..]
        .iter()
        .filter_map(depth_of)
        .find(|next| *next <= depth)
        .is_none_or(|next| next < depth)
}

/// Draws the rows as an indented tree; the rows must satisfy [`is_tree`].
///
/// A row of depth 0 is a root and is written without a connector. A row of
/// depth `d` is a child of the nearest preceding row of depth `d - 1`.
pub(crate) fn draw(rows: &[Record]) -> Vec<String> {
    let mut lines = Vec::with_capacity(rows.len());
    let mut ancestors_last: Vec<bool> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let depth = depth_of(row).unwrap_or(0);
        if depth == 0 {
            ancestors_last.clear();
            lines.push(label(row));
            continue;
        }
        ancestors_last.truncate(depth - 1);
        let last = is_last_sibling(rows, index, depth);
        let guides: String = ancestors_last
            .iter()
            .map(|ancestor_last| if *ancestor_last { BLANK_GUIDE } else { GUIDE })
            .collect();
        let branch = if last { LAST_BRANCH } else { BRANCH };
        lines.push(format!("{guides}{branch}{}", label(row)));
        ancestors_last.push(last);
    }
    lines
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::{RecordKind, Size};

    use super::*;

    fn entry(name: &str, depth: i64, directory: bool, size: u64) -> Record {
        let mut record = Record::new(RecordKind::Entry)
            .with(NAME_FIELD, Value::Text(name.to_owned()))
            .with(DEPTH_FIELD, Value::Int(depth))
            .with(SIZE_FIELD, Value::Size(Size::from_bytes(size)));
        if directory {
            record.set(TYPE_FIELD, Value::Enum(DIRECTORY.to_owned()));
        }
        record
    }

    #[test]
    fn requires_a_depth_in_every_row() {
        let with = entry("a", 0, true, 0);
        let without = Record::new(RecordKind::Entry).with(NAME_FIELD, Value::Text("b".to_owned()));
        assert!(is_tree(std::slice::from_ref(&with)));
        assert!(!is_tree(&[with, without]));
        assert!(!is_tree(&[]));
    }

    #[test]
    fn draws_nested_directories_with_guides() {
        let rows = [
            entry("app", 0, true, 0),
            entry("src", 1, true, 0),
            entry("a.go", 2, false, 1127),
            entry("b.go", 2, false, 8192),
            entry("README", 1, false, 12),
        ];
        assert_eq!(
            draw(&rows),
            [
                "app/",
                "|-- src/",
                "|   |-- a.go  1.1kb",
                "|   `-- b.go  8.2kb",
                "`-- README  12b",
            ]
        );
    }

    #[test]
    fn leaves_no_guide_below_a_last_sibling() {
        let rows = [
            entry("root", 0, true, 0),
            entry("only", 1, true, 0),
            entry("leaf", 2, false, 1),
        ];
        assert_eq!(draw(&rows), ["root/", "`-- only/", "    `-- leaf  1b"]);
    }

    #[test]
    fn writes_each_root_without_a_connector() {
        let rows = [entry("one", 0, true, 0), entry("two", 0, true, 0)];
        assert_eq!(draw(&rows), ["one/", "two/"]);
    }
}
