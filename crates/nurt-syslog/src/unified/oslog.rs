// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::unified::oslog
// Responsibility:   Backend over the `OSLog` framework.
// Dependencies:     objc2, objc2_foundation, objc2_os_log
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Backend over the `OSLog` framework.
//!
//! The local log store is opened with `OSLogStore.local()`, a log archive
//! with `OSLogStore(url:)`. Entries are enumerated in reverse from the
//! position of `until`, or from the newest entry, with an `NSPredicate`
//! built from the request. Every step of the enumeration runs in its own
//! autorelease pool, so objects returned by the framework are released
//! before the next entry is read.

#![allow(
    unsafe_code,
    reason = "the OSLog framework is a foreign interface of the operating system"
)]

use std::path::{Path, PathBuf};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSDate, NSError, NSPredicate, NSString, NSURL};
use objc2_os_log::{
    OSLogEntryFromProcess, OSLogEntryLog, OSLogEntryLogLevel, OSLogEntryWithPayload,
    OSLogEnumerator, OSLogEnumeratorOptions, OSLogPosition, OSLogStore,
};

use super::backend::{BackendError, BackendItem, EntryRequest, Requirement, UnifiedBackend};
use super::entry::{UnifiedEntry, instant_from_unix_seconds, unix_seconds};
use super::level::EntryLevel;
use super::predicate::Predicate;

/// Log store read by an [`OsLogBackend`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    /// The log store of the running system.
    Local,
    /// A `.logarchive` directory.
    Archive(PathBuf),
}

/// Backend over the `OSLog` framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsLogBackend {
    scope: Scope,
}

impl OsLogBackend {
    /// Backend over the log store of the running system.
    ///
    /// Reading it requires root; see [`Requirement::for_local_store`].
    #[must_use]
    pub const fn local() -> Self {
        Self {
            scope: Scope::Local,
        }
    }

    /// Backend over a `.logarchive` directory.
    #[must_use]
    pub fn archive(path: &Path) -> Self {
        Self {
            scope: Scope::Archive(path.to_path_buf()),
        }
    }

    fn open(&self) -> Result<Retained<OSLogStore>, BackendError> {
        match &self.scope {
            Scope::Local => {
                // SAFETY: the class method takes no arguments; a failure is
                // returned as an `NSError` and never as a null store.
                unsafe { OSLogStore::localStoreAndReturnError() }.map_err(|_| {
                    BackendError::AccessDenied(Requirement::for_local_store(is_root()))
                })
            }
            Scope::Archive(path) => {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                // SAFETY: `url` is a valid file URL that outlives the call; a
                // failure is returned as an `NSError`.
                unsafe { OSLogStore::storeWithURL_error(&url) }.map_err(|error| {
                    BackendError::Unavailable(format!("{}: {}", path.display(), describe(&error)))
                })
            }
        }
    }
}

impl UnifiedBackend for OsLogBackend {
    type Items<'a> = OsLogItems;

    fn items<'a>(&'a self, request: &EntryRequest) -> Result<Self::Items<'a>, BackendError> {
        autoreleasepool(|_| {
            let store = self.open()?;
            let position = request.until.map(|until| {
                let date = NSDate::dateWithTimeIntervalSince1970(unix_seconds(until));
                // SAFETY: `date` is a valid date that outlives the call.
                unsafe { store.positionWithDate(&date) }
            });
            let enumerator = enumerate(&store, position.as_deref(), request.predicate.as_ref())?;
            Ok(OsLogItems {
                enumerator,
                _store: store,
            })
        })
    }
}

/// Entries of the `OSLog` framework, newest first.
#[derive(Debug)]
pub struct OsLogItems {
    enumerator: Retained<OSLogEnumerator>,
    /// The store of the enumerator, kept alive while entries are read.
    _store: Retained<OSLogStore>,
}

// SAFETY: the store and the enumerator are owned by this value only and are
// used by one thread at a time, the thread that holds the value. Reference
// counts of Objective-C objects are atomic, so the objects may be released
// on another thread than the one that created them.
unsafe impl Send for OsLogItems {}

impl Iterator for OsLogItems {
    type Item = Result<BackendItem, BackendError>;

    fn next(&mut self) -> Option<Self::Item> {
        autoreleasepool(|_| {
            let object = self.enumerator.nextObject()?;
            let item = match object.downcast::<OSLogEntryLog>() {
                Ok(log) => entry(&log).map_or(BackendItem::Other, |entry| {
                    BackendItem::Log(Box::new(entry))
                }),
                Err(_) => BackendItem::Other,
            };
            Some(Ok(item))
        })
    }
}

/// Enumerator in reverse order. A store that rejects the condition on the
/// level is queried again without it; the final filter decides the level.
fn enumerate(
    store: &OSLogStore,
    position: Option<&OSLogPosition>,
    predicate: Option<&Predicate>,
) -> Result<Retained<OSLogEnumerator>, BackendError> {
    let options = OSLogEnumeratorOptions::Reverse;
    let full = predicate.map(ns_predicate);
    // SAFETY: the store, the position and the predicate are valid objects
    // that outlive the call; a failure is returned as an `NSError`.
    let result = unsafe {
        store.entriesEnumeratorWithOptions_position_predicate_error(
            options,
            position,
            full.as_deref(),
        )
    };
    match result {
        Ok(enumerator) => Ok(enumerator),
        Err(_) if predicate.is_some_and(Predicate::has_level) => {
            let reduced = predicate
                .and_then(Predicate::without_level)
                .map(|reduced| ns_predicate(&reduced));
            // SAFETY: as above.
            unsafe {
                store.entriesEnumeratorWithOptions_position_predicate_error(
                    options,
                    position,
                    reduced.as_deref(),
                )
            }
            .map_err(|error| failed(&error))
        }
        Err(error) => Err(failed(&error)),
    }
}

fn ns_predicate(predicate: &Predicate) -> Retained<NSPredicate> {
    let format = NSString::from_str(&predicate.format());
    let arguments: Vec<Retained<NSString>> = predicate
        .arguments()
        .into_iter()
        .map(NSString::from_str)
        .collect();
    let objects: Vec<&AnyObject> = arguments
        .iter()
        .map(|argument| AsRef::<AnyObject>::as_ref(&**argument))
        .collect();
    let array = NSArray::from_slice(&objects);
    // SAFETY: the format has one `%@` per element of the array and every
    // element is a string, which is what `%@` compares against.
    unsafe { NSPredicate::predicateWithFormat_argumentArray(&format, Some(&array)) }
}

/// Entry of a log message, or `None` when its date lies outside the range of
/// an instant: such an entry is skipped like an entry of another kind, as the
/// archive reader does, because the epoch would put it in the wrong place of
/// the newest-first order.
fn entry(log: &OSLogEntryLog) -> Option<UnifiedEntry> {
    // SAFETY: every call reads a property of a valid `OSLogEntryLog` and
    // takes no arguments.
    let (date, level, message, process, pid, tid, subsystem, category, sender, activity, format) = unsafe {
        (
            log.date(),
            log.level(),
            log.composedMessage(),
            log.process(),
            log.processIdentifier(),
            log.threadIdentifier(),
            log.subsystem(),
            log.category(),
            log.sender(),
            log.activityIdentifier(),
            log.formatString(),
        )
    };
    let time = instant_from_unix_seconds(date.timeIntervalSince1970())?;
    Some(UnifiedEntry {
        time,
        level: entry_level(level),
        message: message.to_string(),
        process: process.to_string(),
        pid: u64::try_from(pid).ok(),
        tid: Some(tid),
        subsystem: Some(subsystem.to_string()).filter(|text| !text.is_empty()),
        category: Some(category.to_string()).filter(|text| !text.is_empty()),
        sender: Some(sender.to_string()).filter(|text| !text.is_empty()),
        activity_id: (activity != 0).then_some(activity),
        format_string: Some(format.to_string()),
        ..UnifiedEntry::default()
    })
}

fn entry_level(level: OSLogEntryLogLevel) -> EntryLevel {
    EntryLevel::from_raw(i64::try_from(level.0).unwrap_or_default())
}

fn describe(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

fn failed(error: &NSError) -> BackendError {
    BackendError::Failed(describe(error))
}

fn is_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framework_levels_map_to_entry_levels() {
        let cases = [
            (0, EntryLevel::Undefined),
            (1, EntryLevel::Debug),
            (2, EntryLevel::Info),
            (3, EntryLevel::Notice),
            (4, EntryLevel::Error),
            (5, EntryLevel::Fault),
            (99, EntryLevel::Undefined),
        ];
        for (raw, level) in cases {
            assert_eq!(entry_level(OSLogEntryLogLevel(raw)), level, "{raw}");
        }
    }

    #[test]
    fn backends_name_their_scope() {
        assert_eq!(OsLogBackend::local(), OsLogBackend::local());
        let path = Path::new("/tmp/a.logarchive");
        assert_ne!(OsLogBackend::archive(path), OsLogBackend::local());
        assert_eq!(OsLogBackend::archive(path), OsLogBackend::archive(path));
    }

    #[test]
    fn archive_that_does_not_exist_is_unavailable() {
        let backend = OsLogBackend::archive(Path::new("/nonexistent/none.logarchive"));
        let error = backend.items(&EntryRequest::default()).err();
        assert!(matches!(error, Some(BackendError::Unavailable(_))));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
