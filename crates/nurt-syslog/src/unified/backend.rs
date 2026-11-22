// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface between the reader and a log store of the unified log.
//!
//! [`UnifiedBackend`] is the only part of the reader that calls the operating
//! system or reads log files. Tests implement it over recorded entries.

use nurt_contracts::value::Instant;

use super::entry::UnifiedEntry;
use super::predicate::Predicate;

/// Permission that reading the log store requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// The command must run as root.
    Root,
    /// The terminal application must have Full Disk Access.
    FullDiskAccess,
}

impl Requirement {
    /// Permission missing when the local log store cannot be opened.
    ///
    /// Opening the local store requires root. A process that already runs
    /// as root and is still refused lacks Full Disk Access.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_syslog::unified::backend::Requirement;
    /// assert_eq!(Requirement::for_local_store(false), Requirement::Root);
    /// assert_eq!(Requirement::for_local_store(true), Requirement::FullDiskAccess);
    /// ```
    #[must_use]
    pub const fn for_local_store(is_root: bool) -> Self {
        if is_root {
            Self::FullDiskAccess
        } else {
            Self::Root
        }
    }

    /// Value of the detail `required` of `E_PRIVILEGE`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::FullDiskAccess => "full_disk_access",
        }
    }
}

/// Failure of the log store reported by a [`UnifiedBackend`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    /// The account may not read the log store.
    #[error("access to the log store is denied")]
    AccessDenied(Requirement),
    /// The log store cannot be opened.
    #[error("the log store is not available: {0}")]
    Unavailable(String),
    /// Reading the log store failed.
    #[error("reading the log store failed: {0}")]
    Failed(String),
}

/// Item of the log store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendItem {
    /// A log message.
    Log(Box<UnifiedEntry>),
    /// The system lost the given number of messages.
    Lost(u64),
    /// An activity, a signpost or another item that is not a log message.
    Other,
}

/// Entries requested from the log store.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryRequest {
    /// Predicate evaluated by the log store.
    pub predicate: Option<Predicate>,
    /// Position to start from, exclusive; the newest entry when absent.
    pub until: Option<Instant>,
}

/// Log store of the unified log.
///
/// Implementations are shared by the stream of a query, which may be moved
/// to another thread; they are therefore `Sync`.
pub trait UnifiedBackend: Sync {
    /// Items of the log store, newest first.
    type Items<'a>: Iterator<Item = Result<BackendItem, BackendError>> + Send
    where
        Self: 'a;

    /// Items of the log store that match a request, newest first.
    ///
    /// # Errors
    ///
    /// [`BackendError::AccessDenied`] when the account may not read the log
    /// store, other [`BackendError`] values when it cannot be opened.
    fn items<'a>(&'a self, request: &EntryRequest) -> Result<Self::Items<'a>, BackendError>;
}
