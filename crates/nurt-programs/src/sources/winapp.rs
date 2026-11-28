// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::sources::winapp
// Responsibility:   Applications registered with Windows.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Applications registered with Windows.
//!
//! The provider reads the uninstall entries of the 64-bit and 32-bit views
//! of `HKLM` and of `HKCU`, and the `App Paths` keys that let the shell start
//! an application by the name of its executable. Entries that Windows hides
//! from the list of installed programs (`SystemComponent`, updates with a
//! `ParentKeyName`) are skipped. The version resource of an executable is
//! read from the bytes of the file when details are requested. Packaged
//! applications (AppX) are not listed.

use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use nurt_contracts::env::Os;

use super::winget::UNINSTALL_KEY;
use crate::model::{
    AliasKind, CategoryHint, CategoryScheme, Detail, DetailWant, EntryKey, EntryKind,
    LocalizedText, RawAlias, RawProgram, SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, RegistryHive, RegistryValue,
    RegistryView, SourceError, SourceId, SourceKind, SourceWarning, WarningCode,
};

/// Key of the executables the shell starts by name.
pub const APP_PATHS_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths";

/// Hives and views read, in order of precedence.
const VIEWS: [(RegistryHive, RegistryView); 3] = [
    (RegistryHive::CurrentUser, RegistryView::Native),
    (RegistryHive::LocalMachine, RegistryView::Native),
    (RegistryHive::LocalMachine, RegistryView::Wow32),
];

const EXE_EXTENSION: &str = ".exe";

/// Bytes read from the start of an executable to find its sections.
const PE_HEADER_BYTES: u64 = 64 * 1024;
/// Largest resource section read for the version resource.
const MAX_RESOURCE_BYTES: u64 = 16 * 1024 * 1024;

/// Offset of the PE header offset in the DOS header.
const DOS_LFANEW_OFFSET: usize = 0x3c;
/// Size of the COFF file header.
const COFF_HEADER_SIZE: usize = 20;
/// Size of one section header.
const SECTION_HEADER_SIZE: usize = 40;
/// Signature of the optional header of 32-bit images.
const PE32_MAGIC: u16 = 0x10b;
/// Signature of the optional header of 64-bit images.
const PE32_PLUS_MAGIC: u16 = 0x20b;
/// Index of the resource table among the data directories.
const RESOURCE_DIRECTORY_INDEX: usize = 2;
/// Resource type of version information.
const RT_VERSION: u32 = 16;
/// Language preferred among several version resources (English, United States).
const PREFERRED_LANGUAGE: u32 = 0x409;
/// Bit of a resource directory entry that marks a subdirectory or a name.
const RESOURCE_HIGH_BIT: u32 = 0x8000_0000;
/// Size of a resource directory header.
const RESOURCE_DIRECTORY_SIZE: usize = 16;
/// Size of a resource directory entry.
const RESOURCE_ENTRY_SIZE: usize = 8;
/// Signature of `VS_FIXEDFILEINFO`.
const FIXED_FILE_INFO_SIGNATURE: u32 = 0xfeef_04bd;
/// Size of `VS_FIXEDFILEINFO`.
const FIXED_FILE_INFO_SIZE: usize = 52;
/// Size of the header of a version block: length, value length and type.
const VERSION_BLOCK_HEADER: usize = 6;
/// Value type of a version block holding text.
const VERSION_TEXT_TYPE: u16 = 1;

/// Provider of applications registered with Windows.
#[derive(Debug, Clone)]
pub struct WinAppSource {
    id: SourceId,
}

impl Default for WinAppSource {
    fn default() -> Self {
        Self::new()
    }
}

impl WinAppSource {
    /// Provider with the identifier `winapp`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::WINAPP),
        }
    }
}

/// Strings and numbers of the version resource of an executable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionInfo {
    /// `FileDescription`.
    pub file_description: Option<String>,
    /// `ProductName`.
    pub product_name: Option<String>,
    /// `CompanyName`.
    pub company_name: Option<String>,
    /// `FileVersion` as text.
    pub file_version: Option<String>,
    /// `ProductVersion` as text.
    pub product_version: Option<String>,
    /// File version of `VS_FIXEDFILEINFO`, four numbers joined by dots.
    pub fixed_file_version: Option<String>,
}

impl VersionInfo {
    /// Description shown on a card: the file description, the product with
    /// its version and the company, one per line.
    #[must_use]
    pub fn description(&self) -> Option<String> {
        let product = self.product_name.as_deref().map(|name| {
            match self
                .product_version
                .as_deref()
                .or(self.fixed_file_version.as_deref())
            {
                Some(version) => format!("{name} {version}"),
                None => name.to_owned(),
            }
        });
        let lines: Vec<String> = [
            self.file_description.clone(),
            product,
            self.company_name.clone(),
        ]
        .into_iter()
        .flatten()
        .filter(|line| !line.trim().is_empty())
        .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let pair = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([pair[0], pair[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let quad = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]))
}

fn to_usize(value: u32) -> Option<usize> {
    usize::try_from(value).ok()
}

fn align4(offset: usize) -> usize {
    offset.saturating_add(3) & !3
}

/// Section table and resource directory of a PE image.
#[derive(Debug, Clone)]
struct PeLayout {
    sections: Vec<SectionHeader>,
    resource_rva: u32,
    resource_size: u32,
}

#[derive(Debug, Clone, Copy)]
struct SectionHeader {
    virtual_address: u32,
    virtual_size: u32,
    raw_pointer: u32,
    raw_size: u32,
}

impl PeLayout {
    /// Reads the headers at the start of an image.
    fn parse(head: &[u8]) -> Option<Self> {
        if head.get(..2)? != b"MZ" {
            return None;
        }
        let pe = to_usize(u32_at(head, DOS_LFANEW_OFFSET)?)?;
        if head.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let coff = pe + 4;
        let section_count = usize::from(u16_at(head, coff + 2)?);
        let optional_size = usize::from(u16_at(head, coff + 16)?);
        let optional = coff + COFF_HEADER_SIZE;
        let (count_offset, directories_offset) = match u16_at(head, optional)? {
            PE32_MAGIC => (92, 96),
            PE32_PLUS_MAGIC => (108, 112),
            _ => return None,
        };
        let directory_count = to_usize(u32_at(head, optional + count_offset)?)?;
        if directory_count <= RESOURCE_DIRECTORY_INDEX {
            return None;
        }
        let resource = optional + directories_offset + RESOURCE_DIRECTORY_INDEX * 8;
        let resource_rva = u32_at(head, resource)?;
        let resource_size = u32_at(head, resource + 4)?;
        if resource_rva == 0 || resource_size == 0 {
            return None;
        }
        let table = optional + optional_size;
        let sections = (0..section_count)
            .map(|index| {
                let at = table + index * SECTION_HEADER_SIZE;
                Some(SectionHeader {
                    virtual_size: u32_at(head, at + 8)?,
                    virtual_address: u32_at(head, at + 12)?,
                    raw_size: u32_at(head, at + 16)?,
                    raw_pointer: u32_at(head, at + 20)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            sections,
            resource_rva,
            resource_size,
        })
    }

    /// File offset and length of the resource data.
    fn resource_range(&self) -> Option<(u64, u64)> {
        let rva = self.resource_rva;
        let section = self.sections.iter().find(|section| {
            let span = section.virtual_size.max(section.raw_size);
            rva >= section.virtual_address && rva - section.virtual_address < span
        })?;
        let delta = rva - section.virtual_address;
        let available = section.raw_size.checked_sub(delta)?;
        let offset = u64::from(section.raw_pointer) + u64::from(delta);
        Some((offset, u64::from(self.resource_size.min(available))))
    }

    /// Version resource in the resource data `resources`, which starts at
    /// the resource directory.
    fn version_in(&self, resources: &[u8]) -> Option<VersionInfo> {
        let types = directory_entries(resources, 0)?;
        let (_, names) = types.iter().find(|(id, _)| *id == RT_VERSION)?;
        let names = directory_entries(resources, subdirectory(*names)?)?;
        let (_, languages) = names.first()?;
        let languages = directory_entries(resources, subdirectory(*languages)?)?;
        let (_, data) = languages
            .iter()
            .find(|(id, _)| *id == PREFERRED_LANGUAGE)
            .or_else(|| languages.first())?;
        if data & RESOURCE_HIGH_BIT != 0 {
            return None;
        }
        let entry = to_usize(*data)?;
        let data_rva = u32_at(resources, entry)?;
        let data_size = to_usize(u32_at(resources, entry + 4)?)?;
        let start = to_usize(data_rva.checked_sub(self.resource_rva)?)?;
        let block = resources.get(start..start.checked_add(data_size)?)?;
        parse_version_block(block)
    }
}

/// Entries of the resource directory at `offset`: identifier (names have the
/// high bit set) and target.
fn directory_entries(resources: &[u8], offset: usize) -> Option<Vec<(u32, u32)>> {
    let named = usize::from(u16_at(resources, offset + 12)?);
    let ids = usize::from(u16_at(resources, offset + 14)?);
    (0..named + ids)
        .map(|index| {
            let at = offset + RESOURCE_DIRECTORY_SIZE + index * RESOURCE_ENTRY_SIZE;
            Some((u32_at(resources, at)?, u32_at(resources, at + 4)?))
        })
        .collect()
}

fn subdirectory(target: u32) -> Option<usize> {
    (target & RESOURCE_HIGH_BIT != 0)
        .then_some(target & !RESOURCE_HIGH_BIT)
        .and_then(to_usize)
}

/// One block of a version resource.
struct VersionBlock<'a> {
    key: String,
    value: &'a [u8],
    children: Vec<VersionBlock<'a>>,
}

/// Reads the block at `start` of `data` and returns it with the offset after
/// it. Offsets are aligned to four bytes from the start of the resource.
fn read_block(data: &[u8], start: usize, depth: usize) -> Option<(VersionBlock<'_>, usize)> {
    const MAX_DEPTH: usize = 4;
    let length = usize::from(u16_at(data, start)?);
    let value_length = usize::from(u16_at(data, start + 2)?);
    let kind = u16_at(data, start + 4)?;
    let end = start.checked_add(length)?;
    if length < VERSION_BLOCK_HEADER || end > data.len() {
        return None;
    }
    let mut cursor = start + VERSION_BLOCK_HEADER;
    let mut key_units = Vec::new();
    while cursor + 2 <= end {
        let unit = u16_at(data, cursor)?;
        cursor += 2;
        if unit == 0 {
            break;
        }
        key_units.push(unit);
    }
    let value_start = align4(cursor).min(end);
    let value_bytes = if kind == VERSION_TEXT_TYPE {
        value_length * 2
    } else {
        value_length
    };
    let value_end = value_start.saturating_add(value_bytes).min(end);
    let mut children = Vec::new();
    let mut child = align4(value_end);
    while depth < MAX_DEPTH && child + VERSION_BLOCK_HEADER <= end {
        let Some((block, next)) = read_block(&data[..end], child, depth + 1) else {
            break;
        };
        children.push(block);
        child = next;
    }
    let block = VersionBlock {
        key: String::from_utf16_lossy(&key_units),
        value: &data[value_start..value_end],
        children,
    };
    Some((block, align4(end)))
}

fn utf16_value(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16_lossy(&units).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn fixed_version(value: &[u8]) -> Option<String> {
    if value.len() < FIXED_FILE_INFO_SIZE || u32_at(value, 0)? != FIXED_FILE_INFO_SIGNATURE {
        return None;
    }
    let high = u32_at(value, 8)?;
    let low = u32_at(value, 12)?;
    Some(format!(
        "{}.{}.{}.{}",
        high >> 16,
        high & 0xffff,
        low >> 16,
        low & 0xffff
    ))
}

/// Reads a `VS_VERSIONINFO` structure.
fn parse_version_block(data: &[u8]) -> Option<VersionInfo> {
    let (root, _) = read_block(data, 0, 0)?;
    if root.key != "VS_VERSION_INFO" {
        return None;
    }
    let mut info = VersionInfo {
        fixed_file_version: fixed_version(root.value),
        ..VersionInfo::default()
    };
    let tables: Vec<&VersionBlock<'_>> = root
        .children
        .iter()
        .filter(|block| block.key == "StringFileInfo")
        .flat_map(|block| block.children.iter())
        .collect();
    let table = tables
        .iter()
        .find(|table| table.key.to_ascii_lowercase().starts_with("0409"))
        .or_else(|| tables.first());
    if let Some(table) = table {
        for entry in &table.children {
            let value = utf16_value(entry.value);
            match entry.key.as_str() {
                "FileDescription" => info.file_description = value,
                "ProductName" => info.product_name = value,
                "CompanyName" => info.company_name = value,
                "FileVersion" => info.file_version = value,
                "ProductVersion" => info.product_version = value,
                _ => {}
            }
        }
    }
    Some(info)
}

/// Reads the version resource of a Portable Executable image held in
/// memory, without loading the image.
///
/// Returns `None` when the bytes are not a PE image or the image has no
/// readable version resource.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::winapp::parse_pe_version;
///
/// assert_eq!(parse_pe_version(b"not an executable"), None);
/// ```
#[must_use]
pub fn parse_pe_version(bytes: &[u8]) -> Option<VersionInfo> {
    let layout = PeLayout::parse(bytes)?;
    let (offset, length) = layout.resource_range()?;
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(length).ok()?)?;
    layout.version_in(bytes.get(start..end.min(bytes.len()))?)
}

/// Reads the version resource of the executable at the host path `path`;
/// only the headers and the resource section are read.
#[must_use]
pub fn read_pe_version(env: &HostEnv, path: &Path) -> Option<VersionInfo> {
    let mut file = std::fs::File::open(env.sys_path(path)).ok()?;
    let mut head = Vec::new();
    (&mut file)
        .take(PE_HEADER_BYTES)
        .read_to_end(&mut head)
        .ok()?;
    let layout = PeLayout::parse(&head)?;
    let (offset, length) = layout.resource_range()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut resources = Vec::new();
    file.take(length.min(MAX_RESOURCE_BYTES))
        .read_to_end(&mut resources)
        .ok()?;
    layout.version_in(&resources)
}

/// Expands `%NAME%` references with the variables of `env`; unknown names
/// are kept.
fn expand(env: &HostEnv, text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match env.var_text(name) {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Executable named by a `DisplayIcon` or an `App Paths` value: quotes and
/// an icon index after a comma are removed; `None` for other files.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::winapp::executable_of;
///
/// assert_eq!(
///     executable_of("\"C:\\Program Files\\Editor\\editor.exe\",0").as_deref(),
///     Some("C:\\Program Files\\Editor\\editor.exe")
/// );
/// assert_eq!(executable_of("C:\\Windows\\icon.ico"), None);
/// ```
#[must_use]
pub fn executable_of(value: &str) -> Option<String> {
    let mut text = value.trim();
    if let Some((path, index)) = text.rsplit_once(',') {
        let index = index.trim();
        if !index.is_empty()
            && index
                .trim_start_matches('-')
                .bytes()
                .all(|byte| byte.is_ascii_digit())
        {
            text = path.trim();
        }
    }
    let text = text.trim_matches('"').trim();
    text.to_ascii_lowercase()
        .ends_with(EXE_EXTENSION)
        .then(|| text.to_owned())
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn text_value(value: Option<&RegistryValue>) -> Option<String> {
    value
        .and_then(RegistryValue::as_text)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Uninstall entry read from the registry.
#[derive(Debug, Clone)]
struct App {
    key: String,
    name: String,
    version: Option<String>,
    publisher: Option<String>,
    location: Option<String>,
    executable: Option<String>,
    homepage: Option<String>,
    aliases: Vec<RawAlias>,
}

impl App {
    fn owns(&self, path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        if self
            .executable
            .as_ref()
            .is_some_and(|exe| exe.to_ascii_lowercase() == path)
        {
            return true;
        }
        self.location.as_ref().is_some_and(|location| {
            let location = location.trim_end_matches(['\\', '/']).to_ascii_lowercase();
            !location.is_empty()
                && path.len() > location.len()
                && path.starts_with(&location)
                && matches!(path.as_bytes()[location.len()], b'\\' | b'/')
        })
    }
}

impl WinAppSource {
    fn read_uninstall(
        &self,
        env: &HostEnv,
        deadline: &Deadline<'_>,
        data: &mut SourceData,
    ) -> Result<Vec<App>, SourceError> {
        let Some(registry) = env.registry() else {
            return Ok(Vec::new());
        };
        let mut apps: Vec<App> = Vec::new();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        for (hive, view) in VIEWS {
            let subkeys = match registry.subkeys(hive, view, UNINSTALL_KEY) {
                Ok(subkeys) => subkeys.unwrap_or_default(),
                Err(error) => {
                    data.warnings.push(SourceWarning::from(error));
                    continue;
                }
            };
            for subkey in subkeys {
                deadline.check(&self.id)?;
                let key = format!("{UNINSTALL_KEY}\\{subkey}");
                let Ok(Some(values)) = registry.values(hive, view, &key) else {
                    continue;
                };
                let hidden = values
                    .get("SystemComponent")
                    .and_then(RegistryValue::as_u64)
                    == Some(1)
                    || values.contains_key("ParentKeyName");
                let Some(name) = text_value(values.get("DisplayName")) else {
                    continue;
                };
                if hidden {
                    continue;
                }
                let version = text_value(values.get("DisplayVersion"));
                let identity = (name.to_lowercase(), version.clone().unwrap_or_default());
                if !seen.insert(identity) {
                    continue;
                }
                let location =
                    text_value(values.get("InstallLocation")).map(|text| expand(env, &text));
                let executable = text_value(values.get("DisplayIcon"))
                    .and_then(|icon| executable_of(&expand(env, &icon)));
                apps.push(App {
                    key: subkey,
                    name,
                    version,
                    publisher: text_value(values.get("Publisher")),
                    location,
                    executable,
                    homepage: text_value(values.get("URLInfoAbout")),
                    aliases: Vec::new(),
                });
            }
        }
        Ok(apps)
    }

    /// `App Paths` entries as executable name and file.
    fn read_app_paths(&self, env: &HostEnv, data: &mut SourceData) -> Vec<(String, String)> {
        let Some(registry) = env.registry() else {
            return Vec::new();
        };
        let mut out: Vec<(String, String)> = Vec::new();
        for (hive, view) in VIEWS {
            let subkeys = match registry.subkeys(hive, view, APP_PATHS_KEY) {
                Ok(subkeys) => subkeys.unwrap_or_default(),
                Err(error) => {
                    data.warnings.push(SourceWarning::from(error));
                    continue;
                }
            };
            for subkey in subkeys {
                if !subkey.to_ascii_lowercase().ends_with(EXE_EXTENSION)
                    || out
                        .iter()
                        .any(|(known, _)| known.eq_ignore_ascii_case(&subkey))
                {
                    continue;
                }
                let key = format!("{APP_PATHS_KEY}\\{subkey}");
                let Ok(Some(values)) = registry.values(hive, view, &key) else {
                    continue;
                };
                let Some(path) = text_value(values.get(""))
                    .and_then(|value| executable_of(&expand(env, &value)))
                else {
                    continue;
                };
                out.push((subkey, path));
            }
        }
        out
    }

    fn program(&self, app: App) -> RawProgram {
        let key = app.executable.clone().unwrap_or_else(|| app.key.clone());
        let mut hints = vec![CategoryHint::new(CategoryScheme::Keyword, app.name.clone())];
        if let Some(publisher) = &app.publisher {
            hints.push(CategoryHint::new(
                CategoryScheme::Keyword,
                publisher.clone(),
            ));
        }
        RawProgram {
            key: Some(EntryKey::new(&self.id, key)),
            kind: EntryKind::App,
            path: app.executable.clone().or(app.location),
            aliases: app.aliases,
            version: app.version,
            app_id: Some(app.key),
            app_name: Some(LocalizedText::new(app.name.clone())),
            executable: app
                .executable
                .as_deref()
                .map(|exe| file_name(exe).to_owned()),
            homepage: app.homepage,
            category_hints: hints,
            installed: Some(true),
            ..RawProgram::new(app.name)
        }
    }
}

impl ProgramSource for WinAppSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::App
    }

    fn detect(&self, env: &HostEnv) -> bool {
        env.os() == Os::Windows
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder = FingerprintBuilder::new(env);
        for (hive, view) in VIEWS {
            builder = builder
                .registry_key(hive, view, UNINSTALL_KEY)
                .registry_key(hive, view, APP_PATHS_KEY);
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let mut data = SourceData::default();
        if env.registry().is_none() {
            data.warnings.push(SourceWarning::new(
                WarningCode::SourceUnavailable,
                &self.id,
                "the registry is not available to this process",
            ));
            return Ok(data);
        }
        let mut apps = self.read_uninstall(env, deadline, &mut data)?;
        for (exe, path) in self.read_app_paths(env, &mut data) {
            let command = exe[..exe.len() - EXE_EXTENSION.len()].to_owned();
            if let Some(app) = apps.iter_mut().find(|app| app.owns(&path)) {
                app.executable.get_or_insert_with(|| path.clone());
                if !app.name.eq_ignore_ascii_case(&command) {
                    app.aliases.push(RawAlias {
                        name: command,
                        kind: AliasKind::Alias,
                        path: Some(path),
                    });
                }
                continue;
            }
            data.programs.push(RawProgram {
                key: Some(EntryKey::new(&self.id, path.clone())),
                kind: EntryKind::App,
                path: Some(path.clone()),
                executable: Some(file_name(&path).to_owned()),
                installed: Some(true),
                category_hints: vec![CategoryHint::new(CategoryScheme::Keyword, command.clone())],
                ..RawProgram::new(command)
            });
        }
        let programs: Vec<RawProgram> = apps.into_iter().map(|app| self.program(app)).collect();
        data.programs.splice(0..0, programs);
        Ok(data)
    }

    fn detail(
        &self,
        env: &HostEnv,
        key: &EntryKey,
        want: &DetailWant,
    ) -> Result<Option<Detail>, SourceError> {
        if key.source != self.id
            || !want.description
            || !key.key.to_ascii_lowercase().ends_with(EXE_EXTENSION)
        {
            return Ok(None);
        }
        let description =
            read_pe_version(env, Path::new(&key.key)).and_then(|info| info.description());
        Ok(description.map(|description| Detail {
            description: Some(description),
            ..Detail::default()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_variables_only() {
        let env = HostEnv::new(Os::Windows).with_var("ProgramFiles", "C:\\Program Files");
        assert_eq!(
            expand(&env, "%ProgramFiles%\\App\\a.exe"),
            "C:\\Program Files\\App\\a.exe"
        );
        assert_eq!(expand(&env, "%Unknown%\\a.exe"), "%Unknown%\\a.exe");
        assert_eq!(expand(&env, "100%"), "100%");
    }

    #[test]
    fn rejects_truncated_pe_headers() {
        let mut bytes = vec![0u8; 0x80];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[DOS_LFANEW_OFFSET] = 0x40;
        bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
        assert_eq!(parse_pe_version(&bytes), None);
    }

    #[test]
    fn describes_version_without_product() {
        let info = VersionInfo {
            file_description: Some("Text editor".to_owned()),
            company_name: Some("Example Ltd".to_owned()),
            ..VersionInfo::default()
        };
        assert_eq!(
            info.description().as_deref(),
            Some("Text editor\nExample Ltd")
        );
        assert_eq!(VersionInfo::default().description(), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
