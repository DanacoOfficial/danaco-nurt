// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `read` command: text of files with line selections, conditional
//! reads, outlines, document keys and notebook cells.
//!
//! In the source form the targets are the paths of `path` or the maps of
//! `targets`, each map with its own selector; in the stage form they are the
//! files of the input `entry` records, and directories among them are
//! skipped. Each target is loaded and recognised as text or binary content;
//! text goes to [`text::read_source`], and content that is not text and
//! `view: hex` go to [`binary::read_binary`].

pub(crate) mod binary;
mod cache;
mod select;
mod text;

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::EntryType;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};

use self::cache::{MemoryReadCache, ReadCache};
use self::select::{CONTEXT, DEFAULT_CONTEXT, PATH, Selector};
use self::text::{Loaded, ReadOptions, TagMatch, View};
use crate::common::params::{EncodingParam, encoding_param};
use crate::common::path::{path_list_arg, record_path, resolve};

const COMMAND: &str = "read";
const TARGETS: &str = "targets";
const VIEW: &str = "view";
const DEPTH: &str = "depth";
const UNCHANGED_SINCE: &str = "unchanged_since";
const SINCE: &str = "since";
const KNOWN: &str = "known";
const LINE_NUMBERS: &str = "line_numbers";
const MAX_LINE: &str = "max_line";

/// Field of `entry` records with the type of the entry.
const TYPE_FIELD: &str = "type";

/// Stage of `read`, with the cache of texts used when the engine registers
/// no shared [`ReadCache`].
#[derive(Debug, Default)]
pub(crate) struct Read {
    cache: MemoryReadCache,
}

/// Stage implementations of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Read::default())]
}

impl StageImpl for Read {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

/// One file to read and its selector.
#[derive(Debug, Clone)]
struct Target {
    path: AbsPath,
    selector: Selector,
}

/// Cache used by one execution.
enum CacheRef<'a> {
    Shared(Arc<dyn ReadCache>),
    Stage(&'a MemoryReadCache),
}

impl CacheRef<'_> {
    fn get(&self) -> &dyn ReadCache {
        match self {
            Self::Shared(cache) => cache.as_ref(),
            Self::Stage(cache) => *cache,
        }
    }
}

type Targets<'a> = Box<dyn Iterator<Item = Result<Target>> + Send + 'a>;

impl ReadStage for Read {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let options = read_options(args, ctx)?;
        let targets: Targets<'a> = if input.is_none() {
            Box::new(source_targets(args, ctx, options.view)?.into_iter().map(Ok))
        } else {
            let selector = Selector::from_args(args)?;
            check_view(options.view, &selector)?;
            let records = input.into_records()?;
            Box::new(
                records.filter_map(move |record| entry_target(record, &selector, ctx).transpose()),
            )
        };
        let cache = ctx
            .extensions()
            .get::<dyn ReadCache>()
            .map_or(CacheRef::Stage(&self.cache), CacheRef::Shared);
        let stream = targets.flat_map(move |target| {
            match target.and_then(|target| read_target(ctx, cache.get(), &target, args, &options)) {
                Ok(records) => records.into_iter().map(Ok).collect::<Vec<_>>(),
                Err(error) => vec![Err(error)],
            }
        });
        Ok(StageOutput::records(RecordStream::new(stream).checked(ctx)))
    }
}

/// Records of one target: text through [`text::read_source`], content that
/// is not text and `view: hex` through [`binary::read_binary`].
fn read_target(
    ctx: &ExecCtx,
    cache: &dyn ReadCache,
    target: &Target,
    args: &BoundArgs,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    if options.view == View::Hex {
        return binary::read_binary(ctx, args, &target.path, &target.selector, options);
    }
    match text::load(ctx, &target.path, options.encoding)? {
        Loaded::Text(source) => text::read_source(ctx, cache, &source, &target.selector, options),
        Loaded::Binary => binary::read_binary(ctx, args, &target.path, &target.selector, options),
    }
}

fn read_options(args: &BoundArgs, ctx: &ExecCtx) -> Result<ReadOptions> {
    let depth = non_negative(args, DEPTH)?.map(|depth| u32::try_from(depth).unwrap_or(u32::MAX));
    Ok(ReadOptions {
        view: view_param(args)?,
        context: non_negative(args, CONTEXT)?.unwrap_or(DEFAULT_CONTEXT),
        depth,
        encoding: encoding_param(args)?.unwrap_or(EncodingParam::Auto),
        line_numbers: args.bool(LINE_NUMBERS)?.unwrap_or(false),
        max_line: max_line_param(args)?,
        unchanged_since: args
            .text(UNCHANGED_SINCE)?
            .map(TagMatch::parse)
            .transpose()?,
        since: args.text(SINCE)?.map(TagMatch::parse).transpose()?,
        known: known_param(args, ctx)?,
    })
}

fn view_param(args: &BoundArgs) -> Result<View> {
    match args.value(VIEW) {
        None | Some(Value::Null) => Ok(View::Full),
        Some(Value::Enum(name) | Value::Text(name)) => View::parse(name).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownValue,
                format!(
                    "`{name}` is not a value of `{VIEW}`: use one of {}",
                    View::NAMES.join(", ")
                ),
            )
            .with_found(name.as_str())
            .with_expected(serde_json::json!(View::NAMES))
        }),
        Some(_) => Err(type_error(args, VIEW, "a view name")),
    }
}

fn non_negative(args: &BoundArgs, name: &str) -> Result<Option<u64>> {
    match args.int(name)? {
        None => Ok(None),
        Some(number) => u64::try_from(number).map(Some).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{name}` cannot be negative, not {number}"),
            )
        }),
    }
}

fn max_line_param(args: &BoundArgs) -> Result<Option<usize>> {
    match args.int(MAX_LINE)? {
        None => Ok(None),
        Some(number) if number >= 1 => Ok(Some(usize::try_from(number).unwrap_or(usize::MAX))),
        Some(number) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{MAX_LINE}` must be at least 1, not {number}"),
        )),
    }
}

fn known_param(args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<(AbsPath, TagMatch)>> {
    match args.value(KNOWN) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Map(entries)) => entries
            .iter()
            .map(|(path, tag)| match tag {
                Value::Text(tag) => Ok((resolve(ctx, path)?, TagMatch::parse(tag)?)),
                _ => Err(type_error(args, KNOWN, "a map from path to etag")),
            })
            .collect(),
        Some(_) => Err(type_error(args, KNOWN, "a map from path to etag")),
    }
}

fn type_error(args: &BoundArgs, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{}` expects {expected} for `{name}`", args.command()),
    )
}

/// Targets of the source form: the paths with the selector of the call,
/// then the maps of `targets` with their own selectors.
fn source_targets(args: &BoundArgs, ctx: &ExecCtx, view: View) -> Result<Vec<Target>> {
    let call = Selector::from_args(args)?;
    let mut targets: Vec<Target> = path_list_arg(args, PATH, ctx)?
        .into_iter()
        .map(|path| Target {
            path,
            selector: call.clone(),
        })
        .collect();
    match args.value(TARGETS) {
        None | Some(Value::Null) => {}
        Some(Value::List(items)) => {
            if !matches!(call, Selector::Whole) {
                return Err(Error::new(
                    ErrorCode::ParamConflict,
                    format!(
                        "`{COMMAND}` takes selectors either for the call or in `{TARGETS}`: move the selector into each target"
                    ),
                ));
            }
            for item in items {
                targets.push(map_target(args, item, ctx)?);
            }
        }
        Some(_) => return Err(type_error(args, TARGETS, "a list of maps")),
    }
    if targets.is_empty() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            format!("`{COMMAND}` needs a `{PATH}` or `{TARGETS}`"),
        ));
    }
    for target in &targets {
        check_view(view, &target.selector)?;
    }
    Ok(targets)
}

fn map_target(args: &BoundArgs, item: &Value, ctx: &ExecCtx) -> Result<Target> {
    let Value::Map(fields) = item else {
        return Err(type_error(args, TARGETS, "a list of maps"));
    };
    let (path, selector) = Selector::from_target(args.command(), fields)?;
    let path = match path {
        Value::Path(text) | Value::Text(text) => resolve(ctx, text)?,
        _ => {
            return Err(type_error(
                args,
                TARGETS,
                "a path for the `path` of each target",
            ));
        }
    };
    Ok(Target { path, selector })
}

/// Target of an input record; `None` for a directory or another entry that
/// is not a file.
fn entry_target(
    record: Result<Record>,
    selector: &Selector,
    ctx: &ExecCtx,
) -> Result<Option<Target>> {
    let record = record?;
    let skipped = [EntryType::Directory.as_str(), EntryType::Other.as_str()];
    if let Some(Value::Enum(kind) | Value::Text(kind)) = record.get(TYPE_FIELD)
        && skipped.contains(&kind.as_str())
    {
        return Ok(None);
    }
    Ok(Some(Target {
        path: record_path(&record, ctx)?,
        selector: selector.clone(),
    }))
}

fn check_view(view: View, selector: &Selector) -> Result<()> {
    if view.combines_with(selector) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::ParamConflict,
            format!(
                "`{VIEW}: {}` describes the whole file and takes no selector: remove the selector or use `{VIEW}: outline_first`",
                view.as_str()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::fs::ContentHash;
    use nurt_contracts::record::Content;
    use nurt_contracts::stage::{ArgValue, Extensions, StageData};
    use nurt_contracts::value::{Fields, Range, Regex};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;

    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new();
        let path = abs(path);
        test.fs().write_file(&path, content).unwrap();
        (test, path)
    }

    fn value_args(pairs: Vec<(&str, Value)>) -> BoundArgs {
        pairs
            .into_iter()
            .fold(BoundArgs::new(COMMAND), |args, (name, value)| {
                args.with(name, ArgValue::Value(value), false)
            })
    }

    fn path_value(path: &AbsPath) -> Value {
        Value::Path(path.to_language_string())
    }

    fn run_with(
        stage: &Read,
        ctx: &ExecCtx,
        args: &BoundArgs,
        input: StageInput<'_>,
    ) -> Result<Vec<Record>> {
        let output = stage.execute(args, input, ctx)?;
        match output.data {
            StageData::Records(stream) => stream.collect(),
            _ => panic!("records expected"),
        }
    }

    fn run(ctx: &ExecCtx, args: &BoundArgs) -> Result<Vec<Record>> {
        run_with(&Read::default(), ctx, args, StageInput::none())
    }

    fn contents(records: Vec<Record>) -> Vec<Content> {
        records
            .into_iter()
            .map(|record| Content::try_from(record).unwrap())
            .collect()
    }

    fn files(test: &TestContext, list: &[(&str, &str)]) -> Vec<AbsPath> {
        list.iter()
            .map(|(path, content)| {
                let path = abs(path);
                test.fs().write_file(&path, content.as_bytes()).unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn descriptor_names_the_source_and_entry_forms() {
        let descriptor = Read::default().descriptor();
        assert_eq!(descriptor.command, "read");
        assert!(descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Entry));
        assert!(!descriptor.input.accepts(RecordKind::Content));
        assert_eq!(stages().len(), 1);
    }

    #[test]
    fn several_paths_are_read_in_order_with_the_call_selector() {
        let test = TestContext::new();
        let paths = files(&test, &[("/p/b.txt", "b1\nb2\n"), ("/p/a.txt", "a1\na2\n")]);
        let args = value_args(vec![
            (PATH, Value::List(paths.iter().map(path_value).collect())),
            (select::LAST, Value::Int(1)),
        ]);
        let read = contents(run(&test.ctx(), &args).unwrap());
        let texts: Vec<&str> = read.iter().map(|content| content.text.as_str()).collect();
        assert_eq!(texts, ["b2\n", "a2\n"]);
        assert_eq!(read[0].path.as_str(), paths[0].to_language_string());
    }

    #[test]
    fn targets_carry_their_own_selectors() {
        let test = TestContext::new();
        let paths = files(
            &test,
            &[("/p/a.txt", "1\n2\n3\n"), ("/p/b.md", "# A\nx\n# B\ny\n")],
        );
        let target = |path: &AbsPath, name: &str, value: Value| {
            let mut fields = Fields::new();
            fields.insert(PATH.to_owned(), path_value(path));
            fields.insert(name.to_owned(), value);
            Value::Map(fields)
        };
        let args = value_args(vec![(
            TARGETS,
            Value::List(vec![
                target(
                    &paths[0],
                    select::LINES,
                    Value::Range(Range::new(Some(2), Some(3)).unwrap()),
                ),
                target(&paths[1], select::SECTION, Value::Text("B".to_owned())),
            ]),
        )]);
        let read = contents(run(&test.ctx(), &args).unwrap());
        assert_eq!(read[0].text, "2\n3\n");
        assert_eq!(read[1].text, "# B\ny\n");
    }

    #[test]
    fn selectors_of_the_call_and_of_targets_conflict() {
        let (test, path) = with_file("/p/a.txt", "x\n".as_bytes());
        let mut fields = Fields::new();
        fields.insert(PATH.to_owned(), path_value(&path));
        let args = value_args(vec![
            (TARGETS, Value::List(vec![Value::Map(fields)])),
            (select::LAST, Value::Int(1)),
        ]);
        assert_eq!(
            run(&test.ctx(), &args).unwrap_err().code(),
            ErrorCode::ParamConflict
        );
    }

    #[test]
    fn a_call_without_targets_is_rejected() {
        let test = TestContext::new();
        let error = run(&test.ctx(), &BoundArgs::new(COMMAND)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }

    #[test]
    fn whole_file_views_reject_selectors() {
        let (test, path) = with_file("/p/a.txt", b"x\n");
        let args = value_args(vec![
            (PATH, path_value(&path)),
            (VIEW, Value::Enum("header".to_owned())),
            (select::LINES, Value::Int(1)),
        ]);
        let error = run(&test.ctx(), &args).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
        let unknown = value_args(vec![
            (PATH, path_value(&path)),
            (VIEW, Value::Enum("tree".to_owned())),
        ]);
        assert_eq!(
            run(&test.ctx(), &unknown).unwrap_err().code(),
            ErrorCode::UnknownValue
        );
    }

    #[test]
    fn entries_are_read_and_directories_skipped() {
        let test = TestContext::new();
        let paths = files(&test, &[("/p/a.txt", "a\n"), ("/p/sub/b.txt", "b\n")]);
        let entry = |path: &AbsPath, kind: EntryType| {
            Record::new(RecordKind::Entry)
                .with(PATH, path_value(path))
                .with(TYPE_FIELD, Value::Enum(kind.as_str().to_owned()))
        };
        let input = vec![
            entry(&paths[0], EntryType::File),
            entry(&abs("/p/sub"), EntryType::Directory),
            entry(&paths[1], EntryType::File),
        ];
        let args = value_args(vec![(select::LINES, Value::Int(1))]);
        let ctx = test.ctx();
        let stream = RecordStream::from_records(input);
        let read =
            contents(run_with(&Read::default(), &ctx, &args, StageInput::records(stream)).unwrap());
        let texts: Vec<&str> = read.iter().map(|content| content.text.as_str()).collect();
        assert_eq!(texts, ["a\n", "b\n"]);
    }

    #[test]
    fn known_files_are_left_out() {
        let test = TestContext::new();
        let paths = files(&test, &[("/p/a.txt", "a\n"), ("/p/b.txt", "b\n")]);
        let all = value_args(vec![(
            PATH,
            Value::List(paths.iter().map(path_value).collect()),
        )]);
        let first = contents(run(&test.ctx(), &all).unwrap());
        let mut known = Fields::new();
        known.insert(
            paths[0].to_language_string(),
            Value::Text(first[0].etag.clone()),
        );
        let args = value_args(vec![
            (PATH, Value::List(paths.iter().map(path_value).collect())),
            (KNOWN, Value::Map(known)),
        ]);
        let read = contents(run(&test.ctx(), &args).unwrap());
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].text, "b\n");
    }

    #[test]
    fn since_uses_the_cache_of_the_stage() {
        let (test, path) = with_file("/p/a.txt", b"one\ntwo\n");
        let ctx = test.ctx();
        let stage = Read::default();
        let plain = value_args(vec![(PATH, path_value(&path))]);
        let first = contents(run_with(&stage, &ctx, &plain, StageInput::none()).unwrap());
        test.fs().write_file(&path, b"one\nTWO\n").unwrap();
        let since = value_args(vec![
            (PATH, path_value(&path)),
            (SINCE, Value::Text(first[0].etag.clone())),
        ]);
        let diff = contents(run_with(&stage, &ctx, &since, StageInput::none()).unwrap());
        assert_eq!(diff[0].diff.as_ref().map(Vec::len), Some(1));
        let fresh = contents(run_with(&Read::default(), &ctx, &since, StageInput::none()).unwrap());
        assert_eq!(fresh[0].since_missing, Some(true));
        assert_eq!(fresh[0].text, "one\nTWO\n");
    }

    #[test]
    fn a_cache_registered_by_the_engine_is_shared() {
        let (test, path) = with_file("/p/a.txt", b"one\n");
        let shared: Arc<dyn ReadCache> = Arc::new(MemoryReadCache::default());
        let hash = ContentHash([7; 32]);
        shared.put(&path, &hash, "zero\n", Duration::ZERO);
        let mut extensions = Extensions::new();
        extensions.insert::<dyn ReadCache>(shared);
        let ctx = test.builder().extensions(extensions).build();
        let args = value_args(vec![
            (PATH, path_value(&path)),
            (SINCE, Value::Text(hash.to_string())),
        ]);
        let read = contents(run(&ctx, &args).unwrap());
        let hunks = read[0].diff.clone().unwrap();
        assert_eq!(hunks[0].lines, ["-zero", "+one"]);
    }

    #[test]
    fn around_with_context_and_line_numbers() {
        let text: String = (1..=30)
            .map(|line| {
                if line == 12 {
                    "target\n".to_owned()
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        let (test, path) = with_file("/p/a.txt", text.as_bytes());
        let args = value_args(vec![
            (PATH, path_value(&path)),
            (
                select::AROUND,
                Value::Regex(Regex::new("^tar", "").unwrap()),
            ),
            (CONTEXT, Value::Int(1)),
            (LINE_NUMBERS, Value::Bool(true)),
        ]);
        let read = contents(run(&test.ctx(), &args).unwrap());
        assert_eq!(read[0].text, "11  11\n12  target\n13  13\n");
        assert_eq!(read[0].range, Some(Range::new(Some(11), Some(13)).unwrap()));
    }

    #[test]
    fn missing_files_and_unknown_binary_content_fail_and_hex_dumps_them() {
        let (test, path) = with_file("/p/a.bin", b"\x00\x01\x02\x03\x00\x00");
        let ctx = test.ctx();
        let read = |path: &AbsPath, extra: Vec<(&str, Value)>| {
            let mut pairs = vec![(PATH, path_value(path))];
            pairs.extend(extra);
            run(&ctx, &value_args(pairs))
        };
        assert_eq!(
            read(&abs("/p/none.txt"), Vec::new()).unwrap_err().code(),
            ErrorCode::NotFound
        );
        let binary = read(&path, Vec::new()).unwrap_err();
        assert_eq!(binary.code(), ErrorCode::Binary);
        assert_eq!(binary.fix.len(), 1);
        assert!(binary.fix[0].ends_with("view: hex"));
        let hex = contents(read(&path, vec![(VIEW, Value::Enum("hex".to_owned()))]).unwrap());
        assert_eq!(
            hex[0].text,
            format!("00000000: 0001 0203 0000{}......\n", " ".repeat(27))
        );
    }

    #[test]
    fn parameter_values_are_checked() {
        let (test, path) = with_file("/p/a.txt", b"x\n");
        let ctx = test.ctx();
        let error = |name: &str, value: Value| {
            run(
                &ctx,
                &value_args(vec![(PATH, path_value(&path)), (name, value)]),
            )
            .unwrap_err()
            .code()
        };
        assert_eq!(error(CONTEXT, Value::Int(-1)), ErrorCode::InvalidValue);
        assert_eq!(error(MAX_LINE, Value::Int(0)), ErrorCode::InvalidValue);
        assert_eq!(
            error(UNCHANGED_SINCE, Value::Text("nope".to_owned())),
            ErrorCode::InvalidValue
        );
        assert_eq!(error(KNOWN, Value::Text("x".to_owned())), ErrorCode::Type);
        assert_eq!(error(TARGETS, Value::Int(1)), ErrorCode::Type);
        assert_eq!(error(VIEW, Value::Int(1)), ErrorCode::Type);
    }
}
