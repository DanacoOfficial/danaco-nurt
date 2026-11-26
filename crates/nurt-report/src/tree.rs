// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Drawing of the tree view of records of kind `entry` that carry a depth.

use nurt_contracts::value::{Record, RecordKind, Value};

use crate::facts::{field_str, value_text};
use crate::lines::single_line;

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
    let mut text = single_line(name);
    if field_str(row, TYPE_FIELD) == Some(DIRECTORY) {
        text.push('/');
    } else if let Some(size) = row.get(SIZE_FIELD) {
        text.push_str("  ");
        text.push_str(&value_text(size));
    }
    text
}

/// Path without its last `up` components; `None` when the path has fewer
/// components or the rest is empty.
fn ancestor_path(path: &str, up: usize) -> Option<&str> {
    let mut ancestor = path;
    for _ in 0..up {
        ancestor = ancestor.rsplit_once('/')?.0;
    }
    (!ancestor.is_empty()).then_some(ancestor)
}

/// Row of a directory that the listing passes through without listing it.
fn directory_row(path: &str, depth: usize) -> Record {
    let name = path.rsplit('/').next().unwrap_or(path);
    Record::new(RecordKind::Entry)
        .with(NAME_FIELD, Value::Text(name.to_owned()))
        .with(PATH_FIELD, Value::Path(path.to_owned()))
        .with(
            DEPTH_FIELD,
            Value::Int(i64::try_from(depth).unwrap_or(i64::MAX)),
        )
        .with(TYPE_FIELD, Value::Enum(DIRECTORY.to_owned()))
}

/// Adds to `completed` the ancestors of the row at `path` that are not among
/// the `open` directories, the paths of the rows above the current position,
/// one for each depth from 1.
fn add_missing_ancestors(
    path: &str,
    depth: usize,
    open: &mut Vec<String>,
    completed: &mut Vec<Record>,
) {
    for level in 1..depth {
        let Some(ancestor) = ancestor_path(path, depth - level) else {
            break;
        };
        if open.get(level - 1).is_some_and(|known| known == ancestor) {
            continue;
        }
        open.truncate(level - 1);
        open.push(ancestor.to_owned());
        completed.push(directory_row(ancestor, level));
    }
}

/// Adds before each row the directories between the root of the listing and
/// the row that the rows do not list, as when only the matches of a search
/// are listed.
///
/// A row of depth `d` lies `d` components below the root, so its ancestors
/// are the `d - 1` directories named by the leading components of the rows
/// `path` that follow the root. A row without a path adds nothing.
fn with_ancestors(rows: &[Record]) -> Vec<Record> {
    let mut completed = Vec::with_capacity(rows.len());
    let mut open: Vec<String> = Vec::new();
    for row in rows {
        let depth = depth_of(row).unwrap_or(0);
        let path = field_str(row, PATH_FIELD).map(|path| path.trim_end_matches('/'));
        if let Some(path) = path {
            add_missing_ancestors(path, depth, &mut open, &mut completed);
        }
        let parents = depth.saturating_sub(1);
        open.truncate(parents);
        open.resize(parents, String::new());
        if depth > 0 {
            open.push(path.unwrap_or_default().to_owned());
        }
        completed.push(row.clone());
    }
    completed
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
/// depth `d` is a child of the nearest preceding row of depth `d - 1`; the
/// directories that rows with a `path` pass through without listing them are
/// drawn too.
pub(crate) fn draw(rows: &[Record]) -> Vec<String> {
    let rows = with_ancestors(rows);
    let rows = rows.as_slice();
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
                "|   |-- a.go  1.1kib",
                "|   `-- b.go  8.0kib",
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

    fn found(path: &str, depth: i64, directory: bool, size: u64) -> Record {
        let name = path.rsplit('/').next().unwrap_or(path);
        entry(name, depth, directory, size).with(PATH_FIELD, Value::Path(path.to_owned()))
    }

    #[test]
    fn draws_the_directories_that_listed_matches_pass_through() {
        let rows = [
            found("/srv/app/src/a.go", 2, false, 1127),
            found("/srv/app/src/sub/b.go", 3, false, 8192),
        ];
        assert_eq!(
            draw(&rows),
            [
                "`-- src/",
                "    |-- a.go  1.1kib",
                "    `-- sub/",
                "        `-- b.go  8.0kib",
            ]
        );
    }

    #[test]
    fn draws_matches_of_different_directories_under_each_directory() {
        let rows = [
            found("/srv/app/lib/x/a.go", 3, false, 1),
            found("/srv/app/lib/x/b.go", 3, false, 2),
            found("/srv/app/src/c.go", 2, false, 3),
        ];
        assert_eq!(
            draw(&rows),
            [
                "|-- lib/",
                "|   `-- x/",
                "|       |-- a.go  1b",
                "|       `-- b.go  2b",
                "`-- src/",
                "    `-- c.go  3b",
            ]
        );
    }

    #[test]
    fn does_not_repeat_a_directory_that_the_rows_list() {
        let rows = [
            found("/srv/app/src", 1, true, 0),
            found("/srv/app/src/a.go", 2, false, 1),
        ];
        assert_eq!(draw(&rows), ["`-- src/", "    `-- a.go  1b"]);
    }

    #[test]
    fn adds_no_directory_to_rows_without_a_path() {
        let rows = [entry("a.go", 2, false, 1)];
        assert_eq!(draw(&rows), ["`-- a.go  1b"]);
    }

    #[test]
    fn writes_a_name_with_a_line_break_on_one_line() {
        let rows = [entry("root", 0, true, 0), entry("a\nb", 1, false, 1)];
        assert_eq!(draw(&rows), ["root/", "`-- a\\nb  1b"]);
    }

    #[test]
    fn writes_each_root_without_a_connector() {
        let rows = [entry("one", 0, true, 0), entry("two", 0, true, 0)];
        assert_eq!(draw(&rows), ["one/", "two/"]);
    }
}
