// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Names created by the back end.

/// Suffix of directory names that Spotlight does not index.
///
/// Spotlight skips every directory whose name ends with `.noindex`, together
/// with its content, so the journal directory of a macOS volume carries this
/// suffix (`.nurt-journal.noindex`). The suffix is part of the stored name:
/// renaming, identity keys and the hidden flag treat it like any other
/// character sequence.
pub const NOINDEX_SUFFIX: &str = ".noindex";

/// Longest name in bytes accepted by APFS and HFS+ for UTF-8 names.
pub const NAME_MAX_BYTES: usize = 255;

/// Name of a temporary file derived from `hint`, unique for the pair of
/// process id and counter.
///
/// The hint is shortened at a character boundary so that the name fits in
/// [`NAME_MAX_BYTES`]. Returns `None` when the hint is empty, is `.` or `..`,
/// or contains `/` or a NUL character.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::temp_name;
///
/// assert_eq!(temp_name(".nurt-tmp-7-1", 0x1f, 2).as_deref(), Some(".nurt-tmp-7-1.1f.2"));
/// assert_eq!(temp_name("a/b", 1, 1), None);
/// ```
#[must_use]
pub fn temp_name(hint: &str, pid: u32, counter: u64) -> Option<String> {
    let invalid = hint.is_empty() || hint == "." || hint == ".." || hint.contains(['/', '\0']);
    if invalid {
        return None;
    }
    let suffix = format!(".{pid:x}.{counter:x}");
    let mut budget = NAME_MAX_BYTES.saturating_sub(suffix.len());
    while !hint.is_char_boundary(budget.min(hint.len())) {
        budget = budget.saturating_sub(1);
    }
    let stem = hint.get(..budget.min(hint.len()))?;
    Some(format!("{stem}{suffix}"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use nurt_contracts::env::Os;
    use nurt_contracts::fs::AbsPath;

    use super::*;
    use crate::key::{name_key, path_key};

    /// Journal directory name used on macOS volumes.
    const JOURNAL_DIR: &str = ".nurt-journal.noindex";

    #[test]
    fn derives_unique_names_from_hint_pid_and_counter() {
        let first = temp_name("data.json", 100, 1).unwrap();
        let second = temp_name("data.json", 100, 2).unwrap();
        let other_process = temp_name("data.json", 101, 1).unwrap();
        assert_eq!(first, "data.json.64.1");
        assert_ne!(first, second);
        assert_ne!(first, other_process);
    }

    #[test]
    fn rejects_hints_that_are_not_single_names() {
        for hint in ["", ".", "..", "dir/name", "nul\0name"] {
            assert_eq!(temp_name(hint, 1, 1), None, "{hint:?}");
        }
    }

    #[test]
    fn shortens_long_hint_at_character_boundary() {
        let hint = "\u{e9}".repeat(200);
        let name = temp_name(&hint, u32::MAX, u64::MAX).unwrap();
        assert!(name.len() <= NAME_MAX_BYTES, "{}", name.len());
        assert!(name.ends_with(".ffffffff.ffffffffffffffff"));
        let stem = name.trim_end_matches(".ffffffff.ffffffffffffffff");
        assert!(stem.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn keeps_noindex_suffix_in_journal_directory_name() {
        assert!(JOURNAL_DIR.ends_with(NOINDEX_SUFFIX));
        let root = AbsPath::for_test("/Volumes/data");
        let journal = root.join_name(JOURNAL_DIR).unwrap();
        assert_eq!(journal.file_name(), Some(OsStr::new(JOURNAL_DIR)));
        let temp = temp_name(JOURNAL_DIR, 1, 1).unwrap();
        assert!(temp.starts_with(JOURNAL_DIR));
        assert!(journal.parent().unwrap().join_name(&temp).is_ok());
    }

    #[test]
    fn keys_noindex_directory_case_insensitively_on_default_volumes() {
        let stored = Path::new("/Volumes/data/.nurt-journal.noindex/blobs");
        let spelled = Path::new("/volumes/DATA/.Nurt-Journal.NOINDEX/Blobs");
        assert_eq!(
            path_key(Os::Macos, false, stored),
            path_key(Os::Macos, false, spelled)
        );
        assert_ne!(
            path_key(Os::Macos, true, stored),
            path_key(Os::Macos, true, spelled)
        );
    }

    #[test]
    fn keys_decomposed_and_composed_names_equally() {
        let composed = OsStr::new("caf\u{e9}.noindex");
        let decomposed = OsStr::new("cafe\u{301}.noindex");
        assert_eq!(
            name_key(Os::Macos, true, composed),
            name_key(Os::Macos, true, decomposed)
        );
        assert_eq!(
            name_key(Os::Macos, false, composed),
            name_key(Os::Macos, false, OsStr::new("CAFE\u{301}.NOINDEX"))
        );
    }
}
