// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Backend over `.logarchive` directories parsed by `macos-unifiedlogs`.
//!
//! The backend does not use the `OSLog` framework. It reads the `tracev3`
//! files of the archive one at a time, resolves the messages with the string
//! files of the archive and keeps the entries that match the request in an
//! [`ArchiveBuffer`].

use std::io::Read;
use std::path::{Path, PathBuf};

use macos_unifiedlogs::cache::MemoryStringCache;
use macos_unifiedlogs::filesystem::LogarchiveProvider;
use macos_unifiedlogs::iterator::UnifiedLogIterator;
use macos_unifiedlogs::parser::{build_log, collect_timesync};
use macos_unifiedlogs::traits::{FileProvider, SourceFile};
use macos_unifiedlogs::unified_log::{EventType, LogData, LogType};

use super::archive::{ArchiveBuffer, ArchiveKind, ArchiveRecord};
use super::backend::{BackendError, BackendItem, EntryRequest, UnifiedBackend};
use super::level::EntryLevel;

/// Backend over a `.logarchive` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveBackend {
    path: PathBuf,
}

impl ArchiveBackend {
    /// Backend over the archive at `path`.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }
}

impl UnifiedBackend for ArchiveBackend {
    type Items<'a> = std::vec::IntoIter<Result<BackendItem, BackendError>>;

    fn items<'a>(&'a self, request: &EntryRequest) -> Result<Self::Items<'a>, BackendError> {
        if !self.path.is_dir() {
            return Err(BackendError::Unavailable(format!(
                "{} is not a log archive directory",
                self.path.display()
            )));
        }
        let provider = LogarchiveProvider::new(&self.path);
        let cache = MemoryStringCache::default();
        let timesync = collect_timesync(&provider).map_err(|error| {
            BackendError::Failed(format!(
                "the time synchronization files of {} cannot be read: {error:?}",
                self.path.display()
            ))
        })?;
        let mut buffer = ArchiveBuffer::new(request);
        let mut oversize = Vec::new();
        for mut file in provider.tracev3_files() {
            let source = file.source_path().to_owned();
            let mut data = Vec::new();
            let read = file.reader().read_to_end(&mut data);
            read.map_err(|error| BackendError::Failed(format!("{source}: {error}")))?;
            let chunks = UnifiedLogIterator {
                data,
                header: Vec::new(),
                evidence: source,
            };
            for mut chunk in chunks {
                chunk.oversize.append(&mut oversize);
                let (records, _) = build_log(&chunk, &provider, &cache, &timesync, false);
                oversize = chunk.oversize;
                for record in records {
                    buffer.push(archive_record(record));
                }
            }
        }
        Ok(buffer.into_items().into_iter())
    }
}

fn archive_record(data: LogData) -> ArchiveRecord {
    ArchiveRecord {
        kind: Some(kind(data.event_type, data.log_type)),
        time: data.time,
        subsystem: data.subsystem,
        category: data.category,
        process: data.process,
        pid: data.pid,
        thread_id: data.thread_id,
        euid: data.euid,
        library: data.library,
        library_uuid: data.library_uuid,
        process_uuid: data.process_uuid,
        boot_uuid: data.boot_uuid,
        activity_id: data.activity_id,
        message: data.message,
        raw_message: data.raw_message,
    }
}

const fn kind(event: EventType, log: LogType) -> ArchiveKind {
    match (event, log) {
        (EventType::Log, LogType::Debug) => ArchiveKind::Log(EntryLevel::Debug),
        (EventType::Log, LogType::Info) => ArchiveKind::Log(EntryLevel::Info),
        (EventType::Log, LogType::Default) => ArchiveKind::Log(EntryLevel::Notice),
        (EventType::Log, LogType::Error) => ArchiveKind::Log(EntryLevel::Error),
        (EventType::Log, LogType::Fault) => ArchiveKind::Log(EntryLevel::Fault),
        (EventType::Log, _) => ArchiveKind::Log(EntryLevel::Undefined),
        (EventType::Loss, _) => ArchiveKind::Loss,
        _ => ArchiveKind::Other,
    }
}
