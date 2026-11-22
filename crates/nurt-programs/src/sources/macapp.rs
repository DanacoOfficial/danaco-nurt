// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! macOS application bundles.
//!
//! The provider lists the bundles in `/Applications`, `/System/Applications`,
//! their `Utilities` directories and `~/Applications`, two levels deep, and
//! reads `Contents/Info.plist` in XML or binary form. Localized names come
//! from `Contents/Resources/<lang>.lproj/InfoPlist.strings` for the preferred
//! languages. A bundle without `Info.plist` is read from the `__info_plist`
//! section embedded in its Mach-O executable.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;
use plist::Value;

use super::{is_dir, list_dir, read_file};
use crate::model::{
    CategoryHint, CategoryScheme, EntryKey, EntryKind, LocalizedText, RawProgram, SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError, SourceId,
    SourceKind, SourceWarning, WarningCode,
};

/// Directories of applications of the system, as host paths.
pub const SYSTEM_APP_DIRS: &[&str] = &[
    "/Applications",
    "/Applications/Utilities",
    "/System/Applications",
    "/System/Applications/Utilities",
];

/// Tag of applications installed from the App Store.
pub const APP_STORE_TAG: &str = "app-store";

const BUNDLE_EXTENSION: &str = ".app";
const LPROJ_EXTENSION: &str = ".lproj";
const STRINGS_FILE: &str = "InfoPlist.strings";
const BINARY_PLIST_MAGIC: &[u8] = b"bplist00";
const XML_PLIST_MAGIC: &[u8] = b"<?xml";

/// Names of `.lproj` directories of older bundles, by language code.
const LEGACY_LPROJ: &[(&str, &str)] = &[
    ("de", "German"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("nl", "Dutch"),
];

const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_CIGAM: u32 = 0xcefa_edfe;
const MH_CIGAM_64: u32 = 0xcffa_edfe;
const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
/// Largest number of architectures accepted in a universal binary; larger
/// counts belong to other formats with the same signature, such as Java classes.
const MAX_FAT_ARCHS: u32 = 32;
const LC_SEGMENT: u32 = 0x1;
const LC_SEGMENT_64: u32 = 0x19;
const TEXT_SEGMENT: &[u8] = b"__TEXT";
const INFO_PLIST_SECTION: &[u8] = b"__info_plist";

/// Provider of macOS application bundles.
#[derive(Debug, Clone)]
pub struct MacAppSource {
    id: SourceId,
}

impl Default for MacAppSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MacAppSource {
    /// Provider with the identifier `macapp`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::MACAPP),
        }
    }
}

/// Members of `Info.plist` used by the provider.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BundleInfo {
    /// `CFBundleIdentifier`.
    pub identifier: Option<String>,
    /// `CFBundleName`.
    pub name: Option<String>,
    /// `CFBundleDisplayName`.
    pub display_name: Option<String>,
    /// `CFBundleShortVersionString`, otherwise `CFBundleVersion`.
    pub version: Option<String>,
    /// `CFBundleExecutable`.
    pub executable: Option<String>,
    /// `LSApplicationCategoryType`.
    pub category: Option<String>,
    /// `NSHumanReadableCopyright`.
    pub copyright: Option<String>,
    /// `CFBundleDevelopmentRegion`.
    pub development_region: Option<String>,
}

/// Reads an `Info.plist` in XML or binary form.
///
/// # Errors
///
/// Returns a short reason when the bytes are not a property list holding a
/// dictionary.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::macapp::parse_info_plist;
///
/// let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
/// <plist version="1.0"><dict>
/// <key>CFBundleIdentifier</key><string>org.example.Viewer</string>
/// <key>CFBundleShortVersionString</key><string>2.4</string>
/// </dict></plist>"#;
/// let info = parse_info_plist(xml).unwrap();
/// assert_eq!(info.identifier.as_deref(), Some("org.example.Viewer"));
/// assert_eq!(info.version.as_deref(), Some("2.4"));
/// ```
pub fn parse_info_plist(bytes: &[u8]) -> Result<BundleInfo, String> {
    let value = Value::from_reader(Cursor::new(bytes))
        .map_err(|error| format!("Info.plist cannot be read: {error}"))?;
    let dict = value
        .as_dictionary()
        .ok_or("Info.plist does not hold a dictionary")?;
    let text = |key: &str| {
        dict.get(key)
            .and_then(Value::as_string)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Ok(BundleInfo {
        identifier: text("CFBundleIdentifier"),
        name: text("CFBundleName"),
        display_name: text("CFBundleDisplayName"),
        version: text("CFBundleShortVersionString").or_else(|| text("CFBundleVersion")),
        executable: text("CFBundleExecutable"),
        category: text("LSApplicationCategoryType"),
        copyright: text("NSHumanReadableCopyright"),
        development_region: text("CFBundleDevelopmentRegion"),
    })
}

/// Reads a `.strings` file: text in UTF-16 (with a byte order mark or
/// without, little endian) or UTF-8, or a property list. Comments are
/// skipped; reading stops at the first malformed pair.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::macapp::parse_strings;
///
/// let text = "/* Localized */\n\"CFBundleDisplayName\" = \"Rechner\";\nCFBundleName = Rechner;\n";
/// let strings = parse_strings(text.as_bytes());
/// assert_eq!(strings.get("CFBundleDisplayName").map(String::as_str), Some("Rechner"));
/// assert_eq!(strings.get("CFBundleName").map(String::as_str), Some("Rechner"));
/// ```
#[must_use]
pub fn parse_strings(bytes: &[u8]) -> BTreeMap<String, String> {
    if bytes.starts_with(BINARY_PLIST_MAGIC) || bytes.starts_with(XML_PLIST_MAGIC) {
        return Value::from_reader(Cursor::new(bytes))
            .ok()
            .and_then(Value::into_dictionary)
            .map(|dict| {
                dict.into_iter()
                    .filter_map(|(key, value)| Some((key, value.into_string()?)))
                    .collect()
            })
            .unwrap_or_default();
    }
    let text = decode_text(bytes);
    StringsParser {
        chars: text.chars().collect(),
        at: 0,
    }
    .pairs()
}

fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |bytes: &[u8], big_endian: bool| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if big_endian {
                    u16::from_be_bytes(*pair)
                } else {
                    u16::from_le_bytes(*pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xff, 0xfe, rest @ ..] => utf16(rest, false),
        [0xfe, 0xff, rest @ ..] => utf16(rest, true),
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [_, 0, ..] => utf16(bytes, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Reader of the old-style text form of `.strings` files.
struct StringsParser {
    chars: Vec<char>,
    at: usize,
}

impl StringsParser {
    fn pairs(mut self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        loop {
            self.skip_blank();
            let Some(key) = self.token() else { break };
            self.skip_blank();
            if self.peek() != Some('=') {
                break;
            }
            self.at += 1;
            self.skip_blank();
            let Some(value) = self.token() else { break };
            self.skip_blank();
            if self.peek() != Some(';') {
                break;
            }
            self.at += 1;
            out.insert(key, value);
        }
        out
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_blank(&mut self) {
        loop {
            match (self.peek(), self.chars.get(self.at + 1).copied()) {
                (Some(c), _) if c.is_whitespace() || c == '\u{feff}' => self.at += 1,
                (Some('/'), Some('*')) => {
                    self.at += 2;
                    while self.at < self.chars.len()
                        && !(self.peek() == Some('*') && self.chars.get(self.at + 1) == Some(&'/'))
                    {
                        self.at += 1;
                    }
                    self.at = (self.at + 2).min(self.chars.len());
                }
                (Some('/'), Some('/')) => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.at += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn token(&mut self) -> Option<String> {
        match self.peek()? {
            '"' => {
                self.at += 1;
                self.quoted()
            }
            c if c.is_alphanumeric() || "_.$:/-".contains(c) => {
                let start = self.at;
                while self
                    .peek()
                    .is_some_and(|c| c.is_alphanumeric() || "_.$:/-".contains(c))
                {
                    self.at += 1;
                }
                Some(self.chars[start..self.at].iter().collect())
            }
            _ => None,
        }
    }

    fn quoted(&mut self) -> Option<String> {
        let mut out = String::new();
        loop {
            let c = self.peek()?;
            self.at += 1;
            match c {
                '"' => return Some(out),
                '\\' => {
                    let escaped = self.peek()?;
                    self.at += 1;
                    match escaped {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'U' | 'u' => {
                            let digits: String =
                                self.chars.get(self.at..self.at + 4)?.iter().collect();
                            self.at += 4;
                            out.push(
                                u32::from_str_radix(&digits, 16)
                                    .ok()
                                    .and_then(char::from_u32)?,
                            );
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
    }
}

fn read_u32(bytes: &[u8], offset: usize, big_endian: bool) -> Option<u32> {
    let quad: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(if big_endian {
        u32::from_be_bytes(quad)
    } else {
        u32::from_le_bytes(quad)
    })
}

fn read_u64(bytes: &[u8], offset: usize, big_endian: bool) -> Option<u64> {
    let octet: [u8; 8] = bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?;
    Some(if big_endian {
        u64::from_be_bytes(octet)
    } else {
        u64::from_le_bytes(octet)
    })
}

fn name_is(field: Option<&[u8]>, name: &[u8]) -> bool {
    field.is_some_and(|field| {
        let end = field
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(field.len());
        &field[..end] == name
    })
}

fn to_usize(value: u64) -> Option<usize> {
    usize::try_from(value).ok()
}

/// Section `__TEXT,__info_plist` of a Mach-O executable: a thin image of 32
/// or 64 bits in either byte order, or the first architecture of a universal
/// binary that has the section.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::macapp::macho_info_plist;
///
/// assert_eq!(macho_info_plist(b"#!/bin/sh\n"), None);
/// ```
#[must_use]
pub fn macho_info_plist(bytes: &[u8]) -> Option<&[u8]> {
    match read_u32(bytes, 0, true)? {
        FAT_MAGIC | FAT_MAGIC_64 => {
            let wide = read_u32(bytes, 0, true)? == FAT_MAGIC_64;
            let count = read_u32(bytes, 4, true)?;
            if count > MAX_FAT_ARCHS {
                return None;
            }
            let entry_size = if wide { 32 } else { 20 };
            (0..to_usize(u64::from(count))?).find_map(|index| {
                let at = 8 + index * entry_size;
                let (offset, size) = if wide {
                    (
                        read_u64(bytes, at + 8, true)?,
                        read_u64(bytes, at + 16, true)?,
                    )
                } else {
                    (
                        u64::from(read_u32(bytes, at + 8, true)?),
                        u64::from(read_u32(bytes, at + 12, true)?),
                    )
                };
                let start = to_usize(offset)?;
                let image = bytes.get(start..start.checked_add(to_usize(size)?)?)?;
                thin_info_plist(image)
            })
        }
        _ => thin_info_plist(bytes),
    }
}

fn thin_info_plist(image: &[u8]) -> Option<&[u8]> {
    let (wide, big_endian) = match read_u32(image, 0, false)? {
        MH_MAGIC => (false, false),
        MH_MAGIC_64 => (true, false),
        MH_CIGAM => (false, true),
        MH_CIGAM_64 => (true, true),
        _ => return None,
    };
    let count = read_u32(image, 16, big_endian)?;
    let mut at = if wide { 32 } else { 28 };
    for _ in 0..count {
        let command = read_u32(image, at, big_endian)?;
        let size = to_usize(u64::from(read_u32(image, at + 4, big_endian)?))?;
        if size < 8 {
            return None;
        }
        let segment = (command == LC_SEGMENT_64 && wide) || (command == LC_SEGMENT && !wide);
        if segment && name_is(image.get(at + 8..at + 24), TEXT_SEGMENT) {
            return text_segment_info_plist(image, at, wide, big_endian);
        }
        at = at.checked_add(size)?;
    }
    None
}

/// Section `__info_plist` of the `__TEXT` segment command at offset `at`.
fn text_segment_info_plist(image: &[u8], at: usize, wide: bool, big_endian: bool) -> Option<&[u8]> {
    let (count_at, first, section_size) = if wide { (64, 72, 80) } else { (48, 56, 68) };
    let sections = read_u32(image, at + count_at, big_endian)?;
    let section = (0..to_usize(u64::from(sections))?)
        .map(|index| at + first + index * section_size)
        .find(|section| name_is(image.get(*section..*section + 16), INFO_PLIST_SECTION))?;
    let (length, offset) = if wide {
        (
            read_u64(image, section + 40, big_endian)?,
            read_u32(image, section + 48, big_endian)?,
        )
    } else {
        (
            u64::from(read_u32(image, section + 36, big_endian)?),
            read_u32(image, section + 40, big_endian)?,
        )
    };
    let start = to_usize(u64::from(offset))?;
    image.get(start..start.checked_add(to_usize(length)?)?)
}

/// Language code of a `.lproj` directory name: `de.lproj`, `pt_BR.lproj`,
/// `zh-Hans.lproj` and the old English names such as `German.lproj`.
fn lproj_language(dir: &str) -> Option<String> {
    let stem = dir.strip_suffix(LPROJ_EXTENSION)?;
    if let Some((code, _)) = LEGACY_LPROJ
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(stem))
    {
        return Some((*code).to_owned());
    }
    let code = stem.split(['_', '-']).next()?.to_ascii_lowercase();
    ((2..=3).contains(&code.len()) && code.bytes().all(|byte| byte.is_ascii_lowercase()))
        .then_some(code)
}

/// Bundles below the application directories, host paths without
/// repetition, in directory order.
fn bundles(env: &HostEnv) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for dir in app_dirs(env) {
        for (name, path) in list_dir(env, &dir) {
            if name.starts_with('.') || !is_dir(env, &path) {
                continue;
            }
            if name.to_ascii_lowercase().ends_with(BUNDLE_EXTENSION) {
                if seen.insert(path.clone()) {
                    out.push(path);
                }
                continue;
            }
            for (inner, inner_path) in list_dir(env, &path) {
                let bundle = inner.to_ascii_lowercase().ends_with(BUNDLE_EXTENSION);
                if bundle && is_dir(env, &inner_path) && seen.insert(inner_path.clone()) {
                    out.push(inner_path);
                }
            }
        }
    }
    out
}

fn app_dirs(env: &HostEnv) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = SYSTEM_APP_DIRS.iter().map(PathBuf::from).collect();
    if let Some(home) = env.home() {
        dirs.push(home.join("Applications"));
    }
    dirs
}

impl MacAppSource {
    fn info(&self, env: &HostEnv, bundle: &Path, stem: &str) -> Result<BundleInfo, String> {
        let contents = bundle.join("Contents");
        let plist = contents.join("Info.plist");
        if let Ok(bytes) = read_file(env, &self.id, &plist) {
            return parse_info_plist(&bytes);
        }
        let executable = contents.join("MacOS").join(stem);
        let bytes = read_file(env, &self.id, &executable).map_err(|error| error.message)?;
        let embedded = macho_info_plist(&bytes)
            .ok_or("bundle has neither Info.plist nor an embedded property list")?;
        parse_info_plist(embedded)
    }

    /// Localized display names by language, for the preferred languages.
    fn localized_names(&self, env: &HostEnv, bundle: &Path) -> BTreeMap<String, String> {
        let resources = bundle.join("Contents").join("Resources");
        let wanted = env.languages();
        let mut out = BTreeMap::new();
        for (dir, path) in list_dir(env, &resources) {
            let Some(language) = lproj_language(&dir) else {
                continue;
            };
            if !wanted.contains(&language) || out.contains_key(&language) {
                continue;
            }
            let Ok(bytes) = read_file(env, &self.id, &path.join(STRINGS_FILE)) else {
                continue;
            };
            let strings = parse_strings(&bytes);
            if let Some(name) = strings
                .get("CFBundleDisplayName")
                .or_else(|| strings.get("CFBundleName"))
            {
                out.insert(language, name.clone());
            }
        }
        out
    }

    fn program(&self, env: &HostEnv, bundle: &Path, data: &mut SourceData) -> Option<RawProgram> {
        let file = bundle.file_name()?.to_string_lossy().into_owned();
        let stem = file
            .get(..file.len().checked_sub(BUNDLE_EXTENSION.len())?)?
            .to_owned();
        let host = bundle.display().to_string();
        let info = match self.info(env, bundle, &stem) {
            Ok(info) => info,
            Err(reason) => {
                data.warnings.push(
                    SourceWarning::new(WarningCode::SourceUnavailable, &self.id, reason)
                        .at(host)
                        .named(stem),
                );
                return None;
            }
        };
        let display = info
            .display_name
            .clone()
            .or_else(|| info.name.clone())
            .unwrap_or_else(|| stem.clone());
        let region = info
            .development_region
            .as_deref()
            .and_then(|region| lproj_language(&format!("{region}{LPROJ_EXTENSION}")));
        let mut app_name = match &region {
            Some(region) => LocalizedText::in_lang(display, region),
            None => LocalizedText::new(display),
        };
        for (language, name) in self.localized_names(env, bundle) {
            app_name.set(&language, name);
        }
        let mut tags = Vec::new();
        if is_dir(env, &bundle.join("Contents").join("_MASReceipt")) {
            tags.push(APP_STORE_TAG.to_owned());
        }
        Some(RawProgram {
            key: Some(EntryKey::new(&self.id, host.clone())),
            kind: EntryKind::App,
            path: Some(host),
            version: info.version,
            app_id: info.identifier,
            app_name: Some(app_name),
            executable: info.executable,
            category_hints: info
                .category
                .into_iter()
                .map(|category| CategoryHint::new(CategoryScheme::MacosCategory, category))
                .collect(),
            tags,
            installed: Some(true),
            ..RawProgram::new(stem)
        })
    }
}

impl ProgramSource for MacAppSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::App
    }

    fn detect(&self, env: &HostEnv) -> bool {
        env.os() == Os::Macos && app_dirs(env).iter().any(|dir| is_dir(env, dir))
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder = FingerprintBuilder::new(env);
        for dir in app_dirs(env) {
            builder = builder.path(dir);
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let mut data = SourceData::default();
        for bundle in bundles(env) {
            deadline.check(&self.id)?;
            if let Some(program) = self.program(env, &bundle, &mut data) {
                data.programs.push(program);
            }
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lproj_names_to_languages() {
        assert_eq!(lproj_language("de.lproj").as_deref(), Some("de"));
        assert_eq!(lproj_language("pt_BR.lproj").as_deref(), Some("pt"));
        assert_eq!(lproj_language("zh-Hans.lproj").as_deref(), Some("zh"));
        assert_eq!(lproj_language("German.lproj").as_deref(), Some("de"));
        assert_eq!(lproj_language("Base.lproj"), None);
        assert_eq!(lproj_language("Resources"), None);
    }

    #[test]
    fn reads_utf16_strings_with_escapes_and_comments() {
        let text = "// line\n\"a\" = \"x\\\"y\\u00e9\";\n/* block */ b = c;";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let strings = parse_strings(&bytes);
        assert_eq!(strings.get("a").map(String::as_str), Some("x\"y\u{e9}"));
        assert_eq!(strings.get("b").map(String::as_str), Some("c"));
    }

    #[test]
    fn stops_at_malformed_pair() {
        let strings = parse_strings(b"\"a\" = \"1\"; \"b\" \"2\"; \"c\" = \"3\";");
        assert_eq!(strings.len(), 1);
    }

    #[test]
    fn rejects_fat_header_with_too_many_architectures() {
        let mut bytes = FAT_MAGIC.to_be_bytes().to_vec();
        bytes.extend(1000u32.to_be_bytes());
        assert_eq!(macho_info_plist(&bytes), None);
    }
}
