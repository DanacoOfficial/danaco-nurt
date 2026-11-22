// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Processes of the operating system.
//!
//! The `processes` command lists the processes of the host as `process`
//! records, ordered by identifier. The parameters `name`, `user` and `pid`
//! select processes; with `tree: true` they select the roots of a tree that
//! also holds all descendants of each root, in depth-first order, with the
//! fields `depth` and `children`.
//!
//! | Field | Linux | macOS | Windows |
//! |---|---|---|---|
//! | process table | `/proc/<pid>/stat`, `status`, `cmdline` | `proc_listpids`, `proc_pidinfo(PROC_PIDTASKALLINFO)` | `CreateToolhelp32Snapshot` |
//! | `executable` | `/proc/<pid>/exe` | `proc_pidpath` | `QueryFullProcessImageNameW` |
//! | `command` | `/proc/<pid>/cmdline` | `sysctl(KERN_PROCARGS2)` | process environment block |
//! | `cwd` | `/proc/<pid>/cwd` | `proc_pidinfo(PROC_PIDVNODEPATHINFO)` | process environment block |
//! | `uid` | effective user identifier | effective user identifier | security identifier of the token |
//!
//! Fields of processes of other accounts that the account may not read stay
//! empty; one `source_unavailable` note of the report gives their number.
//!
//! The `cpu` field is the processor use in percent of one core. When the rest
//! of the pipeline names the fields it uses and `cpu` is among them, the
//! command reads the processor time twice, 200 ms apart, and reports the use
//! in that interval. Otherwise `cpu` is the average over the lifetime of the
//! process, computed from one reading without waiting.

mod cpu;
mod filter;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
mod model;
#[cfg(any(target_os = "macos", test))]
pub(crate) mod procargs;
#[cfg(any(target_os = "linux", all(test, unix)))]
pub(crate) mod procfs;
#[cfg(any(target_os = "linux", all(test, unix)))]
pub(crate) mod sysroot;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
mod tree;
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(any(windows, test))]
pub(crate) mod winpath;

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Process;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, Event, ExecCtx, InputKinds, LogEvent, LogLevel, OrderKey, ReadStage,
    RecordStream, RenderHint, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
    StreamOrder,
};
use nurt_contracts::value::Record;

use self::cpu::{CpuMeter, CpuMode};
use self::filter::Filter;
use self::model::{ProcessInfo, ProcessReader, ProcessSource, ReadRequest};

/// Name of the command in the catalog.
pub const COMMAND: &str = "processes";

pub(crate) const PARAM_NAME: &str = "name";
pub(crate) const PARAM_USER: &str = "user";
pub(crate) const PARAM_PID: &str = "pid";
pub(crate) const PARAM_CWD: &str = "cwd";
pub(crate) const PARAM_TREE: &str = "tree";

/// Default of the parameters `cwd` and `tree`.
const DEFAULT_FLAG: bool = false;

/// Implementation of the `processes` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::processes::Processes;
///
/// let stage = Processes::new();
/// assert_eq!(stage.descriptor().command, "processes");
/// assert!(stage.descriptor().source);
/// ```
#[derive(Clone)]
pub struct Processes {
    source: Arc<dyn ProcessSource>,
}

impl Processes {
    /// The command reading the process table of the running system.
    #[must_use]
    pub fn new() -> Self {
        Self {
            source: system_source(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_source(source: Arc<dyn ProcessSource>) -> Self {
        Self { source }
    }
}

impl Default for Processes {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Processes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Processes").finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
fn system_source() -> Arc<dyn ProcessSource> {
    Arc::new(procfs::ProcFs::live())
}

#[cfg(target_os = "macos")]
fn system_source() -> Arc<dyn ProcessSource> {
    Arc::new(macos::LibprocSource)
}

#[cfg(windows)]
fn system_source() -> Arc<dyn ProcessSource> {
    Arc::new(windows::Win32Source)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn system_source() -> Arc<dyn ProcessSource> {
    Arc::new(Unsupported)
}

/// Source of systems without a process reader.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
struct Unsupported;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
impl ProcessSource for Unsupported {
    fn open(&self, _request: ReadRequest) -> Result<Box<dyn ProcessReader>> {
        Err(Error::new(
            ErrorCode::OsOnly,
            "`processes` is available on Linux, macOS and Windows",
        ))
    }
}

/// Arguments of one call.
#[derive(Debug, Clone)]
struct Options {
    filter: Filter,
    cwd: bool,
    tree: bool,
}

impl Options {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        Ok(Self {
            filter: Filter::from_args(args)?,
            cwd: args.bool(PARAM_CWD)?.unwrap_or(DEFAULT_FLAG),
            tree: args.bool(PARAM_TREE)?.unwrap_or(DEFAULT_FLAG),
        })
    }
}

impl StageImpl for Processes {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Options::from_args(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Processes {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let options = Options::from_args(args)?;
        let mut reader = self.source.open(ReadRequest { cwd: options.cwd })?;
        let pids = match options.filter.pid() {
            Some(pid) if !options.tree => vec![pid],
            _ => reader.pids()?,
        };
        let mode = CpuMode::for_demand(&ctx.demand().fields);
        let meter = CpuMeter::start(mode, reader.as_mut(), &pids, ctx)?;
        if options.tree {
            return tree_output(reader, pids, &options.filter, &meter, ctx);
        }
        let stream = ProcessStream {
            reader,
            pids: pids.into_iter(),
            filter: options.filter,
            meter,
            ctx,
            restricted: Restricted::default(),
        };
        let stream = match ctx.demand().limit {
            Some(limit) => {
                RecordStream::new(stream.take(usize::try_from(limit).unwrap_or(usize::MAX)))
            }
            None => RecordStream::new(stream),
        };
        Ok(StageOutput::records(stream.checked(ctx)).with_order(StreamOrder {
            keys: vec![OrderKey {
                field: "pid".to_owned(),
                descending: false,
            }],
        }))
    }
}

/// Reads all processes and returns them in tree order.
fn tree_output<'a>(
    mut reader: Box<dyn ProcessReader>,
    pids: Vec<u64>,
    filter: &Filter,
    meter: &CpuMeter,
    ctx: &'a ExecCtx,
) -> Result<StageOutput<'a>> {
    let max = ctx.limits().max_records;
    let mut processes = Vec::new();
    let mut restricted = Restricted::default();
    for pid in pids {
        ctx.checkpoint()?;
        if let Some(process) = reader.read(pid) {
            if u64::try_from(processes.len()).unwrap_or(u64::MAX) >= max {
                return Err(Error::new(
                    ErrorCode::Limit,
                    format!("the process tree holds more than {max} processes"),
                ));
            }
            processes.push(process);
        }
    }
    let unfiltered = filter.is_empty();
    let order = tree::order(processes, |process| unfiltered || filter.matches(process));
    if order.cycles > 0 {
        ctx.emit(Event::Log(LogEvent {
            level: LogLevel::Warn,
            message: format!(
                "the parent links of the process table form {} cycle(s); the tree is cut at the \
                 repeated process",
                order.cycles
            ),
            source: Some(COMMAND.to_owned()),
        }));
    }
    let records: Vec<Record> = order
        .entries
        .into_iter()
        .map(|(process, position)| {
            restricted.count(&process);
            let cpu = meter.cpu(&process, ctx);
            Record::from(process.into_record(cpu, Some(position)))
        })
        .collect();
    restricted.report(ctx);
    Ok(StageOutput::records(RecordStream::from_records(records)).with_render(RenderHint::Tree))
}

/// Counter of processes with fields the account may not read.
#[derive(Debug, Default)]
struct Restricted {
    count: u64,
    reported: bool,
}

impl Restricted {
    fn count(&mut self, process: &ProcessInfo) {
        if process.restricted {
            self.count += 1;
        }
    }

    fn report(&mut self, ctx: &ExecCtx) {
        if self.reported || self.count == 0 {
            return;
        }
        self.reported = true;
        let mut note = NotableEntry::new(Notable::SourceUnavailable);
        note.count = Some(self.count);
        note.detail = Some(format!(
            "the executable, arguments or working directory of {} processes are not readable \
             by this account",
            self.count
        ));
        ctx.note(note);
    }
}

/// Lazy stream of `process` records in the order of identifiers.
struct ProcessStream<'a> {
    reader: Box<dyn ProcessReader>,
    pids: std::vec::IntoIter<u64>,
    filter: Filter,
    meter: CpuMeter,
    ctx: &'a ExecCtx,
    restricted: Restricted,
}

impl Iterator for ProcessStream<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        for pid in self.pids.by_ref() {
            let Some(process) = self.reader.read(pid) else {
                continue;
            };
            if !self.filter.matches(&process) {
                continue;
            }
            self.restricted.count(&process);
            let cpu = self.meter.cpu(&process, self.ctx);
            let record: Process = process.into_record(cpu, None);
            return Some(Ok(Record::from(record)));
        }
        self.restricted.report(self.ctx);
        None
    }
}

impl Drop for ProcessStream<'_> {
    fn drop(&mut self) {
        self.restricted.report(self.ctx);
    }
}
