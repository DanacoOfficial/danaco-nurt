// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Credential settings of one remote address: `credential.helper`,
//! `credential.username` and `credential.useHttpPath`, including the
//! `credential.<url>.*` subsections matched as `git` matches them.

use std::path::Path;

use crate::http::urlmatch;

/// How a configured credential helper is served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HelperKind {
    /// `store`: a plain-text file of addresses with credentials; `file` is
    /// the value of `--file`.
    Store { file: Option<String> },
    /// `cache`: the daemon of `git credential-cache`; `socket` is the value
    /// of `--socket`.
    Cache { socket: Option<String> },
    /// `osxkeychain`: internet passwords of the macOS keychain.
    Keychain,
    /// `manager`, `manager-core` or `wincred`.
    CredentialManager,
    /// `libsecret`: the Secret Service of the desktop session.
    SecretService,
    /// Any other helper, run as a program of the user.
    Program(ProgramHelper),
}

/// Command line of a helper run as a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProgramHelper {
    /// A program with arguments; the action is appended as last argument.
    Command { program: String, args: Vec<String> },
    /// A shell snippet (`!…`), run as `sh -c '<snippet> "$@"' <snippet> <action>`.
    Script(String),
}

impl ProgramHelper {
    /// Name of the program for reports, without arguments.
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Command { program, .. } => program,
            Self::Script(_) => "sh",
        }
    }
}

/// One `credential.helper` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Helper {
    /// The configured value.
    pub(crate) definition: String,
    /// How the entry is served.
    pub(crate) kind: HelperKind,
}

impl Helper {
    /// Name of the helper for reports: the first word of the definition.
    pub(crate) fn label(&self) -> String {
        match &self.kind {
            HelperKind::Program(ProgramHelper::Script(_)) => "!".to_owned(),
            _ => self
                .definition
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

/// Credential settings applying to one address.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CredentialConfig {
    /// Helpers in configuration order, after resets by empty values.
    pub(crate) helpers: Vec<Helper>,
    /// `credential.username`.
    pub(crate) username: Option<String>,
    /// `credential.useHttpPath`.
    pub(crate) use_http_path: bool,
}

/// Prefix of helper programs named by a short name.
const PROGRAM_PREFIX: &str = "git-credential-";

impl CredentialConfig {
    /// Reads the settings applying to `url`.
    pub(crate) fn resolve(config: &gix::config::File, url: &str) -> Self {
        let helpers = urlmatch::all_values(config, "credential", "helper", url)
            .into_iter()
            .map(|definition| Helper {
                kind: classify(&definition),
                definition,
            })
            .collect();
        let username = urlmatch::best_value(config, "credential", "username", url)
            .filter(|name| !name.is_empty());
        let use_http_path = urlmatch::best_value(config, "credential", "useHttpPath", url)
            .is_some_and(|value| is_true(&value));
        Self {
            helpers,
            username,
            use_http_path,
        }
    }
}

fn is_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "true" | "yes" | "on" | "1"
    )
}

/// Classifies a `credential.helper` value.
pub(crate) fn classify(definition: &str) -> HelperKind {
    let definition = definition.trim();
    if let Some(script) = definition.strip_prefix('!') {
        return HelperKind::Program(ProgramHelper::Script(script.to_owned()));
    }
    let Some(mut words) = split_words(definition) else {
        return HelperKind::Program(ProgramHelper::Script(definition.to_owned()));
    };
    if words.is_empty() {
        return HelperKind::Program(ProgramHelper::Script(definition.to_owned()));
    }
    let first = words.remove(0);
    let absolute = Path::new(&first).is_absolute();
    let name = if absolute {
        Path::new(&first)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_prefix(PROGRAM_PREFIX))
            .unwrap_or_default()
            .to_owned()
    } else {
        first.clone()
    };
    match name.as_str() {
        "store" => HelperKind::Store {
            file: option_value(&words, "--file"),
        },
        "cache" => HelperKind::Cache {
            socket: option_value(&words, "--socket"),
        },
        "osxkeychain" => HelperKind::Keychain,
        "manager" | "manager-core" | "wincred" => HelperKind::CredentialManager,
        "libsecret" => HelperKind::SecretService,
        _ => HelperKind::Program(ProgramHelper::Command {
            program: if absolute {
                first
            } else {
                format!("{PROGRAM_PREFIX}{first}")
            },
            args: words,
        }),
    }
}

/// Value of `--name=value` or `--name value` among `words`.
fn option_value(words: &[String], name: &str) -> Option<String> {
    let mut iter = words.iter();
    while let Some(word) = iter.next() {
        if word == name {
            return iter.next().cloned();
        }
        if let Some(value) = word
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Some(value.to_owned());
        }
    }
    None
}

/// Splits `text` into words by the quoting rules of the POSIX shell (single
/// quotes, double quotes, backslash); `None` for an unterminated quote.
pub(crate) fn split_words(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        other => current.push(other),
                    }
                }
            }
            '"' => {
                in_word = true;
                read_double_quoted(&mut chars, &mut current)?;
            }
            '\\' => {
                in_word = true;
                current.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    Some(words)
}

fn read_double_quoted(chars: &mut std::str::Chars<'_>, out: &mut String) -> Option<()> {
    loop {
        match chars.next()? {
            '"' => return Some(()),
            '\\' => {
                let next = chars.next()?;
                if !matches!(next, '"' | '\\' | '$' | '`') {
                    out.push('\\');
                }
                out.push(next);
            }
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(config: &str, url: &str) -> CredentialConfig {
        let config: gix::config::File = config.parse().expect("valid configuration");
        CredentialConfig::resolve(&config, url)
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn classifies_known_helpers_by_name_and_path() {
        assert_eq!(classify("store"), HelperKind::Store { file: None });
        assert_eq!(
            classify("store --file ~/.creds"),
            HelperKind::Store {
                file: Some("~/.creds".to_owned())
            }
        );
        assert_eq!(
            classify("cache --timeout=60 --socket=/run/s"),
            HelperKind::Cache {
                socket: Some("/run/s".to_owned())
            }
        );
        assert_eq!(classify("osxkeychain"), HelperKind::Keychain);
        assert_eq!(classify("manager-core"), HelperKind::CredentialManager);
        assert_eq!(classify("wincred"), HelperKind::CredentialManager);
        assert_eq!(
            classify("/usr/lib/git-core/git-credential-libsecret"),
            HelperKind::SecretService
        );
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn classifies_other_helpers_as_programs() {
        assert_eq!(
            classify("!gh auth git-credential"),
            HelperKind::Program(ProgramHelper::Script("gh auth git-credential".to_owned()))
        );
        assert_eq!(
            classify("netrc -f 'my file'"),
            HelperKind::Program(ProgramHelper::Command {
                program: "git-credential-netrc".to_owned(),
                args: vec!["-f".to_owned(), "my file".to_owned()],
            })
        );
        assert_eq!(
            classify("/opt/bin/helper --x"),
            HelperKind::Program(ProgramHelper::Command {
                program: "/opt/bin/helper".to_owned(),
                args: vec!["--x".to_owned()],
            })
        );
    }

    #[test]
    fn splits_words_with_shell_quoting() {
        assert_eq!(
            split_words(r#"a 'b c' "d \"e\" \x" f\ g"#),
            Some(vec![
                "a".to_owned(),
                "b c".to_owned(),
                r#"d "e" \x"#.to_owned(),
                "f g".to_owned()
            ])
        );
        assert_eq!(split_words("''"), Some(vec![String::new()]));
        assert_eq!(split_words("'open"), None);
    }

    #[test]
    fn collects_helpers_of_matching_sections_with_resets() {
        let config = "[credential]\n\thelper = cache\n\thelper =\n\thelper = store\n\
                      [credential \"https://example.com\"]\n\thelper = osxkeychain\n\tusername = bob\n\
                      [credential \"https://other.com\"]\n\thelper = libsecret\n";
        let settings = resolve(config, "https://example.com/r.git");
        let kinds: Vec<_> = settings.helpers.iter().map(|h| h.kind.clone()).collect();
        assert_eq!(
            kinds,
            [HelperKind::Store { file: None }, HelperKind::Keychain]
        );
        assert_eq!(settings.username.as_deref(), Some("bob"));
        assert!(!settings.use_http_path);
        let other = resolve(config, "https://other.com/r.git");
        assert_eq!(other.helpers.len(), 2);
        assert_eq!(other.username, None);
    }

    #[test]
    fn reads_use_http_path() {
        let settings = resolve("[credential]\n\tuseHttpPath = true\n", "https://h/r");
        assert!(settings.use_http_path);
    }

    #[test]
    fn labels_helpers_without_arguments() {
        let helper = Helper {
            definition: "store --file /x".to_owned(),
            kind: classify("store --file /x"),
        };
        assert_eq!(helper.label(), "store");
        let script = Helper {
            definition: "!f() { echo; }; f".to_owned(),
            kind: classify("!f() { echo; }; f"),
        };
        assert_eq!(script.label(), "!");
    }
}
