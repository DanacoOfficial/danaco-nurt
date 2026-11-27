// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::powershell::convert
// Responsibility:   Conversion of PowerShell literals: paths, wildcard patterns, numbers and.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conversion of PowerShell literals: paths, wildcard patterns, numbers and
//! durations.

use super::table::Registry;

/// Milliseconds of one second.
pub(crate) const MS_PER_SECOND: u64 = 1_000;

/// Milliseconds of the duration units of Nurt.
const DURATION_UNITS: [(&str, u64); 5] = [
    ("ms", 1),
    ("s", MS_PER_SECOND),
    ("m", 60 * MS_PER_SECOND),
    ("h", 3_600 * MS_PER_SECOND),
    ("d", 86_400 * MS_PER_SECOND),
];

/// Characters that make a PowerShell path a wildcard pattern.
const WILDCARD_CHARS: [char; 3] = ['*', '?', '['];

/// A path converted to Nurt notation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathText {
    /// The path as Nurt writes it.
    pub(crate) text: String,
    /// Whether the notation differs from the written path.
    pub(crate) changed: bool,
    /// Whether the path is a registry path.
    pub(crate) registry: bool,
}

/// Why a written path has no Nurt form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PathProblem {
    /// A relative path without the call parameter `base`.
    Relative,
    /// The home directory `~`.
    Home,
    /// A path of the environment drive, `env:NAME`.
    Environment(Option<String>),
    /// A variable inside the path.
    Variable,
}

/// Converts a written path: `\` becomes `/`, a drive letter is upper case,
/// doubled and trailing separators are removed and registry drives become
/// `registry:/HIVE/…`. The case of path segments never changes.
pub(crate) fn path(
    written: &str,
    registry: &Registry,
    base: Option<&str>,
) -> Result<PathText, PathProblem> {
    if written.contains('$') {
        return Err(PathProblem::Variable);
    }
    if written == "~" || written.starts_with("~/") || written.starts_with("~\\") {
        return Err(PathProblem::Home);
    }
    if let Some(converted) = registry_path(written, registry) {
        return Ok(converted);
    }
    if let Some((drive, rest)) = written.split_once(':')
        && drive.eq_ignore_ascii_case("env")
    {
        let name = rest.trim_start_matches(['\\', '/']);
        return Err(PathProblem::Environment(
            (!name.is_empty()).then(|| name.to_owned()),
        ));
    }
    let windows = is_drive_path(written) || written.starts_with("\\\\");
    let text = if windows {
        let mut chars: Vec<char> = written.replace('\\', "/").chars().collect();
        if is_drive_path(written)
            && let Some(letter) = chars.first_mut()
        {
            *letter = letter.to_ascii_uppercase();
        }
        collapse(&chars.into_iter().collect::<String>())
    } else if written.starts_with('/') {
        collapse(written)
    } else if base.is_some() {
        collapse(&written.replace('\\', "/"))
    } else {
        return Err(PathProblem::Relative);
    };
    Ok(PathText {
        changed: text != written,
        text,
        registry: false,
    })
}

/// Whether the text is written in Windows notation: `C:\…` or a UNC path
/// `\\server\share`.
pub(crate) fn is_windows_path(written: &str) -> bool {
    written.starts_with("\\\\") || (is_drive_path(written) && written.contains('\\'))
}

/// `C:\…` or `C:/…`; a bare `C:` is relative to the current directory of
/// the drive and is not absolute.
fn is_drive_path(written: &str) -> bool {
    let bytes = written.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// Removes doubled separators (keeping the leading `//` of a UNC path) and a
/// trailing separator other than the root.
fn collapse(text: &str) -> String {
    let (prefix, rest) = match text.strip_prefix("//") {
        Some(rest) => ("//", rest),
        None => ("", text),
    };
    let mut out = String::from(prefix);
    let mut previous_slash = false;
    for c in rest.chars() {
        if c == '/' && previous_slash {
            continue;
        }
        previous_slash = c == '/';
        out.push(c);
    }
    let is_root = out == "/" || (out.len() == 3 && out.ends_with(":/"));
    if out.ends_with('/') && !is_root && out.len() > prefix.len() {
        out.pop();
    }
    out
}

fn registry_path(written: &str, registry: &Registry) -> Option<PathText> {
    let (hive, rest) = match strip_prefix_ignore_case(written, &registry.provider) {
        Some(rest) => {
            let (first, tail) = rest.split_once(['\\', '/']).unwrap_or((rest, ""));
            (registry.hive(first)?, tail)
        }
        None => {
            let (drive, tail) = written.split_once(':')?;
            (registry.hive(drive)?, tail)
        }
    };
    let rest = collapse(&rest.replace('\\', "/"));
    let rest = rest.trim_start_matches('/');
    let text = if rest.is_empty() {
        format!("{}/{hive}", registry.scheme)
    } else {
        format!("{}/{hive}/{rest}", registry.scheme)
    };
    Some(PathText {
        changed: true,
        text,
        registry: true,
    })
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..))
        .flatten()
}

/// Whether the text is a PowerShell wildcard pattern.
pub(crate) fn has_wildcard(text: &str) -> bool {
    text.contains(WILDCARD_CHARS)
}

/// Splits a wildcard path into its directory and the pattern of the last
/// segment; `None` when a directory segment has a wildcard or the path has
/// no directory.
pub(crate) fn split_wildcard(written: &str) -> Option<(&str, &str)> {
    let index = written.rfind(['\\', '/'])?;
    let (directory, pattern) = (written.get(..index)?, written.get(index + 1..)?);
    let directory = if directory.is_empty() || directory.ends_with(':') {
        written.get(..=index)?
    } else {
        directory
    };
    (!has_wildcard(directory) && !pattern.is_empty()).then_some((directory, pattern))
}

/// Translates a `-like` wildcard pattern into an anchored regular
/// expression with the same meaning; a backtick escapes the next character.
pub(crate) fn wildcard_regex(pattern: &str) -> String {
    let mut out = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '[' => {
                let mut class = String::from("[");
                let mut closed = false;
                for inner in chars.by_ref() {
                    if inner == ']' {
                        closed = true;
                        break;
                    }
                    if matches!(inner, '\\' | '[' | '&' | '~') {
                        class.push('\\');
                    }
                    class.push(inner);
                }
                if closed {
                    class.push(']');
                    out.push_str(&class);
                } else {
                    out.push_str(&regex::escape(&format!("[{}", &class[1..])));
                }
            }
            '`' => {
                if let Some(escaped) = chars.next() {
                    out.push_str(&regex::escape(&escaped.to_string()));
                }
            }
            other => out.push_str(&regex::escape(&other.to_string())),
        }
    }
    out.push('$');
    simplify_anchors(out)
}

/// `^.*x$` matches what `x$` matches, and `^x.*$` what `^x` matches.
fn simplify_anchors(mut regex: String) -> String {
    if regex.starts_with("^.*") {
        regex.replace_range(..3, "");
    }
    if regex.ends_with(".*$") && !regex.ends_with("\\.*$") {
        let len = regex.len();
        regex.replace_range(len - 3.., "");
    }
    regex
}

/// Whether the text contains a letter with distinct upper and lower case.
pub(crate) fn has_cased(text: &str) -> bool {
    text.chars().any(|c| c.is_lowercase() || c.is_uppercase())
}

/// Milliseconds of `count` units of a Nurt duration unit; `None` when the
/// result is negative, not whole or too large.
pub(crate) fn duration_ms(count: f64, unit: &str) -> Option<u64> {
    let factor = DURATION_UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, ms)| *ms)?;
    let total = count * factor as f64;
    (total >= 0.0 && total.fract() == 0.0 && total < u64::MAX as f64)
        .then(|| format!("{total:.0}").parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry {
            scheme: "registry:".to_owned(),
            provider: "Registry::".to_owned(),
            drives: vec![
                ("HKLM".to_owned(), "HKLM".to_owned()),
                ("HKCU".to_owned(), "HKCU".to_owned()),
                ("HKEY_LOCAL_MACHINE".to_owned(), "HKLM".to_owned()),
            ],
        }
    }

    fn converted(written: &str) -> Result<String, PathProblem> {
        path(written, &registry(), None).map(|path| path.text)
    }

    #[test]
    fn windows_paths_use_forward_slashes_and_upper_case_drive() {
        assert_eq!(converted(r"c:\proj\src\"), Ok("C:/proj/src".to_owned()));
        assert_eq!(converted(r"C:\"), Ok("C:/".to_owned()));
        assert_eq!(converted(r"C:\a\\b"), Ok("C:/a/b".to_owned()));
        assert_eq!(
            converted(r"C:\Program Files\App"),
            Ok("C:/Program Files/App".to_owned())
        );
    }

    #[test]
    fn unc_and_posix_paths() {
        assert_eq!(
            converted(r"\\server\share\x"),
            Ok("//server/share/x".to_owned())
        );
        assert_eq!(converted("/srv//app/"), Ok("/srv/app".to_owned()));
        assert_eq!(converted("/"), Ok("/".to_owned()));
    }

    #[test]
    fn registry_drives_and_provider_paths() {
        assert_eq!(
            converted(r"HKLM:\Software\Example"),
            Ok("registry:/HKLM/Software/Example".to_owned())
        );
        assert_eq!(
            converted(r"Registry::HKEY_LOCAL_MACHINE\Software"),
            Ok("registry:/HKLM/Software".to_owned())
        );
        assert_eq!(converted("hkcu:"), Ok("registry:/HKCU".to_owned()));
    }

    #[test]
    fn relative_home_environment_and_variable_paths_are_rejected() {
        assert_eq!(converted(r".\a"), Err(PathProblem::Relative));
        assert_eq!(converted("C:a"), Err(PathProblem::Relative));
        assert_eq!(converted("~/a"), Err(PathProblem::Home));
        assert_eq!(
            converted("env:PATH"),
            Err(PathProblem::Environment(Some("PATH".to_owned())))
        );
        assert_eq!(converted("Env:"), Err(PathProblem::Environment(None)));
        assert_eq!(converted(r"$HOME\a"), Err(PathProblem::Variable));
    }

    #[test]
    fn relative_path_is_kept_with_base() {
        let converted = path(r"src\a.go", &registry(), Some("/srv/app")).map(|path| path.text);
        assert_eq!(converted, Ok("src/a.go".to_owned()));
    }

    #[test]
    fn unchanged_path_is_not_reported() {
        assert_eq!(
            path("/srv/a", &registry(), None).map(|path| path.changed),
            Ok(false)
        );
    }

    #[test]
    fn splits_wildcard_paths() {
        assert_eq!(
            split_wildcard(r"C:\logs\*.log"),
            Some((r"C:\logs", "*.log"))
        );
        assert_eq!(split_wildcard(r"C:\*"), Some((r"C:\", "*")));
        assert_eq!(split_wildcard(r"C:\*\a.log"), None);
        assert_eq!(split_wildcard("*.log"), None);
    }

    #[test]
    fn wildcards_become_anchored_regular_expressions() {
        assert_eq!(wildcard_regex("*.go"), r"\.go$");
        assert_eq!(wildcard_regex("test?*"), "^test.");
        assert_eq!(wildcard_regex("a[bc]d"), "^a[bc]d$");
        assert_eq!(wildcard_regex("a`*b"), r"^a\*b$");
        assert_eq!(wildcard_regex("[ab"), r"^\[ab$");
        assert_eq!(wildcard_regex("x.y"), r"^x\.y$");
    }

    #[test]
    fn detects_cased_letters() {
        assert!(has_cased("a.go"));
        assert!(!has_cased("123-_."));
    }

    #[test]
    fn durations_in_milliseconds() {
        assert_eq!(duration_ms(2.0, "d"), Some(172_800_000));
        assert_eq!(duration_ms(1.5, "s"), Some(1_500));
        assert_eq!(duration_ms(0.5, "ms"), None);
        assert_eq!(duration_ms(-1.0, "s"), None);
        assert_eq!(duration_ms(1.0, "w"), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
