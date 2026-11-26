// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resolution of path literals to canonical absolute paths.
//!
//! A bound path is absolute and canonical: forward slashes, no empty, `.` or
//! `..` segment and no trailing `/` except at the root. The admissible roots
//! are `/`, a drive (`C:/`), a share (`//server/share`) and a provider
//! (`registry:/`). A relative path is resolved only against the explicit
//! call parameter `base`; `~` stands for the home directory of the context.

use nurt_contracts::spec::Os;

/// Providers whose paths exist only on Windows.
const WINDOWS_PROVIDERS: [&str; 1] = ["registry"];

/// Shortest name of a provider; a single letter is a drive.
const MIN_PROVIDER_LEN: usize = 2;

/// Reason why a path literal cannot be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PathProblem {
    /// The path is empty.
    Empty,
    /// The path is relative and no `base` is given.
    Relative,
    /// The path starts with `~` and no home directory is known.
    NoHome,
    /// The path is written in a form foreign to the host system.
    Foreign(&'static str),
}

/// Host facts used to resolve paths.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PathHost<'a> {
    pub(crate) os: Os,
    pub(crate) base: Option<&'a str>,
    pub(crate) home: Option<&'a str>,
}

/// Root of an absolute path and the part after it.
struct Rooted<'t> {
    root: String,
    rest: &'t str,
}

/// Resolves a path literal to its canonical absolute form.
pub(crate) fn resolve(text: &str, host: PathHost<'_>) -> Result<String, PathProblem> {
    if text.is_empty() {
        return Err(PathProblem::Empty);
    }
    let text = separators(text, host.os)?;
    if let Some(rooted) = rooted(&text, host.os)? {
        return Ok(build(&rooted.root, &[], rooted.rest));
    }
    let (start, rest) = match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            (host.home.ok_or(PathProblem::NoHome)?, rest)
        }
        _ => (host.base.ok_or(PathProblem::Relative)?, text.as_str()),
    };
    let start = separators(start, host.os)?;
    let anchor = rooted(&start, host.os)?.ok_or(PathProblem::Relative)?;
    Ok(build(&anchor.root, &[anchor.rest], rest))
}

fn separators(text: &str, os: Os) -> Result<String, PathProblem> {
    if os == Os::Windows {
        Ok(text.replace('\\', "/"))
    } else if text.contains('\\') {
        Err(PathProblem::Foreign(
            "a backslash separator is foreign to this system; write `/`",
        ))
    } else {
        Ok(text.to_owned())
    }
}

/// Splits an absolute path into its root and the rest; `None` for a relative
/// path.
fn rooted(text: &str, os: Os) -> Result<Option<Rooted<'_>>, PathProblem> {
    if let Some(unc) = text.strip_prefix("//") {
        if os != Os::Windows {
            return Ok(Some(Rooted {
                root: "/".to_owned(),
                rest: unc,
            }));
        }
        return share(unc).map(Some);
    }
    if let Some(rest) = text.strip_prefix('/') {
        return Ok(Some(Rooted {
            root: "/".to_owned(),
            rest,
        }));
    }
    let Some((prefix, rest)) = text.split_once(':') else {
        return Ok(None);
    };
    if is_drive(prefix) {
        return drive(prefix, rest, os);
    }
    if is_provider(prefix) {
        return provider(prefix, rest, os);
    }
    Ok(None)
}

fn share(unc: &str) -> Result<Rooted<'_>, PathProblem> {
    let mut parts = unc.splitn(3, '/');
    let server = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if server.is_empty() || name.is_empty() {
        return Err(PathProblem::Foreign(
            "a share path needs a server and a share name: //server/share",
        ));
    }
    Ok(Rooted {
        root: format!("//{server}/{name}"),
        rest: parts.next().unwrap_or_default(),
    })
}

fn drive<'t>(prefix: &str, rest: &'t str, os: Os) -> Result<Option<Rooted<'t>>, PathProblem> {
    if os != Os::Windows {
        return Err(PathProblem::Foreign(
            "a drive letter is foreign to this system; write an absolute path that starts with `/`",
        ));
    }
    let Some(rest) = rest.strip_prefix('/') else {
        if rest.is_empty() {
            return Ok(Some(Rooted {
                root: format!("{}:/", prefix.to_ascii_uppercase()),
                rest,
            }));
        }
        return Err(PathProblem::Relative);
    };
    Ok(Some(Rooted {
        root: format!("{}:/", prefix.to_ascii_uppercase()),
        rest,
    }))
}

fn provider<'t>(prefix: &str, rest: &'t str, os: Os) -> Result<Option<Rooted<'t>>, PathProblem> {
    if WINDOWS_PROVIDERS.contains(&prefix) && os != Os::Windows {
        return Err(PathProblem::Foreign("the registry exists only on Windows"));
    }
    match rest.strip_prefix('/') {
        Some(rest) => Ok(Some(Rooted {
            root: format!("{prefix}:/"),
            rest,
        })),
        None => Ok(None),
    }
}

fn is_drive(prefix: &str) -> bool {
    prefix.len() == 1 && prefix.bytes().all(|byte| byte.is_ascii_alphabetic())
}

fn is_provider(prefix: &str) -> bool {
    prefix.len() >= MIN_PROVIDER_LEN
        && prefix
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && prefix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Joins the root with the segments of every part, dropping empty and `.`
/// segments and applying `..`, which never climbs above the root.
fn build(root: &str, anchors: &[&str], rest: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for part in anchors.iter().copied().chain(std::iter::once(rest)) {
        for segment in part.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    segments.pop();
                }
                other => segments.push(other),
            }
        }
    }
    let joined = segments.join("/");
    if root.ends_with('/') {
        format!("{root}{joined}")
    } else if joined.is_empty() {
        root.to_owned()
    } else {
        format!("{root}/{joined}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux() -> PathHost<'static> {
        PathHost {
            os: Os::Linux,
            base: None,
            home: None,
        }
    }

    fn windows() -> PathHost<'static> {
        PathHost {
            os: Os::Windows,
            base: None,
            home: None,
        }
    }

    #[test]
    fn keeps_a_canonical_absolute_path() {
        assert_eq!(resolve("/srv/app", linux()).unwrap(), "/srv/app");
        assert_eq!(resolve("/", linux()).unwrap(), "/");
        assert_eq!(
            resolve("/srv/data/my file.txt", linux()).unwrap(),
            "/srv/data/my file.txt"
        );
    }

    #[test]
    fn removes_empty_dot_and_trailing_segments() {
        assert_eq!(
            resolve("/srv//app/./src/", linux()).unwrap(),
            "/srv/app/src"
        );
        assert_eq!(resolve("/srv/app/../lib", linux()).unwrap(), "/srv/lib");
        assert_eq!(resolve("/..", linux()).unwrap(), "/");
    }

    #[test]
    fn rejects_a_relative_path_without_base() {
        assert_eq!(resolve("src/app", linux()), Err(PathProblem::Relative));
        assert_eq!(resolve("", linux()), Err(PathProblem::Empty));
    }

    #[test]
    fn resolves_a_relative_path_against_base() {
        let host = PathHost {
            base: Some("/srv/app"),
            ..linux()
        };
        assert_eq!(
            resolve("src/main.go", host).unwrap(),
            "/srv/app/src/main.go"
        );
        assert_eq!(resolve("../lib", host).unwrap(), "/srv/lib");
        assert_eq!(resolve(".", host).unwrap(), "/srv/app");
    }

    #[test]
    fn rejects_a_relative_base() {
        let host = PathHost {
            base: Some("srv"),
            ..linux()
        };
        assert_eq!(resolve("a", host), Err(PathProblem::Relative));
    }

    #[test]
    fn expands_the_home_directory() {
        let host = PathHost {
            home: Some("/home/user"),
            ..linux()
        };
        assert_eq!(
            resolve("~/notes.txt", host).unwrap(),
            "/home/user/notes.txt"
        );
        assert_eq!(resolve("~", host).unwrap(), "/home/user");
        assert_eq!(resolve("~/x", linux()), Err(PathProblem::NoHome));
    }

    #[test]
    fn rejects_windows_forms_on_linux() {
        assert!(matches!(
            resolve("C:/proj", linux()),
            Err(PathProblem::Foreign(_))
        ));
        assert!(matches!(
            resolve("/srv\\app", linux()),
            Err(PathProblem::Foreign(_))
        ));
        assert!(matches!(
            resolve("registry:/HKCU/Software", linux()),
            Err(PathProblem::Foreign(_))
        ));
    }

    #[test]
    fn canonicalizes_windows_paths_on_windows() {
        assert_eq!(
            resolve("c:\\proj\\src\\", windows()).unwrap(),
            "C:/proj/src"
        );
        assert_eq!(resolve("C:/", windows()).unwrap(), "C:/");
        assert_eq!(resolve("C:", windows()).unwrap(), "C:/");
        assert_eq!(resolve("C:proj", windows()), Err(PathProblem::Relative));
        assert_eq!(
            resolve("//server/share/a/", windows()).unwrap(),
            "//server/share/a"
        );
        assert_eq!(
            resolve("//server/share", windows()).unwrap(),
            "//server/share"
        );
        assert!(matches!(
            resolve("//server", windows()),
            Err(PathProblem::Foreign(_))
        ));
        assert_eq!(
            resolve("registry:/HKCU/Software/Example", windows()).unwrap(),
            "registry:/HKCU/Software/Example"
        );
    }

    #[test]
    fn collapses_a_double_slash_on_linux() {
        assert_eq!(resolve("//srv/app", linux()).unwrap(), "/srv/app");
    }

    #[test]
    fn keeps_multi_byte_segments_and_applies_dot_dot() {
        assert_eq!(resolve("/srv/é/../日本語/./ü", linux()).unwrap(), "/srv/日本語/ü");
        let host = PathHost {
            base: Some("/srv/ünï"),
            home: Some("/home/ż"),
            ..linux()
        };
        assert_eq!(resolve("ß/../ä", host).unwrap(), "/srv/ünï/ä");
        assert_eq!(resolve("~/日本", host).unwrap(), "/home/ż/日本");
        assert_eq!(resolve("../../../é", host).unwrap(), "/é");
    }

    #[test]
    fn does_not_read_a_multi_byte_prefix_as_a_drive_or_a_provider() {
        let host = PathHost {
            base: Some("/srv"),
            ..linux()
        };
        assert_eq!(resolve("é:/x", host).unwrap(), "/srv/é:/x");
        assert_eq!(resolve("日本:x", host).unwrap(), "/srv/日本:x");
        assert_eq!(resolve("~é/x", host).unwrap(), "/srv/~é/x");
    }

    #[test]
    fn rejects_an_empty_share_part_and_a_drive_without_a_separator() {
        assert!(matches!(
            resolve("//server/", windows()),
            Err(PathProblem::Foreign(_))
        ));
        assert_eq!(resolve("C:é", windows()), Err(PathProblem::Relative));
    }
}
