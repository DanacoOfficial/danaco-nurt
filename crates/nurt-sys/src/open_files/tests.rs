// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::open_files::tests
// Responsibility:   Boot time written to the synthetic `/proc/stat`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::path::{Path, PathBuf};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::Notable;
use nurt_contracts::value::Value;

use super::{Holder, OpenFilesStage, Scan, Target};
use crate::host::test_support::{self, TestHost};

fn target(path: &str, directory: bool) -> Target {
    Target {
        path: PathBuf::from(path),
        directory,
    }
}

#[test]
fn open_files_target_matches_file_or_subtree() {
    let file = target("/data/file.txt", false);
    assert!(file.matches(Path::new("/data/file.txt")));
    assert!(!file.matches(Path::new("/data/file.txt.bak")));
    let dir = target("/data", true);
    assert!(dir.matches(Path::new("/data")));
    assert!(dir.matches(Path::new("/data/sub/deep.txt")));
    assert!(!dir.matches(Path::new("/data2/file.txt")));
}

#[cfg(unix)]
#[test]
fn open_files_holder_matches_paths_that_are_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let name = OsStr::from_bytes(b"/data/caf\xE9.txt");
    let file = Target {
        path: PathBuf::from(name),
        directory: false,
    };
    let mut holder = Holder::new(1);
    holder.check(&file, super::Access::Fd, name);
    assert!(holder.is_holding());
    assert!(holder.files.contains("/data/caf\u{FFFD}.txt"));

    let deleted = OsStr::from_bytes(b"/data/caf\xE9.txt (deleted)");
    let mut holder = Holder::new(2);
    holder.check(&file, super::Access::Fd, deleted);
    assert!(!holder.is_holding());
    let directory = target("/data", true);
    holder.check(&directory, super::Access::Fd, deleted);
    assert!(holder.is_holding() && holder.deleted);
}

#[test]
fn open_files_holder_raw_lists_access_files_and_kind() {
    let mut holder = Holder::new(7);
    assert!(!holder.is_holding());
    assert!(holder.raw().is_empty());
    holder.access.insert(super::Access::Mmap);
    holder.access.insert(super::Access::Fd);
    holder.files.insert("/data/a".to_owned());
    holder.deleted = true;
    holder.kind = Some("service".to_owned());
    let raw = holder.raw();
    assert_eq!(
        raw.get("access"),
        Some(&Value::List(vec![
            Value::Text("fd".to_owned()),
            Value::Text("mmap".to_owned())
        ]))
    );
    assert_eq!(
        raw.get("files"),
        Some(&Value::List(vec![Value::Path("/data/a".to_owned())]))
    );
    assert_eq!(raw.get("deleted"), Some(&Value::Bool(true)));
    assert_eq!(raw.get("kind"), Some(&Value::Text("service".to_owned())));
    let names: Vec<&str> = [super::Access::Cwd, super::Access::Root, super::Access::Exe]
        .iter()
        .map(|access| access.as_str())
        .collect();
    assert_eq!(names, ["cwd", "root", "exe"]);
}

#[test]
fn open_files_report_notes_skipped_and_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = test_support::context(TestHost::new(dir.path()));
    super::report(&ctx, &Scan::default());
    assert!(ctx.take_notes().is_empty());
    let scan = Scan {
        holders: Vec::new(),
        denied: 3,
        truncated: Some(10),
    };
    super::report(&ctx, &scan);
    let notes = ctx.take_notes();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].code, Notable::PermissionDeniedSkipped);
    assert_eq!(notes[0].count, Some(3));
    assert_eq!(notes[1].code, Notable::TargetCountHigh);
    assert_eq!(notes[1].count, Some(10));
}

#[test]
fn open_files_missing_path_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = test_support::context(TestHost::new(dir.path()));
    let missing = dir.path().join("absent.txt");
    let args = test_support::args(
        super::COMMAND,
        vec![(
            super::PATH,
            Value::Path(missing.to_string_lossy().into_owned()),
        )],
    );
    let error = test_support::run(&OpenFilesStage::new(), &args, &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);

    let args = test_support::args(
        super::COMMAND,
        vec![(super::PATH, Value::Path("relative/file.txt".to_owned()))],
    );
    let error = test_support::run(&OpenFilesStage::new(), &args, &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::PathRelative);

    let args = test_support::args(super::COMMAND, vec![(super::PATH, Value::Int(1))]);
    let error = test_support::run(&OpenFilesStage::new(), &args, &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Type);
}

#[test]
#[ignore = "requires the process table of the running system"]
fn it_open_files_finds_self_holding_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("held.txt");
    let _file = std::fs::File::create(&path).unwrap();
    let ctx = test_support::context(TestHost::new(dir.path()));
    let args = test_support::args(
        super::COMMAND,
        vec![(
            super::PATH,
            Value::Path(path.to_string_lossy().into_owned()),
        )],
    );
    let records = test_support::run(&OpenFilesStage::new(), &args, &ctx).unwrap();
    let processes = test_support::processes(records);
    assert!(
        processes
            .iter()
            .any(|process| process.pid == u64::from(std::process::id()))
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires the Restart Manager of the running system"]
fn it_windows_restart_manager_lists_holder() {
    use nurt_contracts::services::FileHolderService;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("held.txt");
    let _file = std::fs::File::create(&path).unwrap();
    let holders = super::SystemFileHolders::new().holders(&path).unwrap();
    let own = holders
        .iter()
        .find(|holder| holder.pid == std::process::id())
        .expect("the test process holds the file");
    assert!(own.kind.is_some());
}

#[cfg(target_os = "linux")]
mod procfs_tree {
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use nurt_contracts::record::{Process, ProcessState};
    use nurt_contracts::services::Services;
    use nurt_contracts::stage::ExecCtx;
    use nurt_contracts::value::Value;

    use super::super::{OpenFilesStage, SystemFileHolders};
    use crate::common::SysRoot;
    use crate::host::test_support::{self, FixedClock, TestHost};

    /// Boot time written to the synthetic `/proc/stat`.
    const BOOT_SECONDS: i64 = 1_700_000_000;
    /// Start time of every synthetic process in clock ticks after boot.
    const START_TICKS: u64 = 5000;
    /// Resident pages of every synthetic process.
    const RESIDENT_PAGES: u64 = 250;

    /// Synthetic system root and a real data directory whose files the
    /// synthetic processes hold.
    struct Tree {
        _dir: tempfile::TempDir,
        root: PathBuf,
        data: PathBuf,
    }

    impl Tree {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("root");
            let data = dir.path().join("data");
            std::fs::create_dir_all(root.join("proc")).unwrap();
            std::fs::create_dir_all(root.join("etc")).unwrap();
            std::fs::create_dir_all(data.join("sub")).unwrap();
            std::fs::write(root.join("proc/stat"), format!("btime {BOOT_SECONDS}\n")).unwrap();
            std::fs::write(
                root.join("etc/passwd"),
                "user:x:1000:1000::/home/user:/bin/sh\n",
            )
            .unwrap();
            let data = std::fs::canonicalize(data).unwrap();
            Self {
                _dir: dir,
                root,
                data,
            }
        }

        fn file(&self, name: &str) -> PathBuf {
            let path = self.data.join(name);
            std::fs::write(&path, "content\n").unwrap();
            path
        }

        /// Process `pid` named `name` with descriptors `fds`, links `links`
        /// (`cwd`, `root`, `exe`) and the memory map `maps`.
        fn process(&self, pid: u32, name: &str, fds: &[&str], links: &[(&str, &str)], maps: &str) {
            let dir = self.root.join(format!("proc/{pid}"));
            std::fs::create_dir_all(dir.join("fd")).unwrap();
            std::fs::write(
                dir.join("stat"),
                format!(
                    "{pid} ({name}) S 1 {pid} {pid} 0 -1 4194560 100 0 0 0 10 5 0 0 20 0 3 0 \
                     {START_TICKS} 12345678 {RESIDENT_PAGES}\n"
                ),
            )
            .unwrap();
            std::fs::write(
                dir.join("status"),
                format!("Name:\t{name}\nUid:\t1000\t1000\t1000\t1000\n"),
            )
            .unwrap();
            std::fs::write(dir.join("cmdline"), format!("{name}\0--flag\0")).unwrap();
            std::fs::write(dir.join("comm"), format!("{name}\n")).unwrap();
            std::fs::write(dir.join("maps"), maps).unwrap();
            for (index, target) in fds.iter().enumerate() {
                symlink(target, dir.join(format!("fd/{}", index + 3))).unwrap();
            }
            for (link, target) in links {
                symlink(target, dir.join(link)).unwrap();
            }
        }

        fn sysroot(&self) -> SysRoot {
            SysRoot::new(&self.root)
        }

        fn run(&self, path: &Path) -> Vec<Process> {
            let ctx = test_support::context(TestHost::new(&self.data));
            let args = test_support::args(
                super::super::COMMAND,
                vec![(
                    super::super::PATH,
                    Value::Path(path.to_string_lossy().into_owned()),
                )],
            );
            let stage = OpenFilesStage::with_root(self.sysroot());
            test_support::processes(test_support::run(&stage, &args, &ctx).unwrap())
        }
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn pids(processes: &[Process]) -> Vec<u64> {
        processes.iter().map(|process| process.pid).collect()
    }

    fn access(process: &Process) -> Vec<String> {
        match process.os_raw.as_ref().and_then(|raw| raw.get("access")) {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn open_files_linux_fd_cwd_mmap_detection() {
        let tree = Tree::new();
        let file = tree.file("file.txt");
        let data = text(&tree.data);
        tree.process(
            100,
            "editor",
            &[&text(&file), "socket:[1234]"],
            &[("cwd", "/")],
            "",
        );
        let maps = format!(
            "7f00-7f10 r--p 00000000 08:01 42   {}\n7f10-7f20 rw-p 00000000 00:00 0 \n\
             7f20-7f30 rw-p 00000000 00:00 0   [heap]\n",
            text(&file)
        );
        tree.process(200, "viewer", &[], &[], &maps);
        tree.process(
            300,
            "shell",
            &[],
            &[("cwd", &data), ("exe", "/usr/bin/shell")],
            "",
        );

        let holders = tree.run(&file);
        assert_eq!(pids(&holders), [100, 200]);
        assert_eq!(access(&holders[0]), ["fd"]);
        assert_eq!(access(&holders[1]), ["mmap"]);
        assert_editor(&holders[0]);

        let in_dir = tree.run(&tree.data);
        assert_eq!(pids(&in_dir), [100, 200, 300]);
        assert_eq!(access(&in_dir[2]), ["cwd"]);
    }

    /// Fields of the synthetic process 100 read from its directory.
    fn assert_editor(editor: &Process) {
        assert_eq!(editor.name, "editor");
        assert_eq!(editor.parent_pid, 1);
        assert_eq!(editor.command, ["editor", "--flag"]);
        assert_eq!(editor.user, "user");
        assert_eq!(editor.uid.as_deref(), Some("1000"));
        assert_eq!(editor.threads, Some(3));
        assert_eq!(editor.state, ProcessState::Sleeping);
        assert_eq!(editor.cwd.as_ref().map(|cwd| cwd.as_str()), Some("/"));
        let page = u64::try_from(rustix::param::page_size()).unwrap();
        assert_eq!(editor.memory.bytes(), RESIDENT_PAGES * page);
        let hz = i64::try_from(rustix::param::clock_ticks_per_second()).unwrap();
        let start_ticks = i64::try_from(START_TICKS).unwrap();
        let started_nanos = BOOT_SECONDS * 1_000_000_000 + start_ticks * 1_000_000_000 / hz;
        assert_eq!(editor.started.as_unix_nanos(), started_nanos);
    }

    #[test]
    fn open_files_directory_matches_subtree() {
        let tree = Tree::new();
        let deep = tree.file("sub/deep.txt");
        let sibling = tree.data.with_file_name("data2");
        std::fs::create_dir_all(&sibling).unwrap();
        tree.process(500, "worker", &[&text(&deep)], &[], "");
        tree.process(600, "other", &[&text(&sibling.join("x"))], &[], "");

        assert_eq!(pids(&tree.run(&tree.data)), [500]);
        assert_eq!(pids(&tree.run(&tree.data.join("sub"))), [500]);
        let empty = tree.data.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(tree.run(&empty).is_empty());
        assert_eq!(pids(&tree.run(&sibling)), [600]);
    }

    #[test]
    fn open_files_deleted_suffix_handled() {
        let tree = Tree::new();
        let gone = tree.data.join("gone.txt");
        tree.process(
            700,
            "keeper",
            &[&format!("{} (deleted)", text(&gone))],
            &[],
            "",
        );

        let holders = tree.run(&tree.data);
        assert_eq!(pids(&holders), [700]);
        let raw = holders[0].os_raw.as_ref().unwrap();
        assert_eq!(raw.get("deleted"), Some(&Value::Bool(true)));
        assert_eq!(
            raw.get("files"),
            Some(&Value::List(vec![Value::Path(text(&gone))]))
        );

        std::fs::write(&gone, "new content\n").unwrap();
        assert!(tree.run(&gone).is_empty());
    }

    #[test]
    fn open_files_skips_vanished_processes_and_reports_missing_proc() {
        let tree = Tree::new();
        let file = tree.file("file.txt");
        let source = crate::processes::source_under(&tree.sysroot());
        let holders =
            super::super::holder_processes(source.as_ref(), &[super::super::Holder::new(999)])
                .unwrap();
        assert!(holders.is_empty());
        assert_eq!(
            super::super::linux::mapped_path("7f00-7f10 r--p 0 0:0 0"),
            None
        );

        let empty = tempfile::tempdir().unwrap();
        let target = super::super::Target::resolve(&file).unwrap();
        let error = super::super::linux::scan(&SysRoot::new(empty.path()), &target).unwrap_err();
        assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Io);
    }

    #[test]
    fn open_files_linux_matches_paths_that_are_not_utf8() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let tree = Tree::new();
        let bad = tree.data.join(OsStr::from_bytes(b"caf\xE9.txt"));
        std::fs::write(&bad, "content\n").unwrap();
        let link = bad.as_os_str().as_bytes().to_vec();
        tree.process(900, "descriptor", &[], &[], "");
        symlink(OsStr::from_bytes(&link), tree.root.join("proc/900/fd/3")).unwrap();
        tree.process(901, "mapper", &[], &[], "");
        let mut maps = b"7f00-7f10 r--p 00000000 08:01 42   ".to_vec();
        maps.extend_from_slice(&link);
        maps.push(b'\n');
        std::fs::write(tree.root.join("proc/901/maps"), maps).unwrap();

        let target = super::super::Target::resolve(&bad).unwrap();
        let scan = super::super::linux::scan(&tree.sysroot(), &target).unwrap();
        let found: Vec<u32> = scan.holders.iter().map(|holder| holder.pid).collect();
        assert_eq!(found, [900, 901]);
    }

    #[test]
    fn open_files_linux_maps_path_column_keeps_spaces_and_bytes() {
        use super::super::linux::{mapped_path, mapped_path_bytes};

        assert_eq!(
            mapped_path("7f00-7f10 r--p 0 08:01 42   /opt/my app/lib.so"),
            Some("/opt/my app/lib.so")
        );
        assert_eq!(
            mapped_path_bytes(b"7f00-7f10 r--p 0 08:01 42 /tmp/\xFF.so"),
            Some(&b"/tmp/\xFF.so"[..])
        );
        assert_eq!(mapped_path_bytes(b"7f00-7f10 r--p 0 08:01 42"), None);
        assert_eq!(mapped_path_bytes(b""), None);
    }

    #[test]
    fn file_holders_returns_process_refs_for_fs() {
        let tree = Tree::new();
        let file = tree.file("locked.bin");
        tree.process(800, "locker", &[&text(&file)], &[], "");
        tree.process(801, "idle", &[], &[("cwd", "/")], "");

        let service = Arc::new(SystemFileHolders::with_root(tree.sysroot()));
        let services = Services::new().with_file_holders(service);
        let ctx = ExecCtx::builder(
            Arc::new(FixedClock::now()),
            Arc::new(TestHost::new(&tree.data)),
        )
        .services(Arc::new(services))
        .build();
        let holders = ctx
            .services()
            .file_holders()
            .unwrap()
            .holders(&file)
            .unwrap();
        assert_eq!(holders.len(), 1);
        assert_eq!(holders[0].pid, 800);
        assert_eq!(holders[0].name, "locker");
        assert_eq!(holders[0].kind, None);

        let missing = ctx
            .services()
            .file_holders()
            .unwrap()
            .holders(&tree.data.join("absent"))
            .unwrap_err();
        assert_eq!(missing.code(), nurt_contracts::error::ErrorCode::NotFound);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
