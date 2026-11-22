// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `sql` stage.

use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::Value;

use crate::sql::{self, SqlParams, SqlRequest};

const COMMAND: &str = "sql";

/// Implementation of the `sql` command, which runs SQL statements on an
/// SQLite database file.
///
/// With `write: false` the stage only reads. The database is opened
/// read-only, and a statement that changes it fails with `E_SQL`. With
/// `write: true` the stage is an effect. The operation journal preserves the
/// database file and its journal files, and the statements run in one
/// transaction.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_doc::commands::sql::SqlStage;
///
/// let descriptor = SqlStage.descriptor();
/// assert_eq!(descriptor.command, "sql");
/// assert!(descriptor.source);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlStage;

impl StageImpl for SqlStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    /// Checks the argument types and that the query holds a statement.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` for an argument of another type, with
    /// `E_MISSING_PARAM` without a path or a query, and with `E_SQL` when the
    /// query holds no statement.
    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let path = path_arg(args)?;
        let query = query_arg(args)?;
        params_arg(args)?;
        write_arg(args)?;
        if sql::statement_count(&query) == 0 {
            return Err(Error::new(
                ErrorCode::Sql,
                "`query` holds no SQL statement; pass at least one statement",
            )
            .with_problem("empty_query"));
        }
        Ok(StaticInfo {
            paths: vec![path],
            ..StaticInfo::default()
        })
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl ReadStage for SqlStage {
    /// Streams the rows of read-only statements.
    ///
    /// # Errors
    ///
    /// Fails with `E_STAGE_FAILED` when changes are allowed, because changes
    /// run in the effect step. Other failures are those of the command,
    /// such as `E_NOT_FOUND`, `E_DOC_PARSE`, `E_SQL`, `E_IN_USE`,
    /// `E_PERMISSION` and `E_IO`.
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let request = request(args, ctx)?;
        if request.write {
            return Err(Error::new(
                ErrorCode::StageFailed,
                "`sql` with `write: true` changes the database and runs in the effect step",
            ));
        }
        Ok(StageOutput::records(sql::read_rows(request, ctx)?))
    }
}

impl EffectStage for SqlStage {
    /// Checks the database file and plans one change of it.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND`, `E_PERMISSION`, `E_IO` or `E_DOC_PARSE` for
    /// the database file, and with the argument errors of the static check.
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let request = request(args, ctx)?;
        sql::check_file(&request)?;
        let mut plan = EffectPlan::new();
        if request.write {
            plan.changes.push(sql::planned_change(&request));
        }
        Ok(plan.with_state(request))
    }

    /// Runs the statements, through the transaction when changes are
    /// allowed.
    ///
    /// # Errors
    ///
    /// Fails with `E_SQL` for a statement that fails or is refused, with
    /// `E_IN_USE` when the database stays locked longer than
    /// `sql.busy_timeout`, with `E_DOC_PARSE` for a damaged database, with
    /// `E_LIMIT` when the statements return more rows than the execution may
    /// hold, with `E_CANCELLED` or `E_TIMEOUT`, and with the errors of the
    /// transaction.
    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let request = match plan.take_state::<SqlRequest>() {
            Some(request) => request,
            None => request(args, ctx)?,
        };
        if !request.write {
            return Ok(EffectOutcome::new(StageOutput::records(sql::read_rows(
                request, ctx,
            )?)));
        }
        let outcome = sql::write(&request, tx, ctx)?;
        let mut effect = EffectOutcome::new(StageOutput::records(RecordStream::from_records(
            outcome.rows,
        )));
        effect.changes.push(outcome.change);
        effect.irreversible = outcome.irreversible;
        Ok(effect)
    }
}

fn request(args: &BoundArgs, ctx: &ExecCtx) -> Result<SqlRequest> {
    let display = path_arg(args)?;
    Ok(SqlRequest {
        path: PathBuf::from(&display),
        display,
        query: query_arg(args)?,
        params: params_arg(args)?,
        write: write_arg(args)?,
        busy_timeout: ctx.config().sql.busy_timeout.as_std(),
    })
}

fn path_arg(args: &BoundArgs) -> Result<String> {
    match args.require("path")? {
        Value::Path(path) | Value::Text(path) => Ok(path.clone()),
        other => Err(type_error("path", "path", other)),
    }
}

fn query_arg(args: &BoundArgs) -> Result<String> {
    match args.require("query")? {
        Value::Text(query) => Ok(query.clone()),
        other => Err(type_error("query", "text", other)),
    }
}

fn params_arg(args: &BoundArgs) -> Result<SqlParams> {
    match args.value("params") {
        None | Some(Value::Null) => Ok(SqlParams::List(Vec::new())),
        Some(Value::List(values)) => Ok(SqlParams::List(values.clone())),
        Some(Value::Map(fields)) => Ok(SqlParams::Map(fields.clone())),
        Some(other) => Err(type_error("params", "list or map", other)),
    }
}

fn write_arg(args: &BoundArgs) -> Result<bool> {
    Ok(args.bool("write")?.unwrap_or(false))
}

fn type_error(name: &str, expected: &str, found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "`sql` expects {expected} for `{name}`, not {}; pass a {expected}",
            found.type_name()
        ),
    )
}
