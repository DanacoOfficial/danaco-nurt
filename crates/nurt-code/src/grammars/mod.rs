// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::grammars
// Responsibility:   Built-in tree-sitter grammars.
// Dependencies:     nurt_contracts, tree_sitter
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Built-in tree-sitter grammars.
//!
//! Every built-in language has a symbol query in `queries/<language>/tags.scm`;
//! the languages with local analysis also have a scope query in
//! `queries/<language>/locals.scm`. The queries follow the capture
//! conventions described in [`crate::registry`].

use std::borrow::Cow;

use nurt_contracts::services::LanguageId;
use tree_sitter::Language;

use crate::registry::{Grammar, GrammarProvider, LanguageSpec};

/// Provider of the grammars compiled into the program.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinGrammars;

struct Builtin {
    id: &'static str,
    name: &'static str,
    extensions: &'static [&'static str],
    file_names: &'static [&'static str],
    interpreters: &'static [&'static str],
    aliases: &'static [&'static str],
    version: &'static str,
    programming: bool,
    language: fn() -> Language,
    tags: &'static str,
    locals: Option<&'static str>,
}

fn go() -> Language {
    tree_sitter_go::LANGUAGE.into()
}
fn rust() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}
fn python() -> Language {
    tree_sitter_python::LANGUAGE.into()
}
fn javascript() -> Language {
    tree_sitter_javascript::LANGUAGE.into()
}
fn typescript() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
fn tsx() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}
fn c() -> Language {
    tree_sitter_c::LANGUAGE.into()
}
fn cpp() -> Language {
    tree_sitter_cpp::LANGUAGE.into()
}
fn java() -> Language {
    tree_sitter_java::LANGUAGE.into()
}
fn ruby() -> Language {
    tree_sitter_ruby::LANGUAGE.into()
}
fn csharp() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}
fn php() -> Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}
fn bash() -> Language {
    tree_sitter_bash::LANGUAGE.into()
}
fn powershell() -> Language {
    tree_sitter_powershell::LANGUAGE.into()
}
fn json() -> Language {
    tree_sitter_json::LANGUAGE.into()
}
fn yaml() -> Language {
    tree_sitter_yaml::LANGUAGE.into()
}
fn toml() -> Language {
    tree_sitter_toml_ng::LANGUAGE.into()
}
fn markdown() -> Language {
    tree_sitter_md::LANGUAGE.into()
}
fn html() -> Language {
    tree_sitter_html::LANGUAGE.into()
}
fn css() -> Language {
    tree_sitter_css::LANGUAGE.into()
}
fn kotlin() -> Language {
    tree_sitter_kotlin_ng::LANGUAGE.into()
}
fn swift() -> Language {
    tree_sitter_swift::LANGUAGE.into()
}
fn xml() -> Language {
    tree_sitter_xml::LANGUAGE_XML.into()
}

const NONE: &[&str] = &[];

const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "go",
        name: "Go",
        extensions: &["go"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["golang"],
        version: "0.25.0",
        programming: true,
        language: go,
        tags: include_str!("../../queries/go/tags.scm"),
        locals: Some(include_str!("../../queries/go/locals.scm")),
    },
    Builtin {
        id: "rust",
        name: "Rust",
        extensions: &["rs"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["rs"],
        version: "0.24.2",
        programming: true,
        language: rust,
        tags: include_str!("../../queries/rust/tags.scm"),
        locals: Some(include_str!("../../queries/rust/locals.scm")),
    },
    Builtin {
        id: "python",
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        file_names: &["SConstruct", "SConscript"],
        interpreters: &["python"],
        aliases: &["py", "python3"],
        version: "0.25.0",
        programming: true,
        language: python,
        tags: include_str!("../../queries/python/tags.scm"),
        locals: Some(include_str!("../../queries/python/locals.scm")),
    },
    Builtin {
        id: "javascript",
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        file_names: NONE,
        interpreters: &["node", "nodejs"],
        aliases: &["js", "jsx", "node"],
        version: "0.25.0",
        programming: true,
        language: javascript,
        tags: include_str!("../../queries/javascript/tags.scm"),
        locals: Some(include_str!("../../queries/javascript/locals.scm")),
    },
    Builtin {
        id: "typescript",
        name: "TypeScript",
        extensions: &["ts", "mts", "cts"],
        file_names: NONE,
        interpreters: &["deno", "ts-node", "tsx"],
        aliases: &["ts"],
        version: "0.23.2",
        programming: true,
        language: typescript,
        tags: include_str!("../../queries/typescript/tags.scm"),
        locals: Some(include_str!("../../queries/typescript/locals.scm")),
    },
    Builtin {
        id: "tsx",
        name: "TypeScript JSX",
        extensions: &["tsx"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["typescriptreact"],
        version: "0.23.2",
        programming: true,
        language: tsx,
        tags: include_str!("../../queries/typescript/tags.scm"),
        locals: Some(include_str!("../../queries/typescript/locals.scm")),
    },
    Builtin {
        id: "c",
        name: "C",
        extensions: &["c", "h"],
        file_names: NONE,
        interpreters: NONE,
        aliases: NONE,
        version: "0.24.2",
        programming: true,
        language: c,
        tags: include_str!("../../queries/c/tags.scm"),
        locals: Some(include_str!("../../queries/c/locals.scm")),
    },
    Builtin {
        id: "cpp",
        name: "C++",
        extensions: &[
            "cc", "cpp", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl",
        ],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["c++", "cxx"],
        version: "0.23.4",
        programming: true,
        language: cpp,
        tags: include_str!("../../queries/cpp/tags.scm"),
        locals: Some(include_str!("../../queries/cpp/locals.scm")),
    },
    Builtin {
        id: "java",
        name: "Java",
        extensions: &["java"],
        file_names: NONE,
        interpreters: NONE,
        aliases: NONE,
        version: "0.23.5",
        programming: true,
        language: java,
        tags: include_str!("../../queries/java/tags.scm"),
        locals: Some(include_str!("../../queries/java/locals.scm")),
    },
    Builtin {
        id: "ruby",
        name: "Ruby",
        extensions: &["rb", "rake", "gemspec", "ru"],
        file_names: &["Gemfile", "Rakefile", "Guardfile", "Vagrantfile", "Podfile"],
        interpreters: &["ruby"],
        aliases: &["rb"],
        version: "0.23.1",
        programming: true,
        language: ruby,
        tags: include_str!("../../queries/ruby/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "csharp",
        name: "C#",
        extensions: &["cs", "csx"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["c#", "cs", "c-sharp", "c_sharp"],
        version: "0.24.0",
        programming: true,
        language: csharp,
        tags: include_str!("../../queries/csharp/tags.scm"),
        locals: Some(include_str!("../../queries/csharp/locals.scm")),
    },
    Builtin {
        id: "php",
        name: "PHP",
        extensions: &["php", "phtml", "php8"],
        file_names: NONE,
        interpreters: &["php"],
        aliases: NONE,
        version: "0.25.2",
        programming: true,
        language: php,
        tags: include_str!("../../queries/php/tags.scm"),
        locals: Some(include_str!("../../queries/php/locals.scm")),
    },
    Builtin {
        id: "bash",
        name: "Bash",
        extensions: &["sh", "bash", "ksh"],
        file_names: &[".bashrc", ".bash_profile", ".profile", "PKGBUILD"],
        interpreters: &["sh", "bash", "dash", "ksh", "zsh"],
        aliases: &["sh", "shell", "zsh"],
        version: "0.25.1",
        programming: true,
        language: bash,
        tags: include_str!("../../queries/bash/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "powershell",
        name: "PowerShell",
        extensions: &["ps1", "psm1", "psd1"],
        file_names: NONE,
        interpreters: &["pwsh", "powershell"],
        aliases: &["ps1", "pwsh", "posh"],
        version: "0.26.6",
        programming: true,
        language: powershell,
        tags: include_str!("../../queries/powershell/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "json",
        name: "JSON",
        extensions: &["json", "jsonc", "geojson", "webmanifest"],
        file_names: &[".babelrc", ".eslintrc", "composer.lock"],
        interpreters: NONE,
        aliases: NONE,
        version: "0.24.8",
        programming: false,
        language: json,
        tags: include_str!("../../queries/json/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "yaml",
        name: "YAML",
        extensions: &["yaml", "yml"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["yml"],
        version: "0.7.2",
        programming: false,
        language: yaml,
        tags: include_str!("../../queries/yaml/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "toml",
        name: "TOML",
        extensions: &["toml"],
        file_names: &["Cargo.lock", "Pipfile", "poetry.lock"],
        interpreters: NONE,
        aliases: NONE,
        version: "0.7.0",
        programming: false,
        language: toml,
        tags: include_str!("../../queries/toml/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "markdown",
        name: "Markdown",
        extensions: &["md", "markdown", "mkd"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["md"],
        version: "0.5.4",
        programming: false,
        language: markdown,
        tags: include_str!("../../queries/markdown/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "html",
        name: "HTML",
        extensions: &["html", "htm", "xhtml"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["xhtml"],
        version: "0.23.2",
        programming: false,
        language: html,
        tags: include_str!("../../queries/html/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "css",
        name: "CSS",
        extensions: &["css"],
        file_names: NONE,
        interpreters: NONE,
        aliases: NONE,
        version: "0.25.0",
        programming: false,
        language: css,
        tags: include_str!("../../queries/css/tags.scm"),
        locals: None,
    },
    Builtin {
        id: "kotlin",
        name: "Kotlin",
        extensions: &["kt", "kts"],
        file_names: NONE,
        interpreters: NONE,
        aliases: &["kt"],
        version: "1.1.0",
        programming: true,
        language: kotlin,
        tags: include_str!("../../queries/kotlin/tags.scm"),
        locals: Some(include_str!("../../queries/kotlin/locals.scm")),
    },
    Builtin {
        id: "swift",
        name: "Swift",
        extensions: &["swift"],
        file_names: NONE,
        interpreters: &["swift"],
        aliases: NONE,
        version: "0.7.4",
        programming: true,
        language: swift,
        tags: include_str!("../../queries/swift/tags.scm"),
        locals: Some(include_str!("../../queries/swift/locals.scm")),
    },
    Builtin {
        id: "xml",
        name: "XML",
        extensions: &[
            "xml", "xsd", "xsl", "xslt", "svg", "csproj", "vbproj", "fsproj", "props", "targets",
            "xaml", "resx", "wsdl", "rss", "atom", "plist",
        ],
        file_names: NONE,
        interpreters: NONE,
        aliases: NONE,
        version: "0.7.0",
        programming: false,
        language: xml,
        tags: include_str!("../../queries/xml/tags.scm"),
        locals: None,
    },
];

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

impl GrammarProvider for BuiltinGrammars {
    fn languages(&self) -> Vec<LanguageSpec> {
        BUILTIN
            .iter()
            .filter_map(|builtin| {
                LanguageId::new(builtin.id).ok().map(|id| LanguageSpec {
                    id,
                    name: builtin.name.to_owned(),
                    extensions: owned(builtin.extensions),
                    file_names: owned(builtin.file_names),
                    interpreters: owned(builtin.interpreters),
                    aliases: owned(builtin.aliases),
                    grammar_version: builtin.version.to_owned(),
                    programming: builtin.programming,
                })
            })
            .collect()
    }

    fn grammar(&self, id: &LanguageId) -> Option<Grammar> {
        BUILTIN
            .iter()
            .find(|builtin| builtin.id == id.as_str())
            .map(|builtin| Grammar {
                language: (builtin.language)(),
                tags: Some(Cow::Borrowed(builtin.tags)),
                locals: builtin.locals.map(Cow::Borrowed),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;

    #[test]
    fn every_builtin_language_is_listed_with_a_valid_identifier() {
        assert_eq!(BuiltinGrammars.languages().len(), BUILTIN.len());
    }

    #[test]
    fn every_builtin_grammar_loads_with_its_queries() {
        let registry = Registry::builtin();
        for builtin in BUILTIN {
            let id = LanguageId::new(builtin.id).unwrap();
            let grammar = registry
                .grammar(&id)
                .unwrap_or_else(|error| panic!("{}: {error}", builtin.id));
            assert!(
                grammar.tags().is_some(),
                "{} has no symbol query",
                builtin.id
            );
            assert_eq!(grammar.locals().is_some(), builtin.locals.is_some());
        }
    }

    #[test]
    fn local_analysis_covers_the_twelve_parity_languages() {
        let with_locals: Vec<&str> = BUILTIN
            .iter()
            .filter(|builtin| builtin.locals.is_some())
            .map(|builtin| builtin.id)
            .collect();
        for id in [
            "python",
            "typescript",
            "javascript",
            "go",
            "rust",
            "c",
            "cpp",
            "java",
            "csharp",
            "php",
            "kotlin",
            "swift",
        ] {
            assert!(with_locals.contains(&id), "{id} has no scope query");
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
