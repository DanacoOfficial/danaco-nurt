// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Extension manifest `nurt-ext.toml`.
//!
//! ```toml
//! [ext]
//! name = "acme"
//! version = "1.2.0"
//! api = "nurt:ext@1.0"
//! kind = "wasm"
//! wasm = "acme.wasm"
//!
//! [[stage]]
//! name = "lint"
//! input = ["entry"]
//! writes = false
//!
//! [[format]]
//! id = "acme-conf"
//! extensions = ["acmeconf"]
//!
//! [[grammar]]
//! language = "acme"
//! extensions = ["acme"]
//! wasm = "grammar/acme.wasm"
//!
//! [permissions]
//! read = ["${project}"]
//! write = []
//! ```
//!
//! [`Manifest::parse`] reads the text; [`Manifest::validate`] checks it
//! against the host: the API version (`nurt:ext@MAJOR.MINOR`, a different
//! major version or a newer minor version is rejected), the names (the
//! extension name is the prefix of its commands and must not be a name of
//! the core), the files named by the manifest (relative, inside the
//! extension directory) and the permissions needed by the declared stages.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path};
use std::str::FromStr;

use nurt_contracts::spec::{ParamSpec, is_identifier};
use nurt_contracts::value::RecordKind;
use serde::{Deserialize, Serialize};

/// Name of the input or output kind that stands for every record kind.
const ANY_KIND: &str = "any";

/// File name of the manifest in an extension directory.
pub const MANIFEST_FILE: &str = "nurt-ext.toml";

/// Interface name of the extension API.
pub const API_NAME: &str = "nurt:ext";

/// Version of the extension API implemented by this host.
pub const HOST_API: ApiVersion = ApiVersion { major: 1, minor: 0 };

/// Longest extension, stage or format name, in bytes.
pub const MAX_NAME_LEN: usize = 64;

/// Version `MAJOR.MINOR` of the extension API.
///
/// ```
/// use nurt_ext::manifest::{ApiVersion, HOST_API};
///
/// let version = ApiVersion::parse("nurt:ext@1.0").unwrap();
/// assert_eq!(version, HOST_API);
/// assert!(HOST_API.accepts(version));
/// assert!(!HOST_API.accepts(ApiVersion::parse("nurt:ext@2.0").unwrap()));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApiVersion {
    /// Incompatible revisions.
    pub major: u32,
    /// Compatible additions.
    pub minor: u32,
}

impl ApiVersion {
    /// Parses `nurt:ext@MAJOR.MINOR`; a patch number (`@1.0.0`) is accepted
    /// and ignored.
    ///
    /// # Errors
    ///
    /// Fails with [`ManifestError::InvalidField`] for another interface name
    /// or a malformed version.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let invalid = || ManifestError::InvalidField {
            field: "ext.api",
            message: format!("`{text}` is not of the form `{API_NAME}@MAJOR.MINOR`"),
        };
        let version = text
            .strip_prefix(API_NAME)
            .and_then(|rest| rest.strip_prefix('@'))
            .ok_or_else(invalid)?;
        let mut parts = version.split('.');
        let mut number = || -> Result<u32, ManifestError> {
            parts
                .next()
                .filter(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|part| part.parse().ok())
                .ok_or_else(invalid)
        };
        let major = number()?;
        let minor = number()?;
        match parts.next() {
            None => {}
            Some(patch) if !patch.is_empty() && patch.bytes().all(|b| b.is_ascii_digit()) => {}
            Some(_) => return Err(invalid()),
        }
        if parts.next().is_some() {
            return Err(invalid());
        }
        Ok(Self { major, minor })
    }

    /// Returns `true` when a host implementing `self` can load an extension
    /// built for `required`: the same major version and a minor version not
    /// newer than the host.
    #[must_use]
    pub fn accepts(self, required: Self) -> bool {
        self.major == required.major && required.minor <= self.minor
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{API_NAME}@{}.{}", self.major, self.minor)
    }
}

/// Distribution form of an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExtKind {
    /// WebAssembly component (`wasm` names the component file).
    Wasm,
    /// Native library (`library` names the file); needs the `native`
    /// feature and `ext.native = true` in the configuration.
    Native,
}

/// Section `[ext]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtSection {
    /// Extension name, the prefix of its commands.
    pub name: String,
    /// Version of the extension (SemVer).
    pub version: String,
    /// Required API version (`nurt:ext@MAJOR.MINOR`).
    pub api: String,
    /// Distribution form.
    pub kind: ExtKind,
    /// Component file, relative to the extension directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<String>,
    /// Native library file, relative to the extension directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    /// One-line description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Entry `[[stage]]`: a command `<extension>.<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageEntry {
    /// Command name without the extension prefix.
    pub name: String,
    /// One-line description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Record kinds accepted as input (`any` for every kind); empty for a
    /// source only.
    #[serde(default)]
    pub input: Vec<String>,
    /// Record kinds produced; empty for any kind.
    #[serde(default)]
    pub output: Vec<String>,
    /// Parameters, with the keys of a parameter of the command catalog.
    #[serde(rename = "param", default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamSpec>,
    /// The command can start a pipeline.
    #[serde(default)]
    pub source: bool,
    /// The command changes files; needs `permissions.write`.
    #[serde(default)]
    pub writes: bool,
    /// The command contacts the network; needs `permissions.network`.
    #[serde(default)]
    pub network: bool,
}

/// Entry `[[format]]`: a structured document format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatEntry {
    /// Format identifier.
    pub id: String,
    /// File name extensions without the leading period.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Keys can be changed with the layout preserved.
    #[serde(default)]
    pub editable: bool,
}

/// Entry `[[grammar]]`: a tree-sitter grammar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrammarEntry {
    /// Language identifier.
    pub language: String,
    /// File name extensions without the leading period.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Grammar component, relative to the extension directory.
    pub wasm: String,
    /// Tag queries, relative to the extension directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    /// Local scope queries, relative to the extension directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locals: Option<String>,
}

/// Section `[permissions]`.
///
/// Paths are absolute or start with a placeholder expanded by the host
/// (`${project}`, `${home}`, `${temp}`, `${ext}`); the extension reaches
/// files only below them, through calls of the host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Directories the extension may read.
    #[serde(default)]
    pub read: Vec<String>,
    /// Directories the extension may change.
    #[serde(default)]
    pub write: Vec<String>,
    /// Hosts the extension may contact.
    #[serde(default)]
    pub network: Vec<String>,
}

/// Placeholders accepted at the start of a permission path.
pub const PATH_PLACEHOLDERS: [&str; 4] = ["${project}", "${home}", "${temp}", "${ext}"];

/// Parsed manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Section `[ext]`.
    pub ext: ExtSection,
    /// Commands.
    #[serde(default)]
    pub stage: Vec<StageEntry>,
    /// Document formats.
    #[serde(default)]
    pub format: Vec<FormatEntry>,
    /// Grammars.
    #[serde(default)]
    pub grammar: Vec<GrammarEntry>,
    /// Permissions.
    #[serde(default)]
    pub permissions: Permissions,
}

/// Rejected manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManifestError {
    /// The text is not valid TOML or does not have the manifest structure.
    Syntax {
        /// Parser message.
        message: String,
    },
    /// A field has an invalid value.
    InvalidField {
        /// Field, for example `ext.version`.
        field: &'static str,
        /// Description of the problem.
        message: String,
    },
    /// The required API version is not implemented by the host.
    ApiMismatch {
        /// Version required by the extension.
        required: ApiVersion,
        /// Version of the host.
        host: ApiVersion,
    },
    /// A name belongs to the core.
    CoreName {
        /// The name.
        name: String,
    },
    /// A stage needs a permission that the manifest does not grant.
    MissingPermission {
        /// Command name with the extension prefix.
        stage: String,
        /// Permission section (`write`, `network`).
        permission: &'static str,
    },
    /// Two entries have the same name.
    Duplicate {
        /// The name.
        name: String,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { message } => write!(f, "invalid {MANIFEST_FILE}: {message}"),
            Self::InvalidField { field, message } => write!(f, "invalid `{field}`: {message}"),
            Self::ApiMismatch { required, host } => write!(
                f,
                "the extension requires {required}, the host implements {host}; install a matching version"
            ),
            Self::CoreName { name } => {
                write!(
                    f,
                    "`{name}` is a name of the core; choose another extension name"
                )
            }
            Self::MissingPermission { stage, permission } => write!(
                f,
                "`{stage}` needs `permissions.{permission}`; declare the paths or hosts it uses"
            ),
            Self::Duplicate { name } => write!(f, "`{name}` is declared twice"),
        }
    }
}

impl std::error::Error for ManifestError {}

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_NAME_LEN
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

fn check_name(field: &'static str, name: &str) -> Result<(), ManifestError> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(ManifestError::InvalidField {
            field,
            message: format!(
                "`{name}` must start with a lowercase letter and hold only lowercase letters, digits, `_` or `-`"
            ),
        })
    }
}

fn check_identifier(field: &'static str, name: &str) -> Result<(), ManifestError> {
    if name.len() <= MAX_NAME_LEN && is_identifier(name) {
        Ok(())
    } else {
        Err(ManifestError::InvalidField {
            field,
            message: format!(
                "`{name}` must start with a lowercase letter and hold only lowercase letters, digits or `_`"
            ),
        })
    }
}

fn check_kinds(field: &'static str, kinds: &[String]) -> Result<(), ManifestError> {
    match kinds
        .iter()
        .find(|kind| *kind != ANY_KIND && RecordKind::from_str(kind).is_err())
    {
        None => Ok(()),
        Some(kind) => Err(ManifestError::InvalidField {
            field,
            message: format!(
                "`{kind}` is not a record kind; use a kind of the catalog or `{ANY_KIND}`"
            ),
        }),
    }
}

fn check_file(field: &'static str, file: &str) -> Result<(), ManifestError> {
    let path = Path::new(file);
    let inside = !file.is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if inside {
        Ok(())
    } else {
        Err(ManifestError::InvalidField {
            field,
            message: format!("`{file}` must be a relative path inside the extension directory"),
        })
    }
}

fn check_permission_path(field: &'static str, path: &str) -> Result<(), ManifestError> {
    let rest = PATH_PLACEHOLDERS
        .iter()
        .find_map(|placeholder| path.strip_prefix(placeholder));
    let valid = match rest {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => Path::new(path).is_absolute() || path.starts_with('/'),
    };
    if valid && !path.split('/').any(|segment| segment == "..") {
        Ok(())
    } else {
        Err(ManifestError::InvalidField {
            field,
            message: format!(
                "`{path}` must be absolute or start with one of {}",
                PATH_PLACEHOLDERS.join(", ")
            ),
        })
    }
}

impl Manifest {
    /// Parses the text of a manifest without checking it against the host.
    ///
    /// # Errors
    ///
    /// Fails with [`ManifestError::Syntax`] for invalid TOML, unknown keys
    /// and missing required keys.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|error| ManifestError::Syntax {
            message: error.to_string(),
        })
    }

    /// Required API version.
    ///
    /// # Errors
    ///
    /// Fails as [`ApiVersion::parse`].
    pub fn api(&self) -> Result<ApiVersion, ManifestError> {
        ApiVersion::parse(&self.ext.api)
    }

    /// Full command names of the declared stages (`<extension>.<name>`).
    #[must_use]
    pub fn commands(&self) -> Vec<String> {
        self.stage
            .iter()
            .map(|stage| format!("{}.{}", self.ext.name, stage.name))
            .collect()
    }

    /// Checks the manifest against a host implementing `host` whose core
    /// uses the command and group names in `core_names`.
    ///
    /// # Errors
    ///
    /// Fails with the first problem found: [`ManifestError::ApiMismatch`],
    /// [`ManifestError::CoreName`], [`ManifestError::InvalidField`] for a
    /// malformed name, version or file, [`ManifestError::Duplicate`] and
    /// [`ManifestError::MissingPermission`].
    pub fn validate(
        &self,
        host: ApiVersion,
        core_names: &BTreeSet<String>,
    ) -> Result<(), ManifestError> {
        let required = self.api()?;
        if !host.accepts(required) {
            return Err(ManifestError::ApiMismatch { required, host });
        }
        check_identifier("ext.name", &self.ext.name)?;
        if core_names.contains(&self.ext.name) {
            return Err(ManifestError::CoreName {
                name: self.ext.name.clone(),
            });
        }
        semver::Version::parse(&self.ext.version).map_err(|error| ManifestError::InvalidField {
            field: "ext.version",
            message: error.to_string(),
        })?;
        self.check_artifact()?;
        self.check_entries()?;
        self.check_permissions()
    }

    fn check_artifact(&self) -> Result<(), ManifestError> {
        let (field, file) = match self.ext.kind {
            ExtKind::Wasm => ("ext.wasm", self.ext.wasm.as_deref()),
            ExtKind::Native => ("ext.library", self.ext.library.as_deref()),
        };
        let file = file.ok_or_else(|| ManifestError::InvalidField {
            field,
            message: "the file of the extension is not named".to_owned(),
        })?;
        check_file(field, file)
    }

    fn check_entries(&self) -> Result<(), ManifestError> {
        let mut seen = BTreeSet::new();
        for stage in &self.stage {
            check_identifier("stage.name", &stage.name)?;
            check_kinds("stage.input", &stage.input)?;
            check_kinds("stage.output", &stage.output)?;
            if !seen.insert(format!("stage:{}", stage.name)) {
                return Err(ManifestError::Duplicate {
                    name: format!("{}.{}", self.ext.name, stage.name),
                });
            }
            if stage.input.is_empty() && !stage.source {
                return Err(ManifestError::InvalidField {
                    field: "stage.input",
                    message: format!(
                        "`{}` accepts no input and is not a source; list input kinds or set `source = true`",
                        stage.name
                    ),
                });
            }
        }
        for format in &self.format {
            check_name("format.id", &format.id)?;
            if !seen.insert(format!("format:{}", format.id)) {
                return Err(ManifestError::Duplicate {
                    name: format.id.clone(),
                });
            }
        }
        for grammar in &self.grammar {
            check_name("grammar.language", &grammar.language)?;
            check_file("grammar.wasm", &grammar.wasm)?;
            for file in grammar.tags.iter().chain(&grammar.locals) {
                check_file("grammar.tags", file)?;
            }
            if !seen.insert(format!("grammar:{}", grammar.language)) {
                return Err(ManifestError::Duplicate {
                    name: grammar.language.clone(),
                });
            }
        }
        Ok(())
    }

    fn check_permissions(&self) -> Result<(), ManifestError> {
        for path in &self.permissions.read {
            check_permission_path("permissions.read", path)?;
        }
        for path in &self.permissions.write {
            check_permission_path("permissions.write", path)?;
        }
        for stage in &self.stage {
            let missing = if stage.writes && self.permissions.write.is_empty() {
                Some("write")
            } else if stage.network && self.permissions.network.is_empty() {
                Some("network")
            } else {
                None
            };
            if let Some(permission) = missing {
                return Err(ManifestError::MissingPermission {
                    stage: format!("{}.{}", self.ext.name, stage.name),
                    permission,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{ApiVersion, HOST_API, Manifest, ManifestError};

    const VALID: &str = r#"
[ext]
name = "acme"
version = "1.2.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "acme.wasm"

[[stage]]
name = "lint"
input = ["entry"]

[[stage]]
name = "fix"
input = ["entry"]
writes = true

[[format]]
id = "acme-conf"
extensions = ["acmeconf"]

[[grammar]]
language = "acme"
wasm = "grammar/acme.wasm"
tags = "grammar/tags.scm"

[permissions]
read = ["${project}"]
write = ["${project}/out", "/var/acme"]
"#;

    fn core() -> BTreeSet<String> {
        ["git", "code", "find"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn rejected(text: &str) -> ManifestError {
        Manifest::parse(text)
            .and_then(|manifest| manifest.validate(HOST_API, &core()))
            .unwrap_err()
    }

    #[test]
    fn valid_manifest_is_accepted() {
        let manifest = Manifest::parse(VALID).unwrap();
        manifest.validate(HOST_API, &core()).unwrap();
        assert_eq!(manifest.commands(), ["acme.lint", "acme.fix"]);
        assert_eq!(manifest.api().unwrap().to_string(), "nurt:ext@1.0");
    }

    #[test]
    fn api_versions_follow_major_and_minor_rules() {
        assert_eq!(
            ApiVersion::parse("nurt:ext@1.0.3").unwrap(),
            ApiVersion { major: 1, minor: 0 }
        );
        for text in [
            "nurt:ext@1",
            "nurt:ext@1.x",
            "other@1.0",
            "nurt:ext@1.0.0.0",
            "nurt:ext@1.0.",
        ] {
            assert!(ApiVersion::parse(text).is_err(), "{text}");
        }
        let host = ApiVersion { major: 1, minor: 2 };
        assert!(host.accepts(ApiVersion { major: 1, minor: 1 }));
        assert!(!host.accepts(ApiVersion { major: 1, minor: 3 }));
        assert!(!host.accepts(ApiVersion { major: 0, minor: 9 }));
    }

    #[test]
    fn another_major_version_is_rejected() {
        let error = rejected(&VALID.replace("nurt:ext@1.0", "nurt:ext@2.0"));
        assert!(matches!(error, ManifestError::ApiMismatch { .. }));
        assert!(error.to_string().contains("nurt:ext@2.0"));
    }

    #[test]
    fn core_names_are_rejected() {
        let error = rejected(&VALID.replace("name = \"acme\"", "name = \"git\""));
        assert_eq!(
            error,
            ManifestError::CoreName {
                name: "git".to_owned()
            }
        );
    }

    #[test]
    fn stages_without_permissions_are_rejected() {
        let error = rejected(&VALID.replace("write = [\"${project}/out\", \"/var/acme\"]", ""));
        assert_eq!(
            error,
            ManifestError::MissingPermission {
                stage: "acme.fix".to_owned(),
                permission: "write"
            }
        );
        let network = VALID.replace("name = \"lint\"", "name = \"lint\"\nnetwork = true");
        assert!(matches!(
            rejected(&network),
            ManifestError::MissingPermission {
                permission: "network",
                ..
            }
        ));
    }

    #[test]
    fn malformed_fields_are_rejected() {
        let cases = [
            VALID.replace("version = \"1.2.0\"", "version = \"one\""),
            VALID.replace("name = \"lint\"", "name = \"Lint\""),
            VALID.replace("wasm = \"acme.wasm\"", "wasm = \"../acme.wasm\""),
            VALID.replace("wasm = \"acme.wasm\"", ""),
            VALID.replace("read = [\"${project}\"]", "read = [\"relative/dir\"]"),
            VALID.replace("read = [\"${project}\"]", "read = [\"${project}/../up\"]"),
            VALID.replace("tags = \"grammar/tags.scm\"", "tags = \"/abs/tags.scm\""),
            VALID.replace("input = [\"entry\"]\n\n[[stage]]", "\n[[stage]]"),
        ];
        for text in &cases {
            assert!(
                matches!(rejected(text), ManifestError::InvalidField { .. }),
                "{text}"
            );
        }
        let twice = VALID.replace("name = \"fix\"", "name = \"lint\"");
        assert!(matches!(rejected(&twice), ManifestError::Duplicate { .. }));
        assert!(matches!(
            rejected("[ext]\nname = 1\n"),
            ManifestError::Syntax { .. }
        ));
        assert!(matches!(
            rejected(&format!("{VALID}\nunknown = 1\n")),
            ManifestError::Syntax { .. }
        ));
    }
}
