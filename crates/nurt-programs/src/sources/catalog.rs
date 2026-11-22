// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! External catalogs in the format `nurt-program-catalog/1`.
//!
//! [`CatalogProvider`] reads one configured catalog with
//! [`read_catalog`](crate::catalog::read_catalog) and turns it into neutral
//! entries. A catalog that cannot be read yields no entries and one warning
//! (`catalog_invalid` or `catalog_format`); faulty objects are skipped with
//! `catalog_entry_invalid`, up to [`MAX_WARNINGS`] warnings per catalog.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Deserialize;

use crate::catalog::{
    self, Catalog, CatalogError, CatalogErrorKind, Diagnostic, DiagnosticCode, I18N_DIR,
    INDEX_FILE, LoadedCatalog, NotForTask, ReadOptions, Section,
};
use crate::model::{
    CategoryHint, CategoryScheme, DescriptionSection, Detail, DetailWant, EntryKey, GuideBody,
    GuideRef, LocalizedText, NotFor, RawAlias, RawCategory, RawGuide, RawProgram, SourceData,
};
use crate::source::{
    CatalogSpec, Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError,
    SourceErrorKind, SourceId, SourceKind, SourceWarning, WarningCode,
};

/// Limit of warnings reported for one catalog; further ones are counted in
/// a single `warnings_truncated` warning.
pub const MAX_WARNINGS: usize = 20;

/// Directory of guides by convention, part of the fingerprint.
const GUIDES_DIR: &str = "guides";

/// Provider of one external catalog.
pub struct CatalogProvider {
    spec: CatalogSpec,
    id: SourceId,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    head: Option<(Fingerprint, Head)>,
    catalog: Option<(Fingerprint, Arc<Catalog>)>,
}

/// Members of the index needed before the catalog is read.
#[derive(Debug, Clone, Default, Deserialize)]
struct Head {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    languages: Vec<String>,
}

impl CatalogProvider {
    /// Provider of the catalog `spec`, with the identifier `catalog:<spec.id>`.
    #[must_use]
    pub fn new(spec: CatalogSpec) -> Self {
        let id = SourceId::catalog(&spec.id);
        Self {
            spec,
            id,
            state: Mutex::new(State::default()),
        }
    }

    /// The configured catalog.
    #[must_use]
    pub fn spec(&self) -> &CatalogSpec {
        &self.spec
    }

    /// Reads the catalog without conversion. Commands that name the catalog
    /// explicitly report the error with the code `E_CATALOG` of the language.
    ///
    /// # Errors
    ///
    /// Fails when the catalog cannot be read as a whole.
    pub fn read(&self, env: &HostEnv) -> Result<LoadedCatalog, CatalogError> {
        catalog::read_catalog(&env.sys_path(&self.spec.path), &ReadOptions::default())
    }

    fn root(&self, env: &HostEnv) -> PathBuf {
        root_of(&env.sys_path(&self.spec.path))
    }

    fn host_root(&self) -> String {
        let path = self.spec.path.to_string_lossy();
        let trimmed = path.trim_end_matches(['/', '\\']);
        trimmed
            .strip_suffix(INDEX_FILE)
            .map_or(trimmed, |rest| rest.trim_end_matches(['/', '\\']))
            .to_owned()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Members of the index needed for the fingerprint, read again only when
    /// `catalog.json` changed.
    fn head(&self, env: &HostEnv) -> Head {
        let index = self.root(env).join(INDEX_FILE);
        let marker = FingerprintBuilder::new(env).path(&index).finish();
        if let Some((known, head)) = &self.state().head {
            if *known == marker {
                return head.clone();
            }
        }
        let head = read_head(&index).unwrap_or_default();
        self.state().head = Some((marker, head.clone()));
        head
    }

    fn catalog(&self, env: &HostEnv) -> Result<Arc<Catalog>, SourceError> {
        let fingerprint = self.fingerprint(env)?;
        if let Some((known, catalog)) = &self.state().catalog {
            if *known == fingerprint {
                return Ok(Arc::clone(catalog));
            }
        }
        let loaded = self
            .read(env)
            .map_err(|error| SourceError::new(&self.id, SourceErrorKind::Corrupt, error.message).at(error.file))?;
        let catalog = Arc::new(loaded.catalog);
        self.state().catalog = Some((fingerprint, Arc::clone(&catalog)));
        Ok(catalog)
    }

    fn warning(&self, diagnostic: &Diagnostic) -> Option<SourceWarning> {
        let code = match diagnostic.code {
            DiagnosticCode::CatalogInvalid => WarningCode::CatalogInvalid,
            DiagnosticCode::CatalogFormat => WarningCode::CatalogFormat,
            DiagnosticCode::EntryInvalid => WarningCode::CatalogEntryInvalid,
            DiagnosticCode::UnknownField => WarningCode::CatalogUnknownField,
            DiagnosticCode::GuideVersionMissing => WarningCode::GuideVersionMissing,
            DiagnosticCode::GuideMismatch | DiagnosticCode::ControlCharacter => return None,
        };
        let reason = match &diagnostic.field {
            Some(field) => format!("{field}: {}", diagnostic.message),
            None => diagnostic.message.clone(),
        };
        let mut warning = SourceWarning::new(code, &self.id, reason)
            .at(format!("{}/{}", self.host_root(), diagnostic.file));
        if let Some(name) = &diagnostic.name {
            warning = warning.named(name.clone());
        }
        if let Some(line) = diagnostic.line {
            warning = warning.line(line);
        }
        Some(warning)
    }

    fn failure(&self, error: &CatalogError) -> SourceWarning {
        let code = if error.kind == CatalogErrorKind::Format {
            WarningCode::CatalogFormat
        } else {
            WarningCode::CatalogInvalid
        };
        let mut warning = SourceWarning::new(code, &self.id, error.message.clone())
            .at(format!("{}/{}", self.host_root(), error.file));
        if let Some(line) = error.line {
            warning = warning.line(line);
        }
        warning
    }

    fn warnings(&self, diagnostics: &[Diagnostic]) -> Vec<SourceWarning> {
        let all: Vec<SourceWarning> = diagnostics.iter().filter_map(|item| self.warning(item)).collect();
        let extra = all.len().saturating_sub(MAX_WARNINGS);
        let mut warnings: Vec<SourceWarning> = all.into_iter().take(MAX_WARNINGS).collect();
        if extra > 0 {
            warnings.push(SourceWarning::new(
                WarningCode::WarningsTruncated,
                &self.id,
                format!("{extra} further warnings of the catalog were left out"),
            ));
        }
        warnings
    }
}

impl std::fmt::Debug for CatalogProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogProvider")
            .field("id", &self.id)
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

impl ProgramSource for CatalogProvider {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Catalog
    }

    fn priority(&self) -> i32 {
        self.spec.priority
    }

    fn detect(&self, env: &HostEnv) -> bool {
        self.root(env).join(INDEX_FILE).is_file()
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let root = self.root(env);
        let head = self.head(env);
        let mut builder = FingerprintBuilder::new(env)
            .text("catalog", &self.spec.id)
            .path(root.join(INDEX_FILE))
            .path(&root)
            .path(root.join(GUIDES_DIR));
        for part in head.include.iter().filter(|part| catalog::check_relpath(part).is_ok()) {
            builder = builder.path(root.join(part));
        }
        for language in head.languages.iter().filter(|tag| catalog::is_valid_language(tag)) {
            builder = builder.path(root.join(I18N_DIR).join(language).join(INDEX_FILE));
        }
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        deadline.check(&self.id)?;
        let fingerprint = self.fingerprint(env)?;
        let loaded = match self.read(env) {
            Ok(loaded) => loaded,
            Err(error) => {
                return Ok(SourceData {
                    warnings: vec![self.failure(&error)],
                    ..SourceData::default()
                });
            }
        };
        deadline.check(&self.id)?;
        let mut data = convert(&loaded.catalog, &self.id, &self.host_root());
        data.warnings = self.warnings(&loaded.diagnostics);
        self.state().catalog = Some((fingerprint, Arc::new(loaded.catalog)));
        Ok(data)
    }

    fn detail(&self, env: &HostEnv, key: &EntryKey, want: &DetailWant) -> Result<Option<Detail>, SourceError> {
        if key.source != self.id {
            return Ok(None);
        }
        let catalog = self.catalog(env)?;
        let languages = env.languages();
        let Some(target) = Target::find(&catalog, &key.key) else {
            return Ok(None);
        };
        let mut detail = Detail::default();
        if want.description {
            detail.description = target.description(&catalog, languages);
        }
        if want.guide {
            if let Some((path, language)) = target.guide(&catalog, languages) {
                let file = catalog
                    .read_guide(&path, &ReadOptions::default().limits)
                    .map_err(|message| {
                        SourceError::new(&self.id, SourceErrorKind::Corrupt, message)
                            .at(format!("{}/{path}", self.host_root()))
                    })?;
                let fallback_version = target.version();
                detail.guide = Some(GuideBody {
                    sections: catalog::guide::sections(&file.body)
                        .iter()
                        .map(|section| section.title.to_owned())
                        .collect(),
                    files: catalog.guide_material(&path, &ReadOptions::default().limits),
                    version: file.front.version.or(fallback_version),
                    language: file.front.language.or(Some(language)),
                    text: file.body,
                });
            }
        }
        Ok(Some(detail))
    }
}

/// Entry of a catalog addressed by a detail key.
enum Target<'a> {
    Program(&'a catalog::Program),
    Guide(&'a catalog::StandaloneGuide),
}

impl<'a> Target<'a> {
    fn find(catalog: &'a Catalog, name: &str) -> Option<Self> {
        catalog
            .program(name)
            .map(Self::Program)
            .or_else(|| catalog.standalone_guide(name).map(Self::Guide))
    }

    fn version(&self) -> Option<String> {
        match self {
            Self::Program(program) => program.version.clone(),
            Self::Guide(_) => None,
        }
    }

    fn description(&self, catalog: &Catalog, languages: &[String]) -> Option<String> {
        let Self::Program(program) = self else {
            return None;
        };
        for language in languages {
            if *language == catalog.language {
                break;
            }
            let translated = catalog
                .overlay(language)
                .and_then(|overlay| overlay.programs.get(&program.name))
                .and_then(|texts| texts.description.clone());
            if translated.is_some() {
                return translated;
            }
        }
        program.description.clone()
    }

    /// Path and language of the guide in the first preferred language that
    /// has one.
    fn guide(&self, catalog: &Catalog, languages: &[String]) -> Option<(String, String)> {
        let (name, main) = match self {
            Self::Program(program) => (&program.name, program.guide.clone()?),
            Self::Guide(guide) => (&guide.name, guide.guide.clone()),
        };
        for language in languages {
            if *language == catalog.language {
                break;
            }
            let Some(overlay) = catalog.overlay(language) else { continue };
            let translated = match self {
                Self::Program(_) => overlay.programs.get(name).and_then(|texts| texts.guide.clone()),
                Self::Guide(_) => overlay.guides.get(name).and_then(|texts| texts.guide.clone()),
            };
            if let Some(path) = translated {
                return Some((path, language.clone()));
            }
        }
        Some((main, catalog.language.clone()))
    }
}

/// Root directory of a catalog named by its directory or its index file.
fn root_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if path.file_name().is_some_and(|name| name == INDEX_FILE) => parent.to_path_buf(),
        _ => path.to_path_buf(),
    }
}

fn read_head(index: &Path) -> Option<Head> {
    let bytes = std::fs::read(index).ok()?;
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf".as_slice()).unwrap_or(&bytes);
    serde_json::from_slice(bytes).ok()
}

/// Identifiers of configured catalogs with the `id` declared in each
/// `catalog.json` in place of the identifier derived from the path; repeated
/// identifiers receive the suffixes `-2`, `-3` and so on, in configuration
/// order.
#[must_use]
pub fn resolve_ids(specs: Vec<CatalogSpec>, env: &HostEnv) -> Vec<CatalogSpec> {
    let mut out: Vec<CatalogSpec> = Vec::with_capacity(specs.len());
    for mut spec in specs {
        let index = root_of(&env.sys_path(&spec.path)).join(INDEX_FILE);
        let base = read_head(&index)
            .and_then(|head| head.id)
            .filter(|id| catalog::is_valid_catalog_id(id))
            .unwrap_or_else(|| spec.id.clone());
        let mut id = base.clone();
        let mut suffix = 2;
        while out.iter().any(|known| known.id == id) {
            id = format!("{base}-{suffix}");
            suffix += 1;
        }
        spec.id = id;
        out.push(spec);
    }
    out
}

/// Converts a catalog into neutral entries of the provider `source`;
/// `host_root` is the catalog root as a host path.
#[must_use]
pub fn convert(catalog: &Catalog, source: &SourceId, host_root: &str) -> SourceData {
    let texts = Texts { catalog };
    SourceData {
        programs: catalog
            .programs
            .iter()
            .map(|program| texts.program(program, source, host_root))
            .collect(),
        categories: catalog.categories.iter().map(|category| texts.category(category)).collect(),
        guides: catalog.guides.iter().map(|guide| texts.guide(guide, source)).collect(),
        synonyms: catalog.synonyms.clone(),
        ..SourceData::default()
    }
}

/// Builder of localized texts from a catalog and its overlays.
struct Texts<'a> {
    catalog: &'a Catalog,
}

impl Texts<'_> {
    fn text(&self, main: &str, translations: impl Fn(&catalog::Overlay) -> Option<String>) -> LocalizedText {
        let mut text = LocalizedText::in_lang(main, &self.catalog.language);
        for overlay in &self.catalog.overlays {
            if let Some(translated) = translations(overlay) {
                text.set(&overlay.language, translated);
            }
        }
        text
    }

    fn optional(&self, main: Option<&str>, translations: impl Fn(&catalog::Overlay) -> Option<String>) -> Option<LocalizedText> {
        main.map(|main| self.text(main, translations))
    }

    fn program(&self, program: &catalog::Program, source: &SourceId, host_root: &str) -> RawProgram {
        let name = &program.name;
        let overlay_of = |overlay: &catalog::Overlay| overlay.programs.get(name).cloned();
        let mut tags = program.tags.clone();
        for overlay in &self.catalog.overlays {
            for tag in overlay_of(overlay).and_then(|texts| texts.tags).unwrap_or_default() {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
        RawProgram {
            key: Some(EntryKey::new(source, name.clone())),
            aliases: program
                .aliases
                .iter()
                .map(|alias| RawAlias {
                    name: alias.name.clone(),
                    kind: alias.kind,
                    path: alias.path.clone(),
                })
                .collect(),
            summary: Some(self.text(&program.summary, |overlay| overlay_of(overlay)?.summary)),
            overview: self.optional(program.overview.as_deref(), |overlay| overlay_of(overlay)?.overview),
            description: self.optional(program.description.as_deref(), |overlay| overlay_of(overlay)?.description),
            sections: self.sections(&program.sections, |overlay| overlay_of(overlay)?.sections),
            when_to_use: self.list(&program.when_to_use, |overlay| overlay_of(overlay)?.when_to_use),
            not_for: self.not_for(&program.not_for, |overlay| overlay_of(overlay)?.not_for),
            example: program.example.clone(),
            category_hints: vec![CategoryHint::new(CategoryScheme::Catalog, program.category.clone())],
            version: program.version.clone(),
            homepage: program.homepage.clone(),
            license: program.license.clone(),
            tags,
            platforms: program.platforms.clone(),
            verified: program.verified.clone(),
            catalog_path: program.path.clone(),
            guide: program.guide.as_deref().and_then(|path| self.guide_ref(path, host_root, |overlay| overlay_of(overlay)?.guide)),
            guide_optional: program.guide_optional,
            guide_required: program.guide_required,
            related: program.related.clone(),
            input: program.input.clone(),
            output: program.output.clone(),
            ..RawProgram::new(name.clone())
        }
    }

    /// Items of a list in the main language, with the items of an overlay as
    /// translations when the overlay has as many items; otherwise the
    /// overlay items are appended in their own language.
    fn list(&self, main: &[String], translations: impl Fn(&catalog::Overlay) -> Option<Vec<String>>) -> Vec<LocalizedText> {
        let mut items: Vec<LocalizedText> = main
            .iter()
            .map(|item| LocalizedText::in_lang(item.as_str(), &self.catalog.language))
            .collect();
        for overlay in &self.catalog.overlays {
            let Some(translated) = translations(overlay) else { continue };
            if translated.len() == main.len() {
                for (item, text) in items.iter_mut().zip(translated) {
                    item.set(&overlay.language, text);
                }
            } else {
                items.extend(translated.into_iter().map(|text| LocalizedText::in_lang(text, &overlay.language)));
            }
        }
        items
    }

    fn not_for(&self, main: &[NotForTask], translations: impl Fn(&catalog::Overlay) -> Option<Vec<NotForTask>>) -> Vec<NotFor> {
        let tasks: Vec<String> = main.iter().map(|task| task.task.clone()).collect();
        let texts = self.list(&tasks, |overlay| {
            translations(overlay).map(|items| items.into_iter().map(|task| task.task).collect())
        });
        let mut instead: Vec<Option<String>> = main.iter().map(|task| task.instead.clone()).collect();
        for overlay in &self.catalog.overlays {
            if let Some(items) = translations(overlay).filter(|items| items.len() != main.len()) {
                instead.extend(items.into_iter().map(|task| task.instead));
            }
        }
        texts
            .into_iter()
            .zip(instead)
            .map(|(task, instead)| NotFor { task, instead })
            .collect()
    }

    fn sections(&self, main: &[Section], translations: impl Fn(&catalog::Overlay) -> Option<Vec<Section>>) -> Vec<DescriptionSection> {
        let mut sections: Vec<DescriptionSection> = main
            .iter()
            .map(|section| DescriptionSection {
                kind: section.kind,
                label: section.label.clone(),
                text: LocalizedText::in_lang(section.text.as_str(), &self.catalog.language),
            })
            .collect();
        for overlay in &self.catalog.overlays {
            let Some(translated) = translations(overlay) else { continue };
            let aligned = translated.len() == main.len()
                && translated.iter().zip(main).all(|(left, right)| left.kind == right.kind);
            if aligned {
                for (section, text) in sections.iter_mut().zip(translated) {
                    section.text.set(&overlay.language, text.text);
                }
            } else {
                sections.extend(translated.into_iter().map(|section| DescriptionSection {
                    kind: section.kind,
                    label: section.label,
                    text: LocalizedText::in_lang(section.text, &overlay.language),
                }));
            }
        }
        sections
    }

    fn guide_ref(
        &self,
        path: &str,
        host_root: &str,
        translated_path: impl Fn(&catalog::Overlay) -> Option<String>,
    ) -> Option<GuideRef> {
        let header = self.catalog.guide_headers.get(path)?;
        let language = header.front.language.clone().unwrap_or_else(|| self.catalog.language.clone());
        let description = header.front.description.as_deref().map(|text| {
            let mut description = LocalizedText::in_lang(text, &language);
            for overlay in &self.catalog.overlays {
                let translated = translated_path(overlay)
                    .and_then(|path| self.catalog.guide_headers.get(&path))
                    .and_then(|header| header.front.description.clone());
                if let Some(translated) = translated {
                    description.set(&overlay.language, translated);
                }
            }
            description
        });
        let _ = host_root;
        Some(GuideRef {
            name: header.name.clone(),
            path: path.to_owned(),
            description,
        })
    }

    fn category(&self, category: &catalog::Category) -> RawCategory {
        let id = &category.id;
        let texts = |overlay: &catalog::Overlay| overlay.categories.get(id).cloned();
        RawCategory {
            id: id.clone(),
            title: self.text(&category.title, |overlay| texts(overlay)?.title),
            summary: self.optional(category.summary.as_deref(), |overlay| texts(overlay)?.summary),
            description: self.optional(category.description.as_deref(), |overlay| texts(overlay)?.description),
            parent: category.parent.clone(),
            aliases: category.aliases.clone(),
            same_as: category.same_as.clone(),
            order: category.order,
        }
    }

    fn guide(&self, guide: &catalog::StandaloneGuide, source: &SourceId) -> RawGuide {
        let texts = |overlay: &catalog::Overlay| overlay.guides.get(&guide.name).cloned();
        let summary = guide.summary.clone().or_else(|| {
            self.catalog
                .guide_headers
                .get(&guide.guide)
                .and_then(|header| header.front.description.clone())
        });
        let mut tags = guide.tags.clone();
        for overlay in &self.catalog.overlays {
            for tag in texts(overlay).and_then(|texts| texts.tags).unwrap_or_default() {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
        RawGuide {
            name: guide.name.clone(),
            path: guide.guide.clone(),
            summary: self.optional(summary.as_deref(), |overlay| texts(overlay)?.summary),
            category: guide.category.clone(),
            tags,
            key: Some(EntryKey::new(source, guide.name.clone())),
        }
    }
}
