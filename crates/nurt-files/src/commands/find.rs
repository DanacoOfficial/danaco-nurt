// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `find`: entries below a directory, selected by name pattern and
//! attributes.
//!
//! The walk, the records and the orders are those of the listing in
//! [`super::list`]. `find` adds name patterns (`glob`, `case_sensitive`),
//! exclusions (`exclude`, `exclude_preset`, `gitignore`), links
//! (`follow_links`), file types (`file_type`, the table of
//! [`super::file_types`] shared with `search`) and fuzzy ranking (`fuzzy`).
//! The root itself is a candidate at depth 0, so `find "/a" depth: 0 type:
//! any` yields exactly one record for an existing path and fails with
//! `E_NOT_FOUND` otherwise.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::Value;

use super::file_types::TypeFilter;
use super::list::{
    Listing, Root, SORT, Selection, SortKey, SortSpec, View, actual_root, choice, depth_param,
    descending, kind_param, root_param, static_info, type_error,
};
use crate::common::params::{WalkFilters, walk_filters};
use crate::fuzzy::FuzzyQuery;
use crate::walk::{KindFilter, WalkOptions, compile_globs};

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
    types: Option<TypeFilter>,
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
            types: file_types(args)?,
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
            names: self.types.clone(),
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

/// Files selected by `file_type`; `None` for an empty list.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not a text or a list of texts and
/// with `E_UNKNOWN_VALUE` for a name of no known type.
fn file_types(args: &BoundArgs) -> Result<Option<TypeFilter>> {
    let wanted = text_list(args, FILE_TYPE, "a list of file types")?;
    if wanted.is_empty() {
        return Ok(None);
    }
    TypeFilter::new(&wanted, &[], FILE_TYPE, FILE_TYPE).map(Some)
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
    fn globs_and_exclusions_match_decomposed_names_in_composed_form() {
        let test = TestContext::new();
        for path in [
            "/docs/cafe\u{301}.txt",
            "/docs/re\u{301}sume\u{301}/cv.txt",
            "/docs/plain.txt",
        ] {
            test.fs().write_file(&abs(path), b"\n").unwrap();
        }
        let composed = run(&test, &args("/docs", &[(GLOB, glob("caf\u{e9}.*"))])).unwrap();
        assert_eq!(relatives(&composed), ["caf\u{e9}.txt"]);
        assert!(composed.entries[0].raw_name.is_some());
        let nested = run(&test, &args("/docs", &[(GLOB, glob("r\u{e9}sum\u{e9}/*"))])).unwrap();
        assert_eq!(relatives(&nested), ["r\u{e9}sum\u{e9}/cv.txt"]);
        let decomposed = run(&test, &args("/docs", &[(GLOB, glob("cafe\u{301}.txt"))])).unwrap();
        assert_eq!(relatives(&decomposed), ["caf\u{e9}.txt"]);
        let sizes = run(
            &test,
            &args(
                "/docs",
                &[(GLOB, glob("caf\u{e9}.*")), ("view", choice_value("sizes"))],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&sizes), ["caf\u{e9}.txt"]);
        let excluded = run(
            &test,
            &args(
                "/docs",
                &[(
                    "exclude",
                    Value::List(vec![glob("caf\u{e9}.txt"), glob("r\u{e9}sum\u{e9}/")]),
                )],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&excluded), ["plain.txt"]);
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
        assert!(unknown.message.contains("`file_type`"));
        let wrong = run(&test, &args("/proj", &[(FILE_TYPE, Value::Int(1))]))
            .err()
            .unwrap();
        assert_eq!(wrong.code(), ErrorCode::Type);
    }

    #[test]
    fn file_types_follow_the_shared_table() {
        let test = TestContext::new();
        for path in [
            "/t/Rakefile",
            "/t/tasks.rake",
            "/t/feed.ndjson",
            "/t/log.jsonl",
            "/t/icon.svg",
            "/t/upper.SVG",
            "/t/data.tsv",
            "/t/style.less",
            "/t/tool.pyw",
        ] {
            test.fs().write_file(&abs(path), b"\n").unwrap();
        }
        let typed = |names: &[&str]| {
            let list = names
                .iter()
                .map(|name| Value::Text((*name).to_owned()))
                .collect();
            relatives(&run(&test, &args("/t", &[(FILE_TYPE, Value::List(list))])).unwrap())
        };
        assert_eq!(typed(&["ruby"]), ["Rakefile", "tasks.rake"]);
        assert_eq!(typed(&["jsonl"]), ["feed.ndjson", "log.jsonl"]);
        assert_eq!(typed(&["svg", "tsv"]), ["data.tsv", "icon.svg"]);
        assert_eq!(typed(&["less"]), ["style.less"]);
        assert_eq!(typed(&["py"]), ["tool.pyw"]);
        let renamed = run(
            &test,
            &args("/t", &[(FILE_TYPE, Value::Text("rb".to_owned()))]),
        )
        .err()
        .unwrap();
        assert_eq!(renamed.code(), ErrorCode::UnknownValue);
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
        let sizes = run(
            &test,
            &args(
                "/proj",
                &[(GLOB, glob("[z-")), ("view", choice_value("sizes"))],
            ),
        )
        .err()
        .unwrap();
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
    fn descriptor_names_a_source_stage() {
        let descriptor = Find.descriptor();
        assert_eq!(descriptor.command, COMMAND);
        assert!(descriptor.source);
        assert_eq!(descriptor.input, InputKinds::None);
        assert!(Find.as_read().is_some());
    }
}
