// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Log store over systemd journal files.
//!
//! Each file yields its entries newest first. Equality conditions use the
//! hash table: `unit` and `source` become sets of DATA objects, the
//! condition with the fewest entries leads the iteration over its entry
//! lists, and the other conditions are checked on the items of each entry
//! without reading payloads. The level threshold excludes entries that
//! reference `PRIORITY=k` above it. Only accepted entries are decoded, and
//! every decoded event passes the final filter, so the result equals a
//! later `filter` on the same fields.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, PoisonError};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::Instant;

use crate::cursor::{Cursor, Id128};
use crate::event::{LogEvent, PRIORITY_DEBUG};
use crate::filter::EventFilter;
use crate::limits::LogLimits;
use crate::merge::{Merge, MergeItem, Source as MergeSource};
use crate::query::{LogNotice, LogQuery, LogStore, LogStream, NameMatch, NoticeCode, Notices};

use super::Fault;
use super::discover::{self, Candidate, Discovery};
use super::file::JournalFile;
use super::header::Header;
use super::index::EntryIndex;
use super::mapping::{self, EXE_FIELD, Field, Origin, SOURCE_FIELDS, UNIT_FIELDS, UNKNOWN_SOURCE};
use super::object::{DataObject, EntryObject};
use super::reader::Source;

const NANOS_PER_MICRO: i64 = 1_000;
const MAX_SUGGESTIONS: usize = 8;
const NAME_VALUE_BYTES: u64 = 512;

/// Store of the systemd journal under a system root.
///
/// ```no_run
/// use nurt_syslog::journald::JournaldStore;
/// use nurt_syslog::limits::LogLimits;
/// use nurt_syslog::query::{LogQuery, LogStore};
///
/// let store = JournaldStore::new("/", LogLimits::default());
/// let query = LogQuery::default();
/// let messages = store
///     .query(&query)?
///     .map(|event| event.map(|event| event.message))
///     .collect::<Result<Vec<_>, _>>()?;
/// assert!(messages.len() <= 100);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct JournaldStore {
    root: PathBuf,
    files: Option<Vec<PathBuf>>,
    limits: LogLimits,
}

impl JournaldStore {
    /// Store of the journal of the machine whose root directory is `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, limits: LogLimits) -> Self {
        Self {
            root: root.into(),
            files: None,
            limits,
        }
    }

    /// Store of exactly the given journal files, of any machine.
    #[must_use]
    pub fn from_files(files: Vec<PathBuf>, limits: LogLimits) -> Self {
        Self {
            root: PathBuf::new(),
            files: Some(files),
            limits,
        }
    }

    /// Store of the journal of the running system.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn local(limits: LogLimits) -> Self {
        Self::new("/", limits)
    }
}

impl LogStore for JournaldStore {
    fn query<'a>(&'a self, query: &'a LogQuery) -> Result<LogStream<'a>, Error> {
        let discovery = match &self.files {
            Some(files) => discover::from_paths(files),
            None => discover::discover(&self.root)?,
        };
        let shared = Arc::new(Shared::new(query, &self.limits)?);
        let candidates = select(discovery, &shared)?;
        let max_open = self.limits.max_open_files.max(1);
        let sources = candidates
            .into_iter()
            .map(|candidate| {
                let shared = Arc::clone(&shared);
                let bound = micros_to_instant(candidate.header.tail_entry_realtime);
                MergeSource::lazy(bound, move || open_path(candidate.path, shared))
            })
            .collect();
        Ok(finish(Merge::new(sources, max_open), shared))
    }
}

/// Reads one journal file held in memory.
///
/// A defect of the file ends the stream with the notice `log_file_skipped`.
///
/// # Errors
///
/// Returns the errors of [`LogQuery`] validation that depend on the store:
/// `E_INVALID_VALUE` for a cursor of another store or a malformed unit.
pub fn query_bytes<'a>(
    data: Vec<u8>,
    query: &'a LogQuery,
    limits: &LogLimits,
) -> Result<LogStream<'a>, Error> {
    let shared = Arc::new(Shared::new(query, limits)?);
    let stream = FileStream::open(
        Source::Memory(data),
        "<memory>".to_owned(),
        Arc::clone(&shared),
    );
    let merge = Merge::new(vec![MergeSource::eager(stream)], 1);
    Ok(finish(merge, shared))
}

fn finish<'a>(merge: Merge<'a, JournalItem>, shared: Arc<Shared<'a>>) -> LogStream<'a> {
    let notices = shared.notices.clone();
    let limit = shared.query.limit;
    let mut done = false;
    let events = merge
        .map(|item| item.map(|item| item.event))
        .chain(std::iter::from_fn(move || {
            if !done {
                done = true;
                shared.report_unknown_names();
            }
            None
        }));
    LogStream::new(events, notices, limit)
}

fn select(discovery: Discovery, shared: &Shared<'_>) -> Result<Vec<Candidate>, Error> {
    let Discovery {
        candidates,
        denied,
        skipped,
    } = discovery;
    if candidates.is_empty() && denied > 0 {
        return Err(Error::new(
            ErrorCode::Permission,
            "no journal file is readable; add the account to the group `systemd-journal` or `adm`, or run as root",
        )
        .with_detail("files", denied.into()));
    }
    if candidates.is_empty() && !skipped.is_empty() {
        let files: Vec<_> = skipped
            .iter()
            .map(|(path, detail)| serde_json::json!({ "path": path, "detail": detail }))
            .collect();
        return Err(Error::new(
            ErrorCode::Format,
            "no journal file has a readable format; the files are damaged or written by a newer systemd",
        )
        .with_detail("files", files.into()));
    }
    if denied > 0 {
        let mut notice = LogNotice::new(NoticeCode::PermissionDeniedSkipped);
        notice.count = Some(denied);
        shared.notices.add(notice);
    }
    for (path, detail) in skipped {
        shared.skip(path, detail);
    }
    if let Some(Cursor::Journal { seqnum_id, .. }) = &shared.query.cursor
        && !candidates
            .iter()
            .any(|candidate| candidate.header.seqnum_id == *seqnum_id)
    {
        shared
            .notices
            .add(LogNotice::new(NoticeCode::CursorApproximate));
    }
    Ok(candidates
        .into_iter()
        .filter(|candidate| shared.may_contain(&candidate.header))
        .collect())
}

fn open_path<'a>(
    path: PathBuf,
    shared: Arc<Shared<'a>>,
) -> Box<dyn Iterator<Item = Result<JournalItem, Error>> + Send + 'a> {
    let name = path.display().to_string();
    match File::open(&path) {
        Ok(file) => Box::new(FileStream::open(Source::File(file), name, shared)),
        Err(_) => {
            shared.skip(name, "io");
            Box::new(std::iter::empty())
        }
    }
}

fn micros_to_instant(micros: u64) -> Instant {
    let micros = i64::try_from(micros).unwrap_or(i64::MAX);
    Instant::from_unix_nanos(micros.saturating_mul(NANOS_PER_MICRO))
}

/// Smallest whole number of microseconds not before `instant`.
fn ceil_micros(instant: Instant) -> u64 {
    let Ok(nanos) = u64::try_from(instant.as_unix_nanos()) else {
        return 0;
    };
    nanos.div_ceil(NANOS_PER_MICRO.unsigned_abs())
}

/// State of one query shared by the streams of its files.
struct Shared<'a> {
    query: &'a LogQuery,
    unit: Option<NameMatch>,
    limits: LogLimits,
    notices: Notices,
    since: Option<u64>,
    until: Option<u64>,
    unit_seen: AtomicBool,
    source_seen: AtomicBool,
    unit_suggestions: Mutex<BTreeSet<String>>,
    source_suggestions: Mutex<BTreeSet<String>>,
}

impl<'a> Shared<'a> {
    fn new(query: &'a LogQuery, limits: &LogLimits) -> Result<Self, Error> {
        if let Some(cursor) = &query.cursor
            && !matches!(cursor, Cursor::Journal { .. })
        {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "the cursor belongs to another log store; use a cursor returned by `logs` on this system",
            )
            .with_found(cursor.to_string()));
        }
        Ok(Self {
            query,
            unit: query
                .unit
                .as_ref()
                .map(mapping::normalize_unit)
                .transpose()?,
            limits: limits.clone(),
            notices: Notices::new(),
            since: query.since.map(ceil_micros),
            until: query.until.map(ceil_micros),
            unit_seen: AtomicBool::new(false),
            source_seen: AtomicBool::new(false),
            unit_suggestions: Mutex::new(BTreeSet::new()),
            source_suggestions: Mutex::new(BTreeSet::new()),
        })
    }

    fn cache_bytes(&self) -> u64 {
        let files = u64::try_from(self.limits.max_open_files.max(1)).unwrap_or(u64::MAX);
        self.limits.read_cache_bytes / files
    }

    /// Reports a file that was skipped or read in part. A file is reported
    /// once, with the first cause: a truncated file that also fails later
    /// would otherwise appear twice with contradicting classes.
    fn skip(&self, path: String, detail: &str) {
        let reported = self.notices.snapshot().iter().any(|known| {
            known.code == NoticeCode::FileSkipped && known.path.as_ref() == Some(&path)
        });
        if reported {
            return;
        }
        let mut notice = LogNotice::new(NoticeCode::FileSkipped);
        notice.path = Some(path);
        notice.detail = Some(detail.to_owned());
        notice.count = Some(1);
        self.notices.add(notice);
    }

    fn may_contain(&self, header: &Header) -> bool {
        if header.n_entries == 0
            || self
                .since
                .is_some_and(|since| header.tail_entry_realtime < since)
            || self
                .until
                .is_some_and(|until| header.head_entry_realtime >= until)
        {
            return false;
        }
        match &self.query.cursor {
            Some(Cursor::Journal {
                seqnum_id, seqnum, ..
            }) => header.seqnum_id != *seqnum_id || header.head_entry_seqnum < *seqnum,
            _ => true,
        }
    }

    fn suggest(&self, unit: bool, names: impl IntoIterator<Item = String>) {
        let (wanted, target) = if unit {
            (self.unit.as_ref(), &self.unit_suggestions)
        } else {
            (self.query.source.as_ref(), &self.source_suggestions)
        };
        let Some(wanted) = wanted.and_then(NameMatch::literal) else {
            return;
        };
        let mut target = target.lock().unwrap_or_else(PoisonError::into_inner);
        for name in names {
            if target.len() < MAX_SUGGESTIONS && mapping::similar(wanted, &name) {
                target.insert(name);
            }
        }
    }

    fn report_unknown_names(&self) {
        let checks = [
            (self.unit.is_some(), &self.unit_seen, &self.unit_suggestions),
            (
                self.query.source.is_some(),
                &self.source_seen,
                &self.source_suggestions,
            ),
        ];
        for (given, seen, suggestions) in checks {
            if given && !seen.load(AtomicOrdering::Relaxed) {
                let names = suggestions.lock().unwrap_or_else(PoisonError::into_inner);
                let mut notice = LogNotice::new(NoticeCode::UnknownName);
                notice.detail = (!names.is_empty())
                    .then(|| names.iter().cloned().collect::<Vec<_>>().join(", "));
                self.notices.add(notice);
            }
        }
    }
}

/// Entry of one file with the keys used for merging.
#[derive(Debug)]
struct JournalItem {
    event: LogEvent,
    seqnum_id: Id128,
    seqnum: u64,
    boot_id: Id128,
    monotonic: u64,
    realtime: u64,
    file_id: Id128,
    offset: u64,
    xor_hash: u64,
}

impl MergeItem for JournalItem {
    type Key = (Id128, u64, u64);

    /// Entries of one sequence space compare by sequence number, entries of
    /// one boot by monotonic time, others by reception time; the file and
    /// the offset break ties.
    fn newer(&self, other: &Self) -> Ordering {
        let primary = if self.seqnum_id == other.seqnum_id {
            self.seqnum.cmp(&other.seqnum)
        } else if self.boot_id == other.boot_id {
            self.monotonic.cmp(&other.monotonic)
        } else {
            self.realtime.cmp(&other.realtime)
        };
        primary
            .then_with(|| self.file_id.cmp(&other.file_id))
            .then_with(|| self.offset.cmp(&other.offset))
    }

    fn time(&self) -> Instant {
        self.event.time
    }

    fn key(&self) -> Option<Self::Key> {
        Some((self.seqnum_id, self.seqnum, self.xor_hash))
    }
}

/// One list of entries walked from the newest position.
struct Lead {
    index: EntryIndex,
    position: u64,
    head: Option<u64>,
}

/// Entries of one file that satisfy the query, newest first.
struct FileStream<'a> {
    file: JournalFile,
    shared: Arc<Shared<'a>>,
    path: String,
    leads: Vec<Lead>,
    unit_set: Option<HashSet<u64>>,
    source_set: Option<HashSet<u64>>,
    excluded: HashSet<u64>,
    done: bool,
}

/// DATA objects that satisfy one condition.
type Condition = Option<Vec<DataObject>>;

/// How one file is searched.
struct Plan {
    leads: Vec<Lead>,
    unit_set: Option<HashSet<u64>>,
    source_set: Option<HashSet<u64>>,
    excluded: HashSet<u64>,
}

impl<'a> FileStream<'a> {
    fn open(
        source: Source,
        path: String,
        shared: Arc<Shared<'a>>,
    ) -> impl Iterator<Item = Result<JournalItem, Error>> + Send + 'a {
        let opened = JournalFile::open(source, shared.cache_bytes()).and_then(|mut file| {
            if file.truncated() {
                shared.skip(path.clone(), Fault::Truncated.class());
            }
            let plan = Self::plan(&mut file, &shared)?;
            Ok((file, plan))
        });
        let stream = match opened {
            Ok((file, Some(plan))) => Some(Self {
                file,
                shared,
                path,
                leads: plan.leads,
                unit_set: plan.unit_set,
                source_set: plan.source_set,
                excluded: plan.excluded,
                done: false,
            }),
            Ok((_, None)) => None,
            Err(fault) => {
                shared.skip(path, fault.class());
                None
            }
        };
        stream.into_iter().flatten()
    }

    fn plan(file: &mut JournalFile, shared: &Shared<'_>) -> Result<Option<Plan>, Fault> {
        let mut excluded = HashSet::new();
        for priority in shared.query.max_priority().saturating_add(1)..=PRIORITY_DEBUG {
            if let Some(data) = file.find_data(format!("PRIORITY={priority}").as_bytes())? {
                excluded.insert(data.offset);
            }
        }
        let unit = match &shared.unit {
            Some(unit) => Some(unit_condition(file, shared, unit)?),
            None => None,
        };
        let source = match &shared.query.source {
            Some(source) => source_condition(file, shared, source)?,
            None => None,
        };
        if unit.as_ref().is_some_and(Vec::is_empty) || source.as_ref().is_some_and(Vec::is_empty) {
            return Ok(None);
        }
        let to_set = |condition: &Condition| {
            condition.as_ref().map(|datas| {
                datas
                    .iter()
                    .map(|data| data.offset)
                    .collect::<HashSet<u64>>()
            })
        };
        let (unit_set, source_set) = (to_set(&unit), to_set(&source));
        let weight = |condition: &Condition| {
            condition.as_ref().map(|datas| {
                datas
                    .iter()
                    .map(|data| data.n_entries)
                    .fold(0_u64, u64::saturating_add)
            })
        };
        let leader = match (weight(&unit), weight(&source)) {
            (Some(left), Some(right)) if right < left => source,
            (Some(_), _) => unit,
            (None, _) => source,
        };
        let indexes = match leader {
            Some(datas) => datas
                .iter()
                .map(|data| EntryIndex::of_data(file, data))
                .collect::<Result<Vec<_>, _>>()?,
            None => vec![EntryIndex::global(file)?],
        };
        let mut leads = Vec::with_capacity(indexes.len());
        for index in indexes {
            let position = match shared.until {
                Some(until) => index.count_before(file, until)?,
                None => index.len(),
            };
            leads.push(Lead {
                index,
                position,
                head: None,
            });
        }
        Ok(Some(Plan {
            leads,
            unit_set,
            source_set,
            excluded,
        }))
    }

    fn next_offset(&mut self) -> Result<Option<u64>, Fault> {
        let limit = self.file.limit();
        for lead in &mut self.leads {
            while lead.head.is_none() && lead.position > 0 {
                lead.position -= 1;
                let offset = lead.index.get(&mut self.file, lead.position)?;
                if offset != 0 && offset < limit {
                    lead.head = Some(offset);
                }
            }
        }
        let newest = self.leads.iter().filter_map(|lead| lead.head).max();
        if let Some(offset) = newest {
            for lead in &mut self.leads {
                if lead.head == Some(offset) {
                    lead.head = None;
                }
            }
        }
        Ok(newest)
    }

    fn admits_items(&self, entry: &EntryObject) -> bool {
        let meets = |set: &Option<HashSet<u64>>| {
            set.as_ref()
                .is_none_or(|set| entry.items.iter().any(|item| set.contains(item)))
        };
        !entry.items.iter().any(|item| self.excluded.contains(item))
            && meets(&self.unit_set)
            && meets(&self.source_set)
    }

    fn admits_position(&self, entry: &EntryObject) -> bool {
        if self
            .shared
            .until
            .is_some_and(|until| entry.realtime >= until)
        {
            return false;
        }
        match &self.shared.query.cursor {
            Some(cursor) => cursor
                .admits_journal(&self.file.header().seqnum_id, entry.seqnum, entry.realtime)
                .is_some(),
            None => true,
        }
    }

    /// Fields of an entry and whether a payload was dropped because it
    /// exceeds `max_data_bytes` and left no name to keep.
    fn decode(&mut self, entry: &EntryObject) -> Result<(Vec<Field>, bool), Fault> {
        let max = self.shared.limits.max_data_bytes;
        let mut fields = Vec::with_capacity(entry.items.len());
        let mut dropped = false;
        for item in &entry.items {
            let data = self.file.data(*item)?;
            let payload = self.file.payload(&data, max)?;
            let cut = payload.truncated;
            match Field::split(payload) {
                Some(field) => fields.push(field),
                None => dropped |= cut,
            }
        }
        Ok((fields, dropped))
    }

    fn step(&mut self) -> Result<Option<JournalItem>, Fault> {
        loop {
            let Some(offset) = self.next_offset()? else {
                return Ok(None);
            };
            let entry = match self.file.entry(offset) {
                Ok(entry) => entry,
                Err(Fault::Beyond) => continue,
                Err(fault) => return Err(fault),
            };
            if self
                .shared
                .since
                .is_some_and(|since| entry.realtime < since)
            {
                return Ok(None);
            }
            if !self.admits_position(&entry) || !self.admits_items(&entry) {
                continue;
            }
            let (fields, dropped) = match self.decode(&entry) {
                Ok(decoded) => decoded,
                Err(Fault::Beyond) => continue,
                Err(fault) => return Err(fault),
            };
            let header = self.file.header();
            let origin = Origin {
                file_id: header.file_id,
                seqnum_id: header.seqnum_id,
            };
            let mut event = mapping::to_event(origin, &entry, fields, &self.shared.limits);
            event.flags.truncated |= dropped;
            let filter = EventFilter::new(self.shared.query).with_unit(self.shared.unit.as_ref());
            if filter.matches(&event) {
                return Ok(Some(JournalItem {
                    event,
                    seqnum_id: origin.seqnum_id,
                    seqnum: entry.seqnum,
                    boot_id: entry.boot_id,
                    monotonic: entry.monotonic,
                    realtime: entry.realtime,
                    file_id: origin.file_id,
                    offset: entry.offset,
                    xor_hash: entry.xor_hash,
                }));
            }
        }
    }
}

impl Iterator for FileStream<'_> {
    type Item = Result<JournalItem, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(item)) => Some(Ok(item)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(fault) => {
                self.done = true;
                self.shared.skip(self.path.clone(), fault.class());
                None
            }
        }
    }
}

fn values(file: &mut JournalFile, name: &str) -> Result<Vec<(DataObject, String)>, Fault> {
    Ok(file
        .field_data(name.as_bytes(), NAME_VALUE_BYTES)?
        .into_iter()
        .filter_map(|(data, payload)| {
            let field = Field::split(payload)?;
            Some((data, String::from_utf8_lossy(&field.value).into_owned()))
        })
        .collect())
}

fn unit_condition(
    file: &mut JournalFile,
    shared: &Shared<'_>,
    unit: &NameMatch,
) -> Result<Vec<DataObject>, Fault> {
    let mut datas = Vec::new();
    match unit.literal() {
        Some(name) => {
            for field in UNIT_FIELDS {
                datas.extend(file.find_data(format!("{field}={name}").as_bytes())?);
            }
        }
        None => {
            for field in UNIT_FIELDS {
                datas.extend(
                    values(file, field)?
                        .into_iter()
                        .filter(|(_, value)| unit.matches(value))
                        .map(|(data, _)| data),
                );
            }
        }
    }
    if datas.is_empty() {
        let known = values(file, UNIT_FIELDS[0])?
            .into_iter()
            .map(|(_, value)| value);
        shared.suggest(true, known);
    } else {
        shared.unit_seen.store(true, AtomicOrdering::Relaxed);
    }
    Ok(datas)
}

fn source_condition(
    file: &mut JournalFile,
    shared: &Shared<'_>,
    source: &NameMatch,
) -> Result<Condition, Fault> {
    if source.matches(UNKNOWN_SOURCE) {
        shared.source_seen.store(true, AtomicOrdering::Relaxed);
        return Ok(None);
    }
    let mut datas = Vec::new();
    for field in SOURCE_FIELDS {
        match source.literal() {
            Some(name) => datas.extend(file.find_data(format!("{field}={name}").as_bytes())?),
            None => datas.extend(
                values(file, field)?
                    .into_iter()
                    .filter(|(_, value)| source.matches(value))
                    .map(|(data, _)| data),
            ),
        }
    }
    datas.extend(
        values(file, EXE_FIELD)?
            .into_iter()
            .filter(|(_, value)| {
                source.matches(&String::from_utf8_lossy(mapping::exe_name(
                    value.as_bytes(),
                )))
            })
            .map(|(data, _)| data),
    );
    if datas.is_empty() {
        let known = values(file, SOURCE_FIELDS[0])?
            .into_iter()
            .map(|(_, value)| value);
        shared.suggest(false, known);
    } else {
        shared.source_seen.store(true, AtomicOrdering::Relaxed);
    }
    Ok(Some(datas))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_time_bounds_to_whole_microseconds() {
        assert_eq!(ceil_micros(Instant::from_unix_nanos(-5)), 0);
        assert_eq!(ceil_micros(Instant::from_unix_nanos(1_000)), 1);
        assert_eq!(ceil_micros(Instant::from_unix_nanos(1_001)), 2);
        assert_eq!(micros_to_instant(u64::MAX).as_unix_nanos(), i64::MAX);
    }

    #[test]
    fn rejects_cursor_of_another_store() {
        let query = LogQuery {
            cursor: Some(Cursor::EventLog {
                channel: "System".to_owned(),
                record_id: 1,
            }),
            ..LogQuery::default()
        };
        let error = query_bytes(Vec::new(), &query, &LogLimits::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn a_file_is_reported_as_skipped_once_with_the_first_cause() {
        let query = LogQuery::default();
        let shared = Shared::new(&query, &LogLimits::default()).unwrap();
        shared.skip("/var/log/journal/a.journal".to_owned(), "truncated");
        shared.skip("/var/log/journal/a.journal".to_owned(), "corrupt");
        shared.skip("/var/log/journal/b.journal".to_owned(), "corrupt");
        let notices = shared.notices.snapshot();
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].detail.as_deref(), Some("truncated"));
        assert_eq!(notices[0].count, Some(1));
        assert_eq!(notices[1].detail.as_deref(), Some("corrupt"));
    }

    #[test]
    fn empty_input_ends_with_skip_notice() {
        let query = LogQuery::default();
        let mut stream = query_bytes(Vec::new(), &query, &LogLimits::default()).unwrap();
        assert!(stream.next().is_none());
        let notices = stream.notices();
        assert_eq!(notices[0].code, NoticeCode::FileSkipped);
        assert_eq!(notices[0].detail.as_deref(), Some("truncated"));
    }
}
