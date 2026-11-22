// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Version comparison of the package managers.

use std::cmp::Ordering;

use nurt_contracts::record::PackageManager;

/// Ordering rules of a package manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionScheme {
    /// Debian policy on epoch, upstream version and revision.
    Dpkg,
    /// The `rpmvercmp` rules on epoch, version and release.
    Rpm,
    /// The `vercmp` rules of pacman on epoch, `pkgver` and `pkgrel`.
    Pacman,
    /// A version followed by the revision suffix `_<revision>`.
    Brew,
    /// Numeric and alphabetic segments, for Scoop, Chocolatey and winget.
    Generic,
}

impl VersionScheme {
    /// Scheme used by `manager`.
    #[must_use]
    pub fn of(manager: PackageManager) -> Self {
        match manager {
            PackageManager::Apt | PackageManager::Dpkg => Self::Dpkg,
            PackageManager::Dnf | PackageManager::Zypper | PackageManager::Rpm => Self::Rpm,
            PackageManager::Pacman => Self::Pacman,
            PackageManager::Brew => Self::Brew,
            _ => Self::Generic,
        }
    }

    /// Compares two versions written in this scheme.
    #[must_use]
    pub fn compare(self, left: &str, right: &str) -> Ordering {
        match self {
            Self::Dpkg => compare_dpkg(left, right),
            Self::Rpm => compare_rpm(left, right),
            Self::Pacman => compare_pacman(left, right),
            Self::Brew => compare_brew(left, right),
            Self::Generic => compare_generic(left, right),
        }
    }
}

fn split_epoch(version: &str) -> (&str, &str) {
    match version.split_once(':') {
        Some((epoch, rest)) if !epoch.is_empty() && epoch.bytes().all(|b| b.is_ascii_digit()) => {
            (epoch, rest)
        }
        _ => ("0", version),
    }
}

fn split_release(version: &str) -> (&str, Option<&str>) {
    match version.rsplit_once('-') {
        Some((upstream, release)) => (upstream, Some(release)),
        None => (version, None),
    }
}

fn compare_numbers(left: impl AsRef<[u8]>, right: impl AsRef<[u8]>) -> Ordering {
    let strip = |digits: &[u8]| {
        let start = digits.iter().position(|b| *b != b'0').unwrap_or(digits.len());
        digits[start..].to_vec()
    };
    let (left, right) = (strip(left.as_ref()), strip(right.as_ref()));
    left.len().cmp(&right.len()).then_with(|| left.cmp(&right))
}

fn dpkg_order(byte: Option<u8>) -> i32 {
    match byte {
        None => 0,
        Some(b) if b.is_ascii_digit() => 0,
        Some(b) if b.is_ascii_alphabetic() => i32::from(b),
        Some(b'~') => -1,
        Some(b) => i32::from(b) + 256,
    }
}

fn dpkg_part(left: &str, right: &str) -> Ordering {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let (mut i, mut j) = (0, 0);
    let digit = |s: &[u8], at: usize| s.get(at).is_some_and(u8::is_ascii_digit);
    while i < left.len() || j < right.len() {
        while (i < left.len() && !digit(left, i)) || (j < right.len() && !digit(right, j)) {
            let order = dpkg_order(left.get(i).copied()).cmp(&dpkg_order(right.get(j).copied()));
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
        while left.get(i) == Some(&b'0') {
            i += 1;
        }
        while right.get(j) == Some(&b'0') {
            j += 1;
        }
        let mut first_difference = Ordering::Equal;
        while digit(left, i) && digit(right, j) {
            if first_difference == Ordering::Equal {
                first_difference = left[i].cmp(&right[j]);
            }
            i += 1;
            j += 1;
        }
        if digit(left, i) {
            return Ordering::Greater;
        }
        if digit(right, j) {
            return Ordering::Less;
        }
        if first_difference != Ordering::Equal {
            return first_difference;
        }
    }
    Ordering::Equal
}

fn compare_dpkg(left: &str, right: &str) -> Ordering {
    let (left_epoch, left) = split_epoch(left);
    let (right_epoch, right) = split_epoch(right);
    let epoch = compare_numbers(left_epoch, right_epoch);
    if epoch != Ordering::Equal {
        return epoch;
    }
    let (left_upstream, left_revision) = split_release(left);
    let (right_upstream, right_revision) = split_release(right);
    dpkg_part(left_upstream, right_upstream)
        .then_with(|| dpkg_part(left_revision.unwrap_or(""), right_revision.unwrap_or("")))
}

/// Handles the `~` and `^` markers of `rpmvercmp`.
///
/// Returns `None` when neither side starts with a marker, `Ok` when both sides
/// have the same marker and the comparison goes on, and `Err` with the result
/// when the markers decide it.
fn rpm_marker(a: Option<u8>, b: Option<u8>) -> Option<Result<(), Ordering>> {
    if a == Some(b'~') || b == Some(b'~') {
        return Some(match (a == Some(b'~'), b == Some(b'~')) {
            (true, true) => Ok(()),
            (false, _) => Err(Ordering::Greater),
            (true, false) => Err(Ordering::Less),
        });
    }
    if a == Some(b'^') || b == Some(b'^') {
        return Some(match (a, b) {
            (None, _) => Err(Ordering::Less),
            (_, None) => Err(Ordering::Greater),
            (Some(b'^'), Some(b'^')) => Ok(()),
            (Some(b'^'), _) => Err(Ordering::Less),
            _ => Err(Ordering::Greater),
        });
    }
    None
}

fn rpm_segment_compare(left: &str, right: &str) -> Ordering {
    if left == right {
        return Ordering::Equal;
    }
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let (mut i, mut j) = (0, 0);
    let skippable = |b: u8| !b.is_ascii_alphanumeric() && b != b'~' && b != b'^';
    while i < left.len() || j < right.len() {
        while i < left.len() && skippable(left[i]) {
            i += 1;
        }
        while j < right.len() && skippable(right[j]) {
            j += 1;
        }
        let (a, b) = (left.get(i).copied(), right.get(j).copied());
        match rpm_marker(a, b) {
            Some(Ok(())) => {
                i += 1;
                j += 1;
                continue;
            }
            Some(Err(order)) => return order,
            None => {}
        }
        let (Some(first), Some(_)) = (a, b) else {
            break;
        };
        let numeric = first.is_ascii_digit();
        let class = |byte: u8| {
            if numeric {
                byte.is_ascii_digit()
            } else {
                byte.is_ascii_alphabetic()
            }
        };
        let (start_left, start_right) = (i, j);
        while i < left.len() && class(left[i]) {
            i += 1;
        }
        while j < right.len() && class(right[j]) {
            j += 1;
        }
        if start_right == j {
            return if numeric {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        let segment_left = &left[start_left..i];
        let segment_right = &right[start_right..j];
        let order = if numeric {
            compare_numbers(segment_left, segment_right)
        } else {
            segment_left.cmp(segment_right)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    match (i < left.len(), j < right.len()) {
        (false, false) => Ordering::Equal,
        (true, _) => Ordering::Greater,
        (false, true) => Ordering::Less,
    }
}

fn compare_rpm(left: &str, right: &str) -> Ordering {
    let (left_epoch, left) = split_epoch(left);
    let (right_epoch, right) = split_epoch(right);
    let epoch = compare_numbers(left_epoch, right_epoch);
    if epoch != Ordering::Equal {
        return epoch;
    }
    let (left_version, left_release) = split_release(left);
    let (right_version, right_release) = split_release(right);
    rpm_segment_compare(left_version, right_version).then_with(|| {
        match (left_release, right_release) {
            (Some(a), Some(b)) => rpm_segment_compare(a, b),
            _ => Ordering::Equal,
        }
    })
}

fn pacman_segment_compare(left: &str, right: &str) -> Ordering {
    if left == right {
        return Ordering::Equal;
    }
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        let (mark_left, mark_right) = (i, j);
        while i < left.len() && !left[i].is_ascii_alphanumeric() {
            i += 1;
        }
        while j < right.len() && !right[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if i == left.len() || j == right.len() {
            break;
        }
        if i - mark_left != j - mark_right {
            return (i - mark_left).cmp(&(j - mark_right));
        }
        let numeric = left[i].is_ascii_digit();
        let class = |byte: u8| {
            if numeric {
                byte.is_ascii_digit()
            } else {
                byte.is_ascii_alphabetic()
            }
        };
        let (start_left, start_right) = (i, j);
        while i < left.len() && class(left[i]) {
            i += 1;
        }
        while j < right.len() && class(right[j]) {
            j += 1;
        }
        if start_right == j {
            return if numeric {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        let segment_left = &left[start_left..i];
        let segment_right = &right[start_right..j];
        let order = if numeric {
            compare_numbers(segment_left, segment_right)
        } else {
            segment_left.cmp(segment_right)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    let (rest_left, rest_right) = (left.get(i).copied(), right.get(j).copied());
    if rest_left.is_none() && rest_right.is_none() {
        return Ordering::Equal;
    }
    let alpha = |byte: Option<u8>| byte.is_some_and(|b| b.is_ascii_alphabetic());
    if (rest_left.is_none() && !alpha(rest_right)) || alpha(rest_left) {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

fn compare_pacman(left: &str, right: &str) -> Ordering {
    let (left_epoch, left) = split_epoch(left);
    let (right_epoch, right) = split_epoch(right);
    let epoch = compare_numbers(left_epoch, right_epoch);
    if epoch != Ordering::Equal {
        return epoch;
    }
    let (left_version, left_release) = split_release(left);
    let (right_version, right_release) = split_release(right);
    pacman_segment_compare(left_version, right_version).then_with(|| {
        match (left_release, right_release) {
            (Some(a), Some(b)) => pacman_segment_compare(a, b),
            _ => Ordering::Equal,
        }
    })
}

fn split_brew_revision(version: &str) -> (&str, u64) {
    if let Some((base, revision)) = version.rsplit_once('_')
        && let Ok(revision) = revision.parse::<u64>()
    {
        return (base, revision);
    }
    (version, 0)
}

fn compare_brew(left: &str, right: &str) -> Ordering {
    let (left_base, left_revision) = split_brew_revision(left);
    let (right_base, right_revision) = split_brew_revision(right);
    compare_generic(left_base, right_base).then(left_revision.cmp(&right_revision))
}

fn generic_segments(version: &str) -> Vec<(bool, &str)> {
    let version = version.strip_prefix(['v', 'V']).unwrap_or(version);
    let bytes = version.as_bytes();
    let mut segments = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let numeric = bytes[i].is_ascii_digit();
        let start = i;
        while i < bytes.len()
            && bytes[i].is_ascii_alphanumeric()
            && bytes[i].is_ascii_digit() == numeric
        {
            i += 1;
        }
        segments.push((numeric, &version[start..i]));
    }
    segments
}

fn compare_generic(left: &str, right: &str) -> Ordering {
    let (left, right) = (generic_segments(left), generic_segments(right));
    for index in 0..left.len().max(right.len()) {
        let order = match (left.get(index), right.get(index)) {
            (Some((true, a)), Some((true, b))) => compare_numbers(a, b),
            (Some((false, a)), Some((false, b))) => a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()),
            (Some((true, _)), Some((false, _))) => Ordering::Greater,
            (Some((false, _)), Some((true, _))) => Ordering::Less,
            (Some((true, number)), None) => {
                if number.trim_start_matches('0').is_empty() {
                    Ordering::Equal
                } else {
                    Ordering::Greater
                }
            }
            (Some((false, _)), None) => Ordering::Less,
            (None, Some((true, number))) => {
                if number.trim_start_matches('0').is_empty() {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            }
            (None, Some((false, _))) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ordering::{Equal, Greater, Less};

    fn check(scheme: VersionScheme, vectors: &[(&str, &str, Ordering)]) {
        for (left, right, expected) in vectors {
            assert_eq!(
                scheme.compare(left, right),
                *expected,
                "{left} against {right}"
            );
            assert_eq!(
                scheme.compare(right, left),
                expected.reverse(),
                "{right} against {left}"
            );
        }
    }

    #[test]
    fn version_cmp_dpkg_vectors() {
        check(
            VersionScheme::Dpkg,
            &[
                ("1.0", "1.0", Equal),
                ("1.0", "1.1", Less),
                ("1.0~rc1", "1.0", Less),
                ("1.0~rc1", "1.0~rc2", Less),
                ("1.0~~", "1.0~", Less),
                ("1.0~", "1.0", Less),
                ("1.0", "1.0+b1", Less),
                ("1.0+b1", "1.0+b2", Less),
                ("1.0a", "1.0", Greater),
                ("1.0a", "1.0+", Less),
                ("1:0.5", "2.0", Greater),
                ("1:1.0", "1:1.0", Equal),
                ("1:1.0", "2:0.1", Less),
                ("0:1.0", "1.0", Equal),
                ("1.0-1", "1.0-2", Less),
                ("1.0-1", "1.0", Greater),
                ("1.0-1ubuntu1", "1.0-1", Greater),
                ("1.0-1ubuntu1", "1.0-1ubuntu2", Less),
                ("1.0-1~bpo12+1", "1.0-1", Less),
                ("1.0-1+deb12u1", "1.0-1", Greater),
                ("1.10", "1.9", Greater),
                ("1.02", "1.2", Equal),
                ("1.0.0", "1.0", Greater),
                ("2.0-0", "2.0", Greater),
                ("2.30.2-1ubuntu1.1", "2.30.2-1ubuntu1", Greater),
                ("9.4p1-5", "9.4-5", Greater),
                ("1.2.3~beta-1", "1.2.3-1", Less),
                ("3:1.0~rc1-1", "3:1.0-1", Less),
                ("1.0-1", "1.0-01", Equal),
                ("a", "1", Greater),
                ("1a", "1.a", Less),
                ("0~1", "0", Less),
                ("5.15.0-91.101", "5.15.0-91.99", Greater),
                ("12.0", "12.0~", Greater),
            ],
        );
    }

    #[test]
    fn version_cmp_rpm_vectors() {
        check(
            VersionScheme::Rpm,
            &[
                ("1.0-1", "1.0-1", Equal),
                ("1.0-1", "1.0-2", Less),
                ("1.0-1.fc40", "1.0-1.fc41", Less),
                ("1.0~rc1-1", "1.0-1", Less),
                ("1.0^git1-1", "1.0-1", Greater),
                ("1.0^git1-1", "1.0.1-1", Less),
                ("1.0~rc1^git1-1", "1.0~rc1-1", Greater),
                ("1:1.0-1", "2.0-1", Greater),
                ("0:1.0-1", "1.0-1", Equal),
                ("1.10-1", "1.9-1", Greater),
                ("1.0a-1", "1.0-1", Greater),
                ("1.0-1", "1.0", Equal),
                ("2.0.1-1", "2.0-5", Greater),
                ("1.0-1.el9_3", "1.0-1.el9", Greater),
                ("1.0_1-1", "1.0.1-1", Equal),
                ("1.0-1.1", "1.0-1", Greater),
                ("10-1", "9-1", Greater),
                ("1.0-1", "1.0-01", Equal),
                ("1.0a-1", "1.0.1-1", Less),
            ],
        );
    }

    #[test]
    fn version_cmp_pacman_vectors() {
        check(
            VersionScheme::Pacman,
            &[
                ("1.0-1", "1.0-1", Equal),
                ("1.0-1", "1.0-2", Less),
                ("1.0-2", "1.1-1", Less),
                ("1:1.0-1", "2.0-1", Greater),
                ("0:1.0-1", "1.0-1", Equal),
                ("1.0.1-1", "1.0-1", Greater),
                ("1.0a-1", "1.0-1", Less),
                ("1.0-1", "1.0.0-1", Less),
                ("1.0rc1-1", "1.0-1", Less),
                ("1.0beta-1", "1.0alpha-1", Greater),
                ("1.0-1", "1.0-1.1", Less),
                ("1.10-1", "1.9-1", Greater),
                ("2.0.r5.gabcdef-1", "2.0-1", Less),
                ("1.0+1-1", "1.0-1", Greater),
                ("1.0_1-1", "1.0.1-1", Less),
            ],
        );
    }

    #[test]
    fn version_cmp_brew_revision_and_generic_segments() {
        check(
            VersionScheme::Brew,
            &[
                ("1.2.3", "1.2.3_1", Less),
                ("1.2.3_2", "1.2.3_10", Less),
                ("1.2.4", "1.2.3_9", Greater),
                ("1.0", "1.0", Equal),
            ],
        );
        check(
            VersionScheme::Generic,
            &[
                ("1.0", "1.0.0", Equal),
                ("1.0.1", "1.0", Greater),
                ("1.0-beta", "1.0", Less),
                ("v2.0", "1.9", Greater),
                ("1.10", "1.9", Greater),
                ("1.0-alpha", "1.0-beta", Less),
                ("1.0a", "1.0.1", Less),
            ],
        );
    }

    #[test]
    fn scheme_follows_manager() {
        assert_eq!(VersionScheme::of(PackageManager::Apt), VersionScheme::Dpkg);
        assert_eq!(VersionScheme::of(PackageManager::Zypper), VersionScheme::Rpm);
        assert_eq!(VersionScheme::of(PackageManager::Pacman), VersionScheme::Pacman);
        assert_eq!(VersionScheme::of(PackageManager::Brew), VersionScheme::Brew);
        assert_eq!(VersionScheme::of(PackageManager::Scoop), VersionScheme::Generic);
    }
}
