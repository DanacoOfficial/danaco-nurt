// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Outcome of a replacement with backup name (`ReplaceFileW`).
//!
//! On success the old file lives under the backup name and the replacement
//! under the target name. The replacement inherits the creation time, short
//! name, object identifier, DACL, encryption, compression and the named
//! streams that it lacks from the old file.
//!
//! A failure leaves the names in one of two documented states, reported
//! through [`FsError::op`]:
//!
//! | `op` | Old content | Replacement | Codes |
//! |---|---|---|---|
//! | [`OP_REPLACE`] | under the target name | under its own name | every code except the next row |
//! | [`OP_REPLACE_OLD_AT_BACKUP`] | under the backup name | under its own name, with inherited attributes and streams; the target name is free | `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` |
//!
//! In both states the old content exists under exactly one known name. After a
//! failure in the first state the backup name may hold a partial copy and is
//! removed by the caller before another attempt.

use std::path::Path;

use nurt_contracts::fs::{FsError, FsErrorKind};

use crate::windows::error::codes::{
    ERROR_UNABLE_TO_MOVE_REPLACEMENT, ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
    ERROR_UNABLE_TO_REMOVE_REPLACED,
};
use crate::windows::error::{is_sharing_conflict, os_error};

/// Operation name of a replacement that left every name unchanged.
pub const OP_REPLACE: &str = "replace_with_backup";

/// Operation name of a replacement that moved the old file to the backup name
/// but could not move the replacement to the target name.
pub const OP_REPLACE_OLD_AT_BACKUP: &str = "replace_with_backup.old_at_backup";

/// Location of the old content after a failed replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplaceState {
    /// Old content under the target name; the replacement is unchanged.
    Unchanged,
    /// Old content under the backup name; the target name is free and the
    /// replacement keeps its own name.
    OldAtBackup,
}

impl ReplaceState {
    /// State left by a failure with the Win32 code `code`.
    pub fn from_code(code: u32) -> Self {
        if code == ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 {
            Self::OldAtBackup
        } else {
            Self::Unchanged
        }
    }

    /// Operation name reported in [`FsError::op`].
    pub fn op(self) -> &'static str {
        match self {
            Self::Unchanged => OP_REPLACE,
            Self::OldAtBackup => OP_REPLACE_OLD_AT_BACKUP,
        }
    }

    /// Returns the state reported by an error of a replacement, or `None` for
    /// errors of other operations.
    ///
    /// Replacement errors are those of
    /// [`FsWrite::replace_with_backup`](nurt_contracts::fs::FsWrite::replace_with_backup).
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_fs::windows::error::os_error;
    /// use nurt_fs::windows::replace::ReplaceState;
    ///
    /// let state = ReplaceState::from_code(1177);
    /// let error = os_error(1177, state.op(), None);
    /// assert_eq!(ReplaceState::of(&error), Some(ReplaceState::OldAtBackup));
    /// ```
    pub fn of(error: &FsError) -> Option<Self> {
        match error.op.as_str() {
            OP_REPLACE => Some(Self::Unchanged),
            OP_REPLACE_OLD_AT_BACKUP => Some(Self::OldAtBackup),
            _ => None,
        }
    }
}

/// Whether a failure leaves every name unchanged and may end when another
/// program closes the target or the replacement.
///
/// Besides sharing and lock violations this covers a target that could not be
/// removed and a replacement that could not be renamed: their usual cause is a
/// handle opened without `FILE_SHARE_DELETE`.
pub fn is_retryable(code: u32) -> bool {
    is_sharing_conflict(code)
        || code == ERROR_UNABLE_TO_REMOVE_REPLACED
        || code == ERROR_UNABLE_TO_MOVE_REPLACEMENT
}

/// Error reported for a replacement that failed with `code` after the retry
/// period: [`FsErrorKind::InUse`] for retryable codes, the classification of
/// the code otherwise, and the operation name of the resulting
/// [`ReplaceState`].
pub fn replace_error(code: u32, target: Option<&Path>) -> FsError {
    let mut error = os_error(code, ReplaceState::from_code(code).op(), target);
    if is_retryable(code) {
        error.kind = FsErrorKind::InUse;
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::error::codes::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};

    #[test]
    fn keeps_old_content_at_target_unless_replacement_cannot_move() {
        for code in [
            ERROR_UNABLE_TO_REMOVE_REPLACED,
            ERROR_UNABLE_TO_MOVE_REPLACEMENT,
            ERROR_SHARING_VIOLATION,
            ERROR_ACCESS_DENIED,
        ] {
            assert_eq!(
                ReplaceState::from_code(code),
                ReplaceState::Unchanged,
                "{code}"
            );
        }
    }

    #[test]
    fn reports_old_content_at_backup_when_replacement_cannot_move() {
        let state = ReplaceState::from_code(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2);
        assert_eq!(state, ReplaceState::OldAtBackup);
        let error = os_error(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2, state.op(), None);
        assert_eq!(error.op, OP_REPLACE_OLD_AT_BACKUP);
        assert_eq!(error.kind, FsErrorKind::Io);
        assert_eq!(error.os_code, Some(1177));
    }

    #[test]
    fn reports_locked_names_as_in_use_after_retries() {
        for code in [
            ERROR_SHARING_VIOLATION,
            ERROR_UNABLE_TO_REMOVE_REPLACED,
            ERROR_UNABLE_TO_MOVE_REPLACEMENT,
        ] {
            assert!(is_retryable(code), "{code}");
            let error = replace_error(code, Some(Path::new("target")));
            assert_eq!(error.kind, FsErrorKind::InUse);
            assert_eq!(error.op, OP_REPLACE);
            assert_eq!(error.path.as_deref(), Some(Path::new("target")));
        }
        assert!(!is_retryable(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2));
        let partial = replace_error(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2, None);
        assert_eq!(partial.kind, FsErrorKind::Io);
        assert_eq!(partial.op, OP_REPLACE_OLD_AT_BACKUP);
        let denied = replace_error(ERROR_ACCESS_DENIED, None);
        assert_eq!(denied.kind, FsErrorKind::PermissionDenied);
        assert_eq!(denied.op, OP_REPLACE);
    }

    #[test]
    fn recognises_states_from_errors() {
        let unchanged = os_error(ERROR_SHARING_VIOLATION, OP_REPLACE, None);
        assert_eq!(ReplaceState::of(&unchanged), Some(ReplaceState::Unchanged));
        let other = os_error(ERROR_SHARING_VIOLATION, "rename", None);
        assert_eq!(ReplaceState::of(&other), None);
    }
}
