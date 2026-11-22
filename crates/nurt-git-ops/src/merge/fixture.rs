// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repositories, objects and execution contexts for unit tests of the
//! merge module.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bstr::ByteSlice;
use gix_hash::ObjectId;
use gix_object::tree::EntryKind;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec, Services,
};
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx};

/// Seconds of the fixed test clock: 2026-01-01T12:00:00Z.
pub(crate) const TEST_TIME: i64 = 1_767_268_800;

pub(crate) struct Fixture {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) repo: gix::Repository,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let repo = gix::init(dir.path()).expect("repository");
        Self { dir, repo }
    }

    pub(crate) fn workdir(&self) -> &Path {
        self.dir.path()
    }

    pub(crate) fn blob(&self, data: &[u8]) -> ObjectId {
        self.repo.write_blob(data).expect("blob").detach()
    }

    /// Tree of text files.
    pub(crate) fn tree(&self, files: &[(&str, &str)]) -> ObjectId {
        let entries: Vec<(&str, EntryKind, &[u8])> = files
            .iter()
            .map(|(path, text)| (*path, EntryKind::Blob, text.as_bytes()))
            .collect();
        self.tree_with(&entries)
    }

    /// Tree of entries of any blob kind.
    pub(crate) fn tree_with(&self, entries: &[(&str, EntryKind, &[u8])]) -> ObjectId {
        let mut editor = gix_object::tree::Editor::new(
            gix_object::Tree::empty(),
            &self.repo.objects,
            gix_hash::Kind::Sha1,
        );
        for (path, kind, data) in entries {
            let id = self.blob(data);
            editor
                .upsert(path.split('/'), *kind, id)
                .expect("tree entry");
        }
        editor
            .write(|tree| self.repo.write_object(tree).map(gix::Id::detach))
            .expect("tree")
    }

    pub(crate) fn commit(&self, tree: ObjectId, parents: &[ObjectId], message: &str) -> ObjectId {
        let signature = gix::actor::Signature {
            name: "A".into(),
            email: "a@x".into(),
            time: gix::date::Time::new(TEST_TIME, 0),
        };
        let commit = gix_object::Commit {
            tree,
            parents: parents.iter().copied().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: format!("{message}\n").into(),
            extra_headers: Vec::new(),
        };
        self.repo.write_object(&commit).expect("commit").detach()
    }

    /// Content of the blob at `path` in `tree`; `None` when absent.
    pub(crate) fn read(&self, tree: ObjectId, path: &str) -> Option<String> {
        let tree = self.repo.find_tree(tree).expect("tree");
        let entry = tree.lookup_entry_by_path(path).expect("lookup")?;
        let blob = self.repo.find_blob(entry.object_id()).expect("blob");
        Some(blob.data.to_str_lossy().into_owned())
    }

    pub(crate) fn mode(&self, tree: ObjectId, path: &str) -> Option<EntryKind> {
        let tree = self.repo.find_tree(tree).expect("tree");
        let entry = tree.lookup_entry_by_path(path).expect("lookup")?;
        Some(entry.mode().kind())
    }
}

pub(crate) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> WallTime {
        WallTime::from_unix_nanos(i128::from(TEST_TIME) * 1_000_000_000)
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: self.now(),
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(
        &self,
        _duration: Duration,
        _cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        Ok(())
    }
}

pub(crate) struct TestHost {
    info: HostInfo,
    env: BTreeMap<String, String>,
}

impl TestHost {
    pub(crate) fn new(env: &[(&str, &str)]) -> Self {
        Self {
            info: HostInfo {
                os: Os::Linux,
                os_version: "6.8.0".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "test".to_owned(),
                user: "test".to_owned(),
                home: PathBuf::from("/nonexistent"),
                temp: PathBuf::from("/tmp"),
                path_style: PathStyle::Posix,
                timezone: "UTC".to_owned(),
                locale: None,
                cpus: 1,
            },
            env: env
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        self.env.get(name).map(OsString::from)
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.env
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect()
    }
}

/// Program host that acts as a merge driver: it writes `output` into the
/// first quoted path of the command and exits with `exit_code`.
pub(crate) struct DriverHost {
    pub(crate) output: Vec<u8>,
    pub(crate) exit_code: i32,
    pub(crate) fail_to_start: bool,
    pub(crate) commands: Mutex<Vec<String>>,
}

impl DriverHost {
    pub(crate) fn new(output: &str, exit_code: i32) -> Self {
        Self {
            output: output.as_bytes().to_vec(),
            exit_code,
            fail_to_start: false,
            commands: Mutex::new(Vec::new()),
        }
    }
}

impl ProcessHost for DriverHost {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(program))
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        if self.fail_to_start {
            return Err(Error::new(ErrorCode::Spawn, "cannot start sh"));
        }
        let command = spec.args.get(1).cloned().unwrap_or_default();
        self.commands.lock().expect("lock").push(command.clone());
        let target = command.split('\'').nth(1).unwrap_or_default();
        std::fs::write(target, &self.output).expect("driver output");
        Ok(ProgramOutcome {
            exit_code: Some(self.exit_code),
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "services are not available in tests",
        ))
    }
}

pub(crate) fn context(
    state_dir: Option<PathBuf>,
    process: Option<Arc<dyn ProcessHost>>,
    env: &[(&str, &str)],
) -> ExecCtx {
    let mut builder = ExecCtx::builder(Arc::new(FixedClock), Arc::new(TestHost::new(env)));
    if let Some(dir) = state_dir {
        builder = builder.state_dir(dir);
    }
    if let Some(process) = process {
        builder = builder.services(Arc::new(Services::new().with_process_host(process)));
    }
    builder.build()
}
