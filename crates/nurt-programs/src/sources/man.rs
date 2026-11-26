// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Manual pages and `whatis` databases.
//!
//! Manual directories come from `MANPATH` (an empty item stands for the
//! defaults), from `MANDATORY_MANPATH`, `MANPATH_MAP` and `MANPATH` lines of
//! `/etc/manpath.config`, `/etc/man_db.conf` and `/etc/man.conf`, from the
//! `../share/man` and `../man` directories of `PATH` and from the defaults.
//! Pages of sections 1, 6 and 8 are read: the first 16 KiB of each file,
//! decompressed (`.gz`, `.xz`, `.bz2`, `.zst`) and decoded (UTF-8, otherwise
//! detected). The `NAME` section is the first `.SH` or `.Sh` section,
//! whatever its title; `.so` redirects once. Localized pages live in
//! `<dir>/<lang>/manN`. `whatis` files (macOS, BSD) are read as well. The
//! binary `mandb` indexes are not read. Manual pages only describe
//! programs; they never create a card alone.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::model::{
    CategoryHint, CategoryScheme, Detail, DetailWant, EntryKey, LocalizedText, RawProgram,
    SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError,
    SourceErrorKind, SourceId, SourceKind,
};
use crate::sources::{Compression, decompress_reader, is_dir, list_dir, read_file};
use crate::text::{decode, flatten};

/// Sections whose pages describe commands.
pub const SECTIONS: &[&str] = &["1", "6", "8"];
/// Default manual directories.
pub const DEFAULT_DIRS: &[&str] = &["/usr/share/man", "/usr/local/share/man", "/usr/local/man"];
/// Configuration files of man implementations.
pub const CONFIG_FILES: &[&str] = &["/etc/manpath.config", "/etc/man_db.conf", "/etc/man.conf"];
/// Bytes of a page read for its `NAME` section.
pub const PAGE_PREFIX_BYTES: u64 = 16 * 1024;
const WHATIS_FILE: &str = "whatis";
/// Marker of the separator between names and description.
const DASH_MARK: char = '\u{1}';

/// Names and description of a manual page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManName {
    /// Names before the separator.
    pub names: Vec<String>,
    /// Description after the separator.
    pub description: String,
}

/// Extracts the names and description of the `NAME` section of a page in
/// `man` or `mdoc` format.
#[must_use]
pub fn parse_name_section(page: &str) -> Option<ManName> {
    let mut inside = false;
    let mut parts: Vec<String> = Vec::new();
    for line in page.lines() {
        let heading = line.starts_with(".SH") || line.starts_with(".Sh");
        if heading {
            if inside {
                break;
            }
            inside = true;
            continue;
        }
        if inside {
            if line.starts_with(".SS") || line.starts_with(".Ss") {
                break;
            }
            if let Some(text) = line_text(line) {
                parts.push(text);
            }
        }
    }
    split_name_line(&flatten(&parts.join(" ")))
}

/// Text of one line of the `NAME` section; `None` for requests without
/// text.
fn line_text(line: &str) -> Option<String> {
    let Some(request) = line.strip_prefix('.').or_else(|| line.strip_prefix('\'')) else {
        return Some(strip_escapes(line));
    };
    let (name, rest) = request
        .trim_start()
        .split_once(char::is_whitespace)
        .unwrap_or((request.trim(), ""));
    let arguments = split_arguments(rest);
    match name {
        "Nm" => Some(arguments.join(" ")),
        "Nd" => Some(format!("{DASH_MARK} {}", arguments.join(" "))),
        "B" | "I" | "SM" | "SB" => Some(arguments.join(" ")),
        "BR" | "BI" | "IB" | "IR" | "RB" | "RI" => Some(arguments.concat()),
        _ => None,
    }
}

fn split_arguments(text: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    arguments.push(strip_escapes(&std::mem::take(&mut current)));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        arguments.push(strip_escapes(&current));
    }
    arguments
}

/// Removes roff escape sequences; `\-` becomes the separator marker when it
/// is the first one, a hyphen otherwise.
#[must_use]
pub fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(next) = chars.next() else { break };
        match next {
            'f' | 'F' => skip_name(&mut chars),
            's' => {
                if matches!(chars.peek(), Some('+' | '-')) {
                    chars.next();
                }
                if chars.peek().is_some_and(|c| *c == '(' || *c == '[') {
                    skip_name(&mut chars);
                } else {
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        chars.next();
                    }
                }
            }
            '(' => out.push_str(glyph(&take_chars(&mut chars, 2))),
            '[' => out.push_str(glyph(&take_until(&mut chars, ']'))),
            '*' => {
                let name = match chars.peek() {
                    Some('(') => {
                        chars.next();
                        take_chars(&mut chars, 2)
                    }
                    Some('[') => {
                        chars.next();
                        take_until(&mut chars, ']')
                    }
                    _ => chars.next().map(String::from).unwrap_or_default(),
                };
                out.push_str(glyph(&name));
            }
            '-' => out.push(if out.contains(DASH_MARK) {
                '-'
            } else {
                DASH_MARK
            }),
            'e' | '\\' => out.push('\\'),
            ' ' | '~' | '0' => out.push(' '),
            '"' => break,
            '&' | '|' | '^' | ')' | 'c' | '%' | ':' => {}
            other => out.push(other),
        }
    }
    out
}

fn skip_name(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        Some('(') => {
            take_chars(chars, 2);
        }
        Some('[') => {
            take_until(chars, ']');
        }
        _ => {}
    }
}

fn take_chars(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, count: usize) -> String {
    chars.take(count).collect()
}

fn take_until(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, end: char) -> String {
    chars.take_while(|c| *c != end).collect()
}

fn glyph(name: &str) -> &'static str {
    match name {
        "em" => "\u{2014}",
        "en" => "\u{2013}",
        "aq" | "oq" | "cq" => "'",
        "dq" | "lq" | "rq" | "Lq" | "Rq" => "\"",
        "hy" | "mi" => "-",
        "bu" => "\u{2022}",
        "R" | "rg" => "\u{ae}",
        "Tm" | "tm" => "\u{2122}",
        "co" => "\u{a9}",
        _ => "",
    }
}

/// Splits `names - description`: at the separator marker, otherwise at
/// the first ` - `.
#[must_use]
pub fn split_name_line(line: &str) -> Option<ManName> {
    let (names, description) = line
        .split_once(DASH_MARK)
        .or_else(|| line.split_once(" - "))
        .or_else(|| line.split_once(" -- "))?;
    let names: Vec<String> = names
        .split(',')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    let description = description
        .replace(DASH_MARK, "-")
        .trim()
        .trim_start_matches('-')
        .trim()
        .to_owned();
    (!names.is_empty() && !description.is_empty()).then_some(ManName { names, description })
}

/// Entries of a `whatis` file: `name(section), ... - description`.
#[must_use]
pub fn parse_whatis(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((names, description)) = line.split_once(" - ") else {
            continue;
        };
        for item in names.split(',') {
            let item = item.trim();
            let Some((name, section)) = item.split_once('(') else {
                continue;
            };
            let section = section.trim_end_matches(')').trim();
            out.push((
                name.trim().to_owned(),
                section.to_owned(),
                description.trim().to_owned(),
            ));
        }
    }
    out
}

/// Target of a `.so` request on the first lines of a page.
fn redirect(page: &str) -> Option<String> {
    page.lines()
        .take(3)
        .find_map(|line| line.strip_prefix(".so"))
        .map(|target| target.trim().to_owned())
        .filter(|target| !target.is_empty())
}

/// Manual directories in priority order, as host paths.
#[must_use]
pub fn man_dirs(env: &HostEnv) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let defaults = default_dirs(env);
    if let Some(list) = env.var_text("MANPATH") {
        for item in list.split(env.list_separator()) {
            if item.is_empty() {
                dirs.extend(defaults.iter().cloned());
            } else {
                dirs.push(PathBuf::from(item));
            }
        }
    } else {
        dirs.extend(defaults);
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()) && is_dir(env, dir));
    dirs
}

fn default_dirs(env: &HostEnv) -> Vec<PathBuf> {
    let id = SourceId::new(SourceId::MAN);
    let mut mandatory = Vec::new();
    let mut maps: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in CONFIG_FILES {
        let Ok(bytes) = read_file(env, &id, Path::new(file)) else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["MANDATORY_MANPATH" | "MANPATH", dir, ..] => mandatory.push(PathBuf::from(dir)),
                ["MANPATH_MAP", bin, man, ..] => {
                    maps.push((PathBuf::from(bin), PathBuf::from(man)))
                }
                _ => {}
            }
        }
    }
    let mut dirs = Vec::new();
    for bin in env.path_dirs() {
        dirs.extend(
            maps.iter()
                .filter(|(dir, _)| *dir == bin)
                .map(|(_, man)| man.clone()),
        );
        if let Some(parent) = bin.parent() {
            dirs.push(parent.join("share/man"));
            dirs.push(parent.join("man"));
        }
    }
    dirs.extend(mandatory);
    dirs.extend(DEFAULT_DIRS.iter().map(PathBuf::from));
    dirs
}

/// Provider of manual pages.
#[derive(Debug, Clone)]
pub struct ManSource {
    id: SourceId,
}

impl ManSource {
    /// Creates the provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::MAN),
        }
    }
}

impl Default for ManSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Page file with its language and section.
struct Page {
    name: String,
    section: String,
    lang: String,
    path: PathBuf,
}

fn pages(env: &HostEnv, dir: &Path) -> Vec<Page> {
    let mut roots: Vec<(String, PathBuf)> = vec![("en".to_owned(), dir.to_path_buf())];
    for lang in env.languages().iter().filter(|lang| *lang != "en") {
        for (name, path) in list_dir(env, dir) {
            if name.split(['_', '.', '@']).next() == Some(lang.as_str()) && is_dir(env, &path) {
                roots.push((lang.clone(), path));
            }
        }
    }
    let mut out = Vec::new();
    for (lang, root) in roots {
        for section in SECTIONS {
            for (file, path) in list_dir(env, &root.join(format!("man{section}"))) {
                let (_, stem) = Compression::of(&file);
                let Some((name, page_section)) = stem.rsplit_once('.') else {
                    continue;
                };
                if page_section.starts_with(section) && !name.is_empty() {
                    out.push(Page {
                        name: name.to_owned(),
                        section: page_section.to_owned(),
                        lang: lang.clone(),
                        path,
                    });
                }
            }
        }
    }
    out
}

/// The relative path of a `.so` redirect with its `.` and `..` components
/// resolved, or `None` when it is absolute or climbs above its start. A
/// redirect such as `man1/../man7/x` stays in the manual directory.
fn normalized_below(target: &str) -> Option<PathBuf> {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for part in Path::new(target).components() {
        match part {
            Component::Normal(name) => parts.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.into_iter().collect())
}

/// Decoded text of the start of a page, following one `.so` redirect.
fn read_page(env: &HostEnv, id: &SourceId, path: &Path, dir: &Path) -> Result<String, SourceError> {
    let text = read_page_file(env, id, path)?;
    match redirect(&text) {
        Some(target) => {
            let Some(target) = normalized_below(&target) else {
                return Err(SourceError::new(
                    id,
                    SourceErrorKind::Corrupt,
                    "page redirect leaves the manual directory",
                )
                .at(path.display().to_string()));
            };
            let target = target
                .to_str()
                .and_then(|text| text.strip_suffix(".gz"))
                .map_or(target.clone(), PathBuf::from);
            let base = dir.join(&target);
            let candidates = [
                base.clone(),
                PathBuf::from(format!("{}.gz", base.display())),
            ];
            candidates
                .iter()
                .find_map(|candidate| read_page_file(env, id, candidate).ok())
                .ok_or_else(|| {
                    SourceError::new(id, SourceErrorKind::Unavailable, "broken page redirect")
                })
        }
        None => Ok(text),
    }
}

fn read_page_file(env: &HostEnv, id: &SourceId, path: &Path) -> Result<String, SourceError> {
    let file = std::fs::File::open(env.sys_path(path))
        .map_err(|error| SourceError::io(id, path, &error))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let (compression, _) = Compression::of(name);
    let raw = decompress_reader(file, compression, PAGE_PREFIX_BYTES).map_err(|error| {
        SourceError::new(id, SourceErrorKind::Corrupt, error.to_string())
            .at(path.display().to_string())
    })?;
    Ok(decode(&raw))
}

impl ProgramSource for ManSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Description
    }

    fn detect(&self, env: &HostEnv) -> bool {
        !man_dirs(env).is_empty()
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder =
            FingerprintBuilder::new(env).text("languages", &env.languages().join(","));
        for dir in man_dirs(env) {
            builder = builder.path(dir.join(WHATIS_FILE));
            for section in SECTIONS {
                builder = builder.path(dir.join(format!("man{section}")));
            }
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let mut data = SourceData::default();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        for dir in man_dirs(env) {
            deadline.check(&self.id)?;
            read_whatis(env, &self.id, &dir, &mut data, &mut by_name);
            for page in pages(env, &dir) {
                deadline.check(&self.id)?;
                match read_page(env, &self.id, &page.path, &dir) {
                    Ok(text) => {
                        if let Some(name) = parse_name_section(&text) {
                            let summary = (page.lang, name.description);
                            add(
                                &mut data,
                                &mut by_name,
                                &self.id,
                                &page.name,
                                &page.section,
                                summary,
                                &page.path,
                            );
                        }
                    }
                    Err(error) => data.warnings.push(error.into()),
                }
            }
        }
        Ok(data)
    }

    fn detail(
        &self,
        env: &HostEnv,
        key: &EntryKey,
        want: &DetailWant,
    ) -> Result<Option<Detail>, SourceError> {
        if !want.man {
            return Ok(None);
        }
        let path = PathBuf::from(&key.key);
        let dir = path
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("/"));
        let text = read_page(env, &self.id, &path, dir)?;
        Ok(parse_name_section(&text).map(|name| Detail {
            man: Some(format!(
                "{} - {} ({})",
                name.names.join(", "),
                name.description,
                path.display()
            )),
            ..Detail::default()
        }))
    }
}

fn read_whatis(
    env: &HostEnv,
    id: &SourceId,
    dir: &Path,
    data: &mut SourceData,
    by_name: &mut HashMap<String, usize>,
) {
    let path = dir.join(WHATIS_FILE);
    let Ok(bytes) = read_file(env, id, &path) else {
        return;
    };
    for (name, section, description) in parse_whatis(&decode(&bytes)) {
        if SECTIONS.iter().any(|wanted| section.starts_with(wanted)) {
            add(
                data,
                by_name,
                id,
                &name,
                &section,
                ("en".to_owned(), description),
                &path,
            );
        }
    }
}

fn add(
    data: &mut SourceData,
    by_name: &mut HashMap<String, usize>,
    id: &SourceId,
    name: &str,
    section: &str,
    (lang, description): (String, String),
    path: &Path,
) {
    if let Some(index) = by_name.get(name) {
        if let Some(summary) = data.programs[*index].summary.as_mut()
            && summary.lang.as_deref() != Some(lang.as_str())
            && !summary.translations.contains_key(&lang)
        {
            summary.set(&lang, description);
        }
        return;
    }
    let mut program = RawProgram::new(name);
    program.key = Some(EntryKey::new(id, path.display().to_string()));
    program.summary = Some(LocalizedText::in_lang(description, &lang));
    program
        .category_hints
        .push(CategoryHint::new(CategoryScheme::ManSection, section));
    by_name.insert(name.to_owned(), data.programs.len());
    data.programs.push(program);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn man_name_section_roff_mdoc() {
        let roff = ".TH LS 1\n.SH NAME\nls, dir \\- list \\fBdirectory\\fR contents \\(em short\n.SH SYNOPSIS\n.B ls\n";
        let name = parse_name_section(roff).unwrap();
        assert_eq!(name.names, vec!["ls", "dir"]);
        assert_eq!(name.description, "list directory contents \u{2014} short");
        let mdoc = ".Dd 2024\n.Sh NAME\n.Nm cat\n.Nd concatenate and print files\n.Sh SYNOPSIS\n";
        let name = parse_name_section(mdoc).unwrap();
        assert_eq!(name.names, vec!["cat"]);
        assert_eq!(name.description, "concatenate and print files");
        let polish = ".SH NAZWA\nls \\- wypisuje zawarto\u{15b}\u{107} katalogu\n";
        assert_eq!(
            parse_name_section(polish).unwrap().description,
            "wypisuje zawarto\u{15b}\u{107} katalogu"
        );
    }

    #[test]
    fn escapes_and_requests() {
        assert_eq!(
            strip_escapes("a\\f[CB]b\\fP\\s-1c\\s0\\&d\\e\\*(lqq\\*(rq"),
            "abcd\\\"q\""
        );
        assert_eq!(line_text(".BR gzip (1)").as_deref(), Some("gzip(1)"));
        assert_eq!(line_text(".PP"), None);
        assert_eq!(line_text(".\\\" comment"), None);
        assert_eq!(strip_escapes("x \\\" comment"), "x ");
    }

    #[test]
    fn page_redirect_stays_below_the_manual_directory() {
        let root = tempfile::tempdir().unwrap();
        let man1 = root.path().join("usr/share/man/man1");
        std::fs::create_dir_all(&man1).unwrap();
        std::fs::write(man1.join("up.1"), ".so ../../../../secret.1\n").unwrap();
        std::fs::write(man1.join("abs.1"), ".so /secret.1\n").unwrap();
        std::fs::write(man1.join("ok.1"), ".so man1/target.1\n").unwrap();
        std::fs::write(man1.join("around.1"), ".so man1/../man1/target.1\n").unwrap();
        std::fs::write(man1.join("target.1"), ".SH NAME\ntarget \\- real page\n").unwrap();
        std::fs::write(
            root.path().join("secret.1"),
            ".SH NAME\nsecret \\- outside the directory\n",
        )
        .unwrap();
        let env = HostEnv::new(nurt_contracts::env::Os::Linux).with_root(root.path());
        let id = SourceId::new(SourceId::MAN);
        let dir = Path::new("/usr/share/man");
        for page in ["up.1", "abs.1"] {
            let error = read_page(&env, &id, &dir.join("man1").join(page), dir).unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Corrupt, "{page}");
        }
        for page in ["ok.1", "around.1"] {
            let text = read_page(&env, &id, &dir.join("man1").join(page), dir).unwrap();
            assert!(text.contains("real page"), "{page}");
        }
        assert!(normalized_below("a/../../x").is_none());
        assert!(normalized_below("/x").is_none());
        assert_eq!(
            normalized_below("man1/../man7/./x").unwrap(),
            Path::new("man7/x")
        );
    }

    #[test]
    fn name_line_without_marker_and_whatis() {
        assert_eq!(
            split_name_line("foo - does foo").unwrap().description,
            "does foo"
        );
        assert!(split_name_line("no separator").is_none());
        let entries = parse_whatis("ls(1), dir(1) - list directory\nbad line\nx (8) - admin\n");
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[2],
            ("x".to_owned(), "8".to_owned(), "admin".to_owned())
        );
        assert_eq!(
            redirect(".so man1/other.1\n").as_deref(),
            Some("man1/other.1")
        );
    }
}
