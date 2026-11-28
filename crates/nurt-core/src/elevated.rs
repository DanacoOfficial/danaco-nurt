// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-core::elevated
// Responsibility:   Routing of requests that ask for elevated rights.
// Dependencies:     nurt_contracts, nurt_proc
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Routing of requests that ask for elevated rights.
//!
//! A request with `privilege: elevated`, with `as_user`, or with
//! `privilege: auto` after a denial runs through the elevation backends of
//! the configuration. The first backend that answers decides: its response is
//! the response of the call.

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::bound::{BoundDocument, BoundExpr, BoundHead, BoundStatement};
use nurt_contracts::envelope::{Privilege, Request, RequestInput, Response};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::{Correction, ReportStatus};
use nurt_contracts::stage::{CancelToken, EventSink};
use nurt_proc::elevation::{Backend, Refusal, System, run_backend};

use crate::elevation::{
    ENVELOPE_VERSION, ElevatedEnvelope, ElevatedOwner, Mark, REASON_AUTO, REASON_ENTRY,
    as_user_unsupported, backends_of, disabled_error, refused_error, system_of,
};
use crate::engine::{Engine, Prepared, failed_report, input_kind, respond_with};
use crate::frontend::Compiled;

/// Result of the compilation of a request, made once by [`Engine::route`] and
/// reused by the run in this process.
pub(crate) type Early = Result<Compiled, Error>;

/// Time allowed to a backend beyond the time limit of the request.
const BACKEND_GRACE: Duration = Duration::from_secs(30);

/// How [`Engine::run_elevated`] picks the backends and what it reports.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Route {
    /// Reason stamped on the report.
    pub(crate) reason: Option<&'static str>,
    /// The only backend to try: the one that wrote the entry that an `undo` or
    /// a `redo` restores.
    pub(crate) only: Option<Backend>,
}

impl Route {
    /// A route that tries the configured backends in order.
    pub(crate) const fn new(reason: Option<&'static str>) -> Self {
        Self { reason, only: None }
    }
}

/// The backends to try: the configured ones, or, for the restore of an entry
/// that names its backend, that backend alone.
///
/// # Errors
///
/// `E_PRIVILEGE` naming the backend when the entry was written through a
/// backend that `privilege.backends` does not allow.
pub(crate) fn choose_backends(
    configured: Vec<Backend>,
    recorded: Option<Backend>,
) -> Result<Vec<Backend>, Error> {
    let Some(backend) = recorded else {
        return Ok(configured);
    };
    if configured.contains(&backend) {
        return Ok(vec![backend]);
    }
    Err(Error::new(
        ErrorCode::Privilege,
        format!(
            "the entry was written through `{backend}`, which `privilege.backends` does not allow"
        ),
    )
    .with_detail(
        "backend",
        serde_json::json!([{"backend": backend.as_str(), "result": "not_allowed"}]),
    )
    .with_next_step(format!(
        "add `{backend}` to `privilege.backends` to restore the entry"
    )))
}

/// The backend named by the action of an entry (`elevated:<backend>`).
pub(crate) fn backend_of_action(action: &str) -> Option<Backend> {
    action
        .strip_prefix("elevated:")
        .and_then(Backend::from_name)
}

impl Engine {
    /// Runs a request with the rights its options ask for.
    pub(crate) fn route(
        &self,
        request: &Request,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
    ) -> Response {
        self.route_early(request, cancel, events, None)
    }

    fn route_early(
        &self,
        request: &Request,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
        early: Option<Early>,
    ) -> Response {
        let Ok(prepared) = self.prepare(request) else {
            return self.execute_local(request, cancel, events, None, None);
        };
        let early = early.or_else(|| self.compile_early(request, &prepared));
        match self.with_stage_call(request, &prepared, early.as_ref()) {
            Ok(Some(forced)) => return self.route_early(&forced, cancel, events, early),
            Ok(None) => {}
            Err(error) => return refusal(request, &prepared, error),
        }
        let privilege = prepared.options.effective_privilege();
        let pages = matches!(request.input(), Ok(RequestInput::Cursor(_)));
        if privilege == Privilege::Account || pages {
            return self.execute_local(request, cancel, events, None, early);
        }
        if !prepared.config.config().privilege.allowed {
            return refusal(request, &prepared, disabled_error());
        }
        if privilege == Privilege::Auto {
            let first = self.execute_local(request, cancel, events.clone(), None, early.clone());
            if !denied(&first) {
                return first;
            }
            return match self.run_elevated(
                request,
                &prepared,
                Route::new(Some(REASON_AUTO)),
                cancel,
                events,
                early,
            ) {
                Ok(response) => response,
                Err(error) => {
                    let cause = first.report.error.clone();
                    let error = cause.map_or(error.clone(), |cause| error.with_cause(cause));
                    refusal(request, &prepared, error)
                }
            };
        }
        match self.run_elevated(request, &prepared, Route::new(None), cancel, events, early) {
            Ok(response) => response,
            Err(error) => refusal(request, &prepared, error),
        }
    }

    /// Compiles the program of a request once; `None` for a request that
    /// reads a page or accepts a candidate.
    fn compile_early(&self, request: &Request, prepared: &Prepared) -> Option<Early> {
        match request.input() {
            Ok(RequestInput::Program { text, variant }) => {
                Some(self.compile(text, variant, prepared))
            }
            Ok(RequestInput::Ast(document)) => Some(self.bind(document, prepared)),
            Ok(RequestInput::Accept(reference)) => Some(self.accept(reference, prepared)),
            _ => None,
        }
    }

    /// Executes one request with the identity of the administrator, for the
    /// hidden subcommand `elevated-exec`.
    ///
    /// The process must be the administrator, or the account named by
    /// `as_user`; otherwise the response reports `E_PRIVILEGE`. The request
    /// runs in this process, whatever its `privilege`; its entry goes to the
    /// journal that this engine was opened with (the journal of the account
    /// that sent the request, see [`EngineConfig::with_account_journal`](crate::EngineConfig::with_account_journal)).
    /// `backend` names the backend that started the process.
    #[must_use]
    pub fn elevated_exec(&self, request: &Request, backend: Option<&str>) -> Response {
        let as_user = request.options.as_user.clone();
        let user = &self.inner.host.info().user;
        let allowed = self.inner.elevation.is_administrator()
            || as_user.as_deref().is_some_and(|name| name == user);
        if !allowed {
            let error = Error::new(
                ErrorCode::Privilege,
                "`elevated-exec` runs only with the identity of the administrator",
            );
            return respond_with(&failed_report(input_kind(request), error), &request.options);
        }
        let mark = Mark {
            backend: backend
                .and_then(Backend::from_name)
                .unwrap_or(Backend::SelfProcess),
            as_user,
            reason: None,
        };
        self.execute_local(request, &CancelToken::new(), None, Some(mark), None)
    }

    /// Runs the request through the backends in order.
    ///
    /// # Errors
    ///
    /// Returns `E_PRIVILEGE` listing the refusal of every backend, or
    /// `E_UNSUPPORTED` for `as_user` on Windows.
    pub(crate) fn run_elevated(
        &self,
        request: &Request,
        prepared: &Prepared,
        route: Route,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
        early: Option<Early>,
    ) -> Result<Response, Error> {
        let system = system_of(self.inner.host.info().os);
        let as_user = prepared.options.as_user.clone();
        if as_user.is_some() && system == System::Windows {
            return Err(as_user_unsupported());
        }
        let backends = choose_backends(
            backends_of(&prepared.config.config().privilege, system)?,
            route.only,
        )?;
        let reason = route.reason;
        let setup = &self.inner.elevation;
        let mut attempts: Vec<(Backend, Refusal)> = Vec::new();
        let mut payload: Option<(Request, Vec<Correction>, Option<String>)> = None;
        for backend in backends {
            if backend == Backend::SelfProcess {
                if as_user.is_some() {
                    attempts.push((backend, Refusal::CannotSwitchUser));
                } else if setup.is_administrator() {
                    let mark = Mark {
                        backend,
                        as_user: None,
                        reason,
                    };
                    return Ok(self.execute_local(request, cancel, events, Some(mark), early));
                } else {
                    attempts.push((backend, Refusal::NotAdministrator));
                }
                continue;
            }
            if payload.is_none() {
                match self.elevated_payload(request, prepared, early.as_ref()) {
                    Some(built) => payload = Some(built),
                    None if matches!(request.input(), Ok(RequestInput::Accept(_))) => {
                        return Err(Error::new(
                            ErrorCode::Internal,
                            "the program of the accepted candidate cannot be sent to the elevated process",
                        ));
                    }
                    None => return Ok(self.execute_local(request, cancel, events, None, early)),
                }
            }
            let Some((elevated, corrections, ran)) = &payload else {
                continue;
            };
            let bytes = self.envelope_bytes(elevated, backend, as_user.is_none());
            let executable = setup.executable()?;
            let limit = prepared.timeout.saturating_add(BACKEND_GRACE);
            match run_backend(
                backend,
                &executable,
                &bytes,
                as_user.as_deref(),
                setup.search_path.as_deref(),
                limit,
            ) {
                Ok(output) => match serde_json::from_slice::<Response>(&output) {
                    Ok(response) => {
                        let mark = Mark {
                            backend,
                            as_user: as_user.clone(),
                            reason,
                        };
                        return Ok(self.finish_elevated(
                            response,
                            &mark,
                            (corrections, ran),
                            &prepared.options,
                        ));
                    }
                    Err(error) => attempts.push((
                        backend,
                        Refusal::Failed(format!("the response is not valid: {error}")),
                    )),
                },
                Err(refusal) => attempts.push((backend, refusal)),
            }
        }
        Err(refused_error(&attempts, as_user.as_deref()))
    }

    /// Marks the reports of an elevated response, restores the corrections
    /// of the program that were made before it was sent and keeps the
    /// references.
    fn finish_elevated(
        &self,
        mut response: Response,
        mark: &Mark,
        corrected: (&[Correction], &Option<String>),
        options: &nurt_contracts::envelope::RequestOptions,
    ) -> Response {
        mark.apply(&mut response.report);
        for report in &mut response.reports {
            mark.apply(report);
        }
        if response.report.corrections.is_empty() && !corrected.0.is_empty() {
            response.report.corrections = corrected.0.to_vec();
            response.report.ran.clone_from(corrected.1);
        }
        self.keep_inverses(&mut response);
        if response.reports.is_empty() {
            response = respond_with(&response.report, options);
        }
        response
    }

    /// Keeps the inverse operations of the entries that the response names:
    /// the account process, not the elevated one, writes them to its state.
    fn keep_inverses(&self, response: &mut Response) {
        self.keep_inverse(&mut response.report);
        for report in &mut response.reports {
            self.keep_inverse(report);
        }
    }

    /// The request that the elevated process reads: the syntax tree of the
    /// program and the effective options, with `privilege` set to
    /// `elevated`. `None` when the program does not parse; the caller then
    /// runs it here to report the error.
    fn elevated_payload(
        &self,
        request: &Request,
        prepared: &Prepared,
        early: Option<&Early>,
    ) -> Option<(Request, Vec<Correction>, Option<String>)> {
        let (mut elevated, corrections, ran) = match request.input().ok()? {
            RequestInput::Program { text, variant } => {
                let corrected = self.correct(text, variant).ok()?;
                (
                    Request::ast(corrected.ast),
                    corrected.corrections,
                    corrected.ran,
                )
            }
            RequestInput::Ast(document) => (Request::ast(document.clone()), Vec::new(), None),
            RequestInput::Accept(_) => {
                let text = early?.as_ref().ok()?.ran.clone()?;
                let corrected = self.correct(&text, None).ok()?;
                (
                    Request::ast(corrected.ast),
                    corrected.corrections,
                    corrected.ran,
                )
            }
            _ => return None,
        };
        elevated.options = prepared.options.clone();
        elevated.options.privilege = Some(Privilege::Elevated);
        Some((elevated, corrections, ran))
    }

    /// What the elevated process reads: the request and, for a process that
    /// keeps the journal of this account, the owner. A process that runs as
    /// another account (`as_user`) keeps the journal of that account.
    fn envelope_bytes(&self, request: &Request, backend: Backend, own_journal: bool) -> Vec<u8> {
        let owner = if own_journal {
            self.account_owner(backend)
        } else {
            None
        };
        let envelope = ElevatedEnvelope {
            nurt_elevated: ENVELOPE_VERSION.to_owned(),
            owner,
            request: request.clone(),
        };
        serde_json::to_vec(&envelope).unwrap_or_default()
    }

    /// The journal of this account, as an owner for the elevated process.
    #[cfg(unix)]
    fn account_owner(&self, backend: Backend) -> Option<ElevatedOwner> {
        let state = self.inner.state.as_ref()?;
        let (uid, gid) = nurt_proc::elevation::effective_ids()?;
        Some(ElevatedOwner {
            uid,
            gid,
            state_dir: state.root().to_string_lossy().into_owned(),
            backend: backend.as_str().to_owned(),
        })
    }

    /// Windows runs the backend `self` with the same account, so the journal of
    /// the elevated process is the journal of the account.
    #[cfg(not(unix))]
    fn account_owner(&self, _backend: Backend) -> Option<ElevatedOwner> {
        None
    }

    /// Whether the program is `undo` or `redo` of an entry that an elevated
    /// operation wrote: such an entry is restored with the rights of the
    /// administrator, through a backend.
    pub(crate) fn is_elevated_entry(&self, bound: &BoundDocument) -> bool {
        let Some(id) = entry_id(bound) else {
            return false;
        };
        let record = |id: u64| {
            self.history(&nurt_contracts::journal::HistoryQuery {
                id: Some(nurt_contracts::journal::OpId(id)),
                ..nurt_contracts::journal::HistoryQuery::default()
            })
            .ok()
            .and_then(|found| found.into_iter().next())
        };
        let elevated = |action: &Option<String>| {
            action
                .as_deref()
                .is_some_and(|action| action.starts_with("elevated"))
        };
        record(id).is_some_and(|found| {
            elevated(&found.action)
                || found
                    .undo_of
                    .iter()
                    .any(|undone| record(*undone).is_some_and(|entry| elevated(&entry.action)))
        })
    }

    /// The backend that wrote the entry that the `undo` or `redo` of the
    /// request restores: the backend of the entry itself, or of the entry that
    /// it undid.
    fn recorded_backend(&self, request: &Request, prepared: &Prepared) -> Option<Backend> {
        let compiled = self.compile_early(request, prepared)?.ok()?;
        let id = entry_id(&compiled.bound)?;
        let record = |id: u64| {
            self.history(&nurt_contracts::journal::HistoryQuery {
                id: Some(nurt_contracts::journal::OpId(id)),
                ..nurt_contracts::journal::HistoryQuery::default()
            })
            .ok()
            .and_then(|found| found.into_iter().next())
        };
        let found = record(id)?;
        found
            .action
            .as_deref()
            .and_then(backend_of_action)
            .or_else(|| {
                found.undo_of.iter().find_map(|undone| {
                    record(*undone)
                        .and_then(|entry| entry.action.as_deref().and_then(backend_of_action))
                })
            })
    }

    /// The entry that a bare `undo` or `redo` (without a number or another
    /// selector) would restore, when an elevated operation wrote it.
    ///
    /// The choice is the one of the journal: for `undo` the newest available
    /// entry of the session that is not an undo, for `redo` the newest
    /// available undo entry of the session. An entry that is itself elevated,
    /// or an undo of an elevated entry, is the case this returns.
    pub(crate) fn bare_elevated_target(
        &self,
        bound: &BoundDocument,
        session: Option<&crate::session::SessionId>,
    ) -> Option<(&'static str, u64)> {
        let name = bare_stage(bound)?;
        let records = self
            .history(&nurt_contracts::journal::HistoryQuery::default())
            .ok()?;
        let by_id = |id: u64| records.iter().find(|record| record.id == id);
        let elevated = |action: &Option<String>| {
            action
                .as_deref()
                .is_some_and(|action| action.starts_with("elevated"))
        };
        let target = bare_choice(&records, name, session)?;
        let hit = elevated(&target.action)
            || target
                .undo_of
                .iter()
                .any(|undone| by_id(*undone).is_some_and(|entry| elevated(&entry.action)));
        hit.then_some((name, target.id))
    }

    /// The entry that a bare `undo` (without a number or another selector)
    /// restores, by the choice of the journal; `None` for every other
    /// program.
    pub(crate) fn bare_undo_target(
        &self,
        bound: &BoundDocument,
        session: Option<&crate::session::SessionId>,
    ) -> Option<u64> {
        if bare_stage(bound) != Some("undo") {
            return None;
        }
        let records = self
            .history(&nurt_contracts::journal::HistoryQuery::default())
            .ok()?;
        bare_choice(&records, "undo", session).map(|record| record.id)
    }

    /// The request that carries the call parameters written on the stages of
    /// the program, when they differ from the options of the call; a stage
    /// parameter wins over the option and the configured default.
    pub(crate) fn with_stage_call(
        &self,
        request: &Request,
        prepared: &Prepared,
        early: Option<&Early>,
    ) -> Result<Option<Request>, Error> {
        let Some(Ok(compiled)) = early else {
            return Ok(None);
        };
        let (privilege, as_user) = stage_call(&compiled.bound)?;
        let mut options = prepared.options.clone();
        if let Some(privilege) = privilege {
            options.privilege = Some(privilege);
        }
        if as_user.is_some() {
            options.as_user = as_user;
        }
        if options.privilege == prepared.options.privilege
            && options.as_user == prepared.options.as_user
        {
            return Ok(None);
        }
        let mut forced = request.clone();
        forced.options.privilege = options.privilege;
        forced.options.as_user = options.as_user;
        Ok(Some(forced))
    }

    /// Refuses a bare `undo` or `redo` whose target is an elevated entry with
    /// `E_PRIVILEGE` and the next step that names the entry.
    pub(crate) fn refuse_bare(
        &self,
        request: &Request,
        prepared: &Prepared,
        target: (&'static str, u64),
    ) -> Response {
        refusal(request, prepared, bare_refusal(target.0, target.1))
    }

    /// Runs the `undo` or `redo` of an elevated entry through the backends.
    pub(crate) fn undo_elevated(
        &self,
        request: &Request,
        prepared: &Prepared,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
    ) -> Response {
        if !prepared.config.config().privilege.allowed {
            return refusal(request, prepared, disabled_error());
        }
        let mut forced = prepared.clone();
        forced.options.privilege = Some(Privilege::Elevated);
        let route = Route {
            reason: Some(REASON_ENTRY),
            only: self.recorded_backend(request, prepared),
        };
        match self.run_elevated(request, &forced, route, cancel, events, None) {
            Ok(response) => response,
            Err(error) => refusal(request, prepared, error),
        }
    }
}

/// The call parameters written on the stages of a program: the first
/// `privilege` and the first `as_user`.
fn stage_call(bound: &BoundDocument) -> Result<(Option<Privilege>, Option<String>), Error> {
    let mut found: Vec<(Option<Privilege>, Option<String>)> = Vec::new();
    for statement in &bound.program.statements {
        let BoundStatement::Pipeline(pipeline) = statement else {
            continue;
        };
        let head = match &pipeline.head {
            BoundHead::Stage(stage) => Some(stage),
            _ => None,
        };
        let mut privilege = None;
        let mut as_user = None;
        for stage in head.into_iter().chain(pipeline.stages.iter()) {
            if privilege.is_none() {
                privilege = stage
                    .call_parameter("privilege")
                    .and_then(|word| Privilege::ALL.iter().copied().find(|v| v.as_str() == word));
            }
            if as_user.is_none() {
                as_user = stage.call_parameter("as_user").map(str::to_owned);
            }
        }
        found.push((privilege, as_user));
    }
    found.retain(|pair| pair.0.is_some() || pair.1.is_some());
    if found.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the pipelines of the program give different `privilege` or `as_user`; one call has one set of rights",
        )
        .with_next_step("give the same `privilege` and `as_user` on every pipeline, or run the pipelines as separate calls"));
    }
    Ok(found.into_iter().next().unwrap_or((None, None)))
}

/// Whether a report ended with a denial that left no change behind.
fn denied(response: &Response) -> bool {
    let report = &response.report;
    report.status == ReportStatus::Failed
        && report.op.is_none()
        && report.changes.is_empty()
        && response
            .reports
            .iter()
            .all(|each| each.op.is_none() && each.changes.is_empty())
        && report.error.as_ref().is_some_and(|error| {
            matches!(error.code(), ErrorCode::Permission | ErrorCode::Privilege)
        })
}

/// Response that reports `error` for a request that could not be elevated.
fn refusal(request: &Request, prepared: &Prepared, error: Error) -> Response {
    respond_with(
        &failed_report(input_kind(request), error),
        &prepared.options,
    )
}

/// The entry of `records`, newest first, that a bare `undo` or `redo` named
/// `name` restores: for `undo` the newest available entry of the session that
/// is not an undo, for `redo` the newest available undo entry of the session.
fn bare_choice<'r>(
    records: &'r [nurt_contracts::journal::OperationRecord],
    name: &str,
    session: Option<&crate::session::SessionId>,
) -> Option<&'r nurt_contracts::journal::OperationRecord> {
    records.iter().find(|record| {
        let available = matches!(
            record.state,
            nurt_contracts::record::OperationState::Available
                | nurt_contracts::record::OperationState::Partial
        );
        let undo = record.kind == "undo";
        let mine =
            session.is_none_or(|session| record.session.as_deref() == Some(session.as_str()));
        available && mine && (undo == (name == "redo"))
    })
}

/// `undo` or `redo` when the program has such a stage that names no entry and
/// no other selector.
fn bare_stage(bound: &BoundDocument) -> Option<&'static str> {
    const SELECTORS: [&str; 6] = ["id", "to", "file", "paths", "since", "all_sessions"];
    for statement in &bound.program.statements {
        let BoundStatement::Pipeline(pipeline) = statement else {
            continue;
        };
        let head = match &pipeline.head {
            BoundHead::Stage(stage) => Some(stage),
            _ => None,
        };
        for stage in head.into_iter().chain(pipeline.stages.iter()) {
            let name = match stage.name.as_str() {
                "undo" => "undo",
                "redo" => "redo",
                _ => continue,
            };
            let named = stage
                .args
                .iter()
                .any(|arg| !arg.default && SELECTORS.contains(&arg.name.as_str()));
            return (!named && entry_id(bound).is_none()).then_some(name);
        }
    }
    None
}

/// Refusal of a bare `undo` or `redo` whose target was written by an elevated
/// operation: the target is named so that the next call can address it.
fn bare_refusal(name: &str, target: u64) -> Error {
    Error::new(
        ErrorCode::Privilege,
        format!(
            "`{name}` without a number would restore operation {target}, which an elevated operation wrote; it is restored with the rights of the administrator, through a backend"
        ),
    )
    .with_next_step(format!("{name} {target} privilege: elevated"))
}

/// The `id` of the first `undo` or `redo` stage of the program.
pub(crate) fn entry_id(bound: &BoundDocument) -> Option<u64> {
    for statement in &bound.program.statements {
        let BoundStatement::Pipeline(pipeline) = statement else {
            continue;
        };
        let head = match &pipeline.head {
            BoundHead::Stage(stage) => Some(stage),
            _ => None,
        };
        for stage in head.into_iter().chain(pipeline.stages.iter()) {
            if stage.name != "undo" && stage.name != "redo" {
                continue;
            }
            let arg = stage.args.iter().find(|arg| arg.name == "id")?;
            return match &arg.value {
                BoundExpr::Int { v } => u64::try_from(*v).ok(),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::record::{Change, ChangeAction};
    use nurt_contracts::report::Report;

    fn denied_response() -> Response {
        let mut report = Report::new(ReportStatus::Failed, "write");
        report.error = Some(Error::new(ErrorCode::Permission, "denied"));
        Response::new(report)
    }

    #[test]
    fn an_entry_is_restored_through_the_backend_that_wrote_it() {
        let configured = vec![Backend::Doas, Backend::Sudo, Backend::Run0];
        assert_eq!(
            choose_backends(configured.clone(), None).unwrap(),
            configured
        );
        assert_eq!(
            choose_backends(configured, Some(Backend::Sudo)).unwrap(),
            vec![Backend::Sudo]
        );
    }

    #[test]
    fn a_backend_that_the_configuration_does_not_allow_is_named_in_the_refusal() {
        let error = choose_backends(vec![Backend::Doas], Some(Backend::Sudo)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Privilege);
        assert!(error.data().message.contains("`sudo`"), "{error:?}");
        assert!(
            error
                .data()
                .next_steps
                .iter()
                .any(|step| step.contains("`sudo`"))
        );
        let listed = error.data().details.get("backend").cloned().unwrap();
        assert_eq!(
            listed,
            serde_json::json!([{"backend": "sudo", "result": "not_allowed"}])
        );
    }

    #[test]
    fn the_action_of_an_entry_names_its_backend() {
        assert_eq!(backend_of_action("elevated:sudo"), Some(Backend::Sudo));
        assert_eq!(
            backend_of_action("elevated:self"),
            Some(Backend::SelfProcess)
        );
        assert_eq!(backend_of_action("elevated:ssh"), None);
        assert_eq!(backend_of_action("sudo"), None);
    }

    #[test]
    fn a_denial_without_a_change_may_be_repeated() {
        assert!(denied(&denied_response()));
    }

    #[test]
    fn a_denial_after_a_change_is_never_repeated() {
        let mut response = denied_response();
        response
            .report
            .changes
            .push(Change::new("/srv/a", ChangeAction::Modify));
        assert!(!denied(&response));
        let mut other = denied_response();
        let mut second = Report::new(ReportStatus::Failed, "write");
        second.op = Some(3);
        other.reports.push(second);
        assert!(!denied(&other));
    }

    #[test]
    fn a_denial_with_an_entry_is_never_repeated() {
        let mut response = denied_response();
        response.report.op = Some(2);
        assert!(!denied(&response));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
