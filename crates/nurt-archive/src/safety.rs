// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Containment of archive members in the destination directory.
//!
//! Every member path and link target is checked lexically, before anything
//! is written. A member is rejected when its path could resolve outside the
//! destination on any of the supported systems: a `..` component, an
//! absolute path, a drive letter, a `\` separator, a path that passes
//! through a symbolic link created by the same archive, a symbolic link
//! whose target leaves the destination, or a hard link to a file that is
//! not extracted from the archive.
//!
//! The checks do not inspect the destination on disk; links that already
//! exist there are handled when the files are written.

use std::collections::HashSet;

use crate::list::{Member, MemberKind};

/// Reason why a member cannot be extracted inside the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Violation {
    /// A path component is `..`, or consists only of dots and spaces,
    /// which Windows reads as `..`.
    ParentComponent,
    /// The path starts with `/`.
    Absolute,
    /// The path starts with a drive letter such as `C:`.
    DriveLetter,
    /// The path contains `\`, a separator on Windows.
    Backslash,
    /// The name is empty, contains NUL or is not valid UTF-8.
    InvalidName,
    /// A symbolic link points outside the destination, is absolute, or its
    /// target leaves and re-enters a directory (`a/../b`).
    LinkOutside,
    /// The path passes through a symbolic link created by the archive.
    ThroughLink,
    /// A hard link refers to a file that is not extracted before it from
    /// the same archive.
    HardLinkOutside,
}

impl Violation {
    /// Stable name used in listings of skipped members (`parent_component`,
    /// `absolute`, `drive_letter`, `backslash`, `invalid_name`,
    /// `link_outside`, `through_link`, `hard_link_outside`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::ParentComponent => "parent_component",
            Self::Absolute => "absolute",
            Self::DriveLetter => "drive_letter",
            Self::Backslash => "backslash",
            Self::InvalidName => "invalid_name",
            Self::LinkOutside => "link_outside",
            Self::ThroughLink => "through_link",
            Self::HardLinkOutside => "hard_link_outside",
        }
    }

    /// Whether `allow_outside: true` lets the member be extracted anyway;
    /// a name that cannot be represented is never extracted.
    #[must_use]
    pub fn is_waivable(self) -> bool {
        self != Self::InvalidName
    }

    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::ParentComponent => "it contains a `..` component",
            Self::Absolute => "it is absolute",
            Self::DriveLetter => "it starts with a drive letter",
            Self::Backslash => "it contains `\\`",
            Self::InvalidName => "it is empty, contains NUL or is not valid UTF-8",
            Self::LinkOutside => "the link target leaves the destination",
            Self::ThroughLink => "it passes through a symbolic link of the archive",
            Self::HardLinkOutside => "the hard link refers to a file outside the archive",
        }
    }
}

/// Splits a relative archive path into its components, dropping `.` and
/// empty components.
///
/// # Errors
///
/// Returns the first [`Violation`] found in the path.
pub(crate) fn components(path: &str) -> Result<Vec<&str>, Violation> {
    if path.contains('\0') {
        return Err(Violation::InvalidName);
    }
    if path.contains('\\') {
        return Err(Violation::Backslash);
    }
    if path.starts_with('/') {
        return Err(Violation::Absolute);
    }
    if has_drive_letter(path) {
        return Err(Violation::DriveLetter);
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.iter().any(|part| is_parent(part)) {
        return Err(Violation::ParentComponent);
    }
    Ok(parts)
}

fn has_drive_letter(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// `..` and the names Windows reduces to it by dropping trailing dots and
/// spaces.
fn is_parent(part: &str) -> bool {
    part != "." && part.contains("..") && part.chars().all(|c| c == '.' || c == ' ')
}

/// Key under which two paths name the same file on a case-insensitive
/// volume that ignores trailing dots and spaces.
fn alias_key(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim_end_matches(['.', ' ']).to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

/// Selection and rewriting of member paths during extraction.
#[derive(Clone, Copy, Default)]
pub struct Rules<'f> {
    /// Selects members by their archive path; `None` selects every member.
    pub only: Option<&'f dyn Fn(&str) -> bool>,
    /// Number of leading path components removed from every member path
    /// and hard link target.
    pub strip_components: usize,
    /// Extract members that would otherwise be rejected, except
    /// [`Violation::InvalidName`].
    pub allow_outside: bool,
}

impl std::fmt::Debug for Rules<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Rules")
            .field("only", &self.only.is_some())
            .field("strip_components", &self.strip_components)
            .field("allow_outside", &self.allow_outside)
            .finish()
    }
}

/// Reason why a member is not extracted although it is not rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// The member does not match `only`.
    NotSelected,
    /// `strip_components` removes every component of the path.
    Stripped,
    /// The path names the destination itself, for example `./`.
    Root,
    /// The member is a device, FIFO or another special file.
    Special,
}

/// Outcome of checking one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Extract {
        dest: String,
        hard_link_to: Option<String>,
        waived: Option<Violation>,
    },
    Skip(SkipReason),
    Reject(Violation),
}

/// Checks members in archive order and remembers the links they create.
pub(crate) struct Guard<'f> {
    rules: Rules<'f>,
    symlinks: HashSet<String>,
    files: HashSet<String>,
}

impl<'f> Guard<'f> {
    pub(crate) fn new(rules: Rules<'f>) -> Self {
        Self {
            rules,
            symlinks: HashSet::new(),
            files: HashSet::new(),
        }
    }

    /// Whether `dest` is a regular file accepted so far and not replaced
    /// by a later member.
    pub(crate) fn is_file(&self, dest: &str) -> bool {
        self.files.contains(dest)
    }

    pub(crate) fn check(&mut self, member: &Member) -> Verdict {
        if self.rules.only.is_some_and(|only| !only(&member.path)) {
            return Verdict::Skip(SkipReason::NotSelected);
        }
        if member.kind == MemberKind::Other {
            return Verdict::Skip(SkipReason::Special);
        }
        if !member.name_is_exact() || member.path.is_empty() {
            return Verdict::Reject(Violation::InvalidName);
        }
        let parts = match components(&member.path) {
            Ok(parts) if parts.is_empty() => return Verdict::Skip(SkipReason::Root),
            Ok(parts) => parts,
            Err(violation) => return self.waive(member, violation),
        };
        let Some(kept) = parts
            .get(self.rules.strip_components..)
            .filter(|kept| !kept.is_empty())
        else {
            return Verdict::Skip(SkipReason::Stripped);
        };
        let dest = kept.join("/");
        if self.passes_through_link(kept) {
            return self.waive(member, Violation::ThroughLink);
        }
        let mut hard_link_to = None;
        match member.kind {
            MemberKind::Symlink => {
                if let Err(violation) = check_symlink(kept, member.link_target.as_deref()) {
                    return self.waive(member, violation);
                }
            }
            MemberKind::HardLink => match self.hard_link_target(member.link_target.as_deref()) {
                Ok(target) => hard_link_to = Some(target),
                Err(violation) => return self.waive(member, violation),
            },
            MemberKind::File | MemberKind::Directory | MemberKind::Other => {}
        }
        self.record(member.kind, kept, &dest);
        Verdict::Extract {
            dest,
            hard_link_to,
            waived: None,
        }
    }

    fn passes_through_link(&self, parts: &[&str]) -> bool {
        (1..parts.len()).any(|end| self.symlinks.contains(&alias_key(&parts[..end])))
    }

    fn hard_link_target(&self, target: Option<&str>) -> Result<String, Violation> {
        let target = target.filter(|target| !target.is_empty());
        let parts = target
            .map(components)
            .ok_or(Violation::InvalidName)?
            .map_err(|_| Violation::HardLinkOutside)?;
        let kept = parts
            .get(self.rules.strip_components..)
            .filter(|kept| !kept.is_empty())
            .ok_or(Violation::HardLinkOutside)?;
        let dest = kept.join("/");
        if self.files.contains(&dest) {
            Ok(dest)
        } else {
            Err(Violation::HardLinkOutside)
        }
    }

    fn record(&mut self, kind: MemberKind, parts: &[&str], dest: &str) {
        self.files.remove(dest);
        match kind {
            MemberKind::File | MemberKind::HardLink => {
                self.files.insert(dest.to_owned());
            }
            MemberKind::Symlink => {
                self.symlinks.insert(alias_key(parts));
            }
            MemberKind::Directory | MemberKind::Other => {}
        }
    }

    /// Rejects the member, or with `allow_outside` extracts it at the path
    /// as stored, without `.` and empty components.
    fn waive(&mut self, member: &Member, violation: Violation) -> Verdict {
        if !self.rules.allow_outside || !violation.is_waivable() {
            return Verdict::Reject(violation);
        }
        let absolute = member.path.starts_with('/');
        let parts: Vec<&str> = member
            .path
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect();
        let Some(kept) = parts
            .get(self.rules.strip_components..)
            .filter(|kept| !kept.is_empty())
        else {
            return Verdict::Skip(SkipReason::Stripped);
        };
        let joined = kept.join("/");
        let dest = if absolute {
            format!("/{joined}")
        } else {
            joined
        };
        let hard_link_to = (member.kind == MemberKind::HardLink)
            .then(|| member.link_target.clone())
            .flatten();
        self.record(member.kind, kept, &dest);
        Verdict::Extract {
            dest,
            hard_link_to,
            waived: Some(violation),
        }
    }
}

/// Resolves the target of a symbolic link at `link` (destination
/// components) lexically.
///
/// A target may climb with leading `..` components and then descend; a
/// `..` after a descending component is rejected because the component may
/// be a link whose physical parent differs from its lexical parent.
fn check_symlink(link: &[&str], target: Option<&str>) -> Result<(), Violation> {
    let target = target
        .filter(|target| !target.is_empty() && !target.contains('\0'))
        .ok_or(Violation::InvalidName)?;
    if target.starts_with('/') || target.contains('\\') || has_drive_letter(target) {
        return Err(Violation::LinkOutside);
    }
    let mut depth = link.len().saturating_sub(1);
    let mut descended = false;
    for part in target
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        if is_parent(part) {
            if descended || depth == 0 {
                return Err(Violation::LinkOutside);
            }
            depth -= 1;
        } else {
            descended = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    fn member(path: &str, kind: MemberKind, target: Option<&str>) -> Member {
        let mut member = Member::new(Cow::Borrowed(path.as_bytes()), kind);
        if let Some(target) = target {
            member.set_link_target(Cow::Borrowed(target.as_bytes()));
        }
        member
    }

    fn file(path: &str) -> Member {
        member(path, MemberKind::File, None)
    }

    fn extracted(dest: &str) -> Verdict {
        Verdict::Extract {
            dest: dest.to_owned(),
            hard_link_to: None,
            waived: None,
        }
    }

    #[test]
    fn rejects_escaping_member_paths() {
        let cases = [
            ("../evil", Violation::ParentComponent),
            ("a/../../evil", Violation::ParentComponent),
            ("a/.. /evil", Violation::ParentComponent),
            ("a/.../evil", Violation::ParentComponent),
            ("/etc/passwd", Violation::Absolute),
            ("C:/Windows/evil", Violation::DriveLetter),
            ("c:evil", Violation::DriveLetter),
            ("..\\evil", Violation::Backslash),
            ("a\\b", Violation::Backslash),
            ("a\0b", Violation::InvalidName),
        ];
        let mut guard = Guard::new(Rules::default());
        for (path, violation) in cases {
            assert_eq!(
                guard.check(&file(path)),
                Verdict::Reject(violation),
                "{path}"
            );
        }
    }

    #[test]
    fn accepts_dot_components_and_normalises_them() {
        let mut guard = Guard::new(Rules::default());
        assert_eq!(guard.check(&file("./a//b/./c.txt")), extracted("a/b/c.txt"));
        assert_eq!(guard.check(&file("a/..b/c")), extracted("a/..b/c"));
        assert_eq!(
            guard.check(&member("./", MemberKind::Directory, None)),
            Verdict::Skip(SkipReason::Root)
        );
    }

    #[test]
    fn strips_leading_components() {
        let rules = Rules {
            strip_components: 1,
            ..Rules::default()
        };
        let mut guard = Guard::new(rules);
        assert_eq!(
            guard.check(&file("pkg-1.0/src/lib.rs")),
            extracted("src/lib.rs")
        );
        assert_eq!(
            guard.check(&member("pkg-1.0", MemberKind::Directory, None)),
            Verdict::Skip(SkipReason::Stripped)
        );
    }

    #[test]
    fn selects_members_with_only() {
        let only = |path: &str| path.ends_with(".rs");
        let rules = Rules {
            only: Some(&only),
            ..Rules::default()
        };
        let mut guard = Guard::new(rules);
        assert_eq!(guard.check(&file("src/lib.rs")), extracted("src/lib.rs"));
        assert_eq!(
            guard.check(&file("README.md")),
            Verdict::Skip(SkipReason::NotSelected)
        );
        assert_eq!(
            guard.check(&file("../x.rs")),
            Verdict::Reject(Violation::ParentComponent)
        );
    }

    #[test]
    fn checks_symbolic_link_targets() {
        let cases = [
            ("lib/current", "v2/lib.so", None),
            ("lib/current", "../share/x", None),
            ("a/b/c", "../../d", None),
            ("link", "..", Some(Violation::LinkOutside)),
            ("a/b", "../../etc", Some(Violation::LinkOutside)),
            ("link", "/etc/passwd", Some(Violation::LinkOutside)),
            ("link", "C:/Windows", Some(Violation::LinkOutside)),
            ("link", "..\\..\\x", Some(Violation::LinkOutside)),
            ("a/link", "b/../../x", Some(Violation::LinkOutside)),
            ("link", "", Some(Violation::InvalidName)),
        ];
        for (path, target, violation) in cases {
            let mut guard = Guard::new(Rules::default());
            let verdict = guard.check(&member(path, MemberKind::Symlink, Some(target)));
            match violation {
                None => assert_eq!(verdict, extracted(path), "{path} -> {target}"),
                Some(violation) => {
                    assert_eq!(verdict, Verdict::Reject(violation), "{path} -> {target}");
                }
            }
        }
    }

    #[test]
    fn rejects_paths_through_archive_links() {
        let mut guard = Guard::new(Rules::default());
        assert_eq!(
            guard.check(&member("dir", MemberKind::Symlink, Some("."))),
            extracted("dir")
        );
        assert_eq!(
            guard.check(&member("dir/up", MemberKind::Symlink, Some("../x"))),
            Verdict::Reject(Violation::ThroughLink)
        );
        assert_eq!(
            guard.check(&file("DIR./file")),
            Verdict::Reject(Violation::ThroughLink)
        );
        assert_eq!(guard.check(&file("other/file")), extracted("other/file"));
    }

    #[test]
    fn hard_links_must_refer_to_earlier_files() {
        let mut guard = Guard::new(Rules::default());
        assert_eq!(guard.check(&file("data/a")), extracted("data/a"));
        assert_eq!(
            guard.check(&member("data/b", MemberKind::HardLink, Some("data/a"))),
            Verdict::Extract {
                dest: "data/b".to_owned(),
                hard_link_to: Some("data/a".to_owned()),
                waived: None,
            }
        );
        for target in ["/etc/shadow", "../outside", "data/missing", "data"] {
            assert_eq!(
                guard.check(&member("data/c", MemberKind::HardLink, Some(target))),
                Verdict::Reject(Violation::HardLinkOutside),
                "{target}"
            );
        }
        assert!(guard.is_file("data/b"));
    }

    #[test]
    fn hard_link_targets_are_stripped_like_paths() {
        let rules = Rules {
            strip_components: 1,
            ..Rules::default()
        };
        let mut guard = Guard::new(rules);
        guard.check(&file("top/a"));
        assert_eq!(
            guard.check(&member("top/b", MemberKind::HardLink, Some("top/a"))),
            Verdict::Extract {
                dest: "b".to_owned(),
                hard_link_to: Some("a".to_owned()),
                waived: None,
            }
        );
        assert_eq!(
            guard.check(&member("top/c", MemberKind::HardLink, Some("top"))),
            Verdict::Reject(Violation::HardLinkOutside)
        );
    }

    #[test]
    fn allow_outside_waives_violations_except_invalid_names() {
        let rules = Rules {
            allow_outside: true,
            ..Rules::default()
        };
        let mut guard = Guard::new(rules);
        assert_eq!(
            guard.check(&file("../up/x")),
            Verdict::Extract {
                dest: "../up/x".to_owned(),
                hard_link_to: None,
                waived: Some(Violation::ParentComponent),
            }
        );
        assert_eq!(
            guard.check(&file("/abs/./x")),
            Verdict::Extract {
                dest: "/abs/x".to_owned(),
                hard_link_to: None,
                waived: Some(Violation::Absolute),
            }
        );
        assert_eq!(
            guard.check(&file("a\0b")),
            Verdict::Reject(Violation::InvalidName)
        );
    }

    #[test]
    fn skips_special_files() {
        let mut guard = Guard::new(Rules::default());
        assert_eq!(
            guard.check(&member("dev/null", MemberKind::Other, None)),
            Verdict::Skip(SkipReason::Special)
        );
    }

    #[test]
    fn violation_names_are_distinct() {
        let all = [
            Violation::ParentComponent,
            Violation::Absolute,
            Violation::DriveLetter,
            Violation::Backslash,
            Violation::InvalidName,
            Violation::LinkOutside,
            Violation::ThroughLink,
            Violation::HardLinkOutside,
        ];
        let names: HashSet<&str> = all.iter().map(|violation| violation.name()).collect();
        assert_eq!(names.len(), all.len());
        assert!(all.iter().all(|violation| !violation.describe().is_empty()));
    }
}
