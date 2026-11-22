// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Files of the schedule store.
//!
//! | Path under `<state>/schedules/` | Content | Written by |
//! |---|---|---|
//! | `definitions/<id>.json` | [`Definition`] | schedule commands, through the operation journal |
//! | `state.json` | [`StateFile`] | the schedule module only, atomically |
//! | `state.lock` | lock of `state.json` writers | the schedule module |
//! | `locks/<id>.lock` | lock held while a run of the schedule executes | the process running it |
//!
//! Readers never see a partial `state.json`: it is written to a temporary
//! file in the same directory and renamed over the previous file.

pub mod model;

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use nurt_contracts::error::Error;

pub use self::model::{
    Definition, Entry, FORMAT, OnMissed, Overlap, RunRecord, RunState, RunTrigger, SkipReason,
    StateFile, Status, run_id,
};
use crate::errors::{format, invalid, io};

const STORE_DIR: &str = "schedules";
const DEFINITIONS_DIR: &str = "definitions";
const LOCKS_DIR: &str = "locks";
const STATE_FILE: &str = "state.json";
const STATE_LOCK: &str = "state.lock";
const JSON_EXTENSION: &str = "json";
const ID_PREFIX: char = 's';
const MAX_ID_LENGTH: usize = 64;

/// Location of the schedule store of one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    root: PathBuf,
}

/// Definitions read from the store.
#[derive(Debug, Clone, Default)]
pub struct Definitions {
    /// Valid definitions, ordered by identifier.
    pub items: Vec<Definition>,
    /// Files that could not be read, with the problem.
    pub invalid: Vec<(PathBuf, String)>,
}

/// Signature of the definition files, used to detect changes made without a
/// notification, such as an undo through the journal.
pub type Fingerprint = Vec<(String, u64, Option<SystemTime>)>;

/// Lock held while a run of a schedule executes; released on drop.
#[derive(Debug)]
pub struct RunLock {
    _file: File,
}

impl Store {
    /// Store under the state directory `state_dir`.
    #[must_use]
    pub fn new(state_dir: &Path) -> Self {
        Self {
            root: state_dir.join(STORE_DIR),
        }
    }

    /// Directory of the store, `<state>/schedules`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of the definitions.
    #[must_use]
    pub fn definitions_dir(&self) -> PathBuf {
        self.root.join(DEFINITIONS_DIR)
    }

    /// File of the definition `id`.
    #[must_use]
    pub fn definition_path(&self, id: &str) -> PathBuf {
        self.definitions_dir()
            .join(format!("{id}.{JSON_EXTENSION}"))
    }

    /// File of the run state.
    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    /// Reads all definitions; a missing directory holds none.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the directory cannot be listed. Files that do not
    /// parse are reported in [`Definitions::invalid`].
    pub fn load_definitions(&self) -> Result<Definitions, Error> {
        let dir = self.definitions_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Definitions::default());
            }
            Err(error) => return Err(io(&dir, &error)),
        };
        let mut definitions = Definitions::default();
        for entry in entries {
            let path = entry.map_err(|error| io(&dir, &error))?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some(JSON_EXTENSION) {
                continue;
            }
            match read_definition(&path) {
                Ok(definition) => definitions.items.push(definition),
                Err(error) => definitions.invalid.push((path, error.message.clone())),
            }
        }
        definitions.items.sort_by(|a, b| id_order(&a.id).cmp(&id_order(&b.id)));
        Ok(definitions)
    }

    /// Reads the definition `id`, `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` or `E_FORMAT` when the file cannot be read.
    pub fn load_definition(&self, id: &str) -> Result<Option<Definition>, Error> {
        let path = self.definition_path(id);
        match read_definition(&path) {
            Ok(definition) => Ok(Some(definition)),
            Err(_) if !path.exists() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Reads the run state; a missing file is an empty state.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` or `E_FORMAT` when the file cannot be read.
    pub fn read_state(&self) -> Result<StateFile, Error> {
        let path = self.state_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(StateFile::default()),
            Err(error) => return Err(io(&path, &error)),
        };
        let state: StateFile =
            serde_json::from_slice(&bytes).map_err(|error| format(&path, error))?;
        if state.format != FORMAT {
            return Err(format(&path, format!("unknown format `{}`", state.format)));
        }
        Ok(state)
    }

    /// Writes the run state atomically under the writer lock.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the file cannot be written.
    pub fn write_state(&self, state: &StateFile) -> Result<(), Error> {
        fs::create_dir_all(&self.root).map_err(|error| io(&self.root, &error))?;
        let lock_path = self.root.join(STATE_LOCK);
        let lock = open_lock(&lock_path)?;
        lock.lock().map_err(|error| io(&lock_path, &error))?;
        let mut bytes =
            serde_json::to_vec_pretty(state).map_err(|error| format(&self.state_path(), error))?;
        bytes.push(b'\n');
        let path = self.state_path();
        let temporary = self
            .root
            .join(format!("{STATE_FILE}.{}.tmp", std::process::id()));
        let written = write_synced(&temporary, &bytes).and_then(|()| fs::rename(&temporary, &path));
        if let Err(error) = written {
            let _ = fs::remove_file(&temporary);
            return Err(io(&path, &error));
        }
        Ok(())
    }

    /// Signature of the definition files.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        let Ok(entries) = fs::read_dir(self.definitions_dir()) else {
            return Vec::new();
        };
        let mut fingerprint: Fingerprint = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                Some((
                    entry.file_name().to_string_lossy().into_owned(),
                    meta.len(),
                    meta.modified().ok(),
                ))
            })
            .collect();
        fingerprint.sort();
        fingerprint
    }

    /// Takes the run lock of the schedule `id`; `None` when another process
    /// holds it.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the lock file cannot be opened.
    pub fn try_run_lock(&self, id: &str) -> Result<Option<RunLock>, Error> {
        let dir = self.root.join(LOCKS_DIR);
        fs::create_dir_all(&dir).map_err(|error| io(&dir, &error))?;
        let path = dir.join(format!("{id}.lock"));
        let file = open_lock(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(RunLock { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(io(&path, &error)),
        }
    }
}

/// JSON content of a definition file.
///
/// # Errors
///
/// Returns `E_FORMAT` when the definition cannot be serialized.
pub fn encode_definition(definition: &Definition) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec_pretty(definition)
        .map_err(|error| format(Path::new(&definition.id), error))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Identifier for a new schedule: `s` followed by the number after the
/// largest number used by a definition or by the run state.
#[must_use]
pub fn next_id(definitions: &[Definition], state: &StateFile) -> String {
    let largest = definitions
        .iter()
        .map(|definition| definition.id.as_str())
        .chain(state.schedules.keys().map(String::as_str))
        .chain(state.history.keys().map(String::as_str))
        .filter_map(id_number)
        .max()
        .unwrap_or(0);
    format!("{ID_PREFIX}{}", largest.saturating_add(1))
}

/// Checks that `id` can name a definition file.
///
/// # Errors
///
/// Returns `E_INVALID_VALUE` for an empty identifier, one longer than 64
/// characters or one with characters other than letters, digits, `-` and `_`.
pub fn check_id(id: &str) -> Result<(), Error> {
    let valid = !id.is_empty()
        && id.len() <= MAX_ID_LENGTH
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{id}` is not a schedule identifier; identifiers look like `s3`"
        )))
    }
}

fn id_number(id: &str) -> Option<u64> {
    id.strip_prefix(ID_PREFIX)?.parse().ok()
}

fn id_order(id: &str) -> (u64, String) {
    (id_number(id).unwrap_or(u64::MAX), id.to_owned())
}

fn read_definition(path: &Path) -> Result<Definition, Error> {
    let bytes = fs::read(path).map_err(|error| io(path, &error))?;
    let definition: Definition =
        serde_json::from_slice(&bytes).map_err(|error| format(path, error))?;
    if definition.format != FORMAT {
        return Err(format(path, format!("unknown format `{}`", definition.format)));
    }
    let expected = path.file_stem().and_then(|stem| stem.to_str());
    if expected != Some(definition.id.as_str()) {
        return Err(format(path, "the identifier differs from the file name"));
    }
    Ok(definition)
}

fn open_lock(path: &Path) -> Result<File, Error> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|error| io(path, &error))
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
