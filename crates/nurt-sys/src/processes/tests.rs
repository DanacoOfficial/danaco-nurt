// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::processes::tests
// Responsibility:   Wall time of the virtual clock in the tests: 2026-01-01T00:00:00Z.
// Dependencies:     ProcessState, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::record::{Process, ProcessState};
use nurt_contracts::report::Notable;
use nurt_contracts::spec::CatalogFile;
use nurt_contracts::stage::{ArgValue, BoundArgs, CancelToken, Demand};
use nurt_contracts::value::{Instant, Record, Regex, Value};

use super::model::{ProcessInfo, ProcessReader, ProcessSource, ReadRequest};
use super::testing::{SystemClock, VirtualClock, context, only, run};
use super::{
    COMMAND, PARAM_CWD, PARAM_NAME, PARAM_PID, PARAM_TREE, PARAM_USER, Processes, procargs, winpath,
};

/// Wall time of the virtual clock in the tests: 2026-01-01T00:00:00Z.
const NOW: i64 = 1_767_225_600;

/// In-memory process table that counts its readings.
#[derive(Default)]
struct FakeTable {
    processes: Mutex<Vec<ProcessInfo>>,
    reads: AtomicUsize,
    cpu_reads: AtomicUsize,
}

impl FakeTable {
    fn with(processes: Vec<ProcessInfo>) -> Arc<Self> {
        Arc::new(Self {
            processes: Mutex::new(processes),
            ..Self::default()
        })
    }
}

struct FakeSource(Arc<FakeTable>);

impl ProcessSource for FakeSource {
    fn open(&self, _request: ReadRequest) -> Result<Box<dyn ProcessReader>> {
        Ok(Box::new(FakeReader(Arc::clone(&self.0))))
    }
}

struct FakeReader(Arc<FakeTable>);

impl ProcessReader for FakeReader {
    fn pids(&mut self) -> Result<Vec<u64>> {
        let mut pids: Vec<u64> = self
            .0
            .processes
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.pid)
            .collect();
        pids.sort_unstable();
        Ok(pids)
    }

    fn read(&mut self, pid: u64) -> Option<ProcessInfo> {
        self.0.reads.fetch_add(1, Ordering::SeqCst);
        self.0
            .processes
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.pid == pid)
            .cloned()
    }

    fn cpu_time(&mut self, pid: u64) -> Option<Duration> {
        self.0.cpu_reads.fetch_add(1, Ordering::SeqCst);
        self.read(pid)?.cpu_time
    }
}

/// Process `pid` with parent `parent`, started ten seconds before `NOW` and
/// with one second of processor time.
fn process(pid: u64, parent: u64, name: &str) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid: parent,
        name: name.to_owned(),
        command: vec![format!("/usr/bin/{name}")],
        user: Some("alice".to_owned()),
        uid: Some("1000".to_owned()),
        started: Instant::from_unix_seconds(NOW - 10),
        cpu_time: Some(Duration::from_secs(1)),
        ..ProcessInfo::default()
    }
}

fn args() -> BoundArgs {
    BoundArgs::new(COMMAND)
}

fn with(args: BoundArgs, name: &str, value: Value) -> BoundArgs {
    args.with(name, ArgValue::Value(value), false)
}

fn processes(records: Vec<Record>) -> Vec<Process> {
    records
        .into_iter()
        .map(|record| Process::try_from(record).unwrap())
        .collect()
}

fn run_fake(
    table: &Arc<FakeTable>,
    args: &BoundArgs,
    demand: Demand,
) -> (Result<Vec<Process>>, Arc<VirtualClock>) {
    let clock = VirtualClock::at(NOW);
    let ctx = context(clock.clone(), demand, CancelToken::new());
    let stage = Processes::with_source(Arc::new(FakeSource(Arc::clone(table))));
    (run(&stage, args, &ctx).map(processes), clock)
}

#[test]
fn processes_no_second_sample_when_cpu_not_requested() {
    let table = FakeTable::with(vec![process(7, 1, "worker")]);
    let (result, clock) = run_fake(&table, &args(), Demand::default());
    let records = result.unwrap();
    assert_eq!(table.reads.load(Ordering::SeqCst), 1);
    assert_eq!(table.cpu_reads.load(Ordering::SeqCst), 0);
    assert_eq!(clock.sleeps(), 0);
    assert_eq!(
        records[0].cpu,
        Some(10.0),
        "one second over a lifetime of ten"
    );
}

#[test]
fn processes_cpu_left_out_when_other_fields_requested() {
    let table = FakeTable::with(vec![process(7, 1, "worker")]);
    let (result, clock) = run_fake(&table, &args(), only(&["pid", "name"]));
    assert_eq!(result.unwrap()[0].cpu, None);
    assert_eq!(clock.sleeps(), 0);
}

#[test]
fn processes_sample_cancelled_returns_cancelled() {
    let table = FakeTable::with(vec![process(7, 1, "worker")]);
    let clock = VirtualClock::at(NOW);
    let cancel = CancelToken::new();
    clock.cancel_on_sleep(cancel.clone());
    let ctx = context(clock.clone(), only(&["cpu"]), cancel);
    let stage = Processes::with_source(Arc::new(FakeSource(table)));
    let error = run(&stage, &args(), &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
}

#[test]
fn processes_cpu_new_and_ended_processes_between_samples() {
    let table = FakeTable::with(vec![process(7, 1, "ending")]);
    let clock = VirtualClock::at(NOW);
    let shared = Arc::clone(&table);
    clock.on_sleep(move |_| {
        *shared.processes.lock().unwrap() = vec![process(8, 1, "new")];
    });
    let ctx = context(clock.clone(), only(&["cpu"]), CancelToken::new());
    let stage = Processes::with_source(Arc::new(FakeSource(table)));
    let records = processes(run(&stage, &args(), &ctx).unwrap());
    assert!(
        records.is_empty(),
        "the ended process is skipped, the new one was not listed"
    );
}

#[test]
fn processes_limit_stops_reading_early() {
    let table = FakeTable::with((1..=50).map(|pid| process(pid, 0, "worker")).collect());
    let demand = Demand {
        limit: Some(1),
        ..Demand::default()
    };
    let (result, _) = run_fake(&table, &args(), demand);
    assert_eq!(result.unwrap().len(), 1);
    assert_eq!(table.reads.load(Ordering::SeqCst), 1);
}

#[test]
fn processes_pid_filter_reads_one_process() {
    let table = FakeTable::with((1..=5).map(|pid| process(pid, 0, "worker")).collect());
    let (result, _) = run_fake(
        &table,
        &with(args(), PARAM_PID, Value::Int(3)),
        Demand::default(),
    );
    let records = result.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].pid, 3);
    assert_eq!(table.reads.load(Ordering::SeqCst), 1);
}

#[test]
fn processes_filter_regex_and_user() {
    let mut other = process(3, 1, "nginx");
    other.user = Some("bob".to_owned());
    other.uid = Some("1001".to_owned());
    let table = FakeTable::with(vec![
        process(1, 0, "init"),
        process(2, 1, "nginx"),
        other,
        process(4, 1, "NGINX.exe"),
    ]);
    let pattern = Value::Regex(Regex::new("^ng", "i").unwrap());
    let call = with(
        with(args(), PARAM_NAME, pattern),
        PARAM_USER,
        Value::from("alice"),
    );
    let pids: Vec<u64> = run_fake(&table, &call, Demand::default())
        .0
        .unwrap()
        .iter()
        .map(|p| p.pid)
        .collect();
    assert_eq!(pids, [2, 4]);

    let by_uid = with(args(), PARAM_USER, Value::from("1001"));
    let pids: Vec<u64> = run_fake(&table, &by_uid, Demand::default())
        .0
        .unwrap()
        .iter()
        .map(|p| p.pid)
        .collect();
    assert_eq!(pids, [3]);

    let text = with(args(), PARAM_NAME, Value::from("nginx"));
    let pids: Vec<u64> = run_fake(&table, &text, Demand::default())
        .0
        .unwrap()
        .iter()
        .map(|p| p.pid)
        .collect();
    assert_eq!(
        pids,
        [2, 3],
        "text matches the whole name, `.exe` only in the same case"
    );
}

#[test]
fn processes_text_name_matches_windows_program_suffix() {
    let table = FakeTable::with(vec![process(4, 1, "nginx.EXE")]);
    let text = with(args(), PARAM_NAME, Value::from("nginx"));
    assert_eq!(
        run_fake(&table, &text, Demand::default()).0.unwrap().len(),
        1
    );
}

#[test]
fn processes_invalid_regex_is_invalid_value() {
    let table = FakeTable::with(Vec::new());
    let call = with(
        args(),
        PARAM_NAME,
        Value::Regex(Regex::new("(", "").unwrap()),
    );
    let error = run_fake(&table, &call, Demand::default()).0.unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    let negative = with(args(), PARAM_PID, Value::Int(-1));
    let error = run_fake(&table, &negative, Demand::default())
        .0
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn processes_tree_depth_children_dfs_order() {
    // Process 1 is the parent of 2 and 3, 2 of 5 and 3 of 4. The parent 8 of
    // process 9 is missing, so 9 is a root of its own.
    let table = FakeTable::with(vec![
        process(1, 0, "init"),
        process(3, 1, "shell"),
        process(2, 1, "nginx"),
        process(4, 3, "editor"),
        process(5, 2, "nginx"),
        process(9, 8, "orphan"),
    ]);
    let tree = with(args(), PARAM_TREE, Value::Bool(true));
    let records = run_fake(&table, &tree, Demand::default()).0.unwrap();
    let order: Vec<(u64, u64)> = records.iter().map(|p| (p.pid, p.depth.unwrap())).collect();
    assert_eq!(order, [(1, 0), (2, 1), (5, 2), (3, 1), (4, 2), (9, 0)]);
    assert_eq!(records[0].children, Some(vec![2, 3]));
    assert_eq!(records[2].children, Some(Vec::new()));

    let roots = with(tree, PARAM_NAME, Value::from("nginx"));
    let records = run_fake(&table, &roots, Demand::default()).0.unwrap();
    let order: Vec<(u64, u64)> = records.iter().map(|p| (p.pid, p.depth.unwrap())).collect();
    assert_eq!(
        order,
        [(2, 0), (5, 1)],
        "a selected descendant appears once, under its root"
    );
}

#[test]
fn processes_tree_breaks_ppid_cycle() {
    let table = FakeTable::with(vec![
        process(1, 0, "init"),
        process(10, 11, "left"),
        process(11, 10, "right"),
        process(12, 11, "child"),
    ]);
    let tree = with(args(), PARAM_TREE, Value::Bool(true));
    let records = run_fake(&table, &tree, Demand::default()).0.unwrap();
    let order: Vec<(u64, u64)> = records.iter().map(|p| (p.pid, p.depth.unwrap())).collect();
    assert_eq!(order, [(1, 0), (10, 0), (11, 1), (12, 2)]);
}

#[test]
fn processes_cwd_permission_denied_warns_once() {
    let mut denied = process(2, 1, "daemon");
    denied.restricted = true;
    let mut other = denied.clone();
    other.pid = 3;
    let table = FakeTable::with(vec![process(1, 0, "init"), denied, other]);
    let clock = VirtualClock::at(NOW);
    let ctx = context(clock, Demand::default(), CancelToken::new());
    let stage = Processes::with_source(Arc::new(FakeSource(table)));
    let call = with(args(), PARAM_CWD, Value::Bool(true));
    let records = run(&stage, &call, &ctx).unwrap();
    assert_eq!(records.len(), 3);
    let notes = ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, Notable::SourceUnavailable);
    assert_eq!(notes[0].count, Some(2));
}

#[test]
fn processes_record_keeps_raw_state_of_other() {
    let mut info = process(5, 1, "dead");
    info.state = ProcessState::Other;
    info.state_raw = Some("X".to_owned());
    info.user = None;
    info.started = None;
    let record = info.into_record(None, None);
    assert_eq!(
        record.user, "1000",
        "the identifier stands in for an unknown name"
    );
    assert_eq!(record.started, Instant::from_unix_nanos(0));
    assert_eq!(
        record.os_raw.unwrap().get("state"),
        Some(&Value::Text("X".to_owned()))
    );
}

#[test]
fn windows_paths_are_canonical() {
    assert_eq!(
        winpath::canonical(r"c:\Windows\System32\svchost.exe"),
        "C:/Windows/System32/svchost.exe"
    );
    assert_eq!(winpath::canonical(r"\\?\C:\Tools\a.exe"), "C:/Tools/a.exe");
    assert_eq!(
        winpath::canonical(r"\\?\UNC\server\share\a.exe"),
        "//server/share/a.exe"
    );
    let wide: Vec<u16> = "cmd.exe\0garbage".encode_utf16().collect();
    assert_eq!(winpath::from_wide(&wide), "cmd.exe");
}

#[test]
fn macos_argument_area_decodes_arguments() {
    let mut area = 2_i32.to_ne_bytes().to_vec();
    area.extend_from_slice(b"/usr/bin/python3\0\0\0\0python3\0-c\0PATH=/usr/bin\0");
    assert_eq!(procargs::decode(&area).unwrap(), ["python3", "-c"]);
    let short = [3_i32.to_ne_bytes().as_slice(), b"/bin/sh\0sh\0"].concat();
    assert_eq!(
        procargs::decode(&short),
        None,
        "the area ends before the third argument"
    );
    assert_eq!(procargs::decode(&[1, 0]), None);
}

#[test]
fn catalog_signatures_match_frozen() {
    let path = format!(
        "{}/../../catalog/system.toml",
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR"))
    );
    let catalog: CatalogFile = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    // Name, type and default of a parameter.
    type Param = (&'static str, &'static str, Option<&'static str>);
    let expected: [(&str, &[Param]); 3] = [
        (
            COMMAND,
            &[
                (PARAM_NAME, "text | regex", None),
                (PARAM_USER, "text", None),
                (PARAM_PID, "int", None),
                (PARAM_CWD, "bool", Some("false")),
                (PARAM_TREE, "bool", Some("false")),
            ],
        ),
        (
            crate::ports::COMMAND,
            &[
                (
                    "state",
                    "enum(listen|all)",
                    Some(crate::ports::StateFilter::default().as_str()),
                ),
                (
                    "protocol",
                    "enum(tcp|udp|any)",
                    Some(crate::ports::ProtocolFilter::default().as_str()),
                ),
            ],
        ),
        (
            crate::disks::COMMAND,
            &[(
                "type",
                "enum(all|fixed|removable|network)",
                Some(crate::disks::TypeFilter::default().as_str()),
            )],
        ),
    ];
    for (command, params) in expected {
        let spec = catalog.command(command).unwrap();
        assert_eq!(spec.output.len(), 1, "{command}");
        let actual: Vec<(String, String, Option<String>)> = spec
            .params
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.ty.to_string(),
                    p.default.as_ref().map(|d| d.as_str().to_owned()),
                )
            })
            .collect();
        let wanted: Vec<(String, String, Option<String>)> = params
            .iter()
            .map(|(n, t, d)| ((*n).to_owned(), (*t).to_owned(), d.map(str::to_owned)))
            .collect();
        assert_eq!(actual, wanted, "{command}");
    }
}

#[cfg(unix)]
mod procfs_tree {
    use super::*;
    use crate::common::SysRoot;
    use crate::processes::procfs::{ProcFs, map_state, parse_passwd, parse_stat, split_command};
    use crate::processes::testing::write;

    /// Clock rate of the synthetic tree, in ticks per second.
    const TICKS: u64 = 100;
    /// Page size of the synthetic tree, in bytes.
    const PAGE: u64 = 4096;
    /// Boot time of the synthetic tree, 1000 seconds before `NOW`.
    const BOOT: i64 = NOW - 1000;

    /// `stat` line of a process with the given name, state, parent and
    /// processor ticks; it started 990 seconds after boot and holds 25 pages.
    fn stat_line(pid: u64, comm: &str, state: char, parent: u64, ticks: u64) -> String {
        format!(
            "{pid} ({comm}) {state} {parent} {pid} {pid} 0 -1 4194560 100 0 0 0 {ticks} 0 0 0 \
             20 0 3 0 99000 1000000 25 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 \
             0 0\n"
        )
    }

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "/proc/stat", format!("cpu  1 2 3\nbtime {BOOT}\n"));
        write(
            root,
            "/etc/passwd",
            "root:x:0:0::/root:/bin/sh\nalice:x:1000:1000::/home/user:/bin/sh\n",
        );
        write(root, "/proc/1/stat", stat_line(1, "init", 'S', 0, 50));
        write(root, "/proc/1/cmdline", b"/sbin/init\0splash\0");
        write(root, "/proc/1/status", "Name:\tinit\nUid:\t0\t0\t0\t0\n");
        write(root, "/proc/2/stat", stat_line(2, "kthreadd", 'S', 0, 0));
        write(root, "/proc/2/cmdline", b"");
        write(root, "/proc/2/status", "Uid:\t0\t0\t0\t0\n");
        write(
            root,
            "/proc/42/stat",
            stat_line(42, "web server (v2", 'R', 1, 100),
        );
        write(
            root,
            "/proc/42/cmdline",
            b"/opt/web/web server (v2\x00--port\x008080\x00",
        );
        write(root, "/proc/42/status", "Uid:\t1000\t1000\t1000\t1000\n");
        std::os::unix::fs::symlink("/opt/web/server", root.join("proc/42/exe")).unwrap();
        std::os::unix::fs::symlink("/srv/www", root.join("proc/42/cwd")).unwrap();
        write(root, "/proc/self", "");
        dir
    }

    fn stage(dir: &tempfile::TempDir) -> Processes {
        let source = ProcFs::new(SysRoot::new(dir.path()), TICKS, PAGE);
        Processes::with_source(Arc::new(source))
    }

    #[test]
    fn processes_parses_synthetic_proc_stat() {
        let stat = parse_stat(&stat_line(42, "a) (b c", 'R', 7, 300)).unwrap();
        assert_eq!(stat.comm, "a) (b c");
        assert_eq!(stat.state, 'R');
        assert_eq!(stat.parent_pid, 7);
        assert_eq!(stat.user_ticks, 300);
        assert_eq!(stat.threads, 3);
        assert_eq!(stat.start_ticks, 99_000);
        assert_eq!(stat.rss_pages, 25);
        assert_eq!(parse_stat("42 (cut"), None);
    }

    #[test]
    fn processes_command_argv_split_on_nul() {
        assert_eq!(
            split_command(b"/bin/sh\0-c\0echo a b\0"),
            ["/bin/sh", "-c", "echo a b"]
        );
        assert_eq!(
            split_command(b"a\0\0b\0"),
            ["a", "", "b"],
            "empty arguments are kept"
        );
        assert!(
            split_command(b"").is_empty(),
            "kernel threads have no arguments"
        );
    }

    #[test]
    fn processes_state_mapping_all_letters() {
        let states: Vec<ProcessState> = "RSDTtZIX".chars().map(|c| map_state(c).0).collect();
        use ProcessState::*;
        assert_eq!(
            states,
            [
                Running, Sleeping, Sleeping, Stopped, Stopped, Zombie, Idle, Other
            ]
        );
        assert_eq!(map_state('X').1.as_deref(), Some("X"));
        assert_eq!(map_state('R').1, None);
    }

    #[test]
    fn processes_passwd_names_by_uid() {
        let names = parse_passwd("# comment\nroot:x:0:0::/:/bin/sh\nbroken\nbob:x:1001:1001::/:\n");
        assert_eq!(names.get(&0).map(String::as_str), Some("root"));
        assert_eq!(names.get(&1001).map(String::as_str), Some("bob"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn processes_reads_synthetic_proc_tree() {
        let dir = tree();
        let clock = VirtualClock::at(NOW);
        let ctx = context(clock, Demand::default(), CancelToken::new());
        let call = with(args(), PARAM_CWD, Value::Bool(true));
        let records = processes(run(&stage(&dir), &call, &ctx).unwrap());
        let pids: Vec<u64> = records.iter().map(|p| p.pid).collect();
        assert_eq!(pids, [1, 2, 42]);
        let kernel = &records[1];
        assert_eq!(kernel.name, "[kthreadd]");
        assert!(kernel.command.is_empty());
        assert_web_server(&records[2]);
        assert_eq!(records[0].user, "root");
    }

    /// Checks every field of process 42 of the synthetic tree.
    fn assert_web_server(web: &Process) {
        assert_eq!(web.name, "web server (v2");
        assert_eq!(web.command, ["/opt/web/web server (v2", "--port", "8080"]);
        assert_eq!(web.user, "alice");
        assert_eq!(web.uid.as_deref(), Some("1000"));
        assert_eq!(web.state, ProcessState::Running);
        assert_eq!(web.parent_pid, 1);
        assert_eq!(web.memory.bytes(), 25 * PAGE);
        assert_eq!(web.started, Instant::from_unix_seconds(BOOT + 990).unwrap());
        assert_eq!(web.threads, Some(3));
        assert_eq!(web.executable.as_ref().unwrap().as_str(), "/opt/web/server");
        assert_eq!(web.cwd.as_ref().unwrap().as_str(), "/srv/www");
        assert_eq!(web.cpu, Some(10.0), "one second over ten seconds of life");
    }

    #[test]
    fn processes_long_kernel_name_takes_program_name() {
        let dir = tree();
        write(
            dir.path(),
            "/proc/50/stat",
            stat_line(50, "language-server", 'S', 1, 0),
        );
        write(
            dir.path(),
            "/proc/50/cmdline",
            b"/usr/bin/language-server-protocol\0",
        );
        let ctx = context(VirtualClock::at(NOW), Demand::default(), CancelToken::new());
        let call = with(args(), PARAM_PID, Value::Int(50));
        let records = processes(run(&stage(&dir), &call, &ctx).unwrap());
        assert_eq!(records[0].name, "language-server-protocol");
    }

    #[test]
    fn processes_comm_cut_inside_a_character_does_not_hide_the_process() {
        let dir = tree();
        let mut bytes = stat_line(60, "ZZ", 'S', 1, 0).into_bytes();
        let at = bytes.windows(2).position(|pair| pair == b"ZZ").unwrap();
        bytes.splice(at..at + 2, [b'a', 0xC3]);
        write(dir.path(), "/proc/60/stat", bytes);
        let ctx = context(VirtualClock::at(NOW), Demand::default(), CancelToken::new());
        let call = with(args(), PARAM_PID, Value::Int(60));
        let records = processes(run(&stage(&dir), &call, &ctx).unwrap());
        assert_eq!(records.len(), 1);
        assert!(records[0].name.starts_with('a'));
    }

    #[test]
    fn processes_long_multibyte_kernel_name_takes_program_name() {
        let dir = tree();
        let comm = "ßßßßßßßx";
        assert_eq!(comm.len(), 15);
        write(dir.path(), "/proc/61/stat", stat_line(61, comm, 'S', 1, 0));
        write(
            dir.path(),
            "/proc/61/cmdline",
            "/usr/bin/ßßßßßßßx-server\0".as_bytes(),
        );
        let ctx = context(VirtualClock::at(NOW), Demand::default(), CancelToken::new());
        let call = with(args(), PARAM_PID, Value::Int(61));
        let records = processes(run(&stage(&dir), &call, &ctx).unwrap());
        assert_eq!(records[0].name, "ßßßßßßßx-server");
    }

    #[test]
    fn processes_stat_with_a_negative_resident_size_reports_zero_pages() {
        let line = stat_line(42, "x", 'R', 1, 5)
            .replace(" 25 18446744073709551615", " -1 18446744073709551615");
        assert_eq!(parse_stat(&line).unwrap().rss_pages, 0);
        assert_eq!(parse_stat("42 (x) R 1 2"), None);
        assert_eq!(parse_stat("42 x) R"), None);
    }

    #[test]
    fn processes_cpu_two_samples_virtual_clock() {
        let dir = tree();
        let clock = VirtualClock::at(NOW);
        let root = dir.path().to_path_buf();
        clock.on_sleep(move |slept| {
            assert_eq!(slept, Duration::from_millis(200));
            write(
                &root,
                "/proc/42/stat",
                stat_line(42, "web server (v2", 'R', 1, 110),
            );
        });
        let ctx = context(clock.clone(), only(&["pid", "cpu"]), CancelToken::new());
        let call = with(args(), PARAM_PID, Value::Int(42));
        let records = processes(run(&stage(&dir), &call, &ctx).unwrap());
        assert_eq!(clock.sleeps(), 1);
        assert_eq!(
            records[0].cpu,
            Some(50.0),
            "ten ticks of 10 ms during 200 ms"
        );
    }

    #[test]
    fn processes_missing_proc_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = context(VirtualClock::at(NOW), Demand::default(), CancelToken::new());
        let error = run(&stage(&dir), &args(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Io);
    }
}

#[test]
#[ignore = "requires the process table of the running system"]
fn it_processes_contains_self_with_argv() {
    let ctx = context(SystemClock::new(), Demand::default(), CancelToken::new());
    let own = u64::from(std::process::id());
    let call = with(args(), PARAM_PID, Value::Int(i64::try_from(own).unwrap()));
    let records = processes(run(&Processes::new(), &call, &ctx).unwrap());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].pid, own);
    assert!(!records[0].command.is_empty());
    assert!(records[0].executable.is_some());
}

#[cfg(windows)]
#[test]
#[ignore = "requires the process table of the running system"]
fn it_windows_snapshot_lists_the_current_process() {
    let own = std::process::id();
    let entries = super::windows::snapshot().unwrap();
    let entry = entries.iter().find(|entry| entry.pid == own).unwrap();
    assert!(!entry.name.is_empty());
    assert!(entry.threads >= 1);
}

#[cfg(windows)]
#[test]
#[ignore = "requires the process table of the running system"]
fn it_windows_reads_the_current_process_with_account_and_executable() {
    let ctx = context(SystemClock::new(), Demand::default(), CancelToken::new());
    let own = u64::from(std::process::id());
    let call = with(args(), PARAM_PID, Value::Int(i64::try_from(own).unwrap()));
    let records = processes(run(&Processes::new(), &call, &ctx).unwrap());
    assert_eq!(records.len(), 1);
    assert!(records[0].executable.is_some());
    assert!(
        records[0]
            .uid
            .as_deref()
            .is_some_and(|sid| sid.starts_with("S-1-"))
    );
}

#[test]
#[ignore = "requires the process table of the running system and an idle processor core"]
fn it_processes_cpu_percent_busy_loop() {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let worker = std::thread::spawn(move || {
        let mut value = 0_u64;
        while !flag.load(Ordering::Relaxed) {
            value = std::hint::black_box(value.wrapping_add(1));
        }
    });
    std::thread::sleep(Duration::from_millis(100));
    let ctx = context(
        SystemClock::new(),
        only(&["pid", "cpu"]),
        CancelToken::new(),
    );
    let own = i64::from(std::process::id());
    let records = processes(
        run(
            &Processes::new(),
            &with(args(), PARAM_PID, Value::Int(own)),
            &ctx,
        )
        .unwrap(),
    );
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    let cpu = records[0].cpu.unwrap();
    assert!(cpu >= 50.0, "busy loop measured at {cpu}%");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
