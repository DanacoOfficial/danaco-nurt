// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `entry` records from file metadata.
//!
//! [`entry_record`] fills the fields every listing and `info` share; fields
//! that need further reads (link targets, hashes, content details) are set by
//! the command that reads them.

use nurt_contracts::fs::{AbsPath, FileKind, FileMeta, Fs, SpecialKind};
use nurt_contracts::record::{Entry, EntryType, LinkType};
use nurt_contracts::value::{Bytes, Instant, Size};
use unicode_normalization::UnicodeNormalization;

use super::etag::etag_text;
use super::path::path_field;

/// Permission bits shown in `mode`, including the set-id and sticky bits.
const PERMISSION_BITS: u32 = 0o7777;

/// Write permission of the account, the group and others.
const WRITE_BITS: u32 = 0o222;

/// `FILE_ATTRIBUTE_HIDDEN` of Windows.
const WINDOWS_HIDDEN: u32 = 0x2;

/// Names starting with this character are hidden on Unix systems.
const HIDDEN_PREFIX: char = '.';

/// Separator of the last extension.
const EXTENSION_SEPARATOR: char = '.';

/// Type of a record entry for a kind of file system entry.
#[must_use]
pub fn entry_type(kind: FileKind) -> EntryType {
    match kind {
        FileKind::File => EntryType::File,
        FileKind::Directory => EntryType::Directory,
        FileKind::Symlink | FileKind::Junction => EntryType::Link,
        _ => EntryType::Other,
    }
}

/// Sets the type and the device numbers of a named pipe or a device node in
/// `entry`, read from `fs`.
///
/// Entries that are not special files are left as they are, and so is an
/// entry whose kind the volume does not report.
pub fn add_special_node(fs: &dyn Fs, path: &AbsPath, entry: &mut Entry) {
    if entry.kind != EntryType::Other {
        return;
    }
    let Ok(Some(node)) = fs.special_node(path) else {
        return;
    };
    match node.kind {
        SpecialKind::Fifo => entry.kind = EntryType::Fifo,
        _ => {
            entry.kind = EntryType::Device;
            entry.major = Some(u64::from(node.major));
            entry.minor = Some(u64::from(node.minor));
        }
    }
}

/// Kind of a link for a kind of file system entry; `None` for entries that
/// are not links.
#[must_use]
pub fn link_type(kind: FileKind) -> Option<LinkType> {
    match kind {
        FileKind::Symlink => Some(LinkType::Symbolic),
        FileKind::Junction => Some(LinkType::Junction),
        _ => None,
    }
}

/// Permission bits in the octal form of the `mode` field, for example
/// `0644`.
///
/// # Examples
///
/// ```
/// use nurt_files::common::entry::mode_text;
///
/// assert_eq!(mode_text(0o100644), "0644");
/// assert_eq!(mode_text(0o4755), "4755");
/// ```
#[must_use]
pub fn mode_text(mode: u32) -> String {
    format!("{:04o}", mode & PERMISSION_BITS)
}

/// `entry` record of an existing entry.
///
/// Sets the path, the name fields, the type, size, times, mode, the
/// read-only and hidden flags, the kind of a link, the number of hard links
/// and the entity tag. The name is in NFC; `raw_name` holds the stored name
/// when it is not valid Unicode or differs from its NFC form, and `raw_path`
/// and `raw_parent` hold the native bytes of a path that is not valid Unicode.
#[must_use]
pub fn entry_record(path: &AbsPath, meta: &FileMeta) -> Entry {
    let names = Names::of(path, meta.kind == FileKind::Directory);
    let hidden =
        names.name.starts_with(HIDDEN_PREFIX) || meta.windows_attributes & WINDOWS_HIDDEN != 0;
    Entry {
        path: path_field(path),
        raw_path: path.raw_bytes(),
        parent: path.parent().map(|parent| path_field(&parent)),
        raw_parent: path.parent().and_then(|parent| parent.raw_bytes()),
        kind: entry_type(meta.kind),
        size: Size::from_bytes(meta.size),
        modified: Instant::from_unix_nanos(meta.modified_ns),
        created: meta.created_ns.map(Instant::from_unix_nanos),
        accessed: meta.accessed_ns.map(Instant::from_unix_nanos),
        mode: Some(mode_text(meta.mode)),
        readonly: Some(meta.mode & WRITE_BITS == 0),
        hidden: Some(hidden),
        link_type: link_type(meta.kind),
        links: Some(meta.links),
        etag: Some(etag_text(&meta.etag())),
        name: names.name,
        stem: names.stem,
        extension: names.extension,
        raw_name: names.raw,
        ..Entry::default()
    }
}

/// `entry` record of an entry that does not exist, as reported by `info`:
/// `exists: false`, size zero and the Unix epoch as modification time.
#[must_use]
pub fn missing_entry(path: &AbsPath) -> Entry {
    let names = Names::of(path, false);
    Entry {
        path: path_field(path),
        raw_path: path.raw_bytes(),
        parent: path.parent().map(|parent| path_field(&parent)),
        raw_parent: path.parent().and_then(|parent| parent.raw_bytes()),
        exists: Some(false),
        size: Size::from_bytes(0),
        modified: Instant::from_unix_nanos(0),
        name: names.name,
        stem: names.stem,
        extension: names.extension,
        raw_name: names.raw,
        ..Entry::default()
    }
}

/// Name fields of an entry.
struct Names {
    name: String,
    stem: Option<String>,
    extension: Option<String>,
    raw: Option<Bytes>,
}

impl Names {
    fn of(path: &AbsPath, directory: bool) -> Self {
        let Some(stored) = path.file_name() else {
            return Self {
                name: path.to_language_string(),
                stem: None,
                extension: None,
                raw: None,
            };
        };
        let lossy = stored.to_string_lossy();
        let name: String = lossy.nfc().collect();
        let raw = (stored.to_str().is_none() || name != lossy)
            .then(|| Bytes::new(stored.as_encoded_bytes().to_vec()));
        let (stem, extension) = if directory {
            (None, None)
        } else {
            split_extension(&name)
        };
        Self {
            name,
            stem,
            extension,
            raw,
        }
    }
}

/// Stem and last extension of a file name; a leading separator, as in
/// `.profile`, does not start an extension.
fn split_extension(name: &str) -> (Option<String>, Option<String>) {
    match name.rsplit_once(EXTENSION_SEPARATOR) {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (Some(stem.to_owned()), Some(extension.to_owned()))
        }
        _ => (Some(name.to_owned()), None),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{Follow, Fs, SpecialKind, SpecialNode};
    use nurt_testkit::ctx::TestContext;

    use super::*;
    use crate::common::etag::parse_etag;
    use crate::common::fixture::{abs, with_file};

    fn meta_of(test: &TestContext, path: &AbsPath) -> FileMeta {
        test.fs().metadata(path, Follow::NoFollow).unwrap()
    }

    #[test]
    fn file_entries_carry_path_and_name_fields() {
        let (test, path) = with_file("/data/report.final.txt", b"12345");
        let entry = entry_record(&path, &meta_of(&test, &path));
        assert_eq!(entry.path.as_str(), path.to_language_string());
        assert_eq!(entry.name, "report.final.txt");
        assert_eq!(entry.stem.as_deref(), Some("report.final"));
        assert_eq!(entry.extension.as_deref(), Some("txt"));
        assert_eq!(
            entry
                .parent
                .as_ref()
                .map(|parent| parent.as_str().to_owned()),
            Some(abs("/data").to_language_string())
        );
        assert_eq!(entry.raw_name, None);
    }

    #[test]
    fn file_entries_carry_metadata_and_tag() {
        let (test, path) = with_file("/data/report.txt", b"12345");
        test.fs().set_mode(&path, 0o640).unwrap();
        let meta = meta_of(&test, &path);
        let entry = entry_record(&path, &meta);
        assert_eq!(entry.kind, EntryType::File);
        assert_eq!(entry.size, Size::from_bytes(5));
        assert_eq!(entry.modified, Instant::from_unix_nanos(meta.modified_ns));
        assert_eq!(entry.mode.as_deref(), Some("0640"));
        assert_eq!(entry.readonly, Some(false));
        assert_eq!(entry.hidden, Some(false));
        assert_eq!(entry.links, Some(meta.links));
        assert_eq!(entry.link_type, None);
        assert_eq!(entry.exists, None);
        let tag = parse_etag(entry.etag.as_deref().unwrap()).unwrap();
        assert_eq!(tag, meta.etag());
    }

    #[test]
    fn read_only_and_hidden_files_are_flagged() {
        let (test, path) = with_file("/data/.env", b"KEY=1");
        test.fs().set_mode(&path, 0o444).unwrap();
        let entry = entry_record(&path, &meta_of(&test, &path));
        assert_eq!(entry.readonly, Some(true));
        assert_eq!(entry.hidden, Some(true));
        assert_eq!(entry.stem.as_deref(), Some(".env"));
        assert_eq!(entry.extension, None);
    }

    #[test]
    fn windows_hidden_attribute_marks_entries_hidden() {
        let (test, path) = with_file("/data/a.txt", b"");
        let mut meta = meta_of(&test, &path);
        meta.windows_attributes = WINDOWS_HIDDEN;
        assert_eq!(entry_record(&path, &meta).hidden, Some(true));
    }

    #[test]
    fn directories_have_no_extension() {
        let test = TestContext::new();
        let path = abs("/data/archive.d");
        test.fs().create_dir_all(&path).unwrap();
        let entry = entry_record(&path, &meta_of(&test, &path));
        assert_eq!(entry.kind, EntryType::Directory);
        assert_eq!(entry.name, "archive.d");
        assert_eq!(entry.stem, None);
        assert_eq!(entry.extension, None);
    }

    #[test]
    fn links_are_reported_as_links() {
        let (test, target) = with_file("/data/a.txt", b"a");
        let link = abs("/data/link");
        test.fs().symlink(target.as_path(), &link).unwrap();
        let entry = entry_record(&link, &meta_of(&test, &link));
        assert_eq!(entry.kind, EntryType::Link);
        assert_eq!(entry.link_type, Some(LinkType::Symbolic));
        assert_eq!(entry.stem.as_deref(), Some("link"));
    }

    #[test]
    fn decomposed_names_are_normalised_and_kept_raw() {
        let (test, path) = with_file("/data/cafe\u{301}.txt", b"");
        let entry = entry_record(&path, &meta_of(&test, &path));
        assert_eq!(entry.name, "caf\u{e9}.txt");
        assert_eq!(
            entry.raw_name.map(|raw| raw.as_slice().to_vec()),
            Some("cafe\u{301}.txt".as_bytes().to_vec())
        );
    }

    #[cfg(unix)]
    #[test]
    fn names_that_are_not_valid_unicode_keep_their_bytes_in_raw_name() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        use std::path::PathBuf;

        let (test, existing) = with_file("/data/a.txt", b"x");
        let meta = meta_of(&test, &existing);
        let stored = OsString::from_vec(b"/data/z\xff.txt".to_vec());
        let odd = AbsPath::new(PathBuf::from(stored)).unwrap();
        let entry = entry_record(&odd, &meta);
        assert_eq!(entry.name, "z\u{fffd}.txt");
        assert_eq!(entry.stem.as_deref(), Some("z\u{fffd}"));
        assert_eq!(entry.extension.as_deref(), Some("txt"));
        assert_eq!(
            entry.raw_name.map(|raw| raw.as_slice().to_vec()),
            Some(b"z\xff.txt".to_vec())
        );
        let missing = missing_entry(&odd);
        assert_eq!(
            missing.raw_name.map(|raw| raw.as_slice().to_vec()),
            Some(b"z\xff.txt".to_vec())
        );
    }

    #[test]
    fn hidden_names_and_extensions_are_split_at_the_last_period() {
        assert_eq!(
            split_extension("archive.tar.gz"),
            (Some("archive.tar".to_owned()), Some("gz".to_owned()))
        );
        assert_eq!(
            split_extension(".profile"),
            (Some(".profile".to_owned()), None)
        );
        assert_eq!(split_extension("name."), (Some("name.".to_owned()), None));
        assert_eq!(
            split_extension(".env.local"),
            (Some(".env".to_owned()), Some("local".to_owned()))
        );
        assert_eq!(split_extension("plain"), (Some("plain".to_owned()), None));
    }

    #[test]
    fn roots_are_named_by_their_path() {
        let test = TestContext::new();
        let root = abs("/");
        let entry = entry_record(&root, &meta_of(&test, &root));
        assert_eq!(entry.name, root.to_language_string());
        assert_eq!(entry.parent, None);
        assert_eq!(entry.stem, None);
    }

    #[test]
    fn missing_entries_report_absence() {
        let entry = missing_entry(&abs("/data/gone.tar.gz"));
        assert_eq!(entry.exists, Some(false));
        assert_eq!(entry.size, Size::from_bytes(0));
        assert_eq!(entry.modified, Instant::from_unix_nanos(0));
        assert_eq!(entry.stem.as_deref(), Some("gone.tar"));
        assert_eq!(entry.extension.as_deref(), Some("gz"));
        assert_eq!(entry.etag, None);
    }

    #[test]
    fn kinds_map_to_entry_and_link_types() {
        assert_eq!(entry_type(FileKind::Junction), EntryType::Link);
        assert_eq!(entry_type(FileKind::Special), EntryType::Other);
        assert_eq!(link_type(FileKind::Junction), Some(LinkType::Junction));
        assert_eq!(link_type(FileKind::File), None);
        assert_eq!(mode_text(0o40755), "0755");
    }

    #[test]
    fn fifos_and_devices_have_their_own_types_and_numbers() {
        let test = TestContext::new();
        let dir = abs("/dev");
        test.fs().create_dir_all(&dir).unwrap();
        let cases = [
            ("/dev/p", SpecialNode::fifo(), EntryType::Fifo),
            (
                "/dev/c",
                SpecialNode::device(SpecialKind::Char, 1, 3),
                EntryType::Device,
            ),
            (
                "/dev/b",
                SpecialNode::device(SpecialKind::Block, 8, 17),
                EntryType::Device,
            ),
        ];
        for (name, node, expected) in cases {
            let path = abs(name);
            test.fs().create_special_node(&path, node, 0o660).unwrap();
            let mut entry = entry_record(&path, &meta_of(&test, &path));
            assert_eq!(entry.mode.as_deref(), Some("0660"));
            add_special_node(test.fs().as_ref(), &path, &mut entry);
            assert_eq!(entry.kind, expected, "{name}");
            let numbers = (entry.major, entry.minor);
            if expected == EntryType::Device {
                assert_eq!(
                    numbers,
                    (Some(u64::from(node.major)), Some(u64::from(node.minor)))
                );
            } else {
                assert_eq!(numbers, (None, None));
            }
        }
    }

    #[test]
    fn other_special_entries_and_regular_files_have_no_numbers() {
        let test = TestContext::new();
        let path = abs("/data/socket");
        test.fs().create_dir_all(&abs("/data")).unwrap();
        test.fs().create_special(&path).unwrap();
        let mut entry = entry_record(&path, &meta_of(&test, &path));
        assert_eq!(entry.kind, EntryType::Other);
        add_special_node(test.fs().as_ref(), &path, &mut entry);
        assert_eq!(entry.kind, EntryType::Other);
        assert_eq!((entry.major, entry.minor), (None, None));
    }
}
