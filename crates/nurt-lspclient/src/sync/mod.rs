// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-lspclient::sync
// Responsibility:   Documents open in a language server and their agreement with the disk.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Documents open in a language server and their agreement with the disk.
//!
//! Before each use of a server the documents opened earlier are compared
//! with the disk by modification time and size: a changed file is sent again
//! with `textDocument/didChange`, a removed or unreadable file is closed with
//! `textDocument/didClose`. Content checked before it is written (diagnostics
//! of text in memory) has no disk stamp and is compared with the disk at the
//! next use.
//!
//! # Examples
//!
//! ```
//! use std::path::Path;
//! use nurt_lspclient::sync::DocumentSet;
//!
//! let mut documents = DocumentSet::new();
//! # let file = if cfg!(windows) { "C:/project/main.py" } else { "/project/main.py" };
//! let path = Path::new(file);
//! assert!(documents.sync(path, "python", "x = 1\n", None).is_some());
//! assert!(documents.sync(path, "python", "x = 1\n", None).is_none());
//! assert_eq!(documents.version(path), Some(1));
//! ```

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::json;

use crate::jsonrpc::Message;

/// Directories never searched for candidate files.
pub const SKIPPED_DIRECTORIES: &[&str] = &[".git", "node_modules", "__pycache__", "venv", ".venv"];

/// State of a file on disk at the time of synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    /// Modification time.
    pub modified: Option<SystemTime>,
    /// Size in bytes.
    pub len: u64,
}

/// Stamp of the file at `path`; `None` when it does not exist.
#[must_use]
pub fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    meta.is_file().then(|| FileStamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

/// Text of a source file; `None` when the file is missing, larger than
/// `max_bytes`, binary (holds a NUL byte) or not UTF-8.
#[must_use]
pub fn read_text(path: &Path, max_bytes: u64) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > max_bytes {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).ok()? > max_bytes || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// `file:` URI of an absolute path.
#[must_use]
pub fn path_to_uri(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

/// Path of a `file:` URI; `None` for other schemes.
#[must_use]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let parsed = url::Url::parse(uri).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }
    parsed.to_file_path().ok()
}

#[derive(Debug, Clone)]
struct OpenDocument {
    path: PathBuf,
    language_id: String,
    version: i64,
    text: String,
    stamp: Option<FileStamp>,
}

/// Document known to the server, as reported by [`DocumentSet::documents`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentState {
    /// File.
    pub path: PathBuf,
    /// Language identifier sent with `didOpen`.
    pub language_id: String,
    /// Stamp of the file at the last synchronization; `None` for content
    /// from memory.
    pub stamp: Option<FileStamp>,
}

/// Documents open in one server, by URI.
#[derive(Debug, Default)]
pub struct DocumentSet {
    documents: HashMap<String, OpenDocument>,
}

impl DocumentSet {
    /// Set without documents.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the document or sends new content. Returns the notification to
    /// send, `None` when the server already has this content.
    pub fn sync(
        &mut self,
        path: &Path,
        language_id: &str,
        text: &str,
        stamp: Option<FileStamp>,
    ) -> Option<Message> {
        let uri = path_to_uri(path)?;
        if let Some(known) = self.documents.get_mut(&uri) {
            known.stamp = stamp;
            if known.text == text {
                return None;
            }
            known.version += 1;
            text.clone_into(&mut known.text);
            return Some(Message::notification(
                "textDocument/didChange",
                json!({
                    "textDocument": {"uri": uri, "version": known.version},
                    "contentChanges": [{"text": text}],
                }),
            ));
        }
        let notification = Message::notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {"uri": uri, "languageId": language_id, "version": 1, "text": text},
            }),
        );
        self.documents.insert(
            uri,
            OpenDocument {
                path: path.to_path_buf(),
                language_id: language_id.to_owned(),
                version: 1,
                text: text.to_owned(),
                stamp,
            },
        );
        Some(notification)
    }

    /// Closes the document; the server returns to the content on disk.
    pub fn forget(&mut self, path: &Path) -> Option<Message> {
        let uri = path_to_uri(path)?;
        self.documents.remove(&uri)?;
        Some(Message::notification(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        ))
    }

    /// Version of an open document.
    #[must_use]
    pub fn version(&self, path: &Path) -> Option<i64> {
        let uri = path_to_uri(path)?;
        self.documents.get(&uri).map(|document| document.version)
    }

    /// Returns `true` when the document is open.
    #[must_use]
    pub fn is_open(&self, path: &Path) -> bool {
        self.version(path).is_some()
    }

    /// Open documents.
    #[must_use]
    pub fn documents(&self) -> Vec<DocumentState> {
        let mut documents: Vec<DocumentState> = self
            .documents
            .values()
            .map(|document| DocumentState {
                path: document.path.clone(),
                language_id: document.language_id.clone(),
                stamp: document.stamp,
            })
            .collect();
        documents.sort_by(|left, right| left.path.cmp(&right.path));
        documents
    }

    /// Compares every open document with the disk and returns the
    /// notifications that bring the server up to date.
    pub fn refresh(&mut self, max_bytes: u64) -> Vec<Message> {
        let mut notifications = Vec::new();
        for document in self.documents() {
            let stamp = file_stamp(&document.path);
            if stamp.is_some() && stamp == document.stamp {
                continue;
            }
            let text = stamp.and_then(|_| read_text(&document.path, max_bytes));
            let notification = match text {
                Some(text) => self.sync(&document.path, &document.language_id, &text, stamp),
                None => self.forget(&document.path),
            };
            notifications.extend(notification);
        }
        notifications
    }
}

/// Limits of [`candidate_files`].
#[derive(Debug, Clone, Copy)]
pub struct CandidateLimits {
    /// Largest number of files returned.
    pub max_files: usize,
    /// Largest number of files examined.
    pub max_scanned: usize,
    /// Largest file examined, in bytes.
    pub max_bytes: u64,
}

/// Files under `root` whose name passes `accept` and whose text contains the
/// identifier `word` as a whole word, in path order, skipping
/// [`SKIPPED_DIRECTORIES`].
#[must_use]
pub fn candidate_files(
    root: &Path,
    word: &str,
    accept: impl Fn(&Path) -> bool,
    limits: CandidateLimits,
) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut scanned = 0usize;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        let mut directories = Vec::new();
        for entry in entries {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let skipped = SKIPPED_DIRECTORIES
                    .iter()
                    .any(|name| entry.file_name() == *name);
                if !skipped {
                    directories.push(path);
                }
                continue;
            }
            if !kind.is_file() || !accept(&path) {
                continue;
            }
            scanned += 1;
            if scanned > limits.max_scanned || found.len() >= limits.max_files {
                return found;
            }
            if read_text(&path, limits.max_bytes).is_some_and(|text| contains_word(&text, word)) {
                found.push(path);
            }
        }
        pending.extend(directories.into_iter().rev());
    }
    found
}

/// Returns `true` when `text` holds `word` not preceded or followed by an
/// identifier character.
#[must_use]
pub fn contains_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let is_ident =
        |character: char| character.is_alphanumeric() || character == '_' || character == '$';
    text.match_indices(word).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method_of(message: &Message) -> &str {
        match message {
            Message::Notification { method, .. } => method,
            other => panic!("expected a notification, found {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn uri_round_trips_names_with_reserved_and_multi_byte_characters() {
        let path = Path::new("/project/a b#c%d?e/za\u{17c}\u{f3}\u{142}\u{107}.rs");
        let uri = path_to_uri(path).expect("uri");
        assert!(
            uri.starts_with("file:///project/a%20b%23c%25d%3Fe/"),
            "{uri}"
        );
        assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
    }

    #[cfg(unix)]
    #[test]
    fn uri_round_trips_names_that_are_not_valid_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/project/n\xff\xfe.rs"));
        let uri = path_to_uri(&path).expect("uri");
        assert!(uri.ends_with("n%FF%FE.rs"), "{uri}");
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[cfg(windows)]
    #[test]
    fn uri_of_a_drive_path_has_three_slashes_and_round_trips() {
        let path = Path::new(r"C:\project\a b.rs");
        let uri = path_to_uri(path).expect("uri");
        assert_eq!(uri, "file:///C:/project/a%20b.rs");
        assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
    }

    #[test]
    fn uri_conversion_refuses_relative_paths_and_other_schemes() {
        assert_eq!(path_to_uri(Path::new("relative/main.py")), None);
        assert_eq!(uri_to_path("https://example.com/main.py"), None);
        assert_eq!(uri_to_path("not a uri"), None);
    }

    #[test]
    fn sync_sends_changes_with_increasing_versions_and_keeps_multi_byte_text() {
        let mut documents = DocumentSet::new();
        let path = Path::new(if cfg!(windows) {
            r"C:\p\main.py"
        } else {
            "/p/main.py"
        });
        let text = "x = '\u{1f600}za\u{17c}\u{f3}\u{142}\u{107}'\r\n";
        let open = documents.sync(path, "python", text, None).expect("open");
        assert_eq!(method_of(&open), "textDocument/didOpen");
        assert!(documents.sync(path, "python", text, None).is_none());
        let change = documents
            .sync(path, "python", "y = 2\n", None)
            .expect("change");
        assert_eq!(method_of(&change), "textDocument/didChange");
        assert_eq!(documents.version(path), Some(2));
        assert!(documents.is_open(path));
        let close = documents.forget(path).expect("close");
        assert_eq!(method_of(&close), "textDocument/didClose");
        assert!(documents.forget(path).is_none());
        assert!(!documents.is_open(path));
    }

    #[test]
    fn refresh_sends_changes_and_closes_removed_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("main.py");
        fs::write(&path, "a = 1\n").expect("file");
        let mut documents = DocumentSet::new();
        documents.sync(&path, "python", "a = 1\n", file_stamp(&path));
        assert!(documents.refresh(1024).is_empty());
        fs::write(&path, "a = 12345\n").expect("file");
        let notifications = documents.refresh(1024);
        assert_eq!(notifications.len(), 1);
        assert_eq!(method_of(&notifications[0]), "textDocument/didChange");
        fs::remove_file(&path).expect("remove");
        let notifications = documents.refresh(1024);
        assert_eq!(notifications.len(), 1);
        assert_eq!(method_of(&notifications[0]), "textDocument/didClose");
        assert!(documents.documents().is_empty());
    }

    #[test]
    fn read_text_refuses_large_binary_and_non_utf8_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let write = |name: &str, bytes: &[u8]| {
            let path = directory.path().join(name);
            fs::write(&path, bytes).expect("file");
            path
        };
        assert_eq!(read_text(&write("a", b"abcd"), 4).as_deref(), Some("abcd"));
        assert_eq!(read_text(&write("b", b"abcde"), 4), None);
        assert_eq!(read_text(&write("c", b"a\0b"), 10), None);
        assert_eq!(read_text(&write("d", b"\xff\xfe"), 10), None);
        assert_eq!(read_text(&directory.path().join("missing"), 10), None);
        assert_eq!(
            read_text(&write("e", "\u{17c}\u{f3}".as_bytes()), 10).as_deref(),
            Some("\u{17c}\u{f3}")
        );
    }

    #[test]
    fn whole_word_search_respects_identifier_characters() {
        assert!(contains_word("call(foo)", "foo"));
        assert!(contains_word("foo", "foo"));
        assert!(!contains_word("foo_bar", "foo"));
        assert!(!contains_word("$foo", "foo"));
        assert!(!contains_word("\u{17c}foo", "foo"));
        assert!(contains_word("x \u{1f600}foo", "foo"));
        assert!(!contains_word("anything", ""));
    }

    #[test]
    fn candidate_files_skip_listed_directories_and_partial_words() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path();
        fs::create_dir_all(root.join("node_modules")).expect("directory");
        fs::create_dir_all(root.join("sub")).expect("directory");
        fs::write(root.join("a.py"), "target = 1\n").expect("file");
        fs::write(root.join("node_modules").join("b.py"), "target\n").expect("file");
        fs::write(root.join("sub").join("c.py"), "target_x\n").expect("file");
        fs::write(root.join("sub").join("d.py"), "use target\n").expect("file");
        fs::write(root.join("e.txt"), "target\n").expect("file");
        let limits = CandidateLimits {
            max_files: 10,
            max_scanned: 100,
            max_bytes: 1024,
        };
        let found = candidate_files(
            root,
            "target",
            |path| path.extension().is_some_and(|extension| extension == "py"),
            limits,
        );
        assert_eq!(
            found,
            vec![root.join("a.py"), root.join("sub").join("d.py")]
        );
        let limited = CandidateLimits {
            max_files: 1,
            ..limits
        };
        assert_eq!(
            candidate_files(root, "target", |_| true, limited),
            vec![root.join("a.py")]
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
