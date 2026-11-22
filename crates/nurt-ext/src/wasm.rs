// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! WebAssembly host of extensions (feature `wasm`).
//!
//! An extension component implements the world `extension` of the interface
//! `nurt:ext@1.0.0` (file `wit/nurt-ext.wit` of this crate): it exports
//! `nurt:ext/stage@1.0.0` and may import `nurt:ext/host@1.0.0`. Each call of
//! a command runs in a fresh instance with a bounded budget of execution
//! fuel and memory. The host calls reach the file system of the execution
//! context, and only below the directories granted by the manifest; file
//! writes are recorded for the plan of the effect, not performed.

use std::fmt;
use std::io::Read;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{Event, ExecCtx, LogEvent, LogLevel};
use wasmtime::component::{Component, ComponentType, InstancePre, Lift, Linker, Lower};
use wasmtime::{Config, Engine, Store, StoreContextMut, StoreLimits, StoreLimitsBuilder, Trap};

use crate::error::{error, io_error, stage_failed};
use crate::manifest::ExtKind;
use crate::policy::PathPolicy;
use crate::stage::{ExtRuntime, GuestCall, GuestOutput, PlannedWrite};
use crate::store::InstalledExt;

/// Name of the imported host interface.
pub const HOST_INTERFACE: &str = "nurt:ext/host@1.0.0";

/// Name of the exported stage interface.
pub const STAGE_INTERFACE: &str = "nurt:ext/stage@1.0.0";

/// Function of the stage interface that runs a command.
pub const RUN_FUNCTION: &str = "run";

/// Execution fuel of one call of a command; roughly one unit per executed
/// WebAssembly instruction.
pub const DEFAULT_FUEL: u64 = 10_000_000_000;

/// Largest linear memory of one instance, in bytes.
pub const MEMORY_LIMIT: usize = 512 * 1024 * 1024;

/// Error of a host call, the record `failure` of the host interface.
#[derive(ComponentType, Lift, Lower, Debug, Clone, PartialEq, Eq)]
#[component(record)]
struct Failure {
    code: String,
    message: String,
}

/// Severity of a message, the enum `log-level` of the host interface.
#[derive(ComponentType, Lift, Lower, Debug, Clone, Copy, PartialEq, Eq)]
#[component(enum)]
#[repr(u8)]
#[expect(
    dead_code,
    reason = "the variants are built by lifting values passed by the guest"
)]
enum GuestLogLevel {
    #[component(name = "error")]
    Error,
    #[component(name = "warn")]
    Warn,
    #[component(name = "info")]
    Info,
    #[component(name = "debug")]
    Debug,
    #[component(name = "trace")]
    Trace,
}

impl From<GuestLogLevel> for LogLevel {
    fn from(level: GuestLogLevel) -> Self {
        match level {
            GuestLogLevel::Error => Self::Error,
            GuestLogLevel::Warn => Self::Warn,
            GuestLogLevel::Info => Self::Info,
            GuestLogLevel::Debug => Self::Debug,
            GuestLogLevel::Trace => Self::Trace,
        }
    }
}

/// State of one instance: what the host calls may reach and what they
/// recorded.
struct HostState {
    ctx: ExecCtx,
    policy: PathPolicy,
    command: String,
    writes_allowed: bool,
    writes: Vec<PlannedWrite>,
    denied: Option<Error>,
    limits: StoreLimits,
}

impl HostState {
    fn refuse(&mut self, problem: Error) -> Failure {
        let failure = Failure {
            code: problem.code().as_str().to_owned(),
            message: problem.message.clone(),
        };
        if problem.code() == ErrorCode::Permission && self.denied.is_none() {
            self.denied = Some(problem);
        }
        failure
    }

    fn read_file(&mut self, path: &str) -> std::result::Result<Vec<u8>, Failure> {
        self.try_read_file(path)
            .map_err(|problem| self.refuse(problem))
    }

    fn try_read_file(&self, path: &str) -> Result<Vec<u8>> {
        let fs = self.ctx.require_fs()?;
        let path = self.policy.check_read(path, fs)?;
        let max = self.ctx.limits().max_file_bytes_text;
        let mut content = Vec::new();
        fs.open_read(&path)?
            .take(max.saturating_add(1))
            .read_to_end(&mut content)
            .map_err(|source| io_error("read", path.as_path(), &source))?;
        if u64::try_from(content.len()).unwrap_or(u64::MAX) > max {
            return Err(error(
                ErrorCode::Limit,
                format!("{} is larger than {max} bytes", path.as_path().display()),
            ));
        }
        Ok(content)
    }

    fn list_dir(&mut self, path: &str) -> std::result::Result<Vec<String>, Failure> {
        self.try_list_dir(path)
            .map_err(|problem| self.refuse(problem))
    }

    fn try_list_dir(&self, path: &str) -> Result<Vec<String>> {
        let fs = self.ctx.require_fs()?;
        let path = self.policy.check_read(path, fs)?;
        let mut names = Vec::new();
        for entry in fs.read_dir(&path)? {
            names.push(entry?.name.to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn write_file(&mut self, path: &str, content: Vec<u8>) -> std::result::Result<(), Failure> {
        match self.try_write_file(path) {
            Ok(path) => {
                self.writes.push(PlannedWrite { path, content });
                Ok(())
            }
            Err(problem) => Err(self.refuse(problem)),
        }
    }

    fn try_write_file(&self, path: &str) -> Result<nurt_contracts::fs::AbsPath> {
        if !self.writes_allowed {
            return Err(error(
                ErrorCode::Permission,
                format!(
                    "`{}` may not write files; declare it with `writes = true`",
                    self.command
                ),
            ));
        }
        self.policy.check_write(path, self.ctx.require_fs()?)
    }

    fn log(&self, level: GuestLogLevel, message: String) {
        self.ctx.emit(Event::Log(LogEvent {
            level: level.into(),
            message,
            source: Some(self.command.clone()),
        }));
    }
}

fn host_linker(engine: &Engine) -> wasmtime::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    {
        let mut host = linker.instance(HOST_INTERFACE)?;
        host.func_wrap(
            "read-file",
            |mut store: StoreContextMut<'_, HostState>, (path,): (String,)| {
                Ok((store.data_mut().read_file(&path),))
            },
        )?;
        host.func_wrap(
            "list-dir",
            |mut store: StoreContextMut<'_, HostState>, (path,): (String,)| {
                Ok((store.data_mut().list_dir(&path),))
            },
        )?;
        host.func_wrap(
            "write-file",
            |mut store: StoreContextMut<'_, HostState>, (path, content): (String, Vec<u8>)| {
                Ok((store.data_mut().write_file(&path, content),))
            },
        )?;
        host.func_wrap(
            "log",
            |store: StoreContextMut<'_, HostState>, (level, message): (GuestLogLevel, String)| {
                store.data().log(level, message);
                Ok(())
            },
        )?;
    }
    Ok(linker)
}

/// Compiler and runtime of extension components, shared by all extensions.
///
/// ```no_run
/// use nurt_ext::wasm::WasmHost;
///
/// let host = WasmHost::new()?;
/// let component = std::fs::read("acme.wasm").map_err(|_| "unreadable")?;
/// let extension = host.compile("acme", &component)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct WasmHost {
    engine: Engine,
    fuel: u64,
}

impl fmt::Debug for WasmHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasmHost")
            .field("fuel", &self.fuel)
            .finish_non_exhaustive()
    }
}

impl WasmHost {
    /// Host with the default fuel of [`DEFAULT_FUEL`] per call.
    ///
    /// # Errors
    ///
    /// Fails with `E_INTERNAL` when the WebAssembly engine cannot start on
    /// this platform.
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|problem| {
            error(
                ErrorCode::Internal,
                format!("the WebAssembly engine cannot start: {problem:#}"),
            )
        })?;
        Ok(Self {
            engine,
            fuel: DEFAULT_FUEL,
        })
    }

    /// Sets the execution fuel of one call of a command.
    #[must_use]
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    /// Loads the component of an installed extension.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNSUPPORTED` for an extension that is not a component,
    /// with `E_IO` or `E_NOT_FOUND` when the file cannot be read, and as
    /// [`WasmHost::compile`].
    pub fn load(&self, extension: &InstalledExt) -> Result<WasmExtension> {
        let path = match (extension.manifest.ext.kind, extension.artifact()) {
            (ExtKind::Wasm, Some(path)) => path,
            _ => {
                return Err(error(
                    ErrorCode::Unsupported,
                    format!("`{}` is not a WebAssembly extension", extension.name()),
                ));
            }
        };
        let bytes = std::fs::read(&path).map_err(|source| io_error("read", &path, &source))?;
        self.compile(extension.name(), &bytes)
    }

    /// Compiles a component, in binary or text form, of the extension
    /// `name` and links it to the host interface.
    ///
    /// # Errors
    ///
    /// Fails with `E_CATALOG` when the bytes are not a valid component or
    /// the component imports something the host does not provide.
    pub fn compile(&self, name: &str, component: &[u8]) -> Result<WasmExtension> {
        let invalid = |problem: wasmtime::Error| {
            error(
                ErrorCode::Catalog,
                format!("the component of extension `{name}` is not usable: {problem:#}"),
            )
        };
        let component = Component::new(&self.engine, component).map_err(invalid)?;
        let linker = host_linker(&self.engine).map_err(invalid)?;
        let pre = linker.instantiate_pre(&component).map_err(invalid)?;
        Ok(WasmExtension {
            name: name.to_owned(),
            engine: self.engine.clone(),
            pre,
            fuel: self.fuel,
        })
    }
}

/// A compiled extension component, ready to run commands.
pub struct WasmExtension {
    name: String,
    engine: Engine,
    pre: InstancePre<HostState>,
    fuel: u64,
}

impl fmt::Debug for WasmExtension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasmExtension")
            .field("name", &self.name)
            .field("fuel", &self.fuel)
            .finish_non_exhaustive()
    }
}

type RunParams = (String, String, Vec<String>);
type RunResults = (std::result::Result<Vec<String>, String>,);

impl WasmExtension {
    /// Name of the extension.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    fn failure(command: &str, problem: &wasmtime::Error) -> Error {
        if matches!(problem.downcast_ref::<Trap>(), Some(Trap::OutOfFuel)) {
            return error(
                ErrorCode::Limit,
                format!("`{command}` used up its execution budget"),
            );
        }
        stage_failed(command, format!("{problem:#}"))
    }

    fn missing_export(&self, item: &str) -> Error {
        error(
            ErrorCode::Catalog,
            format!(
                "the component of extension `{}` does not export {item}",
                self.name
            ),
        )
    }
}

impl ExtRuntime for WasmExtension {
    fn run(&self, call: GuestCall<'_>, ctx: &ExecCtx) -> Result<GuestOutput> {
        let command = call.command;
        let state = HostState {
            ctx: ctx.clone(),
            policy: call.policy.clone(),
            command: command.to_owned(),
            writes_allowed: call.writes,
            writes: Vec::new(),
            denied: None,
            limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.fuel)
            .map_err(|problem| Self::failure(command, &problem))?;
        let instance = self
            .pre
            .instantiate(&mut store)
            .map_err(|problem| Self::failure(command, &problem))?;
        let stage = instance
            .get_export_index(&mut store, None, STAGE_INTERFACE)
            .ok_or_else(|| self.missing_export(STAGE_INTERFACE))?;
        let run = instance
            .get_export_index(&mut store, Some(&stage), RUN_FUNCTION)
            .ok_or_else(|| self.missing_export(RUN_FUNCTION))?;
        let func = instance
            .get_typed_func::<RunParams, RunResults>(&mut store, &run)
            .map_err(|problem| Self::failure(command, &problem))?;
        let (result,) = func
            .call(&mut store, (command.to_owned(), call.args, call.input))
            .map_err(|problem| Self::failure(command, &problem))?;
        let state = store.into_data();
        match result {
            Ok(records) => Ok(GuestOutput {
                records,
                writes: state.writes,
            }),
            Err(message) => Err(state
                .denied
                .unwrap_or_else(|| stage_failed(command, message))),
        }
    }
}
