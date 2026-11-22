// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Localized overlays `i18n/<lang>/catalog.json`.

use serde_json::Value;
use serde_json::value::RawValue;

use crate::catalog::diag::{At, DiagnosticCode};
use crate::catalog::entries::{
    self, CATEGORY_DESCRIPTION, CATEGORY_ID, DESCRIPTION, MAX_TAGS, MAX_WHEN_TO_USE, NAME,
    OVERVIEW, PROGRAM_SUMMARY, SUMMARY, TAG, TITLE, WHEN_TO_USE,
};
use crate::catalog::fields::Fields;
use crate::catalog::index::{Reader, guide_name};
use crate::catalog::json::{self, RawObject};
use crate::catalog::types::{
    CategoryText, FORMAT, GuideText, I18N_DIR, INDEX_FILE, Origin, Overlay, ProgramText,
};

/// Members permitted at the top of an overlay.
const OVERLAY_MEMBERS: [&str; 5] = ["format", "language", "categories", "programs", "guides"];

impl Reader<'_> {
    /// Reads the overlay of every language in `languages` other than the
    /// language of the index.
    pub(crate) fn read_overlays(&mut self) {
        let languages: Vec<String> = self
            .catalog
            .languages
            .iter()
            .filter(|language| **language != self.catalog.language)
            .cloned()
            .collect();
        for language in languages {
            if let Some(overlay) = self.read_overlay(&language) {
                self.catalog.overlays.push(overlay);
            }
        }
    }

    fn read_overlay(&mut self, language: &str) -> Option<Overlay> {
        let file = format!("{I18N_DIR}/{language}/{INDEX_FILE}");
        let path = match self.root.resolve(&file) {
            Ok(path) => path,
            Err(error) => {
                let message = format!(
                    "overlay of language `{language}`: {error}; add the file or remove the language from `languages`"
                );
                self.invalid(
                    At::new(INDEX_FILE, None)
                        .field("languages")
                        .name(Some(language)),
                    message,
                );
                return None;
            }
        };
        let text = match json::read_text(&path, self.options.limits.part_bytes) {
            Ok(text) => text,
            Err(error) => {
                self.invalid(At::new(&file, error.line()), error.message());
                return None;
            }
        };
        let members = match json::parse_object(&text, self.options.limits.depth) {
            Ok(members) => members,
            Err(error) => {
                self.invalid(At::new(&file, error.line()), error.message());
                return None;
            }
        };
        if !self.overlay_header(&file, &text, &members, language) {
            return None;
        }
        let mut overlay = Overlay {
            language: language.to_owned(),
            ..Overlay::default()
        };
        for (key, raw) in &members {
            match key.as_str() {
                "categories" => {
                    self.overlay_items(&file, &text, raw, key, |reader, fields, origin| {
                        reader.overlay_category(fields, origin, &mut overlay);
                    })
                }
                "programs" => {
                    self.overlay_items(&file, &text, raw, key, |reader, fields, origin| {
                        reader.overlay_program(fields, origin, &mut overlay);
                    })
                }
                "guides" => self.overlay_items(&file, &text, raw, key, |reader, fields, origin| {
                    reader.overlay_guide(fields, origin, &mut overlay);
                }),
                _ => {}
            }
        }
        Some(overlay)
    }

    /// Checks `format`, `language` and the top-level members of an overlay.
    fn overlay_header(
        &mut self,
        file: &str,
        text: &str,
        members: &RawObject<'_>,
        language: &str,
    ) -> bool {
        let text_of = |key: &str| {
            members
                .get(key)
                .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        };
        if text_of("format").as_deref() != Some(FORMAT) {
            self.invalid(
                At::new(file, Some(1)).field("format"),
                format!("`format` must be `{FORMAT}`"),
            );
            return false;
        }
        if text_of("language").as_deref() != Some(language) {
            self.invalid(
                At::new(file, Some(1)).field("language"),
                format!("`language` must be `{language}`, the name of the overlay directory"),
            );
            return false;
        }
        for (key, raw) in members {
            if !OVERLAY_MEMBERS.contains(&key.as_str()) {
                let line = Some(json::line_at(text, json::offset_of(text, raw.get())));
                self.diags.push(
                    DiagnosticCode::UnknownField,
                    At::new(file, line).field(key.clone()),
                    "member is not permitted in an overlay; overlays hold texts of categories, programs and guides",
                );
            }
        }
        true
    }

    /// Reads each object of a list member of an overlay with `read`.
    fn overlay_items(
        &mut self,
        file: &str,
        text: &str,
        raw: &RawValue,
        key: &str,
        mut read: impl FnMut(&mut Self, Fields<'_>, &Origin),
    ) {
        let items = match json::parse_array(text, raw) {
            Ok(items) => items,
            Err(error) => {
                self.invalid(
                    At::new(file, error.line()).field(key.to_owned()),
                    "must be an array",
                );
                return;
            }
        };
        for (index, item) in items.iter().enumerate() {
            let number = json::line_at(text, json::offset_of(text, item.get()));
            let line = Some(number);
            let Ok(value) = serde_json::from_str::<Value>(item.get()) else {
                continue;
            };
            let fields = match Fields::of(&value, format!("{key}[{index}]")) {
                Ok(fields) => fields,
                Err(fault) => {
                    self.invalid(At::new(file, line).field(fault.field), fault.message);
                    continue;
                }
            };
            let origin = Origin {
                file: file.to_owned(),
                line: number,
                pointer: format!("{key}[{index}]"),
            };
            read(self, fields, &origin);
        }
    }

    fn overlay_category(&mut self, mut fields: Fields<'_>, origin: &Origin, overlay: &mut Overlay) {
        let id = fields.required_pattern("id", CATEGORY_ID);
        let texts = CategoryText {
            title: fields.text("title", TITLE),
            summary: fields.text("summary", SUMMARY),
            description: fields.text("description", CATEGORY_DESCRIPTION),
        };
        if self.accept(fields, origin, id.as_deref())
            && let Some(id) = id.filter(|id| self.catalog.category(id).is_some())
        {
            overlay.categories.insert(id, texts);
        }
    }

    fn overlay_program(&mut self, mut fields: Fields<'_>, origin: &Origin, overlay: &mut Overlay) {
        let name = fields.required_pattern("name", NAME);
        let mut texts = ProgramText {
            summary: fields.text("summary", PROGRAM_SUMMARY),
            overview: fields.text("overview", OVERVIEW),
            description: fields.text("description", DESCRIPTION),
            when_to_use: fields
                .has("when_to_use")
                .then(|| fields.text_list("when_to_use", MAX_WHEN_TO_USE, WHEN_TO_USE)),
            not_for: fields
                .has("not_for")
                .then(|| entries::read_not_for(&mut fields)),
            sections: fields
                .has("sections")
                .then(|| entries::read_sections(&mut fields)),
            tags: fields
                .has("tags")
                .then(|| fields.text_list("tags", MAX_TAGS, TAG)),
            guide: entries::relpath(&mut fields, "guide"),
        };
        if !self.accept(fields, origin, name.as_deref()) {
            return;
        }
        let Some(program) = name.and_then(|name| self.catalog.program(&name).cloned()) else {
            return;
        };
        let original = program.guide.as_deref().map(guide_name);
        texts.guide = texts
            .guide
            .take()
            .and_then(|path| self.translated_guide(&path, original.as_deref(), origin));
        overlay.programs.insert(program.name, texts);
    }

    fn overlay_guide(&mut self, mut fields: Fields<'_>, origin: &Origin, overlay: &mut Overlay) {
        let name = fields.required_pattern("name", NAME);
        let mut texts = GuideText {
            summary: fields.text("summary", SUMMARY),
            tags: fields
                .has("tags")
                .then(|| fields.text_list("tags", MAX_TAGS, TAG)),
            guide: entries::relpath(&mut fields, "guide"),
        };
        if !self.accept(fields, origin, name.as_deref()) {
            return;
        }
        let Some(guide) = name.and_then(|name| self.catalog.standalone_guide(&name).cloned())
        else {
            return;
        };
        let original = guide_name(&guide.guide);
        texts.guide = texts
            .guide
            .take()
            .and_then(|path| self.translated_guide(&path, Some(&original), origin));
        overlay.guides.insert(guide.name, texts);
    }

    /// Reports the findings of an overlay object; returns `true` when it is
    /// valid.
    fn accept(&mut self, fields: Fields<'_>, origin: &Origin, name: Option<&str>) -> bool {
        self.report(&origin.file, Some(origin.line), name, fields.finish())
    }

    /// Reads the front matter of a translated guide; its name must equal the
    /// name of the original guide.
    fn translated_guide(
        &mut self,
        path: &str,
        original: Option<&str>,
        origin: &Origin,
    ) -> Option<String> {
        let header = self.read_guide_header(path, std::slice::from_ref(origin))?;
        let declared = header
            .front
            .name
            .clone()
            .unwrap_or_else(|| header.name.clone());
        if original.is_some_and(|original| original != declared) {
            let at =
                At::new(&origin.file, Some(origin.line)).field(format!("{}.guide", origin.pointer));
            let message = format!(
                "translated guide has the name `{declared}`; it must carry the name of the original guide, `{}`",
                original.unwrap_or_default()
            );
            self.invalid(at, message);
            return None;
        }
        self.catalog.guide_headers.insert(path.to_owned(), header);
        Some(path.to_owned())
    }
}
