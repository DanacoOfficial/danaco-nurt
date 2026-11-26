// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `rename`: renames entries within their directory.
//!
//! The new name is given by `to`, or derived from the old name by replacing
//! the matches of `pattern` with `with`; a regular expression pattern takes
//! a replacement template with `$1`, `${name}` and `$$`. Only the last
//! component changes; `move` changes the directory. Renames whose new name
//! is the old name of another renamed entry run after that entry is renamed;
//! renames that form a cycle are rejected.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::ops::ControlFlow;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, FileMeta};
use nurt_contracts::journal::{Expect, FileOp, MutationTx};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds,
    StageDescriptor, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, RecordKind, Regex, Value};
use nurt_text::search::{Pattern, SearchOptions, Searcher};
use nurt_text::template::{CaptureSource, Groups, Template};
use serde_json::Value as Json;

use super::conflict::{
    CHECK_REFERENCES, CHECKPOINT_INTERVAL, Occupant, OnConflict, Placer, Source, Step, TO, child,
    directory_exists_error, effect_output, exists_error, flag, fold, is_case_variant,
    kind_mismatch_error, missing_state, on_conflict_param, references_left, run_steps, sources,
    type_error,
};
use crate::common::path::PATH_FIELD;

const COMMAND: &str = "rename";
const PATH: &str = "path";
const PATTERN: &str = "pattern";
const WITH: &str = "with";

/// Stage of `rename`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Rename;

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Rename)]
}

impl StageImpl for Rename {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// How the new name is computed.
enum Rule {
    /// `to`, evaluated per input record.
    To,
    /// Every occurrence of a text replaced by a text.
    Literal { pattern: String, with: String },
    /// Every match of a regular expression replaced by a template.
    Regex {
        searcher: Box<Searcher>,
        template: Template,
    },
}

impl Rule {
    fn read(args: &BoundArgs) -> Result<Self> {
        let to = matches!(
            args.get(TO),
            Some(ArgValue::Expr(_)) | Some(ArgValue::Value(Value::Text(_) | Value::Path(_)))
        );
        let with = args.text(WITH)?;
        let pattern = args
            .value(PATTERN)
            .filter(|value| !matches!(value, Value::Null));
        match (to, with, pattern) {
            (true, Some(_), _) => Err(Error::new(
                ErrorCode::ParamConflict,
                "`rename` takes either `to` or `with`, not both",
            )),
            (true, None, Some(_)) => Err(Error::new(
                ErrorCode::ParamConflict,
                "`pattern` goes with `with`: give `pattern` and `with`, or only `to`",
            )),
            (true, None, None) => Ok(Self::To),
            (false, Some(_), Some(Value::Text(pattern))) if pattern.is_empty() => Err(Error::new(
                ErrorCode::InvalidValue,
                "`pattern` cannot be empty: give the part of the name to replace",
            )),
            (false, Some(with), Some(Value::Text(pattern))) => Ok(Self::Literal {
                pattern: pattern.clone(),
                with: with.to_owned(),
            }),
            (false, Some(with), Some(Value::Regex(regex))) => Self::regex(regex, with),
            (false, Some(_), Some(_)) => Err(type_error(COMMAND, PATTERN, "a text or a regex")),
            (false, Some(_), None) => Err(Error::new(
                ErrorCode::MissingParam,
                "`with` needs `pattern`: give the part of the name to replace",
            )),
            (false, None, _) => Err(Error::new(
                ErrorCode::MissingParam,
                "`rename` needs the new name: give `to`, or `pattern` and `with`",
            )),
        }
    }

    fn regex(regex: &Regex, with: &str) -> Result<Self> {
        let pattern = Pattern::regex_with_flags(regex.pattern.clone(), regex.flags);
        let searcher = Searcher::new(&[pattern], SearchOptions::new())
            .map_err(|error| Error::new(error.code(), format!("`pattern`: {error}")))?;
        let names = capture_names(&regex.pattern, regex.flags.extended);
        let groups =
            Groups::from_names(std::iter::once(None).chain(names.iter().map(Option::as_deref)));
        let template = Template::parse(with, &groups)
            .map_err(|error| Error::new(error.code(), format!("`with`: {error}")))?;
        Ok(Self::Regex {
            searcher: Box::new(searcher),
            template,
        })
    }

    fn new_name(
        &self,
        name: &str,
        args: &BoundArgs,
        record: Option<&Record>,
        ctx: &ExecCtx,
    ) -> Result<String> {
        match self {
            Self::To => match args.eval(TO, record, ctx)? {
                Some(Value::Text(text) | Value::Path(text)) => Ok(text),
                _ => Err(type_error(COMMAND, TO, "a name")),
            },
            Self::Literal { pattern, with } => Ok(name.replace(pattern.as_str(), with)),
            Self::Regex { searcher, template } => Ok(replace_matches(searcher, template, name)),
        }
    }
}

/// `name` with every match of the searcher replaced by the expansion of the
/// template.
fn replace_matches(searcher: &Searcher, template: &Template, name: &str) -> String {
    let mut pieces: Vec<(usize, usize, String)> = Vec::new();
    searcher.search_slice(name.as_bytes(), |hit| {
        for found in hit.matches() {
            let start = usize::try_from(found.start).unwrap_or(usize::MAX);
            let end = usize::try_from(found.end).unwrap_or(usize::MAX);
            let mut expansion = String::new();
            if let Some(captures) = searcher.captures(hit, found) {
                template.expand_into(&GroupText(&captures), &mut expansion);
            }
            pieces.push((start, end, expansion));
        }
        ControlFlow::Continue(())
    });
    let mut result = String::with_capacity(name.len());
    let mut position = 0;
    for (start, end, expansion) in pieces {
        let (Some(before), true) = (name.get(position..start), end <= name.len()) else {
            continue;
        };
        result.push_str(before);
        result.push_str(&expansion);
        position = end;
    }
    result.push_str(name.get(position..).unwrap_or_default());
    result
}

/// Capture groups of the search engine as a template source.
struct GroupText<'a, 'b>(&'b nurt_text::search::Captures<'a>);

impl CaptureSource for GroupText<'_, '_> {
    fn group(&self, index: usize) -> Option<&str> {
        self.0
            .get(index)
            .and_then(|group| std::str::from_utf8(group.bytes).ok())
    }
}

/// Names of the capture groups of a regular expression, in group order;
/// `None` for an unnamed group. Escapes, character classes, non-capturing
/// groups and, in extended mode, comments are skipped.
fn capture_names(source: &str, extended: bool) -> Vec<Option<String>> {
    let mut names = Vec::new();
    let mut characters = source.char_indices().peekable();
    let mut class_depth = 0_usize;
    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => {
                characters.next();
            }
            '[' => class_depth += 1,
            ']' if class_depth > 0 => class_depth -= 1,
            '#' if extended && class_depth == 0 => {
                for (_, skipped) in characters.by_ref() {
                    if skipped == '\n' {
                        break;
                    }
                }
            }
            '(' if class_depth == 0 => {
                let rest = &source[index + 1..];
                if !rest.starts_with('?') {
                    names.push(None);
                } else if let Some(name) = group_name(rest) {
                    names.push(Some(name));
                }
            }
            _ => {}
        }
    }
    names
}

/// Name of a named group that starts with `rest`, the text after `(`.
fn group_name(rest: &str) -> Option<String> {
    let (body, close) = if let Some(body) = rest.strip_prefix("?P<") {
        (body, '>')
    } else if let Some(body) = rest.strip_prefix("?<") {
        if body.starts_with(['=', '!']) {
            return None;
        }
        (body, '>')
    } else {
        (rest.strip_prefix("?'")?, '\'')
    };
    body.split_once(close).map(|(name, _)| name.to_owned())
}

/// Checks that a new name names an entry of the same directory.
fn check_name(name: &str) -> Result<()> {
    let separator = name.contains(['/', '\\', '\0']);
    if name.is_empty() || name == "." || name == ".." || separator {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` is not a name: `rename` changes only the last component; use `move` to change the directory"),
        )
        .with_found(name));
    }
    Ok(())
}

/// Counters of the `effect` record.
#[derive(Debug, Clone, Copy, Default)]
struct Summary {
    renamed: u64,
    overwritten: u64,
    skipped: u64,
    unchanged: u64,
}

impl Summary {
    fn pairs(self) -> [(&'static str, u64); 4] {
        [
            ("renamed", self.renamed),
            ("overwritten", self.overwritten),
            ("skipped", self.skipped),
            ("unchanged", self.unchanged),
        ]
    }
}

/// Private state of the plan.
#[derive(Debug)]
struct RenameState {
    steps: Vec<Step>,
    summary: Summary,
}

/// One planned rename.
struct Item {
    source: AbsPath,
    meta: FileMeta,
    target: AbsPath,
    decision: Decision,
}

/// What happens to the destination of one rename.
#[derive(Debug, Clone)]
enum Decision {
    /// Rename; `true` replaces an existing entry.
    Ready(bool),
    /// The destination is the old name of another renamed entry, which is
    /// renamed first; when that entry stays, the occupant decides.
    After(usize, FileMeta),
    /// The destination is occupied; `on_conflict` decides.
    Occupied(FileMeta),
    /// Kept by `on_conflict: skip`.
    Skipped,
}

impl EffectStage for Rename {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let rule = Rule::read(args)?;
        let on_conflict = on_conflict_param(args, OnConflict::Error)?;
        let check_references = flag(args, CHECK_REFERENCES, false)?;
        let sources = sources(args, input, ctx, PATH)?;
        let mut planner = Planner {
            on_conflict,
            placer: Placer::new(fs),
            items: Vec::new(),
            summary: Summary::default(),
        };
        for (index, source) in sources.iter().enumerate() {
            if index % CHECKPOINT_INTERVAL == 0 {
                ctx.checkpoint()?;
            }
            let meta = source.require(fs)?;
            let name = source
                .path
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(|| unnamed_error(&source.path))?;
            let new = rule.new_name(name, args, source.record.as_ref(), ctx)?;
            if new == name {
                planner.summary.unchanged += 1;
                continue;
            }
            check_name(&new)?;
            planner.add(source, meta, &new)?;
        }
        planner.decide()?;
        let order = planner.order()?;
        let mut notable = Vec::new();
        if check_references {
            let old: Vec<AbsPath> = order
                .iter()
                .map(|index| planner.items[*index].source.clone())
                .collect();
            notable.extend(references_left(ctx, &old)?);
        }
        Ok(planner.finish(&order, notable))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let fs = ctx.require_fs()?;
        let state = plan
            .take_state::<RenameState>()
            .ok_or_else(|| missing_state(COMMAND))?;
        let changes = run_steps(tx, state.steps, fs)?;
        let output = effect_output(COMMAND, tx, &changes, &plan.notable, &state.summary.pairs());
        let mut outcome = EffectOutcome::new(output);
        outcome.changes = changes;
        Ok(outcome)
    }
}

fn unnamed_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{path}` has no name that is valid Unicode: rename it with `move` and an explicit destination"),
    )
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

struct Planner<'a> {
    on_conflict: OnConflict,
    placer: Placer<'a>,
    items: Vec<Item>,
    summary: Summary,
}

impl Planner<'_> {
    fn add(&mut self, source: &Source, meta: FileMeta, new: &str) -> Result<()> {
        let parent = source
            .path
            .parent()
            .ok_or_else(|| unnamed_error(&source.path))?;
        let target = child(&parent, OsStr::new(new))?;
        self.placer.claim(&target, &source.path)?;
        let occupant = self.placer.occupant(&meta, &target)?;
        let decision = match occupant {
            Occupant::Free => Decision::Ready(false),
            Occupant::Same(_) if is_case_variant(&source.path, &target) => Decision::Ready(false),
            Occupant::Same(existing)
            | Occupant::Directory(existing)
            | Occupant::Other(existing) => Decision::Occupied(existing),
        };
        self.items.push(Item {
            source: source.path.clone(),
            meta,
            target,
            decision,
        });
        Ok(())
    }

    /// Links renames whose destination is the old name of another renamed
    /// entry, and decides the remaining occupied destinations.
    fn decide(&mut self) -> Result<()> {
        let mut sources: HashMap<String, usize> = HashMap::new();
        for (index, item) in self.items.iter().enumerate() {
            let sensitive = self.placer.case_sensitive(&item.source)?;
            sources.insert(fold(&item.source, sensitive), index);
        }
        for index in 0..self.items.len() {
            let Decision::Occupied(existing) = self.items[index].decision.clone() else {
                continue;
            };
            let sensitive = self.placer.case_sensitive(&self.items[index].target)?;
            let key = fold(&self.items[index].target, sensitive);
            self.items[index].decision = match sources.get(&key) {
                Some(&other) if other != index => Decision::After(other, existing),
                _ => self.resolve(index, &existing)?,
            };
        }
        Ok(())
    }

    fn resolve(&mut self, index: usize, existing: &FileMeta) -> Result<Decision> {
        let item = &self.items[index];
        if existing.kind == FileKind::Directory {
            return Err(directory_exists_error(&item.target));
        }
        if item.meta.kind == FileKind::Directory {
            return Err(kind_mismatch_error(&item.target));
        }
        match self.on_conflict {
            OnConflict::Overwrite => {
                self.summary.overwritten += 1;
                Ok(Decision::Ready(true))
            }
            OnConflict::Skip => {
                self.summary.skipped += 1;
                Ok(Decision::Skipped)
            }
            OnConflict::Error => Err(exists_error(&item.target)),
        }
    }

    /// Order of the renames: every rename after the rename that frees its
    /// destination. Skipped renames are left out.
    fn order(&mut self) -> Result<Vec<usize>> {
        let mut state = vec![Visit::New; self.items.len()];
        let mut order = Vec::with_capacity(self.items.len());
        for index in 0..self.items.len() {
            self.visit(index, &mut state, &mut order)?;
        }
        Ok(order)
    }

    fn visit(&mut self, index: usize, state: &mut [Visit], order: &mut Vec<usize>) -> Result<()> {
        match state[index] {
            Visit::Done => return Ok(()),
            Visit::Active => return Err(cycle_error(&self.items[index].source)),
            Visit::New => state[index] = Visit::Active,
        }
        if let Decision::After(other, existing) = self.items[index].decision.clone() {
            self.visit(other, state, order)?;
            self.items[index].decision = if matches!(self.items[other].decision, Decision::Skipped)
            {
                self.resolve(index, &existing)?
            } else {
                Decision::Ready(false)
            };
        }
        state[index] = Visit::Done;
        if matches!(self.items[index].decision, Decision::Ready(_)) {
            order.push(index);
        }
        Ok(())
    }

    fn finish(self, order: &[usize], mut notable: Vec<NotableEntry>) -> EffectPlan {
        let (_, placed) = self.placer.finish();
        notable.splice(0..0, placed);
        let mut steps = Vec::with_capacity(order.len());
        let mut summary = self.summary;
        for index in order {
            let item = &self.items[*index];
            let Decision::Ready(replace) = item.decision else {
                continue;
            };
            let op = FileOp::rename(item.source.clone(), item.target.clone(), replace)
                .expect(Expect::Etag(item.meta.etag()))
                .size_hint(item.meta.size);
            steps.push(Step::op(op));
            summary.renamed += 1;
        }
        if steps.is_empty() && summary.skipped == 0 {
            notable.push(NotableEntry::new(Notable::NothingMatched));
        }
        let changes = steps.iter().map(Step::change).collect();
        let bulk = steps.len() > 1;
        let mut plan = EffectPlan::new().with_state(RenameState { steps, summary });
        plan.changes = changes;
        plan.notable = notable;
        plan.bulk = bulk;
        plan
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    Active,
    Done,
}

fn cycle_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!("the renames of `{path}` and other entries form a cycle: rename one of them to a free name first"),
    )
    .with_problem("rename_cycle")
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::Fs;
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::stage::StageImpl;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::VolumeConfig;

    use super::super::conflict::ON_CONFLICT;
    use super::super::conflict::testing::{
        abs, assert_journaled, assert_untouched, choice_arg, dir, entry, file, path_arg, run,
        text_arg,
    };
    use super::*;

    fn named(path: &AbsPath, to: &str) -> BoundArgs {
        BoundArgs::new(COMMAND)
            .with(PATH, path_arg(path), false)
            .with(TO, text_arg(to), false)
    }

    fn by_pattern(pattern: Value, with: &str) -> BoundArgs {
        BoundArgs::new(COMMAND)
            .with(PATTERN, ArgValue::Value(pattern), false)
            .with(WITH, text_arg(with), false)
    }

    fn regex(source: &str) -> Value {
        Value::Regex(Regex::new(source, "").unwrap())
    }

    #[test]
    fn renames_an_entry_to_a_new_name() {
        let test = TestContext::new();
        let source = file(&test, "/data/draft.txt", b"d");
        let run = run(&Rename, &test, &named(&source, "final.txt"), None).unwrap();
        assert!(test.fs().exists(&abs("/data/final.txt")));
        assert!(!test.fs().exists(&source));
        assert_eq!(run.changes[0].action, ChangeAction::Rename);
        assert_eq!(run.summary("renamed"), Some(1));
        assert_journaled(&test);
        assert!(Rename.descriptor().input.accepts(RecordKind::Entry));
    }

    #[test]
    fn a_new_name_with_a_directory_is_rejected() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        for name in ["sub/a.txt", "..", "", "a\\b"] {
            let error = run(&Rename, &test, &named(&source, name), None).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{name}");
        }
        assert_untouched(&test);
    }

    #[test]
    fn literal_patterns_replace_every_occurrence() {
        let test = TestContext::new();
        let records = vec![
            entry(&file(&test, "/img/IMG_001.jpeg", b"1")),
            entry(&file(&test, "/img/IMG_002.jpeg", b"2")),
            entry(&file(&test, "/img/notes.txt", b"n")),
        ];
        let call = by_pattern(Value::Text(".jpeg".to_owned()), ".jpg");
        let run = run(&Rename, &test, &call, Some(records)).unwrap();
        assert!(test.fs().exists(&abs("/img/IMG_001.jpg")));
        assert!(test.fs().exists(&abs("/img/IMG_002.jpg")));
        assert_eq!(run.summary("renamed"), Some(2));
        assert_eq!(run.summary("unchanged"), Some(1));
    }

    #[test]
    fn regex_patterns_expand_templates() {
        let test = TestContext::new();
        let records = vec![entry(&file(&test, "/log/app-2026-11-22.log", b"l"))];
        let call = by_pattern(regex(r"(?<y>\d{4})-(\d{2})-(\d{2})"), "$3.$2.${y}");
        run(&Rename, &test, &call, Some(records)).unwrap();
        assert!(test.fs().exists(&abs("/log/app-22.11.2026.log")));
        let invalid = by_pattern(regex(r"(\d+)"), "$2");
        let records = vec![entry(&abs("/log/app-22.11.2026.log"))];
        assert_eq!(
            run(&Rename, &test, &invalid, Some(records))
                .unwrap_err()
                .code(),
            ErrorCode::Regex
        );
    }

    #[test]
    fn conflicts_fail_by_default_and_follow_on_conflict() {
        let cases = [
            ("error", Err(ErrorCode::Exists)),
            ("skip", Ok(b"old".as_slice())),
            ("overwrite", Ok(b"new".as_slice())),
        ];
        for (value, expected) in cases {
            let test = TestContext::new();
            let source = file(&test, "/data/a.txt", b"new");
            let target = file(&test, "/data/b.txt", b"old");
            let call = named(&source, "b.txt").with(ON_CONFLICT, choice_arg(value), false);
            match (run(&Rename, &test, &call, None), expected) {
                (Ok(_), Ok(content)) => assert_eq!(test.fs().read_file(&target).unwrap(), content),
                (Err(error), Err(code)) => {
                    assert_eq!(error.code(), code);
                    assert_untouched(&test);
                }
                (result, expected) => panic!("{value}: {result:?} {expected:?}"),
            }
        }
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        file(&test, "/data/b.txt", b"b");
        let error = run(&Rename, &test, &named(&source, "b.txt"), None).unwrap_err();
        assert!(error.message.contains("on_conflict: overwrite"));
        dir(&test, "/data/d");
        let call = named(&source, "d").with(ON_CONFLICT, choice_arg("overwrite"), false);
        assert_eq!(
            run(&Rename, &test, &call, None).unwrap_err().code(),
            ErrorCode::Exists
        );
    }

    #[test]
    fn a_rename_onto_the_old_name_of_another_rename_waits_for_it() {
        let test = TestContext::new();
        let short = file(&test, "/c/a", b"short");
        let long = file(&test, "/c/aa", b"long");
        let call = by_pattern(Value::Text("a".to_owned()), "aa");
        let renamed = run(
            &Rename,
            &test,
            &call,
            Some(vec![entry(&short), entry(&long)]),
        )
        .unwrap();
        assert_eq!(test.fs().read_file(&abs("/c/aa")).unwrap(), b"short");
        assert_eq!(test.fs().read_file(&abs("/c/aaaa")).unwrap(), b"long");
        assert_eq!(renamed.changes[0].path.as_str(), long.to_language_string());
        assert_journaled(&test);
    }

    #[test]
    fn renames_that_form_a_cycle_are_rejected() {
        let test = TestContext::new();
        let first = file(&test, "/s/ab", b"1");
        let second = file(&test, "/s/ba", b"2");
        let swap = by_pattern(regex(r"^(.)(.)$"), "$2$1");
        let error = run(
            &Rename,
            &test,
            &swap,
            Some(vec![entry(&first), entry(&second)]),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(error.problem.as_deref(), Some("rename_cycle"));
        assert_eq!(test.fs().read_file(&first).unwrap(), b"1");
        assert_untouched(&test);
    }

    #[test]
    fn letter_case_changes_and_collisions_on_insensitive_volumes() {
        let test = TestContext::new();
        let mut config = VolumeConfig::default();
        config.features.case_sensitive = false;
        test.fs().add_volume(&abs("/ci"), config).unwrap();
        let source = file(&test, "/ci/readme.md", b"r");
        run(&Rename, &test, &named(&source, "README.md"), None).unwrap();
        let names: Vec<String> = test
            .fs()
            .read_dir(&abs("/ci"))
            .unwrap()
            .map(|item| item.unwrap().name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["README.md"]);
        assert_journaled(&test);

        let other = file(&test, "/ci/notes.md", b"n");
        let error = run(&Rename, &test, &named(&other, "readme.MD"), None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
    }

    #[test]
    fn a_rename_change_gives_the_previous_path_and_the_new_one() {
        let test = TestContext::new();
        let source = file(&test, "/data/draft.txt", b"d");
        let run = run(&Rename, &test, &named(&source, "final.txt"), None).unwrap();
        let change = &run.changes[0];
        assert_eq!(
            change.from.as_ref().map(|path| path.as_str().to_owned()),
            Some(source.to_language_string())
        );
        assert_eq!(
            change.to.as_ref().map(|path| path.as_str().to_owned()),
            Some(abs("/data/final.txt").to_language_string())
        );
    }

    #[test]
    fn names_that_do_not_change_are_counted_and_nothing_matches() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let run = run(&Rename, &test, &named(&source, "a.txt"), None).unwrap();
        assert_eq!(run.summary("unchanged"), Some(1));
        assert_eq!(run.summary("renamed"), Some(0));
        assert_eq!(run.codes(), [Notable::NothingMatched]);
        assert!(test.fs().exists(&source));
    }

    #[test]
    fn dot_and_zero_byte_are_not_names() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        for name in [".", "a\0b"] {
            let error = run(&Rename, &test, &named(&source, name), None).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{name:?}");
        }
        assert_untouched(&test);
    }

    #[test]
    fn multibyte_characters_are_replaced_literally() {
        let test = TestContext::new();
        let source = file(&test, "/data/caf\u{e9}.txt", b"c");
        let call =
            by_pattern(Value::Text("\u{e9}".to_owned()), "e").with(PATH, path_arg(&source), false);
        run(&Rename, &test, &call, None).unwrap();
        assert!(test.fs().exists(&abs("/data/cafe.txt")));
        assert!(!test.fs().exists(&source));
    }

    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_unicode_is_rejected_with_a_hint() {
        use std::os::unix::ffi::OsStrExt;

        let test = TestContext::new();
        let name = OsStr::from_bytes(b"data\xff.bin");
        let path = AbsPath::new(abs("/raw").as_path().join(name)).unwrap();
        test.fs().write_file(&path, b"x").unwrap();
        assert!(run(&Rename, &test, &named(&path, "plain.bin"), None).is_err());
        let hint = unnamed_error(&path);
        assert_eq!(hint.code(), ErrorCode::InvalidValue);
        assert!(hint.message.contains("`move`"));
        assert!(test.fs().exists(&path));
    }

    #[test]
    fn capture_names_follow_the_group_order() {
        assert_eq!(
            capture_names(r"(a)(?:b)(?<c>c)(?P<d>d)[(](?=e)\((f)", false),
            [None, Some("c".to_owned()), Some("d".to_owned()), None]
        );
        assert_eq!(capture_names("(a) # (b)\n(c)", true), [None, None]);
        assert_eq!(group_name("?<!x)"), None);
        assert_eq!(group_name("?'n'x)"), Some("n".to_owned()));
    }

    #[test]
    fn rule_needs_a_new_name_and_rejects_conflicting_forms() {
        let missing = BoundArgs::new(COMMAND);
        assert_eq!(
            Rule::read(&missing).err().map(|error| error.code()),
            Some(ErrorCode::MissingParam)
        );
        let with_only = BoundArgs::new(COMMAND).with(WITH, text_arg("x"), false);
        assert_eq!(
            Rule::read(&with_only).err().map(|error| error.code()),
            Some(ErrorCode::MissingParam)
        );
        let both = by_pattern(Value::Text("a".to_owned()), "b").with(TO, text_arg("c"), false);
        assert_eq!(
            Rule::read(&both).err().map(|error| error.code()),
            Some(ErrorCode::ParamConflict)
        );
        let empty = by_pattern(Value::Text(String::new()), "b");
        assert_eq!(
            Rule::read(&empty).err().map(|error| error.code()),
            Some(ErrorCode::InvalidValue)
        );
        let wrong = by_pattern(Value::Int(1), "b");
        assert_eq!(
            Rule::read(&wrong).err().map(|error| error.code()),
            Some(ErrorCode::Type)
        );
    }

    #[test]
    fn check_references_reports_the_old_names() {
        let test = TestContext::new();
        let source = file(&test, "/p/old_name.py", b"x = 1\n");
        file(&test, "/p/main.py", b"import old_name\n");
        let call = named(&source, "new_name.py").with(
            CHECK_REFERENCES,
            ArgValue::Value(Value::Bool(true)),
            false,
        );
        let run = run(&Rename, &test, &call, None).unwrap();
        assert_eq!(run.codes(), [Notable::ReferencesLeft]);
    }
}
