// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Package commands on the running system.
//!
//! The tests read the real package databases and are ignored by default.
//! Each test checks its precondition and passes without assertions when the
//! manager is absent. Programs of the managers serve only as the reference
//! of a comparison and are never started by the commands.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::record::{Package, PackageManager};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, ReadStage, StageData, StageInput,
};
use nurt_contracts::value::Value;
use nurt_sys::package::{PackageEnv, PackageList};

struct SystemClock(Instant);

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i128::try_from(since.as_nanos()).unwrap_or(i128::MAX)
            });
        WallTime(since_epoch)
    }
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("system".to_owned()),
            since_boot_with_suspend: self.monotonic(),
            since_boot_active: self.monotonic(),
        }
    }
    fn sleep(&self, duration: Duration, _cancel: &CancelToken) -> Result<(), CancelReason> {
        std::thread::sleep(duration);
        Ok(())
    }
}

/// Host built from the environment of the test process.
struct ProcessHost(HostInfo);

impl ProcessHost {
    fn current() -> Self {
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        };
        let home_variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        Self(HostInfo {
            os,
            os_version: String::new(),
            architecture: std::env::consts::ARCH.to_owned(),
            hostname: String::new(),
            user: String::new(),
            home: std::env::var_os(home_variable)
                .map(PathBuf::from)
                .unwrap_or_default(),
            temp: std::env::temp_dir(),
            path_style: if cfg!(windows) {
                PathStyle::Windows
            } else {
                PathStyle::Posix
            },
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        })
    }
}

impl Host for ProcessHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }
    fn env_var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }
}

/// Packages listed by `package.list` for `manager`; `None` when the command
/// fails or returns a record that is not a package.
fn list(manager: &str) -> Option<Vec<Package>> {
    let ctx = ExecCtx::builder(
        Arc::new(SystemClock(Instant::now())),
        Arc::new(ProcessHost::current()),
    )
    .build();
    let stage = PackageList::new(Arc::new(PackageEnv::system()));
    let args = BoundArgs::new("package.list").with(
        "manager",
        ArgValue::Value(Value::Enum(manager.to_owned())),
        false,
    );
    let input = StageInput {
        data: StageData::None,
        header: None,
        order: None,
        render: None,
    };
    let output = stage.execute(&args, input, &ctx).ok()?;
    let StageData::Records(stream) = output.data else {
        return None;
    };
    stream
        .map(|record| {
            record
                .ok()
                .and_then(|record| Package::try_from(record).ok())
        })
        .collect()
}

fn program_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
#[ignore = "requires a Debian-based system with dpkg-query"]
fn it_apt_list_matches_dpkg_query() {
    if !Path::new("/var/lib/dpkg/status").is_file() {
        return;
    }
    let Some(reference) = program_output(
        "dpkg-query",
        &[
            "-W",
            "-f",
            "${db:Status-Status} ${Package}:${Architecture} ${Version}\n",
        ],
    ) else {
        return;
    };
    let expected: BTreeSet<String> = reference
        .lines()
        .filter_map(|line| line.strip_prefix("installed "))
        .map(str::to_owned)
        .collect();
    let listed: BTreeSet<String> = list("apt")
        .expect("package.list manager: apt")
        .iter()
        .map(|package| {
            format!(
                "{}:{} {}",
                package.name,
                package.architecture.as_deref().unwrap_or_default(),
                package.version
            )
        })
        .collect();
    assert_eq!(listed, expected);
}

#[test]
#[ignore = "requires a macOS runner with Homebrew"]
fn it_brew_list_on_macos_runner() {
    let Some(reference) = program_output("brew", &["list", "--formula", "-1"]) else {
        return;
    };
    let listed: BTreeSet<String> = list("brew")
        .expect("package.list manager: brew")
        .into_iter()
        .map(|package| package.name)
        .collect();
    for formula in reference.lines().filter(|line| !line.is_empty()) {
        assert!(listed.contains(formula), "{formula} is not listed");
    }
}

#[test]
#[ignore = "requires Windows with App Installer"]
fn it_winget_list_on_windows() {
    if !cfg!(windows) || std::env::var_os("LOCALAPPDATA").is_none() {
        return;
    }
    for package in list("winget").expect("package.list manager: winget") {
        assert_eq!(package.manager, PackageManager::Winget);
        assert!(package.installed);
    }
}

#[test]
#[ignore = "requires Windows with Scoop or Chocolatey"]
fn it_scoop_choco_list_if_present() {
    if !cfg!(windows) {
        return;
    }
    for (manager, kind) in [
        ("scoop", PackageManager::Scoop),
        ("choco", PackageManager::Choco),
    ] {
        if program_output(manager, &["--version"]).is_none() {
            continue;
        }
        let packages = list(manager).expect("package.list");
        assert!(packages.iter().all(|package| package.manager == kind));
    }
}
