// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/journal.rs
// Responsibility:   Journal of the setup: a JSON Lines file with one entry for every change of an installation, read in reverse by Repair and Undo.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Journal of the setup.
//!
//! The journal is a file of the setup and not the journal of `nurt`. It is written
//! next to the program, in `setup-journal.jsonl` of the root of the installation.
//! Every line is one JSON object: the number of the entry `n`, the UTC time `t`,
//! the kind of the change `op` and the fields of that kind. The format is
//! `nurt-setup-journal/1`; `packaging/windows-installer/README.md` describes every
//! kind.
//!
//! A run writes `setup-journal.jsonl.new`; the run that ends well replaces the
//! journal of an earlier installation with it, so that a failed run leaves the
//! earlier journal untouched.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::logfile;
use crate::model::{Options, Scope};
use crate::registry::KeyRecord;

/// Name of the journal in the root of the installation.
pub(crate) const FILE_NAME: &str = "setup-journal.jsonl";

/// Suffix of the journal that a run is writing.
pub(crate) const WRITING_SUFFIX: &str = ".new";

/// Identifier of the format of the journal.
pub(crate) const FORMAT: &str = "nurt-setup-journal/1";

/// A file that the setup wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileRecord {
    /// Path relative to the folder of the entry.
    pub(crate) path: String,
    /// SHA-256 sum of the content that was written.
    pub(crate) sha256: String,
    /// Size in bytes.
    pub(crate) size: u64,
    /// A file of an earlier installation was replaced.
    pub(crate) replaced: bool,
}

/// A settings file of Windows Terminal whose default profile was changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SettingsRecord {
    /// Path of `settings.json`.
    pub(crate) path: String,
    /// The default profile before the change; `None` when the file named none.
    pub(crate) previous: Option<String>,
}

/// A PowerShell profile that got the block of the completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProfileRecord {
    /// Path of the profile.
    pub(crate) path: String,
    /// The profile existed before.
    pub(crate) existed: bool,
}

/// One change of an installation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Entry {
    /// Start of a run: the facts that Repair needs to run it again.
    Begin {
        /// Identifier of the format.
        format: String,
        /// Version that the run installs.
        version: String,
        /// Target of the setup.
        target: String,
        /// Folder of `nurt.exe`.
        bin: String,
        /// Root of the installation.
        root: String,
        /// Choices of the user.
        options: Options,
    },
    /// Folders that the run created, parents first.
    CreateDirs {
        /// Paths.
        paths: Vec<String>,
    },
    /// Files that the run wrote below a folder.
    WriteFiles {
        /// The folder.
        dir: String,
        /// The files.
        files: Vec<FileRecord>,
    },
    /// Registry keys that the run wrote, with their state before.
    Registry {
        /// Hive: the scope of the installation.
        scope: Scope,
        /// The keys.
        keys: Vec<KeyRecord>,
    },
    /// The folder of `nurt.exe` was added to `PATH`.
    SetPath {
        /// Scope of the `PATH`.
        scope: Scope,
        /// The entry that was added.
        entry: String,
        /// The value before; `None` when `PATH` did not exist.
        previous: Option<String>,
    },
    /// Shortcuts that the run created.
    Shortcuts {
        /// Paths of the `.lnk` files.
        paths: Vec<String>,
        /// A folder of the Start menu that the run created.
        dir_created: Option<String>,
    },
    /// The fragment of Windows Terminal with the profile.
    TerminalFragment {
        /// Path of the fragment file.
        path: String,
        /// The folder of the fragment was created by the run.
        dir_created: bool,
    },
    /// The profile became the default profile of Windows Terminal.
    TerminalDefault {
        /// The settings files that changed.
        files: Vec<SettingsRecord>,
    },
    /// The block of the completion was added to PowerShell profiles.
    PowershellProfile {
        /// The profiles that changed.
        files: Vec<ProfileRecord>,
    },
    /// The first configuration of the program.
    ConfigFile {
        /// Path of `nurt.toml`.
        path: String,
        /// The folder was created by the run.
        dir_created: Option<String>,
    },
    /// End of a run.
    End {
        /// Number of the entries written before this one.
        entries: u64,
        /// `complete`.
        result: String,
    },
}

/// A line of the file: the entry with its number and time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    /// Number of the entry; the beginning is 0.
    pub(crate) n: u64,
    /// UTC time, ISO 8601.
    pub(crate) t: String,
    /// The change.
    #[serde(flatten)]
    pub(crate) entry: Entry,
}

/// A journal that a run writes.
#[derive(Debug, Default)]
pub(crate) struct Journal {
    records: Vec<Record>,
    file: Option<File>,
    path: Option<PathBuf>,
}

impl Journal {
    /// An empty journal that keeps its entries in memory.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends an entry and returns its number.
    pub(crate) fn push(&mut self, entry: Entry) -> u64 {
        let n = self.records.len() as u64;
        let record = Record {
            n,
            t: logfile::timestamp(),
            entry,
        };
        self.write_line(&record);
        self.records.push(record);
        n
    }

    fn write_line(&mut self, record: &Record) {
        if let Some(file) = self.file.as_mut()
            && let Ok(line) = serde_json::to_string(record)
        {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }

    /// Starts to write the entries to a file; the entries so far are written first.
    pub(crate) fn attach(&mut self, path: &Path) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        for record in &self.records {
            if let Ok(line) = serde_json::to_string(record) {
                writeln!(file, "{line}")?;
            }
        }
        file.flush()?;
        self.file = Some(file);
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Closes the file and returns its path.
    pub(crate) fn detach(&mut self) -> Option<PathBuf> {
        self.file = None;
        self.path.take()
    }

    /// The entries written so far.
    pub(crate) fn records(&self) -> &[Record] {
        &self.records
    }
}

/// Reads a journal; a line that cannot be read makes the journal unreadable.
pub(crate) fn read(path: &Path) -> Result<Vec<Record>, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| format!("{}: {error}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let record: Record = serde_json::from_str(&line)
            .map_err(|error| format!("{}:{}: {error}", path.display(), index + 1))?;
        records.push(record);
    }
    match records.first() {
        Some(Record {
            entry: Entry::Begin { format, .. },
            ..
        }) if format == FORMAT => Ok(records),
        Some(_) => Err(format!(
            "{}: the first entry is not a beginning of the format {FORMAT}",
            path.display()
        )),
        None => Err(format!("{}: the journal is empty", path.display())),
    }
}

/// The journal ended with the entry `end`.
pub(crate) fn is_complete(records: &[Record]) -> bool {
    matches!(
        records.last(),
        Some(Record {
            entry: Entry::End { .. },
            ..
        })
    )
}

/// Replaces the journal of an installation with the one that a run wrote.
pub(crate) fn commit(root: &Path) -> std::io::Result<()> {
    let writing = root.join(format!("{FILE_NAME}{WRITING_SUFFIX}"));
    fs::rename(writing, root.join(FILE_NAME))
}

/// Path of the journal that a run writes.
pub(crate) fn writing_path(root: &Path) -> PathBuf {
    root.join(format!("{FILE_NAME}{WRITING_SUFFIX}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn begin() -> Entry {
        Entry::Begin {
            format: FORMAT.to_string(),
            version: "0.1.2".into(),
            target: "x86_64-pc-windows-msvc".into(),
            bin: "C:\\N\\bin".into(),
            root: "C:\\N".into(),
            options: Options::default(),
        }
    }

    #[test]
    fn records_round_trip_through_lines() {
        let mut journal = Journal::new();
        journal.push(begin());
        journal.push(Entry::CreateDirs {
            paths: vec!["C:\\N".into(), "C:\\N\\bin".into()],
        });
        journal.push(Entry::WriteFiles {
            dir: "C:\\N\\bin".into(),
            files: vec![FileRecord {
                path: "nurt.exe".into(),
                sha256: "ab".repeat(32),
                size: 7,
                replaced: false,
            }],
        });
        journal.push(Entry::End {
            entries: 3,
            result: "complete".into(),
        });
        let text: Vec<String> = journal
            .records()
            .iter()
            .filter_map(|record| serde_json::to_string(record).ok())
            .collect();
        assert!(text[1].contains("\"op\":\"create_dirs\""));
        let back: Vec<Record> = text
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert_eq!(back, journal.records());
        assert!(is_complete(&back));
    }

    #[test]
    fn numbers_start_at_zero() {
        let mut journal = Journal::new();
        assert_eq!(journal.push(begin()), 0);
        assert_eq!(journal.push(Entry::CreateDirs { paths: vec![] }), 1);
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("nurt-journal-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap_or_default();
        path
    }

    fn every_kind() -> Vec<Entry> {
        vec![
            begin(),
            Entry::CreateDirs {
                paths: vec!["C:\\N".into()],
            },
            Entry::WriteFiles {
                dir: "C:\\N".into(),
                files: vec![],
            },
            Entry::Registry {
                scope: Scope::Machine,
                keys: vec![KeyRecord {
                    key: "Software\\X".into(),
                    previous: crate::registry::Snapshot {
                        existed: true,
                        values: vec![crate::registry::StoredValue {
                            name: "v".into(),
                            kind: 1,
                            data: "6100".into(),
                        }],
                    },
                }],
            },
            Entry::SetPath {
                scope: Scope::User,
                entry: "C:\\N\\bin".into(),
                previous: None,
            },
            Entry::SetPath {
                scope: Scope::User,
                entry: "C:\\N\\bin".into(),
                previous: Some("C:\\Windows".into()),
            },
            Entry::Shortcuts {
                paths: vec!["C:\\a.lnk".into()],
                dir_created: Some("C:\\S".into()),
            },
            Entry::TerminalFragment {
                path: "C:\\f\\nurt.json".into(),
                dir_created: true,
            },
            Entry::TerminalDefault {
                files: vec![SettingsRecord {
                    path: "C:\\s.json".into(),
                    previous: None,
                }],
            },
            Entry::PowershellProfile {
                files: vec![ProfileRecord {
                    path: "C:\\p.ps1".into(),
                    existed: false,
                }],
            },
            Entry::ConfigFile {
                path: "C:\\n.toml".into(),
                dir_created: None,
            },
            Entry::End {
                entries: 11,
                result: "complete".into(),
            },
        ]
    }

    #[test]
    fn every_kind_has_its_name_and_survives_the_file() {
        let directory = scratch("kinds");
        let file = directory.join(FILE_NAME);
        let mut journal = Journal::new();
        assert!(journal.attach(&file).is_ok());
        for entry in every_kind() {
            journal.push(entry);
        }
        journal.detach();
        let text = fs::read_to_string(&file).unwrap_or_default();
        for name in [
            "begin",
            "create_dirs",
            "write_files",
            "registry",
            "set_path",
            "shortcuts",
            "terminal_fragment",
            "terminal_default",
            "powershell_profile",
            "config_file",
            "end",
        ] {
            assert!(text.contains(&format!("\"op\":\"{name}\"")), "{name}");
        }
        assert_eq!(text.lines().count(), every_kind().len());
        let back = read(&file).unwrap_or_default();
        assert_eq!(back, journal.records());
        assert!(
            back.iter()
                .enumerate()
                .all(|(index, record)| record.n == index as u64)
        );
        assert!(is_complete(&back));
        assert!(
            text.lines()
                .next()
                .is_some_and(|line| line.contains(FORMAT))
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn entries_reach_the_file_as_they_are_pushed() {
        let directory = scratch("incremental");
        let file = writing_path(&directory);
        let mut journal = Journal::new();
        journal.push(begin());
        assert!(journal.attach(&file).is_ok());
        assert_eq!(
            fs::read_to_string(&file)
                .unwrap_or_default()
                .lines()
                .count(),
            1
        );
        journal.push(Entry::CreateDirs { paths: vec![] });
        assert_eq!(
            fs::read_to_string(&file)
                .unwrap_or_default()
                .lines()
                .count(),
            2
        );
        let records = read(&file).unwrap_or_default();
        assert_eq!(records.len(), 2);
        assert!(
            !is_complete(&records),
            "a run that did not end is not complete"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_finished_run_replaces_the_old_journal_and_a_failed_one_does_not() {
        let directory = scratch("commit");
        let old = directory.join(FILE_NAME);
        fs::write(&old, "old\n").unwrap_or_default();
        let mut journal = Journal::new();
        assert!(journal.attach(&writing_path(&directory)).is_ok());
        journal.push(begin());
        journal.detach();
        assert_eq!(fs::read_to_string(&old).unwrap_or_default(), "old\n");
        assert!(commit(&directory).is_ok());
        assert!(!writing_path(&directory).exists());
        assert!(read(&old).is_ok());
        assert!(commit(&directory).is_err(), "nothing is left to commit");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_journal_that_cannot_be_read_is_refused() {
        let directory = scratch("refused");
        let file = directory.join(FILE_NAME);
        assert!(read(&file).is_err(), "missing");
        fs::write(&file, "").unwrap_or_default();
        assert!(read(&file).is_err(), "empty");
        fs::write(&file, "not json\n").unwrap_or_default();
        assert!(read(&file).is_err(), "garbage");
        let mut other = Journal::new();
        other.push(Entry::Begin {
            format: "other/1".into(),
            version: "0.1.2".into(),
            target: "t".into(),
            bin: "b".into(),
            root: "r".into(),
            options: Options::default(),
        });
        let line = serde_json::to_string(&other.records()[0]).unwrap_or_default();
        fs::write(&file, format!("{line}\n")).unwrap_or_default();
        assert!(read(&file).is_err(), "other format");
        let mut good = Journal::new();
        good.push(begin());
        good.push(Entry::CreateDirs { paths: vec![] });
        let lines: Vec<String> = good
            .records()
            .iter()
            .filter_map(|record| serde_json::to_string(record).ok())
            .collect();
        fs::write(&file, format!("{}\n\n{}\n", lines[0], lines[1])).unwrap_or_default();
        assert_eq!(
            read(&file).map(|records| records.len()),
            Ok(2),
            "blank lines are skipped"
        );
        let _ = fs::remove_dir_all(&directory);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
