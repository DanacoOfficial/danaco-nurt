// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Neutral data read by providers, before merging into program cards.
//!
//! Providers fill [`SourceData`] with raw entries; the index merges entries
//! of all providers into cards. Paths are host paths written as text, as
//! they appear in records. Every type serializes, so the index keeps
//! [`SourceData`] in its cache.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::source::{SourceId, SourceWarning};

/// Kind of a card.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A command started by name.
    #[default]
    Program,
    /// An application without a command in `PATH`.
    App,
    /// A usage guide not bound to a program.
    Guide,
}

impl EntryKind {
    /// Name used in records.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Program => "program",
            Self::App => "app",
            Self::Guide => "guide",
        }
    }
}

/// Text with variants in several languages.
///
/// `text` is in language `lang` (unknown when `None`); `translations` holds
/// the other variants by language code. Search reads every variant; cards
/// show the variant of the most preferred language.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalizedText {
    /// Main variant.
    pub text: String,
    /// Language of the main variant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Other variants by language code.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: BTreeMap<String, String>,
}

impl LocalizedText {
    /// Text in an unknown language.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            lang: None,
            translations: BTreeMap::new(),
        }
    }

    /// Text in language `lang`.
    #[must_use]
    pub fn in_lang(text: impl Into<String>, lang: &str) -> Self {
        Self {
            text: text.into(),
            lang: Some(lang.to_owned()),
            translations: BTreeMap::new(),
        }
    }

    /// Adds or replaces the variant of language `lang`.
    pub fn set(&mut self, lang: &str, text: impl Into<String>) {
        if self.lang.as_deref() == Some(lang) {
            self.text = text.into();
        } else {
            self.translations.insert(lang.to_owned(), text.into());
        }
    }

    /// Variant of the first language of `languages` that has one, otherwise
    /// the main variant; also returns the language of the chosen variant.
    #[must_use]
    pub fn pick<'a>(&'a self, languages: &[String]) -> (&'a str, Option<&'a str>) {
        for language in languages {
            if self.lang.as_deref() == Some(language.as_str()) {
                return (&self.text, self.lang.as_deref());
            }
            if let Some((lang, text)) = self.translations.get_key_value(language) {
                return (text, Some(lang));
            }
        }
        (&self.text, self.lang.as_deref())
    }

    /// Every variant, main first.
    pub fn variants(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.text.as_str()).chain(self.translations.values().map(String::as_str))
    }

    /// Returns `true` when the main variant is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

impl From<&str> for LocalizedText {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for LocalizedText {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

/// Classification scheme of a category hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryScheme {
    /// Category id of an external catalog, used as it is.
    Catalog,
    /// `Categories=` of a desktop entry.
    Freedesktop,
    /// Section of a Debian package (`utils`, `contrib/net`).
    DebianSection,
    /// `Group` of an RPM package (`Productivity/Archiving/Compression`).
    RpmGroup,
    /// Group of a pacman package; a weak hint.
    PacmanGroup,
    /// `LSApplicationCategoryType` of a macOS application.
    MacosCategory,
    /// Section of a manual page (`1`, `6`, `8`).
    ManSection,
    /// Free word: tag, keyword or package name, read by the classifier.
    Keyword,
}

/// Hint for the category of an entry, mapped by the taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CategoryHint {
    /// Scheme.
    pub scheme: CategoryScheme,
    /// Value as written by the provider.
    pub value: String,
}

impl CategoryHint {
    /// Creates a hint.
    #[must_use]
    pub fn new(scheme: CategoryScheme, value: impl Into<String>) -> Self {
        Self {
            scheme,
            value: value.into(),
        }
    }
}

/// Origin of the category of a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategorySource {
    /// An external catalog.
    Catalog,
    /// A desktop entry or an application bundle.
    Desktop,
    /// Package metadata.
    Package,
    /// The section of the manual page.
    Man,
    /// The keyword classifier.
    Inferred,
}

impl CategorySource {
    /// Name used in records.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Desktop => "desktop",
            Self::Package => "package",
            Self::Man => "man",
            Self::Inferred => "inferred",
        }
    }
}

/// How an alternative name relates to the main entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasKind {
    /// Another name of the same function.
    #[default]
    Alias,
    /// The same program with a version in its name.
    Versioned,
    /// A helper command of the same package.
    Companion,
    /// A symbolic link in `PATH` to another program in `PATH`.
    Symlink,
    /// A Debian alternative (`/etc/alternatives`).
    Alternative,
    /// A shim of Scoop or Chocolatey.
    Shim,
    /// A Windows application execution alias.
    AppAlias,
}

impl AliasKind {
    /// Name used in records.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alias => "alias",
            Self::Versioned => "versioned",
            Self::Companion => "companion",
            Self::Symlink => "symlink",
            Self::Alternative => "alternative",
            Self::Shim => "shim",
            Self::AppAlias => "app_alias",
        }
    }
}

/// Alternative name of an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAlias {
    /// Name as typed.
    pub name: String,
    /// Relation to the main entry.
    pub kind: AliasKind,
    /// Path of the alias file, when it has one.
    pub path: Option<String>,
}

impl RawAlias {
    /// Alias of `kind` without a path.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: AliasKind) -> Self {
        Self {
            name: name.into(),
            kind,
            path: None,
        }
    }
}

/// Task a program is not meant for, with the replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotFor {
    /// Description of the task.
    pub task: LocalizedText,
    /// Program to use instead.
    pub instead: Option<String>,
}

/// Kind of a structured part of a description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    /// What the program does.
    Purpose,
    /// When to use it.
    When,
    /// When not to use it.
    WhenNot,
    /// Input and output.
    Io,
    /// An example invocation.
    Example,
    /// How to check that it works.
    Check,
    /// Pitfalls.
    Pitfalls,
    /// Related programs.
    Related,
    /// Alternative names.
    Aliases,
    /// Anything else.
    #[default]
    Other,
}

/// Structured part of a description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescriptionSection {
    /// Kind.
    pub kind: SectionKind,
    /// Display label in the language of the text.
    pub label: Option<String>,
    /// Text.
    pub text: LocalizedText,
}

/// Reference from a program to its usage guide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuideRef {
    /// Guide name; programs with the same guide name share one guide.
    pub name: String,
    /// Path of the guide relative to the provider root, `/` separated.
    pub path: String,
    /// `description` of the guide front matter, a ranking field.
    pub description: Option<LocalizedText>,
}

/// Package that provides an entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageRef {
    /// Provider id of the package manager.
    pub manager: SourceId,
    /// Package name.
    pub name: String,
}

/// Identity of an entry within its provider, passed back to
/// [`ProgramSource::detail`](crate::source::ProgramSource::detail).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntryKey {
    /// Provider.
    pub source: SourceId,
    /// Key defined by the provider, for example a package name or a file.
    pub key: String,
}

impl EntryKey {
    /// Creates a key.
    #[must_use]
    pub fn new(source: &SourceId, key: impl Into<String>) -> Self {
        Self {
            source: source.clone(),
            key: key.into(),
        }
    }
}

/// A program, application or catalog entry as read by one provider.
///
/// Only `name` and `key` are required; every other field is filled when the
/// provider knows it. The index merges entries with the same name from
/// several providers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RawProgram {
    /// Command name as typed, or the display name of an application without
    /// a command.
    pub name: String,
    /// Identity for [`ProgramSource::detail`](crate::source::ProgramSource::detail).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<EntryKey>,
    /// Kind.
    pub kind: EntryKind,
    /// Local file of the command or application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Further local files with the same name, in resolution order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<String>,
    /// Main entry when this name is an alias of another program.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<String>,
    /// Kind of the alias in `alias_of`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_kind: Option<AliasKind>,
    /// Alternative names of this entry.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<RawAlias>,
    /// One sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<LocalizedText>,
    /// One paragraph for category listings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview: Option<LocalizedText>,
    /// Full description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<LocalizedText>,
    /// Structured parts of the description.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<DescriptionSection>,
    /// Situations in which to use the program.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when_to_use: Vec<LocalizedText>,
    /// Tasks the program is not meant for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_for: Vec<NotFor>,
    /// One working invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// Hints for the category, strongest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub category_hints: Vec<CategoryHint>,
    /// Version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Package that provides the entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageRef>,
    /// Name of the package that provides the entry when the provider does
    /// not know the package manager, as in catalog entries. The index joins
    /// it with a package of that name from any package database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    /// Application identifier (bundle id, desktop file id, AppX family,
    /// uninstall key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Display name of the application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<LocalizedText>,
    /// File name of the main executable of an application
    /// (`CFBundleExecutable`). An application whose executable is a command
    /// in `PATH` is merged with the card of that command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// Project home page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Licence of the program.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Tags and keywords, ranked with a low weight.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Operating systems named by a catalog.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// Date the catalog entry was verified, `YYYY-MM-DD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<String>,
    /// Path named by a catalog for another computer; never executed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_path: Option<String>,
    /// The entry exists on this computer; `None` lets the index decide from
    /// `PATH` and the other providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,
    /// Usage guide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guide: Option<GuideRef>,
    /// The guide is not a prerequisite of use.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub guide_optional: bool,
    /// The guide should be read before the first use.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub guide_required: bool,
    /// Related programs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    /// Description of the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Description of the output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl RawProgram {
    /// Entry `name` of kind [`EntryKind::Program`].
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

/// Metadata of an installed package.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RawPackage {
    /// Package name.
    pub name: String,
    /// Full version as the manager writes it (with epoch and release).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Architecture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    /// One-line summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<LocalizedText>,
    /// Long description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<LocalizedText>,
    /// Section or group as written by the manager.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// Further groups (pacman groups, Homebrew taps).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Project home page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Licence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Source package or repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Installed size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_size: Option<u64>,
    /// Install time, RFC 3339 UTC.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
    /// Installed on explicit request (`false` for dependencies; `None` when
    /// the manager does not record it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit: Option<bool>,
    /// Executable files of the package in binary directories, as host paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub executables: Vec<String>,
}

/// Category declared by a catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RawCategory {
    /// Id.
    pub id: String,
    /// Title.
    pub title: LocalizedText,
    /// One sentence.
    pub summary: Option<LocalizedText>,
    /// Description.
    pub description: Option<LocalizedText>,
    /// Parent category; a category with a parent is a subcategory.
    pub parent: Option<String>,
    /// Alternative names accepted as argument.
    pub aliases: Vec<String>,
    /// Built-in taxonomy id this category is equivalent to.
    pub same_as: Option<String>,
    /// Ordering hint.
    pub order: Option<i64>,
}

/// Usage guide not bound to a program.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RawGuide {
    /// Guide name.
    pub name: String,
    /// Path relative to the provider root.
    pub path: String,
    /// Summary, by default the `description` of the front matter.
    pub summary: Option<LocalizedText>,
    /// Category id.
    pub category: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Identity for [`ProgramSource::detail`](crate::source::ProgramSource::detail).
    pub key: Option<EntryKey>,
}

/// Everything one provider read.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SourceData {
    /// Commands, applications and catalog entries.
    pub programs: Vec<RawProgram>,
    /// Installed packages.
    pub packages: Vec<RawPackage>,
    /// Executable file (host path) to the package that installed it; only
    /// files in binary directories.
    pub file_packages: Vec<(String, PackageRef)>,
    /// Categories (catalogs only).
    pub categories: Vec<RawCategory>,
    /// Standalone guides (catalogs only).
    pub guides: Vec<RawGuide>,
    /// Pairs of words with the same meaning, used by search.
    pub synonyms: Vec<Vec<String>>,
    /// Entries skipped while reading.
    pub warnings: Vec<SourceWarning>,
}

/// Details requested from [`ProgramSource::detail`](crate::source::ProgramSource::detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DetailWant {
    /// Full description.
    pub description: bool,
    /// Guide text.
    pub guide: bool,
    /// Manual page summary.
    pub man: bool,
}

/// Text of a usage guide without its front matter.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GuideBody {
    /// Markdown text.
    pub text: String,
    /// `version` of the front matter.
    pub version: Option<String>,
    /// Language of the text.
    pub language: Option<String>,
    /// Headings of the `## ` sections, outside code blocks.
    pub sections: Vec<String>,
    /// Extra material of the guide, relative paths.
    pub files: Vec<String>,
}

/// Details of one entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Detail {
    /// Full description.
    pub description: Option<String>,
    /// Guide.
    pub guide: Option<GuideBody>,
    /// `NAME` line of the manual page and the page file.
    pub man: Option<String>,
}
