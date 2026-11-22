// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal transaction of a run.
//!
//! A run keeps its transaction open from the preparation of change detection
//! until the program has ended and its changes are recorded. Without a
//! journal store the run proceeds with a transaction that keeps nothing and
//! marks every change irreversible.

use std::fs;

use nurt_contracts::bound::{
    BoundArg, BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundProgram, BoundStage,
    BoundStatement, Phase,
};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{FileKind, WriteHandle};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, BeginRequest, ExemptWrite, ExternalChange,
    ExternalChangeKind, Mutation, MutationTx, OpHandle, OpId, OpKind, PreserveMode, Preserved,
    RollbackReason, RollbackReport, SessionId, TargetIndex, TargetSpec,
};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::stage::ExecCtx;

use crate::iface::RunRequest;

/// Bound program recorded in the journal entry of a run: the program given
/// by the executor in the context extensions, or a program of the single
/// command `command_name` with the command line of the run.
pub(crate) fn bound_program(
    ctx: &ExecCtx,
    command_name: &str,
    request: &RunRequest,
) -> BoundDocument {
    if let Some(document) = ctx.extensions().get::<BoundDocument>() {
        return BoundDocument::clone(&document);
    }
    let command = BoundExpr::List {
        items: request
            .command
            .iter()
            .map(|word| BoundExpr::Text { v: word.clone() })
            .collect(),
    };
    let stage = BoundStage {
        name: command_name.to_owned(),
        effect: EffectKind::Process,
        phase: Phase::Run,
        waits: true,
        args: vec![BoundArg::given("command", command)],
        span: None,
    };
    let pipeline = BoundPipeline {
        head: BoundHead::Stage(stage),
        stages: Vec::new(),
        span: None,
    };
    BoundDocument::new(
        String::new(),
        String::new(),
        BoundProgram {
            statements: vec![BoundStatement::Pipeline(pipeline)],
        },
    )
}

/// Opens the transaction of a run: a journal transaction when the context
/// has a journal store, otherwise a transaction that keeps nothing.
pub(crate) fn begin<'a>(
    ctx: &'a ExecCtx,
    command_name: &str,
    program: BoundDocument,
    request: &RunRequest,
) -> Box<dyn MutationTx + 'a> {
    let kind = OpKind::command(command_name);
    match ctx.journal() {
        Some(store) => store.begin(BeginRequest {
            kind,
            session: request.session.clone().map(SessionId),
            program,
            corrections: Vec::new(),
            bulk: false,
            counts_as_step: true,
            preserve_budget: Some(ctx.config().run.preimage_budget.as_std()),
        }),
        None => Box::new(Unjournaled::new(kind)),
    }
}

/// Transaction used without a journal store: nothing is kept, every
/// declared target and every observed change is irreversible, and no entry
/// is written.
pub(crate) struct Unjournaled {
    kind: OpKind,
    targets: u32,
    irreversible: Vec<IrreversibleEntry>,
}

impl Unjournaled {
    /// Creates an empty transaction of `kind`.
    pub(crate) fn new(kind: OpKind) -> Self {
        Self {
            kind,
            targets: 0,
            irreversible: Vec::new(),
        }
    }

    fn not_kept(&mut self, path: String) {
        self.irreversible.push(IrreversibleEntry {
            path: Some(path),
            ..IrreversibleEntry::new(Irreversible::JournalInactive)
        });
    }
}

impl MutationTx for Unjournaled {
    fn op(&self) -> Option<OpId> {
        None
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        let mut preserved = Vec::with_capacity(targets.len());
        for _ in targets {
            preserved.push(Preserved {
                index: TargetIndex(self.targets),
                volume: None,
                mode: PreserveMode::None,
                bytes_held: 0,
                before: BeforeState::default(),
                irreversible: Some(Irreversible::JournalInactive),
                notable: Vec::new(),
            });
            self.targets = self.targets.saturating_add(1);
        }
        Ok(preserved)
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(direct_writes())
    }

    fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
        Err(direct_writes())
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        let mut applied = Vec::with_capacity(changes.len());
        for (position, change) in changes.iter().enumerate() {
            let after = after_state(change);
            self.not_kept(change.path.as_path().to_string_lossy().into_owned());
            let index = change.declared.unwrap_or_else(|| {
                TargetIndex(
                    self.targets
                        .saturating_add(u32::try_from(position).unwrap_or(0)),
                )
            });
            applied.push(Applied { index, after });
        }
        Ok(applied)
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.irreversible.push(marker);
    }

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Ok(OpHandle {
            op: None,
            kind: self.kind,
            bulk: false,
            targets: Vec::new(),
            irreversible: self.irreversible,
            notable: Vec::new(),
            exempt: Vec::new(),
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Ok(RollbackReport {
            op: None,
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

fn after_state(change: &ExternalChange) -> AfterState {
    if change.kind == ExternalChangeKind::Deleted {
        return AfterState::default();
    }
    match fs::symlink_metadata(change.path.as_path()) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            let kind = if file_type.is_file() {
                FileKind::File
            } else if file_type.is_dir() {
                FileKind::Directory
            } else if file_type.is_symlink() {
                FileKind::Symlink
            } else {
                FileKind::Special
            };
            AfterState {
                exists: true,
                kind: Some(kind),
                size: Some(metadata.len()),
                hash: None,
                mode: None,
                meta: None,
            }
        }
        Err(_) => AfterState::default(),
    }
}

fn direct_writes() -> Error {
    Error::new(
        ErrorCode::StageFailed,
        "a run records the changes of its program only; direct writes need a journal store",
    )
}
