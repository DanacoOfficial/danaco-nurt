// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File types of the `file_type` parameter of `find` and `search` and of the
//! `exclude_file_type` parameter of `search`.
//!
//! A type is a name and a list of file name patterns. The names and patterns
//! follow the built-in types of ripgrep (`rg --type-list`) for common
//! languages and formats, so a type such as `make` or `docker` also selects
//! files without an extension. A few patterns extend those types (`*.csx`,
//! `*.jsonc`, `*.ndjson`, `*.pyw`, `*.rake`), and `tsv` is a type of its own.
//! A pattern is matched against the whole file name; `*` stands for any
//! sequence of characters, and letter case is significant.

use nurt_contracts::error::{Error, ErrorCode, Result};

/// A named group of file name patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileType {
    /// Name used in `file_type`.
    pub(crate) name: &'static str,
    /// Name patterns of the type.
    pub(crate) globs: &'static [&'static str],
}

impl FileType {
    /// Whether a file name belongs to the type.
    pub(crate) fn matches(&self, file_name: &str) -> bool {
        self.globs
            .iter()
            .any(|glob| wildcard(glob.as_bytes(), file_name.as_bytes()))
    }
}

macro_rules! file_types {
    ($($name:literal => [$($glob:literal),+ $(,)?]),+ $(,)?) => {
        /// Known file types, sorted by name.
        pub(crate) const FILE_TYPES: &[FileType] = &[
            $(FileType { name: $name, globs: &[$($glob),+] }),+
        ];
    };
}

file_types! {
    "asm" => ["*.asm", "*.s", "*.S"],
    "c" => ["*.c", "*.h", "*.H", "*.c.in", "*.h.in", "*.cats"],
    "clojure" => ["*.clj", "*.cljc", "*.cljs", "*.cljx", "*.edn"],
    "cmake" => ["*.cmake", "CMakeLists.txt"],
    "config" => ["*.cfg", "*.conf", "*.config", "*.ini"],
    "cpp" => ["*.C", "*.H", "*.cc", "*.cpp", "*.cxx", "*.c++", "*.h", "*.hh", "*.hpp", "*.hxx", "*.h++", "*.inl"],
    "cs" => ["*.cs", "*.csx"],
    "css" => ["*.css", "*.scss"],
    "csv" => ["*.csv"],
    "dart" => ["*.dart"],
    "docker" => ["*Dockerfile*", "*dockerfile*"],
    "elixir" => ["*.ex", "*.eex", "*.exs", "*.heex", "*.leex", "*.livemd"],
    "erlang" => ["*.erl", "*.hrl"],
    "go" => ["*.go"],
    "gradle" => ["*.gradle", "*.gradle.kts"],
    "groovy" => ["*.groovy", "*.gradle"],
    "haskell" => ["*.hs", "*.lhs", "*.cpphs", "*.c2hs", "*.hsc"],
    "html" => ["*.htm", "*.html", "*.ejs"],
    "java" => ["*.java", "*.jsp", "*.jspx", "*.properties"],
    "js" => ["*.js", "*.jsx", "*.vue", "*.cjs", "*.mjs"],
    "json" => ["*.json", "*.jsonc", "composer.lock", "*.sarif"],
    "jsonl" => ["*.jsonl", "*.ndjson"],
    "julia" => ["*.jl"],
    "kotlin" => ["*.kt", "*.kts"],
    "less" => ["*.less"],
    "lua" => ["*.lua"],
    "make" => [
        "GNUmakefile", "Gnumakefile", "gnumakefile", "Makefile", "makefile",
        "Makefile.am", "makefile.am", "Makefile.in", "makefile.in", "*.mk", "*.mak",
    ],
    "markdown" => ["*.markdown", "*.md", "*.mdown", "*.mdwn", "*.mkd", "*.mkdn", "*.mdx"],
    "md" => ["*.markdown", "*.md", "*.mdown", "*.mdwn", "*.mkd", "*.mkdn", "*.mdx"],
    "nix" => ["*.nix"],
    "ocaml" => ["*.ml", "*.mli", "*.mll", "*.mly"],
    "perl" => ["*.perl", "*.pl", "*.PL", "*.plh", "*.plx", "*.pm", "*.t"],
    "php" => ["*.php", "*.php3", "*.php4", "*.php5", "*.php7", "*.php8", "*.pht", "*.phtml"],
    "protobuf" => ["*.proto"],
    "ps" => ["*.cdxml", "*.ps1", "*.ps1xml", "*.psd1", "*.psm1"],
    "py" => ["*.py", "*.pyi", "*.pyw"],
    "r" => ["*.R", "*.r", "*.Rmd", "*.Rnw"],
    "rst" => ["*.rst"],
    "ruby" => ["Gemfile", "*.gemspec", ".irbrc", "Rakefile", "*.rake", "*.rb", "config.ru"],
    "rust" => ["*.rs"],
    "sass" => ["*.sass", "*.scss"],
    "scala" => ["*.scala", "*.sbt"],
    "sh" => [
        "*.bash", ".bashrc", ".bash_profile", ".bash_login", ".bash_logout", "*.sh",
        ".profile", "*.zsh", ".zshrc", ".zshenv", ".zprofile", "*.ksh", "*.csh", "*.tcsh",
    ],
    "sql" => ["*.sql", "*.psql"],
    "svelte" => ["*.svelte"],
    "svg" => ["*.svg"],
    "swift" => ["*.swift"],
    "tex" => ["*.tex", "*.ltx", "*.cls", "*.sty", "*.bib", "*.dtx", "*.ins"],
    "tf" => ["*.tf", "*.tfvars", "*.tf.json"],
    "toml" => ["*.toml", "Cargo.lock"],
    "ts" => ["*.ts", "*.tsx", "*.cts", "*.mts"],
    "tsv" => ["*.tsv"],
    "txt" => ["*.txt"],
    "vue" => ["*.vue"],
    "xml" => ["*.xml", "*.xml.dist", "*.dtd", "*.xsl", "*.xslt", "*.xsd", "*.xjb", "*.rng", "*.sch", "*.xhtml"],
    "yaml" => ["*.yaml", "*.yml"],
    "zig" => ["*.zig"],
}

/// Type with the given name.
pub(crate) fn lookup(name: &str) -> Option<&'static FileType> {
    FILE_TYPES
        .binary_search_by(|known| known.name.cmp(name))
        .ok()
        .and_then(|index| FILE_TYPES.get(index))
}

/// Selection of files by the `file_type` and `exclude_file_type` lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TypeFilter {
    include: Vec<&'static FileType>,
    exclude: Vec<&'static FileType>,
}

impl TypeFilter {
    /// Builds the filter of the named types. Error messages name the
    /// parameters `include_param` and `exclude_param`.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_VALUE` for a name of no known type.
    pub(crate) fn new(
        include: &[String],
        exclude: &[String],
        include_param: &str,
        exclude_param: &str,
    ) -> Result<Self> {
        Ok(Self {
            include: resolve(include, include_param)?,
            exclude: resolve(exclude, exclude_param)?,
        })
    }

    /// Whether the filter selects every file.
    pub(crate) fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Whether a file with this name is searched: it belongs to one of the
    /// included types, or no type is included, and to none of the excluded
    /// types.
    pub(crate) fn accepts(&self, file_name: &str) -> bool {
        (self.include.is_empty() || self.include.iter().any(|kind| kind.matches(file_name)))
            && !self.exclude.iter().any(|kind| kind.matches(file_name))
    }
}

fn resolve(names: &[String], param: &str) -> Result<Vec<&'static FileType>> {
    names
        .iter()
        .map(|name| lookup(name.trim()).ok_or_else(|| unknown_type(name, param)))
        .collect()
}

fn unknown_type(name: &str, param: &str) -> Error {
    let known: Vec<&str> = FILE_TYPES.iter().map(|kind| kind.name).collect();
    Error::new(
        ErrorCode::UnknownValue,
        format!(
            "`{name}` is not a file type of `{param}`: use a type name such as rust, py, ts or md"
        ),
    )
    .with_found(name)
    .with_expected(serde_json::json!(known))
}

/// Matches a whole name against a pattern in which `*` stands for any
/// sequence of bytes.
fn wildcard(pattern: &[u8], name: &[u8]) -> bool {
    let Some(star) = pattern.iter().position(|&byte| byte == b'*') else {
        return pattern == name;
    };
    let (head, tail) = pattern.split_at(star);
    let tail = tail.get(1..).unwrap_or_default();
    let Some(rest) = name.strip_prefix(head) else {
        return false;
    };
    (0..=rest.len()).any(|skip| rest.get(skip..).is_some_and(|part| wildcard(tail, part)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(include: &[&str], exclude: &[&str]) -> TypeFilter {
        let owned = |names: &[&str]| {
            names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        TypeFilter::new(
            &owned(include),
            &owned(exclude),
            "file_type",
            "exclude_file_type",
        )
        .unwrap()
    }

    #[test]
    fn table_is_sorted_and_unique() {
        for pair in FILE_TYPES.windows(2) {
            assert!(
                pair[0].name < pair[1].name,
                "{} before {}",
                pair[0].name,
                pair[1].name
            );
        }
        assert!(FILE_TYPES.iter().all(|kind| !kind.globs.is_empty()));
    }

    #[test]
    fn common_types_follow_ripgrep_names() {
        for name in [
            "rust", "py", "js", "ts", "go", "c", "cpp", "java", "md", "json", "yaml", "toml", "sh",
            "make", "docker",
        ] {
            assert!(lookup(name).is_some(), "{name} is missing");
        }
        assert!(lookup("python").is_none());
    }

    #[test]
    fn extensions_and_whole_names_match() {
        let rust = lookup("rust").unwrap();
        assert!(rust.matches("lib.rs"));
        assert!(!rust.matches("lib.rs.orig"));
        let make = lookup("make").unwrap();
        assert!(make.matches("Makefile"));
        assert!(make.matches("rules.mk"));
        assert!(!make.matches("Makefile.txt"));
        let docker = lookup("docker").unwrap();
        assert!(docker.matches("Dockerfile"));
        assert!(docker.matches("api.Dockerfile"));
        assert!(docker.matches("Dockerfile.dev"));
        let cmake = lookup("cmake").unwrap();
        assert!(cmake.matches("CMakeLists.txt"));
    }

    #[test]
    fn extensions_beyond_ripgrep_belong_to_their_types() {
        for (name, file) in [
            ("cs", "build.csx"),
            ("json", "settings.jsonc"),
            ("jsonl", "events.ndjson"),
            ("py", "gui.pyw"),
            ("ruby", "deploy.rake"),
            ("tsv", "table.tsv"),
            ("svg", "logo.svg"),
            ("sass", "theme.sass"),
            ("less", "theme.less"),
        ] {
            let kind = lookup(name).unwrap();
            assert!(kind.matches(file), "{name} does not match {file}");
        }
        assert!(lookup("rb").is_none());
        assert!(!lookup("csv").unwrap().matches("table.tsv"));
        assert!(!lookup("xml").unwrap().matches("logo.svg"));
    }

    #[test]
    fn letter_case_is_significant() {
        let c = lookup("c").unwrap();
        assert!(c.matches("main.c"));
        assert!(!c.matches("main.C"));
        assert!(lookup("cpp").unwrap().matches("main.C"));
    }

    #[test]
    fn filter_includes_and_excludes() {
        let empty = filter(&[], &[]);
        assert!(empty.is_empty());
        assert!(empty.accepts("anything.bin"));
        let code = filter(&["rust", "toml"], &[]);
        assert!(code.accepts("main.rs"));
        assert!(code.accepts("Cargo.toml"));
        assert!(!code.accepts("README.md"));
        let no_docs = filter(&[], &["md"]);
        assert!(no_docs.accepts("main.rs"));
        assert!(!no_docs.accepts("README.md"));
        let both = filter(&["js"], &["vue"]);
        assert!(both.accepts("app.js"));
        assert!(!both.accepts("view.vue"));
    }

    #[test]
    fn unknown_types_are_rejected_with_the_known_names() {
        let error = TypeFilter::new(
            &["rustlang".to_owned()],
            &[],
            "file_type",
            "exclude_file_type",
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        assert_eq!(error.found.as_deref(), Some("rustlang"));
        assert!(error.message.contains("`file_type`"));
        let excluded =
            TypeFilter::new(&[], &["x".to_owned()], "file_type", "exclude_file_type").unwrap_err();
        assert!(excluded.message.contains("`exclude_file_type`"));
    }

    #[test]
    fn lookup_is_exact_and_the_filter_trims_names() {
        assert!(lookup("").is_none());
        assert!(lookup("Rust").is_none());
        let padded = TypeFilter::new(
            &[" rust ".to_owned()],
            &[],
            "file_type",
            "exclude_file_type",
        )
        .unwrap();
        assert!(padded.accepts("main.rs"));
        assert!(!padded.accepts("main.py"));
    }

    #[test]
    fn wildcards_work_on_the_bytes_of_multibyte_names() {
        let docker = lookup("docker").unwrap();
        assert!(docker.matches("\u{142}\u{f3}d\u{17a}.Dockerfile"));
        assert!(wildcard("*.\u{e9}".as_bytes(), "caf\u{e9}.\u{e9}".as_bytes()));
        assert!(!wildcard("*.\u{e9}".as_bytes(), "caf\u{e9}.e".as_bytes()));
    }

    #[test]
    fn wildcards_match_any_sequence() {
        assert!(wildcard(b"*", b""));
        assert!(wildcard(b"a*b*c", b"aXbYc"));
        assert!(wildcard(b"a*b*c", b"abc"));
        assert!(!wildcard(b"a*b*c", b"acb"));
        assert!(wildcard(b"exact", b"exact"));
        assert!(!wildcard(b"exact", b"exactly"));
    }
}
