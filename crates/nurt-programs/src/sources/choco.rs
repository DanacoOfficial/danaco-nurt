// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Chocolatey packages and shims.
//!
//! The provider reads the package descriptions `lib\<id>\<id>.nuspec` and
//! maps the shims in `bin` to packages by the executable files in the
//! `tools` directory of each package.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;
use quick_xml::Reader as XmlReader;
use quick_xml::events::Event;

use crate::model::{
    CategoryHint, CategoryScheme, LocalizedText, PackageRef, RawPackage, RawProgram, SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError, SourceId,
    SourceKind, SourceWarning, WarningCode,
};

/// Installation directory used when `ChocolateyInstall` is not set.
pub const DEFAULT_ROOT: &str = "C:\\ProgramData\\chocolatey";
const NUSPEC_EXTENSION: &str = ".nuspec";
const EXE_EXTENSION: &str = ".exe";
/// Depth of the search for executables in the `tools` directory of a
/// package.
const TOOLS_DEPTH: usize = 3;

/// Provider of Chocolatey packages.
#[derive(Debug, Clone)]
pub struct ChocoSource {
    id: SourceId,
}

impl Default for ChocoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ChocoSource {
    /// Provider with the identifier `choco`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::CHOCO),
        }
    }
}

/// Metadata of a `.nuspec` file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Nuspec {
    /// Package identifier.
    pub id: Option<String>,
    /// Version.
    pub version: Option<String>,
    /// Display title.
    pub title: Option<String>,
    /// One-line summary.
    pub summary: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Tags, separated by spaces in the file.
    pub tags: Vec<String>,
    /// Project page.
    pub project_url: Option<String>,
    /// License page.
    pub license_url: Option<String>,
}

/// Reads the `metadata` element of a `.nuspec` file; namespaces are
/// ignored.
///
/// # Errors
///
/// Returns a short reason when the text is not well-formed XML.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::choco::parse_nuspec;
///
/// let xml = r#"<package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
///   <metadata><id>git</id><version>2.47.0</version><summary>Version control</summary>
///   <tags>vcs git</tags></metadata></package>"#;
/// let nuspec = parse_nuspec(xml).unwrap();
/// assert_eq!(nuspec.id.as_deref(), Some("git"));
/// assert_eq!(nuspec.tags, ["vcs", "git"]);
/// ```
pub fn parse_nuspec(text: &str) -> Result<Nuspec, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = XmlReader::from_str(text);
    let mut path: Vec<String> = Vec::new();
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                path.push(String::from_utf8_lossy(element.local_name().as_ref()).into_owned());
            }
            Ok(Event::End(_)) => {
                path.pop();
            }
            Ok(Event::Text(content)) => {
                let in_metadata = path.len() == 3 && path.get(1).is_some_and(|name| name == "metadata");
                if let (true, Some(field)) = (in_metadata, path.last()) {
                    let decoded = content
                        .decode()
                        .map_err(|error| format!("nuspec text cannot be decoded: {error}"))?;
                    let text = quick_xml::escape::unescape(&decoded)
                        .map_err(|error| format!("nuspec text cannot be decoded: {error}"))?;
                    values.entry(field.clone()).or_default().push_str(&text);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("nuspec is not well-formed XML: {error}")),
        }
    }
    let take = |values: &mut BTreeMap<String, String>, key: &str| {
        values
            .remove(key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    Ok(Nuspec {
        id: take(&mut values, "id"),
        version: take(&mut values, "version"),
        title: take(&mut values, "title"),
        summary: take(&mut values, "summary"),
        description: take(&mut values, "description"),
        tags: take(&mut values, "tags")
            .map(|tags| tags.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        project_url: take(&mut values, "projectUrl"),
        license_url: take(&mut values, "licenseUrl"),
    })
}

/// Installation directory of Chocolatey as a host path.
#[must_use]
pub fn root(env: &HostEnv) -> PathBuf {
    PathBuf::from(env.var_text("ChocolateyInstall").unwrap_or(DEFAULT_ROOT))
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

/// Lower-case names of executables below `dir`, to [`TOOLS_DEPTH`] levels.
fn executables_below(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), 1usize)];
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < TOOLS_DEPTH {
                pending.push((path, depth + 1));
            } else if let Some(name) = entry.file_name().to_str() {
                let lower = name.to_ascii_lowercase();
                if lower.ends_with(EXE_EXTENSION) {
                    found.push(lower);
                }
            }
        }
    }
    found
}

impl ChocoSource {
    fn read_package(&self, env: &HostEnv, lib: &Path, id: &str, data: &mut SourceData) -> Option<(Nuspec, Vec<String>)> {
        let path = lib.join(id).join(format!("{id}{NUSPEC_EXTENSION}"));
        let text = std::fs::read_to_string(env.sys_path(&path)).ok()?;
        match parse_nuspec(&text) {
            Ok(nuspec) => {
                let tools = executables_below(&env.sys_path(lib.join(id).join("tools")));
                Some((nuspec, tools))
            }
            Err(reason) => {
                data.warnings.push(
                    SourceWarning::new(WarningCode::SourceUnavailable, &self.id, reason)
                        .at(path.display().to_string())
                        .named(id),
                );
                None
            }
        }
    }
}

impl ProgramSource for ChocoSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Package
    }

    fn detect(&self, env: &HostEnv) -> bool {
        env.os() == Os::Windows && env.sys_path(root(env).join("lib")).is_dir()
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let root = root(env);
        Ok(FingerprintBuilder::new(env)
            .path(root.join("lib"))
            .path(root.join("bin"))
            .finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let root = root(env);
        let lib = root.join("lib");
        let bin = root.join("bin");
        let mut data = SourceData::default();
        let mut packages: Vec<(String, Nuspec, Vec<String>)> = Vec::new();
        for id in names_in(&env.sys_path(&lib)) {
            deadline.check(&self.id)?;
            if let Some((nuspec, tools)) = self.read_package(env, &lib, &id, &mut data) {
                packages.push((id, nuspec, tools));
            }
        }
        let mut executables: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for shim in names_in(&env.sys_path(&bin)) {
            let lower = shim.to_ascii_lowercase();
            let Some(command) = shim.get(..shim.len().saturating_sub(EXE_EXTENSION.len())).filter(|_| lower.ends_with(EXE_EXTENSION)) else {
                continue;
            };
            let owner = packages
                .iter()
                .find(|(_, _, tools)| tools.contains(&lower))
                .or_else(|| packages.iter().find(|(id, _, _)| id.eq_ignore_ascii_case(command)));
            let Some((id, nuspec, _)) = owner else { continue };
            let path = bin.join(&shim).display().to_string();
            let package = PackageRef {
                manager: self.id.clone(),
                name: id.clone(),
            };
            data.file_packages.push((path.clone(), package.clone()));
            executables.entry(id.clone()).or_default().push(path.clone());
            let mut hints = vec![CategoryHint::new(CategoryScheme::Keyword, id.clone())];
            hints.extend(nuspec.tags.iter().map(|tag| CategoryHint::new(CategoryScheme::Keyword, tag.clone())));
            data.programs.push(RawProgram {
                path: Some(path),
                summary: nuspec.summary.as_deref().or(nuspec.title.as_deref()).map(LocalizedText::new),
                description: nuspec.description.as_deref().map(LocalizedText::new),
                version: nuspec.version.clone(),
                package: Some(package),
                homepage: nuspec.project_url.clone(),
                tags: nuspec.tags.clone(),
                category_hints: hints,
                ..RawProgram::new(command)
            });
        }
        for (id, nuspec, _) in packages {
            data.packages.push(RawPackage {
                executables: executables.remove(&id).unwrap_or_default(),
                name: nuspec.id.clone().unwrap_or(id),
                version: nuspec.version,
                summary: nuspec.summary.or(nuspec.title).as_deref().map(LocalizedText::new),
                description: nuspec.description.as_deref().map(LocalizedText::new),
                homepage: nuspec.project_url,
                license: nuspec.license_url,
                groups: nuspec.tags,
                ..RawPackage::default()
            });
        }
        Ok(data)
    }
}
