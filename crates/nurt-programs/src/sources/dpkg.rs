// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Debian package database.
//!
//! Installed packages are read from the stanzas of `/var/lib/dpkg/status`
//! and of `/var/lib/dpkg/status.d/*` (minimal images). A package counts as
//! installed when the third word of its `Status` field is `installed`. The
//! executables of a package come from `info/<package>.list` or
//! `info/<package>:<arch>.list`, limited to the binary directories.
//! Translated descriptions come from the `Translation-<lang>` files of APT
//! lists in the preferred languages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::{
    CategoryHint, CategoryScheme, EntryKey, LocalizedText, PackageRef, RawPackage, RawProgram,
    SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError,
    SourceErrorKind, SourceId, SourceKind, SourceWarning,
};
use crate::sources::{Compression, decompress, exists, in_bin_dir, list_dir, read_file};

/// Database directory of dpkg.
pub const DPKG_DIR: &str = "/var/lib/dpkg";
/// Directory of APT package lists with translations.
pub const APT_LISTS_DIR: &str = "/var/lib/apt/lists";

const STATUS_FILE: &str = "status";
const STATUS_DIR: &str = "status.d";
const INFO_DIR: &str = "info";
const LIST_SUFFIX: &str = ".list";
const TRANSLATION_MARK: &str = "_i18n_Translation-";
const KIB: u64 = 1024;
/// Largest decompressed translation file.
const MAX_TRANSLATION_BYTES: u64 = 256 * 1024 * 1024;

/// One stanza of a control file: fields in order, continuation lines joined
/// with `\n`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stanza {
    fields: Vec<(String, String)>,
    /// Line of the first field, from 1.
    pub line: u64,
}

impl Stanza {
    /// Value of a field, compared without case.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Fields in order.
    #[must_use]
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }
}

/// Parses control file stanzas (RFC 822 style): `Field: value` lines,
/// continuation lines starting with a space or tab, a lone ` .` as an empty
/// line, stanzas separated by empty lines. Lines without a colon outside a
/// continuation are skipped.
#[must_use]
pub fn parse_stanzas(text: &str) -> Vec<Stanza> {
    let mut stanzas = Vec::new();
    let mut current = Stanza::default();
    for (index, line) in text.lines().enumerate() {
        let number = index as u64 + 1;
        if line.trim().is_empty() {
            if !current.fields.is_empty() {
                stanzas.push(std::mem::take(&mut current));
            }
            continue;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = current.fields.last_mut() {
                let rest = line[1..].trim_end();
                value.push('\n');
                if rest != "." {
                    value.push_str(rest);
                }
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            if current.fields.is_empty() {
                current.line = number;
            }
            current
                .fields
                .push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    if !current.fields.is_empty() {
        stanzas.push(current);
    }
    stanzas
}

/// Returns `true` when a `Status` value says the package is installed.
#[must_use]
pub fn is_installed(status: &str) -> bool {
    status.split_whitespace().nth(2) == Some("installed")
}

/// Package of an installed stanza; `None` when the stanza has no name or
/// the package is not installed.
#[must_use]
pub fn package_from_stanza(stanza: &Stanza) -> Option<RawPackage> {
    let name = stanza.get("Package")?.to_owned();
    if !stanza.get("Status").is_some_and(is_installed) {
        return None;
    }
    let (summary, description) = split_description(stanza.get("Description").unwrap_or_default());
    Some(RawPackage {
        name,
        version: stanza.get("Version").map(str::to_owned),
        architecture: stanza.get("Architecture").map(str::to_owned),
        summary: summary.map(|text| LocalizedText::in_lang(text, "en")),
        description: description.map(|text| LocalizedText::in_lang(text, "en")),
        section: stanza.get("Section").map(str::to_owned),
        homepage: stanza.get("Homepage").map(str::to_owned),
        origin: stanza
            .get("Source")
            .and_then(|source| source.split_whitespace().next())
            .map(str::to_owned),
        installed_size: stanza
            .get("Installed-Size")
            .and_then(|size| size.trim().parse::<u64>().ok())
            .map(|kib| kib.saturating_mul(KIB)),
        ..RawPackage::default()
    })
}

/// First line of a description as summary, the rest as long description.
fn split_description(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.splitn(2, '\n');
    let summary = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned);
    let long = lines
        .next()
        .map(|rest| rest.trim_matches('\n').to_owned())
        .filter(|rest| !rest.trim().is_empty());
    (summary, long)
}

/// Provider of the Debian package database.
#[derive(Debug, Clone)]
pub struct DpkgSource {
    id: SourceId,
}

impl DpkgSource {
    /// Creates the provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::DPKG),
        }
    }
}

impl Default for DpkgSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgramSource for DpkgSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Package
    }

    fn detect(&self, env: &HostEnv) -> bool {
        let dir = Path::new(DPKG_DIR);
        exists(env, &dir.join(STATUS_FILE)) || exists(env, &dir.join(STATUS_DIR))
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let dir = Path::new(DPKG_DIR);
        let mut builder = FingerprintBuilder::new(env)
            .path(dir.join(STATUS_FILE))
            .path(dir.join(STATUS_DIR))
            .path(dir.join(INFO_DIR))
            .text("languages", &env.languages().join(","));
        for (_, path) in translation_files(env) {
            builder = builder.path(path);
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        read_with_deadline(env, deadline, &self.id)
    }
}

/// Reads the installed packages, their executables and the programs they
/// provide.
///
/// # Errors
///
/// Fails when neither `status` nor `status.d` can be read.
pub fn read_packages(env: &HostEnv) -> Result<SourceData, SourceError> {
    read_with_deadline(env, &Deadline::none(), &SourceId::new(SourceId::DPKG))
}

fn read_with_deadline(
    env: &HostEnv,
    deadline: &Deadline<'_>,
    id: &SourceId,
) -> Result<SourceData, SourceError> {
    let mut data = SourceData::default();
    let stanzas = read_status(env, id, &mut data.warnings)?;
    let translations = read_translations(env, id, &mut data.warnings);
    for stanza in &stanzas {
        let Some(mut package) = package_from_stanza(stanza) else {
            continue;
        };
        apply_translations(&mut package, &translations);
        deadline.check(id)?;
        package.executables = executables(env, &package);
        let reference = PackageRef {
            manager: id.clone(),
            name: package.name.clone(),
        };
        for file in &package.executables {
            data.file_packages.push((file.clone(), reference.clone()));
            data.programs.push(program_of(file, &package, &reference, id));
        }
        data.packages.push(package);
    }
    Ok(data)
}

fn read_status(
    env: &HostEnv,
    id: &SourceId,
    warnings: &mut Vec<SourceWarning>,
) -> Result<Vec<Stanza>, SourceError> {
    let dir = Path::new(DPKG_DIR);
    let mut files: Vec<PathBuf> = Vec::new();
    if exists(env, &dir.join(STATUS_FILE)) {
        files.push(dir.join(STATUS_FILE));
    }
    files.extend(list_dir(env, &dir.join(STATUS_DIR)).into_iter().map(|(_, path)| path));
    if files.is_empty() {
        return Err(SourceError::new(id, SourceErrorKind::Unavailable, "no status file")
            .at(format!("{DPKG_DIR}/{STATUS_FILE}")));
    }
    let mut stanzas = Vec::new();
    let mut read_any = false;
    for file in files {
        match read_file(env, id, &file) {
            Ok(bytes) => {
                read_any = true;
                stanzas.extend(parse_stanzas(&String::from_utf8_lossy(&bytes)));
            }
            Err(error) => warnings.push(error.into()),
        }
    }
    if read_any {
        Ok(stanzas)
    } else {
        Err(SourceError::new(id, SourceErrorKind::Unavailable, "status files cannot be read"))
    }
}

/// Executables of a package in binary directories, from its file list.
fn executables(env: &HostEnv, package: &RawPackage) -> Vec<String> {
    let info = Path::new(DPKG_DIR).join(INFO_DIR);
    let mut names = vec![format!("{}{LIST_SUFFIX}", package.name)];
    if let Some(arch) = &package.architecture {
        names.push(format!("{}:{arch}{LIST_SUFFIX}", package.name));
    }
    let id = SourceId::new(SourceId::DPKG);
    for name in names {
        if let Ok(bytes) = read_file(env, &id, &info.join(name)) {
            return list_executables(&String::from_utf8_lossy(&bytes));
        }
    }
    Vec::new()
}

/// Lines of a file list that name files directly in a binary directory.
#[must_use]
pub fn list_executables(list: &str) -> Vec<String> {
    list.lines()
        .map(str::trim)
        .filter(|line| in_bin_dir(line))
        .map(str::to_owned)
        .collect()
}

fn program_of(file: &str, package: &RawPackage, reference: &PackageRef, id: &SourceId) -> RawProgram {
    let name = file.rsplit('/').next().unwrap_or(file).to_owned();
    let mut program = RawProgram::new(name);
    program.key = Some(EntryKey::new(id, package.name.clone()));
    program.path = Some(file.to_owned());
    program.package = Some(reference.clone());
    program.version.clone_from(&package.version);
    program.homepage.clone_from(&package.homepage);
    if let Some(section) = &package.section {
        program
            .category_hints
            .push(CategoryHint::new(CategoryScheme::DebianSection, section.clone()));
    }
    program
}

/// Translation files of the preferred languages other than English.
fn translation_files(env: &HostEnv) -> Vec<(String, PathBuf)> {
    let wanted: Vec<&String> = env.languages().iter().filter(|lang| *lang != "en").collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    list_dir(env, Path::new(APT_LISTS_DIR))
        .into_iter()
        .filter_map(|(name, path)| {
            let (_, stem) = Compression::of(&name);
            let lang = stem.split(TRANSLATION_MARK).nth(1)?;
            let lang = lang.split(['_', '.', '-']).next().unwrap_or(lang).to_owned();
            wanted.contains(&&lang).then_some((lang, path))
        })
        .collect()
}

/// Translated descriptions by package name and language.
fn read_translations(
    env: &HostEnv,
    id: &SourceId,
    warnings: &mut Vec<SourceWarning>,
) -> HashMap<String, Vec<(String, String)>> {
    let mut out: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (lang, path) in translation_files(env) {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let (compression, _) = Compression::of(name);
        let text = read_file(env, id, &path).and_then(|bytes| {
            decompress(&bytes, compression, MAX_TRANSLATION_BYTES).map_err(|error| {
                SourceError::new(id, SourceErrorKind::Corrupt, error.to_string())
                    .at(path.display().to_string())
            })
        });
        let text = match text {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(error) => {
                warnings.push(SourceWarning::from(error));
                continue;
            }
        };
        let field = format!("Description-{lang}");
        for stanza in parse_stanzas(&text) {
            if let (Some(package), Some(description)) = (stanza.get("Package"), stanza.get(&field)) {
                out.entry(package.to_owned())
                    .or_default()
                    .push((lang.clone(), description.to_owned()));
            }
        }
    }
    out
}

fn apply_translations(package: &mut RawPackage, translations: &HashMap<String, Vec<(String, String)>>) {
    let Some(items) = translations.get(&package.name) else {
        return;
    };
    for (lang, text) in items {
        let (summary, description) = split_description(text);
        if let (Some(summary), Some(target)) = (summary, package.summary.as_mut()) {
            target.set(lang, summary);
        }
        if let (Some(description), Some(target)) = (description, package.description.as_mut()) {
            target.set(lang, description);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Package: coreutils\n\
Status: install ok installed\n\
Installed-Size: 10\n\
Architecture: amd64\n\
Version: 9.4-3\n\
Section: utils\n\
Description: GNU core utilities\n This package contains the basic file utilities.\n .\n Second paragraph.\n\
Homepage: https://example.org/coreutils\n\
\n\
Package: removed\n\
Status: deinstall ok config-files\n\
Description: gone\n";

    #[test]
    fn dpkg_status_parse_multiline() {
        let stanzas = parse_stanzas(STATUS);
        assert_eq!(stanzas.len(), 2);
        assert_eq!(stanzas[1].line, 12);
        let package = package_from_stanza(&stanzas[0]).unwrap();
        assert_eq!(package.name, "coreutils");
        assert_eq!(package.installed_size, Some(10 * KIB));
        assert_eq!(package.summary.unwrap().text, "GNU core utilities");
        assert_eq!(
            package.description.unwrap().text,
            "This package contains the basic file utilities.\n\nSecond paragraph."
        );
        assert!(package_from_stanza(&stanzas[1]).is_none());
    }

    #[test]
    fn status_values() {
        assert!(is_installed("install ok installed"));
        assert!(is_installed("hold ok installed"));
        assert!(!is_installed("install ok half-configured"));
        assert!(!is_installed(""));
    }

    #[test]
    fn list_keeps_only_binary_directories() {
        let list = "/.\n/usr\n/usr/bin\n/usr/bin/ls\n/usr/lib/x/ls\n/bin/cat\n/usr/share/man/man1/ls.1.gz\n";
        assert_eq!(list_executables(list), vec!["/usr/bin/ls", "/bin/cat"]);
    }

    #[test]
    fn continuation_without_field_is_ignored() {
        let stanzas = parse_stanzas(" orphan\nPackage: a\nnot a field\n");
        assert_eq!(stanzas.len(), 1);
        assert_eq!(stanzas[0].fields().len(), 1);
    }
}
