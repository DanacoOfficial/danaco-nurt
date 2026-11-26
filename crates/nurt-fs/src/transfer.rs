// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Metadata transfer onto a file that has just received new content.
//!
//! [`WriteHandle::apply_meta`](nurt_contracts::fs::WriteHandle::apply_meta)
//! and [`FsWrite::restore_meta`](nurt_contracts::fs::FsWrite::restore_meta)
//! transfer every element of the given metadata, including the modification
//! and access times. That is right for an undo or a redo, which put back the
//! state of an earlier moment. A change of content must not do it: the
//! modification time of the new content is the time of the write, and an old
//! value would make `make`, incremental backups and `rsync` skip the file.

use nurt_contracts::fs::{ExtendedMeta, FileMeta};

/// Metadata of the replaced file for transfer onto a file with new content.
///
/// The result equals `previous` except for the modification and access
/// times, which are those of `written`, the metadata that the file with the
/// new content reported after its content was written. Account, group,
/// permission bits, extended attributes, flags, ACL and the creation time of
/// `previous` stay.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::{ExtendedMeta, FileId, FileKind, FileMeta};
/// use nurt_fs::transfer::for_content_change;
///
/// let file = |modified_ns| FileMeta {
///     kind: FileKind::File,
///     size: 0,
///     mode: 0o640,
///     uid: Some(1000),
///     gid: Some(1000),
///     links: 1,
///     modified_ns,
///     accessed_ns: Some(modified_ns),
///     created_ns: None,
///     changed_ns: None,
///     file_id: FileId { volume: 1, low: 2, high: 0 },
///     windows_attributes: 0,
/// };
/// let previous = ExtendedMeta {
///     basic: file(1),
///     xattrs: Vec::new(),
///     acl: None,
///     account_sid: None,
///     flags: None,
///     streams: Vec::new(),
///     link_target: None,
/// };
/// let prepared = for_content_change(&previous, &file(2));
/// assert_eq!(prepared.basic.modified_ns, 2);
/// assert_eq!(prepared.basic.mode, 0o640);
/// ```
#[must_use]
pub fn for_content_change(previous: &ExtendedMeta, written: &FileMeta) -> ExtendedMeta {
    let mut meta = previous.clone();
    meta.basic.modified_ns = written.modified_ns;
    meta.basic.accessed_ns = written.accessed_ns;
    meta
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{FileId, FileKind, Xattr};

    use super::*;

    fn file(mode: u32, modified_ns: i64, accessed_ns: Option<i64>) -> FileMeta {
        FileMeta {
            kind: FileKind::File,
            size: 3,
            mode,
            uid: Some(1000),
            gid: Some(100),
            links: 1,
            modified_ns,
            accessed_ns,
            created_ns: Some(7),
            changed_ns: Some(8),
            file_id: FileId {
                volume: 1,
                low: 2,
                high: 0,
            },
            windows_attributes: 0,
        }
    }

    #[test]
    fn takes_the_times_of_the_written_file_and_keeps_everything_else() {
        let previous = ExtendedMeta {
            basic: file(0o640, 10, Some(11)),
            xattrs: vec![Xattr {
                name: "user.origin".into(),
                value: b"test".to_vec(),
            }],
            acl: Some(vec![1, 2]),
            account_sid: Some("S-1-5-18".to_owned()),
            flags: Some(0x10),
            streams: Vec::new(),
            link_target: None,
        };
        let written = file(0o600, 20, None);
        let prepared = for_content_change(&previous, &written);
        assert_eq!(prepared.basic.modified_ns, 20);
        assert_eq!(prepared.basic.accessed_ns, None);
        let mut expected = previous.clone();
        expected.basic.modified_ns = 20;
        expected.basic.accessed_ns = None;
        assert_eq!(prepared, expected);
        assert_eq!(prepared.basic.mode, 0o640);
    }
}
