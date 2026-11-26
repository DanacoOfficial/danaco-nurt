// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Service managers: systemd, the Windows service control manager and launchd.
//!
//! The stages `services`, `start`, `restart`, `enable` and `disable` do not
//! depend on the operating system. They call the service manager of the
//! system through [`ServiceBackend`], whose implementation is selected at
//! compile time: systemd over D-Bus on Linux, the service control manager on
//! Windows and launchd on macOS.
//!
//! Every action is irreversible: before the manager is asked to change a
//! service, the transaction of the operation receives the marker
//! `service_changed`. [`ServiceControl`] offers the same actions to the
//! commands of other groups, for example `stop` on a `service` record.

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{
    Effect, EffectStatus, Service, ServiceManager, ServiceScope, ServiceState, StartType,
};
use nurt_contracts::report::{
    Change, ChangeAction, Irreversible, IrreversibleEntry, Notable, NotableEntry,
};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind, Regex as RegexValue, Value};

#[cfg(target_os = "linux")]
mod linux;
// The launchd backend reads property lists and talks to `launchctl` through
// `ProcessHost` only, so its tests also run on the other systems.
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(windows)]
mod windows;
// The service control manager rules in `windows/scm.rs` do not call the
// system. On Windows the module belongs to the Windows backend; on the other
// systems it is compiled for its unit tests only.
#[cfg(all(test, not(windows)))]
#[allow(
    dead_code,
    reason = "the Windows backend uses items that the tests do not reach"
)]
#[path = "windows/scm.rs"]
mod scm;

#[cfg(test)]
mod tests;

// Each platform module provides
// `pub(crate) fn backend(ctx: &ExecCtx) -> Box<dyn ServiceBackend + '_>`.
// The backend may keep `ctx` to reach `ExecCtx::process` and `ExecCtx::note`.
// The constructor does not contact the manager, so a missing or unreachable
// manager surfaces as an error of the first call.
#[cfg(target_os = "linux")]
pub(crate) use linux::backend as platform_backend;
#[cfg(target_os = "macos")]
pub(crate) use macos::backend as platform_backend;
#[cfg(windows)]
pub(crate) use windows::backend as platform_backend;

/// Name of the source command in the catalog.
pub const SERVICES_COMMAND: &str = "services";

/// Default of the `wait` parameter of `start` and `restart`.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(10);

const PARAM_NAME: &str = "name";
const PARAM_STATE: &str = "state";
const PARAM_SCOPE: &str = "scope";
const PARAM_WAIT: &str = "wait";

/// Key of the static information that holds the wait of `start` and
/// `restart`.
const DETAIL_WAIT: &str = "wait";

/// Prefix of the target of an effect on a service, for example
/// `service:nginx`.
const TARGET_PREFIX: &str = "service:";

/// First interval between two readings of the state while waiting.
const POLL_FIRST: Duration = Duration::from_millis(100);
/// Growth of the interval after every reading.
const POLL_STEP: Duration = Duration::from_millis(100);
/// Longest interval between two readings.
const POLL_MAX: Duration = Duration::from_millis(500);

/// Selection of services read by [`ServiceBackend::list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServiceQuery {
    /// System services or services of the current account.
    pub(crate) scope: ServiceScope,
}

/// Current state of one service, read while waiting for a transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceStatus {
    /// State mapped to the common enumeration.
    pub(crate) state: ServiceState,
    /// State as reported by the manager, for example `activating/start`.
    pub(crate) state_raw: Option<String>,
    /// Reason of the last failure as reported by the manager, for example
    /// the systemd property `Result` (`exit-code`) or a Windows exit code.
    pub(crate) result: Option<String>,
}

/// Access to the service manager of one operating system.
///
/// The stages call the methods synchronously from one thread at a time per
/// pipeline; implementations block for at most one round trip to the
/// manager and never wait for a state transition, which the stages poll
/// through [`ServiceBackend::status`].
///
/// # Names
///
/// Every method accepts the name as typed by the user or as found in a
/// record returned by [`ServiceBackend::list`] or [`ServiceBackend::get`],
/// and resolves it the way the manager does (systemd: `nginx` and
/// `nginx.service` name the same unit; Windows: the comparison ignores
/// case). Returned records carry the canonical name of the manager without
/// a type suffix.
///
/// # Records
///
/// [`ServiceBackend::list`] and [`ServiceBackend::get`] fill `name`,
/// `state`, `start_type`, `manager` and `scope`, and every optional field
/// the manager reports; `state_raw` holds the raw state and `os_raw` the
/// attributes specific to the manager. A service whose state cannot be
/// mapped has `state: unknown` and its raw state in `state_raw`.
///
/// # Actions
///
/// [`ServiceBackend::start`], [`ServiceBackend::stop`],
/// [`ServiceBackend::restart`], [`ServiceBackend::enable`] and
/// [`ServiceBackend::disable`] return as soon as the manager accepted the
/// request. `restart` starts a stopped service. `enable` and `disable`
/// change the start type persistently and make the manager reload its
/// configuration when it needs that. The stages skip a call whose target
/// state already holds, so implementations need not detect it.
///
/// # States while waiting
///
/// After an action the stages read [`ServiceBackend::status`] until the
/// state is neither `starting` nor `stopping`. While the manager still
/// processes an accepted request (a queued job of systemd, a pending state
/// of Windows), the status reports `starting` or `stopping` even when the
/// state of the service itself has not changed yet. A service that ran to
/// completion after a start reports `stopped`; a service that failed reports
/// `failed` and the reason in [`ServiceStatus::result`].
///
/// # Errors
///
/// Every method maps the errors of the manager to these codes:
///
/// | Situation | Code | Constructor |
/// |---|---|---|
/// | the service does not exist | `E_NOT_FOUND` | [`service_not_found`] |
/// | the account lacks a privilege | `E_PRIVILEGE` with detail `required` | [`service_privilege`] |
/// | the scope is not supported on the system | `E_UNSUPPORTED` | [`scope_unsupported`] |
/// | any other error of the manager, including an unreachable manager | `E_SERVICE` with details `manager`, `unit`, `os_code`, `os_message` | [`service_error`] |
pub(crate) trait ServiceBackend: Send + Sync {
    /// Service manager behind this backend.
    fn manager(&self) -> ServiceManager;

    /// Every service of the scope, in the order of the manager.
    ///
    /// # Errors
    ///
    /// See the error table of [`ServiceBackend`].
    fn list(&self, query: &ServiceQuery) -> Result<Vec<Service>>;

    /// One service by name.
    ///
    /// # Errors
    ///
    /// `E_NOT_FOUND` when the manager knows no service of that name; see the
    /// error table of [`ServiceBackend`].
    fn get(&self, name: &str, scope: ServiceScope) -> Result<Service>;

    /// Current state of one service; cheaper than [`ServiceBackend::get`].
    ///
    /// # Errors
    ///
    /// See the error table of [`ServiceBackend`].
    fn status(&self, name: &str, scope: ServiceScope) -> Result<ServiceStatus>;

    /// Requests the start of a service.
    ///
    /// # Errors
    ///
    /// See the error table of [`ServiceBackend`].
    fn start(&self, name: &str, scope: ServiceScope) -> Result<()>;

    /// Requests the stop of a service.
    ///
    /// # Errors
    ///
    /// See the error table of [`ServiceBackend`].
    fn stop(&self, name: &str, scope: ServiceScope) -> Result<()>;

    /// Requests the restart of a service; a stopped service is started.
    ///
    /// # Errors
    ///
    /// See the error table of [`ServiceBackend`].
    fn restart(&self, name: &str, scope: ServiceScope) -> Result<()>;

    /// Makes a service start automatically at boot or login.
    ///
    /// # Errors
    ///
    /// See the error table of [`ServiceBackend`].
    fn enable(&self, name: &str, scope: ServiceScope) -> Result<()>;

    /// Stops a service from starting automatically.
    ///
    /// # Errors
    ///
    /// See the error table of [`ServiceBackend`].
    fn disable(&self, name: &str, scope: ServiceScope) -> Result<()>;
}

/// `E_NOT_FOUND` for a service that the manager does not know.
pub(crate) fn service_not_found(manager: ServiceManager, name: &str) -> Error {
    Error::new(ErrorCode::NotFound, format!("no service named `{name}`"))
        .with_found(name)
        .with_fix(SERVICES_COMMAND)
        .with_detail("manager", manager.as_str().into())
        .with_detail("unit", name.into())
}

/// `E_PRIVILEGE` for an action that requires the privilege `required`, for
/// example `root` or `administrator`.
pub(crate) fn service_privilege(manager: ServiceManager, name: &str, required: &str) -> Error {
    Error::new(
        ErrorCode::Privilege,
        format!("changing service `{name}` requires the `{required}` privilege"),
    )
    .with_detail("required", required.into())
    .with_detail("manager", manager.as_str().into())
    .with_detail("unit", name.into())
}

/// `E_UNSUPPORTED` for a scope that the manager of the system does not
/// offer.
#[cfg(target_os = "linux")]
pub(crate) fn scope_unsupported(manager: ServiceManager, scope: ServiceScope) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!("services of scope `{scope}` are not available on this system"),
    )
    .with_fix(format!("services scope: {}", ServiceScope::System))
    .with_detail("manager", manager.as_str().into())
    .with_detail("scope", scope.as_str().into())
}

/// `E_SERVICE` for an error reported by the manager.
///
/// `os_code` is the stable error name or number of the manager, for example
/// `org.freedesktop.systemd1.UnitMasked` or `1058`; `os_message` is its
/// message.
pub(crate) fn service_error(
    manager: ServiceManager,
    name: &str,
    os_code: &str,
    os_message: &str,
) -> Error {
    let message = if os_message.is_empty() {
        format!("{manager} failed for service `{name}` ({os_code})")
    } else {
        format!("{manager} failed for service `{name}`: {os_message}")
    };
    Error::new(ErrorCode::Service, message)
        .with_problem(os_code)
        .with_detail("manager", manager.as_str().into())
        .with_detail("unit", name.into())
        .with_detail("os_code", os_code.into())
        .with_detail("os_message", os_message.into())
}

/// Action on a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServiceAction {
    /// Start the service.
    Start,
    /// Stop the service.
    Stop,
    /// Restart the service, or start it when it is stopped.
    Restart,
    /// Start the service automatically at boot or login.
    Enable,
    /// Stop starting the service automatically.
    Disable,
}

impl ServiceAction {
    /// Name of the action, equal to the name of its command.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_sys::services::ServiceAction;
    ///
    /// assert_eq!(ServiceAction::Restart.as_str(), "restart");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Enable => "enable",
            Self::Disable => "disable",
        }
    }

    /// The action changes the start type rather than the state.
    const fn changes_start_type(self) -> bool {
        matches!(self, Self::Enable | Self::Disable)
    }

    /// The action waits for the state to settle.
    const fn waits(self) -> bool {
        matches!(self, Self::Start | Self::Stop | Self::Restart)
    }

    /// State or start type the action leads to.
    const fn expected(self) -> &'static str {
        match self {
            Self::Start | Self::Restart => ServiceState::Running.as_str(),
            Self::Stop => ServiceState::Stopped.as_str(),
            Self::Enable => StartType::Automatic.as_str(),
            Self::Disable => StartType::Disabled.as_str(),
        }
    }

    /// The service is already where the action leads; `restart` always
    /// acts.
    fn already_holds(self, before: &str) -> bool {
        self != Self::Restart && before == self.expected()
    }
}

/// Outcome of an action on one service.
///
/// `before` and `after` hold the state (`start`, `stop`, `restart`) or the
/// start type (`enable`, `disable`). When the service was already where the
/// action leads, the manager was not called, `changed` is `false` and
/// `after` equals `before`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceTransition {
    /// Canonical name of the service.
    pub name: String,
    /// Action performed.
    pub action: ServiceAction,
    /// State or start type before the action.
    pub before: String,
    /// State or start type after the action.
    pub after: String,
    /// The manager was asked to act.
    pub changed: bool,
}

impl ServiceTransition {
    /// Target of the effect, for example `service:nginx`.
    #[must_use]
    pub fn target(&self) -> String {
        format!("{TARGET_PREFIX}{}", self.name)
    }

    /// Change of the effect report with the action `state`.
    #[must_use]
    pub fn to_change(&self) -> Change {
        let mut change = Change::new(self.target(), ChangeAction::State);
        change.before = Some(self.before.clone());
        change.after = Some(self.after.clone());
        change
    }

    /// The `effect` record of an applied action.
    #[must_use]
    pub fn to_effect(&self) -> Record {
        effect_record(self, EffectStatus::Ok)
    }
}

/// Actions on services offered to the commands of other groups.
///
/// The executor shares an implementation through the extensions of the
/// execution context, so that `stop` acts on `service` records without
/// knowing the service manager. Every method reads the current state, skips
/// the manager when the state already holds, otherwise marks the
/// transaction with `service_changed` before it asks the manager to act, and
/// reports the state before and after.
pub trait ServiceControl: Send + Sync {
    /// Starts a service and waits up to `wait` for it to run.
    ///
    /// # Errors
    ///
    /// `E_NOT_FOUND`, `E_PRIVILEGE` and `E_SERVICE` from the manager;
    /// `E_SERVICE` with the reason of the manager when the service fails;
    /// `E_TIMEOUT` with the current state in `actual` when the wait elapses;
    /// `E_CANCELLED` when the execution is cancelled.
    fn start(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition>;

    /// Stops a service and waits up to `wait` for it to stop.
    ///
    /// # Errors
    ///
    /// As [`ServiceControl::start`].
    fn stop(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition>;

    /// Restarts a service and waits up to `wait` for it to run.
    ///
    /// # Errors
    ///
    /// As [`ServiceControl::start`].
    fn restart(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition>;

    /// Makes a service start automatically at boot or login.
    ///
    /// # Errors
    ///
    /// `E_NOT_FOUND`, `E_PRIVILEGE` and `E_SERVICE` from the manager;
    /// `E_CANCELLED` when the execution is cancelled.
    fn enable(
        &self,
        service: &Service,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition>;

    /// Stops a service from starting automatically.
    ///
    /// # Errors
    ///
    /// As [`ServiceControl::enable`].
    fn disable(
        &self,
        service: &Service,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition>;
}

/// [`ServiceControl`] through the service manager of the running system.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::Extensions;
/// use nurt_sys::services::{ServiceControl, SystemServiceControl};
///
/// let mut extensions = Extensions::new();
/// extensions.insert::<dyn ServiceControl>(Arc::new(SystemServiceControl));
/// assert!(extensions.contains::<dyn ServiceControl>());
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemServiceControl;

impl ServiceControl for SystemServiceControl {
    fn start(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        let backend = platform_backend(ctx);
        perform(&*backend, ServiceAction::Start, service, wait, tx, ctx)
    }

    fn stop(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        let backend = platform_backend(ctx);
        perform(&*backend, ServiceAction::Stop, service, wait, tx, ctx)
    }

    fn restart(
        &self,
        service: &Service,
        wait: Duration,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        let backend = platform_backend(ctx);
        perform(&*backend, ServiceAction::Restart, service, wait, tx, ctx)
    }

    fn enable(
        &self,
        service: &Service,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        let backend = platform_backend(ctx);
        perform(
            &*backend,
            ServiceAction::Enable,
            service,
            Duration::ZERO,
            tx,
            ctx,
        )
    }

    fn disable(
        &self,
        service: &Service,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<ServiceTransition> {
        let backend = platform_backend(ctx);
        perform(
            &*backend,
            ServiceAction::Disable,
            service,
            Duration::ZERO,
            tx,
            ctx,
        )
    }
}

/// Every stage of the module, for registration.
///
/// # Examples
///
/// ```
/// let commands: Vec<String> = nurt_sys::services::stages()
///     .iter()
///     .map(|stage| stage.descriptor().command)
///     .collect();
/// assert_eq!(commands, ["services", "start", "restart", "enable", "disable"]);
/// ```
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    let mut stages: Vec<Arc<dyn StageImpl>> = vec![Arc::new(Services)];
    for command in ServiceCommand::ALL {
        stages.push(Arc::new(command));
    }
    stages
}

/// The `services` command: services of the system or of the current
/// account.
///
/// `name` selects one service by name (`E_NOT_FOUND` when there is none) or
/// services whose name matches a regular expression; `state` keeps services
/// whose state, raw state or a part of the raw state equals the text,
/// ignoring case.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::services::Services;
///
/// assert!(Services.descriptor().source);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Services;

impl StageImpl for Services {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: SERVICES_COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        ServiceFilter::from_args(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Services {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let filter = ServiceFilter::from_args(args)?;
        let backend = platform_backend(ctx);
        let records = list_services(&*backend, &filter)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// Selection of `services` by name.
#[derive(Debug, Clone)]
enum NameFilter {
    /// Every service.
    Any,
    /// One service by name.
    Exact(String),
    /// Services whose name matches.
    Pattern(regex::Regex),
}

/// Arguments of `services`.
#[derive(Debug, Clone)]
struct ServiceFilter {
    name: NameFilter,
    state: Option<String>,
    scope: ServiceScope,
}

impl ServiceFilter {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        let name = match args.value(PARAM_NAME) {
            None | Some(Value::Null) => NameFilter::Any,
            Some(Value::Text(text)) => NameFilter::Exact(text.clone()),
            Some(Value::Regex(pattern)) => NameFilter::Pattern(compile_regex(pattern)?),
            Some(other) => return Err(type_error(PARAM_NAME, "text or regex", other)),
        };
        let state = match args.value(PARAM_STATE) {
            None | Some(Value::Null) => None,
            Some(Value::Text(text) | Value::Enum(text)) => Some(text.to_lowercase()),
            Some(other) => return Err(type_error(PARAM_STATE, "text", other)),
        };
        let scope = match args.value(PARAM_SCOPE) {
            None | Some(Value::Null) => ServiceScope::System,
            Some(Value::Text(text) | Value::Enum(text)) => parse_scope(text)?,
            Some(other) => return Err(type_error(PARAM_SCOPE, "system or user", other)),
        };
        Ok(Self { name, state, scope })
    }

    fn matches_state(&self, service: &Service) -> bool {
        let Some(wanted) = &self.state else {
            return true;
        };
        if service.state.as_str() == wanted {
            return true;
        }
        service.state_raw.as_deref().is_some_and(|raw| {
            let raw = raw.to_lowercase();
            raw == *wanted || raw.split('/').any(|part| part == wanted)
        })
    }
}

fn parse_scope(text: &str) -> Result<ServiceScope> {
    text.parse().map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("unknown scope `{text}`; use `system` or `user`"),
        )
        .with_found(text)
        .with_expected(serde_json::json!(["system", "user"]))
    })
}

fn compile_regex(pattern: &RegexValue) -> Result<regex::Regex> {
    regex::RegexBuilder::new(&pattern.pattern)
        .case_insensitive(pattern.flags.case_insensitive)
        .multi_line(pattern.flags.multi_line)
        .dot_matches_new_line(pattern.flags.dot_matches_new_line)
        .ignore_whitespace(pattern.flags.extended)
        .build()
        .map_err(|error| {
            Error::new(
                ErrorCode::Regex,
                format!("invalid regular expression for `{PARAM_NAME}`: {error}"),
            )
            .with_found(pattern.pattern.clone())
        })
}

fn type_error(name: &str, expected: &str, found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{name}` expects {expected}, found {}", found.type_name()),
    )
    .with_expected(serde_json::Value::from(expected))
    .with_actual(serde_json::Value::from(found.type_name()))
}

/// Records of `services` after the filters.
fn list_services(backend: &dyn ServiceBackend, filter: &ServiceFilter) -> Result<Vec<Record>> {
    let services = match &filter.name {
        NameFilter::Exact(name) => vec![backend.get(name, filter.scope)?],
        NameFilter::Any | NameFilter::Pattern(_) => backend.list(&ServiceQuery {
            scope: filter.scope,
        })?,
    };
    Ok(services
        .into_iter()
        .filter(|service| match &filter.name {
            NameFilter::Pattern(pattern) => pattern.is_match(&service.name),
            NameFilter::Any | NameFilter::Exact(_) => true,
        })
        .filter(|service| filter.matches_state(service))
        .map(Record::from)
        .collect())
}

/// The commands `start`, `restart`, `enable` and `disable` on `service`
/// records.
///
/// Each input service is read first; a service already in the target state
/// or start type is left alone and reported as `unchanged`. `start` and
/// `restart` wait up to `wait` for the service to run.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::services::ServiceCommand;
///
/// assert_eq!(ServiceCommand::RESTART.descriptor().command, "restart");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceCommand {
    action: ServiceAction,
}

impl ServiceCommand {
    /// The `start` command.
    pub const START: Self = Self {
        action: ServiceAction::Start,
    };
    /// The `restart` command.
    pub const RESTART: Self = Self {
        action: ServiceAction::Restart,
    };
    /// The `enable` command.
    pub const ENABLE: Self = Self {
        action: ServiceAction::Enable,
    };
    /// The `disable` command.
    pub const DISABLE: Self = Self {
        action: ServiceAction::Disable,
    };
    /// Every command, in catalog order.
    pub const ALL: [Self; 4] = [Self::START, Self::RESTART, Self::ENABLE, Self::DISABLE];

    /// Action of the command.
    #[must_use]
    pub const fn action(self) -> ServiceAction {
        self.action
    }

    fn wait(self, args: &BoundArgs) -> Result<Duration> {
        if !self.action.waits() {
            return Ok(Duration::ZERO);
        }
        match args.value(PARAM_WAIT) {
            None | Some(Value::Null) => Ok(DEFAULT_WAIT),
            Some(Value::Duration(duration)) => duration.to_std().ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    "wait must not be negative; give a value such as 10s",
                )
                .with_found(format!("{}ms", duration.as_millis()))
            }),
            Some(other) => Err(type_error(PARAM_WAIT, "duration", other)),
        }
    }
}

impl StageImpl for ServiceCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.action.as_str().to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Service]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let wait = self.wait(args)?;
        let mut info = StaticInfo {
            waits: self.action.waits(),
            ..StaticInfo::default()
        };
        if self.action.waits() {
            let millis = i64::try_from(wait.as_millis()).unwrap_or(i64::MAX);
            info.details.insert(
                DETAIL_WAIT.to_owned(),
                Value::Duration(nurt_contracts::value::Duration::from_millis(millis)),
            );
        }
        Ok(info)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for ServiceCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        self.wait(args)?;
        let services = input
            .into_records()?
            .map(|record| record.and_then(|record| Ok(Service::try_from(record)?)))
            .collect::<Result<Vec<_>>>()?;
        let backend = platform_backend(ctx);
        plan_actions(&*backend, self.action, services, ctx)
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let wait = self.wait(args)?;
        let services = plan.take_state::<Vec<Service>>().unwrap_or_default();
        let backend = platform_backend(ctx);
        apply_actions(&*backend, self.action, &services, wait, tx, ctx)
    }
}

/// Plan of an action on the input services: the current state of each and
/// the change that the action would make.
fn plan_actions(
    backend: &dyn ServiceBackend,
    action: ServiceAction,
    services: Vec<Service>,
    ctx: &ExecCtx,
) -> Result<EffectPlan> {
    let mut plan = EffectPlan::new();
    if services.is_empty() {
        plan.notable
            .push(NotableEntry::new(Notable::NothingMatched));
    }
    for service in &services {
        ctx.checkpoint()?;
        let target = Target::of(backend, service)?;
        let before = target.read(backend, action)?;
        let holds = action.already_holds(&before);
        let transition = ServiceTransition {
            name: target.name,
            action,
            after: if holds {
                before.clone()
            } else {
                action.expected().to_owned()
            },
            before,
            changed: !holds,
        };
        if holds {
            plan.notable.push(unchanged_note(&transition));
        } else {
            plan.changes.push(transition.to_change());
            plan.irreversible.push(irreversible_entry(&transition));
        }
        plan.preview
            .push(effect_record(&transition, EffectStatus::Planned));
    }
    Ok(plan.with_state(services))
}

/// Applies an action to every planned service, in input order.
fn apply_actions<'a>(
    backend: &dyn ServiceBackend,
    action: ServiceAction,
    services: &[Service],
    wait: Duration,
    tx: &mut dyn MutationTx,
    ctx: &ExecCtx,
) -> Result<EffectOutcome<'a>> {
    let mut records = Vec::with_capacity(services.len());
    let mut changes = Vec::new();
    let mut notable = Vec::new();
    for service in services {
        let transition = perform(backend, action, service, wait, tx, ctx)?;
        if transition.changed {
            changes.push(transition.to_change());
        } else {
            notable.push(unchanged_note(&transition));
        }
        records.push(transition.to_effect());
    }
    let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
    outcome.changes = changes;
    outcome.notable = notable;
    Ok(outcome)
}

/// Service addressed by an action.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    name: String,
    scope: ServiceScope,
}

impl Target {
    fn of(backend: &dyn ServiceBackend, service: &Service) -> Result<Self> {
        let manager = backend.manager();
        if service.manager != manager {
            return Err(Error::new(
                ErrorCode::Unsupported,
                format!(
                    "service `{}` belongs to {}, but this system uses {manager}; read it with `services` on this system",
                    service.name, service.manager
                ),
            )
            .with_fix(format!("services name: \"{}\"", service.name))
            .with_expected(serde_json::Value::from(manager.as_str()))
            .with_actual(serde_json::Value::from(service.manager.as_str())));
        }
        Ok(Self {
            name: service.name.clone(),
            scope: service.scope.unwrap_or(ServiceScope::System),
        })
    }

    /// State or start type that the action changes.
    fn read(&self, backend: &dyn ServiceBackend, action: ServiceAction) -> Result<String> {
        if action.changes_start_type() {
            let service = backend.get(&self.name, self.scope)?;
            Ok(service.start_type.as_str().to_owned())
        } else {
            let status = backend.status(&self.name, self.scope)?;
            Ok(status.state.as_str().to_owned())
        }
    }
}

/// Performs one action: reads the state, marks the transaction, asks the
/// manager and waits for the result.
fn perform(
    backend: &dyn ServiceBackend,
    action: ServiceAction,
    service: &Service,
    wait: Duration,
    tx: &mut dyn MutationTx,
    ctx: &ExecCtx,
) -> Result<ServiceTransition> {
    let target = Target::of(backend, service)?;
    let before = target.read(backend, action)?;
    let mut transition = ServiceTransition {
        name: target.name.clone(),
        action,
        after: before.clone(),
        before,
        changed: false,
    };
    if action.already_holds(&transition.before) {
        return Ok(transition);
    }
    ctx.checkpoint()?;
    tx.mark_irreversible(irreversible_entry(&transition));
    transition.changed = true;
    let (name, scope) = (target.name.as_str(), target.scope);
    match action {
        ServiceAction::Start => backend.start(name, scope)?,
        ServiceAction::Stop => backend.stop(name, scope)?,
        ServiceAction::Restart => backend.restart(name, scope)?,
        ServiceAction::Enable => backend.enable(name, scope)?,
        ServiceAction::Disable => backend.disable(name, scope)?,
    }
    transition.after = settle(backend, action, &target, wait, ctx)
        .map_err(|error| error.with_effect_applied(true))?;
    Ok(transition)
}

/// State or start type after an accepted action.
fn settle(
    backend: &dyn ServiceBackend,
    action: ServiceAction,
    target: &Target,
    wait: Duration,
    ctx: &ExecCtx,
) -> Result<String> {
    if action.changes_start_type() {
        let service = backend.get(&target.name, target.scope)?;
        return Ok(service.start_type.as_str().to_owned());
    }
    let status = if wait.is_zero() {
        backend.status(&target.name, target.scope)?
    } else {
        await_settled(backend, action, target, wait, ctx)?
    };
    if matches!(action, ServiceAction::Start | ServiceAction::Restart)
        && status.state == ServiceState::Failed
    {
        return Err(failed_error(backend.manager(), &target.name, &status));
    }
    Ok(status.state.as_str().to_owned())
}

/// Reads the state until it is neither `starting` nor `stopping`, for no
/// longer than `wait` and the time left to the program.
///
/// The interval between readings starts at [`POLL_FIRST`] and grows by
/// [`POLL_STEP`] up to [`POLL_MAX`]; every sleep observes cancellation.
fn await_settled(
    backend: &dyn ServiceBackend,
    action: ServiceAction,
    target: &Target,
    wait: Duration,
    ctx: &ExecCtx,
) -> Result<ServiceStatus> {
    let limit = ctx
        .remaining()
        .map_or(wait, |remaining| remaining.min(wait));
    let end = ctx.clock().monotonic().saturating_add(limit);
    let mut interval = POLL_FIRST;
    loop {
        ctx.cancel().check()?;
        let status = backend.status(&target.name, target.scope)?;
        if !matches!(
            status.state,
            ServiceState::Starting | ServiceState::Stopping
        ) {
            return Ok(status);
        }
        let now = ctx.clock().monotonic();
        if now >= end {
            return Err(timeout_error(action, &target.name, &status, limit));
        }
        ctx.sleep(interval.min(end.saturating_sub(now)))?;
        interval = interval.saturating_add(POLL_STEP).min(POLL_MAX);
    }
}

fn timeout_error(
    action: ServiceAction,
    name: &str,
    status: &ServiceStatus,
    limit: Duration,
) -> Error {
    let state = status.state.as_str();
    let mut error = Error::new(
        ErrorCode::Timeout,
        format!(
            "service `{name}` is still {state} after {} ms; give a longer wait",
            limit.as_millis()
        ),
    )
    .with_expected(serde_json::Value::from(action.expected()))
    .with_actual(serde_json::Value::from(state))
    .with_next_step(format!("services name: \"{name}\""))
    .with_detail("unit", name.into());
    if let Some(raw) = &status.state_raw {
        error = error.with_detail("state_raw", raw.as_str().into());
    }
    error
}

fn failed_error(manager: ServiceManager, name: &str, status: &ServiceStatus) -> Error {
    let reason = status
        .result
        .as_deref()
        .unwrap_or(ServiceState::Failed.as_str());
    service_error(
        manager,
        name,
        reason,
        "the service entered the failed state",
    )
    .with_actual(serde_json::Value::from(ServiceState::Failed.as_str()))
    .with_next_step(format!("logs unit: \"{name}\""))
    .with_detail("result", reason.into())
}

fn irreversible_entry(transition: &ServiceTransition) -> IrreversibleEntry {
    IrreversibleEntry {
        reason: Irreversible::ServiceChanged,
        path: Some(transition.target()),
        raw_path: None,
        count: None,
        detail: Some(transition.action.as_str().to_owned()),
    }
}

fn unchanged_note(transition: &ServiceTransition) -> NotableEntry {
    let mut note = NotableEntry::new(Notable::Unchanged);
    note.path = Some(transition.target());
    note.detail = Some(format!(
        "{} skipped: the service is already {}",
        transition.action.as_str(),
        transition.before
    ));
    note
}

fn effect_record(transition: &ServiceTransition, status: EffectStatus) -> Record {
    let effect = Effect {
        status,
        kind: transition.action.as_str().to_owned(),
        targets: Some(1),
        notable: (!transition.changed).then(|| vec![Notable::Unchanged.as_str().to_owned()]),
        irreversible: transition
            .changed
            .then(|| vec![irreversible_entry(transition)]),
        target: Some(transition.target()),
        action: Some(transition.action.as_str().to_owned()),
        before: Some(Value::Enum(transition.before.clone())),
        after: Some(Value::Enum(transition.after.clone())),
        ..Effect::default()
    };
    Record::from(effect)
}
