// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/model.rs
// Responsibility:   Data model shared by the interface and the engine: scope, options, layout, events and results.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Data model of the setup program.
//!
//! The types that cross to the elevated worker or into the journal are serialised
//! with serde; the others are plain data.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Scope of an installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Scope {
    /// For the current user: no administrator rights.
    #[default]
    User,
    /// For all users: administrator rights, `%ProgramFiles%`.
    Machine,
}

impl Scope {
    /// Name used in messages and in the journal.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Machine => "machine",
        }
    }
}

/// First configuration of the program, written to `nurt.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ConfigChoices {
    /// `output.language`: `en` or `pl`.
    pub(crate) language: String,
    /// `output.color`: `auto`, `always` or `never`.
    pub(crate) color: String,
    /// `syntax.variant`: `a`, `b` or `c`.
    pub(crate) syntax: String,
}

impl Default for ConfigChoices {
    fn default() -> Self {
        Self {
            language: "en".into(),
            color: "auto".into(),
            syntax: "a".into(),
        }
    }
}

/// Choices of the user for one installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Options {
    /// Scope of the installation.
    pub(crate) scope: Scope,
    /// Folder of `nurt.exe`; empty selects the default of the scope.
    pub(crate) dir: String,
    /// Adds the folder to `PATH`.
    pub(crate) path: bool,
    /// Installs the documentation (effective when the folder is named `bin`).
    pub(crate) docs: bool,
    /// Shortcuts in the Start menu.
    pub(crate) start_menu: bool,
    /// Profile of Windows Terminal.
    pub(crate) terminal_profile: bool,
    /// The profile becomes the default profile of Windows Terminal.
    pub(crate) terminal_default: bool,
    /// Shortcut on the desktop.
    pub(crate) desktop: bool,
    /// Shortcut in the Quick Launch folder.
    pub(crate) quick_launch: bool,
    /// Tab completion in PowerShell.
    pub(crate) completions: bool,
    /// Writes the first configuration.
    pub(crate) write_config: bool,
    /// Values of the first configuration.
    pub(crate) config: ConfigChoices,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            scope: Scope::User,
            dir: String::new(),
            path: true,
            docs: true,
            start_menu: true,
            terminal_profile: false,
            terminal_default: false,
            desktop: false,
            quick_launch: false,
            completions: false,
            write_config: true,
            config: ConfigChoices::default(),
        }
    }
}

/// Directories of the user for whom the program is installed.
///
/// An elevated process may run as another account than the user who started
/// the wizard, so the directories are passed to it explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UserPaths {
    /// `%USERPROFILE%`.
    pub(crate) profile: PathBuf,
    /// `%APPDATA%`.
    pub(crate) appdata: PathBuf,
    /// `%LOCALAPPDATA%`.
    pub(crate) local_appdata: PathBuf,
    /// The Documents folder.
    pub(crate) documents: PathBuf,
    /// The desktop folder.
    pub(crate) desktop: PathBuf,
}

/// Places of one installation, derived from the folder of `nurt.exe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Folder of `nurt.exe`.
    pub(crate) bin: PathBuf,
    /// Root of the installation: the parent of a folder named `bin`, otherwise the folder itself.
    pub(crate) root: PathBuf,
    /// The documentation goes to `root\doc`.
    pub(crate) docs: bool,
}

impl Layout {
    /// Derives the places from the folder of `nurt.exe`; `docs` is the choice of the user.
    pub(crate) fn of(bin: &Path, docs: bool) -> Self {
        let named_bin = bin
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("bin"));
        let root = if named_bin {
            bin.parent()
                .map_or_else(|| bin.to_path_buf(), Path::to_path_buf)
        } else {
            bin.to_path_buf()
        };
        Self {
            bin: bin.to_path_buf(),
            root,
            docs: docs && named_bin,
        }
    }

    /// Folder of the documentation.
    pub(crate) fn doc(&self) -> PathBuf {
        self.root.join("doc")
    }

    /// Place of `nurt.exe`.
    pub(crate) fn exe(&self) -> PathBuf {
        self.bin.join("nurt.exe")
    }

    /// Icon of the program written by the setup.
    pub(crate) fn icon(&self) -> PathBuf {
        self.root.join("nurt.ico")
    }

    /// The uninstaller: a copy of the setup.
    pub(crate) fn uninstaller(&self) -> PathBuf {
        self.root.join("uninstall.exe")
    }

    /// File of the journal of the setup.
    pub(crate) fn journal(&self) -> PathBuf {
        self.root.join(crate::journal::FILE_NAME)
    }

    /// Script of the PowerShell completion.
    pub(crate) fn completion(&self) -> PathBuf {
        if self.docs {
            self.doc().join("completions").join("_nurt.ps1")
        } else {
            self.root.join("completions").join("_nurt.ps1")
        }
    }
}

/// An installation found on the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Existing {
    /// Installed version.
    pub(crate) version: String,
    /// Root of the installation.
    pub(crate) root: PathBuf,
    /// Scope of the installation.
    pub(crate) scope: Scope,
    /// Number of the entries of its journal; 0 when the journal is missing.
    pub(crate) entries: usize,
    /// The journal exists and can be read.
    pub(crate) journal_ok: bool,
}

/// Result of one check of the requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Requirement {
    /// `windows`, `architecture`, `space` or `package`.
    pub(crate) id: &'static str,
    /// The requirement is met.
    pub(crate) ok: bool,
    /// Value found.
    pub(crate) found: String,
    /// Value required.
    pub(crate) required: String,
}

/// Result of the check of the folder of `nurt.exe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocationReport {
    /// Folder after expansion of the variables.
    pub(crate) dir: String,
    /// The first problem; `None` when the folder is acceptable.
    pub(crate) issue: Option<Issue>,
    /// Free space of the volume in bytes; 0 when unknown.
    pub(crate) free: u64,
    /// Space that the installation needs in bytes.
    pub(crate) required: u64,
}

/// A problem of the folder of `nurt.exe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Issue {
    /// Not a full path or with characters Windows does not accept.
    NotFullPath,
    /// For all users the folder must be inside `%ProgramFiles%`.
    OutsideProgramFiles,
    /// The path is a file.
    IsFile,
    /// The folder cannot be written.
    NotWritable,
    /// Not enough free space.
    NoSpace,
}

/// Tone of a line of the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Tone {
    /// Plain answer.
    Plain,
    /// Success: silver.
    Ok,
    /// Failure: alert.
    Alert,
}

/// A line of the log of a run, drawn like a session of `nurt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum LogLine {
    /// `nurt> name "argument"`.
    Command {
        /// Command name.
        name: String,
        /// Argument; empty for none.
        arg: String,
    },
    /// An answer indented by two spaces.
    Answer {
        /// Text.
        text: String,
        /// Tone.
        tone: Tone,
    },
}

impl LogLine {
    /// A command line.
    pub(crate) fn command(name: &str, arg: &str) -> Self {
        Self::Command {
            name: name.to_string(),
            arg: arg.to_string(),
        }
    }

    /// An answer line.
    pub(crate) fn answer(text: impl Into<String>, tone: Tone) -> Self {
        Self::Answer {
            text: text.into(),
            tone,
        }
    }
}

/// State of a stage of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum StageState {
    /// The stage runs.
    Running,
    /// The stage is done.
    Done,
    /// The stage had nothing to do.
    Skipped,
    /// The stage failed.
    Failed,
    /// The entries of the stage are being undone.
    Rollback,
}

/// Event of the progress of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProgressEvent {
    /// Identifier of the stage.
    pub(crate) stage: String,
    /// State of the stage.
    pub(crate) state: StageState,
    /// Progress of the whole run, 0 to 1.
    pub(crate) overall: f32,
    /// Lines for the log.
    pub(crate) log: Vec<LogLine>,
}

/// One entry of the journal that was undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Undone {
    /// Number of the entry.
    pub(crate) n: u64,
    /// Description of the entry.
    pub(crate) text: String,
}

/// Result of a finished run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct Summary {
    /// Folder of `nurt.exe`.
    pub(crate) bin: String,
    /// Root of the installation.
    pub(crate) root: String,
    /// Version.
    pub(crate) version: String,
    /// Scope: `user` or `machine`.
    pub(crate) scope: String,
    /// Number of the entries of the journal.
    pub(crate) entries: u64,
    /// `PATH` was changed.
    pub(crate) path_changed: bool,
    /// The entries that were undone, in the order of the undo.
    pub(crate) undone: Vec<Undone>,
    /// Problems that did not stop the run.
    pub(crate) warnings: Vec<String>,
}

/// Event that ends a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FinishEvent {
    /// The run succeeded.
    pub(crate) ok: bool,
    /// The user cancelled the run.
    pub(crate) cancelled: bool,
    /// Code of the failure; see `error.rs`.
    pub(crate) code: String,
    /// Detail of the failure.
    pub(crate) detail: String,
    /// Stage that failed.
    pub(crate) stage: String,
    /// The changes were rolled back completely.
    pub(crate) rolled_back: bool,
    /// Number of the entries that were undone.
    pub(crate) undone: u64,
    /// Result of a successful run.
    pub(crate) summary: Option<Summary>,
}

/// What a worker sends to the interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub(crate) enum WorkerEvent {
    /// Progress of a stage.
    Progress(ProgressEvent),
    /// The run ended.
    Finish(FinishEvent),
}

/// What the run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum JobKind {
    /// Installs, repairs or updates.
    Install,
    /// Undoes the setup.
    Undo,
}

/// Job handed to the elevated worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Job {
    /// What to do.
    pub(crate) kind: JobKind,
    /// Choices of the user (for an installation).
    pub(crate) options: Options,
    /// Directories of the user who started the setup.
    pub(crate) user: UserPaths,
    /// Root of the installation to undo.
    pub(crate) root: PathBuf,
    /// Scope of the installation to undo.
    pub(crate) scope: Scope,
    /// File that receives the events as JSON lines.
    pub(crate) events: PathBuf,
    /// File whose existence cancels the run.
    pub(crate) cancel: PathBuf,
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
