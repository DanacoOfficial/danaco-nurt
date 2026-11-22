// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `find`: entries below a directory, selected by name pattern and
//! attributes.
//!
//! The walk, the records and the orders are those of the listing in
//! [`super::list`]. `find` adds name patterns (`glob`, `case_sensitive`),
//! exclusions (`exclude`, `exclude_preset`, `gitignore`), links
//! (`follow_links`), file types (`file_type`) and fuzzy ranking (`fuzzy`).
//! The root itself is a candidate at depth 0, so `find "/a" depth: 0 type:
//! any` yields exactly one record for an existing path and fails with
//! `E_NOT_FOUND` otherwise.

use std::sync::Arc;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::Value;

use super::list::{
    Listing, NameFilter, Root, SORT, Selection, SortKey, SortSpec, View, actual_root, choice,
    depth_param, descending, kind_param, root_param, static_info, type_error, unknown_value,
};
use crate::common::params::{WalkFilters, walk_filters};
use crate::fuzzy::FuzzyQuery;
use crate::walk::{KindFilter, WalkOptions};

const COMMAND: &str = "find";
const GLOB: &str = "glob";
const FOLLOW_LINKS: &str = "follow_links";
const FILE_TYPE: &str = "file_type";
const FUZZY: &str = "fuzzy";
const CASE_SENSITIVE: &str = "case_sensitive";

const SORT_VALUES: [&str; 4] = ["path", "modified", "size", "rank"];

/// Glob that matches every path; a list containing it selects every entry.
const MATCH_ALL: &str = "**";

/// Shallowest depth of `find`: the root itself.
const MIN_DEPTH: u32 = 0;

/// File type of `file_type`: extensions and whole file names.
struct FileType {
    name: &'static str,
    extensions: &'static [&'static str],
    names: &'static [&'static str],
}

const fn file_type(
    name: &'static str,
    extensions: &'static [&'static str],
    names: &'static [&'static str],
) -> FileType {
    FileType {
        name,
        extensions,
        names,
    }
}

/// File types of `file_type`, sorted by name.
const FILE_TYPES: &[FileType] = &[
    file_type("c", &["c", "h"], &[]),
    file_type(
        "cpp",
        &["cc", "cpp", "cxx", "c++", "h", "hh", "hpp", "hxx", "inl"],
        &[],
    ),
    file_type("cs", &["cs", "csx"], &[]),
    file_type("css", &["css", "scss", "sass", "less"], &[]),
    file_type("csv", &["csv", "tsv"], &[]),
    file_type("docker", &["dockerfile"], &["Dockerfile"]),
    file_type("go", &["go"], &[]),
    file_type("html", &["htm", "html", "xhtml"], &[]),
    file_type("java", &["java"], &[]),
    file_type("js", &["js", "jsx", "mjs", "cjs"], &[]),
    file_type("json", &["json", "jsonc", "jsonl", "ndjson"], &[]),
    file_type("kotlin", &["kt", "kts"], &[]),
    file_type("lua", &["lua"], &[]),
    file_type(
        "make",
        &["mk", "mak"],
        &["Makefile", "makefile", "GNUmakefile"],
    ),
    file_type("md", &["md", "markdown", "mdx"], &[]),
    file_type("php", &["php"], &[]),
    file_type("ps", &["ps1", "psm1", "psd1"], &[]),
    file_type("py", &["py", "pyi", "pyw"], &[]),
    file_type("rb", &["rb", "rake", "gemspec"], &["Gemfile", "Rakefile"]),
    file_type("rust", &["rs"], &[]),
    file_type("sh", &["sh", "bash", "zsh"], &[]),
    file_type("sql", &["sql"], &[]),
    file_type("swift", &["swift"], &[]),
    file_type("tex", &["tex", "sty", "cls", "bib"], &[]),
    file_type("toml", &["toml"], &["Cargo.lock"]),
    file_type("ts", &["ts", "tsx", "mts", "cts"], &[]),
    file_type("txt", &["txt"], &[]),
    file_type("xml", &["xml", "xsd", "xsl", "xslt", "svg"], &[]),
    file_type("yaml", &["yaml", "yml"], &[]),
];

/// Implementation of `find`.
pub(crate) struct Find;

impl StageImpl for Find {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        FindParams::read(args)?.listing_parts()?;
        Ok(static_info(args))
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Find {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let params = FindParams::read(args)?;
        let (options, selection) = params.listing_parts()?;
        let fs = ctx.require_fs()?;
        let root = actual_root(ctx, fs, root_param(args, ctx)?);
        params.listing(root, options, selection).run(ctx)
    }
}

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Find)]
}

/// Parameters of `find`.
struct FindParams {
    globs: Vec<String>,
    kind: KindFilter,
    depth: Option<u32>,
    follow_links: bool,
    filters: WalkFilters,
    names: Option<NameFilter>,
    fuzzy: Option<String>,
    case_sensitive: bool,
    sort: SortSpec,
    view: View,
}

impl FindParams {
    fn read(args: &BoundArgs) -> Result<Self> {
        let fuzzy = match args.value(FUZZY) {
            None | Some(Value::Null) => None,
            Some(Value::Text(text)) => Some(text.clone()),
            Some(_) => return Err(type_error(args.command(), FUZZY, "a text")),
        };
        let key = match choice(args, SORT, &SORT_VALUES)? {
            Some("modified") => SortKey::Modified,
            Some("size") => SortKey::Size,
            Some("rank") if fuzzy.is_none() => {
                return Err(Error::new(
                    ErrorCode::MissingParam,
                    "`sort: rank` needs `fuzzy`: give the text to rank the names by",
                ));
            }
            Some("rank") => SortKey::Rank,
            _ => SortKey::Path,
        };
        let mut globs = text_list(args, GLOB, "a glob or a list of globs")?;
        if args.value(GLOB).is_none() || globs.iter().any(|glob| glob == MATCH_ALL) {
            globs.clear();
        }
        Ok(Self {
            globs,
            kind: kind_param(args, KindFilter::File)?,
            depth: depth_param(args, MIN_DEPTH)?,
            follow_links: args.bool(FOLLOW_LINKS)?.unwrap_or(false),
            filters: walk_filters(args)?,
            names: file_types(args)?,
            fuzzy,
            case_sensitive: args.bool(CASE_SENSITIVE)?.unwrap_or(true),
            sort: SortSpec {
                key,
                descending: descending(args)?,
            },
            view: View::read(args)?,
        })
    }

    /// Settings of the walker and the selection.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` for a glob that is not valid.
    fn listing_parts(&self) -> Result<(WalkOptions, Selection)> {
        let mut options = WalkOptions {
            min_depth: MIN_DEPTH,
            max_depth: self.depth,
            kind: self.kind,
            follow_links: self.follow_links,
            globs: self.globs.clone(),
            case_sensitive: self.case_sensitive,
            ..WalkOptions::default()
        };
        self.filters.apply(&mut options);
        let globs = if self.view == View::Sizes {
            compile_globs(&self.globs, self.case_sensitive)?
        } else {
            None
        };
        let fuzzy = self.fuzzy.as_deref().map(|query| {
            let case_sensitive = query.chars().any(char::is_uppercase);
            FuzzyQuery::new(query, case_sensitive)
        });
        let selection = Selection {
            min_depth: MIN_DEPTH,
            max_depth: self.depth,
            kind: self.kind,
            globs,
            names: self.names.clone(),
            fuzzy,
        };
        Ok((options, selection))
    }

    fn listing(self, root: Root, options: WalkOptions, selection: Selection) -> Listing {
        Listing {
            root,
            options,
            view: self.view,
            selection,
            sort: self.sort,
        }
    }
}

/// Patterns of a `glob | list<glob>` or `list<text>` parameter.
fn text_list(args: &BoundArgs, name: &str, expected: &str) -> Result<Vec<String>> {
    let item = |value: &Value| match value {
        Value::Glob(text) | Value::Text(text) | Value::Enum(text) => Ok(text.clone()),
        _ => Err(type_error(args.command(), name, expected)),
    };
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(item).collect(),
        Some(value) => item(value).map(|text| vec![text]),
    }
}

/// File names selected by `file_type`; `None` for an empty list.
fn file_types(args: &BoundArgs) -> Result<Option<NameFilter>> {
    let wanted = text_list(args, FILE_TYPE, "a list of file types")?;
    if wanted.is_empty() {
        return Ok(None);
    }
    let mut extensions = Vec::new();
    let mut names = Vec::new();
    for name in &wanted {
        let Some(found) = FILE_TYPES.iter().find(|known| known.name == name.as_str()) else {
            let known: Vec<&str> = FILE_TYPES.iter().map(|known| known.name).collect();
            return Err(unknown_value(FILE_TYPE, name, &known));
        };
        extensions.extend_from_slice(found.extensions);
        names.extend_from_slice(found.names);
    }
    Ok(Some(NameFilter::new(extensions, names)))
}

/// Globs compiled as the walker compiles them: `*` and `?` do not match
/// `/`, `\` escapes, letter case as `case_sensitive`.
fn compile_globs(patterns: &[String], case_sensitive: bool) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let invalid = |pattern: &str, reason: String| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("glob `{pattern}` is not valid: {reason}"),
        )
        .with_found(pattern)
    };
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .backslash_escape(true)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|error| invalid(pattern, error.kind().to_string()))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| invalid(&patterns.join(", "), error.to_string()))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::{Entry, EntryType};
    use nurt_contracts::report::{Notable, NotableEntry};
    use nurt_contracts::stage::{ArgValue, RenderHint};
    use nurt_contracts::value::Size;
    use nurt_testkit::ctx::TestContext;

    use nurt_contracts::fs::AbsPath;
    use nurt_testkit::vfs::test_path;

    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).expect("test paths are absolute")
    }

    struct Run {
        entries: Vec<Entry>,
        notes: Vec<NotableEntry>,
        render: Option<RenderHint>,
    }

    fn args(path: &str, given: &[(&str, Value)]) -> BoundArgs {
        given.iter().fold(
            BoundArgs::new(COMMAND).with(
                "path",
                ArgValue::Value(Value::Path(abs(path).to_language_string())),
                false,
            ),
            |args, (name, value)| args.with(*name, ArgValue::Value(value.clone()), false),
        )
    }

    fn choice_value(text: &str) -> Value {
        Value::Enum(text.to_owned())
    }

    fn glob(text: &str) -> Value {
        Value::Glob(text.to_owned())
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        let ctx = test.ctx();
        let output = Find.execute(args, StageInput::none(), &ctx)?;
        let render = output.render;
        let entries = StageInput::from(output)
            .into_records()?
            .map(|item| item.and_then(|record| Entry::try_from(record).map_err(Error::from)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Run {
            entries,
            notes: ctx.take_notes(),
            render,
        })
    }

    fn relatives(run: &Run) -> Vec<String> {
        run.entries
            .iter()
            .map(|entry| entry.relative.clone().unwrap_or_default())
            .collect()
    }

    fn project() -> TestContext {
        let test = TestContext::new();
        let fs = test.fs();
        for (path, content) in [
            ("/proj/main.rs", "fn main() {}\n"),
            ("/proj/a.txt", "a\n"),
            ("/proj/a/b.rs", "pub fn b() {}\n"),
            ("/proj/src/fooBar.ts", "export {}\n"),
            ("/proj/src/fabric.ts", "export {}\n"),
            ("/proj/src/README.md", "# src\n"),
            ("/proj/node_modules/x/index.js", "module.exports = 1;\n"),
            ("/proj/.git/HEAD", "ref: refs/heads/main\n"),
            ("/proj/.gitignore", "*.log\n"),
            ("/proj/build.log", "ok\n"),
            ("/proj/Makefile", "all:\n"),
        ] {
            fs.write_file(&abs(path), content.as_bytes()).unwrap();
        }
        test
    }

    #[test]
    fn finds_files_at_any_depth_in_path_order() {
        let test = project();
        let found = run(&test, &args("/proj", &[(GLOB, glob("**/*.rs"))])).unwrap();
        assert_eq!(relatives(&found), ["a/b.rs", "main.rs"]);
        assert!(
            found
                .entries
                .iter()
                .all(|entry| entry.kind == EntryType::File)
        );
        assert_eq!(found.entries[0].depth, Some(2));
    }

    #[test]
    fn default_glob_finds_every_file_and_directory_comes_before_its_contents() {
        let test = project();
        let found = run(&test, &args("/proj", &[(GLOB, glob(MATCH_ALL))])).unwrap();
        let listed = relatives(&found);
        let directory_child = listed.iter().position(|path| path == "a/b.rs").unwrap();
        let sibling = listed.iter().position(|path| path == "a.txt").unwrap();
        assert!(directory_child < sibling);
        assert_eq!(listed.len(), 11);
        let without_glob = run(&test, &args("/proj", &[])).unwrap();
        assert_eq!(relatives(&without_glob), listed);
    }

    #[test]
    fn glob_without_double_star_matches_the_top_level_only() {
        let test = project();
        let found = run(&test, &args("/proj", &[(GLOB, glob("*.rs"))])).unwrap();
        assert_eq!(relatives(&found), ["main.rs"]);
        let several = run(
            &test,
            &args(
                "/proj",
                &[(GLOB, Value::List(vec![glob("*.txt"), glob("src/*.md")]))],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&several), ["a.txt", "src/README.md"]);
    }

    #[test]
    fn letter_case_of_globs_follows_case_sensitive() {
        let test = project();
        let exact = run(&test, &args("/proj", &[(GLOB, glob("**/readme.md"))])).unwrap();
        assert!(exact.entries.is_empty());
        let folded = run(
            &test,
            &args(
                "/proj",
                &[
                    (GLOB, glob("**/readme.md")),
                    (CASE_SENSITIVE, Value::Bool(false)),
                ],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&folded), ["src/README.md"]);
    }

    #[test]
    fn depth_zero_yields_the_root_alone() {
        let test = project();
        let given = args(
            "/proj",
            &[("depth", Value::Int(0)), ("type", choice_value("any"))],
        );
        let found = run(&test, &given).unwrap();
        assert_eq!(found.entries.len(), 1);
        assert_eq!(found.entries[0].relative.as_deref(), Some("."));
        assert_eq!(found.entries[0].depth, Some(0));
        assert_eq!(found.entries[0].kind, EntryType::Directory);
        let file = run(&test, &args("/proj/a.txt", &[("depth", Value::Int(0))])).unwrap();
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].name, "a.txt");
        let missing = run(&test, &args("/proj/gone.txt", &[("depth", Value::Int(0))]))
            .err()
            .unwrap();
        assert_eq!(missing.code(), ErrorCode::NotFound);
    }

    #[test]
    fn exclusions_presets_and_ignore_files_remove_entries() {
        let test = project();
        let presets = run(
            &test,
            &args(
                "/proj",
                &[(
                    "exclude_preset",
                    Value::List(vec![choice_value("vcs"), choice_value("deps")]),
                )],
            ),
        )
        .unwrap();
        let listed = relatives(&presets);
        assert!(!listed.iter().any(|path| path.starts_with(".git/")));
        assert!(!listed.iter().any(|path| path.starts_with("node_modules/")));
        assert!(listed.contains(&"build.log".to_owned()));
        let ignored = run(&test, &args("/proj", &[("gitignore", Value::Bool(true))])).unwrap();
        assert!(!relatives(&ignored).contains(&"build.log".to_owned()));
        let patterns = run(
            &test,
            &args(
                "/proj",
                &[(
                    "exclude",
                    Value::List(vec![glob("*.ts"), glob("!fooBar.ts")]),
                )],
            ),
        )
        .unwrap();
        let listed = relatives(&patterns);
        assert!(listed.contains(&"src/fooBar.ts".to_owned()));
        assert!(!listed.contains(&"src/fabric.ts".to_owned()));
    }

    #[test]
    fn file_types_select_by_extension_and_name() {
        let test = project();
        let typed = run(
            &test,
            &args(
                "/proj",
                &[(FILE_TYPE, Value::List(vec![Value::Text("ts".to_owned())]))],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&typed), ["src/fabric.ts", "src/fooBar.ts"]);
        let make = run(
            &test,
            &args("/proj", &[(FILE_TYPE, Value::Text("make".to_owned()))]),
        )
        .unwrap();
        assert_eq!(relatives(&make), ["Makefile"]);
        let unknown = run(
            &test,
            &args("/proj", &[(FILE_TYPE, Value::Text("cobol".to_owned()))]),
        )
        .err()
        .unwrap();
        assert_eq!(unknown.code(), ErrorCode::UnknownValue);
        assert_eq!(unknown.found.as_deref(), Some("cobol"));
    }

    #[test]
    fn fuzzy_ranks_names_and_drops_others() {
        let test = project();
        let ranked = run(
            &test,
            &args(
                "/proj",
                &[
                    (FUZZY, Value::Text("fb".to_owned())),
                    (SORT, choice_value("rank")),
                ],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&ranked), ["src/fooBar.ts", "src/fabric.ts"]);
        let first = ranked.entries[0].rank.unwrap();
        let second = ranked.entries[1].rank.unwrap();
        assert!(first > second);
        let filtered = run(
            &test,
            &args("/proj", &[(FUZZY, Value::Text("fb".to_owned()))]),
        )
        .unwrap();
        assert_eq!(relatives(&filtered), ["src/fabric.ts", "src/fooBar.ts"]);
    }

    #[test]
    fn fuzzy_query_with_capitals_respects_case() {
        let test = project();
        let found = run(
            &test,
            &args("/proj", &[(FUZZY, Value::Text("FB".to_owned()))]),
        )
        .unwrap();
        assert!(found.entries.is_empty());
        let found = run(
            &test,
            &args("/proj", &[(FUZZY, Value::Text("fB".to_owned()))]),
        )
        .unwrap();
        assert_eq!(relatives(&found), ["src/fooBar.ts"]);
    }

    #[test]
    fn rank_order_needs_a_fuzzy_query() {
        let test = project();
        let error = run(&test, &args("/proj", &[(SORT, choice_value("rank"))]))
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        let wrong = run(&test, &args("/proj", &[(FUZZY, Value::Int(1))]))
            .err()
            .unwrap();
        assert_eq!(wrong.code(), ErrorCode::Type);
    }

    #[test]
    fn link_loops_are_not_entered_when_following_links() {
        let test = project();
        test.fs()
            .symlink(abs("/proj").as_path(), &abs("/proj/src/back"))
            .unwrap();
        let followed = run(
            &test,
            &args(
                "/proj",
                &[
                    (GLOB, glob("src/**")),
                    (FOLLOW_LINKS, Value::Bool(true)),
                    ("type", choice_value("any")),
                ],
            ),
        )
        .unwrap();
        assert!(relatives(&followed).contains(&"src/back".to_owned()));
        assert!(
            !relatives(&followed)
                .iter()
                .any(|path| path.starts_with("src/back/"))
        );
        let note = followed
            .notes
            .iter()
            .find(|note| note.code == Notable::LinkLoopSkipped)
            .unwrap();
        assert_eq!(note.count, Some(1));
        let links = run(&test, &args("/proj", &[("type", choice_value("link"))])).unwrap();
        assert_eq!(relatives(&links), ["src/back"]);
    }

    #[test]
    fn sizes_view_totals_every_file_below_matching_directories() {
        let test = project();
        let found = run(
            &test,
            &args(
                "/proj",
                &[
                    (GLOB, glob("src")),
                    ("type", choice_value("directory")),
                    ("view", choice_value("sizes")),
                ],
            ),
        )
        .unwrap();
        assert_eq!(found.render, Some(RenderHint::Sizes));
        assert_eq!(relatives(&found), ["src"]);
        assert_eq!(found.entries[0].file_count, Some(3));
        assert_eq!(
            found.entries[0].total_size,
            Some(Size::from_bytes(10 + 10 + 6))
        );
    }

    #[test]
    fn tree_view_orders_depth_first() {
        let test = project();
        let found = run(
            &test,
            &args(
                "/proj",
                &[(GLOB, glob("src/**")), ("view", choice_value("tree"))],
            ),
        )
        .unwrap();
        assert_eq!(found.render, Some(RenderHint::Tree));
        assert_eq!(
            relatives(&found),
            ["src/README.md", "src/fabric.ts", "src/fooBar.ts"]
        );
    }

    #[test]
    fn invalid_globs_are_rejected() {
        let test = project();
        let error = run(&test, &args("/proj", &[(GLOB, glob("src/[a-"))]))
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        let sizes = compile_globs(&["[z-".to_owned()], true).err().unwrap();
        assert_eq!(sizes.code(), ErrorCode::InvalidValue);
        assert_eq!(sizes.found.as_deref(), Some("[z-"));
        let wrong = run(&test, &args("/proj", &[(GLOB, Value::Int(3))]))
            .err()
            .unwrap();
        assert_eq!(wrong.code(), ErrorCode::Type);
    }

    #[test]
    fn hidden_entries_can_be_left_out() {
        let test = project();
        let found = run(&test, &args("/proj", &[("hidden", Value::Bool(false))])).unwrap();
        assert!(!relatives(&found).iter().any(|path| path.starts_with('.')));
    }

    #[test]
    fn file_type_table_is_sorted_and_unique() {
        let names: Vec<&str> = FILE_TYPES.iter().map(|known| known.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
        assert!(
            FILE_TYPES
                .iter()
                .all(|known| !known.extensions.is_empty() || !known.names.is_empty())
        );
    }

    #[test]
    fn descriptor_names_a_source_stage() {
        let descriptor = Find.descriptor();
        assert_eq!(descriptor.command, COMMAND);
        assert!(descriptor.source);
        assert_eq!(descriptor.input, InputKinds::None);
        assert!(Find.as_read().is_some());
    }
}
