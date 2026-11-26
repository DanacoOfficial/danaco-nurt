// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface of program information providers and the host environment they read.
//!
//! A provider ([`ProgramSource`]) is a pure reader of files and, on Windows,
//! of the registry. It never starts a program. Everything that depends on the
//! computer is reached through [`HostEnv`]: the file system root, environment
//! variables, the home directory, preferred languages and the registry. Tests
//! build a [`HostEnv`] over a temporary directory, so every parser runs on
//! every operating system.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nurt_contracts::config::{CatalogSource, ProgramsConfig};
use nurt_contracts::env::{Clock, Os};
use nurt_contracts::stage::ExecCtx;
use sha2::{Digest, Sha256};

use crate::model::{Detail, DetailWant, EntryKey, SourceData};

/// Stable identifier of a provider, for example `dpkg` or `catalog:media`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    /// Programs found in `PATH`.
    pub const PATH: &'static str = "path";
    /// Debian package database.
    pub const DPKG: &'static str = "dpkg";
    /// RPM package database.
    pub const RPM: &'static str = "rpm";
    /// Arch Linux package database.
    pub const PACMAN: &'static str = "pacman";
    /// Desktop entries of Linux applications.
    pub const DESKTOP: &'static str = "desktop";
    /// Manual pages and `whatis` databases.
    pub const MAN: &'static str = "man";
    /// Output of `--help`, read on request.
    pub const HELP: &'static str = "help";
    /// Homebrew.
    pub const BREW: &'static str = "brew";
    /// Windows Package Manager.
    pub const WINGET: &'static str = "winget";
    /// Scoop.
    pub const SCOOP: &'static str = "scoop";
    /// Chocolatey.
    pub const CHOCO: &'static str = "choco";
    /// Applications registered with Windows.
    pub const WINAPP: &'static str = "winapp";
    /// macOS application bundles.
    pub const MACAPP: &'static str = "macapp";
    /// Prefix of external catalog providers, followed by the catalog id.
    pub const CATALOG_PREFIX: &'static str = "catalog:";

    /// Creates an identifier from its text.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Identifier of the external catalog `id`.
    #[must_use]
    pub fn catalog(id: &str) -> Self {
        Self(format!("{}{id}", Self::CATALOG_PREFIX))
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Catalog id when the provider is an external catalog.
    #[must_use]
    pub fn catalog_id(&self) -> Option<&str> {
        self.0.strip_prefix(Self::CATALOG_PREFIX)
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SourceId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

/// Kind of a provider, used by the `source:` filter of the commands.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Executable files in `PATH`.
    Path,
    /// Package databases.
    Package,
    /// Applications registered with the operating system.
    App,
    /// Descriptions only (manual pages); never creates a card alone.
    Description,
    /// External program catalogs.
    Catalog,
}

impl SourceKind {
    /// Name used in records and in the `source:` filter.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Package => "package",
            Self::App => "app",
            Self::Description => "man",
            Self::Catalog => "catalog",
        }
    }
}

/// Error of a provider. The index turns it into a warning; it never fails a
/// command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{source_id}: {message}")]
pub struct SourceError {
    /// Provider that failed.
    pub source_id: SourceId,
    /// Category of the failure.
    pub kind: SourceErrorKind,
    /// File or registry key concerned.
    pub path: Option<String>,
    /// Short description, lower case, without a final period.
    pub message: String,
}

impl SourceError {
    /// Creates an error of `kind`.
    #[must_use]
    pub fn new(source_id: &SourceId, kind: SourceErrorKind, message: impl Into<String>) -> Self {
        Self {
            source_id: source_id.clone(),
            kind,
            path: None,
            message: message.into(),
        }
    }

    /// Sets the file or key concerned.
    #[must_use]
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Error for a failed read of `path`.
    #[must_use]
    pub fn io(source_id: &SourceId, path: &Path, error: &std::io::Error) -> Self {
        let kind = match error.kind() {
            std::io::ErrorKind::PermissionDenied => SourceErrorKind::Permission,
            _ => SourceErrorKind::Unavailable,
        };
        Self::new(source_id, kind, error.to_string()).at(path.display().to_string())
    }
}

/// Category of a [`SourceError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceErrorKind {
    /// The data cannot be read now (missing, locked).
    Unavailable,
    /// The account may not read the data.
    Permission,
    /// The data is damaged or has an unknown format.
    Corrupt,
    /// The deadline passed before the provider finished.
    Timeout,
}

/// Code of a warning attached to the result of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    /// A provider could not be read; the result has the data of the others.
    SourceUnavailable,
    /// A provider did not finish within the index budget.
    IndexPartial,
    /// A catalog was skipped as a whole.
    CatalogInvalid,
    /// A catalog has a format version this reader does not know.
    CatalogFormat,
    /// One entry or field of a catalog was skipped.
    CatalogEntryInvalid,
    /// A catalog object has a field outside the format.
    CatalogUnknownField,
    /// A guide has no `version` in its front matter.
    GuideVersionMissing,
    /// `source: catalog` was given and no catalog is configured.
    NoCatalogs,
    /// The help text of a program was not read.
    HelpRefused,
    /// Further warnings of the same kind were left out.
    WarningsTruncated,
}

impl WarningCode {
    /// Stable name of the code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "source_unavailable",
            Self::IndexPartial => "index_partial",
            Self::CatalogInvalid => "catalog_invalid",
            Self::CatalogFormat => "catalog_format",
            Self::CatalogEntryInvalid => "catalog_entry_invalid",
            Self::CatalogUnknownField => "catalog_unknown_field",
            Self::GuideVersionMissing => "guide_version_missing",
            Self::NoCatalogs => "no_catalogs",
            Self::HelpRefused => "help_refused",
            Self::WarningsTruncated => "warnings_truncated",
        }
    }
}

/// Warning of a provider: data was skipped, the rest of the result is valid.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceWarning {
    /// Code.
    pub code: WarningCode,
    /// Provider that reported the warning.
    pub source: SourceId,
    /// File, key or entry concerned.
    pub path: Option<String>,
    /// Entry name, when the warning concerns one entry.
    pub name: Option<String>,
    /// Line in the file, when known.
    pub line: Option<u64>,
    /// Short reason, lower case, without a final period.
    pub reason: String,
}

impl SourceWarning {
    /// Creates a warning.
    #[must_use]
    pub fn new(code: WarningCode, source: &SourceId, reason: impl Into<String>) -> Self {
        Self {
            code,
            source: source.clone(),
            path: None,
            name: None,
            line: None,
            reason: reason.into(),
        }
    }

    /// Sets the file or key concerned.
    #[must_use]
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Sets the entry concerned.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the line in the file.
    #[must_use]
    pub fn line(mut self, line: u64) -> Self {
        self.line = Some(line);
        self
    }
}

impl From<SourceError> for SourceWarning {
    fn from(error: SourceError) -> Self {
        let code = if error.kind == SourceErrorKind::Timeout {
            WarningCode::IndexPartial
        } else {
            WarningCode::SourceUnavailable
        };
        Self {
            code,
            source: error.source_id,
            path: error.path,
            name: None,
            line: None,
            reason: error.message,
        }
    }
}

/// Point in time after which a provider stops reading and reports
/// [`SourceErrorKind::Timeout`].
#[derive(Clone, Copy)]
pub struct Deadline<'a> {
    clock: Option<&'a dyn Clock>,
    at: Duration,
}

impl<'a> Deadline<'a> {
    /// A deadline that never passes.
    #[must_use]
    pub fn none() -> Self {
        Self {
            clock: None,
            at: Duration::MAX,
        }
    }

    /// A deadline `budget` after the current monotonic time of `clock`.
    #[must_use]
    pub fn after(clock: &'a dyn Clock, budget: Duration) -> Self {
        Self {
            clock: Some(clock),
            at: clock.monotonic().saturating_add(budget),
        }
    }

    /// Returns `true` when the deadline has passed.
    #[must_use]
    pub fn expired(&self) -> bool {
        self.clock.is_some_and(|clock| clock.monotonic() >= self.at)
    }

    /// Fails with [`SourceErrorKind::Timeout`] when the deadline has passed.
    ///
    /// # Errors
    ///
    /// Returns a timeout error of `source` after the deadline.
    pub fn check(&self, source: &SourceId) -> Result<(), SourceError> {
        if self.expired() {
            Err(SourceError::new(
                source,
                SourceErrorKind::Timeout,
                "index budget exceeded",
            ))
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for Deadline<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Deadline")
            .field("bounded", &self.clock.is_some())
            .field("at", &self.at)
            .finish()
    }
}

/// Root key of the Windows registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RegistryHive {
    /// `HKEY_LOCAL_MACHINE`.
    LocalMachine,
    /// `HKEY_CURRENT_USER`.
    CurrentUser,
}

/// Registry view of a 64-bit Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RegistryView {
    /// The view of the native architecture.
    Native,
    /// The 32-bit view (`WOW6432Node`).
    Wow32,
}

/// Value of a registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryValue {
    /// `REG_SZ`.
    String(String),
    /// `REG_EXPAND_SZ`, not expanded.
    ExpandString(String),
    /// `REG_MULTI_SZ`.
    MultiString(Vec<String>),
    /// `REG_DWORD`.
    Dword(u32),
    /// `REG_QWORD`.
    Qword(u64),
    /// `REG_BINARY` and other types.
    Binary(Vec<u8>),
}

impl RegistryValue {
    /// Text of a string value.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::String(text) | Self::ExpandString(text) => Some(text),
            _ => None,
        }
    }

    /// Number of a numeric value.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Dword(number) => Some(u64::from(*number)),
            Self::Qword(number) => Some(*number),
            _ => None,
        }
    }
}

/// Read access to the Windows registry.
///
/// Key paths are relative to the hive and use `\` as separator, for example
/// `SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`. The operating
/// system implementation may be registered by the embedding application in
/// [`ExecCtx::extensions`] as `dyn RegistryReader`; on Windows
/// [`HostEnv::from_ctx`] otherwise reads the registry of the running system.
/// [`MemoryRegistry`] serves tests on every system. Key names compare
/// without regard to case and keep the case they were created with.
pub trait RegistryReader: Send + Sync {
    /// Names of the subkeys of a key in the case they were created with; the
    /// result is `None` when the key does not exist.
    ///
    /// # Errors
    ///
    /// Fails when the key exists and cannot be read.
    fn subkeys(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<Vec<String>>, SourceError>;

    /// All values of a key, by name; `None` when the key does not exist. The
    /// default value has the empty name.
    ///
    /// # Errors
    ///
    /// Fails when the key exists and cannot be read.
    fn values(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<BTreeMap<String, RegistryValue>>, SourceError>;

    /// Last write time of a key, for fingerprints; `None` when the key does
    /// not exist.
    ///
    /// # Errors
    ///
    /// Fails when the key exists and cannot be read.
    fn last_write(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<SystemTime>, SourceError>;
}

/// Registry held in memory, for tests and for hosts without a registry.
#[derive(Debug, Clone, Default)]
pub struct MemoryRegistry {
    keys: BTreeMap<(RegistryHive, RegistryView, String), MemoryKey>,
}

#[derive(Debug, Clone, Default)]
struct MemoryKey {
    name: String,
    values: BTreeMap<String, RegistryValue>,
    written: Option<SystemTime>,
}

impl MemoryRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a key and its parents.
    ///
    /// A key that exists under a name differing only in case is reused and
    /// keeps the case it was created with.
    pub fn insert_key(&mut self, hive: RegistryHive, view: RegistryView, key: &str) {
        let mut path = String::new();
        for part in key.split('\\').filter(|part| !part.is_empty()) {
            if !path.is_empty() {
                path.push('\\');
            }
            path.push_str(part);
            self.keys
                .entry((hive, view, path.to_ascii_lowercase()))
                .or_insert_with(|| MemoryKey {
                    name: part.to_owned(),
                    ..MemoryKey::default()
                });
        }
    }

    /// Sets a value, creating the key when needed.
    pub fn insert_value(
        &mut self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
        name: &str,
        value: RegistryValue,
    ) {
        self.insert_key(hive, view, key);
        if let Some(entry) = self.keys.get_mut(&(hive, view, normalize_key(key))) {
            entry.values.insert(name.to_owned(), value);
        }
    }

    /// Sets the last write time of a key, creating the key when needed.
    pub fn set_last_write(
        &mut self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
        time: SystemTime,
    ) {
        self.insert_key(hive, view, key);
        if let Some(entry) = self.keys.get_mut(&(hive, view, normalize_key(key))) {
            entry.written = Some(time);
        }
    }

    fn key(&self, hive: RegistryHive, view: RegistryView, key: &str) -> Option<&MemoryKey> {
        self.keys.get(&(hive, view, normalize_key(key)))
    }
}

fn normalize_key(key: &str) -> String {
    key.split('\\')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\\")
        .to_ascii_lowercase()
}

/// Registry of the running system, used when the context registers none.
#[cfg(windows)]
fn system_registry() -> Option<Arc<dyn RegistryReader>> {
    Some(Arc::new(crate::win::SystemRegistry::new()))
}

/// Registry of the running system, used when the context registers none.
#[cfg(not(windows))]
fn system_registry() -> Option<Arc<dyn RegistryReader>> {
    None
}

impl RegistryReader for MemoryRegistry {
    fn subkeys(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<Vec<String>>, SourceError> {
        if self.key(hive, view, key).is_none() {
            return Ok(None);
        }
        let parent = normalize_key(key);
        let prefix = format!("{parent}\\");
        let names = self
            .keys
            .iter()
            .filter(|((h, v, path), _)| *h == hive && *v == view && path.starts_with(&prefix))
            .filter(|((_, _, path), _)| !path[prefix.len()..].contains('\\'))
            .map(|(_, entry)| entry.name.clone())
            .collect();
        Ok(Some(names))
    }

    fn values(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<BTreeMap<String, RegistryValue>>, SourceError> {
        Ok(self.key(hive, view, key).map(|entry| entry.values.clone()))
    }

    fn last_write(
        &self,
        hive: RegistryHive,
        view: RegistryView,
        key: &str,
    ) -> Result<Option<SystemTime>, SourceError> {
        Ok(self
            .key(hive, view, key)
            .map(|entry| entry.written.unwrap_or(UNIX_EPOCH)))
    }
}

/// Everything a provider may read about the computer.
///
/// System paths are written as on the host (`/var/lib/dpkg/status`,
/// `C:\ProgramData\chocolatey`) and mapped by [`HostEnv::sys_path`]. In
/// production the root is the real file system; tests set a root directory
/// and every system path is resolved below it.
#[derive(Clone)]
pub struct HostEnv {
    os: Os,
    root: Option<PathBuf>,
    vars: BTreeMap<String, OsString>,
    home: Option<PathBuf>,
    languages: Vec<String>,
    registry: Option<Arc<dyn RegistryReader>>,
}

/// Default `PATHEXT` of Windows, used when the variable is not set.
pub const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

impl HostEnv {
    /// Environment of operating system `os` over the real file system,
    /// without variables, home directory or registry.
    #[must_use]
    pub fn new(os: Os) -> Self {
        Self {
            os,
            root: None,
            vars: BTreeMap::new(),
            home: None,
            languages: vec!["en".to_owned()],
            registry: None,
        }
    }

    /// Environment of the running stage: operating system, variables and
    /// home directory of the host, preferred languages from the locale and
    /// the registry reader registered in the context. Without a registered
    /// reader the environment reads the registry of the running system on
    /// Windows and has no registry elsewhere.
    #[must_use]
    pub fn from_ctx(ctx: &ExecCtx) -> Self {
        let info = ctx.host().info();
        let mut env = Self::new(info.os).with_home(info.home.clone());
        for (name, value) in ctx.host().env_vars() {
            if let Some(name) = name.to_str() {
                env = env.with_var(name, value);
            }
        }
        if let Some(language) = info.locale.as_deref().and_then(locale_language) {
            env.languages.insert(0, language);
            env.languages.dedup();
        }
        env.registry = ctx
            .extensions()
            .get::<dyn RegistryReader>()
            .or_else(system_registry);
        env
    }

    /// Resolves every system path below `root`.
    #[must_use]
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Sets an environment variable.
    #[must_use]
    pub fn with_var(mut self, name: &str, value: impl Into<OsString>) -> Self {
        self.vars.insert(self.var_key(name), value.into());
        self
    }

    /// Sets the home directory of the account, as a host path.
    #[must_use]
    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.home = Some(home.into());
        self
    }

    /// Sets the preferred languages of descriptions, most preferred first.
    #[must_use]
    pub fn with_languages(mut self, languages: Vec<String>) -> Self {
        self.languages = languages;
        self
    }

    /// Sets the registry reader.
    #[must_use]
    pub fn with_registry(mut self, registry: Arc<dyn RegistryReader>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Operating system whose conventions the providers follow.
    #[must_use]
    pub fn os(&self) -> Os {
        self.os
    }

    /// Root below which system paths are resolved; `None` for the real file
    /// system.
    #[must_use]
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Value of an environment variable, compared without case on Windows.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&OsStr> {
        self.vars.get(&self.var_key(name)).map(OsString::as_os_str)
    }

    /// Value of an environment variable as text; `None` when absent, empty
    /// or not valid Unicode.
    #[must_use]
    pub fn var_text(&self, name: &str) -> Option<&str> {
        self.var(name)
            .and_then(OsStr::to_str)
            .filter(|value| !value.is_empty())
    }

    /// Home directory of the account, as a host path.
    #[must_use]
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Preferred languages of descriptions, most preferred first.
    #[must_use]
    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    /// Registry reader, on Windows hosts and in tests that set one.
    #[must_use]
    pub fn registry(&self) -> Option<&dyn RegistryReader> {
        self.registry.as_deref()
    }

    /// Separator of lists of paths in variables such as `PATH`.
    #[must_use]
    pub fn list_separator(&self) -> char {
        if self.os == Os::Windows { ';' } else { ':' }
    }

    /// Splits a list of paths written in a variable; empty items are
    /// dropped.
    #[must_use]
    pub fn split_paths(&self, list: &str) -> Vec<PathBuf> {
        list.split(self.list_separator())
            .map(|item| item.trim_matches('"'))
            .filter(|item| !item.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    /// Directories of `PATH` in resolution order, as host paths. Relative
    /// entries are dropped: they depend on the working directory, and
    /// Windows never searches the current directory for this purpose. A
    /// directory whose name is not valid Unicode is kept with replacement
    /// characters, so that it does not hide the other directories.
    #[must_use]
    pub fn path_dirs(&self) -> Vec<PathBuf> {
        let list = self.var("PATH").map(OsStr::to_string_lossy);
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut keys: Vec<String> = Vec::new();
        for dir in self.split_paths(list.as_deref().unwrap_or_default()) {
            if !self.is_absolute(&dir) {
                continue;
            }
            let key = self.name_key(dir.to_string_lossy().trim_end_matches(['/', '\\']));
            if !keys.contains(&key) {
                keys.push(key);
                dirs.push(dir);
            }
        }
        dirs
    }

    /// Executable extensions of Windows in upper case with the leading dot,
    /// from `PATHEXT` or [`DEFAULT_PATHEXT`]; empty on other systems.
    #[must_use]
    pub fn path_extensions(&self) -> Vec<String> {
        if self.os != Os::Windows {
            return Vec::new();
        }
        let list = self.var_text("PATHEXT").unwrap_or(DEFAULT_PATHEXT);
        let mut extensions: Vec<String> = Vec::new();
        for item in list.split(';').map(str::trim).filter(|item| item.len() > 1) {
            let mut extension = item.to_ascii_uppercase();
            if !extension.starts_with('.') {
                extension.insert(0, '.');
            }
            if !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
        extensions
    }

    /// Name of a program file without a `PATHEXT` extension on Windows;
    /// `None` when a Windows file has no executable extension. On other
    /// systems the name is returned unchanged.
    #[must_use]
    pub fn command_name(&self, file_name: &str) -> Option<String> {
        if self.os != Os::Windows {
            return Some(file_name.to_owned());
        }
        let upper = file_name.to_ascii_uppercase();
        self.path_extensions()
            .iter()
            .find(|extension| upper.ends_with(extension.as_str()) && upper.len() > extension.len())
            .map(|extension| file_name[..file_name.len() - extension.len()].to_owned())
    }

    /// Key under which program names are compared: lower case on Windows
    /// and macOS, whose file systems ignore case, exact on Linux.
    #[must_use]
    pub fn name_key(&self, name: &str) -> String {
        if self.os == Os::Linux {
            name.to_owned()
        } else {
            name.to_lowercase()
        }
    }

    /// Returns `true` when `path` is absolute in the notation of the host.
    #[must_use]
    pub fn is_absolute(&self, path: &Path) -> bool {
        if self.os == Os::Windows {
            let text = path.to_string_lossy();
            let bytes = text.as_bytes();
            let drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && (bytes[2] == b'\\' || bytes[2] == b'/');
            drive || text.starts_with("\\\\")
        } else {
            path.has_root()
        }
    }

    /// Path of the file system that holds the host path `path`.
    ///
    /// Without a root the path is returned unchanged. With a root, the drive
    /// letter of a Windows path becomes a lower-case directory, separators
    /// `\` and `/` are both accepted, and `..` cannot leave the root.
    #[must_use]
    pub fn sys_path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        let Some(root) = &self.root else {
            return path.to_path_buf();
        };
        let mut out = root.clone();
        let text = path.to_string_lossy();
        let mut parts: Vec<&str> = Vec::new();
        for (index, part) in text.split(['/', '\\']).enumerate() {
            if part.is_empty() || part == "." {
                continue;
            }
            if index == 0 && part.len() == 2 && part.ends_with(':') {
                parts.push(&part[..1]);
                continue;
            }
            if part == ".." {
                parts.pop();
                continue;
            }
            parts.push(part);
        }
        for (index, part) in parts.iter().enumerate() {
            if index == 0 && part.len() == 1 && text.as_bytes().get(1) == Some(&b':') {
                out.push(part.to_ascii_lowercase());
            } else {
                out.push(part);
            }
        }
        out
    }

    /// Host path of a file of the file system: the inverse of
    /// [`sys_path`](Self::sys_path) for paths below the root.
    #[must_use]
    pub fn host_path(&self, path: &Path) -> PathBuf {
        let Some(root) = &self.root else {
            return path.to_path_buf();
        };
        let Ok(rest) = path.strip_prefix(root) else {
            return path.to_path_buf();
        };
        let mut out = PathBuf::from(if self.os == Os::Windows { "" } else { "/" });
        let mut first = true;
        for component in rest.components() {
            if let Component::Normal(part) = component {
                if first && self.os == Os::Windows {
                    let drive = part.to_string_lossy().to_ascii_uppercase();
                    out = PathBuf::from(format!("{drive}:\\"));
                } else {
                    out.push(part);
                }
                first = false;
            }
        }
        out
    }

    fn var_key(&self, name: &str) -> String {
        if self.os == Os::Windows {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
    }
}

impl fmt::Debug for HostEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostEnv")
            .field("os", &self.os)
            .field("root", &self.root)
            .field("vars", &self.vars.len())
            .field("home", &self.home)
            .field("languages", &self.languages)
            .field("registry", &self.registry.is_some())
            .finish()
    }
}

/// Language code of a locale such as `pl_PL.UTF-8`; `None` for `C` and
/// `POSIX`.
#[must_use]
pub fn locale_language(locale: &str) -> Option<String> {
    let language = locale
        .split(['_', '.', '@', '-'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let valid =
        (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase());
    valid.then_some(language)
}

/// Marker of the state of the data a provider reads. Equal fingerprints mean
/// the cached data of the provider is still valid.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// The fingerprint as hexadecimal text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Builder of a [`Fingerprint`] from texts, files and registry keys.
pub struct FingerprintBuilder<'a> {
    env: &'a HostEnv,
    hasher: Sha256,
}

impl<'a> FingerprintBuilder<'a> {
    /// Starts a fingerprint of data read through `env`.
    #[must_use]
    pub fn new(env: &'a HostEnv) -> Self {
        Self {
            env,
            hasher: Sha256::new(),
        }
    }

    /// Adds a text, for example the value of a variable.
    #[must_use]
    pub fn text(mut self, label: &str, value: &str) -> Self {
        self.item(label.as_bytes());
        self.item(value.as_bytes());
        self
    }

    /// Adds the existence, size and modification time of a file or
    /// directory given as a host path. A missing path is part of the state.
    #[must_use]
    pub fn path(mut self, path: impl AsRef<Path>) -> Self {
        let host = path.as_ref();
        let file = self.env.sys_path(host);
        self.item(host.to_string_lossy().as_bytes());
        match std::fs::metadata(&file) {
            Ok(meta) => {
                self.item(&meta.len().to_le_bytes());
                self.item(&nanos(meta.modified().ok()).to_le_bytes());
            }
            Err(_) => self.item(b"absent"),
        }
        self
    }

    /// Adds the last write time of a registry key; nothing without a
    /// registry.
    #[must_use]
    pub fn registry_key(mut self, hive: RegistryHive, view: RegistryView, key: &str) -> Self {
        self.item(format!("{hive:?}\\{view:?}\\{key}").as_bytes());
        let written = self
            .env
            .registry()
            .and_then(|registry| registry.last_write(hive, view, key).ok().flatten());
        match written {
            Some(time) => self.item(&nanos(Some(time)).to_le_bytes()),
            None => self.item(b"absent"),
        }
        self
    }

    /// Finishes the fingerprint.
    #[must_use]
    pub fn finish(self) -> Fingerprint {
        Fingerprint(hex::encode(self.hasher.finalize()))
    }

    fn item(&mut self, bytes: &[u8]) {
        self.hasher.update((bytes.len() as u64).to_le_bytes());
        self.hasher.update(bytes);
    }
}

impl fmt::Debug for FingerprintBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FingerprintBuilder").finish_non_exhaustive()
    }
}

fn nanos(time: Option<SystemTime>) -> u128 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos())
}

/// A provider of program information.
///
/// Providers are pure readers: they read files and registry keys through
/// [`HostEnv`] and never start a program. Every method is cheap to call
/// repeatedly; [`load`](ProgramSource::load) does the full read and is
/// called only when the [`fingerprint`](ProgramSource::fingerprint) changed.
pub trait ProgramSource: Send + Sync {
    /// Stable identifier, for example `dpkg` or `catalog:media`.
    fn id(&self) -> &SourceId;

    /// Kind used by the `source:` filter.
    fn kind(&self) -> SourceKind;

    /// Order among providers when they describe the same program; a higher
    /// value wins. Built-in providers return `0`; catalogs return their
    /// configured priority.
    fn priority(&self) -> i32 {
        0
    }

    /// Cheap presence test: the files or keys the provider reads exist.
    /// Never fails.
    fn detect(&self, env: &HostEnv) -> bool;

    /// Marker of the state of the data the provider reads.
    ///
    /// # Errors
    ///
    /// Fails when the state cannot be determined; the index then reloads the
    /// provider.
    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError>;

    /// Reads all entries of the provider.
    ///
    /// Damaged entries are skipped with a warning in
    /// [`SourceData::warnings`]; the provider checks `deadline` in long
    /// loops.
    ///
    /// # Errors
    ///
    /// Fails when the data cannot be read at all, and with
    /// [`SourceErrorKind::Timeout`] after the deadline.
    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError>;

    /// Reads details of one entry that are not kept in the index: full
    /// description, guide text, manual page. Returns `None` when the
    /// provider has no such details.
    ///
    /// # Errors
    ///
    /// Fails when the details exist and cannot be read.
    fn detail(
        &self,
        _env: &HostEnv,
        _key: &EntryKey,
        _want: &DetailWant,
    ) -> Result<Option<Detail>, SourceError> {
        Ok(None)
    }
}

/// External catalog to read, from `programs.catalogs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSpec {
    /// Catalog id used in `source: catalog:<id>`; the last component of the
    /// path unless the catalog declares its own.
    pub id: String,
    /// Directory of the catalog, or its `catalog.json`, as a host path.
    pub path: PathBuf,
    /// Priority; a higher value wins.
    pub priority: i32,
    /// Position in `programs.catalogs`; earlier wins at equal priority.
    pub position: usize,
}

impl CatalogSpec {
    /// Enabled catalogs of the configuration, in configuration order, with
    /// unique ids (`media`, `media-2`, ...).
    #[must_use]
    pub fn from_config(config: &ProgramsConfig) -> Vec<Self> {
        let mut specs: Vec<Self> = Vec::new();
        for (position, source) in config.catalogs.iter().enumerate() {
            let (path, priority) = match source {
                CatalogSource::Path(path) => (path.as_str(), 0),
                CatalogSource::Entry(entry) if entry.enabled => {
                    (entry.path.as_str(), entry.priority)
                }
                CatalogSource::Entry(_) => continue,
            };
            let base = catalog_id_from_path(path);
            let mut id = base.clone();
            let mut suffix = 2;
            while specs.iter().any(|spec| spec.id == id) {
                id = format!("{base}-{suffix}");
                suffix += 1;
            }
            specs.push(Self {
                id,
                path: PathBuf::from(path),
                priority,
                position,
            });
        }
        specs
    }
}

/// Catalog id derived from the last meaningful component of a path: lower
/// case, characters outside `[a-z0-9._-]` replaced by `-`.
#[must_use]
pub fn catalog_id_from_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let mut parts = trimmed.rsplit(['/', '\\']);
    let mut last = parts.next().unwrap_or_default();
    if last.eq_ignore_ascii_case("catalog.json") {
        last = parts.next().unwrap_or_default();
    }
    let id: String = last
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let id = id.trim_matches(['-', '.', '_']).to_owned();
    if id.is_empty() {
        "catalog".to_owned()
    } else {
        id
    }
}

/// Ordered set of providers used by the index.
#[derive(Clone, Default)]
pub struct SourceRegistry {
    sources: Vec<Arc<dyn ProgramSource>>,
}

impl SourceRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a provider; a provider with the same id replaces the earlier one.
    pub fn add(&mut self, source: Arc<dyn ProgramSource>) {
        if let Some(slot) = self
            .sources
            .iter_mut()
            .find(|existing| existing.id() == source.id())
        {
            *slot = source;
        } else {
            self.sources.push(source);
        }
    }

    /// Adds a provider and returns the registry.
    #[must_use]
    pub fn with(mut self, source: Arc<dyn ProgramSource>) -> Self {
        self.add(source);
        self
    }

    /// Providers in registration order.
    #[must_use]
    pub fn sources(&self) -> &[Arc<dyn ProgramSource>] {
        &self.sources
    }

    /// Provider with the id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Arc<dyn ProgramSource>> {
        self.sources
            .iter()
            .find(|source| source.id().as_str() == id)
    }

    /// Number of providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// Returns `true` when no provider is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

impl fmt::Debug for SourceRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.sources.iter().map(|source| source.id().as_str()))
            .finish()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    #[test]
    fn path_dirs_keep_other_directories_when_one_name_is_not_unicode() {
        let list = OsString::from_vec(b"/usr/bin:/opt/\xff:/bin".to_vec());
        let env = HostEnv::new(Os::Linux).with_var("PATH", list);
        let dirs = env.path_dirs();
        assert!(dirs.contains(&PathBuf::from("/usr/bin")));
        assert!(dirs.contains(&PathBuf::from("/bin")));
        assert_eq!(dirs.len(), 3);
    }

    #[test]
    fn path_extensions_come_from_pathext_in_upper_case() {
        let defaults = HostEnv::new(Os::Windows);
        assert_eq!(defaults.path_extensions(), [".COM", ".EXE", ".BAT", ".CMD"]);
        let custom = HostEnv::new(Os::Windows).with_var("PATHEXT", "exe;.Cmd; ;.;.exe");
        assert_eq!(custom.path_extensions(), [".EXE", ".CMD"]);
        assert!(HostEnv::new(Os::Linux).path_extensions().is_empty());
    }

    #[test]
    fn command_name_strips_only_executable_extensions_on_windows() {
        let windows = HostEnv::new(Os::Windows);
        assert_eq!(windows.command_name("git.exe").as_deref(), Some("git"));
        assert_eq!(windows.command_name("Git.EXE").as_deref(), Some("Git"));
        assert_eq!(windows.command_name("a.exe.bat").as_deref(), Some("a.exe"));
        assert_eq!(windows.command_name(".exe"), None);
        assert_eq!(windows.command_name("readme.txt"), None);
        let linux = HostEnv::new(Os::Linux);
        assert_eq!(linux.command_name("readme.txt").as_deref(), Some("readme.txt"));
    }

    #[test]
    fn sys_path_stays_below_the_root_and_maps_drive_letters() {
        let linux = HostEnv::new(Os::Linux).with_root("/r");
        assert_eq!(linux.sys_path("/usr/bin/../lib"), PathBuf::from("/r/usr/lib"));
        assert_eq!(linux.sys_path("/../../etc"), PathBuf::from("/r/etc"));
        assert_eq!(linux.host_path(Path::new("/r/usr/lib")), PathBuf::from("/usr/lib"));
        let windows = HostEnv::new(Os::Windows).with_root("/r");
        assert_eq!(windows.sys_path("C:\\Tools\\x.exe"), PathBuf::from("/r/c/Tools/x.exe"));
        assert_eq!(windows.sys_path("d:/data"), PathBuf::from("/r/d/data"));
        assert_eq!(HostEnv::new(Os::Linux).sys_path("/usr/bin"), PathBuf::from("/usr/bin"));
    }

    #[test]
    fn absolute_paths_follow_the_notation_of_the_host() {
        let windows = HostEnv::new(Os::Windows);
        assert!(windows.is_absolute(Path::new("C:\\Tools")));
        assert!(windows.is_absolute(Path::new("c:/tools")));
        assert!(windows.is_absolute(Path::new("\\\\server\\share")));
        assert!(!windows.is_absolute(Path::new("C:tools")));
        assert!(!windows.is_absolute(Path::new("\\tools")));
        assert!(!windows.is_absolute(Path::new("tools\\bin")));
        let linux = HostEnv::new(Os::Linux);
        assert!(linux.is_absolute(Path::new("/usr/bin")));
        assert!(!linux.is_absolute(Path::new("bin")));
    }

    #[test]
    fn locale_language_accepts_two_or_three_letters_only() {
        assert_eq!(locale_language("pl_PL.UTF-8").as_deref(), Some("pl"));
        assert_eq!(locale_language("zh-Hans").as_deref(), Some("zh"));
        assert_eq!(locale_language("fil").as_deref(), Some("fil"));
        assert_eq!(locale_language("C.UTF-8"), None);
        assert_eq!(locale_language("POSIX"), None);
        assert_eq!(locale_language(""), None);
    }

    #[test]
    fn catalog_ids_are_derived_from_the_last_path_component() {
        assert_eq!(catalog_id_from_path("/opt/Media Tools/"), "media-tools");
        assert_eq!(catalog_id_from_path("/opt/media/catalog.json"), "media");
        assert_eq!(catalog_id_from_path("C:\\cats\\Media_1"), "media_1");
        assert_eq!(catalog_id_from_path("catalog.json"), "catalog");
        assert_eq!(catalog_id_from_path("///"), "catalog");
        assert_eq!(catalog_id_from_path("/x/-.-"), "catalog");
        assert_eq!(catalog_id_from_path("/x/\u{142}\u{f3}d\u{17a}"), "d");
    }

    #[test]
    fn path_dirs_of_windows_ignore_case_and_trailing_separator() {
        let list = "C:\\Windows\\System32;c:\\WINDOWS\\system32\\;;relative\\bin;D:\\Tools";
        let env = HostEnv::new(Os::Windows).with_var("Path", list);
        assert_eq!(
            env.path_dirs(),
            vec![
                PathBuf::from("C:\\Windows\\System32"),
                PathBuf::from("D:\\Tools")
            ]
        );
        let linux = HostEnv::new(Os::Linux).with_var("PATH", "/usr/bin:/usr/bin/:/USR/bin");
        assert_eq!(
            linux.path_dirs(),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/USR/bin")]
        );
    }
}
