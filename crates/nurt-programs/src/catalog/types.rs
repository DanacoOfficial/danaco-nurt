// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Types of a catalog after reading, limits of the format and the patterns
//! of names and identifiers.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use crate::catalog::guide::FrontMatter;
use crate::model::{AliasKind, SectionKind};

/// Value of `format` of the major version implemented by the reader.
pub const FORMAT: &str = "nurt-program-catalog/1";
/// Prefix of every value of `format`, followed by the major version.
pub const FORMAT_PREFIX: &str = "nurt-program-catalog/";
/// Major version implemented by the reader.
pub const FORMAT_MAJOR: u64 = 1;
/// Latest revision of the major version implemented by the reader.
pub const FORMAT_MINOR: u64 = 0;
/// File name of the index in the catalog root.
pub const INDEX_FILE: &str = "catalog.json";
/// Directory of localized overlays in the catalog root.
pub const I18N_DIR: &str = "i18n";
/// Extension of guide files.
pub const GUIDE_EXTENSION: &str = ".md";
/// Language of a catalog that does not declare one.
pub const DEFAULT_LANGUAGE: &str = "en";

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

/// Limits of the format, enforced while reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Size of `catalog.json` in bytes.
    pub index_bytes: u64,
    /// Size of one included part in bytes.
    pub part_bytes: u64,
    /// Total size of the index and its parts in bytes.
    pub total_bytes: u64,
    /// Size of one program entry in its source text, in bytes.
    pub entry_bytes: usize,
    /// Size of a guide file in bytes.
    pub guide_bytes: u64,
    /// Size of the front matter of a guide in bytes.
    pub front_matter_bytes: usize,
    /// Number of programs.
    pub programs: usize,
    /// Number of categories.
    pub categories: usize,
    /// Number of standalone guides.
    pub guides: usize,
    /// Nesting depth of JSON values.
    pub depth: usize,
    /// Number of files of guide material listed.
    pub guide_files: usize,
    /// Depth of directories of guide material listed.
    pub guide_files_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            index_bytes: 32 * MIB,
            part_bytes: 32 * MIB,
            total_bytes: 128 * MIB,
            entry_bytes: 64 * 1024,
            guide_bytes: MIB,
            front_matter_bytes: 8 * 1024,
            programs: 100_000,
            categories: 10_000,
            guides: 100_000,
            depth: 32,
            guide_files: 256,
            guide_files_depth: 4,
        }
    }
}

/// How a catalog is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReadOptions {
    /// Limits of the format.
    pub limits: Limits,
    /// Validation: every violation is an error and the guides are checked
    /// against the entries that name them. Reading for search and
    /// description leaves it unset.
    pub strict: bool,
}

/// Position of an object in a file of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Origin {
    /// File relative to the catalog root.
    pub file: String,
    /// Line of the start of the object, counted from 1.
    pub line: u64,
    /// Path of the object in the file, for example `programs[3]`.
    pub pointer: String,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} {}", self.file, self.line, self.pointer)
    }
}

/// Category or subcategory of a catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Category {
    /// Identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Description.
    pub description: Option<String>,
    /// Parent category of a subcategory.
    pub parent: Option<String>,
    /// One sentence.
    pub summary: Option<String>,
    /// Alternative names accepted where a category is named.
    pub aliases: Vec<String>,
    /// Built-in category this category is equivalent to.
    pub same_as: Option<String>,
    /// Position in listings.
    pub order: Option<i64>,
    /// Position in the catalog.
    pub origin: Origin,
}

/// Alternative name of a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    /// The name.
    pub name: String,
    /// Relation to the program: alias, versioned or companion.
    pub kind: AliasKind,
    /// Path on the described host; informational.
    pub path: Option<String>,
}

/// Task a program does not serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotForTask {
    /// Description of the task.
    pub task: String,
    /// Program to use instead.
    pub instead: Option<String>,
}

/// Structured part of the description of a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Kind.
    pub kind: SectionKind,
    /// Display label.
    pub label: Option<String>,
    /// Markdown text.
    pub text: String,
}

/// Program entry of a catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Program {
    /// Command or program name.
    pub name: String,
    /// Category or subcategory.
    pub category: String,
    /// One or two sentences.
    pub summary: String,
    /// Alternative names, without collisions.
    pub aliases: Vec<Alias>,
    /// Operating systems.
    pub platforms: Vec<String>,
    /// Path of the guide relative to the root; set only when the guide file
    /// exists inside the root and has valid front matter.
    pub guide: Option<String>,
    /// Date of the last verification.
    pub verified: Option<String>,
    /// Keywords.
    pub tags: Vec<String>,
    /// One paragraph for category listings.
    pub overview: Option<String>,
    /// Full description, Markdown.
    pub description: Option<String>,
    /// Structured parts of the description.
    pub sections: Vec<Section>,
    /// Situations in which the program applies.
    pub when_to_use: Vec<String>,
    /// Tasks the program does not serve.
    pub not_for: Vec<NotForTask>,
    /// One complete invocation.
    pub example: Option<String>,
    /// The guide is not a prerequisite of use.
    pub guide_optional: bool,
    /// The guide should be read before the first use.
    pub guide_required: bool,
    /// Version of the program.
    pub version: Option<String>,
    /// Path on the described host; informational.
    pub path: Option<String>,
    /// Project page.
    pub homepage: Option<String>,
    /// License of the program.
    pub license: Option<String>,
    /// Package that provides the program.
    pub package: Option<String>,
    /// Accepted input.
    pub input: Option<String>,
    /// Produced output.
    pub output: Option<String>,
    /// Related programs.
    pub related: Vec<String>,
    /// Position in the catalog.
    pub origin: Origin,
}

/// Guide not bound to a program.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StandaloneGuide {
    /// Name.
    pub name: String,
    /// Path of the guide relative to the root.
    pub guide: String,
    /// One sentence.
    pub summary: Option<String>,
    /// Category.
    pub category: Option<String>,
    /// Keywords.
    pub tags: Vec<String>,
}

/// Front matter of a guide file, read when the catalog is loaded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuideHeader {
    /// Name of the guide: the file name without `.md`.
    pub name: String,
    /// Path relative to the root.
    pub path: String,
    /// Front matter.
    pub front: FrontMatter,
}

/// Texts of a category in an overlay.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CategoryText {
    /// Title.
    pub title: Option<String>,
    /// One sentence.
    pub summary: Option<String>,
    /// Description.
    pub description: Option<String>,
}

/// Texts of a program in an overlay; a present member replaces the member of
/// the index as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProgramText {
    /// One or two sentences.
    pub summary: Option<String>,
    /// One paragraph.
    pub overview: Option<String>,
    /// Full description.
    pub description: Option<String>,
    /// Situations in which the program applies.
    pub when_to_use: Option<Vec<String>>,
    /// Tasks the program does not serve.
    pub not_for: Option<Vec<NotForTask>>,
    /// Structured parts of the description.
    pub sections: Option<Vec<Section>>,
    /// Keywords.
    pub tags: Option<Vec<String>>,
    /// Translated guide, relative to the root.
    pub guide: Option<String>,
}

/// Texts of a standalone guide in an overlay.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuideText {
    /// One sentence.
    pub summary: Option<String>,
    /// Keywords.
    pub tags: Option<Vec<String>>,
    /// Translated guide, relative to the root.
    pub guide: Option<String>,
}

/// Localized overlay `i18n/<lang>/catalog.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Overlay {
    /// Language of the texts.
    pub language: String,
    /// Texts of categories by identifier.
    pub categories: BTreeMap<String, CategoryText>,
    /// Texts of programs by name.
    pub programs: BTreeMap<String, ProgramText>,
    /// Texts of standalone guides by name.
    pub guides: BTreeMap<String, GuideText>,
}

/// A catalog after reading: the index merged with its parts, the front
/// matter of its guides and its overlays. Every object kept satisfies the
/// format.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalog {
    /// Resolved root directory.
    pub root: PathBuf,
    /// Revision of the format declared by the catalog.
    pub format_minor: u64,
    /// Identifier declared by the catalog.
    pub id: Option<String>,
    /// Display title.
    pub title: Option<String>,
    /// Version of the catalog data.
    pub version: Option<String>,
    /// Language of the texts of the index and of the guides.
    pub language: String,
    /// Languages of overlays.
    pub languages: Vec<String>,
    /// License of the catalog data.
    pub license: Option<String>,
    /// Categories and subcategories.
    pub categories: Vec<Category>,
    /// Programs of the index and of the included parts.
    pub programs: Vec<Program>,
    /// Standalone guides.
    pub guides: Vec<StandaloneGuide>,
    /// Groups of equivalent words.
    pub synonyms: Vec<Vec<String>>,
    /// Included parts that were read, relative to the root.
    pub parts: Vec<String>,
    /// Front matter of every guide named by the catalog, by path.
    pub guide_headers: BTreeMap<String, GuideHeader>,
    /// Localized overlays.
    pub overlays: Vec<Overlay>,
}

impl Catalog {
    /// Program with the name.
    #[must_use]
    pub fn program(&self, name: &str) -> Option<&Program> {
        self.programs.iter().find(|program| program.name == name)
    }

    /// Standalone guide with the name.
    #[must_use]
    pub fn standalone_guide(&self, name: &str) -> Option<&StandaloneGuide> {
        self.guides.iter().find(|guide| guide.name == name)
    }

    /// Category with the identifier.
    #[must_use]
    pub fn category(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|category| category.id == id)
    }

    /// Overlay of the language.
    #[must_use]
    pub fn overlay(&self, language: &str) -> Option<&Overlay> {
        self.overlays
            .iter()
            .find(|overlay| overlay.language == language)
    }

    /// Other programs that name the same guide as `name`, in catalog order.
    #[must_use]
    pub fn guide_shared_with(&self, name: &str) -> Vec<String> {
        let Some(guide) = self.program(name).and_then(|program| program.guide.as_deref()) else {
            return Vec::new();
        };
        self.programs
            .iter()
            .filter(|program| program.name != name && program.guide.as_deref() == Some(guide))
            .map(|program| program.name.clone())
            .collect()
    }
}

/// Checks a program, alias or guide name: a letter or digit followed by up
/// to 119 letters, digits, `.`, `_`, `+` or `-`.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().count() <= 120
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// Checks a category identifier: a lower-case letter or digit followed by
/// up to 59 lower-case letters, digits, `_` or `-`.
#[must_use]
pub fn is_valid_category_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(is_lower_alphanumeric)
        && id.len() <= 60
        && chars.all(|c| is_lower_alphanumeric(c) || matches!(c, '_' | '-'))
}

/// Checks a catalog identifier: a lower-case letter or digit followed by up
/// to 63 lower-case letters, digits, `.`, `_` or `-`.
#[must_use]
pub fn is_valid_catalog_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(is_lower_alphanumeric)
        && id.len() <= 64
        && chars.all(|c| is_lower_alphanumeric(c) || matches!(c, '.' | '_' | '-'))
}

/// Checks a language tag: two or three lower-case letters, optionally
/// followed by `-` and two upper-case letters.
#[must_use]
pub fn is_valid_language(tag: &str) -> bool {
    let (language, region) = match tag.split_once('-') {
        Some((language, region)) => (language, Some(region)),
        None => (tag, None),
    };
    (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_lowercase())
        && region.is_none_or(|region| region.len() == 2 && region.bytes().all(|b| b.is_ascii_uppercase()))
}

/// Checks a date `YYYY-MM-DD` with a month from 01 to 12 and a day from 01
/// to 31.
#[must_use]
pub fn is_valid_date(date: &str) -> bool {
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let digits = |text: &str, count: usize| text.len() == count && text.bytes().all(|b| b.is_ascii_digit());
    if !(digits(year, 4) && digits(month, 2) && digits(day, 2)) {
        return false;
    }
    let month: u8 = month.parse().unwrap_or(0);
    let day: u8 = day.parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_lower_alphanumeric(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}
