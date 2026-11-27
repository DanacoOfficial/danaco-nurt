// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage of the `journal.status` command: one `journal_state` record per
//! journal volume with the effective limits and their sources.

use nurt_contracts::config::{AgeLimit, ConfigLevel, JournalFloor, SizeLimit, StepLimit};
use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{EffectiveRetention, JournalState, JournalStore, Sourced};
use nurt_contracts::record::{JournalLimit, JournalState as StateRecord};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, ReadStage, RecordStream, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::{self, Instant, Path, Record, Size, Value};

use super::Params;

const NAME: &str = "journal.status";

/// Arguments of `journal.status`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StatusArgs {
    /// Path on the volume; `None` selects every journal volume.
    volume: Option<String>,
}

/// Reads the arguments of `journal.status`.
pub(super) fn parse(args: &BoundArgs) -> Result<StatusArgs> {
    Ok(StatusArgs {
        volume: Params::new(args).path("volume")?,
    })
}

/// `journal_state` records of the journal volumes, or of the volume that
/// contains `volume`.
pub(super) fn state_records(
    store: &dyn JournalStore,
    volume: Option<&AbsPath>,
) -> Result<Vec<Record>> {
    Ok(store
        .status(volume)?
        .iter()
        .map(|state| Record::from(state_record(state)))
        .collect())
}

fn state_record(state: &JournalState) -> StateRecord {
    StateRecord {
        active: state.inactive.is_none(),
        entries: state.entries_available,
        bytes: Size::from_bytes(state.bytes_held),
        limits: journal_limits(
            &state.limits,
            state.floor.as_ref().map(|floor| &floor.value),
        ),
        volume: Some(Path::from(state.volume.to_language_string())),
        oldest: state
            .oldest
            .as_ref()
            .map(|oldest| Instant::saturating_from_wall(oldest.time)),
        oldest_id: state.oldest.as_ref().map(|oldest| oldest.op.0),
        reason: state.inactive.map(|reason| reason.as_str().to_owned()),
        ..StateRecord::default()
    }
}

/// The step, age and size limits with the level that set each of them.
pub(super) fn journal_limits(
    limits: &EffectiveRetention,
    floor: Option<&JournalFloor>,
) -> Vec<JournalLimit> {
    vec![
        journal_limit(
            "steps",
            step_value(limits.max_ops.value),
            &limits.max_ops,
            floor.and_then(|floor| floor.max_ops).map(step_value),
        ),
        journal_limit(
            "age",
            age_value(limits.max_age.value),
            &limits.max_age,
            floor.and_then(|floor| floor.max_age).map(age_value),
        ),
        journal_limit(
            "size",
            size_value(limits.max_size.value),
            &limits.max_size,
            None,
        ),
    ]
}

fn journal_limit<T>(
    name: &str,
    value: Value,
    sourced: &Sourced<T>,
    floor: Option<Value>,
) -> JournalLimit {
    let source = if sourced.raised_to_floor {
        "floor"
    } else {
        match sourced.source {
            ConfigLevel::Builtin => "default",
            ConfigLevel::Runtime => "session",
            level => level.as_str(),
        }
    };
    JournalLimit {
        name: name.to_owned(),
        value,
        source: source.to_owned(),
        scope: (sourced.source == ConfigLevel::Runtime).then(|| "session".to_owned()),
        floor,
        expires: None,
    }
}

fn step_value(limit: StepLimit) -> Value {
    match limit {
        StepLimit::Limited(steps) => Value::Int(i64::from(steps)),
        unlimited @ StepLimit::Unlimited => Value::Enum(unlimited.to_string()),
    }
}

fn age_value(limit: AgeLimit) -> Value {
    match limit {
        AgeLimit::Limited(age) => Value::Duration(value::Duration::from_millis(
            i64::try_from(age.as_millis()).unwrap_or(i64::MAX),
        )),
        unlimited @ AgeLimit::Unlimited => Value::Enum(unlimited.to_string()),
    }
}

fn size_value(limit: SizeLimit) -> Value {
    match limit {
        SizeLimit::Bytes(size) => Value::Size(Size::from_bytes(size.bytes())),
        SizeLimit::Unlimited => Value::Enum(limit.to_string()),
        share => Value::Text(share.to_string()),
    }
}

/// The `journal.status` command.
pub(super) struct Status;

impl StageImpl for Status {
    fn descriptor(&self) -> StageDescriptor {
        super::source(NAME)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Ok(super::static_info(
            parse(args)?.volume.into_iter().collect(),
        ))
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Status {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let parsed = parse(args)?;
        let volume = super::resolve_opt(ctx, parsed.volume.as_deref())?;
        let records = state_records(super::store(ctx)?, volume.as_ref())?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::DurationSetting;
    use nurt_contracts::value::Value;

    use super::super::testing::{Env, call, read};
    use super::*;

    #[test]
    fn reports_the_journal_volume_with_its_limits() {
        let env = Env::new();
        env.three_operations();
        let ctx = env.ctx(NAME);
        let listed = read(&Status, &call(NAME, &[]), &ctx).unwrap();
        assert_eq!(listed.len(), 1);
        let state = &listed[0];
        assert_eq!(
            state.get("volume"),
            Some(&Value::Path(nurt_testkit::paths::abs("/home")))
        );
        assert_eq!(state.get("active"), Some(&Value::Bool(true)));
        assert_eq!(state.get("entries"), Some(&Value::Int(3)));
        assert_eq!(state.get("oldest_id"), Some(&Value::Int(1)));
        let limits = state.get("limits").and_then(Value::as_list).unwrap();
        let names: Vec<_> = limits.iter().map(|limit| limit.get("name")).collect();
        let expected = ["steps", "age", "size"].map(|name| Value::Text(name.to_owned()));
        assert_eq!(names, expected.iter().map(Some).collect::<Vec<_>>());
        assert_eq!(
            limits[0].get("source"),
            Some(&Value::Text("default".to_owned()))
        );
    }

    #[test]
    fn a_volume_without_entries_reports_an_empty_journal() {
        let env = Env::new();
        let ctx = env.ctx(NAME);
        let args = call(
            NAME,
            &[("volume", Value::Path(nurt_testkit::paths::abs("/srv")))],
        );
        let listed = read(&Status, &args, &ctx).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].get("volume"),
            Some(&Value::Path(nurt_testkit::paths::abs("/")))
        );
        assert_eq!(listed[0].get("active"), Some(&Value::Bool(true)));
        assert_eq!(listed[0].get("entries"), Some(&Value::Int(0)));
    }

    #[test]
    fn limit_values_are_shown_in_their_natural_types() {
        assert_eq!(step_value(StepLimit::Limited(30)), Value::Int(30));
        assert_eq!(
            size_value("2gib".parse().unwrap()),
            Value::Size(Size::from_bytes(2 << 30))
        );
        assert_eq!(
            size_value(SizeLimit::Unlimited),
            Value::Enum("unlimited".to_owned())
        );
        assert_eq!(
            size_value("50%free".parse().unwrap()),
            Value::Text("50%free".to_owned())
        );
        assert_eq!(
            age_value(AgeLimit::Unlimited),
            Value::Enum("unlimited".to_owned())
        );
        assert_eq!(
            age_value("1d".parse().unwrap()),
            Value::Duration(value::Duration::from_millis(86_400_000))
        );
    }

    #[test]
    fn only_session_limits_name_a_scope_and_a_raised_limit_names_the_floor() {
        let limit = |source: ConfigLevel, raised_to_floor: bool| {
            let sourced = Sourced {
                value: StepLimit::Limited(1),
                source,
                raised_to_floor,
            };
            journal_limit("steps", Value::Int(1), &sourced, None)
        };
        assert_eq!(limit(ConfigLevel::Builtin, false).source, "default");
        assert_eq!(limit(ConfigLevel::Builtin, false).scope, None);
        assert_eq!(
            limit(ConfigLevel::User, false).source,
            ConfigLevel::User.as_str()
        );
        assert_eq!(limit(ConfigLevel::User, false).scope, None);
        assert_eq!(
            limit(ConfigLevel::Runtime, false).scope.as_deref(),
            Some("session")
        );
        for level in [
            ConfigLevel::Builtin,
            ConfigLevel::System,
            ConfigLevel::Runtime,
        ] {
            assert_eq!(limit(level, true).source, "floor");
        }
    }

    #[test]
    fn limits_name_their_level_and_floor() {
        let runtime = Sourced {
            value: StepLimit::Limited(5),
            source: ConfigLevel::Runtime,
            raised_to_floor: false,
        };
        let limit = journal_limit(
            "steps",
            step_value(runtime.value),
            &runtime,
            Some(Value::Int(10)),
        );
        assert_eq!(limit.source, "session");
        assert_eq!(limit.scope.as_deref(), Some("session"));
        assert_eq!(limit.floor, Some(Value::Int(10)));
        let raised = Sourced {
            raised_to_floor: true,
            ..runtime
        };
        assert_eq!(
            journal_limit("steps", Value::Int(10), &raised, None).source,
            "floor"
        );
        assert_eq!(
            step_value(StepLimit::Unlimited),
            Value::Enum("unlimited".to_owned())
        );
        assert_eq!(
            age_value(AgeLimit::Limited(DurationSetting::from_secs(60))),
            Value::Duration(value::Duration::from_millis(60_000))
        );
        assert_eq!(
            size_value("10%".parse().unwrap()),
            Value::Text("10%".to_owned())
        );
    }
}
