// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::index
// Responsibility:   Index of program cards merged from all providers, with its cache.
// Dependencies:     nurt_contracts, rayon, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Index of program cards merged from all providers, with its cache.
//!
//! Providers are read in parallel within a time budget. The data of each
//! provider is cached in `<index dir>/<id>.cbor` together with its
//! fingerprint and reused while the fingerprint is unchanged; the file is
//! written atomically (temporary file and rename) and readable only by the
//! account that wrote it. Without an index directory the index lives in
//! memory only.
//!
//! Entries are merged by name ([`HostEnv::name_key`]): programs in `PATH`
//! first, then package databases, applications, manual pages and external
//! catalogs. A link in `PATH` to another program becomes an alias of that
//! program. A summary is taken, in this order, from a catalog, the manual
//! page, the package (when the package provides one command or the command
//! has the package name), the application entry, or a replacement sentence.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nurt_contracts::config::ProgramsConfig;
use nurt_contracts::env::{Clock, Os};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::model::{
    AliasKind, CategoryHint, CategoryScheme, CategorySource, DescriptionSection, EntryKey,
    EntryKind, GuideRef, LocalizedText, NotFor, PackageRef, RawAlias, RawCategory, RawGuide,
    RawPackage, RawProgram, SourceData,
};
use crate::source::{
    Deadline, Fingerprint, HostEnv, ProgramSource, SourceId, SourceKind, SourceRegistry,
    SourceWarning, WarningCode,
};
use crate::sources::canonical_dir_path;
use crate::taxonomy::{Synonyms, Taxonomy};
use crate::text::fold;

/// Version of the cache file layout.
pub const CACHE_FORMAT: u32 = 1;
/// Default time budget of building the index.
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(3);
/// Source name of the built-in taxonomy in category records.
pub const TAXONOMY_SOURCE: &str = "taxonomy";
const CACHE_SUFFIX: &str = "cbor";

/// Rank of a summary; a lower rank wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum SummaryRank {
    Catalog,
    Man,
    Package,
    App,
}

/// Program card after merging all providers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Card {
    /// Name of the program, application or guide.
    pub name: String,
    /// Kind.
    pub kind: EntryKind,
    /// One sentence in all known languages.
    pub summary: LocalizedText,
    /// Top-level category id.
    pub category: String,
    /// Subcategory id, when a catalog places the program in one.
    pub subcategory: Option<String>,
    /// Origin of the category.
    pub category_source: Option<CategorySource>,
    /// The program exists on this computer.
    pub installed: bool,
    /// Local path.
    pub path: Option<String>,
    /// Further local files with the same name.
    pub alternatives: Vec<String>,
    /// Path named by a catalog.
    pub catalog_path: Option<String>,
    /// Version.
    pub version: Option<String>,
    /// Kind of the provider that gave `version`.
    pub version_source: Option<SourceKind>,
    /// Alternative names.
    pub aliases: Vec<RawAlias>,
    /// Providers that contributed, in priority order.
    pub sources: Vec<SourceId>,
    /// Kind of the main provider.
    pub source: Option<SourceKind>,
    /// Package.
    pub package: Option<PackageRef>,
    /// Package name named without a package manager.
    pub package_name: Option<String>,
    /// Description of the package.
    pub package_description: Option<LocalizedText>,
    /// Application id.
    pub app_id: Option<String>,
    /// Application display name.
    pub app_name: Option<LocalizedText>,
    /// Home page.
    pub homepage: Option<String>,
    /// Licence.
    pub license: Option<String>,
    /// Platforms named by a catalog.
    pub platforms: Vec<String>,
    /// Tags and keywords.
    pub tags: Vec<String>,
    /// Verification date of the catalog entry.
    pub verified: Option<String>,
    /// Paragraph for listings.
    pub overview: Option<LocalizedText>,
    /// Full description.
    pub description: Option<LocalizedText>,
    /// Structured parts of the description.
    pub sections: Vec<DescriptionSection>,
    /// Situations of use.
    pub when_to_use: Vec<LocalizedText>,
    /// Tasks the program is not for.
    pub not_for: Vec<NotFor>,
    /// Example invocation.
    pub example: Option<String>,
    /// Guide and the provider that holds it.
    pub guide: Option<(SourceId, GuideRef)>,
    /// The guide is optional.
    pub guide_optional: bool,
    /// The guide is required.
    pub guide_required: bool,
    /// Related programs.
    pub related: Vec<String>,
    /// Input description.
    pub input: Option<String>,
    /// Output description.
    pub output: Option<String>,
    /// Keys for details, one per contributing provider.
    pub keys: Vec<EntryKey>,
    /// Priority of the best provider; a lower value wins ties.
    pub rank: usize,
    /// Folded search fields.
    pub fields: SearchFields,
    summaries: Vec<(SummaryRank, LocalizedText)>,
    hints: Vec<CategoryHint>,
}

/// Folded fields of a card used by ranking.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchFields {
    /// Name.
    pub name: String,
    /// Situations of use, joined.
    pub when: String,
    /// Summary in all languages.
    pub purpose: String,
    /// Category id.
    pub category: String,
    /// Category title in all languages.
    pub category_title: String,
    /// Guide description.
    pub guide: String,
    /// Tags and keywords.
    pub tags: String,
    /// Package name.
    pub package: String,
    /// Package description.
    pub package_description: String,
    /// Alias names.
    pub aliases: Vec<String>,
}

/// Category known to the index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexCategory {
    /// Id.
    pub id: String,
    /// Title.
    pub title: LocalizedText,
    /// One sentence.
    pub summary: Option<LocalizedText>,
    /// Description.
    pub description: Option<LocalizedText>,
    /// Parent of a subcategory.
    pub parent: Option<String>,
    /// Alternative names.
    pub aliases: Vec<String>,
    /// Definitions: `taxonomy` or `catalog:<id>`.
    pub sources: Vec<String>,
    /// Position among the categories.
    pub order: usize,
}

/// Standalone guide known to the index.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexGuide {
    /// Guide data.
    pub guide: RawGuide,
    /// Provider.
    pub source: SourceId,
    /// Folded name.
    pub folded_name: String,
    /// Folded summary.
    pub folded_summary: String,
    /// Priority of the provider; a lower value wins ties.
    pub rank: usize,
}

/// Merged index of all providers.
pub struct ProgramIndex {
    cards: Vec<Card>,
    by_key: HashMap<String, usize>,
    aliases: HashMap<String, (usize, AliasKind)>,
    guides: Vec<IndexGuide>,
    linked_guides: HashSet<String>,
    categories: Vec<IndexCategory>,
    synonyms: Synonyms,
    warnings: Vec<SourceWarning>,
    sources: Vec<Arc<dyn ProgramSource>>,
    env: HostEnv,
    complete: bool,
}

impl fmt::Debug for ProgramIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgramIndex")
            .field("cards", &self.cards.len())
            .field("guides", &self.guides.len())
            .field("categories", &self.categories.len())
            .field("warnings", &self.warnings.len())
            .finish_non_exhaustive()
    }
}

/// Options of [`ProgramIndex::load`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOptions {
    /// Directory of the cache files; `None` keeps the index in memory.
    pub cache_dir: Option<PathBuf>,
    /// Time budget of reading providers.
    pub budget: Duration,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            cache_dir: None,
            budget: DEFAULT_BUDGET,
        }
    }
}

impl IndexOptions {
    /// Options from the configuration: `programs.index_dir`, otherwise the
    /// cache directory of the user. With a test root there is no default
    /// directory.
    #[must_use]
    pub fn from_config(config: &ProgramsConfig, env: &HostEnv) -> Self {
        let cache_dir = config
            .index_dir
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| default_cache_dir(env).filter(|_| env.root().is_none()));
        Self {
            cache_dir,
            budget: DEFAULT_BUDGET,
        }
    }
}

/// Cache directory of the user for the index.
#[must_use]
pub fn default_cache_dir(env: &HostEnv) -> Option<PathBuf> {
    match env.os() {
        Os::Windows => env.var_text("LOCALAPPDATA").map(|dir| {
            PathBuf::from(dir)
                .join("Nurt")
                .join("cache")
                .join("programs")
        }),
        Os::Macos => env
            .home()
            .map(|home| home.join("Library/Caches/Nurt/programs")),
        _ => env
            .xdg_dir("XDG_CACHE_HOME")
            .or_else(|| env.home().map(|home| home.join(".cache")))
            .map(|dir| dir.join("nurt").join("programs")),
    }
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    cache_format: u32,
    nurt_version: String,
    fingerprint: Fingerprint,
    data: SourceData,
}

/// Data of one provider read for the index.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// Provider id.
    pub id: SourceId,
    /// Data; empty when the provider failed.
    pub data: SourceData,
    /// Fingerprint at reading time.
    pub fingerprint: Option<Fingerprint>,
}

/// Reads one provider: from the cache when its fingerprint is unchanged,
/// otherwise in full, then stores the cache.
#[must_use]
pub fn load_source(
    source: &dyn ProgramSource,
    env: &HostEnv,
    options: &IndexOptions,
    deadline: &Deadline<'_>,
) -> Loaded {
    let id = source.id().clone();
    let fingerprint = source.fingerprint(env).ok();
    let cache_file = options.cache_dir.as_ref().map(|dir| cache_path(dir, &id));
    if let (Some(file), Some(fingerprint)) = (&cache_file, &fingerprint)
        && let Some(data) = read_cache(file, fingerprint)
    {
        return Loaded {
            id,
            data,
            fingerprint: Some(fingerprint.clone()),
        };
    }
    match source.load(env, deadline) {
        Ok(data) => {
            if let (Some(file), Some(fingerprint)) = (&cache_file, &fingerprint) {
                write_cache(file, fingerprint, &data);
            }
            Loaded {
                id,
                data,
                fingerprint,
            }
        }
        Err(error) => Loaded {
            id,
            data: SourceData {
                warnings: vec![error.into()],
                ..SourceData::default()
            },
            fingerprint: None,
        },
    }
}

/// Cache file of a provider: the id with characters outside
/// `[a-z0-9._-]` replaced by `_`.
#[must_use]
pub fn cache_path(dir: &Path, id: &SourceId) -> PathBuf {
    let name: String = id
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{name}.{CACHE_SUFFIX}"))
}

fn read_cache(file: &Path, fingerprint: &Fingerprint) -> Option<SourceData> {
    let bytes = std::fs::read(file).ok()?;
    let cache: CacheFile = ciborium::from_reader(bytes.as_slice()).ok()?;
    (cache.cache_format == CACHE_FORMAT
        && cache.nurt_version == env!("CARGO_PKG_VERSION")
        && cache.fingerprint == *fingerprint)
        .then_some(cache.data)
}

/// Writes the cache file atomically; failures leave the index in memory
/// only.
fn write_cache(file: &Path, fingerprint: &Fingerprint, data: &SourceData) {
    let Some(dir) = file.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let cache = CacheFile {
        cache_format: CACHE_FORMAT,
        nurt_version: env!("CARGO_PKG_VERSION").to_owned(),
        fingerprint: fingerprint.clone(),
        data: data.clone(),
    };
    let mut bytes = Vec::new();
    if ciborium::into_writer(&cache, &mut bytes).is_err() {
        return;
    }
    let temporary = temporary_path(file);
    if write_private(&temporary, &bytes).is_ok() && std::fs::rename(&temporary, file).is_err() {
        let _ignored = std::fs::remove_file(&temporary);
    }
}

/// Name of the temporary file of one writer of the cache file `file`: the
/// process and a counter make it unique for every call, so concurrent
/// writers of one provider never share a file.
fn temporary_path(file: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    file.with_extension(format!(
        "{CACHE_SUFFIX}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    const USER_READ_WRITE: u32 = 0o600;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(USER_READ_WRITE)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Order of merging providers of each kind.
fn kind_rank(kind: SourceKind) -> usize {
    match kind {
        SourceKind::Path => 0,
        SourceKind::Package => 1,
        SourceKind::App => 2,
        SourceKind::Description => 3,
        SourceKind::Catalog => 4,
    }
}

impl ProgramIndex {
    /// Reads every detected provider of `registry` in parallel and merges
    /// their data. Providers that fail or exceed the budget contribute a
    /// warning instead of data.
    #[must_use]
    pub fn load(
        registry: &SourceRegistry,
        env: &HostEnv,
        options: &IndexOptions,
        clock: &dyn Clock,
    ) -> Self {
        let deadline = Deadline::after(clock, options.budget);
        let sources: Vec<Arc<dyn ProgramSource>> = registry
            .sources()
            .iter()
            .filter(|source| source.detect(env))
            .cloned()
            .collect();
        let loaded: Vec<Loaded> = sources
            .par_iter()
            .map(|source| load_source(source.as_ref(), env, options, &deadline))
            .collect();
        Self::build(sources, loaded, env)
    }

    /// Merges data already read.
    #[must_use]
    pub fn build(sources: Vec<Arc<dyn ProgramSource>>, loaded: Vec<Loaded>, env: &HostEnv) -> Self {
        let mut builder = Builder::new(env);
        let mut order: Vec<(usize, &Loaded)> = loaded
            .iter()
            .filter_map(|item| {
                let position = sources.iter().position(|source| source.id() == &item.id)?;
                Some((position, item))
            })
            .collect();
        order.sort_by_key(|(position, item)| {
            let source = &sources[*position];
            (
                kind_rank(source.kind()),
                -i64::from(source.priority()),
                *position,
                item.id.clone(),
            )
        });
        for (rank, (position, item)) in order.iter().enumerate() {
            let source = &sources[*position];
            builder.add(source.as_ref(), &item.data, rank);
        }
        let mut index = builder.finish(sources);
        index.complete = loaded.iter().all(|item| item.fingerprint.is_some());
        index
    }

    /// Cards in index order.
    #[must_use]
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    /// Card at a position.
    #[must_use]
    pub fn card(&self, index: usize) -> Option<&Card> {
        self.cards.get(index)
    }

    /// Position of the card named `name`, or of the program `name` is an
    /// alias of, with the kind of the alias.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<(usize, Option<AliasKind>)> {
        let key = self.env.name_key(name);
        if let Some(index) = self.by_key.get(&key) {
            return Some((*index, None));
        }
        self.aliases
            .get(&key)
            .map(|(index, kind)| (*index, Some(*kind)))
    }

    /// Alias names with the position of their card.
    pub fn alias_names(&self) -> impl Iterator<Item = (&str, usize)> {
        self.aliases
            .iter()
            .map(|(name, (index, _))| (name.as_str(), *index))
    }

    /// Standalone guides.
    #[must_use]
    pub fn guides(&self) -> &[IndexGuide] {
        &self.guides
    }

    /// Returns `true` when a program refers to the guide.
    #[must_use]
    pub fn guide_is_linked(&self, name: &str) -> bool {
        self.linked_guides.contains(name)
    }

    /// Categories: taxonomy first, then catalog categories in order.
    #[must_use]
    pub fn categories(&self) -> &[IndexCategory] {
        &self.categories
    }

    /// Category with an id or alias, compared after folding.
    #[must_use]
    pub fn category(&self, name: &str) -> Option<&IndexCategory> {
        let key = fold(name);
        self.categories
            .iter()
            .find(|category| fold(&category.id) == key)
            .or_else(|| {
                self.categories
                    .iter()
                    .find(|category| category.aliases.iter().any(|alias| fold(alias) == key))
            })
    }

    /// Synonyms: built-in table and catalog tables.
    #[must_use]
    pub fn synonyms(&self) -> &Synonyms {
        &self.synonyms
    }

    /// Warnings of the providers.
    #[must_use]
    pub fn warnings(&self) -> &[SourceWarning] {
        &self.warnings
    }

    /// Returns `false` when a provider failed or its state could not be
    /// determined, so that a later call should read the providers again.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Environment the index was built for.
    #[must_use]
    pub fn env(&self) -> &HostEnv {
        &self.env
    }

    /// Provider with an id.
    #[must_use]
    pub fn source(&self, id: &SourceId) -> Option<&Arc<dyn ProgramSource>> {
        self.sources.iter().find(|source| source.id() == id)
    }

    /// Providers of the index.
    #[must_use]
    pub fn sources(&self) -> &[Arc<dyn ProgramSource>] {
        &self.sources
    }
}

/// Entry that names another program as its main one, kept until every
/// provider has been added.
struct PendingAlias {
    target: String,
    kind: AliasKind,
    source: SourceKind,
    id: SourceId,
    rank: usize,
    raw: RawProgram,
}

struct Builder<'a> {
    env: &'a HostEnv,
    cards: Vec<Card>,
    by_key: HashMap<String, usize>,
    aliases: HashMap<String, (usize, AliasKind)>,
    pending_aliases: Vec<PendingAlias>,
    files: HashMap<String, PackageRef>,
    packages: HashMap<PackageRef, RawPackage>,
    guides: Vec<IndexGuide>,
    linked_guides: HashSet<String>,
    categories: Vec<IndexCategory>,
    catalog_parents: HashMap<(SourceId, String), (String, Option<String>)>,
    synonyms: Synonyms,
    warnings: Vec<SourceWarning>,
}

impl<'a> Builder<'a> {
    fn new(env: &'a HostEnv) -> Self {
        let categories = Taxonomy::builtin()
            .categories()
            .iter()
            .map(|category| IndexCategory {
                id: category.id.clone(),
                title: category.title.clone(),
                summary: Some(category.summary.clone()),
                description: None,
                parent: None,
                aliases: Vec::new(),
                sources: vec![TAXONOMY_SOURCE.to_owned()],
                order: category.order,
            })
            .collect();
        Self {
            env,
            cards: Vec::new(),
            by_key: HashMap::new(),
            aliases: HashMap::new(),
            pending_aliases: Vec::new(),
            files: HashMap::new(),
            packages: HashMap::new(),
            guides: Vec::new(),
            linked_guides: HashSet::new(),
            categories,
            catalog_parents: HashMap::new(),
            synonyms: Synonyms::builtin().clone(),
            warnings: Vec::new(),
        }
    }

    fn add(&mut self, source: &dyn ProgramSource, data: &SourceData, rank: usize) {
        let id = source.id().clone();
        self.warnings.extend(data.warnings.iter().cloned());
        for (file, package) in &data.file_packages {
            self.files.insert(file.clone(), package.clone());
        }
        for package in &data.packages {
            let reference = PackageRef {
                manager: id.clone(),
                name: package.name.clone(),
            };
            self.packages.insert(reference, package.clone());
        }
        self.synonyms.extend(data.synonyms.iter().cloned());
        if source.kind() == SourceKind::Catalog {
            self.add_categories(&id, &data.categories);
        }
        for program in &data.programs {
            self.add_program(source.kind(), &id, program, rank);
        }
        for guide in &data.guides {
            self.guides.push(IndexGuide {
                folded_name: fold(&guide.name),
                folded_summary: guide.summary.as_ref().map(fold_all).unwrap_or_default(),
                guide: guide.clone(),
                source: id.clone(),
                rank,
            });
        }
    }

    fn add_categories(&mut self, id: &SourceId, categories: &[RawCategory]) {
        for raw in categories {
            let target = raw.same_as.clone().unwrap_or_else(|| raw.id.clone());
            let parent = raw.parent.as_ref().map(|parent| {
                categories
                    .iter()
                    .find(|other| other.id == *parent)
                    .and_then(|other| other.same_as.clone())
                    .unwrap_or_else(|| parent.clone())
            });
            let top = parent.clone().unwrap_or_else(|| target.clone());
            let sub = parent.is_some().then(|| target.clone());
            self.catalog_parents
                .insert((id.clone(), raw.id.clone()), (top, sub));
            let order = self.categories.len();
            if let Some(existing) = self
                .categories
                .iter_mut()
                .find(|category| category.id == target)
            {
                if existing
                    .sources
                    .iter()
                    .all(|source| source == TAXONOMY_SOURCE)
                {
                    existing.title = raw.title.clone();
                    existing.summary.clone_from(&raw.summary);
                    existing.description.clone_from(&raw.description);
                }
                existing.sources.push(id.to_string());
                existing.aliases.extend(raw.aliases.iter().cloned());
                continue;
            }
            self.categories.push(IndexCategory {
                id: target,
                title: raw.title.clone(),
                summary: raw.summary.clone(),
                description: raw.description.clone(),
                parent,
                aliases: raw.aliases.clone(),
                sources: vec![id.to_string()],
                order: raw.order.map_or(order, |order| {
                    usize::try_from(order).unwrap_or(if order < 0 { 0 } else { usize::MAX })
                }),
            });
        }
    }

    fn add_program(&mut self, kind: SourceKind, id: &SourceId, raw: &RawProgram, rank: usize) {
        if let Some(guide) = &raw.guide {
            self.linked_guides.insert(guide.name.clone());
        }
        if let (Some(target), Some(alias_kind)) = (&raw.alias_of, raw.alias_kind) {
            self.pending_aliases.push(PendingAlias {
                target: target.clone(),
                kind: alias_kind,
                source: kind,
                id: id.clone(),
                rank,
                raw: raw.clone(),
            });
            return;
        }
        let executable = raw
            .executable
            .as_deref()
            .filter(|_| kind == SourceKind::App)
            .and_then(|name| self.find(name));
        let existing = self
            .find(&raw.name)
            .or(executable)
            .or_else(|| raw.aliases.iter().find_map(|alias| self.find(&alias.name)));
        let index = match (existing, kind) {
            (Some(index), _) => index,
            (None, SourceKind::Description) => return,
            (None, SourceKind::App) if raw.path.is_none() || !self.in_path(raw) => {
                self.new_app_card(raw, rank)
            }
            (None, _) => self.new_card(&raw.name, raw.kind, rank),
        };
        let parents = &self.catalog_parents;
        let category = raw
            .category_hints
            .iter()
            .find(|hint| hint.scheme == CategoryScheme::Catalog)
            .and_then(|hint| parents.get(&(id.clone(), hint.value.clone())).cloned());
        let card = &mut self.cards[index];
        merge(card, kind, id, raw, category);
        for alias in &raw.aliases {
            let key = self.env.name_key(&alias.name);
            if !self.by_key.contains_key(&key) {
                self.aliases.entry(key).or_insert((index, alias.kind));
            }
        }
    }

    fn in_path(&self, raw: &RawProgram) -> bool {
        self.find(&raw.name).is_some()
    }

    fn find(&self, name: &str) -> Option<usize> {
        let key = self.env.name_key(name);
        self.by_key
            .get(&key)
            .copied()
            .or_else(|| self.aliases.get(&key).map(|(index, _)| *index))
    }

    fn new_card(&mut self, name: &str, kind: EntryKind, rank: usize) -> usize {
        let index = self.cards.len();
        self.cards.push(Card {
            name: name.to_owned(),
            kind,
            rank,
            ..Card::default()
        });
        self.by_key.insert(self.env.name_key(name), index);
        index
    }

    fn new_app_card(&mut self, raw: &RawProgram, rank: usize) -> usize {
        let display = raw
            .app_name
            .as_ref()
            .map_or_else(|| raw.name.clone(), |name| name.text.clone());
        let index = self.new_card(&display, EntryKind::App, rank);
        if display != raw.name {
            let key = self.env.name_key(&raw.name);
            self.aliases.entry(key).or_insert((index, AliasKind::Alias));
            self.cards[index]
                .aliases
                .push(RawAlias::new(raw.name.clone(), AliasKind::Alias));
        }
        index
    }

    fn finish(mut self, sources: Vec<Arc<dyn ProgramSource>>) -> ProgramIndex {
        self.resolve_aliases();
        self.attach_packages();
        let taxonomy = Taxonomy::builtin();
        let titles: HashMap<String, String> = self
            .categories
            .iter()
            .map(|category| (category.id.clone(), fold_all(&category.title)))
            .collect();
        for card in &mut self.cards {
            finalize(card, taxonomy, &titles);
        }
        ProgramIndex {
            cards: self.cards,
            by_key: self.by_key,
            aliases: self.aliases,
            guides: self.guides,
            linked_guides: self.linked_guides,
            categories: self.categories,
            synonyms: self.synonyms,
            warnings: self.warnings,
            sources,
            env: self.env.clone(),
            complete: true,
        }
    }

    fn resolve_aliases(&mut self) {
        for pending in std::mem::take(&mut self.pending_aliases) {
            let name = pending.raw.name.clone();
            let key = self.env.name_key(&name);
            if self.by_key.contains_key(&key) {
                continue;
            }
            match self.find(&pending.target) {
                Some(index) => {
                    self.aliases.insert(key, (index, pending.kind));
                    let card = &mut self.cards[index];
                    if !card.aliases.iter().any(|alias| alias.name == name) {
                        card.aliases.push(RawAlias::new(name, pending.kind));
                    }
                }
                None => {
                    let mut raw = pending.raw;
                    raw.alias_of = None;
                    raw.alias_kind = None;
                    self.add_program(pending.source, &pending.id, &raw, pending.rank);
                }
            }
        }
    }

    fn attach_packages(&mut self) {
        let mut executables: HashMap<PackageRef, usize> = HashMap::new();
        for reference in self.files.values() {
            *executables.entry(reference.clone()).or_default() += 1;
        }
        for index in 0..self.cards.len() {
            let Some(reference) = self.package_of(index) else {
                continue;
            };
            let Some(package) = self.packages.get(&reference).cloned() else {
                continue;
            };
            let single = executables.get(&reference).copied().unwrap_or_default() <= 1;
            let card = &mut self.cards[index];
            let own_name = card.name == package.name;
            apply_package(card, &reference, &package, single || own_name);
        }
    }

    fn package_of(&self, index: usize) -> Option<PackageRef> {
        let card = &self.cards[index];
        if let Some(package) = &card.package {
            return Some(package.clone());
        }
        if let Some(name) = &card.package_name
            && let Some(found) = self
                .packages
                .keys()
                .filter(|known| known.name == *name)
                .min()
        {
            return Some(found.clone());
        }
        let path = card.path.as_ref()?;
        if let Some(found) = self.files.get(path) {
            return Some(found.clone());
        }
        let canonical = canonical_dir_path(self.env, Path::new(path));
        if let Some(found) = self.files.get(&canonical.to_string_lossy().into_owned()) {
            return Some(found.clone());
        }
        let resolved = crate::sources::resolve_links(self.env, Path::new(path))?;
        self.files
            .get(&resolved.to_string_lossy().into_owned())
            .cloned()
    }
}

fn apply_package(card: &mut Card, reference: &PackageRef, package: &RawPackage, summary: bool) {
    card.package.get_or_insert_with(|| reference.clone());
    if !card.sources.contains(&reference.manager) {
        card.sources.push(reference.manager.clone());
    }
    if card.version.is_none() && package.version.is_some() {
        card.version.clone_from(&package.version);
        card.version_source = Some(SourceKind::Package);
    }
    if card.homepage.is_none() {
        card.homepage.clone_from(&package.homepage);
    }
    if card.license.is_none() {
        card.license.clone_from(&package.license);
    }
    if card.package_description.is_none() {
        card.package_description = package
            .description
            .clone()
            .or_else(|| package.summary.clone());
    }
    if summary && let Some(text) = &package.summary {
        card.summaries.push((SummaryRank::Package, text.clone()));
    }
    if let Some(section) = &package.section {
        let scheme = if reference.manager.as_str() == SourceId::RPM {
            CategoryScheme::RpmGroup
        } else {
            CategoryScheme::DebianSection
        };
        card.hints.push(CategoryHint::new(scheme, section.clone()));
    }
    card.hints.extend(
        package
            .groups
            .iter()
            .map(|group| CategoryHint::new(CategoryScheme::PacmanGroup, group.clone())),
    );
}

fn merge(
    card: &mut Card,
    kind: SourceKind,
    id: &SourceId,
    raw: &RawProgram,
    category: Option<(String, Option<String>)>,
) {
    if !card.sources.contains(id) {
        card.sources.push(id.clone());
    }
    if card.source.is_none() && kind != SourceKind::Description {
        card.source = Some(kind);
    }
    if let Some(key) = &raw.key {
        card.keys.push(key.clone());
    }
    merge_location(card, kind, raw);
    merge_texts(card, kind, raw);
    if let Some((top, sub)) = category {
        card.category = top;
        card.subcategory = sub;
        card.category_source = Some(CategorySource::Catalog);
    } else {
        card.hints.extend(raw.category_hints.iter().cloned());
    }
    fill(&mut card.package, &raw.package);
    fill(&mut card.package_name, &raw.package_name);
    fill(&mut card.app_id, &raw.app_id);
    fill(&mut card.app_name, &raw.app_name);
    fill(&mut card.homepage, &raw.homepage);
    fill(&mut card.license, &raw.license);
    extend_unique(&mut card.tags, &raw.tags);
    extend_unique(&mut card.platforms, &raw.platforms);
    extend_unique(&mut card.related, &raw.related);
    for alias in &raw.aliases {
        if alias.name != card.name && !card.aliases.iter().any(|known| known.name == alias.name) {
            card.aliases.push(alias.clone());
        }
    }
    if kind == SourceKind::Catalog {
        merge_catalog(card, id, raw);
    }
}

fn merge_location(card: &mut Card, kind: SourceKind, raw: &RawProgram) {
    match kind {
        SourceKind::Catalog => {
            fill(
                &mut card.catalog_path,
                &raw.path.clone().or_else(|| raw.catalog_path.clone()),
            );
            if !card.installed {
                card.installed = raw.installed.unwrap_or(false);
            }
        }
        SourceKind::Description => {}
        _ => {
            fill(&mut card.path, &raw.path);
            extend_unique(&mut card.alternatives, &raw.alternatives);
            card.installed |= raw.installed.unwrap_or(true);
            if card.version.is_none() && raw.version.is_some() {
                card.version.clone_from(&raw.version);
                card.version_source = Some(kind);
            }
        }
    }
}

fn merge_texts(card: &mut Card, kind: SourceKind, raw: &RawProgram) {
    let Some(summary) = raw.summary.clone().filter(|text| !text.is_empty()) else {
        return;
    };
    let rank = match kind {
        SourceKind::Catalog => SummaryRank::Catalog,
        SourceKind::Description => SummaryRank::Man,
        SourceKind::Package => SummaryRank::App,
        SourceKind::App | SourceKind::Path => SummaryRank::App,
    };
    card.summaries.push((rank, summary));
}

fn merge_catalog(card: &mut Card, id: &SourceId, raw: &RawProgram) {
    if card.version.is_none() && raw.version.is_some() {
        card.version.clone_from(&raw.version);
        card.version_source = Some(SourceKind::Catalog);
    }
    fill(&mut card.verified, &raw.verified);
    fill(&mut card.overview, &raw.overview);
    fill(&mut card.description, &raw.description);
    fill(&mut card.example, &raw.example);
    fill(&mut card.input, &raw.input);
    fill(&mut card.output, &raw.output);
    if card.sections.is_empty() {
        card.sections.clone_from(&raw.sections);
    }
    if card.when_to_use.is_empty() {
        card.when_to_use.clone_from(&raw.when_to_use);
    }
    if card.not_for.is_empty() {
        card.not_for.clone_from(&raw.not_for);
    }
    if card.guide.is_none() {
        if let Some(guide) = &raw.guide {
            card.guide = Some((id.clone(), guide.clone()));
            card.guide_optional = raw.guide_optional;
            card.guide_required = raw.guide_required;
        } else {
            card.guide_optional |= raw.guide_optional;
            card.guide_required |= raw.guide_required;
        }
    }
}

fn finalize(card: &mut Card, taxonomy: &Taxonomy, titles: &HashMap<String, String>) {
    card.summaries.sort_by_key(|(rank, _)| *rank);
    card.summary = card
        .summaries
        .first()
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| replacement_summary(card));
    if card.category_source.is_none() {
        let classification = taxonomy.classify(&card.name, &card.hints);
        card.category = classification.id;
        card.category_source = Some(classification.source);
    }
    if card.kind == EntryKind::Program
        && !card.installed
        && card.source != Some(SourceKind::Catalog)
    {
        card.installed = card.path.is_some();
    }
    let when: Vec<String> = card.when_to_use.iter().map(fold_all).collect();
    let mut tags = card.tags.clone();
    if let Some(name) = &card.app_name {
        tags.extend(name.variants().map(str::to_owned));
    }
    card.fields = SearchFields {
        name: fold(&card.name),
        when: when.join(" "),
        purpose: fold_all(&card.summary),
        category: fold(&card.category),
        category_title: titles.get(&card.category).cloned().unwrap_or_default(),
        guide: card
            .guide
            .as_ref()
            .and_then(|(_, guide)| guide.description.as_ref())
            .map(fold_all)
            .unwrap_or_default(),
        tags: fold(&tags.join(" ")),
        package: card
            .package
            .as_ref()
            .map(|package| fold(&package.name))
            .unwrap_or_default(),
        package_description: card
            .package_description
            .as_ref()
            .map(fold_all)
            .unwrap_or_default(),
        aliases: card.aliases.iter().map(|alias| fold(&alias.name)).collect(),
    };
}

/// Summary of a card without any description: the directory of its file.
fn replacement_summary(card: &Card) -> LocalizedText {
    let place = card
        .path
        .as_deref()
        .and_then(|path| Path::new(path).parent())
        .map(|dir| dir.display().to_string());
    match (card.kind, place) {
        (EntryKind::App, _) => LocalizedText::in_lang("Application.", "en"),
        (_, Some(dir)) => LocalizedText::in_lang(format!("Executable in {dir}."), "en"),
        _ => LocalizedText::in_lang("Program.", "en"),
    }
}

/// Folded text of every language variant, joined with new lines.
#[must_use]
pub fn fold_all(text: &LocalizedText) -> String {
    text.variants().map(fold).collect::<Vec<_>>().join("\n")
}

fn fill<T: Clone>(target: &mut Option<T>, value: &Option<T>) {
    if target.is_none() {
        target.clone_from(value);
    }
}

fn extend_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
}

/// Index kept for the lifetime of the process and rebuilt when any
/// provider fingerprint or the environment changes.
#[derive(Default)]
pub struct IndexCache {
    slot: Mutex<Option<(String, Arc<ProgramIndex>)>>,
}

impl fmt::Debug for IndexCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexCache").finish_non_exhaustive()
    }
}

impl IndexCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The cache of the process.
    #[must_use]
    pub fn global() -> &'static Self {
        static CACHE: std::sync::OnceLock<IndexCache> = std::sync::OnceLock::new();
        CACHE.get_or_init(Self::new)
    }

    /// Index for the registry and environment: the cached one when every
    /// fingerprint is unchanged and no provider was cut by the budget,
    /// otherwise a new one.
    pub fn get(
        &self,
        registry: &SourceRegistry,
        env: &HostEnv,
        options: &IndexOptions,
        clock: &dyn Clock,
    ) -> Arc<ProgramIndex> {
        let key = state_key(registry, env, options);
        let mut slot = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached, index)) = slot.as_ref()
            && *cached == key
        {
            return Arc::clone(index);
        }
        let index = Arc::new(ProgramIndex::load(registry, env, options, clock));
        let cut_short = index
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::IndexPartial);
        if index.is_complete() && !cut_short {
            *slot = Some((key, Arc::clone(&index)));
        }
        index
    }
}

fn state_key(registry: &SourceRegistry, env: &HostEnv, options: &IndexOptions) -> String {
    let mut key = format!("{env:?}|{options:?}");
    for source in registry.sources() {
        key.push('|');
        key.push_str(source.id().as_str());
        if source.detect(env)
            && let Ok(fingerprint) = source.fingerprint(env)
        {
            key.push('=');
            key.push_str(fingerprint.as_str());
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_cache_files_are_unique_per_writer() {
        let file = Path::new("/c/dpkg.cbor");
        let first = temporary_path(file);
        let second = temporary_path(file);
        assert_ne!(first, second);
        assert_eq!(first.parent(), file.parent());
        assert!(first.to_string_lossy().ends_with(".tmp"));
        assert!(
            first
                .to_string_lossy()
                .contains(&std::process::id().to_string())
        );
    }

    fn category(id: &str, order: Option<i64>) -> RawCategory {
        RawCategory {
            id: id.to_owned(),
            title: LocalizedText::new(id),
            order,
            ..RawCategory::default()
        }
    }

    #[test]
    fn cache_directory_follows_the_conventions_of_each_system() {
        let linux = HostEnv::new(Os::Linux).with_home("/home/user");
        assert_eq!(
            default_cache_dir(&linux),
            Some(PathBuf::from("/home/user/.cache/nurt/programs"))
        );
        let xdg = linux.with_var("XDG_CACHE_HOME", "/var/cache/ada");
        assert_eq!(
            default_cache_dir(&xdg),
            Some(PathBuf::from("/var/cache/ada/nurt/programs"))
        );
        let mac = HostEnv::new(Os::Macos).with_home("/Users/user");
        assert_eq!(
            default_cache_dir(&mac),
            Some(PathBuf::from("/Users/user/Library/Caches/Nurt/programs"))
        );
        let windows = HostEnv::new(Os::Windows).with_var("LOCALAPPDATA", "C:\\Users\\user");
        let dir = default_cache_dir(&windows).unwrap();
        assert!(dir.ends_with("Nurt/cache/programs"));
        assert_eq!(default_cache_dir(&HostEnv::new(Os::Linux)), None);
    }

    #[test]
    fn negative_category_order_sorts_first_not_last() {
        let env = HostEnv::new(Os::Linux);
        let mut builder = Builder::new(&env);
        let source = SourceId::catalog("sample");
        builder.add_categories(
            &source,
            &[
                category("first-cat", Some(-5)),
                category("middle-cat", Some(7)),
                category("unordered-cat", None),
            ],
        );
        let order_of = |id: &str| {
            builder
                .categories
                .iter()
                .find(|category| category.id == id)
                .map(|category| category.order)
        };
        assert_eq!(order_of("first-cat"), Some(0));
        assert_eq!(order_of("middle-cat"), Some(7));
        assert!(order_of("unordered-cat").is_some_and(|order| order < usize::MAX / 2));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
