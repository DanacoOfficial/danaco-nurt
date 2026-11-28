// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bench::snapshot
// Responsibility:   File trees in the format `nurt-fs-snapshot/1`.
// Dependencies:     base64, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! File trees in the format `nurt-fs-snapshot/1`.
//!
//! A snapshot lists entries with paths relative to a root directory. The
//! runner materializes the initial tree of a task in the task root and
//! captures the tree before and after the run for the oracle.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{BenchError, Result};

/// Format identifier of a snapshot.
pub const SNAPSHOT_FORMAT: &str = "nurt-fs-snapshot/1";

/// Permission bits of a file without an explicit mode.
pub const DEFAULT_FILE_MODE: u32 = 0o644;

/// Permission bits of a directory without an explicit mode.
pub const DEFAULT_DIR_MODE: u32 = 0o755;

/// Type of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symbolic link.
    Symlink,
}

/// One entry of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Path relative to the root, segments separated by `/`.
    pub path: String,
    /// Type of the entry.
    #[serde(rename = "type")]
    pub kind: EntryKind,
    /// Content of a file as UTF-8 text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_utf8: Option<String>,
    /// Content of a file in standard Base64 with padding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    /// Permission bits in octal, for example `"0755"`; Unix only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Modification time, RFC 3339 in UTC with whole seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<String>,
    /// Target of a symbolic link as stored in the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// Snapshot of a directory tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// Format identifier, [`SNAPSHOT_FORMAT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Whether the snapshot describes the whole tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exhaustive: Option<bool>,
    /// Entries of the tree.
    pub entries: Vec<Entry>,
}

/// Decoded node of a tree, keyed by its relative path in [`Tree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Type of the node.
    pub kind: EntryKind,
    /// Content of a file.
    pub content: Vec<u8>,
    /// Permission bits.
    pub mode: Option<u32>,
    /// Target of a symbolic link.
    pub target: Option<String>,
}

/// Decoded tree: relative paths in byte order mapped to nodes.
pub type Tree = BTreeMap<String, Node>;

/// Checks that a path is relative, uses `/` and has no `.`, `..` or empty
/// segments.
///
/// # Errors
///
/// Returns a message describing the defect of the path.
pub fn check_path(path: &str) -> std::result::Result<(), String> {
    let valid = !path.is_empty()
        && !path.contains('\\')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    if valid {
        Ok(())
    } else {
        Err(format!(
            "path {path:?} must be relative, separated by '/', without '.', '..' or empty segments"
        ))
    }
}

fn parse_mode(text: &str) -> std::result::Result<u32, String> {
    let digits = text.strip_prefix('0').unwrap_or(text);
    let octal = digits.bytes().all(|byte| (b'0'..=b'7').contains(&byte));
    if text.len() < 4 || text.len() > 5 || !text.starts_with('0') || !octal {
        return Err(format!(
            "mode {text:?} must be an octal string such as \"0644\""
        ));
    }
    u32::from_str_radix(digits, 8).map_err(|_| format!("mode {text:?} is not octal"))
}

impl Entry {
    /// Decodes the entry into a node.
    ///
    /// # Errors
    ///
    /// Returns a message when the path, the mode or the content is invalid or
    /// when the members do not fit the type.
    pub fn to_node(&self) -> std::result::Result<Node, String> {
        check_path(&self.path)?;
        let mode = self.mode.as_deref().map(parse_mode).transpose()?;
        let content = match (self.kind, &self.content_utf8, &self.content_base64) {
            (EntryKind::File, Some(text), None) => text.as_bytes().to_vec(),
            (EntryKind::File, None, Some(encoded)) => STANDARD
                .decode(encoded)
                .map_err(|error| format!("content of {:?} is not Base64: {error}", self.path))?,
            (EntryKind::File, _, _) => {
                return Err(format!(
                    "file {:?} needs exactly one of content_utf8 and content_base64",
                    self.path
                ));
            }
            (_, None, None) => Vec::new(),
            _ => {
                return Err(format!(
                    "{:?} is not a file and cannot have content",
                    self.path
                ));
            }
        };
        let target = match (self.kind, &self.target) {
            (EntryKind::Symlink, Some(target)) => Some(target.clone()),
            (EntryKind::Symlink, None) => {
                return Err(format!("symbolic link {:?} needs a target", self.path));
            }
            (_, None) => None,
            (_, Some(_)) => return Err(format!("{:?} is not a symbolic link", self.path)),
        };
        Ok(Node {
            kind: self.kind,
            content,
            mode,
            target,
        })
    }

    /// Encodes a node as an entry; text content is stored as UTF-8 and other
    /// content as Base64.
    #[must_use]
    pub fn from_node(path: &str, node: &Node) -> Self {
        let (content_utf8, content_base64) = match node.kind {
            EntryKind::File => match std::str::from_utf8(&node.content) {
                Ok(text) => (Some(text.to_owned()), None),
                Err(_) => (None, Some(STANDARD.encode(&node.content))),
            },
            EntryKind::Dir | EntryKind::Symlink => (None, None),
        };
        Self {
            path: path.to_owned(),
            kind: node.kind,
            content_utf8,
            content_base64,
            mode: node.mode.map(|mode| format!("{mode:04o}")),
            mtime: None,
            target: node.target.clone(),
        }
    }
}

impl Snapshot {
    /// Reads a snapshot from a JSON value.
    ///
    /// # Errors
    ///
    /// Returns a message when the value is not a snapshot.
    pub fn from_value(value: &Value) -> std::result::Result<Self, String> {
        let snapshot: Self =
            serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        match snapshot.format.as_deref() {
            None | Some(SNAPSHOT_FORMAT) => Ok(snapshot),
            Some(other) => Err(format!("format {other:?} is not {SNAPSHOT_FORMAT}")),
        }
    }

    /// Decodes the entries into a tree with implicit parent directories.
    ///
    /// # Errors
    ///
    /// Returns a message for the first invalid or duplicated entry.
    pub fn to_tree(&self) -> std::result::Result<Tree, String> {
        let mut tree = Tree::new();
        for entry in &self.entries {
            if tree.insert(entry.path.clone(), entry.to_node()?).is_some() {
                return Err(format!("path {:?} is listed twice", entry.path));
            }
        }
        Ok(with_parents(tree))
    }

    /// Builds the snapshot of a tree as a capture lists it: files, symbolic
    /// links and empty directories, sorted by path in byte order.
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Self {
        let entries = tree
            .iter()
            .filter(|(path, node)| node.kind != EntryKind::Dir || !has_children(tree, path))
            .map(|(path, node)| Entry::from_node(path, node))
            .collect();
        Self {
            format: Some(SNAPSHOT_FORMAT.to_owned()),
            exhaustive: None,
            entries,
        }
    }
}

fn has_children(tree: &Tree, path: &str) -> bool {
    let prefix = format!("{path}/");
    tree.range(prefix.clone()..)
        .next()
        .is_some_and(|(next, _)| next.starts_with(&prefix))
}

fn directory_node() -> Node {
    Node {
        kind: EntryKind::Dir,
        content: Vec::new(),
        mode: None,
        target: None,
    }
}

/// Adds the parent directories that the entries of a tree imply.
#[must_use]
pub fn with_parents(mut tree: Tree) -> Tree {
    let paths: Vec<String> = tree.keys().cloned().collect();
    for path in paths {
        let mut end = 0;
        while let Some(position) = path[end..].find('/') {
            end += position;
            tree.entry(path[..end].to_owned())
                .or_insert_with(directory_node);
            end += 1;
        }
    }
    tree
}

/// Resolves the `fs_before` member of a task: a snapshot, or a reference
/// `{"fixture": name, "overlay": [entries]}` to `<fixtures>/<name>.json`.
///
/// # Errors
///
/// Returns an error when the fixture cannot be read or an entry is invalid.
pub fn resolve_fs_before(value: &Value, fixtures: &Path) -> Result<Tree> {
    let Some(name) = value.get("fixture") else {
        let snapshot = Snapshot::from_value(value)
            .map_err(|message| BenchError::format(fixtures, "fs_before", message))?;
        return snapshot
            .to_tree()
            .map_err(|message| BenchError::format(fixtures, "fs_before", message));
    };
    let name = name
        .as_str()
        .ok_or_else(|| BenchError::format(fixtures, "fs_before", "fixture must be a text"))?;
    check_path(name).map_err(|message| BenchError::format(fixtures, "fs_before", message))?;
    let path = fixtures.join(format!("{name}.json"));
    let snapshot = Snapshot::from_value(&read_json(&path)?)
        .map_err(|message| BenchError::format(&path, "fixture", message))?;
    let mut tree = snapshot
        .to_tree()
        .map_err(|message| BenchError::format(&path, "fixture", message))?;
    let overlay = value
        .get("overlay")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let entries: Vec<Entry> = serde_json::from_value(overlay)
        .map_err(|error| BenchError::format(&path, "overlay", error.to_string()))?;
    for entry in entries {
        let node = entry
            .to_node()
            .map_err(|message| BenchError::format(&path, "overlay", message))?;
        tree.insert(entry.path, node);
    }
    Ok(with_parents(tree))
}

/// Reads a JSON file.
///
/// # Errors
///
/// Returns an error when the file cannot be read or is not JSON.
pub fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).map_err(|error| BenchError::io(path, error))?;
    serde_json::from_str(&text).map_err(|error| BenchError::format(path, "JSON", error.to_string()))
}

/// Creates the nodes of a tree below `root`: directories first, then files
/// and links. Modes are applied on Unix only.
///
/// # Errors
///
/// Returns an error when an entry cannot be created.
pub fn materialize(tree: &Tree, root: &Path) -> Result<()> {
    for (path, node) in tree {
        let target = root.join(path);
        match node.kind {
            EntryKind::Dir => fs::create_dir_all(&target),
            EntryKind::File => fs::write(&target, &node.content),
            EntryKind::Symlink => {
                create_symlink(node.target.as_deref().unwrap_or_default(), &target)
            }
        }
        .map_err(|error| BenchError::io(&target, error))?;
    }
    for (path, node) in tree.iter().rev() {
        let default = match node.kind {
            EntryKind::File => DEFAULT_FILE_MODE,
            EntryKind::Dir => DEFAULT_DIR_MODE,
            EntryKind::Symlink => continue,
        };
        let target = root.join(path);
        set_mode(&target, node.mode.unwrap_or(default))
            .map_err(|error| BenchError::io(&target, error))?;
    }
    Ok(())
}

/// Captures the tree below `root` without following symbolic links.
///
/// # Errors
///
/// Returns an error when a directory or a file cannot be read.
pub fn capture(root: &Path) -> Result<Tree> {
    let mut tree = Tree::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        let listing =
            fs::read_dir(&directory).map_err(|error| BenchError::io(&directory, error))?;
        for item in listing {
            let item = item.map_err(|error| BenchError::io(&directory, error))?;
            let path = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let meta = fs::symlink_metadata(&path).map_err(|error| BenchError::io(&path, error))?;
            let node = if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).map_err(|error| BenchError::io(&path, error))?;
                Node {
                    kind: EntryKind::Symlink,
                    content: Vec::new(),
                    mode: None,
                    target: Some(target.to_string_lossy().replace('\\', "/")),
                }
            } else if meta.is_dir() {
                pending.push((path.clone(), relative.clone()));
                Node {
                    mode: mode_of(&meta),
                    ..directory_node()
                }
            } else if meta.is_file() {
                Node {
                    kind: EntryKind::File,
                    content: fs::read(&path).map_err(|error| BenchError::io(&path, error))?,
                    mode: mode_of(&meta),
                    target: None,
                }
            } else {
                // A named pipe, a socket or a device is not part of a
                // snapshot, and reading a pipe would never end.
                continue;
            };
            tree.insert(relative, node);
        }
    }
    Ok(tree)
}

#[cfg(unix)]
fn create_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    let resolved = link
        .parent()
        .map_or_else(|| target.into(), |parent| parent.join(target));
    if resolved.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_meta: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "format": "nurt-fs-snapshot/1",
            "entries": [
                {"path": "src/main.py", "type": "file", "content_utf8": "print(1)\n"},
                {"path": "tools/run.sh", "type": "file", "content_utf8": "#!/bin/sh\n", "mode": "0755"},
                {"path": "data/blob.bin", "type": "file", "content_base64": "/wA="},
                {"path": "empty", "type": "dir"},
                {"path": "latest", "type": "symlink", "target": "src"}
            ]
        })
    }

    #[test]
    fn decodes_entries_and_adds_parent_directories() {
        let tree = Snapshot::from_value(&sample()).unwrap().to_tree().unwrap();
        assert_eq!(tree["data/blob.bin"].content, vec![0xFF, 0]);
        assert_eq!(tree["tools/run.sh"].mode, Some(0o755));
        assert_eq!(tree["src"].kind, EntryKind::Dir);
        assert_eq!(tree["latest"].target.as_deref(), Some("src"));
    }

    #[test]
    fn rejects_invalid_paths() {
        for path in ["", "/abs", "a/../b", "a//b", "a\\b", "./a"] {
            assert!(check_path(path).is_err(), "{path:?} accepted");
        }
        assert!(check_path("a/b.txt").is_ok());
    }

    #[test]
    fn rejects_file_with_two_contents_and_directory_with_content() {
        let both = json!({"entries": [
            {"path": "a", "type": "file", "content_utf8": "x", "content_base64": "eA=="}
        ]});
        assert!(Snapshot::from_value(&both).unwrap().to_tree().is_err());
        let dir = json!({"entries": [{"path": "d", "type": "dir", "content_utf8": "x"}]});
        assert!(Snapshot::from_value(&dir).unwrap().to_tree().is_err());
        let link = json!({"entries": [{"path": "l", "type": "symlink"}]});
        assert!(Snapshot::from_value(&link).unwrap().to_tree().is_err());
    }

    #[test]
    fn rejects_bad_mode_duplicates_and_foreign_format() {
        let mode = json!({"entries": [{"path": "a", "type": "dir", "mode": "755"}]});
        assert!(Snapshot::from_value(&mode).unwrap().to_tree().is_err());
        let twice = json!({"entries": [
            {"path": "a", "type": "dir"}, {"path": "a", "type": "dir"}
        ]});
        assert!(Snapshot::from_value(&twice).unwrap().to_tree().is_err());
        let foreign = json!({"format": "other/1", "entries": []});
        assert!(Snapshot::from_value(&foreign).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn capture_returns_the_materialized_tree() {
        let root = tempfile::tempdir().unwrap();
        let tree = Snapshot::from_value(&sample()).unwrap().to_tree().unwrap();
        materialize(&tree, root.path()).unwrap();
        let captured = capture(root.path()).unwrap();
        assert_eq!(
            captured.keys().collect::<Vec<_>>(),
            tree.keys().collect::<Vec<_>>()
        );
        assert_eq!(captured["tools/run.sh"].mode, Some(0o755));
        assert_eq!(captured["src/main.py"].mode, Some(DEFAULT_FILE_MODE));
        assert_eq!(captured["data/blob.bin"].content, vec![0xFF, 0]);
        assert_eq!(captured["latest"].target.as_deref(), Some("src"));
    }

    #[test]
    fn modes_with_signs_or_non_octal_digits_are_rejected() {
        for mode in ["0+64", "0-44", "0 64", "0789", "00 6"] {
            let entry = json!({"entries": [{"path": "a", "type": "dir", "mode": mode}]});
            assert!(
                Snapshot::from_value(&entry).unwrap().to_tree().is_err(),
                "{mode:?} accepted"
            );
        }
        let ok = json!({"entries": [{"path": "a", "type": "dir", "mode": "01777"}]});
        let tree = Snapshot::from_value(&ok).unwrap().to_tree().unwrap();
        assert_eq!(tree["a"].mode, Some(0o1777));
    }

    #[cfg(unix)]
    #[test]
    fn capture_skips_named_pipes_instead_of_blocking_on_them() {
        let root = tempfile::tempdir().unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(root.path().join("pipe"))
            .status();
        if !made.is_ok_and(|status| status.success()) {
            return;
        }
        fs::write(root.path().join("a.txt"), "a").unwrap();
        let captured = capture(root.path()).unwrap();
        assert_eq!(captured.keys().collect::<Vec<_>>(), ["a.txt"]);
    }

    #[test]
    fn captured_snapshot_lists_only_leaves() {
        let tree = Snapshot::from_value(&sample()).unwrap().to_tree().unwrap();
        let snapshot = Snapshot::from_tree(&tree);
        let paths: Vec<&str> = snapshot.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "data/blob.bin",
                "empty",
                "latest",
                "src/main.py",
                "tools/run.sh"
            ]
        );
        assert_eq!(snapshot.entries[0].content_base64.as_deref(), Some("/wA="));
        assert_eq!(Snapshot::from_tree(&snapshot.to_tree().unwrap()), snapshot);
    }

    #[test]
    fn resolves_fixture_with_overlay() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("base.json"),
            r#"{"format":"nurt-fs-snapshot/1","entries":[{"path":"a.txt","type":"file","content_utf8":"old"}]}"#,
        )
        .unwrap();
        let reference = json!({"fixture": "base", "overlay": [
            {"path": "a.txt", "type": "file", "content_utf8": "new"},
            {"path": "d/b.txt", "type": "file", "content_utf8": "b"}
        ]});
        let tree = resolve_fs_before(&reference, dir.path()).unwrap();
        assert_eq!(tree["a.txt"].content, b"new");
        assert_eq!(tree["d"].kind, EntryKind::Dir);
        let missing = json!({"fixture": "absent"});
        assert!(resolve_fs_before(&missing, dir.path()).is_err());
        let inline = resolve_fs_before(&sample(), dir.path()).unwrap();
        assert!(inline.contains_key("empty"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
