// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The apply step: one journal transaction per operation.

use std::time::Duration;

use nurt_catalog::Registry;
use nurt_contracts::ast::Span;
use nurt_contracts::bound::{
    BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundProgram, BoundRef, BoundStage,
    BoundStatement,
};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::FileKind;
use nurt_contracts::journal::{
    BeginRequest, JournalStore, MutationTx, OpHandle, OpKind, RollbackReason, SessionId,
    TargetAction, TargetOutcome, TargetPath,
};
use nurt_contracts::record::Change;
use nurt_contracts::report::{
    Correction, FLAG_DIRECTORY, IrreversibleEntry, NotableEntry, tree_counts,
};
use nurt_contracts::stage::{BoundArgs, EffectOutcome, EffectPlan, ExecCtx, ExecStep};
use serde_json::json;

use crate::flow::at_step;
use crate::grouped::Effect;
use crate::outcome::{ChangeFacts, ResultData};

/// Text that replaces the value of a secret parameter in the program stored
/// by the journal.
pub(crate) const MASKED: &str = "***";

/// Flag of a change that replaced an existing entry.
const OVERWROTE_EXISTING: &str = "overwrote_existing";

/// A committed effect.
#[derive(Debug)]
pub(crate) struct AppliedEffect {
    pub(crate) records: ResultData,
    pub(crate) handle: OpHandle,
    pub(crate) changes: Vec<Change>,
    pub(crate) notable: Vec<NotableEntry>,
    pub(crate) irreversible: Vec<IrreversibleEntry>,
    pub(crate) detached: bool,
}

/// What the journal records about the program of an operation.
pub(crate) struct ProgramSource<'a> {
    pub(crate) registry: &'a Registry,
    pub(crate) catalog_rev: &'a str,
    pub(crate) corr_rev: &'a str,
    pub(crate) corrections: &'a [Correction],
}

impl ProgramSource<'_> {
    /// Bound program of one pipeline with the values of secret parameters
    /// masked.
    pub(crate) fn document<'s>(
        &self,
        head: Option<&BoundRef>,
        stages: impl Iterator<Item = &'s BoundStage>,
        span: Option<Span>,
    ) -> BoundDocument {
        let mut stages: Vec<BoundStage> = stages.map(|stage| self.masked(stage)).collect();
        let head = match head {
            Some(reference) => BoundHead::Ref(reference.clone()),
            None if stages.is_empty() => BoundHead::Ref(BoundRef {
                name: String::new(),
                fields: Vec::new(),
            }),
            None => BoundHead::Stage(stages.remove(0)),
        };
        BoundDocument::new(
            self.catalog_rev,
            self.corr_rev,
            BoundProgram {
                statements: vec![BoundStatement::Pipeline(BoundPipeline {
                    head,
                    stages,
                    span,
                })],
            },
        )
    }

    fn masked(&self, stage: &BoundStage) -> BoundStage {
        let mut stage = stage.clone();
        if let Some(spec) = self.registry.command(&stage.name) {
            for arg in &mut stage.args {
                if spec.param(&arg.name).is_some_and(|param| param.secret) {
                    arg.value = BoundExpr::Text {
                        v: MASKED.to_owned(),
                    };
                }
            }
        }
        stage
    }

    /// Parameters of the transaction of an operation.
    pub(crate) fn begin_request(
        &self,
        command: &str,
        program: BoundDocument,
        ctx: &ExecCtx,
        bulk: bool,
        preserve_budget: Option<Duration>,
    ) -> BeginRequest {
        BeginRequest {
            kind: OpKind::command(command),
            session: ctx.options().session.clone().map(SessionId),
            program,
            corrections: self.corrections.to_vec(),
            bulk,
            counts_as_step: true,
            preserve_budget,
        }
    }
}

/// Opens the transaction, applies the planned effect, collects the records
/// of the effect and commits.
///
/// # Errors
///
/// Fails with the error of the effect or of the transaction after rolling
/// the transaction back; a rollback that leaves targets unrestored is
/// `E_ROLLBACK_FAILED` with the targets in the detail `failed`.
pub(crate) fn apply_effect(
    effect: &Effect,
    args: &BoundArgs,
    plan: EffectPlan,
    journal: &dyn JournalStore,
    request: BeginRequest,
    ctx: &ExecCtx,
) -> Result<AppliedEffect> {
    let mut tx = journal.begin(request);
    let outcome = match effect.apply(args, plan, tx.as_mut(), ctx) {
        Ok(outcome) => outcome,
        Err(error) => return Err(roll_back(tx, error, ctx)),
    };
    let EffectOutcome {
        output,
        changes,
        notable,
        irreversible,
        detached,
    } = outcome;
    let records = match ResultData::collect(output.into(), ctx.limits().max_records) {
        Ok(records) => records,
        Err(error) => return Err(roll_back(tx, error, ctx)),
    };
    let handle = tx
        .commit()
        .map_err(|error| at_step(error, ExecStep::Apply))?;
    Ok(AppliedEffect {
        records,
        handle,
        changes,
        notable,
        irreversible,
        detached,
    })
}

fn roll_back(tx: Box<dyn MutationTx + '_>, error: Error, ctx: &ExecCtx) -> Error {
    let reason = if ctx.cancel().is_cancelled() {
        RollbackReason::Cancelled
    } else {
        RollbackReason::TargetFailed
    };
    let error = at_step(error, ExecStep::Apply);
    match tx.rollback(reason) {
        Ok(report) if report.failed.is_empty() => error,
        Ok(_) if error.code() == ErrorCode::RollbackFailed => error,
        Ok(report) => rollback_failed(&report.failed).with_cause(error),
        Err(failure) if failure.code() == ErrorCode::RolledBack => error,
        Err(failure) => at_step(failure, ExecStep::Apply).with_cause(error),
    }
}

/// `E_ROLLBACK_FAILED` listing the targets that were not restored.
pub(crate) fn rollback_failed(failed: &[TargetPath]) -> Error {
    let paths: Vec<String> = failed.iter().map(target_text).collect();
    Error::new(
        ErrorCode::RollbackFailed,
        format!(
            "{} target(s) could not be restored; inspect them before retrying",
            failed.len()
        ),
    )
    .with_detail("failed", json!(paths))
    .with_stage(ExecStep::Apply)
}

/// Text form of a target: the path of a file system entry, the JSON form of
/// a provider key.
pub(crate) fn target_text(path: &TargetPath) -> String {
    match path {
        TargetPath::Fs(path) => path.as_path().to_string_lossy().into_owned(),
        TargetPath::Provider(key) => serde_json::to_string(key).unwrap_or_default(),
    }
}

/// Changes of an effect with the facts the journal reported for their
/// targets.
///
/// The targets are indexed by their text once, so that the cost grows with the
/// number of changes and not with its square; of several targets with one text
/// the first is used.
pub(crate) fn change_facts(changes: Vec<Change>, handle: Option<&OpHandle>) -> Vec<ChangeFacts> {
    let mut by_text: std::collections::HashMap<String, &TargetOutcome> =
        std::collections::HashMap::new();
    if let Some(handle) = handle {
        by_text.reserve(handle.targets.len());
        for target in &handle.targets {
            by_text.entry(target_text(&target.path)).or_insert(target);
        }
    }
    changes
        .into_iter()
        .map(|change| {
            let flagged = change
                .flags
                .as_ref()
                .is_some_and(|flags| flags.iter().any(|flag| flag == OVERWROTE_EXISTING));
            let target = by_text.get(change.path.as_str()).copied();
            let journaled = target.and_then(|target| {
                if target.before.exists {
                    target.before.kind
                } else {
                    target.after.as_ref().and_then(|after| after.kind)
                }
            });
            let noted_directory = change
                .flags
                .as_ref()
                .is_some_and(|flags| flags.iter().any(|flag| flag == FLAG_DIRECTORY));
            let entry = journaled.or_else(|| noted_directory.then_some(FileKind::Directory));
            let inside = change.flags.as_deref().and_then(tree_counts);
            let replaced = target.is_some_and(|target| {
                target.before.exists && matches!(target.action, TargetAction::Replace)
            });
            ChangeFacts {
                change,
                entry,
                replaced_existing: flagged || replaced,
                inside,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use nurt_contracts::bound::{BoundArg, Phase};
    use nurt_contracts::fs::{AbsPath, FileKind};
    use nurt_contracts::journal::{BeforeState, PreserveMode, TargetIndex, TargetOutcome};
    use nurt_contracts::kinds::EffectKind;
    use nurt_contracts::record::ChangeAction;

    use super::*;

    fn outcome(path: &str, action: TargetAction, kind: FileKind) -> TargetOutcome {
        TargetOutcome {
            index: TargetIndex(0),
            path: TargetPath::Fs(AbsPath::new(path).unwrap()),
            action,
            mode: PreserveMode::Copy,
            bytes_held: 0,
            before: BeforeState {
                exists: true,
                kind: Some(kind),
                ..BeforeState::default()
            },
            after: None,
            irreversible: None,
            notable: Vec::new(),
        }
    }

    fn handle(targets: Vec<TargetOutcome>) -> OpHandle {
        OpHandle {
            op: None,
            kind: OpKind::command("write"),
            bulk: false,
            targets,
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
        }
    }

    #[test]
    fn facts_come_from_the_journal_targets() {
        let handle = handle(vec![
            outcome("/a", TargetAction::Replace, FileKind::Directory),
            outcome("/b", TargetAction::Modify, FileKind::File),
        ]);
        let mut flagged = Change::new("/c", ChangeAction::Modify);
        flagged.flags = Some(vec![OVERWROTE_EXISTING.to_owned()]);
        let facts = change_facts(
            vec![
                Change::new("/a", ChangeAction::Modify),
                Change::new("/b", ChangeAction::Modify),
                flagged,
            ],
            Some(&handle),
        );
        assert_eq!(facts[0].entry, Some(FileKind::Directory));
        assert!(facts[0].replaced_existing);
        assert_eq!(facts[1].entry, Some(FileKind::File));
        assert!(!facts[1].replaced_existing);
        assert_eq!(facts[2].entry, None);
        assert!(facts[2].replaced_existing);
    }

    #[test]
    fn notes_give_the_kind_and_the_content_of_a_directory_without_a_journal_target() {
        let mut tree = Change::new("/tree", ChangeAction::Delete);
        tree.flags = Some(
            ["recursive".to_owned()]
                .into_iter()
                .chain(nurt_contracts::report::tree_flags(4, 2))
                .collect(),
        );
        let plain = Change::new("/plain", ChangeAction::Delete);
        let facts = change_facts(vec![tree.clone(), plain], None);
        assert_eq!(facts[0].entry, Some(FileKind::Directory));
        assert_eq!(facts[0].inside, Some((4, 2)));
        assert_eq!(facts[1].entry, None);
        assert_eq!(facts[1].inside, None);
        let handle = handle(vec![outcome("/tree", TargetAction::Delete, FileKind::File)]);
        let facts = change_facts(vec![tree], Some(&handle));
        assert_eq!(facts[0].entry, Some(FileKind::File));
    }

    #[test]
    fn masks_secret_parameters_in_the_stored_program() {
        let mut registry = Registry::load_embedded().unwrap();
        let mut spec = registry.command("list").cloned().unwrap();
        spec.name = "masked.list".to_owned();
        spec.group.clear();
        spec.params[0].secret = true;
        let secret = (spec.name.clone(), spec.params[0].name.clone());
        registry
            .register_extension("masked", "Commands with a secret parameter.", vec![spec])
            .unwrap();
        let stage = BoundStage {
            name: secret.0,
            effect: EffectKind::External,
            phase: Phase::Run,
            waits: false,
            args: vec![BoundArg::given(
                secret.1,
                BoundExpr::Text {
                    v: "token".to_owned(),
                },
            )],
            span: None,
        };
        let source = ProgramSource {
            registry: &registry,
            catalog_rev: "cat",
            corr_rev: "corr",
            corrections: &[],
        };
        let document = source.document(None, std::iter::once(&stage), None);
        let BoundStatement::Pipeline(pipeline) = &document.program.statements[0] else {
            panic!("pipeline expected");
        };
        let BoundHead::Stage(head) = &pipeline.head else {
            panic!("stage expected");
        };
        assert_eq!(
            head.args[0].value,
            BoundExpr::Text {
                v: MASKED.to_owned()
            }
        );
        assert_eq!(document.catalog_rev, "cat");
    }

    #[test]
    fn rollback_error_lists_targets() {
        let error = rollback_failed(&[TargetPath::Fs(AbsPath::new("/x").unwrap())]);
        assert_eq!(error.code(), ErrorCode::RollbackFailed);
        assert_eq!(error.details["failed"], json!(["/x"]));
        assert_eq!(error.stage(), Some(ExecStep::Apply));
    }
}
