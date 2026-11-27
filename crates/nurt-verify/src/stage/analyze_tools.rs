// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::stage::analyze_tools
// Responsibility:   The `analyze.tools` command.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `analyze.tools` command.

use nurt_contracts::error::Result;
use nurt_contracts::record::{AnalyzeCheck, AnalyzeTool, FindingKind};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Path as RecordPath, Record};

use crate::analyze::execute::run_captured;
use crate::analyze::scope::resolve;
use crate::analyze::tools::{Candidate, Tool, candidates, resolve_program};
use crate::stage::analyze::{base_dir, bool_arg, list_arg};

/// Stage of `analyze.tools`.
pub(crate) struct Tools;

impl StageImpl for Tools {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "analyze.tools".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

fn check_of(tool: Tool) -> AnalyzeCheck {
    match tool.check() {
        FindingKind::Types => AnalyzeCheck::Types,
        FindingKind::Format => AnalyzeCheck::Format,
        _ => AnalyzeCheck::Lint,
    }
}

fn version_of(
    ctx: &ExecCtx,
    dir: &std::path::Path,
    tool: Tool,
    program: &std::path::Path,
) -> Option<String> {
    let mut command = vec![program.to_string_lossy().into_owned()];
    command.extend(tool.version_args()?);
    let captured = run_captured(ctx, dir, command, None).ok()?;
    let text = if captured.output.stdout.trim().is_empty() {
        captured.output.stderr
    } else {
        captured.output.stdout
    };
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

impl ReadStage for Tools {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let dir = base_dir(ctx)?;
        let scope = resolve(
            ctx,
            &dir,
            &list_arg(args, "path"),
            bool_arg(args, "changed_only", false),
        )?;
        let search = ctx.host().env_var("PATH");
        let found: Vec<(Candidate, Option<std::path::PathBuf>)> =
            candidates(&dir, &scope.extensions())
                .into_iter()
                .map(|candidate| {
                    let program = resolve_program(candidate.tool, &dir, search.as_deref());
                    (candidate, program)
                })
                .collect();
        let versions: Vec<Option<String>> = std::thread::scope(|threads| {
            let handles: Vec<_> = found
                .iter()
                .map(|(candidate, program)| {
                    let dir = &dir;
                    threads.spawn(move || {
                        program
                            .as_ref()
                            .and_then(|path| version_of(ctx, dir, candidate.tool, path))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().ok().flatten())
                .collect()
        });
        let records: Vec<Record> = found
            .into_iter()
            .zip(versions)
            .map(|((candidate, program), version)| {
                let tool = candidate.tool;
                let mut command = vec![program.as_ref().map_or_else(
                    || tool.program().to_owned(),
                    |p| p.to_string_lossy().into_owned(),
                )];
                command.extend(tool.check_args(&[]));
                Record::from(AnalyzeTool {
                    tool: tool.name().to_owned(),
                    language: tool.language().to_owned(),
                    check: check_of(tool),
                    found: program.is_some(),
                    version,
                    config: candidate.config.map(RecordPath::new),
                    command: Some(command),
                })
            })
            .collect();
        Ok(StageOutput::records(RecordStream::new(
            records.into_iter().map(Ok),
        )))
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
