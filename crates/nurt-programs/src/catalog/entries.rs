// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Readers of the objects of a catalog: categories, programs, aliases,
//! sections, tasks outside the purpose and standalone guides.

use serde_json::Value;

use crate::catalog::fields::{Fields, Len, Pattern, len};
use crate::catalog::paths::check_relpath;
use crate::catalog::types::{
    Alias, Category, NotForTask, Origin, Program, Section, StandaloneGuide, is_valid_category_id,
    is_valid_date, is_valid_name,
};
use crate::model::{AliasKind, SectionKind};

/// Program, alias and guide names.
pub(crate) const NAME: Pattern = Pattern {
    check: is_valid_name,
    rule: "use a letter or digit followed by letters, digits, `.`, `_`, `+` or `-`, at most 120 characters",
};

/// Category identifiers.
pub(crate) const CATEGORY_ID: Pattern = Pattern {
    check: is_valid_category_id,
    rule: "use lower-case letters, digits, `_` and `-`, starting with a letter or digit, at most 60 characters",
};

const DATE: Pattern = Pattern {
    check: is_valid_date,
    rule: "use the form YYYY-MM-DD",
};

/// Limits of members shared by the index and the overlays.
pub(crate) const TITLE: Len = len(1, 120);
pub(crate) const CATEGORY_DESCRIPTION: Len = len(0, 2000);
pub(crate) const SUMMARY: Len = len(0, 300);
pub(crate) const PROGRAM_SUMMARY: Len = len(1, 300);
pub(crate) const OVERVIEW: Len = len(0, 600);
pub(crate) const DESCRIPTION: Len = len(0, 8000);
pub(crate) const WHEN_TO_USE: Len = len(1, 300);
pub(crate) const MAX_WHEN_TO_USE: usize = 32;
pub(crate) const TAG: Len = len(1, 60);
pub(crate) const MAX_TAGS: usize = 64;
pub(crate) const MAX_NOT_FOR: usize = 16;
pub(crate) const MAX_SECTIONS: usize = 32;

const CATEGORY_ALIAS: Len = len(1, 120);
const MAX_CATEGORY_ALIASES: usize = 16;
const SAME_AS: Len = len(0, 60);
const MAX_ALIASES: usize = 64;
const HOST_PATH: Len = len(0, 1024);
const EXAMPLE: Len = len(0, 500);
const VERSION: Len = len(0, 60);
const HOMEPAGE: Len = len(0, 300);
const LICENSE: Len = len(0, 200);
const PACKAGE: Len = len(0, 120);
const IO_TEXT: Len = len(0, 400);
const MAX_RELATED: usize = 32;
const TASK: Len = len(1, 300);
const INSTEAD: Len = len(0, 200);
const LABEL: Len = len(0, 60);
const SECTION_TEXT: Len = len(1, 4000);
const MAX_PLATFORMS: usize = 16;
const PLATFORMS: [&str; 5] = ["linux", "macos", "windows", "freebsd", "any"];
const ALIAS_KINDS: &str = "`alias`, `versioned` or `companion`";
const SECTION_KINDS: &str = "`purpose`, `when`, `when_not`, `io`, `example`, `check`, `pitfalls`, `related`, `aliases` or `other`";

/// Reads a category.
pub(crate) fn read_category(fields: &mut Fields<'_>) -> Category {
    let category = Category {
        id: fields
            .required_pattern("id", CATEGORY_ID)
            .unwrap_or_default(),
        title: fields.required_text("title", TITLE).unwrap_or_default(),
        description: fields.text("description", CATEGORY_DESCRIPTION),
        parent: fields.pattern("parent", CATEGORY_ID),
        summary: fields.text("summary", SUMMARY),
        aliases: fields.text_list("aliases", MAX_CATEGORY_ALIASES, CATEGORY_ALIAS),
        same_as: fields.text("same_as", SAME_AS),
        order: fields.integer("order", i64::MIN),
        origin: Origin::default(),
    };
    fields.extension();
    category
}

/// Reads a program entry.
pub(crate) fn read_program(fields: &mut Fields<'_>) -> Program {
    let mut program = Program {
        name: fields.required_pattern("name", NAME).unwrap_or_default(),
        category: fields
            .required_pattern("category", CATEGORY_ID)
            .unwrap_or_default(),
        summary: fields
            .required_text("summary", PROGRAM_SUMMARY)
            .unwrap_or_default(),
        aliases: read_aliases(fields),
        platforms: read_platforms(fields),
        guide: relpath(fields, "guide"),
        verified: fields.pattern("verified", DATE),
        tags: fields.text_list("tags", MAX_TAGS, TAG),
        overview: fields.text("overview", OVERVIEW),
        description: fields.text("description", DESCRIPTION),
        sections: read_sections(fields),
        when_to_use: fields.text_list("when_to_use", MAX_WHEN_TO_USE, WHEN_TO_USE),
        not_for: read_not_for(fields),
        example: fields.text("example", EXAMPLE),
        guide_optional: fields.boolean("guide_optional").unwrap_or(false),
        guide_required: fields.boolean("guide_required").unwrap_or(false),
        version: fields.text("version", VERSION),
        path: fields.text("path", HOST_PATH),
        homepage: fields.text("homepage", HOMEPAGE),
        license: fields.text("license", LICENSE),
        package: fields.text("package", PACKAGE),
        input: fields.text("input", IO_TEXT),
        output: fields.text("output", IO_TEXT),
        related: fields.pattern_list("related", MAX_RELATED, NAME, false),
        origin: Origin::default(),
    };
    fields.extension();
    if program.guide_optional && program.guide_required {
        let path = fields.path("guide_required");
        fields.fault(
            path,
            "`guide_optional` and `guide_required` cannot both be true; keep one",
        );
        program.guide_required = false;
    }
    program
}

/// Reads a standalone guide.
pub(crate) fn read_standalone_guide(fields: &mut Fields<'_>) -> StandaloneGuide {
    let name = fields.required_pattern("name", NAME).unwrap_or_default();
    let guide = if fields.get("guide").is_some() {
        relpath(fields, "guide").unwrap_or_default()
    } else {
        let path = fields.path("guide");
        fields.fault(path, "member `guide` is required; add it");
        String::new()
    };
    StandaloneGuide {
        name,
        guide,
        summary: fields.text("summary", SUMMARY),
        category: fields.pattern("category", CATEGORY_ID),
        tags: fields.text_list("tags", MAX_TAGS, TAG),
    }
}

/// Reads an optional relative path member.
pub(crate) fn relpath(fields: &mut Fields<'_>, key: &'static str) -> Option<String> {
    let value = fields.get(key)?;
    let path = fields.path(key);
    let Value::String(text) = value else {
        fields.fault(path, "must be text");
        return None;
    };
    match check_relpath(text) {
        Ok(()) => Some(text.clone()),
        Err(error) => {
            fields.fault(path, error.to_string());
            None
        }
    }
}

fn read_aliases(fields: &mut Fields<'_>) -> Vec<Alias> {
    let Some((items, path)) = fields.array("aliases", MAX_ALIASES) else {
        return Vec::new();
    };
    let mut aliases = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        if let Value::String(_) = item {
            if let Some(name) = fields.pattern_value(item, item_path, NAME) {
                aliases.push(Alias {
                    name,
                    kind: AliasKind::Alias,
                    path: None,
                });
            }
            continue;
        }
        match Fields::of(item, item_path) {
            Ok(mut nested) => {
                let alias = read_alias_object(&mut nested);
                fields.absorb(nested.finish());
                aliases.extend(alias);
            }
            Err(fault) => fields.fault(fault.field, "must be a name or an object with `name`"),
        }
    }
    aliases
}

fn read_alias_object(fields: &mut Fields<'_>) -> Option<Alias> {
    let name = fields.required_pattern("name", NAME);
    let kind = match fields.get("kind") {
        None => AliasKind::Alias,
        Some(Value::String(kind)) => match kind.as_str() {
            "alias" => AliasKind::Alias,
            "versioned" => AliasKind::Versioned,
            "companion" => AliasKind::Companion,
            other => {
                let path = fields.path("kind");
                fields.unknown_value(path, other, ALIAS_KINDS);
                AliasKind::Alias
            }
        },
        Some(_) => {
            let path = fields.path("kind");
            fields.fault(path, format!("must be one of {ALIAS_KINDS}"));
            AliasKind::Alias
        }
    };
    let path = fields.text("path", HOST_PATH);
    name.map(|name| Alias { name, kind, path })
}

fn read_platforms(fields: &mut Fields<'_>) -> Vec<String> {
    let Some((items, path)) = fields.array("platforms", MAX_PLATFORMS) else {
        return Vec::new();
    };
    let mut platforms: Vec<String> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Value::String(platform) = item else {
            fields.fault(item_path, "must be text");
            continue;
        };
        if platforms.contains(platform) {
            fields.fault(item_path, format!("`{platform}` is repeated; list it once"));
        } else if PLATFORMS.contains(&platform.as_str()) {
            platforms.push(platform.clone());
        } else {
            fields.unknown_value(
                item_path,
                platform,
                "`linux`, `macos`, `windows`, `freebsd` or `any`",
            );
        }
    }
    platforms
}

/// Reads the member `not_for`.
pub(crate) fn read_not_for(fields: &mut Fields<'_>) -> Vec<NotForTask> {
    let Some((items, path)) = fields.array("not_for", MAX_NOT_FOR) else {
        return Vec::new();
    };
    let mut tasks = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match Fields::of(item, format!("{path}[{index}]")) {
            Ok(mut nested) => {
                let task = nested.required_text("task", TASK);
                let instead = nested.text("use", INSTEAD);
                fields.absorb(nested.finish());
                tasks.extend(task.map(|task| NotForTask { task, instead }));
            }
            Err(fault) => fields.fault(fault.field, fault.message),
        }
    }
    tasks
}

/// Reads the member `sections`.
pub(crate) fn read_sections(fields: &mut Fields<'_>) -> Vec<Section> {
    let Some((items, path)) = fields.array("sections", MAX_SECTIONS) else {
        return Vec::new();
    };
    let mut sections = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match Fields::of(item, format!("{path}[{index}]")) {
            Ok(mut nested) => {
                let section = read_section(&mut nested);
                fields.absorb(nested.finish());
                sections.extend(section);
            }
            Err(fault) => fields.fault(fault.field, fault.message),
        }
    }
    sections
}

fn read_section(fields: &mut Fields<'_>) -> Option<Section> {
    let kind = match fields.get("kind") {
        Some(Value::String(kind)) => section_kind(kind).unwrap_or_else(|| {
            let path = fields.path("kind");
            fields.unknown_value(path, kind, SECTION_KINDS);
            SectionKind::Other
        }),
        Some(_) => {
            let path = fields.path("kind");
            fields.fault(path, format!("must be one of {SECTION_KINDS}"));
            SectionKind::Other
        }
        None => {
            let path = fields.path("kind");
            fields.fault(path, "member `kind` is required; add it");
            SectionKind::Other
        }
    };
    let label = fields.text("label", LABEL);
    let text = fields.required_text("text", SECTION_TEXT)?;
    Some(Section { kind, label, text })
}

fn section_kind(kind: &str) -> Option<SectionKind> {
    Some(match kind {
        "purpose" => SectionKind::Purpose,
        "when" => SectionKind::When,
        "when_not" => SectionKind::WhenNot,
        "io" => SectionKind::Io,
        "example" => SectionKind::Example,
        "check" => SectionKind::Check,
        "pitfalls" => SectionKind::Pitfalls,
        "related" => SectionKind::Related,
        "aliases" => SectionKind::Aliases,
        "other" => SectionKind::Other,
        _ => return None,
    })
}
