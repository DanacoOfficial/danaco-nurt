// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Desktop entries of Linux applications.
//!
//! Entries are read from `$XDG_DATA_HOME/applications` (default
//! `~/.local/share/applications`), from `applications` of every directory
//! of `$XDG_DATA_DIRS` (default `/usr/local/share:/usr/share`) and from the
//! export directories of Flatpak and Snap. The first entry with a desktop
//! file id wins. Only `Type=Application` entries of the `[Desktop Entry]`
//! group that are neither `NoDisplay` nor `Hidden` are used. The command
//! of `Exec` is its first argument without environment assignments and
//! field codes; the index merges the entry with the program of that name.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::model::{
    CategoryHint, CategoryScheme, EntryKey, EntryKind, LocalizedText, RawProgram, SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError, SourceId,
    SourceKind,
};
use crate::sources::{is_dir, list_dir, read_prefix};

const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";
const EXTRA_DIRS: &[&str] = &[
    "/var/lib/flatpak/exports/share/applications",
    "/var/lib/snapd/desktop/applications",
];
const USER_FLATPAK_DIR: &str = ".local/share/flatpak/exports/share/applications";
const USER_DATA_DIR: &str = ".local/share";
const APPLICATIONS: &str = "applications";
const DESKTOP_SUFFIX: &str = ".desktop";
const ENTRY_GROUP: &str = "[Desktop Entry]";
/// Largest desktop file read.
const MAX_ENTRY_BYTES: u64 = 256 * 1024;
/// Deepest subdirectory of an applications directory that is read.
const MAX_DEPTH: usize = 3;

/// Parsed `[Desktop Entry]` group: keys with their localized variants.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesktopEntry {
    values: BTreeMap<String, String>,
}

impl DesktopEntry {
    /// Parses the `[Desktop Entry]` group of a desktop file.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut values = BTreeMap::new();
        let mut inside = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                inside = line == ENTRY_GROUP;
                continue;
            }
            if !inside || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                values
                    .entry(key.trim().to_owned())
                    .or_insert_with(|| unescape(value.trim()));
            }
        }
        Self { values }
    }

    /// Unlocalized value of a key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// Value of a key with its `key[lang]` variants.
    #[must_use]
    pub fn localized(&self, key: &str) -> Option<LocalizedText> {
        let mut text = LocalizedText::in_lang(self.get(key)?, "en");
        let prefix = format!("{key}[");
        for (name, value) in self.values.range(prefix.clone()..) {
            let Some(locale) = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                break;
            };
            let lang = locale.split(['_', '.', '@']).next().unwrap_or(locale);
            if !lang.is_empty() && !value.is_empty() && !text.translations.contains_key(lang) {
                text.set(lang, value.clone());
            }
        }
        Some(text)
    }

    /// Returns `true` when a boolean key is `true`.
    #[must_use]
    pub fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("true")
    }

    /// List value split at `;`.
    #[must_use]
    pub fn list(&self, key: &str) -> Vec<String> {
        self.get(key)
            .map(|value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The entry is an application shown to the user.
    #[must_use]
    pub fn is_visible_application(&self) -> bool {
        self.get("Type") == Some("Application") && !self.flag("NoDisplay") && !self.flag("Hidden")
    }
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Options of `env` that take the following argument as their value.
const ENV_OPTIONS_WITH_VALUE: &[&str] = &["-u", "--unset", "-C", "--chdir"];

/// Command of an `Exec` value: the first argument after `env`, its options
/// and variable assignments, with quotes removed.
#[must_use]
pub fn exec_command(exec: &str) -> Option<String> {
    let arguments = split_exec(exec);
    let mut index = 0;
    if arguments
        .first()
        .is_some_and(|first| first.rsplit('/').next() == Some("env"))
    {
        index = 1;
        while let Some(argument) = arguments.get(index) {
            if ENV_OPTIONS_WITH_VALUE.contains(&argument.as_str()) {
                index += 2;
            } else if argument.starts_with('-') || argument.contains('=') {
                index += 1;
            } else {
                break;
            }
        }
    }
    arguments
        .get(index)
        .filter(|argument| !argument.starts_with('%'))
        .cloned()
}

fn split_exec(exec: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    arguments.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    arguments
}

/// Provider of desktop entries.
#[derive(Debug, Clone)]
pub struct DesktopSource {
    id: SourceId,
}

impl DesktopSource {
    /// Creates the provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::DESKTOP),
        }
    }
}

impl Default for DesktopSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Applications directories in priority order, as host paths.
#[must_use]
pub fn application_dirs(env: &HostEnv) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let user_data = env
        .xdg_dir("XDG_DATA_HOME")
        .or_else(|| env.home().map(|home| home.join(USER_DATA_DIR)));
    if let Some(data) = user_data {
        dirs.push(data.join(APPLICATIONS));
    }
    let data_dirs = env.var_text("XDG_DATA_DIRS").unwrap_or(DEFAULT_DATA_DIRS);
    dirs.extend(
        env.split_paths(data_dirs)
            .into_iter()
            .filter(|dir| env.is_absolute(dir))
            .map(|dir| dir.join(APPLICATIONS)),
    );
    if let Some(home) = env.home() {
        dirs.push(home.join(USER_FLATPAK_DIR));
    }
    dirs.extend(EXTRA_DIRS.iter().map(PathBuf::from));
    let mut seen = HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

impl ProgramSource for DesktopSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::App
    }

    fn detect(&self, env: &HostEnv) -> bool {
        env.os() == nurt_contracts::env::Os::Linux
            && application_dirs(env).iter().any(|dir| is_dir(env, dir))
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder =
            FingerprintBuilder::new(env).text("languages", &env.languages().join(","));
        for dir in application_dirs(env) {
            builder = builder.path(dir);
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let mut data = SourceData::default();
        let mut seen: HashSet<String> = HashSet::new();
        for dir in application_dirs(env) {
            deadline.check(&self.id)?;
            let mut files = Vec::new();
            collect_files(env, &dir, "", 0, &mut files);
            for (desktop_id, path) in files {
                if !seen.insert(desktop_id.clone()) {
                    continue;
                }
                match read_prefix(env, &self.id, &path, MAX_ENTRY_BYTES) {
                    Ok((bytes, _)) => {
                        let entry = DesktopEntry::parse(&String::from_utf8_lossy(&bytes));
                        if let Some(program) = program_of(&entry, &desktop_id, &path, &self.id) {
                            data.programs.push(program);
                        }
                    }
                    Err(error) => data.warnings.push(error.into()),
                }
            }
        }
        Ok(data)
    }
}

/// Desktop files below `dir` with their desktop file ids (subdirectories
/// joined with `-`).
fn collect_files(
    env: &HostEnv,
    dir: &Path,
    prefix: &str,
    depth: usize,
    out: &mut Vec<(String, PathBuf)>,
) {
    for (name, path) in list_dir(env, dir) {
        if let Some(stem) = name.strip_suffix(DESKTOP_SUFFIX) {
            out.push((format!("{prefix}{stem}"), path));
        } else if depth < MAX_DEPTH && is_dir(env, &path) {
            collect_files(env, &path, &format!("{prefix}{name}-"), depth + 1, out);
        }
    }
}

/// Raw entry of a visible application; `None` for other entries.
#[must_use]
pub fn program_of(
    entry: &DesktopEntry,
    desktop_id: &str,
    path: &Path,
    id: &SourceId,
) -> Option<RawProgram> {
    if !entry.is_visible_application() {
        return None;
    }
    let app_name = entry.localized("Name")?;
    let command = entry
        .get("Exec")
        .and_then(exec_command)
        .or_else(|| entry.get("TryExec").map(str::to_owned));
    let command_name = command
        .as_deref()
        .map(|command| command.rsplit('/').next().unwrap_or(command).to_owned());
    let mut program = RawProgram::new(
        command_name
            .clone()
            .unwrap_or_else(|| app_name.text.clone()),
    );
    program.kind = EntryKind::App;
    program.key = Some(EntryKey::new(id, path.display().to_string()));
    program.app_id = Some(desktop_id.to_owned());
    program.summary = entry
        .localized("Comment")
        .or_else(|| entry.localized("GenericName"));
    program.path = command.filter(|command| command.starts_with('/'));
    program.app_name = Some(app_name);
    program.executable = command_name;
    program.category_hints = entry
        .list("Categories")
        .into_iter()
        .map(|value| CategoryHint::new(CategoryScheme::Freedesktop, value))
        .collect();
    if let Some(keywords) = entry.localized("Keywords") {
        for variant in keywords.variants() {
            program.tags.extend(
                variant
                    .split(';')
                    .map(str::trim)
                    .filter(|word| !word.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    Some(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Image Viewer\nName[pl]=Przegl\u{105}darka obraz\u{f3}w\nComment=View images\nComment[pl_PL]=Przegl\u{105}danie obraz\u{f3}w\nExec=env GDK_BACKEND=x11 \"/usr/bin/eog viewer\" %U\nCategories=Graphics;Viewer;\nKeywords=picture;photo;\n\n[Desktop Action new]\nName=Other\n";

    #[test]
    fn desktop_categories_map() {
        let entry = DesktopEntry::parse(ENTRY);
        let program = program_of(
            &entry,
            "org.example.Viewer",
            Path::new("/a.desktop"),
            &SourceId::new("desktop"),
        )
        .unwrap();
        assert_eq!(program.name, "eog viewer");
        assert_eq!(program.path.as_deref(), Some("/usr/bin/eog viewer"));
        assert_eq!(
            program
                .app_name
                .as_ref()
                .unwrap()
                .pick(&["pl".to_owned()])
                .0,
            "Przegl\u{105}darka obraz\u{f3}w"
        );
        assert_eq!(
            program.summary.as_ref().unwrap().pick(&["pl".to_owned()]).0,
            "Przegl\u{105}danie obraz\u{f3}w"
        );
        assert_eq!(program.tags, vec!["picture", "photo"]);
        let classified =
            crate::taxonomy::Taxonomy::builtin().classify(&program.name, &program.category_hints);
        assert_eq!(classified.id, "images");
    }

    #[test]
    fn hidden_and_non_applications_are_skipped() {
        let id = SourceId::new("desktop");
        for text in [
            "[Desktop Entry]\nType=Application\nName=A\nNoDisplay=true\n",
            "[Desktop Entry]\nType=Application\nName=A\nHidden=true\n",
            "[Desktop Entry]\nType=Link\nName=A\n",
        ] {
            assert!(program_of(&DesktopEntry::parse(text), "a", Path::new("/a"), &id).is_none());
        }
    }

    #[test]
    fn exec_parsing() {
        assert_eq!(exec_command("foo --bar %f").as_deref(), Some("foo"));
        assert_eq!(exec_command("env -u X A=1 prog").as_deref(), Some("prog"));
        assert_eq!(exec_command("%U"), None);
        assert_eq!(
            exec_command("/usr/bin/env A=1 prog %u").as_deref(),
            Some("prog")
        );
        assert_eq!(
            exec_command("/usr/bin/environment-tool").as_deref(),
            Some("/usr/bin/environment-tool")
        );
        assert_eq!(unescape("a\\sb\\\\c"), "a b\\c");
    }
}
