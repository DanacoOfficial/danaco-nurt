// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the language of a file.

use std::path::Path;

use nurt_contracts::services::LanguageId;

use super::Registry;

/// Lines at the start and at the end of a file searched for a modeline, as
/// in Vim (`modelines` defaults to 5).
const MODELINE_LINES: usize = 5;

/// Bytes at the start and at the end of a file searched for a modeline.
const MODELINE_WINDOW: usize = 4096;

/// Language of a file, from the modeline, the file name, the extension and
/// the shebang line, in this order.
///
/// ```
/// use std::path::Path;
/// use nurt_code::registry::{Registry, detect_language};
///
/// let registry = Registry::builtin();
/// let script = b"#!/usr/bin/env python3\nprint(1)\n";
/// let id = detect_language(registry, Some(Path::new("tool")), script);
/// assert_eq!(id.map(|id| id.to_string()), Some("python".to_owned()));
/// ```
#[must_use]
pub fn detect_language(
    registry: &Registry,
    path: Option<&Path>,
    bytes: &[u8],
) -> Option<LanguageId> {
    modeline_language(bytes)
        .and_then(|name| registry.resolve_name(&name))
        .or_else(|| path.and_then(|path| by_name(registry, path)))
        .or_else(|| {
            interpreter_of_shebang(bytes)
                .and_then(|interpreter| by_interpreter(registry, &interpreter))
        })
}

pub(super) fn by_name(registry: &Registry, path: &Path) -> Option<LanguageId> {
    let file_name = path.file_name()?.to_str()?;
    if let Some(spec) = registry
        .specs()
        .find(|spec| spec.file_names.iter().any(|name| name == file_name))
    {
        return Some(spec.id.clone());
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    registry
        .specs()
        .find(|spec| spec.extensions.contains(&extension))
        .map(|spec| spec.id.clone())
}

fn by_interpreter(registry: &Registry, interpreter: &str) -> Option<LanguageId> {
    registry
        .specs()
        .find(|spec| spec.interpreters.iter().any(|name| name == interpreter))
        .map(|spec| spec.id.clone())
}

/// Interpreter named by the shebang line, without directory and version
/// suffix: `#!/usr/bin/env -S python3.12 -u` gives `python`.
///
/// ```
/// use nurt_code::registry::interpreter_of_shebang;
///
/// assert_eq!(interpreter_of_shebang(b"#!/bin/bash -e\n").as_deref(), Some("bash"));
/// assert_eq!(interpreter_of_shebang(b"#!/usr/bin/env node\n").as_deref(), Some("node"));
/// assert_eq!(interpreter_of_shebang(b"print(1)\n"), None);
/// ```
#[must_use]
pub fn interpreter_of_shebang(bytes: &[u8]) -> Option<String> {
    let rest = bytes.strip_prefix(b"#!")?;
    let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
    let line = std::str::from_utf8(&rest[..end]).ok()?.trim();
    let mut words = line.split_whitespace();
    let mut program = base_name(words.next()?);
    if program == "env" {
        program = words
            .find(|word| !word.starts_with('-') && !word.contains('='))
            .map(base_name)?;
    }
    let name = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    (!name.is_empty()).then(|| name.to_owned())
}

fn base_name(program: &str) -> &str {
    program.rsplit(['/', '\\']).next().unwrap_or(program)
}

/// Language named by a Vim or Emacs modeline in the first or last lines.
///
/// Recognized forms: `vim: set ft=python:`, `vi: filetype=ruby`,
/// `ex: syntax=sh`, `-*- mode: python -*-` and `-*- python -*-`.
///
/// ```
/// use nurt_code::registry::modeline_language;
///
/// assert_eq!(modeline_language(b"# vim: set ft=ruby :\n").as_deref(), Some("ruby"));
/// assert_eq!(modeline_language(b"// -*- mode: c++ -*-\n").as_deref(), Some("c++"));
/// assert_eq!(modeline_language(b"plain text\n"), None);
/// ```
#[must_use]
pub fn modeline_language(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(MODELINE_WINDOW)];
    let tail = &bytes[bytes.len().saturating_sub(MODELINE_WINDOW)..];
    let head = String::from_utf8_lossy(head);
    let tail = String::from_utf8_lossy(tail);
    head.lines()
        .take(MODELINE_LINES)
        .chain(tail.lines().rev().take(MODELINE_LINES))
        .find_map(|line| emacs_mode(line).or_else(|| vim_filetype(line)))
}

fn emacs_mode(line: &str) -> Option<String> {
    let start = line.find("-*-")? + "-*-".len();
    let end = start + line[start..].find("-*-")?;
    let body = line[start..end].trim();
    if !body.contains(':') {
        return (!body.is_empty()).then(|| body.to_ascii_lowercase());
    }
    body.split(';').find_map(|setting| {
        let (key, value) = setting.split_once(':')?;
        (key.trim().eq_ignore_ascii_case("mode")).then(|| value.trim().to_ascii_lowercase())
    })
}

fn vim_filetype(line: &str) -> Option<String> {
    let marker = ["vim:", "vi:", "ex:"]
        .iter()
        .filter_map(|marker| line.find(marker).map(|at| at + marker.len()))
        .min()?;
    line[marker..]
        .split(|c: char| c.is_whitespace() || c == ':')
        .find_map(|setting| {
            let (key, value) = setting.split_once('=')?;
            matches!(key, "ft" | "filetype" | "syntax" | "syn")
                .then(|| value.trim().to_ascii_lowercase())
        })
        .filter(|value| !value.is_empty())
}
