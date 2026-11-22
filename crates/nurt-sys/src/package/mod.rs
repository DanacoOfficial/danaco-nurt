// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Package managers.
//!
//! `package.list` and `package.search` read the databases and repository
//! indexes of the managers and start no program. `package.install`,
//! `package.remove` and `package.upgrade` start the program of the manager
//! through the process host of the execution context; the change is marked
//! in the journal as irreversible (`package_changed`).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::{Effect, EffectStatus, Package, PackageManager};
use nurt_contracts::report::{Change, ChangeAction, Irreversible, IrreversibleEntry};
use nurt_contracts::services::{FileHolderService, ProgramOutcome};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, Event, ExecCtx, ExternalProgram,
    InputKinds, LogEvent, LogLevel, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, Size, Value};
use nurt_programs::model::RawPackage;

mod compress;
mod control;
mod detect;
mod exec;
mod index;
mod installed;
mod root;
mod version;

#[cfg(test)]
mod tests;

use detect::{detect_managers, is_present};
pub use exec::{Action, ChangePlan, Invocation, build as build_change};
pub use index::{IndexEntry, IndexReport, IndexWarning, read_indexes};
pub use installed::{InstalledSet, InstalledSource, ManagerDatabases};
use root::{HostEnv, SysPaths};
pub use version::VersionScheme;

const SOURCE_NAME: &str = "package";
const STDERR_TAIL_BYTES: usize = 400;
const PHASED_COMPLETE: u8 = 100;
const ARCHITECTURE_ALL: &str = "all";

type PrivilegeCheck = Arc<dyn Fn() -> bool + Send + Sync>;

/// Facilities shared by the package stages.
pub struct PackageEnv {
    root: Option<PathBuf>,
    installed: Arc<dyn InstalledSource>,
    is_root: PrivilegeCheck,
}

impl PackageEnv {
    /// Environment of the running system that reads the installed packages
    /// from the databases of the managers.
    #[must_use]
    pub fn system() -> Self {
        Self::new(Arc::new(ManagerDatabases))
    }

    /// Environment of the running system with the given source of installed
    /// packages.
    #[must_use]
    pub fn new(installed: Arc<dyn InstalledSource>) -> Self {
        Self {
            root: None,
            installed,
            is_root: Arc::new(effective_user_is_root),
        }
    }

    /// The same environment with system paths resolved below `dir`, which
    /// stands for the root of the inspected system.
    #[must_use]
    pub fn with_root(mut self, dir: impl Into<PathBuf>) -> Self {
        self.root = Some(dir.into());
        self
    }

    /// The same environment with another check of the superuser.
    #[must_use]
    pub fn with_root_check(mut self, check: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.is_root = Arc::new(check);
        self
    }
}

#[cfg(unix)]
fn effective_user_is_root() -> bool {
    rustix::process::geteuid().is_root()
}

#[cfg(not(unix))]
fn effective_user_is_root() -> bool {
    false
}

/// The five stages of the group, sharing `env`.
#[must_use]
pub fn stages(env: &Arc<PackageEnv>) -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(PackageList::new(Arc::clone(env))),
        Arc::new(PackageSearch::new(Arc::clone(env))),
        Arc::new(PackageChange::new(Arc::clone(env), Action::Install)),
        Arc::new(PackageChange::new(Arc::clone(env), Action::Remove)),
        Arc::new(PackageChange::new(Arc::clone(env), Action::Upgrade)),
    ]
}

fn host_env(env: &PackageEnv, ctx: &ExecCtx) -> HostEnv {
    let host = HostEnv::from_ctx(ctx);
    match &env.root {
        Some(dir) => host.with_root(dir.clone()),
        None => host,
    }
}

fn warn(ctx: &ExecCtx, message: String) {
    ctx.emit(Event::Log(LogEvent {
        level: LogLevel::Warn,
        message,
        source: Some(SOURCE_NAME.to_owned()),
    }));
}

fn manager_arg(args: &BoundArgs) -> Result<Option<PackageManager>> {
    let text = match args.value("manager") {
        None => return Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => text.as_str(),
        Some(_) => return args.text("manager").map(|_| None),
    };
    if text == "auto" {
        return Ok(None);
    }
    text.parse::<PackageManager>().map(Some).map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("unknown package manager `{text}`"),
        )
    })
}

fn names_arg(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let wrong_type = || Error::new(ErrorCode::Type, format!("`{name}` expects a list of text"));
    match args.value(name) {
        None => Ok(Vec::new()),
        Some(Value::Text(text)) => Ok(vec![text.clone()]),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(text) => Ok(text.clone()),
                _ => Err(wrong_type()),
            })
            .collect(),
        Some(_) => Err(wrong_type()),
    }
}

fn missing_manager(requested: Option<PackageManager>) -> Error {
    let message = match requested {
        Some(manager) => format!("{manager} is not installed on this system"),
        None => "no supported package manager found; pass `manager`".to_owned(),
    };
    let mut error = Error::new(ErrorCode::ExternalMissing, message);
    if let Some(manager) = requested {
        error = error.with_detail("program", serde_json::json!(exec::program_name(manager)));
    }
    error
}

fn select_managers(
    env: &HostEnv,
    requested: Option<PackageManager>,
) -> Result<Vec<PackageManager>> {
    match requested {
        Some(manager) if is_present(env, manager) => Ok(vec![manager]),
        Some(manager) => Err(missing_manager(Some(manager))),
        None => {
            let found = detect_managers(env);
            if found.is_empty() {
                Err(missing_manager(None))
            } else {
                Ok(found)
            }
        }
    }
}

fn report_warnings(ctx: &ExecCtx, command: &str, report: &IndexReport) {
    for warning in &report.warnings {
        warn(
            ctx,
            format!("{command}: {}: {}", warning.code, warning.detail),
        );
    }
}

fn index_failure(manager: PackageManager) -> Error {
    Error::new(
        ErrorCode::Format,
        format!(
            "the repository indexes of {manager} cannot be read; refresh them with the manager"
        ),
    )
}

fn package_record(manager: PackageManager, name: &str, version: &str, installed: bool) -> Package {
    Package {
        name: name.to_owned(),
        version: version.to_owned(),
        manager,
        installed,
        ..Package::default()
    }
}

fn installed_package(manager: PackageManager, item: &RawPackage) -> Package {
    Package {
        architecture: item.architecture.clone(),
        description: item.summary.as_ref().map(|summary| summary.text.clone()),
        size: item.installed_size.map(Size::from_bytes),
        source: item.origin.clone(),
        ..package_record(
            manager,
            &item.name,
            item.version.as_deref().unwrap_or(""),
            true,
        )
    }
}

fn read_installed(
    env: &PackageEnv,
    host: &HostEnv,
    manager: PackageManager,
    ctx: &ExecCtx,
) -> Result<Vec<RawPackage>> {
    let set = env.installed.installed(manager, host)?;
    for warning in set.warnings {
        warn(ctx, warning);
    }
    Ok(set.packages)
}

fn architecture_fits(installed: Option<&str>, offered: Option<&str>) -> bool {
    match (installed, offered) {
        (Some(have), Some(offered)) => {
            have == offered || have == ARCHITECTURE_ALL || offered == ARCHITECTURE_ALL
        }
        _ => true,
    }
}

/// Newest offered version of each installed package, by name.
fn newest_available(
    host: &HostEnv,
    manager: PackageManager,
    installed: &[RawPackage],
) -> (HashMap<String, IndexEntry>, IndexReport) {
    let wanted: HashMap<&str, Option<&str>> = installed
        .iter()
        .map(|item| (item.name.as_str(), item.architecture.as_deref()))
        .collect();
    let scheme = VersionScheme::of(manager);
    let mut best: HashMap<String, IndexEntry> = HashMap::new();
    let report = read_indexes(host, manager, &mut |entry| {
        let Some(architecture) = wanted.get(entry.name.as_str()) else {
            return;
        };
        if !architecture_fits(*architecture, entry.architecture.as_deref()) {
            return;
        }
        if entry
            .phased
            .is_some_and(|percentage| percentage < PHASED_COMPLETE)
        {
            return;
        }
        let newer = best
            .get(&entry.name)
            .is_none_or(|current| scheme.compare(&entry.version, &current.version).is_gt());
        if newer {
            best.insert(entry.name.clone(), entry);
        }
    });
    (best, report)
}

fn outdated(
    manager: PackageManager,
    installed: Vec<RawPackage>,
    available: &HashMap<String, IndexEntry>,
) -> Vec<(RawPackage, String)> {
    let scheme = VersionScheme::of(manager);
    installed
        .into_iter()
        .filter_map(|item| {
            let offered = available.get(&item.name)?;
            let have = item.version.as_deref()?;
            scheme
                .compare(&offered.version, have)
                .is_gt()
                .then(|| (item, offered.version.clone()))
        })
        .collect()
}

fn check_record_limit(ctx: &ExecCtx, count: usize, what: &str) -> Result<()> {
    if count as u64 > ctx.limits().max_records {
        return Err(Error::new(
            ErrorCode::Limit,
            format!("too many {what} for the record limit; narrow the query"),
        ));
    }
    Ok(())
}

/// The command `package.list`.
pub struct PackageList {
    env: Arc<PackageEnv>,
}

impl PackageList {
    /// Stage over `env`.
    #[must_use]
    pub fn new(env: Arc<PackageEnv>) -> Self {
        Self { env }
    }
}

impl StageImpl for PackageList {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "package.list".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for PackageList {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let host = host_env(&self.env, ctx);
        let requested = manager_arg(args)?;
        let filter = args.text("name")?.map(str::to_lowercase);
        let only_outdated = args.bool("outdated")?.unwrap_or(false);
        let mut records: Vec<Record> = Vec::new();
        for manager in select_managers(&host, requested)? {
            ctx.checkpoint()?;
            let mut packages = read_installed(&self.env, &host, manager, ctx)?;
            if let Some(filter) = &filter {
                packages.retain(|item| item.name.to_lowercase().contains(filter));
            }
            if only_outdated {
                let (available, report) = newest_available(&host, manager, &packages);
                if requested.is_some() && report.unreadable() {
                    return Err(index_failure(manager));
                }
                report_warnings(ctx, "package.list", &report);
                for (item, offered) in outdated(manager, packages, &available) {
                    let mut package = installed_package(manager, &item);
                    package.available_version = Some(offered);
                    records.push(package.into());
                }
            } else {
                records.extend(
                    packages
                        .iter()
                        .map(|item| Record::from(installed_package(manager, item))),
                );
            }
        }
        check_record_limit(ctx, records.len(), "packages")?;
        Ok(StageOutput::records(RecordStream::from_records(sorted(
            records,
        ))))
    }
}

fn sorted(mut records: Vec<Record>) -> Vec<Record> {
    let key = |record: &Record| {
        let text = |name: &str| match record.get(name) {
            Some(Value::Text(text) | Value::Enum(text)) => text.clone(),
            _ => String::new(),
        };
        (text("manager"), text("name"))
    };
    records.sort_by_key(key);
    records
}

/// The command `package.search`.
pub struct PackageSearch {
    env: Arc<PackageEnv>,
}

impl PackageSearch {
    /// Stage over `env`.
    #[must_use]
    pub fn new(env: Arc<PackageEnv>) -> Self {
        Self { env }
    }

    fn search_manager(
        &self,
        host: &HostEnv,
        manager: PackageManager,
        query: &str,
        explicit: bool,
        ctx: &ExecCtx,
    ) -> Result<Vec<Package>> {
        let scheme = VersionScheme::of(manager);
        let mut best: BTreeMap<String, IndexEntry> = BTreeMap::new();
        let report = read_indexes(host, manager, &mut |entry| {
            let matches = entry.name.to_lowercase().contains(query)
                || entry
                    .description
                    .as_deref()
                    .is_some_and(|text| text.to_lowercase().contains(query));
            if !matches {
                return;
            }
            let newer = best
                .get(&entry.name)
                .is_none_or(|current| scheme.compare(&entry.version, &current.version).is_gt());
            if newer {
                best.insert(entry.name.clone(), entry);
            }
        });
        if explicit && report.unreadable() {
            return Err(index_failure(manager));
        }
        report_warnings(ctx, "package.search", &report);
        let installed = read_installed(&self.env, host, manager, ctx).unwrap_or_default();
        let installed: HashMap<&str, &RawPackage> = installed
            .iter()
            .map(|item| (item.name.as_str(), item))
            .collect();
        let mut found = Vec::with_capacity(best.len());
        for entry in best.into_values() {
            let have = installed.get(entry.name.as_str());
            let mut package = Package {
                architecture: entry.architecture.clone(),
                description: entry.description.clone(),
                size: entry.size.map(Size::from_bytes),
                source: Some(entry.source.clone()),
                ..package_record(manager, &entry.name, &entry.version, have.is_some())
            };
            if let Some(have) = have {
                package.version = have.version.clone().unwrap_or_default();
                if scheme.compare(&entry.version, &package.version).is_gt() {
                    package.available_version = Some(entry.version.clone());
                }
            }
            found.push(package);
        }
        Ok(found)
    }
}

impl StageImpl for PackageSearch {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "package.search".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for PackageSearch {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let host = host_env(&self.env, ctx);
        let requested = manager_arg(args)?;
        let Value::Text(query) = args.require("query")? else {
            return Err(Error::new(ErrorCode::Type, "`query` expects text"));
        };
        let query = query.to_lowercase();
        let mut found: Vec<Package> = Vec::new();
        for manager in select_managers(&host, requested)? {
            ctx.checkpoint()?;
            found.extend(self.search_manager(&host, manager, &query, requested.is_some(), ctx)?);
        }
        found.sort_by(|a, b| {
            let inexact = |package: &Package| !package.name.eq_ignore_ascii_case(&query);
            inexact(a)
                .cmp(&inexact(b))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.manager.as_str().cmp(b.manager.as_str()))
        });
        check_record_limit(ctx, found.len(), "matches")?;
        Ok(StageOutput::records(RecordStream::from_records(
            found.into_iter().map(Record::from).collect(),
        )))
    }
}

struct ChangeState {
    manager: PackageManager,
    calls: Vec<Invocation>,
    targets: Vec<String>,
    before: BTreeMap<String, Option<String>>,
}

/// The commands `package.install`, `package.remove` and `package.upgrade`.
pub struct PackageChange {
    env: Arc<PackageEnv>,
    action: Action,
}

impl PackageChange {
    /// Stage that performs `action` over `env`.
    #[must_use]
    pub fn new(env: Arc<PackageEnv>, action: Action) -> Self {
        Self { env, action }
    }

    fn command(&self) -> String {
        format!("package.{}", self.action.as_str())
    }

    fn check_locks(&self, host: &HostEnv, manager: PackageManager, ctx: &ExecCtx) -> Result<()> {
        let holders = ctx.extensions().get::<dyn FileHolderService>();
        for lock in exec::lock_files(manager) {
            let path = host.resolve(lock);
            let found = match &holders {
                Some(service) => service.holders(&path)?,
                None => Vec::new(),
            };
            let stale_prone = manager == PackageManager::Pacman && path.exists();
            if found.is_empty() && !stale_prone {
                continue;
            }
            let listed: Vec<serde_json::Value> = found
                .iter()
                .map(|holder| serde_json::json!({"pid": holder.pid, "name": holder.name}))
                .collect();
            return Err(Error::new(
                ErrorCode::InUse,
                format!("the package database of {manager} is locked by another process; retry when it ends"),
            )
            .with_detail("holders", serde_json::Value::Array(listed)));
        }
        Ok(())
    }

    fn targets(
        &self,
        host: &HostEnv,
        manager: PackageManager,
        names: Vec<String>,
        installed: &[RawPackage],
    ) -> Vec<String> {
        if !names.is_empty() {
            return names;
        }
        let (available, _) = newest_available(host, manager, installed);
        outdated(manager, installed.to_vec(), &available)
            .into_iter()
            .map(|(item, _)| item.name)
            .collect()
    }
}

impl StageImpl for PackageChange {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let mut info = StaticInfo::default();
        if let Some(manager) = manager_arg(args)? {
            info.programs.push(ExternalProgram {
                program: exec::program_name(manager).to_owned(),
                implementation: Implementation::PackageManager,
            });
        }
        Ok(info)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for PackageChange {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let host = host_env(&self.env, ctx);
        let requested = manager_arg(args)?;
        let names = names_arg(args, "names")?;
        let version = args.text("version")?;
        let managers = select_managers(&host, requested)?;
        let manager = managers[0];
        if managers.len() > 1 {
            warn(
                ctx,
                format!(
                    "{}: several managers found; using {manager}",
                    self.command()
                ),
            );
        }
        exec::check_privilege(manager, (self.env.is_root)(), host.os() != Os::Windows)?;
        self.check_locks(&host, manager, ctx)?;
        let change = exec::build(manager, self.action, &names, version)?;
        if let (Some(process), Some(first)) = (ctx.process(), change.invocations.first()) {
            process.resolve_program(&first.program, &host.resolve("/"))?;
        }
        for remark in &change.warnings {
            warn(ctx, remark.clone());
        }
        let installed = read_installed(&self.env, &host, manager, ctx).unwrap_or_default();
        let targets = self.targets(&host, manager, names, &installed);
        let before: BTreeMap<String, Option<String>> = targets
            .iter()
            .map(|name| {
                let have = installed
                    .iter()
                    .find(|item| &item.name == name)
                    .and_then(|item| item.version.clone());
                (name.clone(), have)
            })
            .collect();
        let mut plan = EffectPlan::new();
        plan.bulk = targets.len() > 1;
        for name in &targets {
            let was = before.get(name).cloned().flatten();
            let mut item = Change::new(target_of(name), ChangeAction::State);
            item.before.clone_from(&was);
            plan.changes.push(item);
            plan.preview
                .push(effect_record(self.action, name, EffectStatus::Planned, was, None).into());
        }
        plan.irreversible
            .push(change_marker(manager, targets.len()));
        Ok(plan.with_state(ChangeState {
            manager,
            calls: change.invocations,
            targets,
            before,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan
            .take_state::<ChangeState>()
            .ok_or_else(|| Error::new(ErrorCode::StageFailed, "the package change has no plan"))?;
        let marker = change_marker(state.manager, state.targets.len());
        tx.mark_irreversible(marker.clone());
        let failure = run_calls(&state, ctx)?;
        let host = host_env(&self.env, ctx);
        let after = read_installed(&self.env, &host, state.manager, ctx).unwrap_or_default();
        let mut outcome = EffectOutcome::new(StageOutput::none());
        let mut records = Vec::new();
        let status = if failure.is_some() {
            EffectStatus::Failed
        } else {
            EffectStatus::Ok
        };
        for name in &state.targets {
            let now = after
                .iter()
                .find(|item| &item.name == name)
                .and_then(|item| item.version.clone());
            let was = state.before.get(name).cloned().flatten();
            let mut item = Change::new(target_of(name), ChangeAction::State);
            item.before.clone_from(&was);
            item.after.clone_from(&now);
            outcome.changes.push(item);
            records.push(Record::from(effect_record(
                self.action,
                name,
                status,
                was,
                now,
            )));
        }
        outcome.irreversible.push(marker);
        if let Some(error) = failure {
            return Err(error.with_effect_applied(true));
        }
        outcome.output = StageOutput::records(RecordStream::from_records(records));
        Ok(outcome)
    }
}

/// Runs the invocations of a change in order and stops at the first one
/// that fails; the failure is returned, not raised, because the system may
/// already be changed.
fn run_calls(state: &ChangeState, ctx: &ExecCtx) -> Result<Option<Error>> {
    let process = ctx
        .process()
        .ok_or_else(|| missing_manager(Some(state.manager)))?;
    let defaults: Vec<(String, String)> = ctx
        .config()
        .run
        .env_defaults
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let cwd = if ctx.host().info().os == Os::Windows {
        "C:\\"
    } else {
        "/"
    };
    for call in &state.calls {
        ctx.checkpoint()?;
        let timeout = ctx
            .remaining()
            .map(|left| u64::try_from(left.as_millis()).unwrap_or(u64::MAX));
        let spec = exec::program_spec(call, &defaults, cwd, timeout);
        let outcome = match process.run_program(&spec, ctx.cancel()) {
            Ok(outcome) => outcome,
            Err(error) if error.code() == ErrorCode::ProgramNotFound => {
                return Err(missing_manager(Some(state.manager)));
            }
            Err(error) => return Err(error.with_effect_applied(false)),
        };
        if outcome.cancelled || outcome.timed_out {
            ctx.checkpoint()?;
        }
        if outcome.exit_code != Some(0) {
            return Ok(Some(call_failure(call, &outcome)));
        }
    }
    Ok(None)
}

fn target_of(name: &str) -> String {
    format!("package:{name}")
}

fn change_marker(manager: PackageManager, count: usize) -> IrreversibleEntry {
    let mut marker = IrreversibleEntry::new(Irreversible::PackageChanged);
    marker.count = Some(count as u64);
    marker.detail = Some(manager.as_str().to_owned());
    marker
}

fn effect_record(
    action: Action,
    name: &str,
    status: EffectStatus,
    before: Option<String>,
    after: Option<String>,
) -> Effect {
    Effect {
        status,
        kind: format!("package.{}", action.as_str()),
        target: Some(target_of(name)),
        action: Some(action.as_str().to_owned()),
        before: before.map(Value::Text),
        after: after.map(Value::Text),
        ..Effect::default()
    }
}

fn call_failure(call: &Invocation, outcome: &ProgramOutcome) -> Error {
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    let mut tail_start = stderr.len().saturating_sub(STDERR_TAIL_BYTES);
    while !stderr.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let tail = stderr[tail_start..].trim();
    let status = outcome.exit_code.map_or_else(
        || "was stopped".to_owned(),
        |code| format!("exited with code {code}"),
    );
    Error::new(
        ErrorCode::StageFailed,
        format!("{} {status}: {tail}", call.program),
    )
}
