// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Native extension libraries (feature `native`, configuration key
//! `ext.native`).
//!
//! A native extension is a shared library with three C functions:
//!
//! ```c
//! uint32_t nurt_ext_api(void);
//! int32_t nurt_ext_run(const uint8_t *request, size_t request_len,
//!                      uint8_t **response, size_t *response_len);
//! void nurt_ext_free(uint8_t *response, size_t response_len);
//! ```
//!
//! `nurt_ext_api` returns the API version as `MAJOR << 16 | MINOR`. The
//! request is the JSON object `{"command", "args", "input"}` with the
//! arguments and the input records in the form of [`crate::wire`]; the
//! response, allocated by the library and released with `nurt_ext_free`, is
//! `{"records": [...]}` for status 0 and `{"error": "<message>"}` otherwise.
//! Native code runs with the rights of the process: the host cannot limit
//! the paths it reaches, so commands that write files are not supported and
//! native libraries load only when the configuration allows them.
#![allow(
    unsafe_code,
    reason = "a native extension is foreign code reached through its C functions"
)]

use std::fmt;
use std::path::Path;

use libloading::Library;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::stage::ExecCtx;
use serde_json::{Map, Value as Json};

use crate::error::{error, stage_failed};
use crate::locations::ExtLocations;
use crate::manifest::{ApiVersion, ExtKind, HOST_API};
use crate::stage::{ExtRuntime, GuestCall, GuestOutput};
use crate::store::InstalledExt;

/// Symbol returning the API version of the library.
pub const API_SYMBOL: &str = "nurt_ext_api";
/// Symbol running a command.
pub const RUN_SYMBOL: &str = "nurt_ext_run";
/// Symbol releasing a response.
pub const FREE_SYMBOL: &str = "nurt_ext_free";

/// Bits of the minor version in the value of `nurt_ext_api`.
const MINOR_BITS: u32 = 16;
/// Mask of the minor version in the value of `nurt_ext_api`.
const MINOR_MASK: u32 = 0xFFFF;

type ApiFn = unsafe extern "C" fn() -> u32;
type RunFn = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);

/// A loaded native extension library.
pub struct NativeExtension {
    name: String,
    run: RunFn,
    free: FreeFn,
    _library: Library,
}

impl fmt::Debug for NativeExtension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeExtension")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

fn unusable(name: &str, problem: impl fmt::Display) -> nurt_contracts::error::Error {
    error(
        ErrorCode::Catalog,
        format!("the library of extension `{name}` is not usable: {problem}"),
    )
}

impl NativeExtension {
    /// Loads the library of an installed native extension.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNSUPPORTED` when native libraries are not allowed, the
    /// extension is not native or a command writes files, and as
    /// [`NativeExtension::open`].
    pub fn load(extension: &InstalledExt, locations: &ExtLocations) -> Result<Self> {
        if !locations.native_allowed() {
            return Err(error(
                ErrorCode::Unsupported,
                "native extensions are disabled; set `ext.native = true` to allow them",
            ));
        }
        let path = match (extension.manifest.ext.kind, extension.artifact()) {
            (ExtKind::Native, Some(path)) => path,
            _ => {
                return Err(error(
                    ErrorCode::Unsupported,
                    format!("`{}` is not a native extension", extension.name()),
                ));
            }
        };
        if let Some(stage) = extension.manifest.stage.iter().find(|stage| stage.writes) {
            return Err(error(
                ErrorCode::Unsupported,
                format!(
                    "`{}.{}` writes files, which native extensions cannot do; use a WebAssembly component",
                    extension.name(),
                    stage.name
                ),
            ));
        }
        Self::open(extension.name(), &path)
    }

    /// Opens the library at `path` for the extension `name` and checks its
    /// API version.
    ///
    /// # Errors
    ///
    /// Fails with `E_CATALOG` when the file is not a library, lacks one of
    /// the functions or implements another API version.
    pub fn open(name: &str, path: &Path) -> Result<Self> {
        // SAFETY: loading a library runs its initialisers. The library is
        // installed in the extension directory and loads only when the
        // configuration allows native extensions.
        let library = unsafe { Library::new(path) }.map_err(|problem| unusable(name, problem))?;
        // SAFETY: the symbols have the C signatures documented for native
        // extensions; the function pointers are kept together with the
        // library, which stays loaded as long as they are used.
        let (api, run, free) = unsafe {
            let api = *library
                .get::<ApiFn>(API_SYMBOL)
                .map_err(|problem| unusable(name, problem))?;
            let run = *library
                .get::<RunFn>(RUN_SYMBOL)
                .map_err(|problem| unusable(name, problem))?;
            let free = *library
                .get::<FreeFn>(FREE_SYMBOL)
                .map_err(|problem| unusable(name, problem))?;
            (api, run, free)
        };
        // SAFETY: `nurt_ext_api` takes no arguments and returns a number.
        let version = unsafe { api() };
        let required = ApiVersion {
            major: version >> MINOR_BITS,
            minor: version & MINOR_MASK,
        };
        if !HOST_API.accepts(required) {
            return Err(unusable(
                name,
                format!("it requires {required}, the host implements {HOST_API}"),
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            run,
            free,
            _library: library,
        })
    }

    fn call(&self, request: &[u8]) -> (i32, Vec<u8>) {
        let mut response: *mut u8 = std::ptr::null_mut();
        let mut length: usize = 0;
        // SAFETY: the request buffer is valid for its length during the
        // call; the library stores a buffer of `length` bytes in `response`
        // or leaves it null.
        let status = unsafe {
            (self.run)(
                request.as_ptr(),
                request.len(),
                &raw mut response,
                &raw mut length,
            )
        };
        if response.is_null() {
            return (status, Vec::new());
        }
        // SAFETY: the library returned `length` initialised bytes at
        // `response`; they are copied before the buffer is released with the
        // function of the same library, exactly once.
        let bytes = unsafe {
            let bytes = std::slice::from_raw_parts(response, length).to_vec();
            (self.free)(response, length);
            bytes
        };
        (status, bytes)
    }
}

fn decode(command: &str, status: i32, bytes: &[u8]) -> Result<Vec<String>> {
    let invalid = || stage_failed(command, "the library returned an invalid response");
    let Ok(Json::Object(mut response)) = serde_json::from_slice::<Json>(bytes) else {
        return Err(invalid());
    };
    if status != 0 {
        let message = match response.remove("error") {
            Some(Json::String(message)) => message,
            _ => format!("the library returned status {status}"),
        };
        return Err(stage_failed(command, message));
    }
    match response.remove("records") {
        Some(Json::Array(records)) => Ok(records.iter().map(Json::to_string).collect()),
        _ => Err(invalid()),
    }
}

impl ExtRuntime for NativeExtension {
    fn run(&self, call: GuestCall<'_>, _ctx: &ExecCtx) -> Result<GuestOutput> {
        let parse = |text: &str| serde_json::from_str::<Json>(text).unwrap_or(Json::Null);
        let mut request = Map::new();
        request.insert("command".to_owned(), Json::from(call.command));
        request.insert("args".to_owned(), parse(&call.args));
        request.insert(
            "input".to_owned(),
            Json::Array(call.input.iter().map(|record| parse(record)).collect()),
        );
        let request = Json::Object(request).to_string();
        let (status, bytes) = self.call(request.as_bytes());
        let records = decode(call.command, status, &bytes)?;
        Ok(GuestOutput {
            records,
            writes: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::{NativeExtension, decode};
    use crate::locations::ExtLocations;
    use crate::manifest::Manifest;
    use crate::store::InstalledExt;

    fn extension(kind: &str, writes: bool) -> InstalledExt {
        let file = if kind == "native" { "library" } else { "wasm" };
        let text = format!(
            "[ext]\nname = \"acme\"\nversion = \"1.0.0\"\napi = \"nurt:ext@1.0\"\nkind = \"{kind}\"\n{file} = \"acme.so\"\n\n[[stage]]\nname = \"lint\"\ninput = [\"entry\"]\nwrites = {writes}\n\n[permissions]\nwrite = [\"/x\"]\n"
        );
        InstalledExt {
            manifest: Manifest::parse(&text).unwrap(),
            dir: std::env::temp_dir().join("nurt-native-test"),
        }
    }

    #[test]
    fn native_libraries_need_permission_and_a_read_only_manifest() {
        let allowed = ExtLocations::new(std::env::temp_dir()).with_native(true);
        let denied = ExtLocations::new(std::env::temp_dir());
        let code =
            |result: nurt_contracts::error::Result<NativeExtension>| result.unwrap_err().code();
        assert_eq!(
            code(NativeExtension::load(&extension("native", false), &denied)),
            ErrorCode::Unsupported
        );
        assert_eq!(
            code(NativeExtension::load(&extension("wasm", false), &allowed)),
            ErrorCode::Unsupported
        );
        assert_eq!(
            code(NativeExtension::load(&extension("native", true), &allowed)),
            ErrorCode::Unsupported
        );
        assert_eq!(
            code(NativeExtension::load(&extension("native", false), &allowed)),
            ErrorCode::Catalog
        );
    }

    #[test]
    fn files_that_are_not_libraries_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("acme.so");
        std::fs::write(&path, b"not a library").unwrap();
        let error = NativeExtension::open("acme", &path).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Catalog);
        assert!(error.message.contains("acme"));
    }

    #[test]
    fn responses_are_decoded_by_status() {
        let records = decode("acme.lint", 0, br#"{"records":[{"kind":"item"}]}"#).unwrap();
        assert_eq!(records, [r#"{"kind":"item"}"#]);
        let failed = decode("acme.lint", 1, br#"{"error":"bad input"}"#).unwrap_err();
        assert_eq!(failed.code(), ErrorCode::StageFailed);
        assert!(failed.message.contains("bad input"));
        let status = decode("acme.lint", 2, b"{}").unwrap_err();
        assert!(status.message.contains("status 2"));
        assert!(decode("acme.lint", 0, b"[]").is_err());
        assert!(decode("acme.lint", 0, b"{}").is_err());
    }
}
