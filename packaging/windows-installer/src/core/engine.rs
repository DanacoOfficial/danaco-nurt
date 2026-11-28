// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/engine.rs
// Responsibility:   Installation engine: runs an installation or an undo as stages, writes the journal of every change and rolls back when a stage fails.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Installation engine.
//!
//! An installation is six stages: `verify`, `extract`, `copy`, `path`, `shortcuts` and
//! `journal`. Every change is written to the journal of the setup before the next change
//! is made; when a stage fails or the user cancels, the entries written so far are undone
//! in the reverse order. The undo of a finished installation uses the same function with
//! the entries read from the journal file.
//!
//! Problems of the optional parts (shortcuts, Windows Terminal, completion, configuration)
//! become warnings and do not stop the installation.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{SetupError, SetupResult};
use crate::journal::{self, Entry, FileRecord, Journal, Record};
use crate::model::{
    FinishEvent, Layout, LogLine, Options, ProgressEvent, Scope, StageState, Summary, Tone, Undone,
    UserPaths,
};
use crate::registry::{self, KeyRecord, UninstallEntry};
use crate::{location, logfile, paths, payload, shortcuts, terminal, textedit};

/// Stages of an installation with their share of the progress.
pub(crate) const INSTALL_STAGES: [(&str, f32); 6] = [
    ("verify", 0.08),
    ("extract", 0.34),
    ("copy", 0.22),
    ("path", 0.10),
    ("shortcuts", 0.16),
    ("journal", 0.10),
];

/// Stages of an undo with their share of the progress.
pub(crate) const UNDO_STAGES: [(&str, f32); 4] = [
    ("read", 0.10),
    ("remove", 0.50),
    ("restore", 0.30),
    ("close", 0.10),
];

/// Shortest time a stage stays visible in the window.
const MIN_STAGE: Duration = Duration::from_millis(480);

/// Suffix of the copy that a replaced file keeps until the installation is complete.
pub(crate) const BACKUP_SUFFIX: &str = ".setup-bak";

/// Prefix of the folder that holds the extracted files until they are placed.
const STAGING_PREFIX: &str = ".setup-staging-";

/// How an entry is undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The run failed or was cancelled: put back what the run replaced.
    Rollback,
    /// The user undoes a finished installation: remove what the setup installed.
    Remove,
}

/// Everything a run needs.
pub(crate) struct Env<'a> {
    /// Choices of the user.
    pub(crate) options: &'a Options,
    /// Directories of the user.
    pub(crate) user: &'a UserPaths,
    /// Version of the program in the archive.
    pub(crate) version: &'a str,
    /// Receiver of the events of progress.
    pub(crate) emit: &'a dyn Fn(ProgressEvent),
    /// Tells whether the user cancelled.
    pub(crate) cancel: &'a dyn Fn() -> bool,
    /// Keeps every stage visible for a minimum time; off in the silent mode.
    pub(crate) pace: bool,
}

/// A failed installation.
#[derive(Debug)]
pub(crate) struct Failure {
    /// The cause.
    pub(crate) error: SetupError,
    /// Stage that failed.
    pub(crate) stage: String,
    /// Every change was undone.
    pub(crate) rolled_back: bool,
    /// Number of the entries that were undone.
    pub(crate) undone: u64,
    /// The user cancelled.
    pub(crate) cancelled: bool,
}

impl Failure {
    /// The event that tells the interface about the failure.
    pub(crate) fn event(&self) -> FinishEvent {
        FinishEvent {
            ok: false,
            cancelled: self.cancelled,
            code: self.error.code.to_string(),
            detail: self.error.detail.clone(),
            stage: self.stage.clone(),
            rolled_back: self.rolled_back,
            undone: self.undone,
            summary: None,
        }
    }
}

/// Planned entries of an installation, in the order of the journal; the interface shows them
/// on the review screen and the engine writes exactly these entries.
pub(crate) fn plan(options: &Options, user: &UserPaths) -> Vec<String> {
    let layout = Layout::of(&location::resolve(options, user), options.docs);
    let mut items = vec![
        format!("create folder {}", layout.bin.display()),
        format!("copy nurt.exe to {}", layout.bin.display()),
    ];
    if layout.docs {
        items.push(format!(
            "copy the documentation to {}",
            layout.doc().display()
        ));
    } else {
        items.push(format!(
            "copy LICENSE and NOTICE to {}",
            layout.root.display()
        ));
    }
    items.push(format!(
        "copy the uninstaller and the icon to {}",
        layout.root.display()
    ));
    items.push("register Danaco Nurt in Windows".to_string());
    if options.path {
        items.push(format!(
            "add {} to the {} PATH",
            layout.bin.display(),
            options.scope.name()
        ));
    }
    if options.start_menu {
        items.push("create the Start Menu shortcuts Danaco Nurt".to_string());
    }
    if options.desktop {
        items.push("create the desktop shortcut Danaco Nurt".to_string());
    }
    if options.quick_launch {
        items.push("create the Quick Launch shortcut Danaco Nurt".to_string());
    }
    if options.terminal_profile {
        items.push("add the Windows Terminal profile Danaco Nurt".to_string());
        if options.terminal_default && !paths::terminal_settings(user).is_empty() {
            items.push("make Danaco Nurt the default profile of Windows Terminal".to_string());
        }
    }
    if options.completions {
        items.push("add the tab completion to the PowerShell profiles".to_string());
    }
    if options.write_config && !paths::user_config(user).exists() {
        items.push(format!(
            "write the first configuration {}",
            paths::user_config(user).display()
        ));
    }
    items.push("close the setup journal".to_string());
    items
}

/// Text of an entry of the journal, as the review screen and the log of the undo show it.
pub(crate) fn describe(entry: &Entry) -> String {
    match entry {
        Entry::Begin { .. } => "open the setup journal".to_string(),
        Entry::CreateDirs { paths } => {
            let main = paths
                .iter()
                .find(|path| path.to_ascii_lowercase().ends_with("\\bin"))
                .or_else(|| paths.last());
            format!("create folder {}", main.map_or("", String::as_str))
        }
        Entry::WriteFiles { dir, files } => {
            if files
                .iter()
                .any(|file| file.path.eq_ignore_ascii_case("nurt.exe"))
            {
                format!("copy nurt.exe to {dir}")
            } else if files
                .iter()
                .any(|file| file.path.eq_ignore_ascii_case("uninstall.exe"))
            {
                format!("copy the uninstaller and the icon to {dir}")
            } else if dir.to_ascii_lowercase().ends_with("\\doc") {
                format!("copy the documentation to {dir}")
            } else {
                format!("copy LICENSE and NOTICE to {dir}")
            }
        }
        Entry::Registry { .. } => "register Danaco Nurt in Windows".to_string(),
        Entry::SetPath { scope, entry, .. } => format!("add {entry} to the {} PATH", scope.name()),
        Entry::Shortcuts { paths, .. } => {
            let first = paths
                .first()
                .map(|path| path.to_ascii_lowercase())
                .unwrap_or_default();
            if first.contains("\\desktop\\") {
                "create the desktop shortcut Danaco Nurt".to_string()
            } else if first.contains("quick launch") {
                "create the Quick Launch shortcut Danaco Nurt".to_string()
            } else {
                "create the Start Menu shortcuts Danaco Nurt".to_string()
            }
        }
        Entry::TerminalFragment { .. } => {
            "add the Windows Terminal profile Danaco Nurt".to_string()
        }
        Entry::TerminalDefault { .. } => {
            "make Danaco Nurt the default profile of Windows Terminal".to_string()
        }
        Entry::PowershellProfile { .. } => {
            "add the tab completion to the PowerShell profiles".to_string()
        }
        Entry::ConfigFile { path, .. } => format!("write the first configuration {path}"),
        Entry::End { .. } => "close the setup journal".to_string(),
    }
}

fn sleep_to(started: Instant, pace: bool) {
    if pace {
        let elapsed = started.elapsed();
        if elapsed < MIN_STAGE {
            thread::sleep(MIN_STAGE - elapsed);
        }
    }
}

// ------------------------------------------------------------------------------------------
// Installation

struct Install<'a> {
    env: &'a Env<'a>,
    layout: Layout,
    scope: Scope,
    journal: Journal,
    summary: Summary,
    stage: &'static str,
    base: f32,
    weight: f32,
    started: Instant,
    skipped: bool,
    staging: PathBuf,
    extracted: Vec<payload::Extracted>,
    backups: Vec<PathBuf>,
    obsolete: Vec<PathBuf>,
    placed: Vec<(PathBuf, bool)>,
    inherited: Vec<Entry>,
    inherited_dirs: usize,
    size: u64,
    log: Vec<LogLine>,
}

/// Runs the installation: writes every change to the journal and undoes them when the run fails.
pub(crate) fn install(env: &Env<'_>) -> Result<Summary, Failure> {
    let bin = location::resolve(env.options, env.user);
    let layout = Layout::of(&bin, env.options.docs);
    let staging = layout
        .root
        .join(format!("{STAGING_PREFIX}{}", std::process::id()));
    let mut run = Install {
        env,
        scope: env.options.scope,
        journal: Journal::new(),
        summary: Summary {
            bin: layout.bin.display().to_string(),
            root: layout.root.display().to_string(),
            version: env.version.to_string(),
            scope: env.options.scope.name().to_string(),
            ..Summary::default()
        },
        layout,
        stage: "verify",
        base: 0.0,
        weight: 0.0,
        started: Instant::now(),
        skipped: false,
        staging,
        extracted: Vec::new(),
        backups: Vec::new(),
        obsolete: Vec::new(),
        placed: Vec::new(),
        inherited: Vec::new(),
        inherited_dirs: 0,
        size: 0,
        log: Vec::new(),
    };
    logfile::line(&format!(
        "install {} scope={} bin={}",
        env.version,
        run.scope.name(),
        run.layout.bin.display()
    ));
    match run.execute() {
        Ok(()) => {
            run.commit();
            logfile::line("install done");
            Ok(run.summary)
        }
        Err(error) => {
            logfile::line(&format!("install failed in {}: {error}", run.stage));
            let cancelled = error.code == "cancelled";
            let stage = run.stage.to_string();
            let (rolled_back, undone) = run.rollback();
            Err(Failure {
                error,
                stage,
                rolled_back,
                undone,
                cancelled,
            })
        }
    }
}

impl Install<'_> {
    fn execute(&mut self) -> SetupResult<()> {
        self.inherited = inheritable(&self.layout.journal());
        self.journal.push(Entry::Begin {
            format: journal::FORMAT.to_string(),
            version: self.env.version.to_string(),
            target: crate::app::TARGET.to_string(),
            bin: self.layout.bin.display().to_string(),
            root: self.layout.root.display().to_string(),
            options: self.env.options.clone(),
        });
        for entry in inherited_dirs(&self.layout.journal()) {
            self.journal.push(entry);
            self.inherited_dirs += 1;
        }
        self.stage("verify", Self::verify)?;
        self.stage("extract", Self::extract)?;
        self.stage("copy", Self::copy)?;
        self.stage("path", Self::registry)?;
        self.stage("shortcuts", Self::integrate)?;
        self.stage("journal", Self::close)
    }

    fn stage(
        &mut self,
        id: &'static str,
        action: fn(&mut Self) -> SetupResult<()>,
    ) -> SetupResult<()> {
        if (self.env.cancel)() {
            return Err(SetupError::new("cancelled", ""));
        }
        self.stage = id;
        self.base += self.weight;
        self.weight = INSTALL_STAGES
            .iter()
            .find(|(name, _)| *name == id)
            .map_or(0.0, |(_, weight)| *weight);
        self.started = Instant::now();
        self.skipped = false;
        self.emit(StageState::Running, 0.0);
        let result = action(self);
        if let Err(error) = &result {
            self.log
                .push(LogLine::answer(error_line(error), Tone::Alert));
            self.emit(StageState::Failed, 0.0);
            return result;
        }
        sleep_to(self.started, self.env.pace && !self.skipped);
        self.emit(
            if self.skipped {
                StageState::Skipped
            } else {
                StageState::Done
            },
            1.0,
        );
        Ok(())
    }

    fn emit(&mut self, state: StageState, fraction: f32) {
        let overall = (self.base + self.weight * fraction.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let log = std::mem::take(&mut self.log);
        (self.env.emit)(ProgressEvent {
            stage: self.stage.to_string(),
            state,
            overall,
            log,
        });
    }

    fn tick(&mut self, fraction: f32) {
        self.emit(StageState::Running, fraction);
    }

    fn warn(&mut self, text: impl Into<String>) {
        let text = text.into();
        logfile::line(&format!("warning: {text}"));
        self.log
            .push(LogLine::answer(format!("warning  {text}"), Tone::Plain));
        self.summary.warnings.push(text);
    }

    fn number(&self) -> u64 {
        self.journal.records().len() as u64
    }

    // -------------------------------------------------------------------------- verify

    fn verify(&mut self) -> SetupResult<()> {
        self.log.push(LogLine::command("verify", "sha256"));
        let mut last = 0.0f32;
        let mut ticks = Vec::new();
        payload::verify(&mut |done, total| {
            let fraction = done as f32 / total.max(1) as f32;
            if fraction - last >= 0.05 || done == total {
                last = fraction;
                ticks.push(fraction);
            }
        })?;
        for fraction in ticks {
            self.tick(fraction);
        }
        self.log.push(LogLine::answer(
            "ok  the digest matches checksums.txt",
            Tone::Ok,
        ));
        Ok(())
    }

    // -------------------------------------------------------------------------- extract

    /// Directories that have to exist for the files, with the missing ones first.
    fn missing_dirs(&self, wanted: &HashSet<PathBuf>) -> Vec<PathBuf> {
        let mut missing: Vec<PathBuf> = Vec::new();
        for directory in wanted {
            for ancestor in directory.ancestors() {
                if ancestor.as_os_str().is_empty()
                    || ancestor.exists()
                    || missing.iter().any(|known| known == ancestor)
                {
                    continue;
                }
                missing.push(ancestor.to_path_buf());
            }
        }
        missing.sort_by_key(|path| path.components().count());
        missing
    }

    fn extract(&mut self) -> SetupResult<()> {
        self.log.push(LogLine::command("extract", ""));
        let items = payload::list()?;
        let mut wanted: HashSet<PathBuf> = HashSet::new();
        wanted.insert(self.layout.bin.clone());
        for item in &items {
            if let Some(destination) =
                payload::destination(&item.name, &self.layout, self.env.options.completions)
                && let Some(parent) = destination.parent()
            {
                wanted.insert(parent.to_path_buf());
            }
        }
        wanted.insert(self.layout.root.clone());
        let missing = self.missing_dirs(&wanted);
        let mut created: Vec<String> = Vec::new();
        for directory in &missing {
            if let Err(error) = fs::create_dir(directory) {
                for path in created.iter().rev() {
                    let _ = fs::remove_dir(path);
                }
                return Err(SetupError::io(directory, &error));
            }
            created.push(directory.display().to_string());
        }
        let writing = journal::writing_path(&self.layout.root);
        if let Err(error) = self.journal.attach(&writing) {
            for path in created.iter().rev() {
                let _ = fs::remove_dir(path);
            }
            return Err(SetupError::io(&writing, &error));
        }
        if !created.is_empty() {
            self.journal.push(Entry::CreateDirs { paths: created });
        }
        fs::create_dir_all(&self.staging).map_err(|error| SetupError::io(&self.staging, &error))?;
        let staging = self.staging.clone();
        let cancel = self.env.cancel;
        let mut ticks: Vec<f32> = Vec::new();
        let mut last = 0.0f32;
        let extracted = payload::extract(&staging, cancel, &mut |done, total, _| {
            let fraction = done as f32 / total.max(1) as f32;
            if fraction - last >= 0.04 || done == total {
                last = fraction;
                ticks.push(fraction);
            }
        })?;
        for fraction in ticks {
            self.tick(fraction);
        }
        self.extracted = extracted;
        let what = if self.layout.docs {
            "nurt.exe and the documentation"
        } else {
            "nurt.exe"
        };
        self.log.push(LogLine::answer(what, Tone::Plain));
        Ok(())
    }

    // ---------------------------------------------------------------------------- copy

    /// Moves a file that is in the way to a backup name; the rollback moves it back.
    fn keep_backup(&mut self, path: &Path) -> SetupResult<bool> {
        if !path.exists() {
            return Ok(false);
        }
        let backup = backup_of(path);
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).map_err(|error| SetupError::io(path, &error))?;
        self.backups.push(backup);
        Ok(true)
    }

    fn place(&mut self, staged: &Path, destination: &Path) -> SetupResult<bool> {
        let replaced = self.keep_backup(destination)?;
        self.placed.push((destination.to_path_buf(), replaced));
        if fs::rename(staged, destination).is_err() {
            fs::copy(staged, destination).map_err(|error| SetupError::io(destination, &error))?;
            let _ = fs::remove_file(staged);
        }
        Ok(replaced)
    }

    fn write_bytes(&mut self, destination: &Path, bytes: &[u8]) -> SetupResult<bool> {
        let replaced = self.keep_backup(destination)?;
        self.placed.push((destination.to_path_buf(), replaced));
        fs::write(destination, bytes).map_err(|error| SetupError::io(destination, &error))?;
        Ok(replaced)
    }

    fn copy(&mut self) -> SetupResult<()> {
        let old = journal::read(&self.layout.journal())
            .ok()
            .map(|records| written_paths(&records))
            .unwrap_or_default();
        let extracted = std::mem::take(&mut self.extracted);
        let completions = self.env.options.completions;
        let total = extracted.len().max(1) as f32 + 3.0;
        let mut groups: Vec<(PathBuf, Vec<FileRecord>)> = Vec::new();
        let mut placed: HashSet<String> = HashSet::new();
        let mut exe_record: Option<FileRecord> = None;
        for (index, file) in extracted.iter().enumerate() {
            if (self.env.cancel)() {
                return Err(SetupError::new("cancelled", ""));
            }
            let Some(destination) = payload::destination(&file.name, &self.layout, completions)
            else {
                continue;
            };
            let replaced = self.place(&file.path, &destination)?;
            self.size += file.size;
            placed.insert(destination.display().to_string().to_ascii_lowercase());
            let folder = if file.name.eq_ignore_ascii_case("nurt.exe") {
                self.layout.bin.clone()
            } else if self.layout.docs {
                self.layout.doc()
            } else {
                self.layout.root.clone()
            };
            let relative = destination.strip_prefix(&folder).map_or_else(
                |_| destination.display().to_string(),
                |rest| rest.display().to_string(),
            );
            let record = FileRecord {
                path: relative,
                sha256: file.sha256.clone(),
                size: file.size,
                replaced,
            };
            if file.name.eq_ignore_ascii_case("nurt.exe") {
                exe_record = Some(record);
            } else if let Some(group) = groups.iter_mut().find(|(dir, _)| *dir == folder) {
                group.1.push(record);
            } else {
                groups.push((folder, vec![record]));
            }
            self.tick((index as f32 + 1.0) / total);
        }
        let Some(exe) = exe_record else {
            return Err(SetupError::new(
                "payload-corrupt",
                "the archive has no nurt.exe",
            ));
        };
        self.log.push(LogLine::command(
            "copy",
            &self.layout.exe().display().to_string(),
        ));
        self.journal.push(Entry::WriteFiles {
            dir: self.layout.bin.display().to_string(),
            files: vec![exe],
        });
        self.log.push(LogLine::answer(
            format!("journal entry {}", self.number() - 1),
            Tone::Plain,
        ));
        for (dir, files) in groups {
            self.journal.push(Entry::WriteFiles {
                dir: dir.display().to_string(),
                files,
            });
        }
        // The files of an earlier installation that the new archive no longer has.
        for path in old {
            if !placed.contains(&path.display().to_string().to_ascii_lowercase())
                && path.is_file()
                && !is_setup_file(&path, &self.layout)
            {
                self.keep_backup(&path)?;
                self.obsolete.push(path);
            }
        }
        self.tick((total - 2.0) / total);
        let icon = self.layout.icon();
        let icon_replaced = self.write_bytes(&icon, payload::ICON)?;
        let uninstaller = self.layout.uninstaller();
        let uninstaller_replaced = self.keep_backup(&uninstaller)?;
        self.placed
            .push((uninstaller.clone(), uninstaller_replaced));
        let copied = payload::copy_self(&uninstaller)
            .map_err(|error| SetupError::io(&uninstaller, &error))?;
        self.size += copied + payload::ICON.len() as u64;
        let uninstaller_sum = payload::sha256_of_file(&uninstaller)
            .map_err(|error| SetupError::io(&uninstaller, &error))?;
        self.journal.push(Entry::WriteFiles {
            dir: self.layout.root.display().to_string(),
            files: vec![
                FileRecord {
                    path: "uninstall.exe".into(),
                    sha256: uninstaller_sum,
                    size: copied,
                    replaced: uninstaller_replaced,
                },
                FileRecord {
                    path: "nurt.ico".into(),
                    sha256: payload::sha256_of(payload::ICON),
                    size: payload::ICON.len() as u64,
                    replaced: icon_replaced,
                },
            ],
        });
        self.tick(1.0);
        Ok(())
    }

    // ------------------------------------------------------------------------- registry

    fn registry(&mut self) -> SetupResult<()> {
        let scope = self.scope;
        let entry = UninstallEntry {
            version: self.env.version.to_string(),
            layout: self.layout.clone(),
            size_kib: u32::try_from(self.size / 1024).unwrap_or(u32::MAX),
            date: logfile::today(),
        };
        let registry_error = |what: &str, error: std::io::Error| {
            SetupError::new("registry", format!("{what}: {error}"))
        };
        let uninstall_before = registry::capture(scope, registry::UNINSTALL_KEY)
            .map_err(|error| registry_error("Uninstall", error))?;
        let app_paths_before = registry::capture(scope, registry::APP_PATHS_KEY)
            .map_err(|error| registry_error("App Paths", error))?;
        // The entry is written before the keys, so that a failure in the middle is still undone.
        self.journal.push(Entry::Registry {
            scope,
            keys: vec![
                KeyRecord {
                    key: registry::UNINSTALL_KEY.to_string(),
                    previous: uninstall_before,
                },
                KeyRecord {
                    key: registry::APP_PATHS_KEY.to_string(),
                    previous: app_paths_before,
                },
            ],
        });
        registry::write_uninstall(scope, &entry)
            .map_err(|error| registry_error("Uninstall", error))?;
        registry::write_app_paths(scope, &self.layout)
            .map_err(|error| registry_error("App Paths", error))?;
        self.log.push(LogLine::command("register", "Danaco Nurt"));
        self.log.push(LogLine::answer(
            "installed apps and the Run dialog",
            Tone::Plain,
        ));
        self.tick(0.5);
        self.log.push(LogLine::command("path", scope.name()));
        if self.env.options.path {
            let bin = self.layout.bin.display().to_string();
            match registry::path_insert(scope, &bin)
                .map_err(|error| registry_error("PATH", error))?
            {
                Some(previous) => {
                    self.summary.path_changed = true;
                    self.journal.push(Entry::SetPath {
                        scope,
                        entry: bin.clone(),
                        previous,
                    });
                    self.log
                        .push(LogLine::answer(format!("added {bin}"), Tone::Plain));
                }
                None => self.log.push(LogLine::answer("already there", Tone::Plain)),
            }
        } else {
            self.log
                .push(LogLine::answer("left unchanged", Tone::Plain));
        }
        Ok(())
    }

    // ------------------------------------------------------------------------ integrate

    fn links(&self, paths_and_links: &mut Vec<(PathBuf, shortcuts::Link)>, kind: &str) {
        let options = self.env.options;
        let user = self.env.user;
        let profile = options.terminal_profile;
        let file = |folder: PathBuf, name: &str| folder.join(format!("{name}.lnk"));
        match kind {
            "start" => {
                let folder = paths::start_menu(self.scope, user).join(shortcuts::PROGRAM_NAME);
                let program = file(folder.clone(), shortcuts::PROGRAM_NAME);
                let undo = file(folder, shortcuts::UNDO_NAME);
                paths_and_links.push((
                    program.clone(),
                    shortcuts::program_link(program, &self.layout, &user.profile, profile),
                ));
                paths_and_links.push((undo.clone(), shortcuts::undo_link(undo, &self.layout)));
            }
            "desktop" => {
                let path = file(paths::desktop(self.scope, user), shortcuts::PROGRAM_NAME);
                paths_and_links.push((
                    path.clone(),
                    shortcuts::program_link(path, &self.layout, &user.profile, profile),
                ));
            }
            _ => {
                let path = file(paths::quick_launch(user), shortcuts::PROGRAM_NAME);
                paths_and_links.push((
                    path.clone(),
                    shortcuts::program_link(path, &self.layout, &user.profile, profile),
                ));
            }
        }
    }

    fn make_shortcuts(&mut self, kind: &str) {
        let mut pairs = Vec::new();
        self.links(&mut pairs, kind);
        let folder = pairs
            .first()
            .and_then(|(path, _)| path.parent().map(Path::to_path_buf));
        let folder_created = (kind == "start")
            .then(|| folder.clone())
            .flatten()
            .filter(|dir| !dir.exists());
        self.log.push(LogLine::command(
            "shortcut",
            &pairs[0].0.display().to_string(),
        ));
        let links: Vec<shortcuts::Link> = pairs.iter().map(|(_, link)| link.clone()).collect();
        let existing: Vec<bool> = pairs.iter().map(|(path, _)| path.exists()).collect();
        // A failed creation removes the shortcuts that were written before the failure.
        let written: Vec<String> = pairs
            .iter()
            .zip(&existing)
            .filter(|(_, existed)| !**existed)
            .map(|((path, _), _)| path.display().to_string())
            .collect();
        let entry_paths: Vec<String> = pairs
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect();
        match shortcuts::create(&links) {
            Ok(()) => {
                self.journal.push(Entry::Shortcuts {
                    paths: entry_paths,
                    dir_created: folder_created.map(|dir| dir.display().to_string()),
                });
                self.log.push(LogLine::answer("created", Tone::Plain));
            }
            Err(error) => {
                for path in &written {
                    let _ = fs::remove_file(path);
                }
                self.warn(format!("the shortcut was not created: {error}"));
            }
        }
    }

    fn integrate(&mut self) -> SetupResult<()> {
        let options = self.env.options.clone();
        let user = self.env.user.clone();
        let total = [
            options.start_menu,
            options.desktop,
            options.quick_launch,
            options.terminal_profile,
            options.completions,
            options.write_config,
        ]
        .iter()
        .filter(|on| **on)
        .count();
        if total == 0 {
            self.skipped = true;
            return Ok(());
        }
        let mut done = 0f32;
        let step = |done: &mut f32, run: &mut Self| {
            *done += 1.0;
            run.tick(*done / total as f32);
        };
        if options.start_menu {
            self.make_shortcuts("start");
            step(&mut done, self);
        }
        if options.desktop {
            self.make_shortcuts("desktop");
            step(&mut done, self);
        }
        if options.quick_launch {
            self.make_shortcuts("quick");
            step(&mut done, self);
        }
        if options.terminal_profile {
            self.terminal(&user);
            step(&mut done, self);
        }
        if options.completions {
            self.completion(&user);
            step(&mut done, self);
        }
        if options.write_config {
            self.config(&user);
            step(&mut done, self);
        }
        Ok(())
    }

    fn terminal(&mut self, user: &UserPaths) {
        let directory = paths::terminal_fragments(self.scope, user);
        let dir_created = !directory.exists();
        self.log.push(LogLine::command("terminal", "Danaco Nurt"));
        match terminal::write_fragment(&directory, &self.layout.exe(), &self.layout.icon()) {
            Ok(()) => {
                self.journal.push(Entry::TerminalFragment {
                    path: directory
                        .join(terminal::FRAGMENT_FILE)
                        .display()
                        .to_string(),
                    dir_created,
                });
                self.log.push(LogLine::answer(
                    "profile added to Windows Terminal",
                    Tone::Plain,
                ));
            }
            Err(error) => {
                let _ = fs::remove_file(directory.join(terminal::FRAGMENT_FILE));
                if dir_created {
                    let _ = fs::remove_dir(&directory);
                }
                self.warn(format!(
                    "the Windows Terminal profile was not written: {error}"
                ));
                return;
            }
        }
        if self.env.options.terminal_default {
            match terminal::set_default(user) {
                Ok(files) if files.is_empty() && paths::terminal_settings(user).is_empty() => {
                    self.warn(
                        "Windows Terminal has no settings file: the profile is not the default",
                    );
                }
                Ok(files) if files.is_empty() => self.log.push(LogLine::answer(
                    "already the default profile of Windows Terminal",
                    Tone::Plain,
                )),
                Ok(files) => {
                    self.journal.push(Entry::TerminalDefault { files });
                    self.log.push(LogLine::answer(
                        "the default profile of Windows Terminal",
                        Tone::Plain,
                    ));
                }
                Err(error) => self.warn(format!("the default profile was not changed: {error}")),
            }
        }
    }

    fn completion(&mut self, user: &UserPaths) {
        let script = self.layout.completion();
        self.log.push(LogLine::command("completion", "powershell"));
        if !script.is_file() {
            self.warn(format!(
                "the completion script is missing: {}",
                script.display()
            ));
            return;
        }
        match terminal::add_completion(user, &script) {
            Ok(files) => {
                if !files.is_empty() {
                    self.journal.push(Entry::PowershellProfile { files });
                }
                self.log.push(LogLine::answer(
                    "tab completion in the PowerShell profiles",
                    Tone::Plain,
                ));
            }
            Err(error) => self.warn(format!("the completion was not added: {error}")),
        }
    }

    fn config(&mut self, user: &UserPaths) {
        let path = paths::user_config(user);
        self.log
            .push(LogLine::command("config", &path.display().to_string()));
        if path.exists() {
            self.log
                .push(LogLine::answer("exists, left unchanged", Tone::Plain));
            return;
        }
        let Some(parent) = path.parent().map(Path::to_path_buf) else {
            return;
        };
        let dir_created = !parent.exists();
        let written = fs::create_dir_all(&parent).and_then(|()| {
            fs::write(
                &path,
                textedit::config_toml(&self.env.options.config, self.env.version),
            )
        });
        match written {
            Ok(()) => {
                self.journal.push(Entry::ConfigFile {
                    path: path.display().to_string(),
                    dir_created: dir_created.then(|| parent.display().to_string()),
                });
                self.log.push(LogLine::answer("written", Tone::Plain));
            }
            Err(error) => {
                let _ = fs::remove_file(&path);
                self.warn(format!("the configuration was not written: {error}"));
            }
        }
    }

    // ----------------------------------------------------------------------------- close

    fn close(&mut self) -> SetupResult<()> {
        self.carry_over();
        let entries = self.number();
        self.log.push(LogLine::command("journal", "close"));
        self.journal.push(Entry::End {
            entries,
            result: "complete".to_string(),
        });
        self.summary.entries = entries;
        self.log.push(LogLine::answer(
            format!("{entries} entries written"),
            Tone::Ok,
        ));
        Ok(())
    }

    /// Takes over the entries of the journal of an earlier installation for changes that this run
    /// found in place and therefore did not repeat; Undo still has to reverse them.
    fn carry_over(&mut self) {
        let inherited = std::mem::take(&mut self.inherited);
        for old in inherited {
            let present = |records: &[Record], wanted: &Entry| {
                records.iter().any(|record| match (&record.entry, wanted) {
                    (
                        Entry::SetPath {
                            scope: left,
                            entry: a,
                            ..
                        },
                        Entry::SetPath {
                            scope: right,
                            entry: b,
                            ..
                        },
                    ) => left == right && a.eq_ignore_ascii_case(b),
                    (Entry::TerminalDefault { .. }, Entry::TerminalDefault { .. })
                    | (Entry::PowershellProfile { .. }, Entry::PowershellProfile { .. }) => true,
                    _ => false,
                })
            };
            if !present(self.journal.records(), &old) {
                self.journal.push(old);
            }
        }
    }

    fn commit(&mut self) {
        let _ = self.journal.detach();
        if let Err(error) = journal::commit(&self.layout.root) {
            logfile::line(&format!("the journal could not be put in place: {error}"));
        }
        let _ = remove_tree(&self.staging);
        for backup in &self.backups {
            let _ = fs::remove_file(backup);
        }
    }

    /// Removes what the run left outside the journal: the journal being written, the staging
    /// folder and the files that were placed before their entry was written.
    fn clear_traces(&mut self) -> bool {
        let mut clean = true;
        let _ = fs::remove_file(journal::writing_path(&self.layout.root));
        if remove_tree(&self.staging).is_err() {
            clean = false;
        }
        for (path, replaced) in std::mem::take(&mut self.placed).into_iter().rev() {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => clean = false,
            }
            if replaced && fs::rename(backup_of(&path), &path).is_ok() {
                self.backups.retain(|backup| *backup != backup_of(&path));
            }
        }
        clean
    }

    fn rollback(&mut self) -> (bool, u64) {
        let _ = self.journal.detach();
        let mut clean = self.clear_traces();
        let mut records: Vec<Record> = self.journal.records().to_vec();
        let total = records.len().max(1);
        let start = (self.base + self.weight * 0.5).clamp(0.0, 1.0);
        let mut done = 0usize;
        let mut undone = 0u64;
        while let Some(record) = records.pop() {
            if matches!(record.entry, Entry::Begin { .. }) {
                continue;
            }
            // The folders of an earlier installation are not this run's to remove.
            if matches!(record.entry, Entry::CreateDirs { .. })
                && record.n >= 1
                && record.n <= self.inherited_dirs as u64
            {
                continue;
            }
            done += 1;
            let text = describe(&record.entry);
            let overall = start * (1.0 - done as f32 / total as f32);
            (self.env.emit)(ProgressEvent {
                stage: self.stage.to_string(),
                state: StageState::Rollback,
                overall,
                log: vec![
                    LogLine::command("undo", &record.n.to_string()),
                    LogLine::answer(format!("undone {}  {text}", record.n), Tone::Plain),
                ],
            });
            match undo_entry(&record.entry, Mode::Rollback) {
                Ok(()) => {
                    undone += 1;
                    logfile::line(&format!("rollback: {text}"));
                }
                Err(error) => {
                    clean = false;
                    logfile::line(&format!("rollback failed: {text}: {error}"));
                }
            }
            if self.env.pace {
                thread::sleep(Duration::from_millis(70));
            }
        }
        for backup in std::mem::take(&mut self.backups) {
            let original = original_of(&backup);
            if backup.exists() {
                let _ = fs::remove_file(&original);
                if fs::rename(&backup, &original).is_err() {
                    clean = false;
                }
            }
        }
        (clean, undone)
    }
}

fn error_line(error: &SetupError) -> String {
    if error.detail.is_empty() {
        error.code.to_string()
    } else {
        format!("{}  {}", error.code, error.detail)
    }
}

fn is_setup_file(path: &Path, layout: &Layout) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    path.parent() == Some(layout.root.as_path())
        && matches!(
            name.as_str(),
            "uninstall.exe" | "nurt.ico" | "setup-journal.jsonl"
        )
}

/// Path of the backup of a file.
pub(crate) fn backup_of(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}{BACKUP_SUFFIX}", path.display()))
}

fn original_of(backup: &Path) -> PathBuf {
    let text = backup.display().to_string();
    PathBuf::from(text.strip_suffix(BACKUP_SUFFIX).unwrap_or(&text))
}

/// Removes a directory tree; a missing directory is not an error.
pub(crate) fn remove_tree(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Paths of the files that a journal records.
pub(crate) fn written_paths(records: &[Record]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for record in records {
        if let Entry::WriteFiles { dir, files } = &record.entry {
            for file in files {
                out.push(Path::new(dir).join(&file.path));
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------------
// Undo of one entry, used by the rollback of a run and by Undo of a finished installation

/// Undoes one entry of the journal.
pub(crate) fn undo_entry(entry: &Entry, mode: Mode) -> Result<(), String> {
    match entry {
        Entry::Begin { .. } | Entry::End { .. } => Ok(()),
        Entry::CreateDirs { paths } => {
            for path in paths.iter().rev() {
                match fs::remove_dir(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => logfile::line(&format!("the folder stays: {path}: {error}")),
                }
            }
            Ok(())
        }
        Entry::WriteFiles { dir, files } => undo_files(dir, files, mode),
        Entry::Registry { scope, keys } => undo_registry(*scope, keys, mode),
        Entry::SetPath {
            scope,
            entry,
            previous,
        } => {
            if mode == Mode::Rollback {
                registry::write_path(*scope, previous.as_deref()).map_err(|error| error.to_string())
            } else {
                registry::path_delete(*scope, entry)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
        }
        Entry::Shortcuts { paths, dir_created } => {
            for path in paths {
                remove_file_quietly(path)?;
            }
            if let Some(dir) = dir_created {
                let _ = fs::remove_dir(dir);
            }
            Ok(())
        }
        Entry::TerminalFragment { path, dir_created } => {
            remove_file_quietly(path)?;
            if *dir_created && let Some(parent) = Path::new(path).parent() {
                let _ = fs::remove_dir(parent);
            }
            Ok(())
        }
        Entry::TerminalDefault { files } => {
            terminal::release_default(files).map_err(|error| error.to_string())
        }
        Entry::PowershellProfile { files } => {
            terminal::remove_completion(files).map_err(|error| error.to_string())
        }
        Entry::ConfigFile { path, dir_created } => {
            // The configuration of the user is never removed by Undo; a rollback removes the file that the run wrote.
            if mode == Mode::Rollback {
                let _ = fs::remove_file(path);
                if let Some(dir) = dir_created {
                    let _ = fs::remove_dir(dir);
                }
            }
            Ok(())
        }
    }
}

fn remove_file_quietly(path: &str) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{path}: {error}"))
        }
        _ => Ok(()),
    }
}

fn undo_files(dir: &str, files: &[FileRecord], mode: Mode) -> Result<(), String> {
    let mut failed = Vec::new();
    for file in files {
        let path = Path::new(dir).join(&file.path);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error)
                if mode == Mode::Remove
                    && file.path.eq_ignore_ascii_case("uninstall.exe")
                    && matches!(error.raw_os_error(), Some(5 | 32)) =>
            {
                // The uninstaller that started this run still holds the file: a helper deletes it after the exit.
                logfile::line(&format!("deferred until the exit: {}", path.display()));
                crate::app::delete_file_after_exit(&path);
                continue;
            }
            Err(error) => {
                failed.push(format!("{}: {error}", path.display()));
                continue;
            }
        }
        if mode == Mode::Rollback && file.replaced {
            let backup = backup_of(&path);
            if backup.exists() {
                let _ = fs::rename(&backup, &path);
            }
        }
        remove_empty_parents(&path, Path::new(dir));
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

fn undo_registry(scope: Scope, keys: &[KeyRecord], mode: Mode) -> Result<(), String> {
    let mut failed = Vec::new();
    for record in keys {
        let result = if mode == Mode::Rollback {
            registry::restore(scope, &record.key, &record.previous)
        } else {
            registry::delete_tree(scope, &record.key)
        };
        if let Err(error) = result {
            failed.push(format!("{}: {error}", record.key));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

fn remove_empty_parents(path: &Path, stop: &Path) {
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory == stop || !directory.starts_with(stop) {
            break;
        }
        if fs::remove_dir(directory).is_err() {
            break;
        }
        current = directory.parent();
    }
}

// ------------------------------------------------------------------------------------------
// Undo of a finished installation

/// What the undo needs.
pub(crate) struct UndoEnv<'a> {
    /// Root of the installation.
    pub(crate) root: &'a Path,
    /// Directories of the user.
    pub(crate) user: &'a UserPaths,
    /// Removes `nurt.toml` of the user too.
    pub(crate) remove_config: bool,
    /// Receiver of the events of progress.
    pub(crate) emit: &'a dyn Fn(ProgressEvent),
    /// Keeps every stage visible for a minimum time.
    pub(crate) pace: bool,
}

/// The stage of the undo that an entry belongs to.
fn undo_stage(entry: &Entry) -> &'static str {
    match entry {
        Entry::Registry { .. }
        | Entry::SetPath { .. }
        | Entry::TerminalDefault { .. }
        | Entry::PowershellProfile { .. } => "restore",
        Entry::End { .. } | Entry::Begin { .. } => "close",
        _ => "remove",
    }
}

/// Undoes a finished installation from its journal; the entries are read in the reverse order.
pub(crate) fn undo(env: &UndoEnv<'_>) -> Result<Summary, SetupError> {
    let file = env.root.join(journal::FILE_NAME);
    logfile::line(&format!("undo root={}", env.root.display()));
    let started = Instant::now();
    let emit = |stage: &str,
                state: StageState,
                base: f32,
                weight: f32,
                fraction: f32,
                log: Vec<LogLine>| {
        (env.emit)(ProgressEvent {
            stage: stage.to_string(),
            state,
            overall: (base + weight * fraction).clamp(0.0, 1.0),
            log,
        });
    };
    emit(
        "read",
        StageState::Running,
        0.0,
        0.1,
        0.0,
        vec![LogLine::command("journal", "read")],
    );
    let records =
        journal::read(&file).map_err(|detail| SetupError::new("journal-unreadable", detail))?;
    let count = records.len().saturating_sub(1) as u64;
    emit(
        "read",
        StageState::Running,
        0.0,
        0.1,
        1.0,
        vec![LogLine::answer(format!("{count} entries"), Tone::Plain)],
    );
    sleep_to(started, env.pace);
    emit("read", StageState::Done, 0.0, 0.1, 1.0, Vec::new());

    let mut summary = Summary {
        root: env.root.display().to_string(),
        entries: count,
        ..Summary::default()
    };
    if let Some(Record {
        entry:
            Entry::Begin {
                version,
                bin,
                options,
                ..
            },
        ..
    }) = records.first()
    {
        summary.version = version.clone();
        summary.bin = bin.clone();
        summary.scope = options.scope.name().to_string();
    }
    let mut base = 0.1f32;
    let mut current_stage = "remove";
    let mut stage_started = Instant::now();
    let stages = &UNDO_STAGES;
    let weight_of = |stage: &str| {
        stages
            .iter()
            .find(|(name, _)| *name == stage)
            .map_or(0.0, |(_, weight)| *weight)
    };
    let mut announced: Vec<&str> = Vec::new();
    let reversed: Vec<&Record> = records.iter().skip(1).rev().collect();
    let total = reversed.len().max(1);
    for (index, record) in reversed.iter().enumerate() {
        let stage = undo_stage(&record.entry);
        if matches!(record.entry, Entry::End { .. }) {
            continue;
        }
        if stage != current_stage {
            if announced.contains(&current_stage) {
                sleep_to(stage_started, env.pace);
                emit(
                    current_stage,
                    StageState::Done,
                    base,
                    weight_of(current_stage),
                    1.0,
                    Vec::new(),
                );
                base += weight_of(current_stage);
            }
            current_stage = stage;
            stage_started = Instant::now();
        }
        if !announced.contains(&stage) {
            announced.push(stage);
            emit(
                stage,
                StageState::Running,
                base,
                weight_of(stage),
                0.0,
                Vec::new(),
            );
        }
        let text = describe(&record.entry);
        let result = undo_entry(&record.entry, Mode::Remove);
        let mut log = vec![LogLine::command(
            if stage == "restore" {
                "restore"
            } else {
                "remove"
            },
            &text,
        )];
        match result {
            Ok(()) => {
                log.push(LogLine::answer("done", Tone::Plain));
                summary.undone.push(Undone { n: record.n, text });
            }
            Err(error) => {
                log.push(LogLine::answer(error.clone(), Tone::Alert));
                summary.warnings.push(error);
            }
        }
        emit(
            stage,
            StageState::Running,
            base,
            weight_of(stage),
            (index + 1) as f32 / total as f32,
            log,
        );
        if env.pace {
            thread::sleep(Duration::from_millis(80));
        }
    }
    if announced.contains(&current_stage) {
        sleep_to(stage_started, env.pace);
        emit(
            current_stage,
            StageState::Done,
            base,
            weight_of(current_stage),
            1.0,
            Vec::new(),
        );
    }
    for skipped in ["remove", "restore"] {
        if !announced.contains(&skipped) {
            emit(skipped, StageState::Skipped, base, 0.0, 1.0, Vec::new());
        }
    }
    close_undo(env, &emit);
    Ok(summary)
}

type UndoEmit<'a> = dyn Fn(&str, StageState, f32, f32, f32, Vec<LogLine>) + 'a;

/// Last stage of an undo: the configuration on request, then the journal files and the folder.
fn close_undo(env: &UndoEnv<'_>, emit: &UndoEmit<'_>) {
    let closing = Instant::now();
    emit(
        "close",
        StageState::Running,
        0.9,
        0.1,
        0.0,
        vec![LogLine::command("journal", "close")],
    );
    if env.remove_config {
        let config = paths::user_config(env.user);
        let _ = fs::remove_file(&config);
        if let Some(parent) = config.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
    delete_journal_files(env.root);
    sleep_to(closing, env.pace);
    emit(
        "close",
        StageState::Done,
        0.9,
        0.1,
        1.0,
        vec![LogLine::answer("setup undone", Tone::Ok)],
    );
}

fn delete_journal_files(root: &Path) {
    let _ = fs::remove_file(root.join(journal::FILE_NAME));
    let _ = fs::remove_file(journal::writing_path(root));
    // The uninstaller that runs from the root cannot delete itself; the caller removes it after the exit.
    let uninstaller = root.join("uninstall.exe");
    let _ = fs::remove_file(uninstaller);
    let _ = fs::remove_dir(root);
}

/// Summary of the installation that a journal describes, for the maintenance screen.
pub(crate) fn inspect(root: &Path) -> (usize, bool) {
    match journal::read(&root.join(journal::FILE_NAME)) {
        Ok(records) => (
            records.len().saturating_sub(1),
            journal::is_complete(&records),
        ),
        Err(_) => (0, false),
    }
}

/// Options that a journal recorded for the run that wrote it, for Repair.
pub(crate) fn recorded_options(root: &Path) -> Option<Options> {
    match journal::read(&root.join(journal::FILE_NAME))
        .ok()?
        .into_iter()
        .next()?
        .entry
    {
        Entry::Begin { options, .. } => Some(options),
        _ => None,
    }
}

/// Files of an installation whose content differs from the sum in the journal.
pub(crate) fn damaged_files(root: &Path) -> Vec<String> {
    let Ok(records) = journal::read(&root.join(journal::FILE_NAME)) else {
        return Vec::new();
    };
    let mut damaged = Vec::new();
    for record in &records {
        if let Entry::WriteFiles { dir, files } = &record.entry {
            for file in files {
                let path = Path::new(dir).join(&file.path);
                match payload::sha256_of_file(&path) {
                    Ok(sum) if sum == file.sha256 => {}
                    _ => damaged.push(path.display().to_string()),
                }
            }
        }
    }
    damaged
}

/// Entries of the journal of an earlier installation that a later run has to keep: the changes
/// of `PATH`, of the default profile and of the PowerShell profiles stay in place when the run
/// finds them done, and Undo still has to reverse them.
fn inheritable(file: &Path) -> Vec<Entry> {
    entries_of(file, |entry| {
        matches!(
            entry,
            Entry::SetPath { .. } | Entry::TerminalDefault { .. } | Entry::PowershellProfile { .. }
        )
    })
}

/// The folders that an earlier installation created; they stay recorded so that Undo removes them.
fn inherited_dirs(file: &Path) -> Vec<Entry> {
    entries_of(file, |entry| matches!(entry, Entry::CreateDirs { .. }))
}

fn entries_of(file: &Path, keep: fn(&Entry) -> bool) -> Vec<Entry> {
    let Ok(records) = journal::read(file) else {
        return Vec::new();
    };
    records
        .into_iter()
        .map(|record| record.entry)
        .filter(keep)
        .collect()
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
