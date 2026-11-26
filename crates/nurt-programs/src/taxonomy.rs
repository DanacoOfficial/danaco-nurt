// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Built-in taxonomy of program categories, mappings of system
//! classifications and the table of synonyms.
//!
//! The data lives in `data/taxonomy.toml`, `data/category-maps.toml` and
//! `data/synonyms.toml` and is compiled into the crate. A category is chosen
//! from the hints of all providers in a fixed order: catalog, desktop entry
//! or application bundle, package section or group, command name in the
//! keyword table, manual page section, keyword classifier, `other`.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::{CategoryHint, CategoryScheme, CategorySource, LocalizedText};
use crate::text::fold;

const TAXONOMY_DATA: &str = include_str!("../data/taxonomy.toml");
const MAPS_DATA: &str = include_str!("../data/category-maps.toml");
const SYNONYMS_DATA: &str = include_str!("../data/synonyms.toml");

/// Id of the category of programs without a more specific one.
pub const OTHER: &str = "other";

/// Id of the replacement subcategory that collects programs of a divided
/// category outside all its subcategories.
pub const OTHER_SUBCATEGORY: &str = "_other";

/// Prefix of the uniform type identifiers of macOS application categories.
const MACOS_CATEGORY_PREFIX: &str = "public.app-category.";

/// Category of the built-in taxonomy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxonomyCategory {
    /// Id.
    pub id: String,
    /// Title.
    pub title: LocalizedText,
    /// One sentence.
    pub summary: LocalizedText,
    /// Position in the taxonomy.
    pub order: usize,
}

/// Category chosen for an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// Category id.
    pub id: String,
    /// Origin of the category.
    pub source: CategorySource,
}

#[derive(Debug, Default, Deserialize)]
struct TaxonomyFile {
    #[serde(default)]
    category: Vec<CategoryRow>,
}

#[derive(Debug, Deserialize)]
struct CategoryRow {
    id: String,
    title: String,
    summary: String,
    #[serde(flatten)]
    translations: BTreeMap<String, TranslationRow>,
}

#[derive(Debug, Deserialize)]
struct TranslationRow {
    title: String,
    summary: String,
}

#[derive(Debug, Default, Deserialize)]
struct MapsFile {
    #[serde(default)]
    debian_section: BTreeMap<String, String>,
    #[serde(default)]
    debian_weak: ValuesRow,
    #[serde(default)]
    freedesktop: BTreeMap<String, String>,
    #[serde(default)]
    freedesktop_main: ValuesRow,
    #[serde(default)]
    macos_category: BTreeMap<String, String>,
    #[serde(default)]
    rpm_group: BTreeMap<String, String>,
    #[serde(default)]
    man_section: BTreeMap<String, String>,
    #[serde(default)]
    keywords: toml::Table,
}

#[derive(Debug, Default, Deserialize)]
struct ValuesRow {
    #[serde(default, alias = "sections")]
    values: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SynonymsFile {
    #[serde(default)]
    groups: Vec<Vec<String>>,
}

/// Taxonomy with its mappings.
#[derive(Debug, Clone, Default)]
pub struct Taxonomy {
    categories: Vec<TaxonomyCategory>,
    debian: HashMap<String, String>,
    debian_weak: Vec<String>,
    freedesktop: HashMap<String, String>,
    freedesktop_main: Vec<String>,
    macos: HashMap<String, String>,
    rpm: Vec<(String, String)>,
    man: HashMap<String, String>,
    keywords: Vec<(String, Vec<String>)>,
}

impl Taxonomy {
    /// The built-in taxonomy; an empty one when the embedded data is
    /// damaged, which [`try_builtin`](Self::try_builtin) reports.
    #[must_use]
    pub fn builtin() -> &'static Self {
        static FALLBACK: OnceLock<Taxonomy> = OnceLock::new();
        match Self::loaded() {
            Ok(taxonomy) => taxonomy,
            Err(_) => FALLBACK.get_or_init(Self::default),
        }
    }

    /// The built-in taxonomy.
    ///
    /// # Errors
    ///
    /// Returns the parse error of the embedded data files.
    pub fn try_builtin() -> Result<&'static Self, &'static str> {
        Self::loaded().as_ref().map_err(String::as_str)
    }

    fn loaded() -> &'static Result<Taxonomy, String> {
        static TAXONOMY: OnceLock<Result<Taxonomy, String>> = OnceLock::new();
        TAXONOMY.get_or_init(|| Self::parse(TAXONOMY_DATA, MAPS_DATA))
    }

    fn parse(taxonomy: &str, maps: &str) -> Result<Self, String> {
        let taxonomy: TaxonomyFile =
            toml::from_str(taxonomy).map_err(|error| format!("taxonomy.toml: {error}"))?;
        let maps: MapsFile =
            toml::from_str(maps).map_err(|error| format!("category-maps.toml: {error}"))?;
        let categories = taxonomy
            .category
            .into_iter()
            .enumerate()
            .map(|(order, row)| {
                let mut title = LocalizedText::in_lang(row.title, "en");
                let mut summary = LocalizedText::in_lang(row.summary, "en");
                for (lang, translation) in row.translations {
                    title.set(&lang, translation.title);
                    summary.set(&lang, translation.summary);
                }
                TaxonomyCategory {
                    id: row.id,
                    title,
                    summary,
                    order,
                }
            })
            .collect();
        let mut rpm: Vec<(String, String)> = maps
            .rpm_group
            .into_iter()
            .map(|(prefix, id)| (prefix.to_lowercase(), id))
            .collect();
        rpm.sort_by(|left, right| right.0.len().cmp(&left.0.len()).then(left.0.cmp(&right.0)));
        let order: Vec<String> = maps.keywords.keys().cloned().collect();
        let keywords = order
            .into_iter()
            .filter_map(|id| {
                let words = maps.keywords.get(&id)?.as_array()?;
                let words = words
                    .iter()
                    .filter_map(|word| word.as_str().map(fold))
                    .collect();
                Some((id, words))
            })
            .collect();
        Ok(Self {
            categories,
            debian: maps.debian_section.into_iter().collect(),
            debian_weak: maps.debian_weak.values,
            freedesktop: maps.freedesktop.into_iter().collect(),
            freedesktop_main: maps.freedesktop_main.values,
            macos: maps.macos_category.into_iter().collect(),
            rpm,
            man: maps.man_section.into_iter().collect(),
            keywords,
        })
    }

    /// Categories in taxonomy order.
    #[must_use]
    pub fn categories(&self) -> &[TaxonomyCategory] {
        &self.categories
    }

    /// Category with the id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&TaxonomyCategory> {
        self.categories.iter().find(|category| category.id == id)
    }

    /// Taxonomy id of one hint, without the keyword classifier.
    #[must_use]
    pub fn map_hint<'a>(&'a self, hint: &'a CategoryHint) -> Option<&'a str> {
        let value = hint.value.trim();
        match hint.scheme {
            CategoryScheme::Catalog => Some(value),
            CategoryScheme::Freedesktop => self.freedesktop.get(value).map(String::as_str),
            CategoryScheme::DebianSection => {
                let section = value.rsplit('/').next().unwrap_or(value);
                self.debian.get(section).map(String::as_str)
            }
            CategoryScheme::RpmGroup => {
                let group = value.to_lowercase();
                self.rpm
                    .iter()
                    .find(|(prefix, _)| group.starts_with(prefix.as_str()))
                    .map(|(_, id)| id.as_str())
            }
            CategoryScheme::MacosCategory => {
                let key = value.strip_prefix(MACOS_CATEGORY_PREFIX).unwrap_or(value);
                self.macos.get(key).map(String::as_str)
            }
            CategoryScheme::ManSection => {
                let section: String = value.chars().take_while(char::is_ascii_digit).collect();
                self.man.get(&section).map(String::as_str)
            }
            CategoryScheme::PacmanGroup | CategoryScheme::Keyword => None,
        }
    }

    /// Chooses the category of an entry named `name` from its hints.
    #[must_use]
    pub fn classify(&self, name: &str, hints: &[CategoryHint]) -> Classification {
        let found = |id: &str, source| Classification {
            id: id.to_owned(),
            source,
        };
        if let Some(hint) = hints
            .iter()
            .find(|hint| hint.scheme == CategoryScheme::Catalog && !hint.value.is_empty())
        {
            return found(&hint.value, CategorySource::Catalog);
        }
        if let Some(id) = self.desktop_category(hints) {
            return found(id, CategorySource::Desktop);
        }
        let (strong, weak) = self.package_category(hints);
        if let Some(id) = strong {
            return found(id, CategorySource::Package);
        }
        let name_key = fold(name);
        if let Some(id) = self.keyword_category(std::iter::once(name_key.as_str())) {
            return found(id, CategorySource::Inferred);
        }
        if let Some(id) = hints
            .iter()
            .filter(|hint| hint.scheme == CategoryScheme::ManSection)
            .find_map(|hint| self.map_hint(hint))
        {
            return found(id, CategorySource::Man);
        }
        let words = keyword_words(&name_key, hints);
        if let Some(id) = self.keyword_category(words.iter().map(String::as_str)) {
            return found(id, CategorySource::Inferred);
        }
        if let Some(id) = weak {
            return found(id, CategorySource::Package);
        }
        found(OTHER, CategorySource::Inferred)
    }

    fn desktop_category<'a>(&'a self, hints: &'a [CategoryHint]) -> Option<&'a str> {
        let desktop = hints.iter().filter(|hint| {
            matches!(
                hint.scheme,
                CategoryScheme::Freedesktop | CategoryScheme::MacosCategory
            )
        });
        let mut main: Option<&'a str> = None;
        for hint in desktop {
            let Some(id) = self.map_hint(hint) else {
                continue;
            };
            let is_main = hint.scheme == CategoryScheme::Freedesktop
                && self
                    .freedesktop_main
                    .iter()
                    .any(|value| value == &hint.value);
            if !is_main {
                return Some(id);
            }
            main.get_or_insert(id);
        }
        main
    }

    fn package_category<'a>(
        &'a self,
        hints: &'a [CategoryHint],
    ) -> (Option<&'a str>, Option<&'a str>) {
        let mut weak: Option<&'a str> = None;
        for hint in hints.iter().filter(|hint| {
            matches!(
                hint.scheme,
                CategoryScheme::DebianSection | CategoryScheme::RpmGroup
            )
        }) {
            let Some(id) = self.map_hint(hint) else {
                continue;
            };
            let section = hint.value.rsplit('/').next().unwrap_or(&hint.value);
            let is_weak = hint.scheme == CategoryScheme::DebianSection
                && self.debian_weak.iter().any(|value| value == section);
            if !is_weak && id != OTHER {
                return (Some(id), weak);
            }
            weak.get_or_insert(id);
        }
        (None, weak)
    }

    /// Category whose keyword list matches most of `words`; ties go to the
    /// smaller category id.
    #[must_use]
    pub fn keyword_category<'a>(&self, words: impl Iterator<Item = &'a str>) -> Option<&str> {
        let words: Vec<&str> = words.filter(|word| !word.is_empty()).collect();
        let mut best: Option<(&str, usize)> = None;
        for (id, keywords) in &self.keywords {
            let count = words
                .iter()
                .filter(|word| keywords.iter().any(|keyword| keyword == *word))
                .count();
            if count > 0 && best.is_none_or(|(_, top)| count > top) {
                best = Some((id, count));
            }
        }
        best.map(|(id, _)| id)
    }
}

/// Folded words for the keyword classifier: the command name, its parts
/// split at `-` and `_`, and every keyword, tag and group hint.
fn keyword_words(name_key: &str, hints: &[CategoryHint]) -> Vec<String> {
    let mut words = vec![name_key.to_owned()];
    words.extend(
        name_key
            .split(['-', '_', '.'])
            .filter(|part| part.len() > 1 && *part != name_key)
            .map(str::to_owned),
    );
    for hint in hints.iter().filter(|hint| {
        matches!(
            hint.scheme,
            CategoryScheme::Keyword | CategoryScheme::PacmanGroup
        )
    }) {
        let folded = fold(&hint.value);
        words.extend(folded.split_whitespace().map(str::to_owned));
        words.push(folded);
    }
    words
}

/// Groups of words with the same meaning.
#[derive(Debug, Clone, Default)]
pub struct Synonyms {
    groups: Vec<Vec<String>>,
    by_word: HashMap<String, Vec<usize>>,
}

impl Synonyms {
    /// The built-in table; an empty one when the embedded data is damaged,
    /// which [`try_builtin`](Self::try_builtin) reports.
    #[must_use]
    pub fn builtin() -> &'static Self {
        static FALLBACK: OnceLock<Synonyms> = OnceLock::new();
        match Self::loaded() {
            Ok(synonyms) => synonyms,
            Err(_) => FALLBACK.get_or_init(Self::default),
        }
    }

    /// The built-in table.
    ///
    /// # Errors
    ///
    /// Returns the parse error of the embedded data file.
    pub fn try_builtin() -> Result<&'static Self, &'static str> {
        Self::loaded().as_ref().map_err(String::as_str)
    }

    fn loaded() -> &'static Result<Synonyms, String> {
        static SYNONYMS: OnceLock<Result<Synonyms, String>> = OnceLock::new();
        SYNONYMS.get_or_init(|| Self::parse(SYNONYMS_DATA))
    }

    fn parse(data: &str) -> Result<Self, String> {
        let file: SynonymsFile =
            toml::from_str(data).map_err(|error| format!("synonyms.toml: {error}"))?;
        let mut synonyms = Self::default();
        synonyms.extend(file.groups);
        Ok(synonyms)
    }

    /// Adds groups, for example from a catalog; words are folded.
    pub fn extend(&mut self, groups: impl IntoIterator<Item = Vec<String>>) {
        for group in groups {
            let folded: Vec<String> = group.iter().map(|word| fold(word)).collect();
            if folded.len() < 2 {
                continue;
            }
            let index = self.groups.len();
            for word in &folded {
                self.by_word.entry(word.clone()).or_default().push(index);
            }
            self.groups.push(folded);
        }
    }

    /// Other words of every group that contains the folded `word`.
    #[must_use]
    pub fn alternatives(&self, word: &str) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for index in self.by_word.get(word).into_iter().flatten() {
            for other in &self.groups[*index] {
                if other != word && !out.contains(&other.as_str()) {
                    out.push(other);
                }
            }
        }
        out
    }

    /// Number of groups.
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Returns `true` when there is no group.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(scheme: CategoryScheme, value: &str) -> CategoryHint {
        CategoryHint::new(scheme, value)
    }

    #[test]
    fn builtin_data_parses() {
        let taxonomy = Taxonomy::builtin();
        assert!(taxonomy.categories().len() >= 24);
        assert!(taxonomy.get(OTHER).is_some());
        let audio = taxonomy
            .get("audio")
            .map(|c| c.title.pick(&["pl".to_owned()]).0);
        assert_eq!(audio, Some("D\u{17a}wi\u{119}k"));
        assert!(Synonyms::builtin().len() > 100);
    }

    #[test]
    fn embedded_data_files_parse_without_error() {
        if let Err(error) = Taxonomy::try_builtin() {
            panic!("{error}");
        }
        if let Err(error) = Synonyms::try_builtin() {
            panic!("{error}");
        }
    }

    #[test]
    fn damaged_data_is_reported_with_the_name_of_the_file() {
        let error = Taxonomy::parse("[[category]\nid = 1", MAPS_DATA).unwrap_err();
        assert!(error.starts_with("taxonomy.toml:"), "{error}");
        let error = Taxonomy::parse(TAXONOMY_DATA, "[debian_section\n").unwrap_err();
        assert!(error.starts_with("category-maps.toml:"), "{error}");
        let error = Synonyms::parse("groups = [[\"a\", 1]]").unwrap_err();
        assert!(error.starts_with("synonyms.toml:"), "{error}");
        assert!(Synonyms::parse("").unwrap().is_empty());
    }

    #[test]
    fn every_mapping_targets_a_taxonomy_id() {
        let taxonomy = Taxonomy::builtin();
        let targets = taxonomy
            .debian
            .values()
            .chain(taxonomy.freedesktop.values())
            .chain(taxonomy.macos.values())
            .chain(taxonomy.rpm.iter().map(|(_, id)| id))
            .chain(taxonomy.man.values())
            .chain(taxonomy.keywords.iter().map(|(id, _)| id));
        for id in targets {
            assert!(taxonomy.get(id).is_some(), "unknown taxonomy id {id}");
        }
    }

    #[test]
    fn mapping_table_cases() {
        let taxonomy = Taxonomy::builtin();
        let cases = [
            (hint(CategoryScheme::DebianSection, "sound"), Some("audio")),
            (
                hint(CategoryScheme::DebianSection, "contrib/net"),
                Some("network"),
            ),
            (hint(CategoryScheme::Freedesktop, "3DGraphics"), Some("3d")),
            (hint(CategoryScheme::Freedesktop, "Unknown"), None),
            (
                hint(
                    CategoryScheme::MacosCategory,
                    "public.app-category.developer-tools",
                ),
                Some("code"),
            ),
            (
                hint(CategoryScheme::RpmGroup, "Productivity/Graphics/3D/Blender"),
                Some("3d"),
            ),
            (
                hint(
                    CategoryScheme::RpmGroup,
                    "productivity/archiving/compression",
                ),
                Some("archives"),
            ),
            (hint(CategoryScheme::RpmGroup, "Unspecified"), None),
            (hint(CategoryScheme::ManSection, "6x"), Some("games")),
            (hint(CategoryScheme::ManSection, "1"), None),
        ];
        for (input, expected) in cases {
            assert_eq!(taxonomy.map_hint(&input), expected, "{input:?}");
        }
    }

    #[test]
    fn classify_prefers_specific_desktop_category() {
        let taxonomy = Taxonomy::builtin();
        let hints = [
            hint(CategoryScheme::Freedesktop, "AudioVideo"),
            hint(CategoryScheme::Freedesktop, "Video"),
            hint(CategoryScheme::Freedesktop, "Player"),
        ];
        let result = taxonomy.classify("mpv", &hints);
        assert_eq!(result.id, "video");
        assert_eq!(result.source, CategorySource::Desktop);
        let main_only = [hint(CategoryScheme::Freedesktop, "Development")];
        assert_eq!(taxonomy.classify("x", &main_only).id, "code");
    }

    #[test]
    fn classify_weak_section_yields_to_keywords() {
        let taxonomy = Taxonomy::builtin();
        let tar = taxonomy.classify("tar", &[hint(CategoryScheme::DebianSection, "utils")]);
        assert_eq!(
            (tar.id.as_str(), tar.source),
            ("archives", CategorySource::Inferred)
        );
        let other = taxonomy.classify(
            "frobnicate",
            &[hint(CategoryScheme::DebianSection, "utils")],
        );
        assert_eq!(
            (other.id.as_str(), other.source),
            ("system", CategorySource::Package)
        );
        let strong = taxonomy.classify("tar", &[hint(CategoryScheme::DebianSection, "devel")]);
        assert_eq!(strong.id, "code");
    }

    #[test]
    fn classify_catalog_man_and_fallback() {
        let taxonomy = Taxonomy::builtin();
        let catalog = taxonomy.classify("x", &[hint(CategoryScheme::Catalog, "video-encoding")]);
        assert_eq!(
            (catalog.id.as_str(), catalog.source),
            ("video-encoding", CategorySource::Catalog)
        );
        let man = taxonomy.classify("frob", &[hint(CategoryScheme::ManSection, "8")]);
        assert_eq!(
            (man.id.as_str(), man.source),
            ("system", CategorySource::Man)
        );
        let tags = taxonomy.classify(
            "frob",
            &[
                hint(CategoryScheme::Keyword, "fuzzing"),
                hint(CategoryScheme::Keyword, "test"),
            ],
        );
        assert_eq!(tags.id, "tests");
        let none = taxonomy.classify("frob", &[]);
        assert_eq!(
            (none.id.as_str(), none.source),
            (OTHER, CategorySource::Inferred)
        );
    }

    #[test]
    fn synonyms_link_polish_and_english() {
        let synonyms = Synonyms::builtin();
        let alternatives = synonyms.alternatives("wideo");
        assert!(alternatives.contains(&"video"));
        assert!(synonyms.alternatives("dzwiek").contains(&"audio"));
        assert!(synonyms.alternatives("nonexistent").is_empty());
        let mut extended = Synonyms::default();
        extended.extend(vec![
            vec!["Napisy".to_owned(), "subtitles".to_owned()],
            vec!["x".to_owned()],
        ]);
        assert_eq!(extended.len(), 1);
        assert_eq!(extended.alternatives("napisy"), vec!["subtitles"]);
    }
}
