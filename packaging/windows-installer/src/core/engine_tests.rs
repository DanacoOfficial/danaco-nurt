// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/engine_tests.rs
// Responsibility:   Tests of the engine on Windows: install, undo, repair and the rollback of failed runs with injected faults.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the engine.
//!
//! The tests change the real registry of the user (`HKCU`), so they run one at a time. Every
//! test works in its own folder below the temporary directory and puts back what it changed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a failed step of a test has to stop the test with a message"
)]
#![allow(
    clippy::cognitive_complexity,
    reason = "a scenario test lists the state of the machine before and after one run"
)]

use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::*;
use crate::model::ConfigChoices;
use crate::registry::{Snapshot, UNINSTALL_KEY};
use crate::sys;

static SERIAL: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU32 = AtomicU32::new(0);

const VERSION: &str = "0.1.2";
const POWERSHELL_DEFAULT: &str = "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}";

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A folder of one test with the directories of a user inside it.
struct Fixture {
    base: PathBuf,
    user: UserPaths,
    denied: RefCell<Vec<PathBuf>>,
}

impl Fixture {
    fn new() -> Self {
        let number = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base =
            std::env::temp_dir().join(format!("nurt-setup-test-{}-{number}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let user = UserPaths {
            profile: base.join("user"),
            appdata: base.join("user\\AppData\\Roaming"),
            local_appdata: base.join("user\\AppData\\Local"),
            documents: base.join("user\\Documents"),
            desktop: base.join("user\\Desktop"),
        };
        fs::create_dir_all(&user.desktop).unwrap();
        Self {
            base,
            user,
            denied: RefCell::new(Vec::new()),
        }
    }

    fn bin(&self) -> PathBuf {
        self.base.join("Programs\\Nurt\\bin")
    }

    fn root(&self) -> PathBuf {
        self.base.join("Programs\\Nurt")
    }

    /// Options with every optional part that works in a folder of the test.
    fn full(&self) -> Options {
        Options {
            scope: Scope::User,
            dir: self.bin().display().to_string(),
            path: true,
            docs: true,
            start_menu: true,
            terminal_profile: true,
            terminal_default: true,
            desktop: true,
            quick_launch: true,
            completions: true,
            write_config: true,
            config: ConfigChoices::default(),
        }
    }

    /// Options with the parts that need no shell: files, registry and `PATH`.
    fn basic(&self) -> Options {
        Options {
            start_menu: false,
            terminal_profile: false,
            terminal_default: false,
            desktop: false,
            quick_launch: false,
            completions: false,
            write_config: false,
            ..self.full()
        }
    }

    fn terminal_settings(&self) -> PathBuf {
        self.user
            .local_appdata
            .join("Packages\\Microsoft.WindowsTerminal_8wekyb3d8bbwe\\LocalState\\settings.json")
    }

    fn write_terminal_settings(&self) -> String {
        let path = self.terminal_settings();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = format!(
            "{{\n  \"defaultProfile\": \"{POWERSHELL_DEFAULT}\",\n  \"profiles\": {{ \"list\": [] }}\n}}\n"
        );
        fs::write(&path, &text).unwrap();
        text
    }

    /// Denies the creation of files and folders in a directory for everyone.
    fn deny_writes(&self, directory: &Path) {
        let status = Command::new("icacls")
            .arg(directory)
            .args(["/deny", "*S-1-1-0:(OI)(CI)(WD,AD)"])
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "icacls: {}",
            String::from_utf8_lossy(&status.stdout)
        );
        self.denied.borrow_mut().push(directory.to_path_buf());
    }

    fn allow_writes(&self) {
        for directory in self.denied.borrow_mut().drain(..) {
            let _ = Command::new("icacls")
                .arg(&directory)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.allow_writes();
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// The files below a folder, relative and sorted; empty when the folder does not exist.
fn tree(root: &Path) -> Vec<String> {
    fn walk(base: &Path, directory: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(base).unwrap().display().to_string();
            if path.is_dir() {
                out.push(format!("{relative}\\"));
                walk(base, &path, out);
            } else {
                out.push(relative);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

struct Outcome {
    result: Result<Summary, Failure>,
    events: Vec<ProgressEvent>,
}

fn install_with(
    options: &Options,
    user: &UserPaths,
    cancel: &dyn Fn(&ProgressEvent) -> bool,
) -> Outcome {
    let events: RefCell<Vec<ProgressEvent>> = RefCell::new(Vec::new());
    let stop = Cell::new(false);
    let emit = |event: ProgressEvent| {
        if cancel(&event) {
            stop.set(true);
        }
        events.borrow_mut().push(event);
    };
    let cancelled = || stop.get();
    let env = Env {
        options,
        user,
        version: VERSION,
        emit: &emit,
        cancel: &cancelled,
        pace: false,
    };
    let result = install(&env);
    Outcome {
        result,
        events: events.into_inner(),
    }
}

fn install_plain(options: &Options, user: &UserPaths) -> Outcome {
    install_with(options, user, &|_| false)
}

fn undo_run(root: &Path, user: &UserPaths, remove_config: bool) -> Result<Summary, SetupError> {
    let emit = |_: ProgressEvent| {};
    undo(&UndoEnv {
        root,
        user,
        remove_config,
        emit: &emit,
        pace: false,
    })
}

fn registry_state() -> Snapshot {
    registry::capture(Scope::User, UNINSTALL_KEY).unwrap()
}

fn app_paths_state() -> Snapshot {
    registry::capture(Scope::User, registry::APP_PATHS_KEY).unwrap()
}

fn user_path() -> Option<String> {
    registry::read_path(Scope::User).unwrap()
}

fn lock(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

fn failure(outcome: Outcome) -> Failure {
    match outcome.result {
        Ok(_) => panic!("the run was expected to fail"),
        Err(failure) => failure,
    }
}

fn success(outcome: Outcome) -> Summary {
    match outcome.result {
        Ok(summary) => summary,
        Err(failure) => panic!("the run failed in {}: {}", failure.stage, failure.error),
    }
}

#[test]
fn install_writes_every_part_and_undo_removes_it() {
    let _guard = serial();
    let fixture = Fixture::new();
    let settings = fixture.write_terminal_settings();
    let (registry_before, apps_before, path_before) =
        (registry_state(), app_paths_state(), user_path());
    let options = fixture.full();

    let summary = success(install_plain(&options, &fixture.user));
    let root = fixture.root();
    for file in [
        "bin\\nurt.exe",
        "doc\\README.md",
        "doc\\CHANGELOG.md",
        "doc\\LICENSE",
        "doc\\NOTICE",
        "doc\\index.html",
        "doc\\spec\\catalog.html",
        "doc\\completions\\_nurt.ps1",
        "uninstall.exe",
        "nurt.ico",
        "setup-journal.jsonl",
    ] {
        assert!(root.join(file).is_file(), "missing {file}");
    }
    assert!(!journal::writing_path(&root).exists());
    assert!(registry_state().existed && app_paths_state().existed);
    assert!(
        user_path()
            .unwrap_or_default()
            .contains(&fixture.bin().display().to_string())
    );
    assert!(summary.path_changed);
    assert!(fixture.user.desktop.join("Danaco Nurt.lnk").is_file());
    assert!(
        fixture
            .user
            .appdata
            .join(
                "Microsoft\\Windows\\Start Menu\\Programs\\Danaco Nurt\\Uninstall Danaco Nurt.lnk"
            )
            .is_file()
    );
    assert!(
        fixture
            .user
            .appdata
            .join("Microsoft\\Internet Explorer\\Quick Launch\\Danaco Nurt.lnk")
            .is_file()
    );
    assert!(
        fixture
            .user
            .local_appdata
            .join("Microsoft\\Windows Terminal\\Fragments\\Danaco Nurt\\nurt.json")
            .is_file()
    );
    assert!(
        fs::read_to_string(fixture.terminal_settings())
            .unwrap()
            .contains("b7e0c4a2-5d1f-4c6e-9a38-2f6d0a71c5e4")
    );
    assert!(
        fs::read_to_string(fixture.user.documents.join("PowerShell\\profile.ps1"))
            .unwrap()
            .contains("_nurt.ps1")
    );
    assert!(fixture.user.appdata.join("Nurt\\nurt.toml").is_file());

    let records = journal::read(&root.join(journal::FILE_NAME)).unwrap();
    let kinds: Vec<&str> = records
        .iter()
        .map(|record| match record.entry {
            Entry::Begin { .. } => "begin",
            Entry::CreateDirs { .. } => "create_dirs",
            Entry::WriteFiles { .. } => "write_files",
            Entry::Registry { .. } => "registry",
            Entry::SetPath { .. } => "set_path",
            Entry::Shortcuts { .. } => "shortcuts",
            Entry::TerminalFragment { .. } => "terminal_fragment",
            Entry::TerminalDefault { .. } => "terminal_default",
            Entry::PowershellProfile { .. } => "powershell_profile",
            Entry::ConfigFile { .. } => "config_file",
            Entry::End { .. } => "end",
        })
        .collect();
    assert_eq!(kinds.first(), Some(&"begin"));
    assert_eq!(kinds.last(), Some(&"end"));
    for wanted in [
        "create_dirs",
        "write_files",
        "registry",
        "set_path",
        "shortcuts",
        "terminal_fragment",
        "terminal_default",
        "powershell_profile",
        "config_file",
    ] {
        assert!(kinds.contains(&wanted), "no {wanted} entry in {kinds:?}");
    }
    assert!(journal::is_complete(&records));
    assert_eq!(inspect(&root), (records.len() - 1, true));

    let undone = undo_run(&root, &fixture.user, false).unwrap();
    assert!(undone.warnings.is_empty(), "{:?}", undone.warnings);
    assert!(!root.exists(), "left: {:?}", tree(&root));
    assert_eq!(registry_state(), registry_before);
    assert_eq!(app_paths_state(), apps_before);
    assert_eq!(user_path(), path_before);
    assert!(!fixture.user.desktop.join("Danaco Nurt.lnk").exists());
    assert!(
        !fixture
            .user
            .appdata
            .join("Microsoft\\Windows\\Start Menu\\Programs\\Danaco Nurt")
            .exists()
    );
    assert!(
        !fixture
            .user
            .local_appdata
            .join("Microsoft\\Windows Terminal\\Fragments\\Danaco Nurt")
            .exists()
    );
    assert_eq!(
        fs::read_to_string(fixture.terminal_settings()).unwrap(),
        settings
    );
    assert!(
        !fixture
            .user
            .documents
            .join("PowerShell\\profile.ps1")
            .exists()
    );
    assert!(
        fixture.user.appdata.join("Nurt\\nurt.toml").is_file(),
        "the configuration stays unless asked"
    );
}

#[test]
fn undo_removes_the_configuration_on_request() {
    let _guard = serial();
    let fixture = Fixture::new();
    let options = Options {
        write_config: true,
        ..fixture.basic()
    };
    success(install_plain(&options, &fixture.user));
    assert!(fixture.user.appdata.join("Nurt\\nurt.toml").is_file());
    undo_run(&fixture.root(), &fixture.user, true).unwrap();
    assert!(!fixture.user.appdata.join("Nurt").exists());
    assert!(!fixture.root().exists());
}

#[test]
fn undo_runs_in_the_reverse_order_of_the_journal() {
    let _guard = serial();
    let fixture = Fixture::new();
    success(install_plain(&fixture.basic(), &fixture.user));
    let summary = undo_run(&fixture.root(), &fixture.user, false).unwrap();
    let order: Vec<u64> = summary.undone.iter().map(|item| item.n).collect();
    let mut sorted = order.clone();
    sorted.sort_unstable_by(|left, right| right.cmp(left));
    assert_eq!(
        order, sorted,
        "the entries are undone from the last to the first"
    );
    assert!(order.len() >= 4);
}

#[test]
fn repair_keeps_the_ownership_of_path_and_the_default_profile() {
    let _guard = serial();
    let fixture = Fixture::new();
    let settings = fixture.write_terminal_settings();
    let path_before = user_path();
    let options = fixture.full();
    success(install_plain(&options, &fixture.user));
    let first = journal::read(&fixture.root().join(journal::FILE_NAME)).unwrap();

    let recorded = recorded_options(&fixture.root()).unwrap();
    assert_eq!(recorded, options);
    let repaired = success(install_plain(&recorded, &fixture.user));
    assert!(!repaired.path_changed, "PATH already held the folder");
    let second = journal::read(&fixture.root().join(journal::FILE_NAME)).unwrap();
    for wanted in ["set_path", "terminal_default", "powershell_profile"] {
        let count = |records: &[Record]| {
            records
                .iter()
                .filter(|record| match &record.entry {
                    Entry::SetPath { .. } => wanted == "set_path",
                    Entry::TerminalDefault { .. } => wanted == "terminal_default",
                    Entry::PowershellProfile { .. } => wanted == "powershell_profile",
                    _ => false,
                })
                .count()
        };
        assert_eq!(count(&first), 1, "{wanted} in the first journal");
        assert_eq!(count(&second), 1, "{wanted} in the repaired journal");
    }
    assert!(damaged_files(&fixture.root()).is_empty());

    let undone = undo_run(&fixture.root(), &fixture.user, false).unwrap();
    assert!(undone.warnings.is_empty(), "{:?}", undone.warnings);
    assert_eq!(user_path(), path_before);
    assert_eq!(
        fs::read_to_string(fixture.terminal_settings()).unwrap(),
        settings
    );
    assert!(
        !fixture
            .user
            .documents
            .join("PowerShell\\profile.ps1")
            .exists()
    );
    assert!(
        !fixture.root().exists(),
        "left: {:?}",
        tree(&fixture.root())
    );
}

#[test]
fn damaged_files_lists_changed_and_missing_files() {
    let _guard = serial();
    let fixture = Fixture::new();
    success(install_plain(&fixture.basic(), &fixture.user));
    assert!(damaged_files(&fixture.root()).is_empty());
    fs::write(fixture.bin().join("nurt.exe"), b"changed").unwrap();
    fs::remove_file(fixture.root().join("doc\\README.md")).unwrap();
    let damaged = damaged_files(&fixture.root());
    assert_eq!(damaged.len(), 2, "{damaged:?}");
    assert!(damaged.iter().any(|path| path.ends_with("nurt.exe")));
    assert!(damaged.iter().any(|path| path.ends_with("README.md")));

    success(install_plain(
        &recorded_options(&fixture.root()).unwrap(),
        &fixture.user,
    ));
    assert!(damaged_files(&fixture.root()).is_empty());
    assert!(fixture.root().join("doc\\README.md").is_file());
    undo_run(&fixture.root(), &fixture.user, false).unwrap();
}

#[test]
fn undo_after_a_second_installation_removes_the_folders_of_the_first() {
    let _guard = serial();
    let fixture = Fixture::new();
    success(install_plain(&fixture.basic(), &fixture.user));
    success(install_plain(&fixture.basic(), &fixture.user));
    let undone = undo_run(&fixture.root(), &fixture.user, false).unwrap();
    assert!(undone.warnings.is_empty(), "{:?}", undone.warnings);
    assert!(
        !fixture.root().exists(),
        "left: {:?}",
        tree(&fixture.root())
    );
}

#[test]
fn a_run_writes_the_new_journal_beside_the_old_one() {
    let _guard = serial();
    let fixture = Fixture::new();
    success(install_plain(&fixture.basic(), &fixture.user));
    let old = fs::read(fixture.root().join(journal::FILE_NAME)).unwrap();
    let seen = Cell::new(false);
    let root = fixture.root();
    let outcome = install_with(&fixture.basic(), &fixture.user, &|event| {
        if event.stage == "copy" && journal::writing_path(&root).is_file() {
            assert_eq!(
                fs::read(root.join(journal::FILE_NAME)).unwrap(),
                old,
                "the old journal stays during the run"
            );
            seen.set(true);
        }
        false
    });
    success(outcome);
    assert!(seen.get());
    assert_ne!(
        fs::read(fixture.root().join(journal::FILE_NAME)).unwrap(),
        old
    );
    undo_run(&fixture.root(), &fixture.user, false).unwrap();
}

#[test]
fn corrupt_payload_stops_before_any_change() {
    let _guard = serial();
    let fixture = Fixture::new();
    let (registry_before, path_before) = (registry_state(), user_path());
    *payload::TEST_SUM.lock().unwrap() = Some("0".repeat(64));
    let outcome = install_plain(&fixture.full(), &fixture.user);
    *payload::TEST_SUM.lock().unwrap() = None;
    let failure = failure(outcome);
    assert_eq!(failure.error.code, "payload-corrupt");
    assert_eq!(failure.stage, "verify");
    assert!(failure.rolled_back && !failure.cancelled);
    assert!(!fixture.root().exists());
    assert_eq!(registry_state(), registry_before);
    assert_eq!(user_path(), path_before);
}

#[test]
fn cancel_after_the_registry_restores_registry_and_path() {
    let _guard = serial();
    let fixture = Fixture::new();
    let (registry_before, apps_before, path_before) =
        (registry_state(), app_paths_state(), user_path());
    let outcome = install_with(&fixture.full(), &fixture.user, &|event| {
        event.stage == "shortcuts" && event.state == StageState::Running
    });
    assert!(
        outcome
            .events
            .iter()
            .any(|event| event.state == StageState::Rollback)
    );
    let failure = failure(outcome);
    assert!(failure.cancelled && failure.rolled_back, "{failure:?}");
    assert_eq!(failure.error.code, "cancelled");
    assert!(failure.undone >= 3);
    assert!(
        !fixture.root().exists(),
        "left: {:?}",
        tree(&fixture.root())
    );
    assert_eq!(registry_state(), registry_before);
    assert_eq!(app_paths_state(), apps_before);
    assert_eq!(user_path(), path_before);
    assert!(!fixture.user.desktop.join("Danaco Nurt.lnk").exists());
}

#[test]
fn a_locked_file_rolls_back_a_fresh_installation() {
    let _guard = serial();
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.bin()).unwrap();
    fs::write(fixture.bin().join("nurt.exe"), b"in use").unwrap();
    let held = lock(&fixture.bin().join("nurt.exe"));
    let (registry_before, path_before) = (registry_state(), user_path());

    let failure = failure(install_plain(&fixture.basic(), &fixture.user));
    assert_eq!(failure.error.code, "in-use", "{:?}", failure.error);
    assert_eq!(failure.stage, "copy");
    assert!(failure.rolled_back);
    drop(held);
    assert_eq!(
        tree(&fixture.root()),
        vec!["bin\\".to_string(), "bin\\nurt.exe".to_string()]
    );
    assert_eq!(fs::read(fixture.bin().join("nurt.exe")).unwrap(), b"in use");
    assert_eq!(registry_state(), registry_before);
    assert_eq!(user_path(), path_before);
}

#[test]
fn a_locked_file_leaves_the_earlier_installation_intact() {
    let _guard = serial();
    let fixture = Fixture::new();
    success(install_plain(&fixture.full(), &fixture.user));
    let journal_before = fs::read(fixture.root().join(journal::FILE_NAME)).unwrap();
    let tree_before = tree(&fixture.root());
    let (registry_before, path_before) = (registry_state(), user_path());
    let exe_sum = payload::sha256_of_file(&fixture.bin().join("nurt.exe")).unwrap();

    let held = lock(&fixture.bin().join("nurt.exe"));
    let outcome = install_plain(&fixture.full(), &fixture.user);
    drop(held);
    let shown: Vec<String> = outcome
        .events
        .iter()
        .flat_map(|event| event.log.iter())
        .filter_map(|line| match line {
            LogLine::Answer { text, .. } => Some(text.clone()),
            LogLine::Command { .. } => None,
        })
        .collect();
    assert!(
        !shown.iter().any(|text| text.contains("create folder")),
        "the folders of the earlier installation are not undone: {shown:?}"
    );
    let failure = failure(outcome);
    assert_eq!(failure.error.code, "in-use");
    assert!(failure.rolled_back);
    assert_eq!(
        fs::read(fixture.root().join(journal::FILE_NAME)).unwrap(),
        journal_before
    );
    assert_eq!(
        tree(&fixture.root()),
        tree_before,
        "no backup, staging or new journal is left"
    );
    assert_eq!(
        payload::sha256_of_file(&fixture.bin().join("nurt.exe")).unwrap(),
        exe_sum
    );
    assert_eq!(registry_state(), registry_before);
    assert_eq!(user_path(), path_before);
    assert!(damaged_files(&fixture.root()).is_empty());

    undo_run(&fixture.root(), &fixture.user, false).unwrap();
    assert!(!fixture.root().exists());
}

#[test]
fn a_failure_in_the_middle_of_the_copy_removes_the_files_placed_before() {
    let _guard = serial();
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root().join("doc")).unwrap();
    fs::write(fixture.root().join("doc\\README.md"), b"mine").unwrap();
    let held = lock(&fixture.root().join("doc\\README.md"));
    let (registry_before, path_before) = (registry_state(), user_path());

    let failure = failure(install_plain(&fixture.basic(), &fixture.user));
    drop(held);
    assert_eq!(failure.error.code, "in-use", "{:?}", failure.error);
    assert_eq!(failure.stage, "copy");
    assert!(failure.rolled_back);
    assert_eq!(
        tree(&fixture.root()),
        vec!["doc\\".to_string(), "doc\\README.md".to_string()],
        "only what was there before"
    );
    assert_eq!(
        fs::read(fixture.root().join("doc\\README.md")).unwrap(),
        b"mine"
    );
    assert_eq!(registry_state(), registry_before);
    assert_eq!(user_path(), path_before);
}

#[test]
fn a_denied_root_fails_before_anything_is_written() {
    let _guard = serial();
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root()).unwrap();
    fixture.deny_writes(&fixture.root());
    let failure = failure(install_plain(&fixture.basic(), &fixture.user));
    fixture.allow_writes();
    assert_eq!(failure.error.code, "access-denied", "{:?}", failure.error);
    assert!(failure.rolled_back);
    assert!(tree(&fixture.root()).is_empty());
}

#[test]
fn a_full_volume_rolls_back() {
    let Some(volume) = std::env::var_os("NURT_TEST_SMALL_VOLUME").map(PathBuf::from) else {
        return;
    };
    let _guard = serial();
    let fixture = Fixture::new();
    let filler = volume.join("filler.bin");
    let free = sys::free_space(&volume).unwrap();
    fs::write(
        &filler,
        vec![7u8; usize::try_from(free.saturating_sub(120_000)).unwrap()],
    )
    .unwrap();
    let options = Options {
        dir: volume.join("Nurt\\bin").display().to_string(),
        ..fixture.basic()
    };
    let (registry_before, path_before) = (registry_state(), user_path());

    let failure = failure(install_plain(&options, &fixture.user));
    assert_eq!(failure.error.code, "no-space", "{:?}", failure.error);
    assert!(failure.rolled_back);
    assert!(!volume.join("Nurt").exists(), "left: {:?}", tree(&volume));
    assert_eq!(registry_state(), registry_before);
    assert_eq!(user_path(), path_before);
    let _ = fs::remove_file(&filler);
}

#[test]
fn undo_goes_on_when_one_file_is_locked() {
    let _guard = serial();
    let fixture = Fixture::new();
    let (registry_before, path_before) = (registry_state(), user_path());
    success(install_plain(&fixture.basic(), &fixture.user));
    let held = lock(&fixture.bin().join("nurt.exe"));
    let summary = undo_run(&fixture.root(), &fixture.user, false).unwrap();
    drop(held);
    assert_eq!(summary.warnings.len(), 1, "{:?}", summary.warnings);
    assert!(summary.warnings[0].contains("nurt.exe"));
    assert_eq!(
        registry_state(),
        registry_before,
        "the other entries are undone"
    );
    assert_eq!(user_path(), path_before);
    assert!(!fixture.root().join("doc").exists());
}

#[test]
fn undo_of_an_unreadable_journal_reports_the_cause() {
    let _guard = serial();
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root()).unwrap();
    fs::write(
        fixture.root().join(journal::FILE_NAME),
        "{\"n\":0,\"t\":\"x\",\"op\":\"end\",\"entries\":0,\"result\":\"complete\"}\n",
    )
    .unwrap();
    let error = undo_run(&fixture.root(), &fixture.user, false).unwrap_err();
    assert_eq!(error.code, "journal-unreadable");
    assert_eq!(inspect(&fixture.root()), (0, false));
}

#[test]
fn the_plan_lists_the_entries_of_the_options() {
    let _guard = serial();
    let fixture = Fixture::new();
    fixture.write_terminal_settings();
    let full = plan(&fixture.full(), &fixture.user);
    assert!(full.first().unwrap().starts_with("create folder "));
    assert!(
        full.iter()
            .any(|item| item.starts_with("copy the documentation to "))
    );
    assert!(full.iter().any(|item| item.contains("PATH")));
    assert!(full.iter().any(|item| item.contains("Quick Launch")));
    assert!(full.iter().any(|item| item.contains("default profile")));
    assert!(
        full.iter()
            .any(|item| item.starts_with("write the first configuration"))
    );
    assert_eq!(
        full.last().map(String::as_str),
        Some("close the setup journal")
    );

    let bare = plan(
        &Options {
            path: false,
            docs: false,
            start_menu: false,
            terminal_profile: false,
            desktop: false,
            quick_launch: false,
            completions: false,
            write_config: false,
            ..fixture.full()
        },
        &fixture.user,
    );
    assert_eq!(bare.len(), 6, "{bare:?}");
    assert!(
        bare.iter()
            .any(|item| item.starts_with("copy LICENSE and NOTICE"))
    );
    assert!(!bare.iter().any(|item| item.contains("PATH")));
}

#[test]
fn describe_names_every_kind_of_entry() {
    let begin = Entry::Begin {
        format: journal::FORMAT.into(),
        version: VERSION.into(),
        target: "t".into(),
        bin: "C:\\N\\bin".into(),
        root: "C:\\N".into(),
        options: Options::default(),
    };
    assert_eq!(describe(&begin), "open the setup journal");
    assert_eq!(
        describe(&Entry::CreateDirs {
            paths: vec!["C:\\N".into(), "C:\\N\\bin".into()]
        }),
        "create folder C:\\N\\bin"
    );
    let file = |path: &str| FileRecord {
        path: path.into(),
        sha256: String::new(),
        size: 0,
        replaced: false,
    };
    assert_eq!(
        describe(&Entry::WriteFiles {
            dir: "C:\\N\\bin".into(),
            files: vec![file("nurt.exe")]
        }),
        "copy nurt.exe to C:\\N\\bin"
    );
    assert_eq!(
        describe(&Entry::WriteFiles {
            dir: "C:\\N".into(),
            files: vec![file("uninstall.exe")]
        }),
        "copy the uninstaller and the icon to C:\\N"
    );
    assert_eq!(
        describe(&Entry::WriteFiles {
            dir: "C:\\N\\doc".into(),
            files: vec![file("README.md")]
        }),
        "copy the documentation to C:\\N\\doc"
    );
    assert_eq!(
        describe(&Entry::SetPath {
            scope: Scope::Machine,
            entry: "C:\\N\\bin".into(),
            previous: None
        }),
        "add C:\\N\\bin to the machine PATH"
    );
    assert_eq!(
        describe(&Entry::End {
            entries: 3,
            result: "complete".into()
        }),
        "close the setup journal"
    );
}

#[test]
fn machine_scope_installs_and_undoes_when_elevated() {
    if !sys::is_elevated() {
        return;
    }
    let _guard = serial();
    let fixture = Fixture::new();
    let program_files =
        std::env::var("ProgramW6432").unwrap_or_else(|_| "C:\\Program Files".into());
    let root = PathBuf::from(&program_files).join(format!("Nurt-test-{}", std::process::id()));
    let options = Options {
        scope: Scope::Machine,
        dir: root.join("bin").display().to_string(),
        desktop: false,
        quick_launch: false,
        terminal_default: false,
        ..fixture.full()
    };
    let before = (
        registry::capture(Scope::Machine, UNINSTALL_KEY).unwrap(),
        registry::read_path(Scope::Machine).unwrap(),
    );
    let report = location::check(&options, &fixture.user, true, 1000);
    assert_eq!(report.issue, None, "{report:?}");

    let summary = success(install_plain(&options, &fixture.user));
    assert_eq!(summary.scope, "machine");
    assert!(root.join("bin\\nurt.exe").is_file());
    assert!(
        registry::capture(Scope::Machine, UNINSTALL_KEY)
            .unwrap()
            .existed
    );
    assert!(
        registry::read_path(Scope::Machine)
            .unwrap()
            .unwrap_or_default()
            .contains(&root.join("bin").display().to_string())
    );

    let undone = undo_run(&root, &fixture.user, false).unwrap();
    assert!(undone.warnings.is_empty(), "{:?}", undone.warnings);
    assert!(!root.exists());
    let after = (
        registry::capture(Scope::Machine, UNINSTALL_KEY).unwrap(),
        registry::read_path(Scope::Machine).unwrap(),
    );
    assert_eq!(after, before);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
