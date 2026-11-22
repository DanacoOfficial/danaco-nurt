// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage of the `redo` command: applies again the changes reversed by an
//! undo entry, as a new journal entry.

use nurt_contracts::error::Result;
use nurt_contracts::journal::{MutationTx, OpId, RedoRequest};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, StageDescriptor,
    StageImpl, StageInput, StaticInfo,
};

use super::{Params, records};

const NAME: &str = "redo";

/// Arguments of `redo`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RedoArgs {
    /// Undo entry; `None` selects the latest undo of the session.
    undo: Option<OpId>,
}

/// Reads the arguments of `redo`.
pub(super) fn parse(args: &BoundArgs) -> Result<RedoArgs> {
    Ok(RedoArgs {
        undo: Params::new(args).op("id")?,
    })
}

fn request(parsed: &RedoArgs, ctx: &ExecCtx) -> Result<RedoRequest> {
    Ok(RedoRequest {
        undo_entry: parsed.undo,
        session: super::session(ctx),
        program: super::program(ctx)?,
    })
}

/// The `redo` command.
pub(super) struct Redo;

impl StageImpl for Redo {
    fn descriptor(&self) -> StageDescriptor {
        super::source(NAME)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        parse(args)?;
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Redo {
    /// Plans the redo; with `id` the plan lists the items that the redo
    /// restores.
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let request = request(&parse(args)?, ctx)?;
        let mut plan = EffectPlan::new();
        if let Some(undo) = request.undo_entry {
            plan.changes = records::reversals(&super::store(ctx)?.details(undo)?, None);
            plan.bulk = plan.changes.len() > 1;
        }
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
        let result = super::store(ctx)?.redo(&request)?;
        Ok(records::undo_outcome(NAME, result))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::value::Value;

    use super::super::testing::{Env, apply, call, plan};
    use super::super::undo::Undo;
    use super::*;

    #[test]
    fn applies_the_latest_undo_again() {
        let env = Env::new();
        let notes = env.seed("notes.txt", b"draft");
        env.write(&notes, b"final");
        let ctx = env.ctx(NAME);
        apply(&env, &Undo, &call("undo", &[]), &ctx).unwrap();
        assert_eq!(env.read(&notes), b"draft");

        let planned = plan(&Redo, &call(NAME, &[("id", Value::Int(2))]), &ctx).unwrap();
        assert_eq!(planned.changes.len(), 1);
        let done = apply(&env, &Redo, &call(NAME, &[]), &ctx).unwrap();
        assert_eq!(env.read(&notes), b"final");
        let effect = &done.records[0];
        assert_eq!(effect.get("op"), Some(&Value::Int(3)));
        assert_eq!(effect.get("kind"), Some(&Value::Text("redo".to_owned())));
        let sources = effect.get("after").and_then(|after| after.get("sources"));
        assert_eq!(sources, Some(&Value::List(vec![Value::Int(2)])));
    }

    #[test]
    fn fails_when_nothing_was_undone() {
        let env = Env::new();
        let notes = env.seed("notes.txt", b"draft");
        env.write(&notes, b"final");
        let ctx = env.ctx(NAME);
        let error = apply(&env, &Redo, &call(NAME, &[]), &ctx).err().unwrap();
        assert_eq!(error.code, ErrorCode::JournalNoop);
        assert_eq!(env.read(&notes), b"final");
    }
}
