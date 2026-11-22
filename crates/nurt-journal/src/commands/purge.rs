// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage of the `journal.purge` command: releases journal entries and the
//! space they hold at once.

use nurt_contracts::error::Result;
use nurt_contracts::journal::{MutationTx, PurgeRequest, PurgeSelector};
use nurt_contracts::record::EffectStatus;
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, StageDescriptor,
    StageImpl, StageInput, StaticInfo,
};
use nurt_contracts::value::{Fields, Size, Value};

use super::{Params, records};

const NAME: &str = "journal.purge";

/// Value of `volume` that selects every volume.
const ALL_VOLUMES: &str = "all";

/// Arguments of `journal.purge`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PurgeArgs {
    /// Path on the volume; `None` selects every volume.
    volume: Option<String>,
    selector: PurgeSelector,
}

/// Reads the arguments of `journal.purge`.
pub(super) fn parse(args: &BoundArgs) -> Result<PurgeArgs> {
    let params = Params::new(args);
    let volume = match params.value("volume")? {
        None => None,
        Some(Value::Enum(word)) if word == ALL_VOLUMES => None,
        Some(Value::Path(path) | Value::Text(path)) => Some(path.clone()),
        Some(_) => return Err(params.type_error("volume", "path or `all`")),
    };
    let up_to = params.op("up_to")?;
    let ids = params.ops("id")?;
    let selector = match (up_to, ids) {
        (Some(_), Some(_)) => {
            return Err(params.invalid("give either `up_to` or `id`, not both"));
        }
        (Some(op), None) => PurgeSelector::UpTo(op),
        (None, Some(ids)) if ids.is_empty() => {
            return Err(
                params.invalid("`id` lists no entry; give at least one number or remove `id`")
            );
        }
        (None, Some(ids)) => PurgeSelector::Ids(ids),
        (None, None) => PurgeSelector::All,
    };
    Ok(PurgeArgs { volume, selector })
}

fn purged() -> IrreversibleEntry {
    IrreversibleEntry {
        reason: Irreversible::Purged,
        path: None,
        count: None,
        detail: None,
    }
}

/// The `journal.purge` command.
pub(super) struct Purge;

impl StageImpl for Purge {
    fn descriptor(&self) -> StageDescriptor {
        super::source(NAME)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Ok(super::static_info(
            parse(args)?.volume.into_iter().collect(),
        ))
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Purge {
    /// Plans the release; the plan states that the effect is irreversible.
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let parsed = parse(args)?;
        super::resolve_opt(ctx, parsed.volume.as_deref())?;
        let mut plan = EffectPlan::new();
        plan.irreversible.push(purged());
        plan.preview.push(records::planned(NAME));
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
        let parsed = parse(args)?;
        let result = super::store(ctx)?.purge(&PurgeRequest {
            volume: super::resolve_opt(ctx, parsed.volume.as_deref())?,
            selector: parsed.selector,
        })?;
        let mut after = Fields::new();
        after.insert("released".to_owned(), records::op_list(&result.released));
        after.insert(
            "bytes".to_owned(),
            Value::Size(Size::from_bytes(result.bytes)),
        );
        let summary = [("released".to_owned(), records::count(result.released.len()))]
            .into_iter()
            .collect();
        let effect = nurt_contracts::record::Effect {
            targets: Some(records::count(result.released.len())),
            summary: Some(summary),
            after: Some(Value::Map(after)),
            ..records::effect(NAME, EffectStatus::Ok)
        };
        let irreversible = records::marker(Irreversible::Purged, &result.released)
            .into_iter()
            .collect();
        Ok(records::outcome(effect, Vec::new(), irreversible))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::journal::OpId;
    use nurt_contracts::value::Value;

    use super::super::testing::{Env, apply, call, plan};
    use super::*;

    #[test]
    fn releases_the_selected_entries() {
        let env = Env::new();
        env.three_operations();
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[("id", Value::List(vec![Value::Int(2)]))]);
        let planned = plan(&Purge, &args, &ctx).unwrap();
        assert_eq!(planned.irreversible[0].reason, Irreversible::Purged);

        let done = apply(&env, &Purge, &args, &ctx).unwrap();
        let after = done.records[0].get("after").unwrap();
        assert_eq!(
            after.get("released"),
            Some(&Value::List(vec![Value::Int(2)]))
        );
        assert_eq!(done.irreversible[0].reason, Irreversible::Purged);
        assert_eq!(done.irreversible[0].count, Some(1));
        assert_eq!(env.available(), 2);
    }

    #[test]
    fn an_unknown_volume_is_not_found() {
        let env = Env::new();
        env.three_operations();
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[("volume", Value::Path("/srv".to_owned()))]);
        let error = apply(&env, &Purge, &args, &ctx).err().unwrap();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(env.available(), 3);
    }

    #[test]
    fn selects_one_form_of_release() {
        let all = parse(&call(NAME, &[])).unwrap();
        assert_eq!((all.volume, all.selector), (None, PurgeSelector::All));
        let up_to = parse(&call(NAME, &[("up_to", Value::Int(4))])).unwrap();
        assert_eq!(up_to.selector, PurgeSelector::UpTo(OpId(4)));
        let both = call(
            NAME,
            &[
                ("up_to", Value::Int(4)),
                ("id", Value::List(vec![Value::Int(2)])),
            ],
        );
        assert_eq!(parse(&both).unwrap_err().code, ErrorCode::InvalidValue);
    }
}
