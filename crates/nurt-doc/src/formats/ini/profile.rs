// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! INI dialects.

use std::fmt;
use std::str::FromStr;

/// Dialect of an INI document.
///
/// The dialect decides the separators, comment rules, quoting, line
/// continuations, case sensitivity of names and the meaning of a repeated
/// key.
///
/// | Profile | Files | Separators | Trailing comments | Repeated key |
/// |---|---|---|---|---|
/// | `generic` | `.ini`, `.cfg`, `.conf` | `=`, `:` | no | one occurrence per index |
/// | `systemd` | `.service`, `.timer`, `.socket`, `.mount`, `.network`, … | `=` | no | list |
/// | `gitconfig` | `.gitconfig`, `.git/config` | `=` | `#`, `;` | list |
/// | `desktop` | `.desktop` | `=` | no | one occurrence per index |
/// | `php` | `php.ini` | `=` | `;` | one occurrence per index; `key[]` is a list |
/// | `editorconfig` | `.editorconfig` | `=`, `:` | no | one occurrence per index |
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::ini::IniProfile;
///
/// assert_eq!(IniProfile::from_file_name("nginx.service"), Some(IniProfile::Systemd));
/// assert_eq!("gitconfig".parse::<IniProfile>(), Ok(IniProfile::Gitconfig));
/// assert_eq!(IniProfile::default(), IniProfile::Generic);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum IniProfile {
    /// Plain INI: sections, `=` and `:`, full-line comments.
    #[default]
    Generic,
    /// systemd unit and network files.
    Systemd,
    /// Git configuration files.
    Gitconfig,
    /// freedesktop.org desktop entries.
    Desktop,
    /// PHP configuration (`php.ini`).
    Php,
    /// EditorConfig files.
    Editorconfig,
}

/// File name extensions of systemd unit and network files.
const SYSTEMD_EXTENSIONS: [&str; 15] = [
    "service",
    "socket",
    "timer",
    "mount",
    "automount",
    "swap",
    "target",
    "path",
    "slice",
    "scope",
    "device",
    "network",
    "netdev",
    "link",
    "nspawn",
];

/// Repetition rule of a key written more than once in a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Repeat {
    /// Every occurrence is an element of a list.
    List,
    /// Occurrences are distinct; the last one is the value of the key.
    Duplicate,
}

/// How a value continues on the next line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Continuation {
    /// No continuation lines.
    None,
    /// A final `\` joins the next line directly.
    Join,
    /// A final `\` is replaced by a space and the next line is trimmed.
    Space,
}

/// Syntax rules of a profile.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rules {
    pub(crate) separators: &'static [char],
    pub(crate) comment_chars: &'static [char],
    /// Characters that start a trailing comment outside quotes.
    pub(crate) trailing_comment_chars: &'static [char],
    pub(crate) continuation: Continuation,
    pub(crate) repeat: Repeat,
    pub(crate) fold_section_names: bool,
    pub(crate) fold_key_names: bool,
    /// Separator written in new entries when the file has no entry to copy.
    pub(crate) default_separator: &'static str,
    /// Indentation of new entries when the section has no entry to copy.
    pub(crate) default_indent: &'static str,
}

impl IniProfile {
    /// Every profile.
    pub const ALL: [Self; 6] = [
        Self::Generic,
        Self::Systemd,
        Self::Gitconfig,
        Self::Desktop,
        Self::Php,
        Self::Editorconfig,
    ];

    /// Name of the profile as accepted by [`FromStr`].
    pub const fn name(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Systemd => "systemd",
            Self::Gitconfig => "gitconfig",
            Self::Desktop => "desktop",
            Self::Php => "php",
            Self::Editorconfig => "editorconfig",
        }
    }

    /// Profile implied by a file name or path; `None` when the name does not
    /// identify an INI dialect.
    ///
    /// Names ending in `.ini`, `.cfg` and `.conf` select
    /// [`IniProfile::Generic`]; `.conf` files are also used by other formats,
    /// so callers confirm them by content.
    pub fn from_file_name(path: &str) -> Option<Self> {
        let normalized = path.replace('\\', "/");
        let name = normalized.rsplit('/').next().unwrap_or(&normalized);
        let lower = name.to_ascii_lowercase();
        if lower == ".gitconfig" || lower == "gitconfig" || normalized.ends_with(".git/config") {
            return Some(Self::Gitconfig);
        }
        if lower == ".editorconfig" {
            return Some(Self::Editorconfig);
        }
        if lower == "php.ini" || (lower.starts_with("php") && lower.ends_with(".ini")) {
            return Some(Self::Php);
        }
        let extension = lower.rsplit_once('.').map(|(_, e)| e)?;
        if extension == "desktop" {
            Some(Self::Desktop)
        } else if SYSTEMD_EXTENSIONS.contains(&extension) {
            Some(Self::Systemd)
        } else if matches!(extension, "ini" | "cfg" | "conf") {
            Some(Self::Generic)
        } else {
            None
        }
    }

    pub(crate) const fn rules(self) -> Rules {
        match self {
            Self::Generic => Rules {
                separators: &['=', ':'],
                comment_chars: &[';', '#'],
                trailing_comment_chars: &[],
                continuation: Continuation::Join,
                repeat: Repeat::Duplicate,
                fold_section_names: false,
                fold_key_names: false,
                default_separator: " = ",
                default_indent: "",
            },
            Self::Systemd => Rules {
                separators: &['='],
                comment_chars: &['#', ';'],
                trailing_comment_chars: &[],
                continuation: Continuation::Space,
                repeat: Repeat::List,
                fold_section_names: false,
                fold_key_names: false,
                default_separator: "=",
                default_indent: "",
            },
            Self::Gitconfig => Rules {
                separators: &['='],
                comment_chars: &['#', ';'],
                trailing_comment_chars: &['#', ';'],
                continuation: Continuation::Join,
                repeat: Repeat::List,
                fold_section_names: true,
                fold_key_names: true,
                default_separator: " = ",
                default_indent: "\t",
            },
            Self::Desktop => Rules {
                separators: &['='],
                comment_chars: &['#'],
                trailing_comment_chars: &[],
                continuation: Continuation::None,
                repeat: Repeat::Duplicate,
                fold_section_names: false,
                fold_key_names: false,
                default_separator: "=",
                default_indent: "",
            },
            Self::Php => Rules {
                separators: &['='],
                comment_chars: &[';', '#'],
                trailing_comment_chars: &[';'],
                continuation: Continuation::None,
                repeat: Repeat::Duplicate,
                fold_section_names: false,
                fold_key_names: false,
                default_separator: " = ",
                default_indent: "",
            },
            Self::Editorconfig => Rules {
                separators: &['=', ':'],
                comment_chars: &['#', ';'],
                trailing_comment_chars: &[],
                continuation: Continuation::None,
                repeat: Repeat::Duplicate,
                fold_section_names: false,
                fold_key_names: true,
                default_separator: " = ",
                default_indent: "",
            },
        }
    }
}

impl fmt::Display for IniProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Error of [`IniProfile::from_str`] for an unknown profile name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownIniProfile(pub String);

impl fmt::Display for UnknownIniProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown INI profile `{}`; use generic, systemd, gitconfig, desktop, php or editorconfig",
            self.0
        )
    }
}

impl std::error::Error for UnknownIniProfile {}

impl FromStr for IniProfile {
    type Err = UnknownIniProfile;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|profile| profile.name() == s)
            .ok_or_else(|| UnknownIniProfile(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_profiles_from_file_names() {
        let cases = [
            ("a/b/sshd.service", Some(IniProfile::Systemd)),
            ("eth0.network", Some(IniProfile::Systemd)),
            ("backup.timer", Some(IniProfile::Systemd)),
            ("/home/u/.gitconfig", Some(IniProfile::Gitconfig)),
            ("repo/.git/config", Some(IniProfile::Gitconfig)),
            ("repo\\.git\\config", Some(IniProfile::Gitconfig)),
            ("app.desktop", Some(IniProfile::Desktop)),
            ("etc/php.ini", Some(IniProfile::Php)),
            ("php-fpm.ini", Some(IniProfile::Php)),
            (".editorconfig", Some(IniProfile::Editorconfig)),
            ("setup.cfg", Some(IniProfile::Generic)),
            ("my.INI", Some(IniProfile::Generic)),
            ("x.conf", Some(IniProfile::Generic)),
            ("config", None),
            ("a.toml", None),
        ];
        for (name, expected) in cases {
            assert_eq!(IniProfile::from_file_name(name), expected, "{name}");
        }
    }

    #[test]
    fn parses_and_prints_profile_names() {
        for profile in IniProfile::ALL {
            assert_eq!(profile.name().parse::<IniProfile>(), Ok(profile));
            assert_eq!(profile.to_string(), profile.name());
        }
        let error = "toml".parse::<IniProfile>().unwrap_err();
        assert!(error.to_string().starts_with("unknown INI profile `toml`"));
    }
}
