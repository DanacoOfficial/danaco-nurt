// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/rules_paths.rs
// Responsibility:   Classification of system directories, home configuration and project.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Classification of system directories, home configuration and project
//! roots on Linux, macOS and Windows.

// Assertions in tests unwrap results and panic on unexpected values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::path::PathBuf;

use nurt_contracts::config::{ReportConfig, ScopeConfig};
use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};
use nurt_report::assemble::ReportContext;
use nurt_report::rules::{PathContext, is_fs_path};

fn linux() -> PathContext {
    PathContext::new(Os::Linux).with_home("/home/user")
}

fn macos() -> PathContext {
    PathContext::new(Os::Macos).with_home("/Users/user")
}

fn windows() -> PathContext {
    PathContext::new(Os::Windows).with_home("C:/Users/user")
}

fn check(context: &PathContext, rule: fn(&PathContext, &str) -> bool, cases: &[(&str, bool)]) {
    for (path, expected) in cases {
        assert_eq!(rule(context, path), *expected, "{:?} {path}", context.os());
    }
}

#[test]
fn linux_system_directories() {
    check(
        &linux(),
        PathContext::is_system_path,
        &[
            ("/etc", true),
            ("/etc/hosts", true),
            ("/usr/bin/ls", true),
            ("/usr/local/bin/tool", true),
            ("/bin/sh", true),
            ("/sbin/init", true),
            ("/lib/x86_64-linux-gnu/libc.so.6", true),
            ("/lib64/ld-linux-x86-64.so.2", true),
            ("/lib32", true),
            ("/boot/vmlinuz", true),
            ("/sys/class/net", true),
            ("/proc/1/environ", true),
            ("/etcetera/file", false),
            ("/library/docs", false),
            ("/libexec/tool", false),
            ("/Etc/hosts", false),
            ("/srv/usr/file", false),
            ("/home/user/etc/file", false),
            ("/System/Library", false),
            ("/", false),
        ],
    );
}

#[test]
fn macos_system_directories() {
    check(
        &macos(),
        PathContext::is_system_path,
        &[
            ("/System/Library/CoreServices", true),
            ("/system/library", true),
            ("/Library/Preferences/com.apple.plist", true),
            ("/private/etc/hosts", true),
            ("/etc/hosts", true),
            ("/usr/local/bin/brew", true),
            ("/bin/zsh", true),
            ("/private/var/tmp/x", false),
            ("/Users/user/Library/Caches", false),
            ("/Applications/App.app", false),
            ("/boot", false),
        ],
    );
}

#[test]
fn windows_system_directories() {
    check(
        &windows(),
        PathContext::is_system_path,
        &[
            ("C:/Windows", true),
            ("C:/Windows/System32/drivers/etc/hosts", true),
            ("c:/windows/system32", true),
            ("C:/Program Files/App/app.exe", true),
            ("C:/Program Files (x86)/App/app.exe", true),
            ("C:/ProgramData/App/config.json", true),
            ("C:/WindowsApps/x", false),
            ("C:/Users/user/Documents", false),
            ("D:/Windows/System32", false),
            ("/etc/hosts", false),
        ],
    );
}

#[test]
fn linux_home_configuration() {
    check(
        &linux(),
        PathContext::is_home_config,
        &[
            ("/home/user/.ssh", true),
            ("/home/user/.ssh/id_ed25519", true),
            ("/home/user/.gnupg/pubring.kbx", true),
            ("/home/user/.config/app/settings.toml", true),
            ("/home/user/.bashrc", true),
            ("/home/user/.zshrc", true),
            ("/home/user/.profile", true),
            ("/home/user/.bash_history", false),
            ("/home/user/.SSH/config", false),
            ("/home/user/projects/.config", false),
            ("/home/user/Library/Preferences/x.plist", false),
            ("/home/other/.ssh/config", false),
            ("/home/user", false),
            ("/home/username/.ssh", false),
        ],
    );
}

#[test]
fn macos_home_configuration() {
    check(
        &macos(),
        PathContext::is_home_config,
        &[
            ("/Users/user/Library/Preferences/com.example.plist", true),
            ("/Users/user/.ssh/config", true),
            ("/Users/user/.SSH/config", true),
            ("/users/USER/.zshrc", true),
            ("/Users/user/Library/Caches/x", false),
            ("/Library/Preferences/x.plist", false),
        ],
    );
}

#[test]
fn windows_home_configuration() {
    check(
        &windows(),
        PathContext::is_home_config,
        &[
            ("C:/Users/user/AppData/Roaming/App/settings.json", true),
            ("c:/users/user/appdata/roaming", true),
            ("C:/Users/user/.ssh/config", true),
            ("C:/Users/user/.config/git/config", true),
            ("C:/Users/user/AppData/Local/Temp/x", false),
            ("C:/Users/user/Documents/report.docx", false),
            ("C:/Users/other/AppData/Roaming/x", false),
        ],
    );
    let relocated = windows().with_app_data("D:\\Profiles\\user\\Roaming\\");
    check(
        &relocated,
        PathContext::is_home_config,
        &[
            ("D:/Profiles/user/Roaming/App/x.json", true),
            ("d:/profiles/user/roaming", true),
            ("D:/Profiles/user/Local", false),
        ],
    );
}

#[test]
fn home_configuration_needs_a_home_directory() {
    let without_home = PathContext::new(Os::Linux);
    assert_eq!(without_home.home(), None);
    assert!(!without_home.is_home_config("/home/user/.ssh/config"));
    let root_home = PathContext::new(Os::Linux).with_home("/");
    assert!(root_home.is_home_config("/.ssh/config"));
    assert!(!root_home.is_home_config("/srv/.ssh"));
}

#[test]
fn within_compares_whole_components() {
    let linux = linux();
    assert!(linux.is_within("/srv/app", "/srv/app"));
    assert!(linux.is_within("/srv/app/src/a.go", "/srv/app/"));
    assert!(!linux.is_within("/srv/application", "/srv/app"));
    assert!(!linux.is_within("/srv", "/srv/app"));
    assert!(!linux.is_within("/SRV/app", "/srv/app"));
    assert!(macos().is_within("/SRV/app", "/srv/app"));
    assert!(windows().is_within("c:/src/app/x", "C:/Src/App"));
    assert!(!windows().is_within("D:/src/app/x", "C:/src/app"));
}

#[test]
fn case_folding_follows_the_operating_system() {
    assert!(!linux().folds_case());
    assert!(macos().folds_case());
    assert!(windows().folds_case());
    assert_eq!(PathContext::default().os(), Os::Linux);
}

#[test]
fn file_system_paths_are_told_from_provider_identifiers() {
    for path in [
        "/srv/app",
        "/",
        "C:/Users/user",
        "c:",
        "//server/share/file",
    ] {
        assert!(is_fs_path(path), "{path}");
    }
    for path in [
        "service:nginx",
        "registry:/HKCU/Software",
        "env:/PATH",
        "archive:/home/user/a.zip!/dir",
        "relative/path",
        "",
        "1:/x",
    ] {
        assert!(!is_fs_path(path), "{path}");
    }
}

struct FixedHost {
    info: HostInfo,
    app_data: Option<OsString>,
}

impl FixedHost {
    fn new(os: Os, home: &str, app_data: Option<&str>) -> Self {
        let path_style = if os == Os::Windows {
            PathStyle::Windows
        } else {
            PathStyle::Posix
        };
        Self {
            info: HostInfo {
                os,
                os_version: "1.0".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "host".to_owned(),
                user: "user".to_owned(),
                home: PathBuf::from(home),
                temp: PathBuf::from("/tmp"),
                path_style,
                timezone: "UTC".to_owned(),
                locale: None,
                cpus: 1,
            },
            app_data: app_data.map(OsString::from),
        }
    }
}

impl Host for FixedHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        (name == "APPDATA").then(|| self.app_data.clone()).flatten()
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.app_data
            .iter()
            .map(|value| (OsString::from("APPDATA"), value.clone()))
            .collect()
    }
}

#[test]
fn context_from_host_reads_home_and_application_data() {
    let host = FixedHost::new(Os::Windows, "C:\\Users\\user", Some("E:\\Roaming\\user"));
    let context = PathContext::from_host(&host);
    assert_eq!(context.os(), Os::Windows);
    assert_eq!(context.home(), Some("C:/Users/user"));
    assert!(context.is_home_config("E:/Roaming/user/App/x"));
    assert!(context.is_home_config("C:/Users/user/AppData/Roaming/x"));

    let empty = FixedHost::new(Os::Windows, "C:/Users/user", Some(""));
    assert!(!PathContext::from_host(&empty).is_home_config("E:/Roaming/user/x"));

    let linux = FixedHost::new(Os::Linux, "/home/user/", Some("/srv/roaming"));
    let context = PathContext::from_host(&linux);
    assert_eq!(context.home(), Some("/home/user"));
    assert!(!context.is_home_config("/srv/roaming/x"));
    assert!(context.is_home_config("/home/user/.gnupg"));
}

#[test]
fn report_context_normalizes_project_roots() {
    let context = ReportContext::new(ReportConfig::default(), windows()).with_project_roots([
        "C:\\Src\\App\\",
        "",
        "D:/lib/",
    ]);
    assert_eq!(context.project_roots(), ["C:/Src/App", "D:/lib"]);
    assert_eq!(context.paths(), &windows());
    assert_eq!(context.config(), &ReportConfig::default());

    let mut scope = ScopeConfig::default();
    scope.project_roots = vec!["/srv/app/".to_owned(), "/".to_owned()];
    let context = ReportContext::default().with_scope(&scope);
    assert_eq!(context.project_roots(), ["/srv/app", "/"]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
