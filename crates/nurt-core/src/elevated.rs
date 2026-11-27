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
use nurt_contracts::journal::{HistoryQuery, OpId};
use nurt_contracts::report::{Correction, Report, ReportStatus};
use nurt_contracts::stage::{CancelToken, EventSink};
use nurt_proc::elevation::{Backend, Refusal, System, run_backend};

use crate::elevation::{
    ElevatedReference, Mark, REASON_AUTO, REASON_REFERENCE, as_user_unsupported, backends_of,
    disabled_error, refused_error, system_of, system_state_dir,
};
use crate::engine::{Engine, Prepared, failed_report, input_kind, request_config, respond_with};

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
        let Ok(prepared) = self.prepare(request) else {
            return self.execute_local(request, cancel, events, None);
        };
        if let Some(forced) = self.with_stage_call(request, &prepared) {
            return self.route(&forced, cancel, events);
        }
        let privilege = prepared.options.effective_privilege();
        let pages = matches!(
            request.input(),
            Ok(RequestInput::Cursor(_) | RequestInput::Accept(_))
        );
        if privilege == Privilege::Account || pages {
            return self.execute_local(request, cancel, events, None);
        }
        if !prepared.config.config().privilege.allowed {
            return refusal(request, &prepared, disabled_error());
        }
        if privilege == Privilege::Auto {
            let first = self.execute_local(request, cancel, events.clone(), None);
            if !denied(&first.report) {
                return first;
            }
            return match self.run_elevated(request, &prepared, Some(REASON_AUTO), cancel, events) {
                Ok(response) => response,
                Err(error) => {
                    let cause = first.report.error.clone();
                    let error = cause.map_or(error.clone(), |cause| error.with_cause(cause));
                    refusal(request, &prepared, error)
                }
            };
        }
        match self.run_elevated(request, &prepared, None, cancel, events) {
            Ok(response) => response,
            Err(error) => refusal(request, &prepared, error),
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
        self.execute_local(request, &CancelToken::new(), None, Some(mark))
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
                    return Ok(self.execute_local(request, cancel, events, Some(mark)));
                } else {
                    attempts.push((backend, Refusal::NotAdministrator));
                }
                continue;
            }
            if payload.is_none() {
                match self.elevated_payload(request, prepared) {
                    Some(built) => payload = Some(built),
                    None => return Ok(self.execute_local(request, cancel, events, None)),
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
            journal: self.system_state_dir().to_string_lossy().into_owned(),
        };
        let _ = log.append(&reference);
    }

    /// The reference that an `undo` of the program names, when the entry is
    /// one of the system journal and the journal of the account has no entry
    /// with that number.
    pub(crate) fn elevated_entry_of(&self, bound: &BoundDocument) -> Option<ElevatedReference> {
        let log = self.inner.references.as_ref()?;
        let id = undo_id(bound)?;
        let reference = log.find(id)?;
        let local = self.history(&HistoryQuery {
            id: Some(OpId(id)),
            ..HistoryQuery::default()
        });
        match local {
            Ok(records) if records.is_empty() => Some(reference),
            Err(_) => Some(reference),
            Ok(_) => None,
        }
    }

    /// The request that carries the call parameters written on the stages of
    /// the program, when they differ from the options of the call; a stage
    /// parameter wins over the option and the configured default.
    pub(crate) fn with_stage_call(
        &self,
        request: &Request,
        prepared: &Prepared,
    ) -> Option<Request> {
        let config = request_config(prepared);
        let compiled = match request.input().ok()? {
            RequestInput::Program { text, variant } => {
                self.inner.frontend.compile(text, variant, &config).ok()?
            }
            RequestInput::Ast(document) => self.bind(document, prepared).ok()?,
            _ => return None,
        };
        let (privilege, as_user) = stage_call(&compiled.bound);
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
            return None;
        }
        let mut forced = request.clone();
        forced.options.privilege = options.privilege;
        forced.options.as_user = options.as_user;
        Some(forced)
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
        match self.run_elevated(request, &forced, Some(REASON_REFERENCE), cancel, events) {
            Ok(response) => response,
            Err(error) => refusal(request, prepared, error),
        }
    }
}

/// The call parameters written on the stages of a program: the first
/// `privilege` and the first `as_user`.
fn stage_call(bound: &BoundDocument) -> (Option<Privilege>, Option<String>) {
    let mut privilege = None;
    let mut as_user = None;
    for statement in &bound.program.statements {
        let BoundStatement::Pipeline(pipeline) = statement else {
            continue;
        };
        let head = match &pipeline.head {
            BoundHead::Stage(stage) => Some(stage),
            _ => None,
        };
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
    }
    (privilege, as_user)
}

/// Whether a report ended with a denial that left no change behind.
fn denied(report: &Report) -> bool {
    report.status == ReportStatus::Failed
        && report.op.is_none()
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
