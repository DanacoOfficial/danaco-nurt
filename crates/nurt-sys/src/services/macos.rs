// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::services::macos
// Responsibility:   Launchd backend.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! launchd backend.
//!
//! Definitions of services are read from property lists in the launchd
//! directories of the scope. The state of services and every change go
//! through `launchctl`, the component of macOS that offers the only access
//! to launchd; there is no public programming interface. The program is run
//! through [`ProcessHost`] with its arguments as a list, without a shell and
//! with a time limit of five seconds per call, and the service commands
//! declare this use in the catalog (`external`, kind `system_program`).
//!
//! System services live in the domain `system`; services of the current
//! account in `gui/<uid>`, or in `user/<uid>` when the account has no
//! graphical session (for example over SSH).
//!
//! | Action | Calls of `launchctl` |
//! |---|---|
//! | `start` | `bootstrap <domain> <plist>` when the service is not loaded, then `kickstart <domain>/<label>` |
//! | `restart` | as `start`, with `kickstart -k` |
//! | `stop` | `kill SIGTERM <domain>/<label>`; for a service with `KeepAlive`, `bootout <domain>/<label>`, because launchd would start it again |
//! | `enable` | `enable <domain>/<label>`, then `bootstrap` when not loaded |
//! | `disable` | `disable <domain>/<label>`, then `bootout` when loaded |
//!
//! A service stopped through `bootout` is unloaded and is loaded again by
//! the next `start`.

mod definitions;
mod launchctl;

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Service, ServiceManager, ServiceScope, ServiceState};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::ProgramSpec;
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{Fields, Path, Value};

use self::definitions::{Definition, Layout, PROTECTED_PREFIX};
use self::launchctl::{LastExit, ListedService, ServiceDetail};
use super::{
    ServiceBackend, ServiceQuery, ServiceStatus, service_error, service_not_found,
    service_privilege,
};

const MANAGER: ServiceManager = ServiceManager::Launchd;

/// Location of `launchctl` on every macOS release.
const LAUNCHCTL: &str = "/bin/launchctl";
/// Name of the program in errors.
const PROGRAM_NAME: &str = "launchctl";
/// Working directory of the calls.
const WORKING_DIRECTORY: &str = "/";
/// Time limit of one call, in milliseconds.
const CALL_TIMEOUT_MS: u64 = 5_000;
/// Bytes kept from each output stream; the listing of a domain is a few
/// hundred kilobytes.
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// Further attempts of a call that found another operation in progress.
const RETRY_LIMIT: u32 = 3;
/// Pause before a further attempt.
const RETRY_DELAY: Duration = Duration::from_millis(200);

/// Domain of system services.
const SYSTEM_DOMAIN: &str = "system";

/// `ESRCH`: the service has no process or is not loaded.
const EXIT_NO_PROCESS: i32 = 3;
/// `EIO`: returned by `bootstrap` for a service that is already loaded.
const EXIT_ALREADY_LOADED: i32 = 5;
/// `EPERM`: the operation is not permitted.
const EXIT_NOT_PERMITTED: i32 = 1;
/// `EINPROGRESS`: another operation on the service is in progress.
const EXIT_IN_PROGRESS: i32 = 37;
/// Could not find the service in the domain.
const EXIT_NOT_FOUND: i32 = 113;
/// Operation not permitted by System Integrity Protection.
const EXIT_PROTECTED: i32 = 150;

/// Message part of a denied operation.
const NOT_PERMITTED: &str = "Operation not permitted";
/// Message part of an operation denied by System Integrity Protection.
const PROTECTION_MESSAGE: &str = "System Integrity Protection";

/// Raw state of a service whose property list is not loaded.
const STATE_NOT_LOADED: &str = "not loaded";

/// Creates the launchd backend of the execution context.
#[cfg(target_os = "macos")]
pub(crate) fn backend(ctx: &ExecCtx) -> Box<dyn ServiceBackend + '_> {
    let layout = Layout {
        root: std::path::PathBuf::from("/"),
        home: ctx.host().info().home.to_string_lossy().into_owned(),
        uid: rustix::process::getuid().as_raw(),
    };
    Box::new(LaunchdBackend::new(ctx, layout))
}

/// Kind of `launchctl` call, for the interpretation of exit codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Print,
    Bootstrap,
    Bootout,
    Kickstart,
    Kill,
    Enable,
    Disable,
}

/// Captured result of one call.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Output {
    fn message(&self) -> &str {
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            self.stdout.trim()
        } else {
            stderr
        }
    }
}

/// Service a call acts on.
#[derive(Debug, Clone, Copy)]
struct Target<'t> {
    domain: &'t str,
    label: &'t str,
    path: Option<&'t str>,
}

fn external_missing() -> Error {
    Error::new(
        ErrorCode::ExternalMissing,
        format!(
            "{PROGRAM_NAME} is not available at {LAUNCHCTL}; services on macOS are managed through this component of the operating system"
        ),
    )
    .with_detail("program", PROGRAM_NAME.into())
}

fn denied(output: &Output, target: &Target<'_>) -> Error {
    let message = output.message();
    let protected = message.contains(PROTECTION_MESSAGE)
        || target
            .path
            .is_some_and(|path| path.starts_with(PROTECTED_PREFIX));
    if protected {
        return Error::new(
            ErrorCode::Permission,
            format!(
                "service `{}` belongs to macOS and is protected by System Integrity Protection; it cannot be changed",
                target.label
            ),
        )
        .with_detail("manager", MANAGER.as_str().into())
        .with_detail("unit", target.label.into())
        .with_detail("os_message", message.into());
    }
    if target.domain == SYSTEM_DOMAIN {
        return service_privilege(MANAGER, target.label, "root");
    }
    Error::new(
        ErrorCode::Permission,
        format!(
            "launchd denied the change of service `{}`: {message}",
            target.label
        ),
    )
    .with_detail("manager", MANAGER.as_str().into())
    .with_detail("unit", target.label.into())
    .with_detail("os_message", message.into())
}

/// Maps the exit code of a call to the result of the action.
fn classify(output: &Output, action: Action, target: &Target<'_>) -> Result<()> {
    match output.code {
        0 => Ok(()),
        EXIT_NO_PROCESS if matches!(action, Action::Bootout | Action::Kill) => Ok(()),
        EXIT_NOT_FOUND => Err(service_not_found(MANAGER, target.label)),
        EXIT_NOT_PERMITTED | EXIT_PROTECTED if output.message().contains(NOT_PERMITTED) => {
            Err(denied(output, target))
        }
        code => Err(service_error(
            MANAGER,
            target.label,
            &code.to_string(),
            output.message(),
        )),
    }
}

fn check_label(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && !name.contains('/')
        && !name.chars().any(|ch| ch.is_whitespace() || ch.is_control());
    if valid {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::InvalidValue,
        format!("`{name}` is not a launchd label; give a label such as com.example.agent"),
    )
    .with_found(name)
    .with_expected("label without `/` and spaces".into()))
}

/// State of a service as observed through `launchctl`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
    state: ServiceState,
    state_raw: Option<String>,
    pid: Option<u64>,
    last_exit: Option<LastExit>,
    loaded: bool,
    path: Option<String>,
    program: Option<String>,
}

impl Observed {
    fn not_loaded() -> Self {
        Self {
            state: ServiceState::Stopped,
            state_raw: Some(STATE_NOT_LOADED.to_owned()),
            pid: None,
            last_exit: None,
            loaded: false,
            path: None,
            program: None,
        }
    }

    fn unknown() -> Self {
        Self {
            state: ServiceState::Unknown,
            state_raw: None,
            ..Self::not_loaded()
        }
    }

    fn listed(service: &ListedService) -> Self {
        let (state, raw) = launchctl::listed_state(service);
        Self {
            state,
            state_raw: Some(raw.to_owned()),
            pid: service.pid,
            last_exit: Some(service.last_exit.clone()),
            loaded: true,
            path: None,
            program: None,
        }
    }

    fn detail(detail: ServiceDetail) -> Self {
        let state = detail
            .state_raw
            .as_deref()
            .map_or(ServiceState::Unknown, |raw| {
                launchctl::map_state(raw, &detail.last_exit)
            });
        Self {
            state,
            state_raw: detail.state_raw,
            pid: detail.pid,
            last_exit: Some(detail.last_exit),
            loaded: true,
            path: detail.path,
            program: detail.program,
        }
    }
}

fn raw_attributes(definition: Option<&Definition>, observed: &Observed, domain: &str) -> Fields {
    let mut raw = Fields::new();
    raw.insert("domain".to_owned(), Value::Text(domain.to_owned()));
    raw.insert("loaded".to_owned(), Value::Bool(observed.loaded));
    match &observed.last_exit {
        Some(LastExit::Code(code)) => {
            raw.insert("last_exit_code".to_owned(), Value::Int(*code));
        }
        Some(LastExit::Signal(signal)) => {
            raw.insert("last_signal".to_owned(), Value::Text(signal.clone()));
        }
        _ => {}
    }
    let program = definition
        .and_then(|known| known.program.clone())
        .or_else(|| observed.program.clone());
    if let Some(program) = program {
        raw.insert("program".to_owned(), Value::Text(program));
    }
    if let Some(known) = definition {
        raw.insert(
            "arguments".to_owned(),
            Value::List(known.arguments.iter().cloned().map(Value::Text).collect()),
        );
        raw.insert("run_at_load".to_owned(), Value::Bool(known.run_at_load));
        raw.insert("keep_alive".to_owned(), Value::Bool(known.keep_alive));
    }
    raw
}

fn record(
    label: &str,
    scope: ServiceScope,
    definition: Option<&Definition>,
    observed: Observed,
    disabled: Option<bool>,
    domain: &str,
) -> Service {
    let os_raw = raw_attributes(definition, &observed, domain);
    Service {
        name: label.to_owned(),
        state: observed.state,
        start_type: definitions::start_type(definition, disabled),
        manager: MANAGER,
        display_name: None,
        state_raw: observed.state_raw,
        pid: observed.pid,
        description: None,
        scope: Some(scope),
        path: definition
            .map(|known| known.path.clone())
            .or(observed.path)
            .map(Path::from),
        account: definition.and_then(|known| known.user_name.clone()),
        os_raw: Some(os_raw),
    }
}

/// Backend over `launchctl` and the property lists below a system root.
struct LaunchdBackend<'a> {
    ctx: &'a ExecCtx,
    layout: Layout,
    user_domain: OnceLock<String>,
}

impl<'a> LaunchdBackend<'a> {
    fn new(ctx: &'a ExecCtx, layout: Layout) -> Self {
        Self {
            ctx,
            layout,
            user_domain: OnceLock::new(),
        }
    }

    fn run(&self, args: &[&str]) -> Result<Output> {
        let process = self.ctx.process().ok_or_else(external_missing)?;
        let mut spec = ProgramSpec::new(
            LAUNCHCTL,
            args.iter().map(|arg| (*arg).to_owned()).collect(),
            WORKING_DIRECTORY,
        );
        spec.timeout_ms = Some(CALL_TIMEOUT_MS);
        spec.output_limit = Some(OUTPUT_LIMIT);
        let mut attempt = 0;
        loop {
            self.ctx.checkpoint()?;
            let outcome =
                process
                    .run_program(&spec, self.ctx.cancel())
                    .map_err(|error| match error.code() {
                        ErrorCode::ExternalMissing
                        | ErrorCode::ProgramNotFound
                        | ErrorCode::Spawn => external_missing(),
                        _ => error,
                    })?;
            if outcome.cancelled {
                self.ctx.checkpoint()?;
                return Err(Error::new(
                    ErrorCode::Cancelled,
                    "the launchctl call was cancelled",
                ));
            }
            if outcome.timed_out {
                return Err(Error::new(
                    ErrorCode::Timeout,
                    format!(
                        "launchctl {} did not finish within {} ms",
                        args.join(" "),
                        CALL_TIMEOUT_MS
                    ),
                )
                .with_detail("program", PROGRAM_NAME.into()));
            }
            let Some(code) = outcome.exit_code else {
                let signal = outcome.signal.unwrap_or_default();
                return Err(service_error(
                    MANAGER,
                    args.last().copied().unwrap_or_default(),
                    &format!("signal {signal}"),
                    "launchctl was ended by a signal",
                ));
            };
            if code == EXIT_IN_PROGRESS && attempt < RETRY_LIMIT {
                attempt += 1;
                self.ctx.sleep(RETRY_DELAY)?;
                continue;
            }
            return Ok(Output {
                code,
                stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
            });
        }
    }

    fn call(&self, args: &[&str], action: Action, target: &Target<'_>) -> Result<Output> {
        let output = self.run(args)?;
        classify(&output, action, target)?;
        Ok(output)
    }

    /// Finds the domain of the user scope and returns its listing: `gui/<uid>`
    /// when the account has a graphical session, `user/<uid>` otherwise.
    fn probe_user_domain(&self) -> Result<(String, Output)> {
        let gui = format!("gui/{}", self.layout.uid);
        let output = self.run(&["print", &gui])?;
        let (domain, output) = if output.code == 0 {
            (gui, output)
        } else {
            let user = format!("user/{}", self.layout.uid);
            let target = Target {
                domain: &user,
                label: &user,
                path: None,
            };
            let output = self.call(&["print", &user], Action::Print, &target)?;
            (user, output)
        };
        Ok((self.user_domain.get_or_init(|| domain).clone(), output))
    }

    fn domain(&self, scope: ServiceScope) -> Result<String> {
        match scope {
            ServiceScope::User => match self.user_domain.get() {
                Some(domain) => Ok(domain.clone()),
                None => self.probe_user_domain().map(|(domain, _)| domain),
            },
            _ => Ok(SYSTEM_DOMAIN.to_owned()),
        }
    }

    fn listing(&self, scope: ServiceScope) -> Result<(String, Output)> {
        if scope == ServiceScope::User && self.user_domain.get().is_none() {
            return self.probe_user_domain();
        }
        let domain = self.domain(scope)?;
        let target = Target {
            domain: &domain,
            label: &domain,
            path: None,
        };
        let output = self.call(&["print", &domain], Action::Print, &target)?;
        Ok((domain, output))
    }

    fn overrides(&self, domain: &str) -> Result<BTreeMap<String, bool>> {
        let target = Target {
            domain,
            label: domain,
            path: None,
        };
        let output = self.call(&["print-disabled", domain], Action::Print, &target)?;
        Ok(launchctl::parse_disabled(&output.stdout))
    }

    /// Details of a loaded service; `None` when the service is not loaded.
    fn print_service(&self, domain: &str, label: &str) -> Result<Option<ServiceDetail>> {
        let service = format!("{domain}/{label}");
        let output = self.run(&["print", &service])?;
        if output.code == EXIT_NOT_FOUND {
            return Ok(None);
        }
        let target = Target {
            domain,
            label,
            path: None,
        };
        classify(&output, Action::Print, &target)?;
        Ok(Some(launchctl::parse_service(&output.stdout)))
    }

    fn find(&self, label: &str, scope: ServiceScope) -> Option<Definition> {
        definitions::find(&self.layout, scope, label, self.ctx)
    }

    fn require_definition(&self, label: &str, scope: ServiceScope) -> Result<Definition> {
        self.find(label, scope)
            .ok_or_else(|| service_not_found(MANAGER, label))
    }

    fn bootstrap(&self, domain: &str, label: &str, path: &str) -> Result<()> {
        let target = Target {
            domain,
            label,
            path: Some(path),
        };
        let output = self.run(&["bootstrap", domain, path])?;
        // Exit code 5 is the answer for a service that is already loaded and
        // also the generic input/output error of a refusal, so only a service
        // that `print` finds loaded counts as bootstrapped.
        if output.code == EXIT_ALREADY_LOADED && self.print_service(domain, label)?.is_some() {
            return Ok(());
        }
        classify(&output, Action::Bootstrap, &target)
    }

    fn launch(&self, name: &str, scope: ServiceScope, kickstart: &[&str]) -> Result<()> {
        check_label(name)?;
        let domain = self.domain(scope)?;
        let path = match self.print_service(&domain, name)? {
            Some(detail) => detail.path,
            None => {
                let definition = self.require_definition(name, scope)?;
                self.bootstrap(&domain, name, &definition.path)?;
                Some(definition.path)
            }
        };
        let service = format!("{domain}/{name}");
        let mut args = kickstart.to_vec();
        args.push(&service);
        let target = Target {
            domain: &domain,
            label: name,
            path: path.as_deref(),
        };
        self.call(&args, Action::Kickstart, &target).map(drop)
    }

    fn note_listing(&self, domain: &str, listing: &launchctl::DomainListing) {
        if listing.services.is_none() {
            self.ctx.note(NotableEntry {
                detail: Some(format!(
                    "launchctl print {domain} has no services block; service states are unknown"
                )),
                ..NotableEntry::new(Notable::SourceUnavailable)
            });
        } else if listing.skipped > 0 {
            self.ctx.note(NotableEntry {
                count: u64::try_from(listing.skipped).ok(),
                detail: Some(format!(
                    "lines of launchctl print {domain} were not recognized and were skipped"
                )),
                ..NotableEntry::new(Notable::SourceUnavailable)
            });
        }
    }
}

impl ServiceBackend for LaunchdBackend<'_> {
    fn manager(&self) -> ServiceManager {
        MANAGER
    }

    fn list(&self, query: &ServiceQuery) -> Result<Vec<Service>> {
        let scope = query.scope;
        let found = definitions::read_all(&self.layout, scope, self.ctx);
        let (domain, output) = self.listing(scope)?;
        let listing = launchctl::parse_domain(&output.stdout);
        self.note_listing(&domain, &listing);
        let overrides = self.overrides(&domain)?;
        let listed: BTreeMap<&str, &ListedService> = listing
            .services
            .iter()
            .flatten()
            .map(|service| (service.label.as_str(), service))
            .collect();
        let mut services: BTreeMap<String, Service> = BTreeMap::new();
        for definition in &found {
            let observed = match (&listing.services, listed.get(definition.label.as_str())) {
                (None, _) => Observed::unknown(),
                (Some(_), Some(service)) => Observed::listed(service),
                (Some(_), None) => Observed::not_loaded(),
            };
            let disabled = overrides.get(&definition.label).copied();
            services.insert(
                definition.label.clone(),
                record(
                    &definition.label,
                    scope,
                    Some(definition),
                    observed,
                    disabled,
                    &domain,
                ),
            );
        }
        for (label, service) in listed {
            if !services.contains_key(label) {
                let disabled = overrides.get(label).copied();
                let observed = Observed::listed(service);
                services.insert(
                    label.to_owned(),
                    record(label, scope, None, observed, disabled, &domain),
                );
            }
        }
        Ok(services.into_values().collect())
    }

    fn get(&self, name: &str, scope: ServiceScope) -> Result<Service> {
        check_label(name)?;
        let domain = self.domain(scope)?;
        let definition = self.find(name, scope);
        let observed = match self.print_service(&domain, name)? {
            Some(detail) => Observed::detail(detail),
            None if definition.is_some() => Observed::not_loaded(),
            None => return Err(service_not_found(MANAGER, name)),
        };
        let disabled = self.overrides(&domain)?.get(name).copied();
        Ok(record(
            name,
            scope,
            definition.as_ref(),
            observed,
            disabled,
            &domain,
        ))
    }

    fn status(&self, name: &str, scope: ServiceScope) -> Result<ServiceStatus> {
        check_label(name)?;
        let domain = self.domain(scope)?;
        match self.print_service(&domain, name)? {
            Some(detail) => {
                let result = detail.last_exit.describe();
                let observed = Observed::detail(detail);
                Ok(ServiceStatus {
                    state: observed.state,
                    state_raw: observed.state_raw,
                    result,
                })
            }
            None => {
                self.require_definition(name, scope)?;
                Ok(ServiceStatus {
                    state: ServiceState::Stopped,
                    state_raw: Some(STATE_NOT_LOADED.to_owned()),
                    result: None,
                })
            }
        }
    }

    fn start(&self, name: &str, scope: ServiceScope) -> Result<()> {
        self.launch(name, scope, &["kickstart"])
    }

    fn stop(&self, name: &str, scope: ServiceScope) -> Result<()> {
        check_label(name)?;
        let domain = self.domain(scope)?;
        let definition = self.find(name, scope);
        let service = format!("{domain}/{name}");
        let (action, args) = if definition.as_ref().is_some_and(|known| known.keep_alive) {
            (Action::Bootout, vec!["bootout", service.as_str()])
        } else {
            (Action::Kill, vec!["kill", "SIGTERM", service.as_str()])
        };
        let output = self.run(&args)?;
        if output.code == EXIT_NOT_FOUND && definition.is_some() {
            return Ok(());
        }
        let target = Target {
            domain: &domain,
            label: name,
            path: definition.as_ref().map(|known| known.path.as_str()),
        };
        classify(&output, action, &target)
    }

    fn restart(&self, name: &str, scope: ServiceScope) -> Result<()> {
        self.launch(name, scope, &["kickstart", "-k"])
    }

    fn enable(&self, name: &str, scope: ServiceScope) -> Result<()> {
        check_label(name)?;
        let domain = self.domain(scope)?;
        let detail = self.print_service(&domain, name)?;
        let unloaded = match &detail {
            Some(_) => None,
            None => Some(self.require_definition(name, scope)?),
        };
        let service = format!("{domain}/{name}");
        let path = detail
            .as_ref()
            .and_then(|known| known.path.as_deref())
            .or_else(|| unloaded.as_ref().map(|known| known.path.as_str()));
        let target = Target {
            domain: &domain,
            label: name,
            path,
        };
        self.call(&["enable", &service], Action::Enable, &target)?;
        match unloaded {
            Some(definition) => self.bootstrap(&domain, name, &definition.path),
            None => Ok(()),
        }
    }

    fn disable(&self, name: &str, scope: ServiceScope) -> Result<()> {
        check_label(name)?;
        let domain = self.domain(scope)?;
        let detail = self.print_service(&domain, name)?;
        if detail.is_none() {
            self.require_definition(name, scope)?;
        }
        let service = format!("{domain}/{name}");
        let target = Target {
            domain: &domain,
            label: name,
            path: detail.as_ref().and_then(|known| known.path.as_deref()),
        };
        self.call(&["disable", &service], Action::Disable, &target)?;
        if detail.is_none() {
            return Ok(());
        }
        let output = self.run(&["bootout", &service])?;
        if output.code == EXIT_NOT_FOUND {
            return Ok(());
        }
        classify(&output, Action::Bootout, &target)
    }
}

#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
