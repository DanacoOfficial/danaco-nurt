// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Build script of the command line program.
//!
//! For a Windows target that links with the MSVC toolchain it embeds the icon
//! (the file `windows/nurt.ico`, when it exists), the version information and
//! the application manifest into `nurt.exe`. The manifest declares the
//! execution level `asInvoker` (the program runs with the rights of the
//! terminal that starts it, so an elevated terminal gives it administrator
//! rights), long path support and the UTF-8 code page.
//!
//! The resource compiler is `llvm-rc` or `rc.exe`; the variable `NURT_RC`
//! names a different one. When no compiler is found the build continues
//! without the resources, unless `NURT_REQUIRE_RESOURCES` is set to `1`, which
//! the release build does.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name of the resource compiler that is searched when `NURT_RC` is not set.
const COMPILERS: [&str; 2] = ["llvm-rc", "rc"];

fn main() -> Result<(), String> {
    let mut out = std::io::stdout();
    for directive in [
        "cargo:rerun-if-changed=build.rs",
        "cargo:rerun-if-changed=windows",
        "cargo:rerun-if-env-changed=NURT_RC",
        "cargo:rerun-if-env-changed=NURT_REQUIRE_RESOURCES",
    ] {
        writeln!(out, "{directive}").map_err(|error| error.to_string())?;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return Ok(());
    }

    let required = env::var("NURT_REQUIRE_RESOURCES").is_ok_and(|value| value == "1");
    match compile_resources() {
        Ok(resource) => {
            writeln!(out, "cargo:rustc-link-arg-bins={}", resource.display())
                .map_err(|error| error.to_string())?;
        }
        Err(message) if required => return Err(format!("Windows resources: {message}")),
        Err(message) => {
            writeln!(
                out,
                "cargo:warning=nurt.exe is built without Windows resources: {message}"
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Version of the package in the forms the resource files need.
struct Version {
    /// `1.2.3`.
    dotted: String,
    /// `1.2.3.0`, the form of the manifest.
    quad: String,
    /// `1,2,3,0`, the form of the `VERSIONINFO` fixed fields.
    comma: String,
}

impl Version {
    fn of_package() -> Self {
        let part = |name: &str| {
            env::var(name)
                .ok()
                .and_then(|text| text.parse::<u16>().ok())
                .unwrap_or(0)
        };
        let (major, minor, patch) = (
            part("CARGO_PKG_VERSION_MAJOR"),
            part("CARGO_PKG_VERSION_MINOR"),
            part("CARGO_PKG_VERSION_PATCH"),
        );
        Self {
            dotted: format!("{major}.{minor}.{patch}"),
            quad: format!("{major}.{minor}.{patch}.0"),
            comma: format!("{major},{minor},{patch},0"),
        }
    }

    fn fill(&self, template: &str) -> String {
        template
            .replace("@VERSION_QUAD@", &self.quad)
            .replace("@VERSION_COMMA@", &self.comma)
            .replace("@VERSION@", &self.dotted)
    }
}

/// Writes the resource files into the output directory and compiles them.
/// Returns the path of the compiled resource file.
fn compile_resources() -> Result<PathBuf, String> {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|error| error.to_string())?);
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|error| error.to_string())?);
    let source = manifest_dir.join("windows");
    let version = Version::of_package();

    let icon = source.join("nurt.ico");
    let has_icon = icon.is_file();
    if has_icon {
        fs::copy(&icon, out_dir.join("nurt.ico")).map_err(|error| format!("nurt.ico: {error}"))?;
    }
    let icon_line = if has_icon { r#"1 ICON "nurt.ico""# } else { "" };
    for (template, name) in [
        ("nurt.rc.in", "nurt.rc"),
        ("nurt.manifest.in", "nurt.manifest"),
    ] {
        let text = fs::read_to_string(source.join(template))
            .map_err(|error| format!("{template}: {error}"))?;
        let text = version.fill(&text).replace("@ICON@", icon_line);
        fs::write(out_dir.join(name), text).map_err(|error| format!("{name}: {error}"))?;
    }

    let compiler = find_compiler().ok_or_else(|| {
        "no resource compiler found; install llvm-rc or rc.exe, or set NURT_RC".to_string()
    })?;
    let resource = out_dir.join("nurt.res");
    run_compiler(&compiler, &out_dir, &resource)?;
    Ok(resource)
}

/// Resource compiler named by `NURT_RC` or the first one found in `PATH`.
fn find_compiler() -> Option<PathBuf> {
    if let Ok(named) = env::var("NURT_RC") {
        return Some(PathBuf::from(named));
    }
    let path = env::var_os("PATH")?;
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    COMPILERS.iter().find_map(|name| {
        env::split_paths(&path)
            .map(|directory| directory.join(format!("{name}{suffix}")))
            .find(|candidate| candidate.is_file())
    })
}

/// Runs the compiler in `work` on `nurt.rc` and writes `resource`.
fn run_compiler(compiler: &Path, work: &Path, resource: &Path) -> Result<(), String> {
    let name = compiler
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let mut command = Command::new(compiler);
    command.current_dir(work);
    // llvm-rc takes the options with a hyphen, because a leading slash is a
    // path on Linux; rc.exe takes them with a slash.
    if name.starts_with("llvm-rc") {
        command.arg("-no-preprocess").arg("-FO");
    } else {
        command.arg("/nologo").arg("/fo");
    }
    command.arg(resource).arg("nurt.rc");
    let output = command
        .output()
        .map_err(|error| format!("{}: {error}", compiler.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed: {}{}",
            compiler.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
