// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Service definitions read from launchd property lists.
//!
//! Property lists are read in XML and binary form. Directories are read
//! below a system root, so tests read a synthetic tree; paths in records and
//! in `launchctl` arguments are the paths on the system.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use nurt_contracts::record::{ServiceScope, StartType};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::ExecCtx;
use plist::{Dictionary, Value as Plist};

/// Directories of system services (launch daemons).
const SYSTEM_DIRECTORIES: [&str; 2] = ["/System/Library/LaunchDaemons", "/Library/LaunchDaemons"];
/// Directories of user services (launch agents) shared by all accounts.
const USER_DIRECTORIES: [&str; 2] = ["/System/Library/LaunchAgents", "/Library/LaunchAgents"];
/// Directory of the launch agents of one account, below its home.
const HOME_AGENTS: &str = "Library/LaunchAgents";
/// Extension of property list files.
const EXTENSION: &str = "plist";
/// Directory protected by System Integrity Protection.
pub(crate) const PROTECTED_PREFIX: &str = "/System/";

/// Keys that start a service on an event rather than at load.
const ON_DEMAND_KEYS: [&str; 6] = [
    "StartInterval",
    "StartCalendarInterval",
    "WatchPaths",
    "QueueDirectories",
    "Sockets",
    "MachServices",
];

/// Locations used by the launchd backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    /// System root below which directories are read; `/` on a real system.
    pub(crate) root: PathBuf,
    /// Home directory of the account, as a path on the system.
    pub(crate) home: String,
    /// User identifier of the account, for the user domains.
    pub(crate) uid: u32,
}

impl Layout {
    /// Directories of the scope in the order launchd reads them.
    pub(crate) fn directories(&self, scope: ServiceScope) -> Vec<String> {
        match scope {
            ServiceScope::User => USER_DIRECTORIES
                .iter()
                .map(|directory| (*directory).to_owned())
                .chain(std::iter::once(format!(
                    "{}/{HOME_AGENTS}",
                    self.home.trim_end_matches('/')
                )))
                .collect(),
            _ => SYSTEM_DIRECTORIES
                .iter()
                .map(|directory| (*directory).to_owned())
                .collect(),
        }
    }

    /// File below the root for a path on the system.
    pub(crate) fn physical(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
}

/// Fields of a property list used by the backend.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Definition {
    /// `Label`.
    pub(crate) label: String,
    /// Property list file, as a path on the system.
    pub(crate) path: String,
    /// `Program`, or the first element of `ProgramArguments`.
    pub(crate) program: Option<String>,
    /// `ProgramArguments`.
    pub(crate) arguments: Vec<String>,
    /// `RunAtLoad`.
    pub(crate) run_at_load: bool,
    /// `KeepAlive` set to `true` or to a dictionary of conditions.
    pub(crate) keep_alive: bool,
    /// `Disabled`.
    pub(crate) disabled: bool,
    /// `UserName`.
    pub(crate) user_name: Option<String>,
    /// One of the keys that start the service on an event.
    pub(crate) on_demand: bool,
}

fn flag(dictionary: &Dictionary, key: &str) -> bool {
    dictionary.get(key).and_then(Plist::as_boolean) == Some(true)
}

fn text(dictionary: &Dictionary, key: &str) -> Option<String> {
    dictionary
        .get(key)
        .and_then(Plist::as_string)
        .map(str::to_owned)
}

/// Reads a property list in XML or binary form.
///
/// # Errors
///
/// A description of the problem when the content is not a property list,
/// is not a dictionary or has no `Label`.
pub(crate) fn parse(bytes: &[u8], path: &str) -> Result<Definition, String> {
    let value = Plist::from_reader(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let dictionary = value
        .as_dictionary()
        .ok_or_else(|| "the property list is not a dictionary".to_owned())?;
    let label =
        text(dictionary, "Label").ok_or_else(|| "the property list has no Label".to_owned())?;
    let arguments: Vec<String> = dictionary
        .get("ProgramArguments")
        .and_then(Plist::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Plist::as_string)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let keep_alive = match dictionary.get("KeepAlive") {
        Some(Plist::Boolean(flag)) => *flag,
        Some(Plist::Dictionary(_)) => true,
        _ => false,
    };
    Ok(Definition {
        label,
        path: path.to_owned(),
        program: text(dictionary, "Program").or_else(|| arguments.first().cloned()),
        arguments,
        run_at_load: flag(dictionary, "RunAtLoad"),
        keep_alive,
        disabled: flag(dictionary, "Disabled"),
        user_name: text(dictionary, "UserName"),
        on_demand: ON_DEMAND_KEYS
            .iter()
            .any(|key| dictionary.contains_key(key)),
    })
}

/// Start type of a service.
///
/// An override of `launchctl print-disabled` takes precedence over the
/// `Disabled` key; a service without a property list has an unknown start
/// type unless it is disabled.
pub(crate) fn start_type(definition: Option<&Definition>, disabled: Option<bool>) -> StartType {
    let disabled = disabled.unwrap_or_else(|| definition.is_some_and(|known| known.disabled));
    match definition {
        _ if disabled => StartType::Disabled,
        None => StartType::Unknown,
        Some(known) if known.run_at_load || known.keep_alive => StartType::Automatic,
        Some(known) if known.on_demand => StartType::OnDemand,
        Some(_) => StartType::Manual,
    }
}

fn skipped(ctx: &ExecCtx, path: &str, problem: &str) {
    ctx.note(NotableEntry {
        path: Some(path.to_owned()),
        detail: Some(format!("property list skipped: {problem}")),
        ..NotableEntry::new(Notable::SourceUnavailable)
    });
}

fn read(layout: &Layout, path: &str, ctx: &ExecCtx) -> Option<Definition> {
    let parsed = std::fs::read(layout.physical(path))
        .map_err(|error| error.to_string())
        .and_then(|bytes| parse(&bytes, path));
    match parsed {
        Ok(definition) => Some(definition),
        Err(problem) => {
            skipped(ctx, path, &problem);
            None
        }
    }
}

fn plist_files(layout: &Layout, directory: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(layout.physical(directory)) else {
        return Vec::new();
    };
    let mut files: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext == EXTENSION)
        })
        .filter_map(|name| name.to_str().map(|name| format!("{directory}/{name}")))
        .collect();
    files.sort();
    files
}

/// Every definition of the scope; for a label defined twice the first
/// directory wins. Unreadable files are skipped with a note.
pub(crate) fn read_all(layout: &Layout, scope: ServiceScope, ctx: &ExecCtx) -> Vec<Definition> {
    let mut definitions: Vec<Definition> = Vec::new();
    for directory in layout.directories(scope) {
        for path in plist_files(layout, &directory) {
            if let Some(definition) = read(layout, &path, ctx)
                && !definitions
                    .iter()
                    .any(|known| known.label == definition.label)
            {
                definitions.push(definition);
            }
        }
    }
    definitions
}

/// The definition of `label`: first the conventional file name
/// `<label>.plist`, then every file of the scope.
pub(crate) fn find(
    layout: &Layout,
    scope: ServiceScope,
    label: &str,
    ctx: &ExecCtx,
) -> Option<Definition> {
    for directory in layout.directories(scope) {
        let path = format!("{directory}/{label}.{EXTENSION}");
        if layout.physical(&path).is_file()
            && let Ok(bytes) = std::fs::read(layout.physical(&path))
            && let Ok(definition) = parse(&bytes, &path)
            && definition.label == label
        {
            return Some(definition);
        }
    }
    read_all(layout, scope, ctx)
        .into_iter()
        .find(|definition| definition.label == label)
}
