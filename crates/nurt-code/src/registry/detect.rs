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
        .filter_map(|marker| {
            line.match_indices(marker)
                .find(|(at, _)| line[..*at].chars().next_back().is_none_or(char::is_whitespace))
                .map(|(at, _)| at + marker.len())
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shebang_with_a_carriage_return_or_spaces_names_the_interpreter() {
        assert_eq!(
            interpreter_of_shebang(b"#!/bin/bash\r\necho\r\n").as_deref(),
            Some("bash")
        );
        assert_eq!(
            interpreter_of_shebang(b"#!  /usr/bin/python3.12  -u\n").as_deref(),
            Some("python")
        );
        assert_eq!(
            interpreter_of_shebang(b"#!/usr/bin/env -S node --flag\n").as_deref(),
            Some("node")
        );
    }

    #[test]
    fn shebang_without_an_interpreter_or_with_invalid_text_names_nothing() {
        for source in [
            &b""[..],
            b"#!",
            b"#!\n",
            b"#!/usr/bin/env\n",
            b"#!/usr/bin/env -S\n",
            b"#!/usr/bin/env FOO=1\n",
            b"#!\xff\xfe\n",
            b"# !/bin/sh\n",
        ] {
            assert_eq!(interpreter_of_shebang(source), None, "{source:?}");
        }
    }

    #[test]
    fn modeline_is_read_from_the_first_and_the_last_lines_only() {
        let near_head = b"1\n2\n3\n4\n# vim: set ft=ruby:\n6\n";
        assert_eq!(modeline_language(near_head).as_deref(), Some("ruby"));
        let too_deep = b"1\n2\n3\n4\n5\n# vim: ft=ruby\n7\n8\n9\n10\n11\n12\n";
        assert_eq!(modeline_language(too_deep), None);
        let tail = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n# vim: ft=go\n";
        assert_eq!(modeline_language(tail).as_deref(), Some("go"));
    }

    #[test]
    fn vim_marker_is_a_separate_word() {
        assert_eq!(modeline_language(b"index: syntax=ruby\n"), None);
        assert_eq!(modeline_language(b"# convex: ft=go\n"), None);
        assert_eq!(modeline_language(b"# invim: ft=go\n"), None);
        assert_eq!(modeline_language(b"ex: ft=go\n").as_deref(), Some("go"));
        assert_eq!(modeline_language(b"/* ex: ft=go */\n").as_deref(), Some("go"));
        assert_eq!(modeline_language(b"# vi: ft=lua\n").as_deref(), Some("lua"));
        assert_eq!(
            modeline_language(b"# index: x vim: ft=go\n").as_deref(),
            Some("go")
        );
    }

    #[test]
    fn emacs_modelines_name_the_mode() {
        assert_eq!(
            modeline_language(b"# -*- mode: Python; coding: utf-8 -*-\n").as_deref(),
            Some("python")
        );
        assert_eq!(
            modeline_language(b"// -*- C++ -*-\n").as_deref(),
            Some("c++")
        );
        assert_eq!(modeline_language(b"# -*- coding: utf-8 -*-\n"), None);
        assert_eq!(modeline_language(b"# -*- unterminated\n"), None);
    }

    #[test]
    fn modeline_survives_invalid_utf8_empty_values_and_empty_text() {
        assert_eq!(
            modeline_language(b"\xff\xfe\n# vim: ft=go\n").as_deref(),
            Some("go")
        );
        assert_eq!(modeline_language(b"# vim: ft=\n"), None);
        assert_eq!(modeline_language(b""), None);
    }

    #[test]
    fn file_name_and_extension_decide_before_the_shebang() {
        let registry = Registry::builtin();
        let by = |path: Option<&str>, bytes: &[u8]| {
            detect_language(registry, path.map(Path::new), bytes).map(|id| id.to_string())
        };
        assert_eq!(
            by(Some("/p/tool.py"), b"#!/bin/bash\n").as_deref(),
            Some("python")
        );
        assert_eq!(by(Some("/p/MAIN.RS"), b"").as_deref(), Some("rust"));
        assert_eq!(by(Some("/p/.bashrc"), b"").as_deref(), Some("bash"));
        assert_eq!(by(Some("/"), b"#!/bin/bash\n").as_deref(), Some("bash"));
        assert_eq!(by(Some("/p/data.unknownext"), b""), None);
        assert_eq!(by(None, b"#!/usr/bin/env ruby\n").as_deref(), Some("ruby"));
    }
}
