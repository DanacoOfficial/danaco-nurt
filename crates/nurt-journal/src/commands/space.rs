// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage of the `journal.space` command: free space, reclaimable journal
//! space and the thresholds of each volume, as reported by the space service
//! through the journal store.

use nurt_contracts::error::Result;
use nurt_contracts::journal::VolumeSpace;
use nurt_contracts::record::Volume;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, ReadStage, RecordStream, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::{Path, Record, Size};

use super::Params;

const NAME: &str = "journal.space";

/// Arguments of `journal.space`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SpaceArgs {
    /// Path on the volume; `None` selects every volume of the space service.
    volume: Option<String>,
}

/// Reads the arguments of `journal.space`.
pub(super) fn parse(args: &BoundArgs) -> Result<SpaceArgs> {
    Ok(SpaceArgs {
        volume: Params::new(args).path("volume")?,
    })
}

fn volume_record(space: &VolumeSpace) -> Volume {
    Volume {
        mount: Path::from(space.mount.to_language_string()),
        filesystem: space.filesystem.clone(),
        total: Size::from_bytes(space.total),
        free: Size::from_bytes(space.free),
        free_without_journal: Size::from_bytes(space.free_without_journal),
        reclaimable: Size::from_bytes(space.reclaimable),
        reserve: Size::from_bytes(space.reserve),
        soft: Size::from_bytes(space.soft),
        journal_bytes: Size::from_bytes(space.journal_bytes),
        journal: space.inactive.is_none(),
        journal_reason: space.inactive.map(|reason| reason.as_str().to_owned()),
    }
}

/// The `journal.space` command.
pub(super) struct Space;

impl StageImpl for Space {
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

impl ReadStage for Space {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let parsed = parse(args)?;
        let volume = super::resolve_opt(ctx, parsed.volume.as_deref())?;
        let volumes = super::store(ctx)?.space(volume.as_ref())?;
        let records = volumes
            .iter()
            .map(|space| Record::from(volume_record(space)))
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::journal::InactiveReason;
    use nurt_contracts::value::Value;

    use super::super::testing::{Env, call, read};
    use super::*;

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn reports_the_volumes_of_the_space_service() {
        let env = Env::with_space();
        let ctx = env.ctx(NAME);
        let listed = read(&Space, &call(NAME, &[]), &ctx).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed[0].get("mount"),
            Some(&Value::Path("/home".to_owned()))
        );
        assert_eq!(listed[0].get("journal"), Some(&Value::Bool(true)));
        assert_eq!(listed[1].get("journal"), Some(&Value::Bool(false)));
        assert_eq!(
            listed[1].get("journal_reason"),
            Some(&Value::Text(InactiveReason::NoAnchor.as_str().to_owned()))
        );

        let home = call(
            NAME,
            &[("volume", Value::Path("/home/user/docs".to_owned()))],
        );
        let listed = read(&Space, &home, &ctx).unwrap();
        assert_eq!(listed.len(), 1);
        let error = read(
            &Space,
            &call(NAME, &[("volume", Value::Path("/srv".to_owned()))]),
            &ctx,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn fails_while_the_space_service_is_not_running() {
        let env = Env::new();
        let ctx = env.ctx(NAME);
        let error = read(&Space, &call(NAME, &[]), &ctx).unwrap_err();
        assert_eq!(error.code, ErrorCode::ServiceNotRunning);
    }
}
