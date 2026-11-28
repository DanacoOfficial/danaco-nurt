// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/build.rs
// Responsibility:   Build script of the setup wizard: embeds the release archive with its SHA-256 sum and compiles the Windows resources.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Build script of the setup wizard.
//!
//! The program archive of the release (`nurt-<version>-<target>.zip`) is named by
//! the variable `NURT_SETUP_PAYLOAD`. The script copies it to the build directory,
//! from where `src/payload.rs` embeds it, and writes its SHA-256 sum next to it.
//! When `NURT_SETUP_PAYLOAD_SHA256` is set, the sum of the archive must equal it,
//! so that the sum recorded in `checksums.txt` and the embedded archive cannot
//! differ. Without `NURT_SETUP_PAYLOAD` an empty archive is embedded: the wizard
//! builds and starts, and the requirements step reports the missing archive.
//!
//! On a Windows target the script compiles `setup.rc` (icon, manifest, version
//! information) with `llvm-rc` or `rc` and links the result into the executable.
//! The compiler is the first of `NURT_RC`, `llvm-rc` and `rc` that is found.

#![allow(
    clippy::print_stdout,
    reason = "a build script talks to cargo through lines on the standard output"
)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use sha2::{Digest, Sha256};

const COMPILERS: [&str; 2] = ["llvm-rc", "rc"];

fn fail(message: &str) -> ! {
    println!("cargo::error={message}");
    exit(1);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_env(name: &str) -> Option<String> {
    env::var(name).ok()
}

fn main() {
    for name in ["NURT_SETUP_PAYLOAD", "NURT_SETUP_PAYLOAD_SHA256", "NURT_RC"] {
        println!("cargo::rerun-if-env-changed={name}");
    }
    for name in [
        "build.rs",
        "setup.manifest",
        "setup.rc.in",
        "../../branding/icons/windows/nurt.ico",
    ] {
        println!("cargo::rerun-if-changed={name}");
    }
    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        fail("OUT_DIR is not set");
    };
    embed_payload(&out_dir);
    if read_env("CARGO_CFG_TARGET_OS").as_deref() == Some("windows")
        && read_env("CARGO_CFG_TARGET_ENV").as_deref() == Some("msvc")
    {
        compile_resources(&out_dir);
    }
}

fn embed_payload(out_dir: &Path) {
    let payload = match env::var_os("NURT_SETUP_PAYLOAD") {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo::rerun-if-changed={}", path.display());
            match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => fail(&format!(
                    "cannot read the archive {}: {error}",
                    path.display()
                )),
            }
        }
        None => Vec::new(),
    };
    let sum = hex(&Sha256::digest(&payload));
    if let Some(expected) = read_env("NURT_SETUP_PAYLOAD_SHA256")
        && !expected.trim().eq_ignore_ascii_case(&sum)
    {
        fail(&format!(
            "the SHA-256 sum of the archive is {sum}, not {}",
            expected.trim()
        ));
    }
    if let Err(error) = fs::write(out_dir.join("payload.zip"), &payload) {
        fail(&format!(
            "cannot write the archive to the build directory: {error}"
        ));
    }
    if let Err(error) = fs::write(out_dir.join("payload.sha256"), &sum) {
        fail(&format!(
            "cannot write the sum to the build directory: {error}"
        ));
    }
}

/// Version of the package as `1,2,3,0`, `1.2.3.0` and `1.2.3`.
fn version_forms() -> (String, String, String) {
    let part = |name: &str| {
        read_env(name)
            .and_then(|text| text.parse::<u16>().ok())
            .unwrap_or(0)
    };
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    (
        format!("{major},{minor},{patch},0"),
        format!("{major}.{minor}.{patch}.0"),
        format!("{major}.{minor}.{patch}"),
    )
}

fn compile_resources(out_dir: &Path) {
    let Some(manifest_dir) = read_env("CARGO_MANIFEST_DIR").map(PathBuf::from) else {
        fail("CARGO_MANIFEST_DIR is not set");
    };
    let (comma, quad, dotted) = version_forms();
    let copy = |from: &str, to: &str, replace: bool| {
        let text = fs::read_to_string(manifest_dir.join(from))
            .unwrap_or_else(|error| fail(&format!("{from}: {error}")));
        let text = if replace {
            text.replace("@VERSION_QUAD@", &quad)
                .replace("@VERSION_COMMA@", &comma)
                .replace("@VERSION@", &dotted)
        } else {
            text
        };
        fs::write(out_dir.join(to), text).unwrap_or_else(|error| fail(&format!("{to}: {error}")));
    };
    copy("setup.rc.in", "setup.rc", true);
    copy("setup.manifest", "setup.manifest", false);
    fs::copy(
        manifest_dir.join("../../branding/icons/windows/nurt.ico"),
        out_dir.join("nurt.ico"),
    )
    .unwrap_or_else(|error| fail(&format!("nurt.ico: {error}")));

    let Some(compiler) = find_compiler() else {
        fail("no resource compiler found: install llvm-rc or rc.exe, or set NURT_RC");
    };
    let resource = out_dir.join("setup.res");
    let name = compiler
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();
    let mut command = Command::new(&compiler);
    command.current_dir(out_dir);
    // llvm-rc takes the options with a hyphen, because a leading slash is a path on Linux.
    if name.starts_with("llvm-rc") {
        command.arg("-no-preprocess").arg("-FO");
    } else {
        command.arg("/nologo").arg("/fo");
    }
    command.arg(&resource).arg("setup.rc");
    match command.output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => fail(&format!(
            "{} failed: {}{}",
            compiler.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )),
        Err(error) => fail(&format!("{}: {error}", compiler.display())),
    }
    println!("cargo::rustc-link-arg-bins={}", resource.display());
    // The static C runtime of `.cargo/config.toml` (`crt-static`): a cross build links no default
    // libraries, so the three libraries of the runtime are named here.
    if read_env("CARGO_CFG_TARGET_FEATURE")
        .is_some_and(|features| features.split(',').any(|feature| feature == "crt-static"))
    {
        for library in ["libcmt", "libvcruntime", "libucrt"] {
            println!("cargo::rustc-link-lib=static={library}");
        }
    }
}

fn find_compiler() -> Option<PathBuf> {
    if let Some(named) = read_env("NURT_RC") {
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
