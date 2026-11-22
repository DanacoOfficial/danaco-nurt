// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::record::{
    Account, EnvironmentVariable, HostInfo as HostRecord, Process, ServiceAvailability,
};
use nurt_contracts::services::Services;
use nurt_contracts::stage::{ExecCtx, StageImpl};
use nurt_contracts::value::{Record, Value};

use super::service_state::{self, Probe};
use super::test_support::{self, FixedClock, TestHost};
use super::{HostStage, SysRoot, procfs};

/// System tree with `os-release`, `meminfo`, `uptime` and `stat`.
fn fixture_root() -> SysRoot {
    SysRoot::new(test_support::fixtures().join("sysroot"))
}

fn host_record(records: Vec<Record>) -> HostRecord {
    assert_eq!(records.len(), 1);
    HostRecord::try_from(records.into_iter().next().unwrap()).unwrap()
}

#[test]
fn host_parses_os_release_quoted_values() {
    let text = "# comment\nNAME=\"Debian GNU/Linux\"\nID=debian\n\
                PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nVERSION_ID='13'\n\
                VARIANT=\"Say \\\"hi\\\"\"\nBROKEN\n";
    let release = procfs::parse_os_release(text);
    assert_eq!(release.id.as_deref(), Some("debian"));
    assert_eq!(release.version_id.as_deref(), Some("13"));
    assert_eq!(
        release.pretty_name.as_deref(),
        Some("Debian GNU/Linux 13 (trixie)")
    );

    let from_file = fixture_root().read_to_string("/etc/os-release").unwrap();
    let release = procfs::parse_os_release(&from_file);
    assert_eq!(release.version_id.as_deref(), Some("13"));
    assert_eq!(
        release.pretty_name.as_deref(),
        Some("Debian GNU/Linux 13 (trixie)")
    );
}

#[test]
fn host_meminfo_uptime_from_sysroot() {
    let root = fixture_root();
    let (total, available) = procfs::parse_meminfo(&root.read_to_string("/proc/meminfo").unwrap());
    assert_eq!(total, Some(16_000_000 * 1024));
    assert_eq!(available, Some(8_000_000 * 1024));
    let uptime = procfs::parse_uptime(&root.read_to_string("/proc/uptime").unwrap());
    assert_eq!(uptime, Some(Duration::from_millis(3_600_500)));
    let boot = procfs::parse_btime(&root.read_to_string("/proc/stat").unwrap());
    assert_eq!(boot, Some(1_700_000_000));
    assert_eq!(procfs::parse_uptime(""), None);
    assert_eq!(procfs::parse_meminfo("MemTotal: x kB\n"), (None, None));
}

#[test]
fn host_virtualization_detects_container_markers() {
    let dir = tempfile::tempdir().unwrap();
    let root = SysRoot::new(dir.path());
    assert_eq!(procfs::detect_virtualization(&root), None);

    let dmi = dir.path().join("sys/class/dmi/id");
    std::fs::create_dir_all(&dmi).unwrap();
    std::fs::write(dmi.join("sys_vendor"), "QEMU\n").unwrap();
    assert_eq!(
        procfs::detect_virtualization(&root).as_deref(),
        Some("qemu")
    );

    std::fs::write(dmi.join("sys_vendor"), "Microsoft Corporation\n").unwrap();
    std::fs::write(dmi.join("product_name"), "Virtual Machine\n").unwrap();
    assert_eq!(
        procfs::detect_virtualization(&root).as_deref(),
        Some("hyperv")
    );

    std::fs::create_dir_all(dir.path().join("proc/1")).unwrap();
    std::fs::write(
        dir.path().join("proc/1/cgroup"),
        "0::/system.slice/docker/4f1c2b.scope\n",
    )
    .unwrap();
    assert_eq!(
        procfs::detect_virtualization(&root).as_deref(),
        Some("docker")
    );

    std::fs::create_dir_all(dir.path().join("run/systemd")).unwrap();
    std::fs::write(dir.path().join("run/systemd/container"), "lxc-libvirt\n").unwrap();
    assert_eq!(procfs::detect_virtualization(&root).as_deref(), Some("lxc"));

    std::fs::write(dir.path().join(".dockerenv"), "").unwrap();
    assert_eq!(
        procfs::detect_virtualization(&root).as_deref(),
        Some("docker")
    );
}

fn heartbeat(pid: u32, beat: jiff::Timestamp) -> String {
    format!(r#"{{"pid":{pid},"started":"{beat}","beat":"{beat}","version":"0.1.0"}}"#)
}

fn probe<'a>(root: &'a SysRoot, state: &Path, home: &Path, now: jiff::Timestamp) -> Probe<'a> {
    Probe {
        root,
        state_dir: Some(state.to_path_buf()),
        home: home.to_path_buf(),
        config_home: None,
        now: WallTime::from_unix_nanos(now.as_nanosecond()),
        max_age: Duration::from_millis(2000),
    }
}

#[test]
fn host_service_heartbeat_age_limits() {
    let beat: jiff::Timestamp = "2026-11-22T10:00:00Z".parse().unwrap();
    let text = heartbeat(42, beat);
    let at = |offset_ms: i64| {
        let now = beat
            .checked_add(jiff::SignedDuration::from_millis(offset_ms))
            .unwrap();
        WallTime::from_unix_nanos(now.as_nanosecond())
    };
    let limit = Duration::from_millis(2000);
    assert_eq!(
        service_state::heartbeat_pid_if_fresh(&text, at(1500), limit),
        Some(42)
    );
    assert_eq!(
        service_state::heartbeat_pid_if_fresh(&text, at(2500), limit),
        None
    );
    assert_eq!(
        service_state::heartbeat_pid_if_fresh("{}", at(0), limit),
        None
    );
    assert!(service_state::is_pong(r#"{"v":1,"ok":true}"#));
    assert!(!service_state::is_pong(r#"{"v":2,"ok":true}"#));
    assert_eq!(service_state::ping_request(), r#"{"v":1,"op":"ping"}"#);
}

#[cfg(unix)]
#[test]
fn host_service_running_fresh_heartbeat_and_ping() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use super::service_state::{Reply, Request};

    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let pid = std::process::id();
    std::fs::create_dir_all(dir.path().join(format!("proc/{pid}"))).unwrap();
    let now = jiff::Timestamp::now();
    std::fs::write(
        state.join(service_state::HEARTBEAT_FILE),
        heartbeat(pid, now),
    )
    .unwrap();

    let listener = UnixListener::bind(state.join(service_state::SOCKET_FILE)).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        let request: Request = serde_json::from_str(&line).unwrap();
        let reply = Reply {
            v: request.v,
            ok: request.op == service_state::PING,
        };
        let mut writer = &stream;
        writeln!(writer, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
    });

    let root = SysRoot::new(dir.path());
    let state_of = service_state::availability(&probe(&root, &state, dir.path(), now));
    server.join().unwrap();
    assert_eq!(state_of, ServiceAvailability::Running);
}

#[cfg(target_os = "linux")]
#[test]
fn host_service_stopped_stale_heartbeat() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let pid = std::process::id();
    std::fs::create_dir_all(dir.path().join(format!("proc/{pid}"))).unwrap();
    let now = jiff::Timestamp::now();
    let stale = now
        .checked_sub(jiff::SignedDuration::from_secs(30))
        .unwrap();
    std::fs::write(
        state.join(service_state::HEARTBEAT_FILE),
        heartbeat(pid, stale),
    )
    .unwrap();
    let units = dir.path().join("etc/systemd/system");
    std::fs::create_dir_all(&units).unwrap();
    std::fs::write(units.join(service_state::SYSTEMD_UNIT), "[Service]\n").unwrap();

    let root = SysRoot::new(dir.path());
    let home = dir.path().join("home");
    let state_of = service_state::availability(&probe(&root, &state, &home, now));
    assert_eq!(state_of, ServiceAvailability::Stopped);

    std::fs::remove_file(units.join(service_state::SYSTEMD_UNIT)).unwrap();
    let user_units = home.join(".config/systemd/user");
    std::fs::create_dir_all(&user_units).unwrap();
    std::fs::write(user_units.join(service_state::SYSTEMD_UNIT), "[Service]\n").unwrap();
    let state_of = service_state::availability(&probe(&root, &state, &home, now));
    assert_eq!(state_of, ServiceAvailability::Stopped);
}

#[cfg(target_os = "linux")]
#[test]
fn host_service_not_installed_without_unit() {
    let dir = tempfile::tempdir().unwrap();
    let root = SysRoot::new(dir.path());
    let now = jiff::Timestamp::now();
    let state_of = service_state::availability(&probe(&root, dir.path(), dir.path(), now));
    assert_eq!(state_of, ServiceAvailability::NotInstalled);
}

#[test]
fn host_record_fields_from_context() {
    let dir = tempfile::tempdir().unwrap();
    let host = TestHost::new(dir.path()).with_var("LANG", "en_US.UTF-8");
    let ctx = test_support::context(host);
    let stage = HostStage::with_root(fixture_root());
    let args = test_support::args("host", Vec::new());
    let record = host_record(test_support::run(&stage, &args, &ctx).unwrap());
    assert_eq!(record.hostname, "build-host");
    assert_eq!(record.user, "builder");
    assert_eq!(record.architecture, "x86_64");
    assert_eq!(record.cpus, 4);
    assert_eq!(record.timezone, "America/New_York");
    assert_eq!(record.locale.as_deref(), Some("en_US.UTF-8"));
    assert_eq!(record.nurt_version, env!("CARGO_PKG_VERSION"));
    assert!(record.programs.is_none());
    assert!(record.service.is_some());
    assert!(!record.home.as_str().contains('\\'));
    #[cfg(target_os = "linux")]
    assert_linux_fixture_facts(&record);
}

/// Facts read from the synthetic tree on Linux.
#[cfg(target_os = "linux")]
fn assert_linux_fixture_facts(record: &HostRecord) {
    {
        assert_eq!(record.os_version, "13");
        assert_eq!(record.memory_total.bytes(), 16_000_000 * 1024);
        assert_eq!(
            record.uptime.map(|uptime| uptime.as_millis()),
            Some(3_600_500)
        );
        assert!(record.kernel.is_some());
        assert!(record.uid.is_some());
    }
}

#[test]
fn host_rejects_programs_that_are_not_names() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = test_support::context(TestHost::new(dir.path()));
    let args = test_support::args("host", vec![(super::PROGRAMS, Value::Int(3))]);
    let error = test_support::run(&HostStage::with_root(fixture_root()), &args, &ctx).unwrap_err();
    assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Type);
}

#[test]
fn host_versions_never_spawns_process() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = bin.join(if cfg!(windows) { "tool.exe" } else { "tool" });
    std::fs::write(&program, b"#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let host = TestHost::new(dir.path()).with_var("PATH", bin.as_os_str());
    let process_host = Arc::new(test_support::CountingProcessHost::default());
    let services = Services::new().with_process_host(process_host.clone());
    let ctx = ExecCtx::builder(Arc::new(FixedClock::now()), Arc::new(host))
        .services(Arc::new(services))
        .build();
    let args = test_support::args(
        "host",
        vec![
            (
                super::PROGRAMS,
                Value::List(vec![
                    Value::Text("tool".to_owned()),
                    Value::Text("absent-program".to_owned()),
                ]),
            ),
            (super::VERSIONS, Value::Bool(true)),
        ],
    );
    let record = host_record(test_support::run(&HostStage::new(), &args, &ctx).unwrap());
    assert_eq!(process_host.count(), 0);
    let programs = record.programs.unwrap();
    assert_eq!(programs.len(), 2);
    assert!(programs[0].found);
    assert_eq!(programs[0].version, None);
    assert_eq!(programs[0].all_paths.as_ref().map(Vec::len), Some(1));
    assert!(!programs[1].found);
    assert_eq!(programs[1].path, None);
}

#[test]
fn host_normalizes_architecture_and_paths() {
    assert_eq!(super::normalize_architecture("AMD64"), "x86_64");
    assert_eq!(super::normalize_architecture("arm64"), "aarch64");
    assert_eq!(super::normalize_architecture("i686"), "x86");
    assert_eq!(super::normalize_architecture("riscv64"), "riscv64");
    assert_eq!(
        super::canonical_path(Path::new("/home/user/a")).as_str(),
        "/home/user/a"
    );
    #[cfg(windows)]
    {
        assert_eq!(
            super::canonical_path(Path::new(r"c:\Users\user")).as_str(),
            "C:/Users/user"
        );
        assert_eq!(
            super::canonical_path(Path::new(r"\\server\share\dir")).as_str(),
            "//server/share/dir"
        );
    }
}

/// Parameters of one command in the catalog: name, type, default, position
/// and whether the parameter is required.
type Signature = Vec<(String, String, Option<String>, Option<i64>, bool)>;

fn catalog_command(catalog: &toml::Value, name: &str) -> (Signature, Vec<String>) {
    let command = catalog["command"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("`{name}` is missing in the catalog"));
    let params = command
        .get("param")
        .and_then(toml::Value::as_array)
        .map(|params| {
            params
                .iter()
                .map(|param| {
                    (
                        param["name"].as_str().unwrap().to_owned(),
                        param["type"].as_str().unwrap().to_owned(),
                        param
                            .get("default")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        param.get("position").and_then(toml::Value::as_integer),
                        param
                            .get("required")
                            .and_then(toml::Value::as_bool)
                            .unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let output = command["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kind| kind.as_str().unwrap().to_owned())
        .collect();
    (params, output)
}

fn param(
    name: &str,
    ty: &str,
    default: Option<&str>,
    position: Option<i64>,
    required: bool,
) -> (String, String, Option<String>, Option<i64>, bool) {
    (
        name.to_owned(),
        ty.to_owned(),
        default.map(str::to_owned),
        position,
        required,
    )
}

#[test]
fn catalog_signatures_match_frozen() {
    use crate::{accounts, environment, open_files};

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog/system.toml");
    let catalog: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let expected = [
        (
            super::COMMAND,
            vec![
                param(super::PROGRAMS, "list<text>", Some("[]"), None, false),
                param(super::VERSIONS, "bool", Some("false"), None, false),
            ],
            HostRecord::KIND,
        ),
        (
            environment::COMMAND,
            vec![
                param(environment::NAME, "text | regex", None, Some(1), false),
                param(
                    environment::SCOPE,
                    "enum(process|user|machine)",
                    Some("process"),
                    None,
                    false,
                ),
            ],
            EnvironmentVariable::KIND,
        ),
        (
            open_files::COMMAND,
            vec![param(open_files::PATH, "path", None, Some(1), true)],
            Process::KIND,
        ),
        (
            accounts::COMMAND,
            vec![
                param(
                    accounts::KIND,
                    "enum(user|group|all)",
                    Some("all"),
                    None,
                    false,
                ),
                param(accounts::NAME, "text", None, None, false),
            ],
            Account::KIND,
        ),
    ];
    for (command, params, kind) in expected {
        let (found, output) = catalog_command(&catalog, command);
        assert_eq!(found, params, "parameters of `{command}`");
        assert_eq!(
            output,
            vec![kind.as_str().to_owned()],
            "output of `{command}`"
        );
    }
    let stages: [Box<dyn StageImpl>; 4] = [
        Box::new(HostStage::new()),
        Box::new(environment::EnvironmentStage::new()),
        Box::new(open_files::OpenFilesStage::new()),
        Box::new(accounts::AccountsStage::new()),
    ];
    for stage in stages {
        let descriptor = stage.descriptor();
        assert!(descriptor.source, "`{}` is a source", descriptor.command);
        assert!(stage.as_read().is_some());
    }
}

#[test]
#[ignore = "requires the running operating system"]
fn it_host_fields_present_three_systems() {
    let home = std::env::temp_dir();
    let ctx = test_support::context(TestHost::new(&home));
    let args = test_support::args("host", Vec::new());
    let record = host_record(test_support::run(&HostStage::new(), &args, &ctx).unwrap());
    assert!(record.memory_total.bytes() > 0);
    assert!(record.memory_free.is_some());
    assert!(record.uptime.is_some());
    assert!(record.boot_time.is_some());
    assert!(record.kernel.is_some());
    assert!(record.uid.is_some());
    assert!(record.groups.is_some_and(|groups| !groups.is_empty()));
}

#[cfg(unix)]
#[test]
#[ignore = "requires the programs uname and id in PATH (differential test)"]
fn it_host_matches_uname_id_differential() {
    let output = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let (Some(release), Some(uid), Some(groups)) = (
        output("uname", &["-r"]),
        output("id", &["-u"]),
        output("id", &["-Gn"]),
    ) else {
        return;
    };
    let home = std::env::temp_dir();
    let ctx = test_support::context(TestHost::new(&home));
    let args = test_support::args("host", Vec::new());
    let record = host_record(test_support::run(&HostStage::new(), &args, &ctx).unwrap());
    assert_eq!(record.kernel.as_deref(), Some(release.as_str()));
    assert_eq!(record.uid.as_deref(), Some(uid.as_str()));
    let mut expected: Vec<&str> = groups.split_whitespace().collect();
    let mut found: Vec<String> = record.groups.unwrap_or_default();
    expected.sort_unstable();
    found.sort_unstable();
    assert_eq!(found, expected);
}
