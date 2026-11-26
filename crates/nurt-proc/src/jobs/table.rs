// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Job table in the state directory.
//!
//! Layout of `<state>/jobs`:
//!
//! | File | Content |
//! |---|---|
//! | `sequence`, `sequence.lock` | last job number; identifiers are never reused |
//! | `<id>.json` | job file: public record, request without environment, result |
//! | `<id>.input.json` | environment and standard input, readable only by the account of the engine |
//! | `<id>.lock` | locked by the process that supervises the job |
//! | `<id>.control/` | control requests to the supervising process and replies |
//! | `host.lock` | locked by the detached supervisor |
//! | `inbox/` | start requests to the detached supervisor and replies |
//!
//! Files are replaced atomically by renaming a complete temporary file, so
//! readers in other processes never see a partial file and need no lock. A
//! job whose file says it runs while its lock is free has lost its
//! supervisor: the operating system releases the lock when the supervising
//! process ends, however it ends.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::record::{Job, JobState};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{Instant, Record};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::state::is_live;
use super::wire::{StoredInput, StoredRequest, StoredResult};

/// Version of the job file and supervisor message format.
pub(crate) const PROTOCOL: u32 = 1;

/// Largest number of identifiers offered for an unknown job.
const MAX_CANDIDATES: usize = 3;

const JOBS_DIR: &str = "jobs";
const INBOX_DIR: &str = "inbox";
const SEQUENCE_FILE: &str = "sequence";
const SEQUENCE_LOCK: &str = "sequence.lock";
const HOST_LOCK: &str = "host.lock";
const ID_PREFIX: &str = "j";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Everything kept about one job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct JobFile {
    /// Format version.
    pub(crate) protocol: u32,
    /// Public record.
    pub(crate) record: Job,
    /// Command that started the job, for example `run` or `test.run`.
    pub(crate) command_name: String,
    /// Request of the run, without environment and standard input.
    pub(crate) request: StoredRequest,
    /// Waiting time of the call that started the job.
    pub(crate) wait: Option<Duration>,
    /// Process that supervises the job.
    pub(crate) supervisor_pid: u32,
    /// The call that started the job returned before the end.
    pub(crate) detached: bool,
    /// Finished run, once the job ended.
    pub(crate) result: Option<StoredResult>,
    /// Final record of a background tool job, for example a test run.
    pub(crate) final_record: Option<Record>,
}

impl JobFile {
    /// Identifier of the job.
    pub(crate) fn id(&self) -> &str {
        &self.record.id
    }
}

/// Access to the job files of one state directory.
#[derive(Debug, Clone)]
pub(crate) struct JobTable {
    dir: PathBuf,
}

impl JobTable {
    /// Opens the table of the state directory of `ctx`, creating it.
    ///
    /// # Errors
    ///
    /// Fails with `E_CONFIG` without a state directory and with `E_IO` when
    /// the directory cannot be created.
    pub(crate) fn open(ctx: &ExecCtx) -> Result<Self> {
        let state_dir = ctx.state_dir().ok_or_else(|| {
            Error::new(
                ErrorCode::Config,
                "background jobs need a state directory; configure the state directory of the engine",
            )
        })?;
        Self::at(state_dir.join(JOBS_DIR))
    }

    /// Opens the table kept in `dir`, creating it.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the directory cannot be created.
    pub(crate) fn at(dir: PathBuf) -> Result<Self> {
        create_private_dir(&dir)?;
        create_private_dir(&dir.join(INBOX_DIR))?;
        Ok(Self { dir })
    }

    /// Directory of start requests to the detached supervisor.
    pub(crate) fn inbox(&self) -> PathBuf {
        self.dir.join(INBOX_DIR)
    }

    /// Directory of control requests of one job.
    pub(crate) fn control_dir(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.control"))
    }

    /// Allocates the next job identifier.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the sequence cannot be read or written.
    pub(crate) fn allocate(&self) -> Result<String> {
        let lock = open_lock(&self.dir.join(SEQUENCE_LOCK))?;
        lock.lock()
            .map_err(|error| io_error("lock the job sequence", &error))?;
        let path = self.dir.join(SEQUENCE_FILE);
        let stored = match fs::read_to_string(&path) {
            Ok(text) => text.trim().parse::<u64>().unwrap_or(0),
            Err(error) if error.kind() == ErrorKind::NotFound => 0,
            Err(error) => return Err(io_error("read the job sequence", &error)),
        };
        let highest = self
            .ids()?
            .iter()
            .filter_map(|id| number(id))
            .max()
            .unwrap_or(0);
        let next = stored.max(highest).saturating_add(1);
        write_atomic(&path, next.to_string().as_bytes(), false)?;
        Ok(format!("{ID_PREFIX}{next}"))
    }

    /// Identifiers of all jobs, in ascending order.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the directory cannot be read.
    pub(crate) fn ids(&self) -> Result<Vec<String>> {
        let entries = fs::read_dir(&self.dir).map_err(|error| io_error("list the jobs", &error))?;
        let mut ids: Vec<(u64, String)> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let id = name.strip_suffix(".json")?;
                number(id).map(|value| (value, id.to_owned()))
            })
            .collect();
        ids.sort();
        Ok(ids.into_iter().map(|(_, id)| id).collect())
    }

    /// Writes the job file.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the file cannot be written.
    pub(crate) fn write(&self, file: &JobFile) -> Result<()> {
        write_json(&self.record_path(file.id()), file, false)
    }

    /// Reads the job file as stored, without checking the supervisor.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the file exists but cannot be read.
    pub(crate) fn read_raw(&self, id: &str) -> Result<Option<JobFile>> {
        read_json(&self.record_path(id))
    }

    /// Reads the job file; a job that runs according to the file but whose
    /// supervisor is gone is marked `lost` first.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the file cannot be read or updated.
    pub(crate) fn read(&self, id: &str) -> Result<Option<JobFile>> {
        let Some(file) = self.read_raw(id)? else {
            return Ok(None);
        };
        if !is_live(file.record.state) {
            return Ok(Some(file));
        }
        let lock = open_lock(&self.lock_path(id))?;
        match lock.try_lock() {
            Err(TryLockError::WouldBlock) => Ok(Some(file)),
            Err(TryLockError::Error(error)) => Err(io_error("check the job supervisor", &error)),
            Ok(()) => {
                let Some(mut file) = self.read_raw(id)? else {
                    return Ok(None);
                };
                if is_live(file.record.state) {
                    file.record.state = JobState::Lost;
                    self.write(&file)?;
                }
                Ok(Some(file))
            }
        }
    }

    /// Reads the job file or fails with `E_UNKNOWN_JOB` and the identifiers
    /// of existing jobs as candidates.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB` for an unknown job and `E_IO` when the
    /// table cannot be read.
    pub(crate) fn get(&self, id: &str) -> Result<JobFile> {
        if number(id).is_some()
            && let Some(file) = self.read(id)?
        {
            return Ok(file);
        }
        Err(self.unknown(id))
    }

    fn unknown(&self, id: &str) -> Error {
        let mut error = Error::new(
            ErrorCode::UnknownJob,
            format!("no job `{id}`; list the jobs with `jobs`"),
        )
        .with_found(id);
        let wanted = number(id);
        let mut known: Vec<(u64, String)> = self
            .ids()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|other| number(&other).map(|value| (value, other)))
            .collect();
        match wanted {
            Some(value) => known.sort_by_key(|(other, _)| other.abs_diff(value)),
            None => known.sort_by_key(|other| std::cmp::Reverse(other.0)),
        }
        for (rank, (_, other)) in known.into_iter().take(MAX_CANDIDATES).enumerate() {
            error = error.with_candidate(Candidate {
                rank: u32::try_from(rank).unwrap_or(u32::MAX).saturating_add(1),
                reference: None,
                program: format!("jobs \"{other}\""),
                why: None,
            });
        }
        error
    }

    /// Writes the environment and standard input of a job, readable by the
    /// account of the engine only.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the file cannot be written.
    pub(crate) fn write_input(&self, id: &str, input: &StoredInput) -> Result<()> {
        write_json(&self.input_path(id), input, true)
    }

    /// Reads the environment and standard input of a job.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the file exists but cannot be read.
    pub(crate) fn read_input(&self, id: &str) -> Result<Option<StoredInput>> {
        read_json(&self.input_path(id))
    }

    /// Locks the job for its supervisor; the lock is held until the returned
    /// file is dropped.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the lock cannot be taken.
    pub(crate) fn lock_job(&self, id: &str) -> Result<File> {
        let lock = open_lock(&self.lock_path(id))?;
        lock.lock()
            .map_err(|error| io_error("lock the job", &error))?;
        Ok(lock)
    }

    /// Takes the lock of the detached supervisor; `None` when another
    /// supervisor holds it.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the lock file cannot be opened.
    pub(crate) fn try_lock_host(&self) -> Result<Option<File>> {
        let lock = open_lock(&self.dir.join(HOST_LOCK))?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(lock)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(io_error("lock the supervisor", &error)),
        }
    }

    /// Whether a detached supervisor holds its lock.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the lock file cannot be opened.
    pub(crate) fn host_running(&self) -> Result<bool> {
        Ok(self.try_lock_host()?.is_none())
    }

    /// Removes all files of a job.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when a file exists but cannot be removed.
    pub(crate) fn remove(&self, id: &str) -> Result<()> {
        for path in [
            self.record_path(id),
            self.input_path(id),
            self.lock_path(id),
        ] {
            remove_file(&path)?;
        }
        match fs::remove_dir_all(self.control_dir(id)) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                Err(io_error("remove the job control files", &error))
            }
            _ => Ok(()),
        }
    }

    /// Identifiers of finished jobs that ended more than `ttl` before `now`;
    /// running and suspended jobs are never selected, nor are lost jobs, whose
    /// program may still run and whose end is unknown.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the table cannot be read.
    pub(crate) fn expired(&self, now: WallTime, ttl: Duration) -> Result<Vec<String>> {
        let mut expired = Vec::new();
        for id in self.ids()? {
            let Some(file) = self.read(&id)? else {
                continue;
            };
            if is_live(file.record.state) || file.record.state == JobState::Lost {
                continue;
            }
            let ended = file.record.finished.unwrap_or(file.record.started);
            if now.saturating_since(wall_time(ended)) > ttl {
                expired.push(id);
            }
        }
        Ok(expired)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn input_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.input.json"))
    }

    fn lock_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.lock"))
    }
}

/// Number of a job identifier `j<number>`.
pub(crate) fn number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix(ID_PREFIX)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Instant of a record as wall time.
pub(crate) fn wall_time(instant: Instant) -> WallTime {
    WallTime::from_unix_nanos(i128::from(instant.as_unix_nanos()))
}

/// Wall time as the instant of a record.
pub(crate) fn instant(time: WallTime) -> Instant {
    let nanos = time
        .unix_nanos()
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    Instant::from_unix_nanos(i64::try_from(nanos).unwrap_or_default())
}

/// Writes `value` as JSON to `path`, replacing the file atomically.
///
/// # Errors
///
/// Fails with `E_IO` when the file cannot be written.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T, private: bool) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("cannot encode {}: {error}", path.display()),
        )
    })?;
    write_atomic(path, &bytes, private)
}

/// Reads JSON from `path`; `None` when the file does not exist.
///
/// # Errors
///
/// Fails with `E_IO` when the file cannot be read or decoded.
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(&format!("read {}", path.display()), &error)),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("cannot decode {}: {error}", path.display()),
        )
    })
}

/// Removes a file; a missing file is not an error.
///
/// # Errors
///
/// Fails with `E_IO` when the file exists but cannot be removed.
pub(crate) fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(io_error(&format!("remove {}", path.display()), &error))
        }
        _ => Ok(()),
    }
}

/// Creates a directory accessible to the account of the engine only.
///
/// # Errors
///
/// Fails with `E_IO` when the directory cannot be created.
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(dir)
        .map_err(|error| io_error(&format!("create {}", dir.display()), &error))
}

/// Error `E_IO` for a failed file operation.
pub(crate) fn io_error(what: &str, error: &io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {what}: {error}"))
        .with_detail("os_error", serde_json::json!(error.to_string()))
}

fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(".{name}.{}.{counter}.tmp", std::process::id()));
    let result = write_new(&temp, bytes, private).and_then(|()| fs::rename(&temp, path));
    result.map_err(|error| {
        let _ = fs::remove_file(&temp);
        io_error(&format!("write {}", path.display()), &error)
    })
}

fn write_new(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn open_lock(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| io_error(&format!("open {}", path.display()), &error))
}
