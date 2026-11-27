// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u11_registry.rs
// Responsibility:   Tests of the language registry, language detection, grammar providers.
// Dependencies:     nurt_contracts, u11_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the language registry, language detection, grammar providers
//! and name matching.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u11_support;

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use nurt_code::registry::{
    Grammar, GrammarProvider, LanguageSpec, Registry, detect_language, interpreter_of_shebang,
    modeline_language,
};
use nurt_code::symbols::matching::{MatchMode, NameMatcher, RANK_EXACT, name_words};
use nurt_code::symbols::{kind_name, parse_kind, symbols_of};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::services::{LanguageId, SymbolKind};
use nurt_contracts::value::Regex;

use u11_support::language;

/// Languages of the built-in registry.
const BUILTIN_LANGUAGES: &[&str] = &[
    "bash",
    "c",
    "cpp",
    "csharp",
    "css",
    "go",
    "html",
    "java",
    "javascript",
    "json",
    "kotlin",
    "markdown",
    "php",
    "powershell",
    "python",
    "ruby",
    "rust",
    "swift",
    "toml",
    "tsx",
    "typescript",
    "xml",
    "yaml",
];

fn detect(path: &str, content: &str) -> Option<String> {
    Registry::builtin()
        .detect(Some(Path::new(path)), content.as_bytes())
        .map(|id| id.to_string())
}

#[test]
fn builtin_registry_lists_every_language_with_its_grammar_version() {
    let registry = Registry::builtin();
    let mut ids: Vec<String> = registry.specs().map(|spec| spec.id.to_string()).collect();
    ids.sort();
    assert_eq!(ids, BUILTIN_LANGUAGES);
    let languages = registry.languages();
    assert_eq!(languages.len(), BUILTIN_LANGUAGES.len());
    for info in &languages {
        let version = info.grammar_version.as_deref().unwrap_or_default();
        assert!(!version.is_empty(), "{} has no grammar version", info.id);
        assert!(!info.extensions.is_empty(), "{} has no extension", info.id);
    }
    let version_of = |id: &str| {
        registry
            .spec(&language(id))
            .map(|spec| spec.grammar_version.clone())
    };
    assert_eq!(version_of("kotlin").as_deref(), Some("1.1.0"));
    assert_eq!(version_of("swift").as_deref(), Some("0.7.4"));
    assert_eq!(version_of("xml").as_deref(), Some("0.7.0"));
}

#[test]
fn every_builtin_grammar_loads_with_its_queries() {
    let registry = Registry::builtin();
    for spec in registry.specs() {
        let grammar = registry.grammar(&spec.id).unwrap();
        assert!(grammar.tags().is_some(), "{} has no tags query", spec.id);
    }
    let with_locals = ["go", "rust", "python", "typescript", "java", "csharp"];
    for id in with_locals {
        let grammar = registry.grammar(&language(id)).unwrap();
        assert!(grammar.locals().is_some(), "{id} has no locals query");
    }
}

#[test]
fn language_is_detected_by_extension() {
    let cases = [
        ("/p/main.go", "go"),
        ("/p/lib.rs", "rust"),
        ("/p/app.py", "python"),
        ("/p/types.pyi", "python"),
        ("/p/index.mjs", "javascript"),
        ("/p/view.jsx", "javascript"),
        ("/p/api.ts", "typescript"),
        ("/p/page.tsx", "tsx"),
        ("/p/list.h", "c"),
        ("/p/vec.hpp", "cpp"),
        ("/p/Main.java", "java"),
        ("/p/task.rake", "ruby"),
        ("/p/Program.cs", "csharp"),
        ("/p/index.php", "php"),
        ("/p/run.sh", "bash"),
        ("/p/setup.ps1", "powershell"),
        ("/p/data.jsonc", "json"),
        ("/p/ci.yml", "yaml"),
        ("/p/Cargo.toml", "toml"),
        ("/p/README.md", "markdown"),
        ("/p/index.htm", "html"),
        ("/p/site.css", "css"),
        ("/p/App.csproj", "xml"),
        ("/p/Main.kt", "kotlin"),
        ("/p/App.swift", "swift"),
    ];
    for (path, expected) in cases {
        assert_eq!(detect(path, "").as_deref(), Some(expected), "{path}");
    }
}

#[test]
fn language_is_detected_by_file_name() {
    assert_eq!(detect("/p/Gemfile", "").as_deref(), Some("ruby"));
    assert_eq!(detect("/p/Rakefile", "").as_deref(), Some("ruby"));
    assert_eq!(detect("/p/Cargo.lock", "").as_deref(), Some("toml"));
    assert_eq!(detect("/p/.bashrc", "").as_deref(), Some("bash"));
    assert_eq!(detect("/p/PKGBUILD", "").as_deref(), Some("bash"));
}

#[test]
fn language_is_detected_by_shebang() {
    assert_eq!(
        detect("/p/tool", "#!/usr/bin/env python3\nprint(1)\n").as_deref(),
        Some("python")
    );
    assert_eq!(
        detect("/p/run", "#!/bin/bash\necho hi\n").as_deref(),
        Some("bash")
    );
    assert_eq!(
        detect("/p/serve", "#!/usr/bin/env node\nconsole.log(1)\n").as_deref(),
        Some("javascript")
    );
    assert_eq!(
        interpreter_of_shebang(b"#!/usr/bin/env -S deno run\n").as_deref(),
        Some("deno")
    );
    assert_eq!(interpreter_of_shebang(b"echo no shebang\n"), None);
}

#[test]
fn modeline_overrides_the_file_name() {
    assert_eq!(
        detect("/p/notes.txt", "# vim: set ft=ruby:\nputs 1\n").as_deref(),
        Some("ruby")
    );
    assert_eq!(
        detect("/p/script", "# -*- mode: python -*-\nprint(1)\n").as_deref(),
        Some("python")
    );
    assert_eq!(
        modeline_language(b"// vim: filetype=go\n").as_deref(),
        Some("go")
    );
    assert_eq!(modeline_language(b"plain text\n"), None);
}

#[test]
fn unknown_files_have_no_language() {
    assert_eq!(detect("/p/image.png", ""), None);
    assert_eq!(detect("/p/notes", "plain text\n"), None);
    assert_eq!(
        detect_language(Registry::builtin(), None, b"text without hints\n"),
        None
    );
}

#[test]
fn language_names_resolve_through_aliases() {
    let registry = Registry::builtin();
    let resolve = |name: &str| registry.resolve_name(name).map(|id| id.to_string());
    assert_eq!(resolve("py").as_deref(), Some("python"));
    assert_eq!(resolve("golang").as_deref(), Some("go"));
    assert_eq!(resolve("c#").as_deref(), Some("csharp"));
    assert_eq!(resolve("c++").as_deref(), Some("cpp"));
    assert_eq!(resolve("yml").as_deref(), Some("yaml"));
    assert_eq!(resolve("kotlin").as_deref(), Some("kotlin"));
    assert_eq!(resolve("cobol"), None);
}

#[test]
fn unknown_language_fails_with_unsupported_language() {
    let registry = Registry::builtin();
    let error = registry
        .parse(&language("cobol"), b"DISPLAY 'X'.")
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    let error = symbols_of(registry, b"x", &language("cobol")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
}

/// Provider of a Starlark language that reuses the Python grammar.
struct StarlarkProvider;

impl GrammarProvider for StarlarkProvider {
    fn languages(&self) -> Vec<LanguageSpec> {
        vec![LanguageSpec {
            id: language("starlark"),
            name: "Starlark".to_owned(),
            extensions: vec!["star".to_owned(), "bzl".to_owned()],
            file_names: vec!["BUILD.bazel".to_owned()],
            interpreters: Vec::new(),
            aliases: vec!["bazel".to_owned()],
            grammar_version: "0.25.0".to_owned(),
            programming: true,
        }]
    }

    fn grammar(&self, id: &LanguageId) -> Option<Grammar> {
        (id.as_str() == "starlark").then(|| Grammar {
            language: tree_sitter_python::LANGUAGE.into(),
            tags: Some(Cow::Borrowed(
                "(function_definition name: (identifier) @name) @definition.function",
            )),
            locals: None,
        })
    }
}

#[test]
fn grammar_provider_adds_a_language() {
    let registry = Registry::with_provider(Arc::new(StarlarkProvider)).unwrap();
    let starlark = language("starlark");
    assert_eq!(
        registry.detect(Some(Path::new("/w/BUILD.bazel")), b""),
        Some(starlark.clone())
    );
    assert_eq!(
        registry.detect(Some(Path::new("/w/defs.bzl")), b""),
        Some(starlark.clone())
    );
    assert_eq!(registry.resolve_name("bazel"), Some(starlark.clone()));
    let symbols = symbols_of(&registry, b"def rule_impl(ctx):\n    pass\n", &starlark).unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "rule_impl");
    assert_eq!(symbols[0].kind, SymbolKind::Function);
    let listed = registry.languages();
    assert_eq!(listed.len(), Registry::builtin().specs().count() + 1);
    let entry = listed
        .iter()
        .find(|info| info.id == starlark)
        .expect("starlark is listed");
    assert_eq!(entry.grammar_version.as_deref(), Some("0.25.0"));
    assert_eq!(
        registry.detect(Some(Path::new("/w/main.rs")), b""),
        Some(language("rust"))
    );
}

#[test]
fn grammar_provider_cannot_register_a_language_twice() {
    let mut registry = Registry::with_provider(Arc::new(StarlarkProvider)).unwrap();
    let before = registry.specs().count();
    assert!(registry.add(Arc::new(StarlarkProvider)).is_err());
    assert_eq!(registry.specs().count(), before);
    assert_eq!(before, Registry::builtin().specs().count() + 1);
}

#[test]
fn empty_registry_knows_no_language() {
    let registry = Registry::empty();
    assert_eq!(registry.specs().count(), 0);
    assert_eq!(registry.detect(Some(Path::new("/p/main.rs")), b""), None);
}

#[test]
fn symbol_kinds_round_trip_through_their_names() {
    let kinds = [
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::Variable,
        SymbolKind::Constant,
        SymbolKind::Field,
        SymbolKind::Module,
        SymbolKind::Type,
        SymbolKind::Macro,
    ];
    for kind in kinds {
        assert_eq!(parse_kind(kind_name(kind)), Some(kind));
    }
    assert_eq!(parse_kind("widget"), None);
}

fn rank(query: &str, mode: MatchMode, name: &str) -> u8 {
    NameMatcher::new(query, mode).unwrap().rank(name)
}

#[test]
fn exact_match_requires_the_same_name() {
    assert_eq!(rank("parse", MatchMode::Exact, "parse"), RANK_EXACT);
    assert_eq!(rank("parse", MatchMode::Exact, "Parse"), RANK_EXACT - 1);
    assert_eq!(rank("parse", MatchMode::Exact, "parser"), 0);
}

#[test]
fn prefix_match_ignores_case() {
    assert_eq!(rank("get", MatchMode::Prefix, "getUser"), 4);
    assert_eq!(rank("GET", MatchMode::Prefix, "getUser"), 4);
    assert_eq!(rank("user", MatchMode::Prefix, "getUser"), 0);
}

#[test]
fn substring_match_finds_inner_text() {
    assert_eq!(rank("user", MatchMode::Substring, "getUserById"), 3);
    assert_eq!(rank("get", MatchMode::Substring, "getUserById"), 4);
    assert_eq!(rank("admin", MatchMode::Substring, "getUserById"), 0);
}

#[test]
fn fuzzy_match_uses_camel_and_snake_case_words() {
    assert_eq!(rank("get_user", MatchMode::Fuzzy, "getUserById"), 2);
    assert_eq!(rank("userBy", MatchMode::Fuzzy, "get_user_by_id"), 2);
    assert_eq!(rank("gubi", MatchMode::Fuzzy, "getUserById"), 1);
    assert_eq!(rank("xyz", MatchMode::Fuzzy, "getUserById"), 0);
    let exact = rank("parse", MatchMode::Fuzzy, "parse");
    let words = rank("parse", MatchMode::Fuzzy, "parse_config");
    let letters = rank("pcfg", MatchMode::Fuzzy, "parse_config");
    assert!(exact > words && words > letters && letters > 0);
}

#[test]
fn regex_match_ranks_equal_names_first() {
    assert_eq!(rank("^get", MatchMode::Regex, "getUser"), 2);
    assert_eq!(rank("^get", MatchMode::Regex, "setUser"), 0);
    let regex = Regex::new("^(get|set)User$", "").unwrap();
    let matcher = NameMatcher::from_regex(&regex).unwrap();
    assert_eq!(matcher.mode(), MatchMode::Regex);
    assert_eq!(matcher.rank("setUser"), 2);
    let case_blind = NameMatcher::from_regex(&Regex::new("^GET", "i").unwrap()).unwrap();
    assert!(case_blind.rank("getUser") > 0);
}

#[test]
fn invalid_regex_is_rejected() {
    assert!(NameMatcher::new("(", MatchMode::Regex).is_err());
}

#[test]
fn match_modes_parse_from_their_names() {
    for (text, mode) in [
        ("exact", MatchMode::Exact),
        ("prefix", MatchMode::Prefix),
        ("substring", MatchMode::Substring),
        ("fuzzy", MatchMode::Fuzzy),
        ("regex", MatchMode::Regex),
    ] {
        assert_eq!(text.parse::<MatchMode>().unwrap(), mode);
    }
    assert!("glob".parse::<MatchMode>().is_err());
}

#[test]
fn names_split_into_words() {
    assert_eq!(name_words("getUserById"), ["get", "user", "by", "id"]);
    assert_eq!(name_words("get_user_by_id"), ["get", "user", "by", "id"]);
    assert_eq!(
        name_words("HTTPServer2Config"),
        ["http", "server2", "config"]
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
