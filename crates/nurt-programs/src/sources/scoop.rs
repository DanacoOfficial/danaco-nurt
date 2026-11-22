// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Scoop applications and shims.
//!
//! The provider reads `apps/<app>/current/manifest.json` and
//! `install.json` of the user and global installations and maps the shims in
//! `shims` to their applications through the `path` line of each `.shim`
//! file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;
use serde_json::Value;

use crate::model::{
    AliasKind, CategoryHint, CategoryScheme, LocalizedText, PackageRef, RawPackage, RawProgram,
    SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError, SourceId,
    SourceKind, SourceWarning, WarningCode,
};

const MANIFEST: &str = "manifest.json";
const INSTALL: &str = "install.json";
const SHIM_EXTENSION: &str = ".shim";
const DEFAULT_PROGRAM_DATA: &str = "C:\\ProgramData";

/// Provider of Scoop applications.
#[derive(Debug, Clone)]
pub struct ScoopSource {
    id: SourceId,
}

impl Default for ScoopSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ScoopSource {
    /// Provider with the identifier `scoop`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::SCOOP),
        }
    }
}

/// Command declared by the `bin` member of a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinEntry {
    /// Name of the shim, the command typed by the user.
    pub command: String,
    /// File of the application the shim starts, relative to the
    /// application directory.
    pub target: String,
}

/// Fields of a Scoop manifest used by the provider.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    /// Version.
    pub version: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Project page.
    pub homepage: Option<String>,
    /// License identifier.
    pub license: Option<String>,
    /// Commands.
    pub bin: Vec<BinEntry>,
}

/// Reads a Scoop manifest. The member `bin` is a path, a list of paths or a
/// list of lists of a path, an alias and arguments. The command of a path is
/// its file name without extension.
///
/// # Errors
///
/// Returns a short reason when the text is not a JSON object.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::scoop::parse_manifest;
///
/// let manifest = parse_manifest(r#"{"version": "14.1", "bin": ["rg.exe", ["bin\\fd.exe", "find-fd"]]}"#).unwrap();
/// assert_eq!(manifest.bin[0].command, "rg");
/// assert_eq!(manifest.bin[1].command, "find-fd");
/// ```
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("manifest is not JSON: {error}"))?;
    if !value.is_object() {
        return Err("manifest is not a JSON object".to_owned());
    }
    let text_of = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let license = match value.get("license") {
        Some(Value::String(license)) => Some(license.clone()),
        Some(Value::Object(license)) => license
            .get("identifier")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    };
    let bin = match value.get("bin") {
        Some(Value::String(path)) => bin_entry(path, None).into_iter().collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(path) => bin_entry(path, None),
                Value::Array(pair) => {
                    bin_entry(pair.first()?.as_str()?, pair.get(1).and_then(Value::as_str))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(Manifest {
        version: text_of("version"),
        description: text_of("description"),
        homepage: text_of("homepage"),
        license,
        bin,
    })
}

fn bin_entry(path: &str, alias: Option<&str>) -> Option<BinEntry> {
    let command = match alias.filter(|alias| !alias.is_empty()) {
        Some(alias) => alias.to_owned(),
        None => file_stem(path)?,
    };
    Some(BinEntry {
        command,
        target: path.to_owned(),
    })
}

fn file_stem(path: &str) -> Option<String> {
    let file = path.rsplit(['\\', '/']).next()?;
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    (!stem.is_empty()).then(|| stem.to_owned())
}

/// Application started by a shim file. The application is the directory
/// name after `apps` in the value of the `path` line.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::scoop::shim_application;
///
/// let shim = "path = \"C:\\Users\\a\\scoop\\apps\\ripgrep\\current\\rg.exe\"\n";
/// assert_eq!(shim_application(shim).as_deref(), Some("ripgrep"));
/// ```
#[must_use]
pub fn shim_application(text: &str) -> Option<String> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("path"))?;
    let (_, value) = line.split_once('=')?;
    let path = value.trim().trim_matches('"');
    let parts: Vec<&str> = path.split(['\\', '/']).collect();
    let at = parts
        .iter()
        .position(|part| part.eq_ignore_ascii_case("apps"))?;
    parts.get(at + 1).map(|app| (*app).to_owned())
}

/// Installation directories of Scoop as host paths: `SCOOP` or `~\scoop`,
/// then `SCOOP_GLOBAL` or `%PROGRAMDATA%\scoop`.
#[must_use]
pub fn roots(env: &HostEnv) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(root) = env.var_text("SCOOP") {
        out.push(PathBuf::from(root));
    } else if let Some(home) = env.home() {
        out.push(home.join("scoop"));
    }
    match env.var_text("SCOOP_GLOBAL") {
        Some(global) => out.push(PathBuf::from(global)),
        None => {
            let data = env.var_text("PROGRAMDATA").unwrap_or(DEFAULT_PROGRAM_DATA);
            out.push(Path::new(data).join("scoop"));
        }
    }
    out
}

fn names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}

impl ScoopSource {
    fn read_root(
        &self,
        env: &HostEnv,
        root: &Path,
        deadline: &Deadline<'_>,
        data: &mut SourceData,
    ) -> Result<(), SourceError> {
        let shims = root.join("shims");
        let mut shims_by_app: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for file in names_in(&env.sys_path(&shims)) {
            let Some(command) = file.strip_suffix(SHIM_EXTENSION) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(env.sys_path(shims.join(&file))) else {
                continue;
            };
            if let Some(app) = shim_application(&text) {
                shims_by_app
                    .entry(app.to_ascii_lowercase())
                    .or_default()
                    .push(command.to_owned());
            }
        }
        let apps = root.join("apps");
        for app in names_in(&env.sys_path(&apps)) {
            deadline.check(&self.id)?;
            let current = apps.join(&app).join("current");
            let manifest_path = current.join(MANIFEST);
            let Ok(text) = std::fs::read_to_string(env.sys_path(&manifest_path)) else {
                continue;
            };
            let manifest = match parse_manifest(&text) {
                Ok(manifest) => manifest,
                Err(reason) => {
                    data.warnings.push(
                        SourceWarning::new(WarningCode::SourceUnavailable, &self.id, reason)
                            .at(manifest_path.display().to_string())
                            .named(app.clone()),
                    );
                    continue;
                }
            };
            let bucket = std::fs::read_to_string(env.sys_path(current.join(INSTALL)))
                .ok()
                .and_then(|text| {
                    serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok()
                })
                .and_then(|value| {
                    value
                        .get("bucket")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            let shim_commands = shims_by_app
                .remove(&app.to_ascii_lowercase())
                .unwrap_or_default();
            self.add_app(&shims, &app, manifest, bucket, &shim_commands, data);
        }
        Ok(())
    }

    fn add_app(
        &self,
        shims: &Path,
        app: &str,
        manifest: Manifest,
        bucket: Option<String>,
        shim_commands: &[String],
        data: &mut SourceData,
    ) {
        let package = PackageRef {
            manager: self.id.clone(),
            name: app.to_owned(),
        };
        let mut commands: Vec<(String, Option<String>)> = manifest
            .bin
            .iter()
            .map(|entry| {
                let stem = file_stem(&entry.target)
                    .filter(|stem| !stem.eq_ignore_ascii_case(&entry.command));
                (entry.command.clone(), stem)
            })
            .collect();
        for command in shim_commands {
            if !commands
                .iter()
                .any(|(known, _)| known.eq_ignore_ascii_case(command))
            {
                commands.push((command.clone(), None));
            }
        }
        let summary = manifest.description.as_deref().map(LocalizedText::new);
        let mut executables = Vec::new();
        for (command, target) in commands {
            let path = shims.join(format!("{command}.exe")).display().to_string();
            data.file_packages.push((path.clone(), package.clone()));
            executables.push(path.clone());
            data.programs.push(RawProgram {
                path: Some(path),
                alias_of: target,
                alias_kind: None,
                summary: summary.clone(),
                version: manifest.version.clone(),
                package: Some(package.clone()),
                homepage: manifest.homepage.clone(),
                license: manifest.license.clone(),
                category_hints: vec![CategoryHint::new(CategoryScheme::Keyword, app)],
                ..RawProgram::new(command)
            });
        }
        for program in data
            .programs
            .iter_mut()
            .filter(|program| program.alias_of.is_some())
        {
            program.alias_kind = Some(AliasKind::Shim);
        }
        data.packages.push(RawPackage {
            name: app.to_owned(),
            version: manifest.version,
            summary,
            homepage: manifest.homepage,
            license: manifest.license,
            origin: bucket,
            executables,
            ..RawPackage::default()
        });
    }
}

impl ProgramSource for ScoopSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Package
    }

    fn detect(&self, env: &HostEnv) -> bool {
        env.os() == Os::Windows
            && roots(env)
                .iter()
                .any(|root| env.sys_path(root.join("apps")).is_dir())
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder = FingerprintBuilder::new(env);
        for root in roots(env) {
            builder = builder.path(root.join("apps")).path(root.join("shims"));
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let mut data = SourceData::default();
        for root in roots(env) {
            self.read_root(env, &root, deadline, &mut data)?;
        }
        Ok(data)
    }
}
