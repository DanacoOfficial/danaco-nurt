// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::services::macos::tests
// Responsibility:   Daemon started by a calendar interval, defined in XML.
// Dependencies:     nurt_contracts, plist
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::collections::{HashMap, VecDeque};
use std::path::{Path as StdPath, PathBuf};
use std::sync::{Arc, Mutex};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::StartType;
use nurt_contracts::services::{
    ProcessHost, ProgramOutcome, ServiceChild, ServiceSpawnSpec, Services,
};
use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{CancelReason, CancelToken};
use plist::{Dictionary, Value as Plist};

use super::*;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/d7/u07/",
            $name
        ))
    };
}

const UID: u32 = 501;
const HOME: &str = "/Users/user";

/// Daemon started by a calendar interval, defined in XML.
const BACKUP_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>org.example.backup</string>
    <key>ProgramArguments</key><array><string>/usr/local/bin/backup</string><string>--quiet</string></array>
    <key>StartCalendarInterval</key><dict><key>Hour</key><integer>3</integer></dict>
    <key>UserName</key><string>backup</string>
</dict>
</plist>
"#;

/// System daemon activated by a socket and disabled by default.
const SSH_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>Label</key><string>com.openssh.sshd</string>
    <key>Program</key><string>/usr/libexec/sshd-keygen-wrapper</string>
    <key>Disabled</key><true/>
    <key>Sockets</key><dict><key>Listeners</key><dict/></dict>
</dict>
</plist>
"#;

/// Agent of the account, kept alive.
const AGENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>Label</key><string>org.example.agent</string>
    <key>ProgramArguments</key><array><string>/bin/sleep</string><string>600</string></array>
    <key>KeepAlive</key><true/>
</dict>
</plist>
"#;

fn agentd_binary() -> Vec<u8> {
    let mut dictionary = Dictionary::new();
    dictionary.insert(
        "Label".to_owned(),
        Plist::String("org.example.agentd".to_owned()),
    );
    dictionary.insert(
        "Program".to_owned(),
        Plist::String("/usr/local/libexec/agentd".to_owned()),
    );
    dictionary.insert("RunAtLoad".to_owned(), Plist::Boolean(true));
    dictionary.insert("KeepAlive".to_owned(), Plist::Dictionary(Dictionary::new()));
    let mut bytes = Vec::new();
    Plist::Dictionary(dictionary)
        .to_writer_binary(&mut bytes)
        .unwrap();
    bytes
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> WallTime {
        WallTime::UNIX_EPOCH
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: WallTime::UNIX_EPOCH,
            boot: BootId("boot".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(&self, _duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
        cancel.reason().map_or(Ok(()), Err)
    }
}

struct TestHost(HostInfo);

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<std::ffi::OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        Vec::new()
    }
}

fn host() -> Arc<TestHost> {
    Arc::new(TestHost(HostInfo {
        os: Os::Macos,
        os_version: "15.1".to_owned(),
        architecture: "aarch64".to_owned(),
        hostname: "host".to_owned(),
        user: "tester".to_owned(),
        home: HOME.into(),
        temp: "/tmp".into(),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 1,
    }))
}

/// Recorded reply of the program.
#[derive(Debug, Clone, Default)]
struct Reply {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

/// `launchctl` replaced by replies keyed by the argument line; the last
/// reply of a line is repeated, a line without replies exits with 0.
#[derive(Default)]
struct FakeLaunchctl {
    replies: Mutex<HashMap<String, VecDeque<Reply>>>,
    calls: Mutex<Vec<String>>,
}

impl FakeLaunchctl {
    fn on(self, args: &str, code: i32, stdout: &str, stderr: &str) -> Self {
        self.push(
            args,
            Reply {
                code: Some(code),
                stdout: stdout.to_owned(),
                stderr: stderr.to_owned(),
                timed_out: false,
            },
        )
    }

    fn push(self, args: &str, reply: Reply) -> Self {
        self.replies
            .lock()
            .unwrap()
            .entry(args.to_owned())
            .or_default()
            .push_back(reply);
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ProcessHost for FakeLaunchctl {
    fn resolve_program(&self, program: &str, _cwd: &StdPath) -> Result<PathBuf> {
        Ok(PathBuf::from(program))
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        assert_eq!(spec.program, LAUNCHCTL);
        assert_eq!(spec.timeout_ms, Some(CALL_TIMEOUT_MS));
        let line = spec.args.join(" ");
        self.calls.lock().unwrap().push(line.clone());
        let mut replies = self.replies.lock().unwrap();
        let reply = match replies.get_mut(&line) {
            Some(queue) if queue.len() > 1 => queue.pop_front().unwrap(),
            Some(queue) => queue.front().cloned().unwrap(),
            None => Reply {
                code: Some(0),
                ..Reply::default()
            },
        };
        Ok(ProgramOutcome {
            exit_code: reply.code,
            signal: if reply.code.is_none() && !reply.timed_out {
                Some(9)
            } else {
                None
            },
            timed_out: reply.timed_out,
            stdout: reply.stdout.into_bytes(),
            stderr: reply.stderr.into_bytes(),
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "helper processes are not used",
        ))
    }
}

/// System root with the property lists of the tests.
fn system_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let write = |path: &str, bytes: &[u8]| {
        let file = root.path().join(path.trim_start_matches('/'));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    };
    write(
        "/Library/LaunchDaemons/org.example.backup.plist",
        BACKUP_XML.as_bytes(),
    );
    write(
        "/Library/LaunchDaemons/org.example.agentd.plist",
        &agentd_binary(),
    );
    write(
        "/Library/LaunchDaemons/broken.plist",
        b"not a property list",
    );
    write("/Library/LaunchDaemons/notes.txt", b"ignored");
    write(
        "/System/Library/LaunchDaemons/ssh.plist",
        SSH_XML.as_bytes(),
    );
    write(
        &format!("{HOME}/Library/LaunchAgents/org.example.agent.plist"),
        AGENT_XML.as_bytes(),
    );
    root
}

struct Fixture {
    root: tempfile::TempDir,
    fake: Arc<FakeLaunchctl>,
    ctx: ExecCtx,
}

impl Fixture {
    fn new(fake: FakeLaunchctl) -> Self {
        let fake = Arc::new(fake);
        let ctx = ExecCtx::builder(Arc::new(FixedClock), host())
            .services(Arc::new(Services::new().with_process_host(fake.clone())))
            .build();
        Self {
            root: system_root(),
            fake,
            ctx,
        }
    }

    fn backend(&self) -> LaunchdBackend<'_> {
        LaunchdBackend::new(
            &self.ctx,
            Layout {
                root: self.root.path().to_path_buf(),
                home: HOME.to_owned(),
                uid: UID,
            },
        )
    }
}

const NOT_FOUND_STDERR: &str = "Could not find service in domain for port";

#[test]
fn launchd_plist_xml_and_binary_parsed() {
    let xml = definitions::parse(BACKUP_XML.as_bytes(), "/Library/LaunchDaemons/b.plist").unwrap();
    assert_eq!(xml.label, "org.example.backup");
    assert_eq!(xml.path, "/Library/LaunchDaemons/b.plist");
    assert_eq!(xml.program.as_deref(), Some("/usr/local/bin/backup"));
    assert_eq!(xml.arguments, ["/usr/local/bin/backup", "--quiet"]);
    assert_eq!(xml.user_name.as_deref(), Some("backup"));
    assert!(xml.on_demand && !xml.run_at_load && !xml.keep_alive && !xml.disabled);

    let binary = definitions::parse(&agentd_binary(), "/Library/LaunchDaemons/a.plist").unwrap();
    assert_eq!(binary.label, "org.example.agentd");
    assert_eq!(binary.program.as_deref(), Some("/usr/local/libexec/agentd"));
    assert!(binary.run_at_load && binary.keep_alive);

    assert!(definitions::parse(b"garbage", "/x.plist").is_err());
    let array = br#"<?xml version="1.0"?><plist version="1.0"><array/></plist>"#;
    assert!(definitions::parse(array, "/x.plist").is_err());
    let unlabeled = br#"<?xml version="1.0"?><plist version="1.0"><dict/></plist>"#;
    assert!(definitions::parse(unlabeled, "/x.plist").is_err());
}

#[test]
fn launchd_start_type_from_plist_keys_and_disabled_override() {
    let base = Definition {
        label: "org.example.x".to_owned(),
        ..Definition::default()
    };
    let with = |change: fn(&mut Definition)| {
        let mut definition = base.clone();
        change(&mut definition);
        definition
    };
    let at_load = with(|d| d.run_at_load = true);
    let kept = with(|d| d.keep_alive = true);
    let on_demand = with(|d| d.on_demand = true);
    let disabled = with(|d| {
        d.disabled = true;
        d.run_at_load = true;
    });
    assert_eq!(
        definitions::start_type(Some(&at_load), None),
        StartType::Automatic
    );
    assert_eq!(
        definitions::start_type(Some(&kept), None),
        StartType::Automatic
    );
    assert_eq!(
        definitions::start_type(Some(&on_demand), None),
        StartType::OnDemand
    );
    assert_eq!(
        definitions::start_type(Some(&base), None),
        StartType::Manual
    );
    assert_eq!(
        definitions::start_type(Some(&disabled), None),
        StartType::Disabled
    );
    assert_eq!(
        definitions::start_type(Some(&disabled), Some(false)),
        StartType::Automatic
    );
    assert_eq!(
        definitions::start_type(Some(&at_load), Some(true)),
        StartType::Disabled
    );
    assert_eq!(definitions::start_type(None, None), StartType::Unknown);
    assert_eq!(
        definitions::start_type(None, Some(true)),
        StartType::Disabled
    );
}

/// Lists the system scope against the macOS 15 fixtures.
fn system_listing() -> (Fixture, BTreeMap<String, Service>) {
    let fixture = Fixture::new(
        FakeLaunchctl::default()
            .on("print system", 0, fixture!("print-system-macos15.txt"), "")
            .on(
                "print-disabled system",
                0,
                fixture!("print-disabled-system.txt"),
                "",
            ),
    );
    let services = fixture
        .backend()
        .list(&ServiceQuery {
            scope: ServiceScope::System,
        })
        .unwrap();
    let by_name = services
        .into_iter()
        .map(|service| (service.name.clone(), service))
        .collect();
    (fixture, by_name)
}

#[test]
fn launchd_list_merges_definitions_listing_and_overrides() {
    let (fixture, by_name) = system_listing();
    assert_eq!(
        fixture.fake.calls(),
        ["print system", "print-disabled system"]
    );
    assert_eq!(
        by_name.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "com.apple.example.killed",
            "com.openssh.sshd",
            "org.example.agentd",
            "org.example.backup"
        ]
    );
    let ssh = &by_name["com.openssh.sshd"];
    assert_eq!(ssh.state, ServiceState::Running);
    assert_eq!(ssh.pid, Some(412));
    assert_eq!(ssh.start_type, StartType::OnDemand);
    assert_eq!(
        ssh.path.as_ref().map(Path::as_str),
        Some("/System/Library/LaunchDaemons/ssh.plist")
    );
    assert_eq!(ssh.manager, ServiceManager::Launchd);
    assert_eq!(ssh.scope, Some(ServiceScope::System));
}

#[test]
fn launchd_list_reports_overrides_exit_codes_and_skipped_files() {
    let (fixture, by_name) = system_listing();
    let backup = &by_name["org.example.backup"];
    assert_eq!(backup.state, ServiceState::Stopped);
    assert_eq!(backup.start_type, StartType::Disabled);
    assert_eq!(backup.account.as_deref(), Some("backup"));

    let agentd = &by_name["org.example.agentd"];
    assert_eq!(agentd.state, ServiceState::Failed);
    assert_eq!(agentd.start_type, StartType::Automatic);
    let raw = agentd.os_raw.as_ref().unwrap();
    assert_eq!(raw["last_exit_code"], Value::Int(78));
    assert_eq!(raw["keep_alive"], Value::Bool(true));

    let killed = &by_name["com.apple.example.killed"];
    assert_eq!(killed.start_type, StartType::Unknown);
    assert_eq!(killed.path, None);

    let notes = fixture.ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(
        notes[0].path.as_deref(),
        Some("/Library/LaunchDaemons/broken.plist")
    );
}

#[test]
fn launchd_list_without_services_block_warns_and_reports_unknown() {
    let fixture = Fixture::new(FakeLaunchctl::default().on(
        "print system",
        0,
        "system = {\n\ttype = system\n}\n",
        "",
    ));
    let services = fixture
        .backend()
        .list(&ServiceQuery {
            scope: ServiceScope::System,
        })
        .unwrap();
    assert_eq!(services.len(), 3);
    assert!(
        services
            .iter()
            .all(|service| service.state == ServiceState::Unknown)
    );
    let notes = fixture.ctx.take_notes();
    assert!(notes.iter().any(|note| {
        note.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("no services block"))
    }));
}

const BOOTSTRAP_IO_ERROR: &str = "Bootstrap failed: 5: Input/output error";

#[test]
fn launchd_bootstrap_exit_code_5_is_an_error_while_the_service_is_not_loaded() {
    let fixture = Fixture::new(
        FakeLaunchctl::default()
            .on(
                "print system/org.example.backup",
                EXIT_NOT_FOUND,
                "",
                NOT_FOUND_STDERR,
            )
            .on(
                "bootstrap system /Library/LaunchDaemons/org.example.backup.plist",
                5,
                "",
                BOOTSTRAP_IO_ERROR,
            ),
    );
    let error = fixture
        .backend()
        .start("org.example.backup", ServiceScope::System)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Service);
    assert_eq!(
        fixture.fake.calls(),
        [
            "print system/org.example.backup",
            "bootstrap system /Library/LaunchDaemons/org.example.backup.plist",
            "print system/org.example.backup",
        ]
    );
}

#[test]
fn launchd_bootstrap_exit_code_5_is_success_when_the_service_is_loaded() {
    let fixture = Fixture::new(
        FakeLaunchctl::default()
            .on(
                "print system/org.example.backup",
                EXIT_NOT_FOUND,
                "",
                NOT_FOUND_STDERR,
            )
            .on(
                "print system/org.example.backup",
                0,
                fixture!("print-service-running.txt"),
                "",
            )
            .on(
                "bootstrap system /Library/LaunchDaemons/org.example.backup.plist",
                5,
                "",
                BOOTSTRAP_IO_ERROR,
            ),
    );
    fixture
        .backend()
        .start("org.example.backup", ServiceScope::System)
        .unwrap();
    assert_eq!(
        fixture.fake.calls(),
        [
            "print system/org.example.backup",
            "bootstrap system /Library/LaunchDaemons/org.example.backup.plist",
            "print system/org.example.backup",
            "kickstart system/org.example.backup",
        ]
    );
}

#[test]
fn launchd_start_bootstrap_then_kickstart_argv() {
    let fixture = Fixture::new(FakeLaunchctl::default().on(
        "print system/org.example.backup",
        EXIT_NOT_FOUND,
        "",
        NOT_FOUND_STDERR,
    ));
    fixture
        .backend()
        .start("org.example.backup", ServiceScope::System)
        .unwrap();
    assert_eq!(
        fixture.fake.calls(),
        [
            "print system/org.example.backup",
            "bootstrap system /Library/LaunchDaemons/org.example.backup.plist",
            "kickstart system/org.example.backup",
        ]
    );
}

#[test]
fn launchd_restart_uses_kickstart_k() {
    let fixture = Fixture::new(FakeLaunchctl::default().on(
        "print system/com.openssh.sshd",
        0,
        fixture!("print-service-running.txt"),
        "",
    ));
    fixture
        .backend()
        .restart("com.openssh.sshd", ServiceScope::System)
        .unwrap();
    assert_eq!(
        fixture.fake.calls(),
        [
            "print system/com.openssh.sshd",
            "kickstart -k system/com.openssh.sshd"
        ]
    );
}

#[test]
fn launchd_disable_disable_then_bootout() {
    let fixture = Fixture::new(FakeLaunchctl::default().on(
        "print system/org.example.agentd",
        0,
        fixture!("print-service-failed.txt"),
        "",
    ));
    fixture
        .backend()
        .disable("org.example.agentd", ServiceScope::System)
        .unwrap();
    assert_eq!(
        fixture.fake.calls(),
        [
            "print system/org.example.agentd",
            "disable system/org.example.agentd",
            "bootout system/org.example.agentd",
        ]
    );

    let unloaded = Fixture::new(FakeLaunchctl::default().on(
        "print system/org.example.backup",
        EXIT_NOT_FOUND,
        "",
        NOT_FOUND_STDERR,
    ));
    unloaded
        .backend()
        .disable("org.example.backup", ServiceScope::System)
        .unwrap();
    assert_eq!(
        unloaded.fake.calls(),
        [
            "print system/org.example.backup",
            "disable system/org.example.backup"
        ]
    );
}

#[test]
fn launchd_enable_bootstraps_unloaded_service() {
    let fixture = Fixture::new(FakeLaunchctl::default().on(
        "print system/org.example.backup",
        EXIT_NOT_FOUND,
        "",
        NOT_FOUND_STDERR,
    ));
    fixture
        .backend()
        .enable("org.example.backup", ServiceScope::System)
        .unwrap();
    assert_eq!(
        fixture.fake.calls(),
        [
            "print system/org.example.backup",
            "enable system/org.example.backup",
            "bootstrap system /Library/LaunchDaemons/org.example.backup.plist",
        ]
    );

    let loaded = Fixture::new(FakeLaunchctl::default().on(
        "print system/com.openssh.sshd",
        0,
        fixture!("print-service-running.txt"),
        "",
    ));
    loaded
        .backend()
        .enable("com.openssh.sshd", ServiceScope::System)
        .unwrap();
    assert_eq!(
        loaded.fake.calls(),
        [
            "print system/com.openssh.sshd",
            "enable system/com.openssh.sshd"
        ]
    );
}

#[test]
fn launchd_stop_kills_or_boots_out_keep_alive_services() {
    let fixture = Fixture::new(FakeLaunchctl::default());
    let backend = fixture.backend();
    backend
        .stop("org.example.backup", ServiceScope::System)
        .unwrap();
    backend
        .stop("org.example.agentd", ServiceScope::System)
        .unwrap();
    assert_eq!(
        fixture.fake.calls(),
        [
            "kill SIGTERM system/org.example.backup",
            "bootout system/org.example.agentd"
        ]
    );

    let unloaded = Fixture::new(FakeLaunchctl::default().on(
        "kill SIGTERM system/org.example.backup",
        EXIT_NOT_FOUND,
        "",
        NOT_FOUND_STDERR,
    ));
    assert!(
        unloaded
            .backend()
            .stop("org.example.backup", ServiceScope::System)
            .is_ok()
    );
    let unknown = Fixture::new(FakeLaunchctl::default().on(
        "kill SIGTERM system/org.example.none",
        EXIT_NOT_FOUND,
        "",
        NOT_FOUND_STDERR,
    ));
    assert_eq!(
        unknown
            .backend()
            .stop("org.example.none", ServiceScope::System)
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
}

#[test]
fn launchd_exit_113_not_found() {
    let fixture = Fixture::new(FakeLaunchctl::default().on(
        "print system/org.example.none",
        EXIT_NOT_FOUND,
        "",
        NOT_FOUND_STDERR,
    ));
    let backend = fixture.backend();
    for error in [
        backend
            .get("org.example.none", ServiceScope::System)
            .unwrap_err(),
        backend
            .status("org.example.none", ServiceScope::System)
            .unwrap_err(),
        backend
            .start("org.example.none", ServiceScope::System)
            .unwrap_err(),
        backend
            .enable("org.example.none", ServiceScope::System)
            .unwrap_err(),
        backend
            .disable("org.example.none", ServiceScope::System)
            .unwrap_err(),
    ] {
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert_eq!(error.details["manager"], "launchd");
    }
}

#[test]
fn launchd_sip_protected_permission() {
    let fixture = Fixture::new(
        FakeLaunchctl::default()
            .on(
                "print system/com.openssh.sshd",
                0,
                fixture!("print-service-running.txt"),
                "",
            )
            .on(
                "kickstart -k system/com.openssh.sshd",
                EXIT_NOT_PERMITTED,
                "",
                "Could not kickstart service \"com.openssh.sshd\": 1: Operation not permitted",
            ),
    );
    let error = fixture
        .backend()
        .restart("com.openssh.sshd", ServiceScope::System)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Permission);
    assert!(error.message.contains("System Integrity Protection"));
}

#[test]
fn launchd_exit_codes_from_fixture() {
    let table: toml::Table = fixture!("exit-codes.toml").parse().unwrap();
    let cases = table["case"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let action = match case["action"].as_str().unwrap() {
            "print" => Action::Print,
            "bootstrap" => Action::Bootstrap,
            "bootout" => Action::Bootout,
            "kickstart" => Action::Kickstart,
            "kill" => Action::Kill,
            "enable" => Action::Enable,
            "disable" => Action::Disable,
            other => panic!("unknown action {other}"),
        };
        let output = Output {
            code: i32::try_from(case["exit"].as_integer().unwrap()).unwrap(),
            stdout: String::new(),
            stderr: case["stderr"].as_str().unwrap().to_owned(),
        };
        let target = Target {
            domain: case["domain"].as_str().unwrap(),
            label: "org.example.service",
            path: case.get("path").and_then(toml::Value::as_str),
        };
        let expected = case["code"].as_str().unwrap();
        match classify(&output, action, &target) {
            Ok(()) => assert_eq!(expected, "ok", "{case}"),
            Err(error) => assert_eq!(error.code().as_str(), expected, "{case}"),
        }
    }
    let privilege = classify(
        &Output {
            code: EXIT_NOT_PERMITTED,
            stdout: "Operation not permitted".to_owned(),
            stderr: String::new(),
        },
        Action::Kickstart,
        &Target {
            domain: SYSTEM_DOMAIN,
            label: "org.example.backup",
            path: None,
        },
    )
    .unwrap_err();
    assert_eq!(privilege.details["required"], "root");
}

#[test]
fn launchd_gui_domain_fallback_to_user_domain() {
    let fixture = Fixture::new(
        FakeLaunchctl::default()
            .on(
                "print gui/501",
                125,
                "",
                "Bad request.\nCould not find domain for port",
            )
            .on(
                "print user/501",
                0,
                "user/501 = {\n\tservices = {\n\t\t     733      -     org.example.agent\n\t}\n}\n",
                "",
            )
            .on(
                "print-disabled user/501",
                0,
                "disabled services = {\n}\n",
                "",
            ),
    );
    let backend = fixture.backend();
    let services = backend
        .list(&ServiceQuery {
            scope: ServiceScope::User,
        })
        .unwrap();
    let agent = services
        .iter()
        .find(|service| service.name == "org.example.agent")
        .unwrap();
    assert_eq!(agent.state, ServiceState::Running);
    assert_eq!(agent.scope, Some(ServiceScope::User));
    assert_eq!(agent.start_type, StartType::Automatic);
    assert_eq!(
        agent.path.as_ref().map(Path::as_str),
        Some("/Users/user/Library/LaunchAgents/org.example.agent.plist")
    );
    backend
        .restart("org.example.agent", ServiceScope::User)
        .unwrap();
    assert_eq!(
        fixture.fake.calls(),
        [
            "print gui/501",
            "print user/501",
            "print-disabled user/501",
            "print user/501/org.example.agent",
            "kickstart -k user/501/org.example.agent",
        ]
    );
}

#[test]
fn launchd_get_and_status_read_service_detail() {
    let fixture = Fixture::new(
        FakeLaunchctl::default()
            .on(
                "print system/org.example.agentd",
                0,
                fixture!("print-service-failed.txt"),
                "",
            )
            .on(
                "print system/org.example.backup",
                EXIT_NOT_FOUND,
                "",
                NOT_FOUND_STDERR,
            )
            .on(
                "print system/com.openssh.sshd",
                0,
                fixture!("print-service-running.txt"),
                "",
            )
            .on(
                "print-disabled system",
                0,
                fixture!("print-disabled-system.txt"),
                "",
            ),
    );
    let backend = fixture.backend();
    assert_eq!(backend.manager(), ServiceManager::Launchd);
    let agentd = backend
        .get("org.example.agentd", ServiceScope::System)
        .unwrap();
    assert_eq!(agentd.state, ServiceState::Failed);
    assert_eq!(agentd.state_raw.as_deref(), Some("not running"));
    let status = backend
        .status("org.example.agentd", ServiceScope::System)
        .unwrap();
    assert_eq!(status.state, ServiceState::Failed);
    assert_eq!(status.result.as_deref(), Some("78"));

    let backup = backend
        .get("org.example.backup", ServiceScope::System)
        .unwrap();
    assert_eq!(backup.state, ServiceState::Stopped);
    assert_eq!(backup.state_raw.as_deref(), Some(STATE_NOT_LOADED));
    assert_eq!(backup.start_type, StartType::Disabled);
    let status = backend
        .status("org.example.backup", ServiceScope::System)
        .unwrap();
    assert_eq!(status.state_raw.as_deref(), Some(STATE_NOT_LOADED));

    let ssh = backend
        .get("com.openssh.sshd", ServiceScope::System)
        .unwrap();
    assert_eq!(ssh.state, ServiceState::Running);
    assert_eq!(ssh.start_type, StartType::OnDemand);
    let raw = ssh.os_raw.unwrap();
    assert_eq!(
        raw["program"],
        Value::Text("/usr/libexec/sshd-keygen-wrapper".to_owned())
    );
}

#[test]
fn launchd_retries_in_progress_and_reports_timeouts() {
    let fixture = Fixture::new(
        FakeLaunchctl::default()
            .on(
                "kickstart -k system/org.example.backup",
                EXIT_IN_PROGRESS,
                "",
                "",
            )
            .on("kickstart -k system/org.example.backup", 0, "", "")
            .on(
                "print system/org.example.backup",
                0,
                fixture!("print-service-signal.txt"),
                "",
            )
            .push(
                "print system/org.example.agentd",
                Reply {
                    timed_out: true,
                    ..Reply::default()
                },
            )
            .push("print system/com.openssh.sshd", Reply::default()),
    );
    let backend = fixture.backend();
    backend
        .restart("org.example.backup", ServiceScope::System)
        .unwrap();
    assert_eq!(
        fixture
            .fake
            .calls()
            .iter()
            .filter(|call| call.starts_with("kickstart"))
            .count(),
        2
    );
    let timeout = backend
        .status("org.example.agentd", ServiceScope::System)
        .unwrap_err();
    assert_eq!(timeout.code(), ErrorCode::Timeout);
    let signalled = backend
        .status("com.openssh.sshd", ServiceScope::System)
        .unwrap_err();
    assert_eq!(signalled.code(), ErrorCode::Service);
    assert_eq!(signalled.details["os_code"], "signal 9");
}

#[test]
fn launchd_missing_process_host_external_missing() {
    let ctx = ExecCtx::builder(Arc::new(FixedClock), host()).build();
    let root = system_root();
    let backend = LaunchdBackend::new(
        &ctx,
        Layout {
            root: root.path().to_path_buf(),
            home: HOME.to_owned(),
            uid: UID,
        },
    );
    let error = backend
        .status("org.example.backup", ServiceScope::System)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    assert_eq!(error.details["program"], "launchctl");
}

#[test]
fn launchd_rejects_labels_that_are_not_labels() {
    let fixture = Fixture::new(FakeLaunchctl::default());
    let backend = fixture.backend();
    for name in [
        "",
        "system/org.example.backup",
        "org example",
        "org.example\n",
    ] {
        let error = backend.status(name, ServiceScope::System).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{name:?}");
    }
    assert!(fixture.fake.calls().is_empty());
}

#[test]
fn launchd_stage_spec_external_marks_launchctl_x() {
    let path = format!(
        "{}/../../catalog/administration.toml",
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR"))
    );
    let catalog: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
    let commands = catalog["command"].as_array().unwrap();
    for name in ["services", "start", "restart", "enable", "disable"] {
        let entry = commands
            .iter()
            .find(|command| command["name"].as_str() == Some(name))
            .unwrap()
            .clone();
        let spec: StageSpec = entry.try_into().unwrap();
        let external = spec.external.unwrap();
        assert_eq!(external.kind, Implementation::SystemProgram, "{name}");
        assert_eq!(external.programs, [PROGRAM_NAME], "{name}");
        assert_eq!(external.when, None, "{name}");
    }
    assert!(LAUNCHCTL.ends_with(PROGRAM_NAME));
}

#[test]
fn launchd_denied_outside_the_system_domain_is_not_a_privilege_error() {
    let output = Output {
        code: EXIT_NOT_PERMITTED,
        stdout: String::new(),
        stderr: "Operation not permitted".to_owned(),
    };
    let target = Target {
        domain: "gui/501",
        label: "org.example.agent",
        path: Some("/Users/user/Library/LaunchAgents/org.example.agent.plist"),
    };
    let error = classify(&output, Action::Kickstart, &target).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Permission);
    assert!(error.message.contains("launchd denied"));
    assert!(error.details.get("required").is_none());
    let protected = Target {
        path: Some("/System/Library/LaunchAgents/com.apple.example.plist"),
        ..target
    };
    let error = classify(&output, Action::Kickstart, &protected).unwrap_err();
    assert!(error.message.contains("System Integrity Protection"));
}

#[test]
fn launchd_output_message_prefers_stderr_and_falls_back_to_stdout() {
    let output = |stdout: &str, stderr: &str| Output {
        code: 1,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    };
    assert_eq!(output(" out \n", " err \n").message(), "err");
    assert_eq!(output(" out \n", "  \n").message(), "out");
    assert_eq!(output("", "").message(), "");
}

#[test]
fn launchd_stop_of_a_service_without_a_process_succeeds_and_a_denied_stop_needs_root() {
    let idle = Fixture::new(FakeLaunchctl::default().on(
        "kill SIGTERM system/org.example.backup",
        EXIT_NO_PROCESS,
        "",
        "No such process",
    ));
    idle.backend()
        .stop("org.example.backup", ServiceScope::System)
        .unwrap();
    let denied = Fixture::new(FakeLaunchctl::default().on(
        "kill SIGTERM system/org.example.backup",
        EXIT_NOT_PERMITTED,
        "",
        "Could not signal service: 1: Operation not permitted",
    ));
    let error = denied
        .backend()
        .stop("org.example.backup", ServiceScope::System)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Privilege);
    assert_eq!(error.details["required"], "root");
}

#[test]
fn launchd_find_scans_files_with_another_name_and_rejects_a_foreign_label() {
    let fixture = Fixture::new(FakeLaunchctl::default());
    let layout = Layout {
        root: fixture.root.path().to_path_buf(),
        home: HOME.to_owned(),
        uid: UID,
    };
    let scope = ServiceScope::System;
    let scanned = definitions::find(&layout, scope, "com.openssh.sshd", &fixture.ctx).unwrap();
    assert_eq!(scanned.path, "/System/Library/LaunchDaemons/ssh.plist");
    let liar = fixture
        .root
        .path()
        .join("Library/LaunchDaemons/org.example.liar.plist");
    std::fs::write(liar, BACKUP_XML).unwrap();
    assert!(definitions::find(&layout, scope, "org.example.liar", &fixture.ctx).is_none());
    let backups: Vec<Definition> = definitions::read_all(&layout, scope, &fixture.ctx)
        .into_iter()
        .filter(|definition| definition.label == "org.example.backup")
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        backups[0].path,
        "/Library/LaunchDaemons/org.example.backup.plist"
    );
}

#[test]
fn launchd_layout_directories_and_physical_paths() {
    let layout = Layout {
        root: PathBuf::from("/tmp/root"),
        home: "/Users/user/".to_owned(),
        uid: UID,
    };
    let user = layout.directories(ServiceScope::User);
    assert_eq!(
        user.last().map(String::as_str),
        Some("/Users/user/Library/LaunchAgents")
    );
    assert_eq!(user.len(), 3);
    assert_eq!(layout.directories(ServiceScope::System).len(), 2);
    assert_eq!(
        layout.physical("/Library/LaunchDaemons/a.plist"),
        PathBuf::from("/tmp/root/Library/LaunchDaemons/a.plist")
    );
}

/// Program runner of the macOS integration tests.
#[cfg(target_os = "macos")]
struct SystemLaunchctl;

#[cfg(target_os = "macos")]
impl ProcessHost for SystemLaunchctl {
    fn resolve_program(&self, program: &str, _cwd: &StdPath) -> Result<PathBuf> {
        Ok(PathBuf::from(program))
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        let output = std::process::Command::new(&spec.program)
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .output()
            .map_err(|error| Error::new(ErrorCode::Spawn, error.to_string()))?;
        Ok(ProgramOutcome {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "helper processes are not used",
        ))
    }
}

#[cfg(target_os = "macos")]
fn system_ctx() -> ExecCtx {
    ExecCtx::builder(Arc::new(FixedClock), host())
        .services(Arc::new(
            Services::new().with_process_host(Arc::new(SystemLaunchctl)),
        ))
        .build()
}

#[cfg(target_os = "macos")]
fn system_backend(ctx: &ExecCtx) -> LaunchdBackend<'_> {
    LaunchdBackend::new(
        ctx,
        Layout {
            root: PathBuf::from("/"),
            home: ctx.host().info().home.to_string_lossy().into_owned(),
            uid: rustix::process::getuid().as_raw(),
        },
    )
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires macOS with launchd"]
fn it_launchd_lists_system_daemons() {
    if !StdPath::new(LAUNCHCTL).exists() {
        return;
    }
    let ctx = system_ctx();
    let backend = backend(&ctx);
    let services = backend
        .list(&ServiceQuery {
            scope: ServiceScope::System,
        })
        .unwrap();
    assert!(
        services
            .iter()
            .any(|service| service.name.starts_with("com.apple."))
    );
    assert!(
        services
            .iter()
            .any(|service| service.state == ServiceState::Running)
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires macOS with a logged-in user session"]
fn it_launchd_test_agent_start_restart_disable() {
    const LABEL: &str = "org.example.nurt-test";
    if !StdPath::new(LAUNCHCTL).exists() {
        return;
    }
    let ctx = system_ctx();
    let home = ctx.host().info().home.clone();
    let agents = home.join("Library/LaunchAgents");
    let plist = agents.join(format!("{LABEL}.plist"));
    if plist.exists() || std::fs::create_dir_all(&agents).is_err() {
        return;
    }
    let mut dictionary = Dictionary::new();
    dictionary.insert("Label".to_owned(), Plist::String(LABEL.to_owned()));
    dictionary.insert(
        "ProgramArguments".to_owned(),
        Plist::Array(vec![
            Plist::String("/bin/sleep".to_owned()),
            Plist::String("600".to_owned()),
        ]),
    );
    Plist::Dictionary(dictionary).to_file_xml(&plist).unwrap();
    let backend = system_backend(&ctx);
    let wait_running = || {
        for _ in 0..50 {
            if backend
                .status(LABEL, ServiceScope::User)
                .is_ok_and(|status| status.state == ServiceState::Running)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    };
    let started = backend.start(LABEL, ServiceScope::User).is_ok() && wait_running();
    let restarted = backend.restart(LABEL, ServiceScope::User).is_ok() && wait_running();
    let disabled = backend.disable(LABEL, ServiceScope::User).is_ok();
    let start_type = backend
        .get(LABEL, ServiceScope::User)
        .ok()
        .map(|service| service.start_type);
    let cleaned = backend.enable(LABEL, ServiceScope::User).is_ok()
        && backend.stop(LABEL, ServiceScope::User).is_ok();
    let _removed = std::fs::remove_file(&plist);
    let domain = backend.domain(ServiceScope::User).unwrap_or_default();
    let _unloaded = SystemLaunchctl.run_program(
        &ProgramSpec::new(
            LAUNCHCTL,
            vec!["bootout".to_owned(), format!("{domain}/{LABEL}")],
            "/",
        ),
        &CancelToken::new(),
    );
    assert!(started && restarted && disabled && cleaned);
    assert_eq!(start_type, Some(StartType::Disabled));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
