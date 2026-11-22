// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixtures of the `sql` tests: an execution context, bound arguments,
//! databases created in temporary directories and a journal transaction that
//! keeps copies of the declared files and can restore them.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nurt_contracts::config::{Config, DurationSetting};
use nurt_contracts::env::{
    BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime,
};
use nurt_contracts::error::{Error, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle,
    OpId, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction, TargetIndex,
    TargetPath, TargetSpec,
};
use nurt_contracts::record::Row;
use nurt_contracts::report::{Change, Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, Limits, StageData, StageImpl,
    StageInput,
};
use nurt_contracts::value::{Record, Value};
use nurt_doc::commands::sql::SqlStage;
use rusqlite::Connection;

/// Busy timeout of the tests that wait for a lock.
pub const SHORT_BUSY: Duration = Duration::from_millis(150);

struct TestClock {
    origin: Instant,
}

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime(0)
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: WallTime(0),
            boot: BootId("test-boot".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }

    fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.wait_timeout(duration).map_or(Ok(()), Err)
    }
}

struct TestHost {
    info: HostInfo,
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

/// Options of a test context.
pub struct CtxOptions {
    pub busy: Duration,
    pub cancel: CancelToken,
    pub max_records: u64,
}

impl Default for CtxOptions {
    fn default() -> Self {
        Self {
            busy: Duration::from_secs(5),
            cancel: CancelToken::new(),
            max_records: Limits::default().max_records,
        }
    }
}

/// Execution context without file system and services.
pub fn context(options: CtxOptions) -> ExecCtx {
    let mut config = Config::default();
    config.sql.busy_timeout =
        DurationSetting::from_millis(u64::try_from(options.busy.as_millis()).unwrap());
    let host = TestHost {
        info: HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "test".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from("/home/user"),
            temp: std::env::temp_dir(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        },
    };
    ExecCtx::builder(
        Arc::new(TestClock {
            origin: Instant::now(),
        }),
        Arc::new(host),
    )
    .config(Arc::new(config))
    .cancel(options.cancel)
    .limits(Limits {
        max_records: options.max_records,
        ..Limits::default()
    })
    .build()
}

/// Context with the default options.
pub fn ctx() -> ExecCtx {
    context(CtxOptions::default())
}

/// Canonical text of a test path.
pub fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Bound arguments of one call.
pub fn args(path: &Path, query: &str, params: Vec<Value>, write: bool) -> BoundArgs {
    let defaulted = params.is_empty();
    BoundArgs::new("sql")
        .with("path", ArgValue::Value(Value::Path(text(path))), false)
        .with("query", ArgValue::Value(Value::Text(query.to_owned())), false)
        .with("params", ArgValue::Value(Value::List(params)), defaulted)
        .with("write", ArgValue::Value(Value::Bool(write)), !write)
}

/// Creates a database file `name` in `dir` by running `setup`.
pub fn database(dir: &Path, name: &str, setup: &str) -> PathBuf {
    let path = dir.join(name);
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(setup).unwrap();
    connection.close().unwrap();
    path
}

/// Runs `sql` in the read step and collects its rows.
pub fn read(args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let stage = SqlStage;
    let output = stage
        .as_read()
        .unwrap()
        .execute(args, StageInput::none(), ctx)?;
    match output.data {
        StageData::Records(stream) => stream.collect(),
        other => panic!("`sql` returned {other:?} instead of records"),
    }
}

/// Result of an effect.
pub struct Written {
    pub rows: Vec<Record>,
    pub changes: Vec<Change>,
    pub irreversible: Vec<IrreversibleEntry>,
}

/// Runs `sql` in the plan and apply steps.
pub fn write(args: &BoundArgs, ctx: &ExecCtx, tx: &mut dyn MutationTx) -> Result<Written> {
    let stage = SqlStage;
    let effect = stage.as_effect().unwrap();
    let plan = effect.plan(args, StageInput::none(), ctx)?;
    let outcome = effect.apply(args, plan, tx, ctx)?;
    let rows = match outcome.output.data {
        StageData::Records(stream) => stream.collect::<Result<Vec<_>>>()?,
        other => panic!("`sql` returned {other:?} instead of records"),
    };
    Ok(Written {
        rows,
        changes: outcome.changes,
        irreversible: outcome.irreversible,
    })
}

/// Typed form of a `row` record.
pub fn row(record: &Record) -> Row {
    Row::try_from(record.clone()).unwrap()
}

/// Cells of a `row` record as name and value pairs, in column order.
pub fn cells(record: &Record) -> Vec<(String, Value)> {
    row(record).cells.into_iter().collect()
}

/// One integer read with a separate connection.
pub fn count(path: &Path, query: &str) -> i64 {
    let connection = Connection::open(path).unwrap();
    connection.query_row(query, [], |row| row.get(0)).unwrap()
}

/// The database file followed by its `-wal`, `-shm` and `-journal` files.
pub fn database_files(path: &Path) -> Vec<PathBuf> {
    let mut files = vec![path.to_path_buf()];
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        files.push(PathBuf::from(name));
    }
    files
}

/// Content of every file, `None` for a missing file.
pub fn contents(files: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    files.iter().map(|file| fs::read(file).ok()).collect()
}

/// The `sqlite3` program from the search path, when installed.
pub fn sqlite3() -> Option<PathBuf> {
    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search)
        .map(|dir| dir.join("sqlite3"))
        .find(|candidate| candidate.is_file())
}

/// Error code of a failed result.
pub fn code<T>(result: Result<T>) -> nurt_contracts::error::ErrorCode {
    match result {
        Ok(_) => panic!("the call succeeded"),
        Err(error) => error.code(),
    }
}

/// Error of a failed result.
pub fn failure<T>(result: Result<T>) -> Error {
    match result {
        Ok(_) => panic!("the call succeeded"),
        Err(error) => error,
    }
}

/// Journal transaction for tests: keeps a copy of every declared file that
/// exists, or refuses copies as admission does without space, and restores
/// the declared files on [`RecordingTx::undo`].
pub struct RecordingTx {
    store: tempfile::TempDir,
    refuse_copies: bool,
    copies: Vec<Option<PathBuf>>,
    /// Declared targets, in order.
    pub declared: Vec<TargetSpec>,
    /// Content of every declared file when it was preserved.
    pub preserved: Vec<Option<Vec<u8>>>,
    /// Changes reported after the external write.
    pub observed: Vec<ExternalChange>,
}

impl RecordingTx {
    /// A transaction that copies existing files.
    pub fn new() -> Self {
        Self {
            store: tempfile::tempdir().unwrap(),
            refuse_copies: false,
            copies: Vec::new(),
            declared: Vec::new(),
            preserved: Vec::new(),
            observed: Vec::new(),
        }
    }

    /// A transaction whose admission refuses every copy.
    pub fn refusing_copies() -> Self {
        Self {
            refuse_copies: true,
            ..Self::new()
        }
    }

    /// Restores every declared file: copies are written back and files that
    /// did not exist are removed.
    pub fn undo(&self) {
        for ((spec, copy), before) in self.declared.iter().zip(&self.copies).zip(&self.preserved) {
            let TargetPath::Fs(path) = &spec.path else {
                panic!("`sql` declares file targets only");
            };
            match (copy, before) {
                (Some(copy), _) => {
                    fs::copy(copy, path.as_path()).unwrap();
                }
                (None, None) if path.as_path().exists() => fs::remove_file(path.as_path()).unwrap(),
                _ => {}
            }
        }
    }

    fn preserve_one(&mut self, spec: &TargetSpec) -> Preserved {
        let TargetPath::Fs(path) = &spec.path else {
            panic!("`sql` declares file targets only");
        };
        assert_eq!(spec.action, TargetAction::ExternalWrite);
        let index = TargetIndex(u32::try_from(self.declared.len()).unwrap());
        let content = fs::read(path.as_path()).ok();
        let (mode, irreversible, copy) = match (&content, self.refuse_copies) {
            (None, _) => (PreserveMode::Created, None, None),
            (Some(_), true) => (PreserveMode::None, Some(Irreversible::NoSpaceForCopy), None),
            (Some(bytes), false) => {
                let copy = self.store.path().join(index.0.to_string());
                fs::write(&copy, bytes).unwrap();
                (PreserveMode::Copy, None, Some(copy))
            }
        };
        let size = content.as_ref().map(|bytes| bytes.len() as u64);
        self.declared.push(spec.clone());
        self.copies.push(copy);
        self.preserved.push(content.clone());
        Preserved {
            index,
            volume: None,
            mode,
            bytes_held: if mode == PreserveMode::Copy { size.unwrap_or(0) } else { 0 },
            before: BeforeState {
                exists: content.is_some(),
                size,
                ..BeforeState::default()
            },
            irreversible,
            notable: Vec::new(),
        }
    }
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        Some(OpId(1))
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        assert!(self.observed.is_empty(), "preserve after a change");
        Ok(targets.iter().map(|spec| self.preserve_one(spec)).collect())
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        panic!("`sql` never stages content")
    }

    fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
        panic!("`sql` never writes through the file system layer")
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.observed.extend_from_slice(changes);
        Ok(changes
            .iter()
            .map(|change| Applied {
                index: change.declared.unwrap(),
                after: AfterState {
                    exists: change.path.as_path().exists(),
                    ..AfterState::default()
                },
            })
            .collect())
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {
        panic!("`sql` has no effect outside its files")
    }

    fn record_exempt(&mut self, _write: ExemptWrite) {
        panic!("`sql` has no exempt writes")
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        panic!("the stage never commits")
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        panic!("the stage never rolls back")
    }
}
