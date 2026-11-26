// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage of the `undo` command: restores the state that preceded selected
//! journal entries, as a new journal entry.

use nurt_contracts::env::WallTime;
use nurt_contracts::error::Result;
use nurt_contracts::journal::{
    MutationTx, OnConflict, OpId, Since, TargetPath, UndoRequest, UndoSelector,
};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, StageDescriptor,
    StageImpl, StageInput, StaticInfo,
};
use nurt_contracts::value::{self, Value};

use super::{Params, records};

const NAME: &str = "undo";

const ON_CONFLICT: [(&str, OnConflict); 5] = [
    ("report", OnConflict::Report),
    ("force", OnConflict::Force),
    ("include_later", OnConflict::IncludeLater),
    ("partial", OnConflict::Partial),
    ("abort", OnConflict::Abort),
];

/// Lower bound of `undo file:`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum SinceArg {
    /// Entries with a greater number.
    Op(OpId),
    /// Entries committed at or after this time.
    Time(value::Instant),
}

/// Entries selected by the arguments of `undo`.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Selection {
    /// The latest available entry of the session.
    Last,
    /// One entry.
    Op(OpId),
    /// The entries newer than `after`.
    To { after: OpId, all_sessions: bool },
    /// The entries that touched one file.
    File {
        path: String,
        since: Option<SinceArg>,
    },
    /// Selected items of one entry.
    Paths { op: OpId, paths: Vec<String> },
}

/// Arguments of `undo`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UndoArgs {
    selection: Selection,
    on_conflict: OnConflict,
}

/// Reads the arguments of `undo`.
pub(super) fn parse(args: &BoundArgs) -> Result<UndoArgs> {
    let params = Params::new(args);
    let id = params.op("id")?;
    let to = params.op("to")?;
    let file = params.path("file")?;
    let since = match params.value("since")? {
        None => None,
        Some(Value::Int(number)) => Some(SinceArg::Op(params.op_number(*number)?)),
        Some(Value::Instant(time)) => Some(SinceArg::Time(*time)),
        Some(_) => return Err(params.type_error("since", "int or instant")),
    };
    let paths = params.paths("paths")?;
    let all_sessions = params.flag("all_sessions")?;
    let on_conflict = params
        .choice("on_conflict", &ON_CONFLICT)?
        .unwrap_or_default();

    let forms = [id.is_some(), to.is_some(), file.is_some()];
    if forms.iter().filter(|given| **given).count() > 1 {
        return Err(params.invalid("give only one of `id`, `to` and `file`"));
    }
    if since.is_some() && file.is_none() {
        return Err(
            params.invalid("`since` applies only with `file`; add `file` or remove `since`")
        );
    }
    if all_sessions && to.is_none() {
        return Err(params
            .invalid("`all_sessions` applies only with `to`; add `to` or remove `all_sessions`"));
    }
    if paths.as_ref().is_some_and(Vec::is_empty) {
        return Err(
            params.invalid("`paths` lists no path; give at least one path or remove `paths`")
        );
    }
    let selection = match (id, to, file, paths) {
        (Some(op), _, _, None) => Selection::Op(op),
        (Some(op), _, _, Some(paths)) => Selection::Paths { op, paths },
        (None, _, _, Some(_)) => {
            return Err(
                params.invalid("`paths` applies only with `id`; add the number of the operation")
            );
        }
        (None, Some(after), _, None) => Selection::To {
            after,
            all_sessions,
        },
        (None, None, Some(path), None) => Selection::File { path, since },
        (None, None, None, None) => Selection::Last,
    };
    Ok(UndoArgs {
        selection,
        on_conflict,
    })
}

fn request(parsed: &UndoArgs, ctx: &ExecCtx) -> Result<UndoRequest> {
    let selector = match &parsed.selection {
        Selection::Last => UndoSelector::Last,
        Selection::Op(op) => UndoSelector::Op(*op),
        Selection::To {
            after,
            all_sessions,
        } => UndoSelector::To {
            after: *after,
            all_sessions: *all_sessions,
        },
        Selection::File { path, since } => UndoSelector::File {
            path: super::resolve(ctx, path)?,
            since: since.as_ref().map(|since| match since {
                SinceArg::Op(op) => Since::Op(*op),
                SinceArg::Time(time) => {
                    Since::Time(WallTime::from_unix_nanos(i128::from(time.as_unix_nanos())))
                }
            }),
        },
        Selection::Paths { op, paths } => UndoSelector::Paths {
            op: *op,
            paths: paths
                .iter()
                .map(|path| super::resolve(ctx, path).map(TargetPath::Fs))
                .collect::<Result<Vec<TargetPath>>>()?,
        },
    };
    Ok(UndoRequest {
        selector,
        on_conflict: parsed.on_conflict,
        session: super::session(ctx),
        program: super::program(ctx)?,
    })
}

/// The `undo` command.
pub(super) struct Undo;

impl StageImpl for Undo {
    fn descriptor(&self) -> StageDescriptor {
        super::source(NAME)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let paths = match parse(args)?.selection {
            Selection::File { path, .. } => vec![path],
            Selection::Paths { paths, .. } => paths,
            Selection::Last | Selection::Op(_) | Selection::To { .. } => Vec::new(),
        };
        Ok(super::static_info(paths))
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Undo {
    /// Plans the undo; for one entry or selected items of an entry the plan
    /// lists the items that the undo restores.
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let request = request(&parse(args)?, ctx)?;
        let store = super::store(ctx)?;
        let mut plan = EffectPlan::new();
        match &request.selector {
            UndoSelector::Op(op) => plan.changes = records::reversals(&store.details(*op)?, None),
            UndoSelector::Paths { op, paths } => {
                plan.changes = records::reversals(&store.details(*op)?, Some(paths.as_slice()));
            }
            _ => {}
        }
        plan.bulk = plan.changes.len() > 1;
        if plan.changes.is_empty() {
            plan.preview.push(records::planned(NAME));
        }
        Ok(plan)
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        _plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        ctx.checkpoint()?;
        let request = request(&parse(args)?, ctx)?;
        let result = super::store(ctx)?.undo(&request)?;
        Ok(records::undo_outcome(NAME, result))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::value::Value;

    use super::super::testing::{Env, apply, bare_ctx, call, plan, sample_path};
    use super::*;

    #[test]
    fn undoes_one_operation_as_a_new_entry() {
        let env = Env::new();
        let notes = env.seed("notes.txt", b"draft");
        env.write(&notes, b"final");
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[("id", Value::Int(1))]);

        let planned = plan(&Undo, &args, &ctx).unwrap();
        assert_eq!(planned.changes.len(), 1);
        assert_eq!(planned.changes[0].action, ChangeAction::Modify);
        assert_eq!(env.read(&notes), b"final");

        let done = apply(&env, &Undo, &args, &ctx).unwrap();
        assert_eq!(env.read(&notes), b"draft");
        assert_eq!(done.changes.len(), 1);
        let effect = &done.records[0];
        assert_eq!(effect.get("op"), Some(&Value::Int(2)));
        assert_eq!(effect.get("kind"), Some(&Value::Text("undo".to_owned())));
        let after = effect.get("after").unwrap();
        assert_eq!(after.get("scope"), Some(&Value::Enum("op".to_owned())));
        assert_eq!(
            after.get("sources"),
            Some(&Value::List(vec![Value::Int(1)]))
        );
    }

    #[test]
    fn undoes_the_latest_operation_without_a_selection() {
        let env = Env::new();
        let notes = env.seed("notes.txt", b"draft");
        env.write(&notes, b"final");
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[]);
        let planned = plan(&Undo, &args, &ctx).unwrap();
        assert!(planned.changes.is_empty());
        assert_eq!(planned.preview.len(), 1);
        apply(&env, &Undo, &args, &ctx).unwrap();
        assert_eq!(env.read(&notes), b"draft");
    }

    #[test]
    fn an_unknown_operation_is_reported_before_anything_is_written() {
        let env = Env::new();
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[("id", Value::Int(7))]);
        let error = plan(&Undo, &args, &ctx).unwrap_err();
        assert_eq!(error.code, ErrorCode::JournalNoop);
        let error = apply(&env, &Undo, &args, &ctx).err().unwrap();
        assert_eq!(error.code, ErrorCode::JournalNoop);
    }

    #[test]
    fn needs_the_bound_program_of_the_request() {
        let ctx = bare_ctx(NAME);
        let error = request(&parse(&call(NAME, &[])).unwrap(), &ctx).unwrap_err();
        assert_eq!(error.code, ErrorCode::StageFailed);
        assert!(error.message.contains("bound program"));
    }

    fn instant() -> Value {
        Value::Instant(value::Instant::from_unix_nanos(1_700_000_000_000_000_000))
    }

    #[test]
    fn since_accepts_numbers_and_instants_only() {
        let given = |since: Value| call(NAME, &[("file", sample_path()), ("since", since)]);
        assert!(matches!(
            parse(&given(Value::Int(2))).unwrap().selection,
            Selection::File {
                since: Some(SinceArg::Op(OpId(2))),
                ..
            }
        ));
        assert!(matches!(
            parse(&given(instant())).unwrap().selection,
            Selection::File {
                since: Some(SinceArg::Time(_)),
                ..
            }
        ));
        for wrong in [Value::Text("2".to_owned()), Value::Bool(true)] {
            assert_eq!(parse(&given(wrong)).unwrap_err().code, ErrorCode::Type);
        }
        for number in [0, -1] {
            let error = parse(&given(Value::Int(number))).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidValue);
        }
    }

    #[test]
    fn every_conflict_word_selects_its_behaviour_and_unknown_words_are_refused() {
        for (word, expected) in ON_CONFLICT {
            let args = call(NAME, &[("on_conflict", Value::Enum(word.to_owned()))]);
            assert_eq!(parse(&args).unwrap().on_conflict, expected, "{word}");
        }
        assert_eq!(
            parse(&call(NAME, &[])).unwrap().on_conflict,
            OnConflict::default()
        );
        let unknown = call(NAME, &[("on_conflict", Value::Enum("ignore".to_owned()))]);
        assert_eq!(parse(&unknown).unwrap_err().code, ErrorCode::InvalidValue);
    }

    #[test]
    fn a_single_path_text_selects_one_item_of_the_entry() {
        let args = call(
            NAME,
            &[
                ("id", Value::Int(3)),
                ("paths", Value::Text("/home/użytkownik/zażółć.txt".to_owned())),
            ],
        );
        assert_eq!(
            parse(&args).unwrap().selection,
            Selection::Paths {
                op: OpId(3),
                paths: vec!["/home/użytkownik/zażółć.txt".to_owned()],
            }
        );
        let wrong = call(NAME, &[("id", Value::Int(3)), ("paths", Value::Int(1))]);
        assert_eq!(parse(&wrong).unwrap_err().code, ErrorCode::Type);
    }

    #[test]
    fn requests_resolve_relative_paths_and_convert_instants_to_wall_times() {
        use super::super::testing::path;

        let env = Env::new();
        let ctx = env.ctx_with(NAME, |options| options.base = Some("/home/user".to_owned()));
        let args = call(
            NAME,
            &[
                ("file", Value::Path("docs/notes.txt".to_owned())),
                ("since", instant()),
            ],
        );
        let made = request(&parse(&args).unwrap(), &ctx).unwrap();
        assert_eq!(
            made.selector,
            UndoSelector::File {
                path: path("/home/user/docs/notes.txt"),
                since: Some(Since::Time(WallTime::from_unix_nanos(
                    1_700_000_000_000_000_000
                ))),
            }
        );
        let args = call(
            NAME,
            &[
                ("id", Value::Int(3)),
                (
                    "paths",
                    Value::List(vec![
                        Value::Path("a.txt".to_owned()),
                        Value::Text("/srv/b.txt".to_owned()),
                    ]),
                ),
            ],
        );
        let made = request(&parse(&args).unwrap(), &ctx).unwrap();
        assert_eq!(
            made.selector,
            UndoSelector::Paths {
                op: OpId(3),
                paths: vec![
                    TargetPath::Fs(path("/home/user/a.txt")),
                    TargetPath::Fs(path("/srv/b.txt")),
                ],
            }
        );
        let without_base = env.ctx(NAME);
        let error = request(&parse(&args).unwrap(), &without_base)
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::InvalidValue);
    }

    #[test]
    fn selects_exactly_one_form() {
        let selection = |given: &[(&str, Value)]| parse(&call(NAME, given)).map(|a| a.selection);
        assert_eq!(selection(&[]).unwrap(), Selection::Last);
        assert_eq!(
            selection(&[("id", Value::Int(3)), ("paths", sample_path())]).unwrap(),
            Selection::Paths {
                op: OpId(3),
                paths: vec!["/home/user/docs/notes.txt".to_owned()],
            }
        );
        assert_eq!(
            selection(&[("file", sample_path()), ("since", Value::Int(2))]).unwrap(),
            Selection::File {
                path: "/home/user/docs/notes.txt".to_owned(),
                since: Some(SinceArg::Op(OpId(2))),
            }
        );
        let invalid = [
            vec![("id", Value::Int(1)), ("to", Value::Int(1))],
            vec![("since", Value::Int(2))],
            vec![("paths", sample_path())],
            vec![("all_sessions", Value::Bool(true))],
            vec![("id", Value::Int(1)), ("paths", Value::List(Vec::new()))],
        ];
        for given in invalid {
            let error = selection(&given).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidValue, "{given:?}");
        }
        let force = call(NAME, &[("on_conflict", Value::Enum("force".to_owned()))]);
        assert_eq!(parse(&force).unwrap().on_conflict, OnConflict::Force);
    }
}
