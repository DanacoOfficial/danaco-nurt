// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `process` records of holders on macOS and Windows.

use nurt_contracts::record::{Process, ProcessState};
use nurt_contracts::value::{Instant, Size};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

use super::Holder;
use crate::host::{SysRoot, canonical_path};

/// `process` records of `holders`; a process that cannot be read keeps the
/// identifier and the name reported with the holder.
pub(crate) fn processes(_root: &SysRoot, holders: &[Holder]) -> Vec<Process> {
    if holders.is_empty() {
        return Vec::new();
    }
    let pids: Vec<Pid> = holders
        .iter()
        .map(|holder| Pid::from_u32(holder.pid))
        .collect();
    let mut system = System::new().unwrap_or_default();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing()
            .with_memory()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    holders
        .iter()
        .map(|holder| record(&system, holder))
        .collect()
}

fn record(system: &System, holder: &Holder) -> Process {
    let pid = u64::from(holder.pid);
    let Some(process) = system.process(Pid::from_u32(holder.pid)) else {
        return Process {
            pid,
            name: holder.name.clone().unwrap_or_default(),
            ..Process::default()
        };
    };
    let (uid, user) = account(holder.pid, process);
    Process {
        pid,
        parent_pid: process
            .parent()
            .map_or(0, |parent| u64::from(parent.as_u32())),
        name: process.name().to_string_lossy().into_owned(),
        command: process
            .cmd()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect(),
        user: user.unwrap_or_default(),
        memory: Size::from_bytes(process.memory()),
        started: i64::try_from(process.start_time())
            .ok()
            .and_then(Instant::from_unix_seconds)
            .unwrap_or_default(),
        state: state(process.status()),
        executable: process.exe().map(canonical_path),
        uid,
        cwd: process.cwd().map(canonical_path),
        ..Process::default()
    }
}

#[cfg(target_os = "macos")]
fn account(_pid: u32, process: &sysinfo::Process) -> (Option<String>, Option<String>) {
    let uid = process.user_id().map(|uid| **uid);
    (
        uid.map(|uid| uid.to_string()),
        uid.and_then(crate::accounts::unix::user_name),
    )
}

#[cfg(windows)]
fn account(pid: u32, _process: &sysinfo::Process) -> (Option<String>, Option<String>) {
    crate::accounts::windows::process_account(pid)
        .map_or((None, None), |(sid, name)| (Some(sid), Some(name)))
}

fn state(status: ProcessStatus) -> ProcessState {
    match status {
        ProcessStatus::Run => ProcessState::Running,
        ProcessStatus::Stop | ProcessStatus::Tracing => ProcessState::Stopped,
        ProcessStatus::Zombie | ProcessStatus::Dead => ProcessState::Zombie,
        ProcessStatus::Idle => ProcessState::Idle,
        _ => ProcessState::Sleeping,
    }
}
