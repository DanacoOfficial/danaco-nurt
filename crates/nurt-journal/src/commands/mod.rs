// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage implementations of the journal commands.
//!
//! Every stage converts the bound arguments of its catalog command into a
//! request of [`JournalStore`], calls the journal store of the execution
//! context and returns records of the output kind declared in the catalog.
//!
//! | Command | Interface | Journal call | Output |
//! |---|---|---|---|
//! | `history` | read | `history`, `details` | `operation` |
//! | `undo` | effect | `undo` | `effect` |
//! | `redo` | effect | `redo` | `effect` |
//! | `journal.pin` | effect | `pin` | `effect` |
//! | `journal.limits` | effect | `limits`, `status` | `journal_state`, `effect` |
//! | `journal.status` | read | `status` | `journal_state` |
//! | `journal.purge` | effect | `purge` | `effect` |
//! | `journal.space` | read | `space` | `volume` |
//!
//! The effect stages leave the transaction opened by the executor unused:
//! `undo` and `redo` record their own journal entry inside the journal store,
//! and the other journal commands record no entry. The number of the entry of
//! `undo` and `redo` is the `op` field of their `effect` record. Both read the
//! bound program of the request, stored in the new entry, from
//! [`ExecCtx::extensions`] as an `Arc<BoundDocument>`.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, OpId, SessionId};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, ExecCtx, InputKinds, StageDescriptor, StageImpl, StageRegistrar,
    StaticInfo,
};
use nurt_contracts::value::{self, Value};

mod history;
mod limits;
mod pin;
mod purge;
mod records;
mod redo;
mod space;
mod status;
#[cfg(test)]
mod testing;
mod undo;

/// Adds the stages of the journal commands to `registrar`.
///
/// The stages are `history`, `undo`, `redo`, `journal.pin`,
/// `journal.limits`, `journal.status`, `journal.purge` and `journal.space`.
/// Each of them reads the journal store of the execution context
/// ([`ExecCtx::journal`]).
///
/// # Errors
///
/// Fails with the first error of the registrar, such as `E_UNKNOWN_COMMAND`
/// when a command is not in the catalog of the registry.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::StageImpl;
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_journal::register(&mut stages)?;
/// assert_eq!(stages.len(), 8);
/// assert_eq!(stages[0].descriptor().command, "history");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    let stages: [Arc<dyn StageImpl>; 8] = [
        Arc::new(history::History),
        Arc::new(undo::Undo),
        Arc::new(redo::Redo),
        Arc::new(pin::Pin),
        Arc::new(limits::Limits),
        Arc::new(status::Status),
        Arc::new(purge::Purge),
        Arc::new(space::Space),
    ];
    stages
        .into_iter()
        .try_for_each(|stage| registrar.register(stage))
}

/// Descriptor of a command that only starts a pipeline.
fn source(command: &str) -> StageDescriptor {
    StageDescriptor {
        command: command.to_owned(),
        source: true,
        input: InputKinds::None,
    }
}

/// Static facts of a stage that names the given paths.
fn static_info(paths: Vec<String>) -> StaticInfo {
    StaticInfo {
        paths,
        ..StaticInfo::default()
    }
}

/// Typed access to the bound arguments of one command.
///
/// The executor passes every parameter of the signature, with defaults made
/// explicit, so an argument without an entry means that the stage and the
/// catalog disagree.
struct Params<'a> {
    args: &'a BoundArgs,
}

impl<'a> Params<'a> {
    fn new(args: &'a BoundArgs) -> Self {
        Self { args }
    }

    fn command(&self) -> &'a str {
        self.args.command()
    }

    fn value(&self, name: &str) -> Result<Option<&'a Value>> {
        match self.args.get(name) {
            None => Err(Error::new(
                ErrorCode::StageFailed,
                format!(
                    "`{}` received no argument `{name}`; the stage does not match the catalog",
                    self.command()
                ),
            )),
            Some(ArgValue::Absent | ArgValue::Value(Value::Null)) => Ok(None),
            Some(ArgValue::Value(value)) => Ok(Some(value)),
            Some(_) => Err(self.type_error(name, "a value known before the stage runs")),
        }
    }

    fn int(&self, name: &str) -> Result<Option<i64>> {
        match self.value(name)? {
            None => Ok(None),
            Some(Value::Int(number)) => Ok(Some(*number)),
            Some(_) => Err(self.type_error(name, "int")),
        }
    }

    fn op(&self, name: &str) -> Result<Option<OpId>> {
        self.int(name)?
            .map(|number| self.op_number(number))
            .transpose()
    }

    fn op_number(&self, number: i64) -> Result<OpId> {
        u64::try_from(number)
            .ok()
            .filter(|number| *number > 0)
            .map(OpId)
            .ok_or_else(|| {
                self.invalid(format!(
                    "operation number {number} is not valid; operation numbers start at 1"
                ))
            })
    }

    fn ops(&self, name: &str) -> Result<Option<Vec<OpId>>> {
        let items = match self.value(name)? {
            None => return Ok(None),
            Some(Value::Int(number)) => return Ok(Some(vec![self.op_number(*number)?])),
            Some(Value::List(items)) => items,
            Some(_) => return Err(self.type_error(name, "list<int>")),
        };
        items
            .iter()
            .map(|item| match item {
                Value::Int(number) => self.op_number(*number),
                _ => Err(self.type_error(name, "list<int>")),
            })
            .collect::<Result<Vec<OpId>>>()
            .map(Some)
    }

    fn flag(&self, name: &str) -> Result<bool> {
        match self.value(name)? {
            None => Ok(false),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(_) => Err(self.type_error(name, "bool")),
        }
    }

    fn path(&self, name: &str) -> Result<Option<String>> {
        match self.value(name)? {
            None => Ok(None),
            Some(Value::Path(text) | Value::Text(text)) => Ok(Some(text.clone())),
            Some(_) => Err(self.type_error(name, "path")),
        }
    }

    fn paths(&self, name: &str) -> Result<Option<Vec<String>>> {
        let items = match self.value(name)? {
            None => return Ok(None),
            Some(Value::Path(text) | Value::Text(text)) => return Ok(Some(vec![text.clone()])),
            Some(Value::List(items)) => items,
            Some(_) => return Err(self.type_error(name, "list<path>")),
        };
        items
            .iter()
            .map(|item| match item {
                Value::Path(text) | Value::Text(text) => Ok(text.clone()),
                _ => Err(self.type_error(name, "list<path>")),
            })
            .collect::<Result<Vec<String>>>()
            .map(Some)
    }

    fn duration(&self, name: &str) -> Result<Option<value::Duration>> {
        match self.value(name)? {
            None => Ok(None),
            Some(Value::Duration(duration)) => Ok(Some(*duration)),
            Some(_) => Err(self.type_error(name, "duration")),
        }
    }

    fn choice<T: Copy>(&self, name: &str, choices: &[(&str, T)]) -> Result<Option<T>> {
        let word = match self.value(name)? {
            None => return Ok(None),
            Some(Value::Enum(word) | Value::Text(word)) => word,
            Some(_) => return Err(self.type_error(name, "an enumeration value")),
        };
        choices
            .iter()
            .find(|(choice, _)| *choice == word)
            .map(|(_, value)| Some(*value))
            .ok_or_else(|| {
                let known: Vec<&str> = choices.iter().map(|(choice, _)| *choice).collect();
                self.invalid(format!(
                    "`{word}` is not a value of `{name}`; use one of {}",
                    known.join(", ")
                ))
            })
    }

    fn type_error(&self, name: &str, expected: &str) -> Error {
        Error::new(
            ErrorCode::Type,
            format!("`{}` expects {expected} for `{name}`", self.command()),
        )
    }

    fn invalid(&self, message: impl fmt::Display) -> Error {
        Error::new(
            ErrorCode::InvalidValue,
            format!("`{}`: {message}", self.command()),
        )
    }
}

/// The journal store of the execution context.
fn store(ctx: &ExecCtx) -> Result<&dyn JournalStore> {
    ctx.journal().ok_or_else(|| missing(ctx, "journal store"))
}

/// Session of the request.
fn session(ctx: &ExecCtx) -> Option<SessionId> {
    ctx.options().session.clone().map(SessionId)
}

/// Bound program of the request, stored in the entries of `undo` and `redo`.
fn program(ctx: &ExecCtx) -> Result<BoundDocument> {
    ctx.extensions()
        .get::<BoundDocument>()
        .map(|document| document.as_ref().clone())
        .ok_or_else(|| missing(ctx, "bound program"))
}

fn missing(ctx: &ExecCtx, what: &str) -> Error {
    Error::new(
        ErrorCode::StageFailed,
        format!(
            "the execution context of `{}` has no {what}",
            ctx.frame().command
        ),
    )
}

/// Absolute path of a path argument; relative paths are resolved against the
/// `base` option of the request.
fn resolve(ctx: &ExecCtx, text: &str) -> Result<AbsPath> {
    if let Ok(path) = AbsPath::new(text) {
        return Ok(path);
    }
    ctx.options()
        .base
        .as_deref()
        .and_then(|base| AbsPath::new(PathBuf::from(base).join(text)).ok())
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "path `{text}` is not an absolute path; give an absolute path or set the request option `base`"
                ),
            )
        })
}

/// Absolute path of an optional path argument.
fn resolve_opt(ctx: &ExecCtx, text: Option<&str>) -> Result<Option<AbsPath>> {
    text.map(|text| resolve(ctx, text)).transpose()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::kinds::EffectKind;
    use nurt_contracts::spec::{IoSpec, StageSpec};
    use nurt_contracts::stage::{BoundArgs, InputKinds, StageImpl};
    use nurt_contracts::value::{Fields, Value};

    use super::testing::{self, TestHost};
    use super::{history, limits, pin, purge, redo, register, space, status, undo};

    const COMMANDS: [&str; 8] = [
        "history",
        "undo",
        "redo",
        "journal.pin",
        "journal.limits",
        "journal.status",
        "journal.purge",
        "journal.space",
    ];

    /// Parsed arguments of a command in a comparable form.
    fn parsed(command: &str, args: &BoundArgs) -> Result<String, ErrorCode> {
        let text = match command {
            "history" => history::parse(args).map(|parsed| format!("{parsed:?}")),
            "undo" => undo::parse(args).map(|parsed| format!("{parsed:?}")),
            "redo" => redo::parse(args).map(|parsed| format!("{parsed:?}")),
            "journal.pin" => pin::parse(args).map(|parsed| format!("{parsed:?}")),
            "journal.limits" => limits::parse(args).map(|parsed| format!("{parsed:?}")),
            "journal.status" => status::parse(args).map(|parsed| format!("{parsed:?}")),
            "journal.purge" => purge::parse(args).map(|parsed| format!("{parsed:?}")),
            "journal.space" => space::parse(args).map(|parsed| format!("{parsed:?}")),
            other => panic!("no parser for {other}"),
        };
        text.map_err(|error| error.code)
    }

    /// Arguments a parameter needs besides itself to be accepted.
    fn companions(command: &str, param: &str) -> Vec<(&'static str, Value)> {
        match (command, param) {
            ("undo", "since") => vec![("file", testing::sample_path())],
            ("undo", "paths") => vec![("id", Value::Int(1))],
            ("undo", "all_sessions") => vec![("to", Value::Int(1))],
            ("journal.pin", other) if other != "id" => {
                vec![("id", Value::List(vec![Value::Int(1)]))]
            }
            _ => Vec::new(),
        }
    }

    fn given<'a>(
        companions: &[(&'a str, Value)],
        name: &'a str,
        value: Value,
    ) -> Vec<(&'a str, Value)> {
        let mut given = companions.to_vec();
        given.push((name, value));
        given
    }

    fn check_parameters(stage: &dyn StageImpl, spec: &StageSpec) {
        let host = TestHost::new();
        let config = nurt_contracts::config::Config::default();
        let ctx = nurt_contracts::stage::CheckCtx::new(spec, &host, &config);
        for param in &spec.params {
            let companions = companions(&spec.name, &param.name);
            let wrong = testing::args(
                spec,
                &given(&companions, &param.name, Value::Map(Fields::new())),
            );
            let error = stage.check(&wrong, &ctx).unwrap_err();
            assert_eq!(error.code, ErrorCode::Type, "{}.{}", spec.name, param.name);
            assert!(
                error.message.contains(&format!("`{}`", param.name)),
                "{}",
                error.message
            );
            for sample in testing::samples(&param.ty) {
                let args = testing::args(spec, &given(&companions, &param.name, sample.clone()));
                let accepted = stage.check(&args, &ctx);
                assert!(
                    accepted.is_ok(),
                    "{}.{} rejects {sample:?}: {accepted:?}",
                    spec.name,
                    param.name
                );
            }
        }
    }

    #[test]
    fn registers_one_stage_per_journal_command() {
        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        register(&mut stages).unwrap();
        let names: Vec<String> = stages
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        assert_eq!(names, COMMANDS);
    }

    #[test]
    fn every_stage_matches_its_catalog_signature() {
        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        register(&mut stages).unwrap();
        let host = TestHost::new();
        let config = nurt_contracts::config::Config::default();
        for stage in &stages {
            let descriptor = stage.descriptor();
            let spec = testing::spec(&descriptor.command);
            assert!(spec.check().is_empty(), "{:?}", spec.check());
            assert_eq!(spec.input, [IoSpec::None], "{}", spec.name);
            assert!(descriptor.source);
            assert_eq!(descriptor.input, InputKinds::None);
            let reads = spec.effect == EffectKind::Read;
            assert_eq!(stage.as_read().is_some(), reads, "{}", spec.name);
            assert_eq!(stage.as_effect().is_some(), !reads, "{}", spec.name);

            let defaults = testing::args(&spec, &[]);
            let ctx = nurt_contracts::stage::CheckCtx::new(&spec, &host, &config);
            if let Err(error) = stage.check(&defaults, &ctx) {
                assert_eq!(error.code, ErrorCode::MissingParam, "{}", spec.name);
            }
            assert_eq!(
                parsed(&spec.name, &testing::absent(&spec)),
                parsed(&spec.name, &defaults),
                "{}: an absent argument and the catalog default differ",
                spec.name
            );
            check_parameters(stage.as_ref(), &spec);
        }
    }

    #[test]
    fn a_stage_without_an_argument_entry_fails() {
        let args = BoundArgs::new("journal.status");
        let error = status::parse(&args).unwrap_err();
        assert_eq!(error.code, ErrorCode::StageFailed);
    }

    #[test]
    fn rejects_operation_numbers_below_one() {
        let args = testing::call("redo", &[("id", Value::Int(0))]);
        assert_eq!(
            redo::parse(&args).unwrap_err().code,
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn resolves_relative_paths_against_the_base_option() {
        let env = testing::Env::new();
        let ctx = env.ctx_with("journal.status", |options| {
            options.base = Some(nurt_testkit::paths::abs("/home/user"));
        });
        assert_eq!(
            super::resolve(&ctx, "docs").unwrap(),
            testing::path("/home/user/docs")
        );
        let ctx = env.ctx("journal.status");
        let error = super::resolve(&ctx, "docs").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
    }

    #[test]
    fn absolute_paths_ignore_the_base_option_and_multibyte_names_are_kept() {
        let env = testing::Env::new();
        let ctx = env.ctx_with("journal.status", |options| {
            options.base = Some(nurt_testkit::paths::abs("/home/użytkownik"));
        });
        assert_eq!(
            super::resolve(&ctx, "docs/zażółć.txt").unwrap(),
            testing::path("/home/użytkownik/docs/zażółć.txt")
        );
        assert_eq!(
            super::resolve(&ctx, "/srv/data").unwrap(),
            testing::path("/srv/data")
        );
        assert_eq!(super::resolve_opt(&ctx, None).unwrap(), None);
        assert_eq!(
            super::resolve_opt(&ctx, Some("/srv/data")).unwrap(),
            Some(testing::path("/srv/data"))
        );
        let relative_base = env.ctx_with("journal.status", |options| {
            options.base = Some("relative".to_owned());
        });
        let error = super::resolve(&relative_base, "docs").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
    }

    #[test]
    fn the_session_of_the_request_is_taken_from_its_options() {
        let env = testing::Env::new();
        let with = env.ctx("journal.status");
        assert_eq!(
            super::session(&with).map(|session| session.0),
            Some("s1".to_owned())
        );
        let without = env.ctx_with("journal.status", |options| options.session = None);
        assert_eq!(super::session(&without), None);
    }

    #[test]
    fn a_context_without_a_journal_store_fails() {
        let ctx = testing::bare_ctx("journal.status");
        let error = super::store(&ctx).err().unwrap();
        assert_eq!(error.code, ErrorCode::StageFailed);
        assert!(error.message.contains("journal store"));
    }
}
