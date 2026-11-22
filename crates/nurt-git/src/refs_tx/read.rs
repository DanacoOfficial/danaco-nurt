// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading loose reference files and `packed-refs`.

use std::io::ErrorKind;
use std::path::Path;

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_ref::{FullName, Target};
use nurt_contracts::error::Result;

use super::{io_error, ref_error};

/// Header Git writes at the top of `packed-refs`.
pub(super) const PACKED_HEADER: &[u8] = b"# pack-refs with: peeled fully-peeled sorted \n";

const SYMREF_PREFIX: &[u8] = b"ref:";

/// State of a path that may hold a loose reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Loose {
    /// Nothing at the path.
    Absent,
    /// A file with its raw content.
    File(Vec<u8>),
    /// A directory (references below the name).
    Dir,
}

/// Reads the entry at `path` without following a final symbolic link.
pub(super) fn read_loose(path: &Path) -> Result<Loose> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => Ok(Loose::Dir),
        Ok(_) => std::fs::read(path)
            .map(Loose::File)
            .map_err(|error| io_error(path, &error)),
        Err(error) if is_missing(&error) => Ok(Loose::Absent),
        Err(error) => Err(io_error(path, &error)),
    }
}

/// Whether an error means that the path does not exist.
pub(super) fn is_missing(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

/// Parses the content of a loose reference file.
pub(super) fn parse_loose(content: &[u8], name: &BStr) -> Result<Target> {
    let text = content.trim();
    if let Some(rest) = text.strip_prefix(SYMREF_PREFIX) {
        let target = rest.trim();
        return FullName::try_from(target.as_bstr())
            .map(Target::Symbolic)
            .map_err(|_| corrupt(name));
    }
    ObjectId::from_hex(text)
        .map(Target::Object)
        .map_err(|_| corrupt(name))
}

/// Content of a loose reference file holding `target`.
pub(super) fn loose_content(target: &Target) -> Vec<u8> {
    match target {
        Target::Object(id) => format!("{}\n", id.to_hex()).into_bytes(),
        Target::Symbolic(name) => {
            let mut out = b"ref: ".to_vec();
            out.extend_from_slice(name.as_bstr());
            out.push(b'\n');
            out
        }
    }
}

fn corrupt(name: &BStr) -> nurt_contracts::error::Error {
    ref_error(
        "corrupt",
        name,
        format!("reference {name} has unreadable content; repair or delete the reference file"),
    )
}

/// One reference of `packed-refs` with the byte range of its lines.
#[derive(Debug, Clone)]
pub(super) struct PackedEntry {
    pub(super) name: BString,
    pub(super) id: ObjectId,
    start: usize,
    end: usize,
}

/// Content of `packed-refs`.
#[derive(Debug, Clone, Default)]
pub(super) struct Packed {
    /// Raw content; `None` when the file does not exist.
    pub(super) raw: Option<Vec<u8>>,
    pub(super) entries: Vec<PackedEntry>,
}

impl Packed {
    /// Reads and parses `packed-refs`.
    pub(super) fn read(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(raw) => Self::parse(raw),
            Err(error) if is_missing(&error) => Ok(Self::default()),
            Err(error) => Err(io_error(path, &error)),
        }
    }

    fn parse(raw: Vec<u8>) -> Result<Self> {
        let mut entries: Vec<PackedEntry> = Vec::new();
        let mut offset = 0;
        for line in raw.lines_with_terminator() {
            let start = offset;
            offset += line.len();
            let text = line.trim_end();
            if text.is_empty() || text.starts_with(b"#") {
                continue;
            }
            if text.starts_with(b"^") {
                if let Some(last) = entries.last_mut() {
                    last.end = offset;
                }
                continue;
            }
            let (id, name) = text
                .split_once_str(" ")
                .ok_or_else(|| corrupt("packed-refs".into()))?;
            let id = ObjectId::from_hex(id).map_err(|_| corrupt(name.as_bstr()))?;
            entries.push(PackedEntry {
                name: name.into(),
                id,
                start,
                end: offset,
            });
        }
        Ok(Self {
            raw: Some(raw),
            entries,
        })
    }

    /// Value of `name`, when packed.
    pub(super) fn find(&self, name: &BStr) -> Option<ObjectId> {
        self.entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.id)
    }

    /// Content without the given references; the Git header is written when
    /// the file had none.
    pub(super) fn without(&self, names: &[BString]) -> Vec<u8> {
        let raw = self.raw.as_deref().unwrap_or_default();
        let mut out = Vec::with_capacity(raw.len());
        if !raw.starts_with(b"#") {
            out.extend_from_slice(PACKED_HEADER);
        }
        let mut copied = 0;
        for entry in self.entries.iter().filter(|e| names.contains(&e.name)) {
            out.extend_from_slice(&raw[copied..entry.start]);
            copied = entry.end;
        }
        out.extend_from_slice(&raw[copied..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "1111111111111111111111111111111111111111";
    const TWO: &str = "2222222222222222222222222222222222222222";

    fn packed(text: &str) -> Packed {
        Packed::parse(text.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn removing_a_packed_ref_drops_its_peeled_line() {
        let text = format!(
            "# pack-refs with: peeled fully-peeled sorted \n{ONE} refs/heads/a\n{TWO} refs/tags/v1\n^{ONE}\n{ONE} refs/tags/v2\n"
        );
        let packed = packed(&text);
        let out = packed.without(&["refs/tags/v1".into()]);
        assert_eq!(
            out.as_bstr(),
            format!(
                "# pack-refs with: peeled fully-peeled sorted \n{ONE} refs/heads/a\n{ONE} refs/tags/v2\n"
            )
        );
        assert_eq!(
            packed.find("refs/tags/v2".into()),
            ObjectId::from_hex(ONE.as_bytes()).ok()
        );
    }

    #[test]
    fn header_is_added_to_a_file_without_one() {
        let packed = packed(&format!("{ONE} refs/heads/a\n{TWO} refs/heads/b\n"));
        let out = packed.without(&["refs/heads/a".into()]);
        assert_eq!(
            out.as_bstr(),
            format!("{}{TWO} refs/heads/b\n", PACKED_HEADER.as_bstr())
        );
    }

    #[test]
    fn malformed_line_is_reported_as_corrupt() {
        let error = Packed::parse(b"zz refs/heads/a\n".to_vec()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("corrupt"));
        let error = Packed::parse(b"nospace\n".to_vec()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("corrupt"));
    }

    #[test]
    fn loose_content_round_trips() {
        let id = Target::Object(ObjectId::from_hex(ONE.as_bytes()).unwrap());
        let sym = Target::Symbolic(FullName::try_from("refs/heads/main").unwrap());
        for target in [id, sym] {
            let content = loose_content(&target);
            assert_eq!(parse_loose(&content, "x".into()).unwrap(), target);
        }
        assert!(parse_loose(b"ref: \n", "x".into()).is_err());
        assert!(parse_loose(b"xyz\n", "x".into()).is_err());
    }
}
