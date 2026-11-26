// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Homebrew formulae and casks.
//!
//! The provider reads the installation receipts in `Cellar`, the descriptive
//! lines of the formula files kept next to them, the cask metadata in
//! `Caskroom`, the links in `bin` and the API cache of Homebrew. Formula
//! files are Ruby programs: only the lines `desc`, `homepage` and `license`
//! with a quoted text are matched, nothing is evaluated. The signature of the
//! API cache is checked for its structure only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;
use serde_json::Value;

use crate::model::{
    CategoryHint, CategoryScheme, EntryKind, LocalizedText, PackageRef, RawPackage, RawProgram,
    SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError, SourceId,
    SourceKind, SourceWarning, WarningCode,
};
use crate::sources::{MAX_METADATA_BYTES, read_prefix};

/// Prefixes searched when `HOMEBREW_PREFIX` is not set, in order.
pub const DEFAULT_PREFIXES: [&str; 3] =
    ["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"];

const RECEIPT: &str = "INSTALL_RECEIPT.json";
const FORMULA_API: &str = "formula.jws.json";
const CASK_API: &str = "cask.jws.json";
/// Directory where casks install applications.
const APPLICATIONS: &str = "/Applications";
const APP_SUFFIX: &str = ".app";
/// Largest receipt, formula or cask file read, in bytes.
const MAX_SMALL_FILE_BYTES: u64 = 1024 * 1024;

/// Provider of Homebrew formulae and casks.
#[derive(Debug, Clone)]
pub struct BrewSource {
    id: SourceId,
}

impl Default for BrewSource {
    fn default() -> Self {
        Self::new()
    }
}

impl BrewSource {
    /// Provider with the identifier `brew`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::BREW),
        }
    }
}

/// Descriptive fields of a formula or cask.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormulaInfo {
    /// One-line description.
    pub desc: Option<String>,
    /// Project page.
    pub homepage: Option<String>,
    /// License, when written as one text.
    pub license: Option<String>,
    /// Stable version.
    pub version: Option<String>,
    /// Display names of a cask.
    pub names: Vec<String>,
}

/// Prefixes of Homebrew installations as host paths: `HOMEBREW_PREFIX`
/// first, then [`DEFAULT_PREFIXES`] that apply to the operating system.
#[must_use]
pub fn prefixes(env: &HostEnv) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(prefix) = env.var_text("HOMEBREW_PREFIX") {
        out.push(PathBuf::from(prefix));
    }
    let defaults: &[&str] = match env.os() {
        Os::Macos => &DEFAULT_PREFIXES[..2],
        Os::Linux => &DEFAULT_PREFIXES[2..],
        _ => &[],
    };
    for prefix in defaults {
        let prefix = PathBuf::from(prefix);
        if !out.contains(&prefix) {
            out.push(prefix);
        }
    }
    out
}

/// Cache directory of Homebrew as a host path: `HOMEBREW_CACHE`, otherwise
/// `~/Library/Caches/Homebrew` on macOS and `$XDG_CACHE_HOME/Homebrew` or
/// `~/.cache/Homebrew` elsewhere.
#[must_use]
pub fn cache_dir(env: &HostEnv) -> Option<PathBuf> {
    if let Some(cache) = env.var_text("HOMEBREW_CACHE") {
        return Some(PathBuf::from(cache));
    }
    if env.os() != Os::Macos
        && let Some(xdg) = env.xdg_dir("XDG_CACHE_HOME")
    {
        return Some(xdg.join("Homebrew"));
    }
    let home = env.home()?;
    Some(match env.os() {
        Os::Macos => home.join("Library").join("Caches").join("Homebrew"),
        _ => home.join(".cache").join("Homebrew"),
    })
}

/// Reads `desc`, `homepage` and `license` from the text of a formula or cask
/// file by matching lines of the form `desc "…"`; the file is not evaluated.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::brew::parse_formula_rb;
///
/// let info = parse_formula_rb("class Jq < Formula\n  desc \"Lightweight JSON processor\"\n  license \"MIT\"\nend\n");
/// assert_eq!(info.desc.as_deref(), Some("Lightweight JSON processor"));
/// assert_eq!(info.license.as_deref(), Some("MIT"));
/// ```
#[must_use]
pub fn parse_formula_rb(text: &str) -> FormulaInfo {
    let mut info = FormulaInfo::default();
    for line in text.lines().map(str::trim) {
        let slot = if line.starts_with("desc ") {
            &mut info.desc
        } else if line.starts_with("homepage ") {
            &mut info.homepage
        } else if line.starts_with("license ") {
            &mut info.license
        } else {
            continue;
        };
        if slot.is_none() {
            *slot = quoted_argument(line);
        }
    }
    info
}

/// Text of the single double-quoted argument of a line such as
/// `desc "Text"`; `None` for any other form, such as an interpolated or
/// computed value.
fn quoted_argument(line: &str) -> Option<String> {
    let (_, rest) = line.split_once(' ')?;
    let inner = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains("#{") || inner.contains('"') {
        return None;
    }
    Some(inner.replace("\\\\", "\\"))
}

/// Reads the entries of the API cache file `*.jws.json`: a JSON Web
/// Signature whose `payload` is a JSON text with an array of formulae or
/// casks. Only the structure of the signature is checked.
///
/// # Errors
///
/// Returns a short reason when the file is not a JWS document or the payload
/// is not an array.
pub fn parse_api_cache(text: &str) -> Result<BTreeMap<String, FormulaInfo>, String> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| format!("api cache is not JSON: {error}"))?;
    let signed = document
        .get("signatures")
        .and_then(Value::as_array)
        .is_some_and(|signatures| {
            !signatures.is_empty()
                && signatures
                    .iter()
                    .all(|signature| signature.get("signature").is_some_and(Value::is_string))
        });
    if !signed {
        return Err("api cache has no signatures".to_owned());
    }
    let payload = document
        .get("payload")
        .and_then(Value::as_str)
        .ok_or_else(|| "api cache has no payload".to_owned())?;
    let entries: Vec<Value> = serde_json::from_str(payload)
        .map_err(|error| format!("api cache payload is not an array: {error}"))?;
    let mut out = BTreeMap::new();
    for entry in entries {
        let token = entry.get("token").and_then(Value::as_str);
        let Some(name) = token.or_else(|| entry.get("name").and_then(Value::as_str)) else {
            continue;
        };
        out.insert(name.to_owned(), info_from_json(&entry));
    }
    Ok(out)
}

fn info_from_json(entry: &Value) -> FormulaInfo {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
    let version = entry
        .get("versions")
        .and_then(|versions| versions.get("stable"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| text("version"));
    let names = match entry.get("name") {
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    FormulaInfo {
        desc: text("desc"),
        homepage: text("homepage"),
        license: text("license"),
        version,
        names,
    }
}

/// Installation receipt of a formula version.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Receipt {
    /// Installed on request, not as a dependency.
    pub on_request: Option<bool>,
    /// Install time, RFC 3339 UTC.
    pub installed_at: Option<String>,
    /// Stable version of the formula source.
    pub stable: Option<String>,
    /// Tap the formula comes from.
    pub tap: Option<String>,
}

/// Reads an `INSTALL_RECEIPT.json`.
///
/// # Errors
///
/// Returns a short reason when the text is not a JSON object.
pub fn parse_receipt(text: &str) -> Result<Receipt, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("receipt is not JSON: {error}"))?;
    if !value.is_object() {
        return Err("receipt is not a JSON object".to_owned());
    }
    let source = value.get("source");
    Ok(Receipt {
        on_request: value.get("installed_on_request").and_then(Value::as_bool),
        installed_at: value
            .get("time")
            .and_then(Value::as_i64)
            .map(super::rfc3339),
        stable: source
            .and_then(|source| source.get("versions"))
            .and_then(|versions| versions.get("stable"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        tap: source
            .and_then(|source| source.get("tap"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Orders version texts by their numeric parts, then as text.
fn version_order(left: &str, right: &str) -> std::cmp::Ordering {
    let parts = |text: &str| -> Vec<u64> {
        text.split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse().unwrap_or(u64::MAX))
            .collect()
    };
    parts(left).cmp(&parts(right)).then_with(|| left.cmp(right))
}

fn dir_names(path: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// State of the reading of one prefix.
struct Reader<'a> {
    env: &'a HostEnv,
    id: &'a SourceId,
    api: Option<BTreeMap<String, FormulaInfo>>,
    casks_api: Option<BTreeMap<String, FormulaInfo>>,
    data: SourceData,
}

impl Reader<'_> {
    fn formula_api(&mut self) -> &BTreeMap<String, FormulaInfo> {
        if self.api.is_none() {
            self.api = Some(self.read_api(FORMULA_API));
        }
        self.api.get_or_insert_with(BTreeMap::new)
    }

    fn cask_api(&mut self) -> &BTreeMap<String, FormulaInfo> {
        if self.casks_api.is_none() {
            self.casks_api = Some(self.read_api(CASK_API));
        }
        self.casks_api.get_or_insert_with(BTreeMap::new)
    }

    fn read_api(&mut self, file: &str) -> BTreeMap<String, FormulaInfo> {
        let Some(path) = cache_dir(self.env).map(|dir| dir.join("api").join(file)) else {
            return BTreeMap::new();
        };
        let Some(text) = self.read_text(&path, MAX_METADATA_BYTES) else {
            return BTreeMap::new();
        };
        match parse_api_cache(&text) {
            Ok(entries) => entries,
            Err(reason) => {
                self.data.warnings.push(
                    SourceWarning::new(WarningCode::SourceUnavailable, self.id, reason)
                        .at(path.display().to_string()),
                );
                BTreeMap::new()
            }
        }
    }

    /// Text of a file given as a host path; `None` when the file cannot be
    /// read, is larger than `limit` bytes or is not UTF-8.
    fn read_text(&self, host: &Path, limit: u64) -> Option<String> {
        let (bytes, complete) = read_prefix(self.env, self.id, host, limit).ok()?;
        if complete {
            String::from_utf8(bytes).ok()
        } else {
            None
        }
    }

    /// Version of `formula` that `<prefix>/opt/<formula>` links to, when it is
    /// one of `versions`.
    fn linked_version(&self, cellar: &Path, formula: &str, versions: &[String]) -> Option<String> {
        let opt = cellar.parent()?.join("opt").join(formula);
        let target = std::fs::read_link(self.env.sys_path(&opt)).ok()?;
        let name = target.file_name()?.to_str()?;
        versions.iter().find(|version| *version == name).cloned()
    }

    fn read_prefix(&mut self, prefix: &Path, deadline: &Deadline<'_>) -> Result<(), SourceError> {
        let links = self.bin_links(prefix);
        let cellar = prefix.join("Cellar");
        for formula in dir_names(&self.env.sys_path(&cellar)) {
            deadline.check(self.id)?;
            self.read_formula(
                &cellar,
                &formula,
                links.get(&formula).cloned().unwrap_or_default(),
            );
        }
        let caskroom = prefix.join("Caskroom");
        for cask in dir_names(&self.env.sys_path(&caskroom)) {
            deadline.check(self.id)?;
            self.read_cask(&caskroom.join(&cask), &cask);
        }
        Ok(())
    }

    /// Links in `<prefix>/bin` by formula: host path of the link and
    /// command name.
    fn bin_links(&self, prefix: &Path) -> BTreeMap<String, Vec<(String, String)>> {
        let bin = prefix.join("bin");
        let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(self.env.sys_path(&bin)) else {
            return out;
        };
        for entry in entries.flatten() {
            let Ok(target) = std::fs::read_link(entry.path()) else {
                continue;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Some(formula) = formula_of_link(&target) {
                let host = bin.join(&name).display().to_string();
                out.entry(formula).or_default().push((host, name));
            }
        }
        for commands in out.values_mut() {
            commands.sort();
        }
        out
    }

    fn read_formula(&mut self, cellar: &Path, formula: &str, commands: Vec<(String, String)>) {
        let dir = cellar.join(formula);
        let versions = dir_names(&self.env.sys_path(&dir));
        let Some(version) = self.linked_version(cellar, formula, &versions).or_else(|| {
            versions
                .iter()
                .max_by(|left, right| version_order(left, right))
                .cloned()
        }) else {
            return;
        };
        let version_dir = dir.join(&version);
        let receipt_path = version_dir.join(RECEIPT);
        let receipt = match self.read_text(&receipt_path, MAX_SMALL_FILE_BYTES) {
            Some(text) => match parse_receipt(&text) {
                Ok(receipt) => receipt,
                Err(reason) => {
                    self.data.warnings.push(
                        SourceWarning::new(WarningCode::SourceUnavailable, self.id, reason)
                            .at(receipt_path.display().to_string())
                            .named(formula),
                    );
                    Receipt::default()
                }
            },
            None => Receipt::default(),
        };
        let rb = version_dir.join(".brew").join(format!("{formula}.rb"));
        let mut info = self
            .read_text(&rb, MAX_SMALL_FILE_BYTES)
            .map(|text| parse_formula_rb(&text))
            .unwrap_or_default();
        if info.desc.is_none()
            && let Some(api) = self.formula_api().get(formula).cloned()
        {
            info.desc = api.desc;
            info.homepage = info.homepage.or(api.homepage);
            info.license = info.license.or(api.license);
        }
        let package = PackageRef {
            manager: self.id.clone(),
            name: formula.to_owned(),
        };
        for (path, name) in &commands {
            self.data
                .file_packages
                .push((path.clone(), package.clone()));
            self.data.programs.push(RawProgram {
                path: Some(path.clone()),
                summary: info.desc.as_deref().map(LocalizedText::new),
                version: Some(version.clone()),
                package: Some(package.clone()),
                homepage: info.homepage.clone(),
                license: info.license.clone(),
                category_hints: vec![CategoryHint::new(CategoryScheme::Keyword, formula)],
                ..RawProgram::new(name.clone())
            });
        }
        self.data.packages.push(RawPackage {
            name: formula.to_owned(),
            version: Some(version),
            summary: info.desc.as_deref().map(LocalizedText::new),
            homepage: info.homepage,
            license: info.license,
            origin: receipt.tap,
            installed_at: receipt.installed_at,
            explicit: receipt.on_request,
            executables: commands.into_iter().map(|(path, _)| path).collect(),
            ..RawPackage::default()
        });
    }

    fn read_cask(&mut self, dir: &Path, token: &str) {
        let metadata = dir.join(".metadata");
        let versions = dir_names(&self.env.sys_path(&metadata));
        let Some(version) = versions
            .iter()
            .max_by(|left, right| version_order(left, right))
            .cloned()
        else {
            return;
        };
        let mut info = FormulaInfo {
            version: Some(version.clone()),
            ..FormulaInfo::default()
        };
        let mut apps = Vec::new();
        let stamps = dir_names(&self.env.sys_path(metadata.join(&version)));
        if let Some(stamp) = stamps.last() {
            let casks = metadata.join(&version).join(stamp).join("Casks");
            let json = casks.join(format!("{token}.json"));
            let rb = casks.join(format!("{token}.rb"));
            if let Some(text) = self.read_text(&json, MAX_SMALL_FILE_BYTES) {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    info = FormulaInfo {
                        version: info.version,
                        ..info_from_json(&value)
                    };
                    apps = cask_apps(&value);
                }
            } else if let Some(text) = self.read_text(&rb, MAX_SMALL_FILE_BYTES) {
                info = FormulaInfo {
                    version: info.version,
                    ..parse_formula_rb(&text)
                };
            }
        }
        if info.desc.is_none()
            && let Some(api) = self.cask_api().get(token).cloned()
        {
            info.desc = api.desc;
            info.homepage = info.homepage.or(api.homepage);
            if info.names.is_empty() {
                info.names = api.names;
            }
        }
        let package = PackageRef {
            manager: self.id.clone(),
            name: token.to_owned(),
        };
        for app in &apps {
            let name = app.strip_suffix(APP_SUFFIX).unwrap_or(app);
            self.data.programs.push(RawProgram {
                kind: EntryKind::App,
                path: Some(format!("{APPLICATIONS}/{app}")),
                app_name: info
                    .names
                    .first()
                    .map(|name| LocalizedText::new(name.as_str())),
                summary: info.desc.as_deref().map(LocalizedText::new),
                version: info.version.clone(),
                package: Some(package.clone()),
                homepage: info.homepage.clone(),
                category_hints: vec![CategoryHint::new(CategoryScheme::Keyword, token)],
                ..RawProgram::new(name)
            });
        }
        self.data.packages.push(RawPackage {
            name: token.to_owned(),
            version: info.version,
            summary: info.desc.as_deref().map(LocalizedText::new),
            homepage: info.homepage,
            groups: vec!["cask".to_owned()],
            ..RawPackage::default()
        });
    }
}

/// Formula of a link target such as `../Cellar/jq/1.7.1/bin/jq`.
fn formula_of_link(target: &Path) -> Option<String> {
    let parts: Vec<String> = target
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let at = parts.iter().position(|part| part == "Cellar")?;
    parts.get(at + 1).cloned()
}

/// Application bundles named by the `artifacts` of a cask.
fn cask_apps(cask: &Value) -> Vec<String> {
    let Some(artifacts) = cask.get("artifacts").and_then(Value::as_array) else {
        return Vec::new();
    };
    artifacts
        .iter()
        .filter_map(|artifact| artifact.get("app").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .filter(|app| app.ends_with(APP_SUFFIX))
        .map(|app| app.rsplit('/').next().unwrap_or(app).to_owned())
        .collect()
}

impl ProgramSource for BrewSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Package
    }

    fn detect(&self, env: &HostEnv) -> bool {
        prefixes(env).iter().any(|prefix| {
            env.sys_path(prefix.join("Cellar")).is_dir()
                || env.sys_path(prefix.join("Caskroom")).is_dir()
        })
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder = FingerprintBuilder::new(env);
        for prefix in prefixes(env) {
            builder = builder
                .path(prefix.join("Cellar"))
                .path(prefix.join("Caskroom"))
                .path(prefix.join("bin"));
        }
        if let Some(cache) = cache_dir(env) {
            builder = builder
                .path(cache.join("api").join(FORMULA_API))
                .path(cache.join("api").join(CASK_API));
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let mut reader = Reader {
            env,
            id: &self.id,
            api: None,
            casks_api: None,
            data: SourceData::default(),
        };
        for prefix in prefixes(env) {
            reader.read_prefix(&prefix, deadline)?;
        }
        Ok(reader.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_numerically() {
        assert_eq!(version_order("1.10", "1.9"), std::cmp::Ordering::Greater);
        assert_eq!(version_order("2.0_1", "2.0"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn interpolated_desc_is_ignored() {
        assert_eq!(parse_formula_rb("desc \"Tool #{version}\"").desc, None);
    }

    #[test]
    fn link_target_names_the_formula() {
        assert_eq!(
            formula_of_link(Path::new("../Cellar/jq/1.7/bin/jq")).as_deref(),
            Some("jq")
        );
        assert_eq!(formula_of_link(Path::new("/usr/bin/jq")), None);
    }
}
