// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
use nurt_contracts::report::{Correction, Report, ReportStatus};
use nurt_contracts::stage::{CancelToken, EventSink};
use nurt_proc::elevation::{Backend, Refusal, System, run_backend};

use crate::elevation::{
    ELEVATED_OP_BASE, ElevatedReference, Mark, REASON_AUTO, REASON_REFERENCE, as_user_unsupported,
    backends_of, disabled_error, refused_error, system_of, system_state_dir,
};
use crate::engine::{Engine, Prepared, failed_report, input_kind, respond_with};
use crate::frontend::Compiled;

/// Result of the compilation of a request, made once by [`Engine::route`] and
/// reused by the run in this process.
pub(crate) type Early = Result<Compiled, Error>;

/// Time allowed to a backend beyond the time limit of the request.
const BACKEND_GRACE: Duration = Duration::from_secs(30);

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
                Some(REASON_AUTO),
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
        match self.run_elevated(request, &prepared, None, cancel, events, early) {
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
    /// runs in this process, whatever its `privilege`, and its journal entry
    /// goes to the journal of this process.
    #[must_use]
    pub fn elevated_exec(&self, request: &Request) -> Response {
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
            backend: Backend::SelfProcess,
            as_user,
            reason: None,
        };
        self.execute_local(request, &CancelToken::new(), None, Some(mark), None)
    }

    /// The references that this account keeps to entries of the system
    /// journal, oldest first.
    #[must_use]
    pub fn elevated_references(&self) -> Vec<ElevatedReference> {
        self.inner
            .references
            .as_ref()
            .map(crate::elevation::ReferenceLog::all)
            .unwrap_or_default()
    }

    /// The system state directory for the host of this engine.
    #[must_use]
    pub fn system_state_dir(&self) -> std::path::PathBuf {
        let program_data = self
            .inner
            .host
            .env_var("ProgramData")
            .map(|value| value.to_string_lossy().into_owned());
        system_state_dir(self.inner.host.info().os, program_data.as_deref())
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
        reason: Option<&'static str>,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
        early: Option<Early>,
    ) -> Result<Response, Error> {
        let system = system_of(self.inner.host.info().os);
        let as_user = prepared.options.as_user.clone();
        if as_user.is_some() && system == System::Windows {
            return Err(as_user_unsupported());
        }
        let backends = backends_of(&prepared.config.config().privilege, system)?;
        let setup = &self.inner.elevation;
        let mut attempts: Vec<(Backend, Refusal)> = Vec::new();
        let mut payload: Option<(Vec<u8>, Vec<Correction>, Option<String>)> = None;
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
                    None => return Ok(self.execute_local(request, cancel, events, None, early)),
                }
            }
            let Some((bytes, corrections, ran)) = &payload else {
                continue;
            };
            let executable = setup.executable()?;
            let limit = prepared.timeout.saturating_add(BACKEND_GRACE);
            match run_backend(
                backend,
                &executable,
                bytes,
                as_user.as_deref(),
                setup.search_path.as_deref(),
                limit,
            ) {
                Ok(output) => match serde_json::from_slice::<Response>(&output) {
                    Ok(mut response) => {
                        let mark = Mark {
                            backend,
                            as_user: as_user.clone(),
                            reason,
                        };
                        mark.apply(&mut response.report);
                        for report in &mut response.reports {
                            mark.apply(report);
                        }
                        if response.report.corrections.is_empty() && !corrections.is_empty() {
                            response.report.corrections.clone_from(corrections);
                            response.report.ran.clone_from(ran);
                        }
                        self.remember(&response.report, &mark);
                        if let Some(op) = response.report.op {
                            response.report.op = Some(ELEVATED_OP_BASE + op);
                        }
                        if response.reports.is_empty() {
                            response = respond_with(&response.report, &prepared.options);
                        }
                        return Ok(response);
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

    /// The request that the elevated process reads: the syntax tree of the
    /// program and the effective options, with `privilege` set to
    /// `elevated`. `None` when the program does not parse; the caller then
    /// runs it here to report the error.
    fn elevated_payload(
        &self,
        request: &Request,
        prepared: &Prepared,
        early: Option<&Early>,
    ) -> Option<(Vec<u8>, Vec<Correction>, Option<String>)> {
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
        let bytes = serde_json::to_vec(&elevated).ok()?;
        Some((bytes, corrections, ran))
    }

    /// Keeps the reference to the entry of the system journal that an
    /// elevated operation wrote.
    fn remember(&self, report: &Report, mark: &Mark) {
        let (Some(op), Some(log)) = (report.op, &self.inner.references) else {
            return;
        };
        if report.status == ReportStatus::Planned {
            return;
        }
        let reference = ElevatedReference {
            op,
            backend: mark.backend.as_str().to_owned(),
            as_user: mark.as_user.clone(),
            kind: report.kind.clone(),
            time_ms: i64::try_from(self.now().unix_nanos() / 1_000_000).unwrap_or(0),
            journal: match &mark.as_user {
                Some(user) => format!("account:{user}"),
                None => self.system_state_dir().to_string_lossy().into_owned(),
            },
        };
        let _ = log.append(&reference);
    }

    /// The reference that an `undo` of the program names, when the entry is
    /// one of the system journal and the journal of the account has no entry
    /// with that number.
    pub(crate) fn elevated_entry_of(&self, bound: &BoundDocument) -> Option<ElevatedReference> {
        let log = self.inner.references.as_ref()?;
        let id = undo_id(bound)?;
        log.find(id.checked_sub(ELEVATED_OP_BASE)?)
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

    /// Runs the `undo` of an elevated entry through the backends.
    pub(crate) fn undo_elevated(
        &self,
        request: &Request,
        prepared: &Prepared,
        reference: &ElevatedReference,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
    ) -> Response {
        if !prepared.config.config().privilege.allowed {
            return refusal(request, prepared, disabled_error());
        }
        let inner = Request::program(format!("undo {}", reference.op))
            .with_options(request.options.clone());
        let mut options = prepared.options.clone();
        options.privilege = Some(Privilege::Elevated);
        options.as_user.clone_from(&reference.as_user);
        let forced = Prepared {
            config: Arc::clone(&prepared.config),
            options,
            session: prepared.session.clone(),
            timeout: prepared.timeout,
            elevation: None,
        };
        match self.run_elevated(
            &inner,
            &forced,
            Some(REASON_REFERENCE),
            cancel,
            events,
            None,
        ) {
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

/// The `id` of the first `undo` stage of the program.
pub(crate) fn undo_id(bound: &BoundDocument) -> Option<u64> {
    for statement in &bound.program.statements {
        let BoundStatement::Pipeline(pipeline) = statement else {
            continue;
        };
        let head = match &pipeline.head {
            BoundHead::Stage(stage) => Some(stage),
            _ => None,
        };
        for stage in head.into_iter().chain(pipeline.stages.iter()) {
            if stage.name != "undo" {
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

    fn denied_response() -> Response {
        let mut report = Report::new(ReportStatus::Failed, "write");
        report.error = Some(Error::new(ErrorCode::Permission, "denied"));
        Response::new(report)
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
