// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `sysctl.set`, `modules.load` and `modules.unload`.
//!
//! | Command | Linux | macOS |
//! |---|---|---|
//! | `sysctl.set` | `sysctl -n` and `sysctl -w`; `persistent: true` edits `/etc/sysctl.d/90-nurt.conf` | `sysctl -n` and `sysctl -w` |
//! | `modules.load`, `modules.unload` | `lsmod`, `modprobe`, `modprobe -r` | not available |

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::FileOp;
use nurt_contracts::stage::BoundArgs;

use super::sys::{Call, Sys};
use super::{Plan, StateChange, args};

/// File that holds the persistent values of `sysctl.set`.
pub(crate) const SYSCTL_FILE: &str = "/etc/sysctl.d/90-nurt.conf";

/// State text of a loaded module.
pub(crate) const LOADED: &str = "loaded";
/// State text of a module that is not loaded.
pub(crate) const UNLOADED: &str = "unloaded";

pub(crate) fn plan(sys: &dyn Sys, args: &BoundArgs) -> Result<Plan> {
    match args.command() {
        "sysctl.set" => sysctl_set(sys, args),
        "modules.load" => module(sys, args, true),
        _ => module(sys, args, false),
    }
}

fn sysctl_set(sys: &dyn Sys, args: &BoundArgs) -> Result<Plan> {
    let key = args::required(args, "key")?;
    let value = args::required(args, "value")?;
    check_key(&key)?;
    if value.chars().any(char::is_control) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the value of a kernel parameter must not hold a control character",
        ));
    }
    let persistent = args::flag(args, "persistent", false)?;
    if persistent && sys.os() != Os::Linux {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "`persistent` of `sysctl.set` is available on Linux only",
        ));
    }
    let output = sys.run(&Call::new("sysctl", ["-n", &key]))?;
    if !output.ok() {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!("there is no kernel parameter `{key}`"),
        ));
    }
    let before = output.stdout.trim().to_owned();
    let mut files = Vec::new();
    if persistent {
        let text = sys.read_file(SYSCTL_FILE)?.unwrap_or_default();
        let edited = sysctl_file_set(&text, &key, &value)?;
        if edited != text {
            let path = super::system_file(SYSCTL_FILE)?;
            files.push(FileOp::write(path, edited.into_bytes()));
        }
    }
    if same_value(&before, &value) && files.is_empty() {
        return Ok(Plan {
            unchanged: Some(format!("`{key}` is already `{before}`")),
            ..Plan::default()
        });
    }
    let calls = if same_value(&before, &value) {
        Vec::new()
    } else {
        vec![Call::new(
            "sysctl",
            ["-w".to_owned(), format!("{key}={value}")],
        )]
    };
    let changes = if calls.is_empty() {
        Vec::new()
    } else {
        vec![StateChange::new(
            format!("sysctl:{key}"),
            Some(before),
            Some(value),
        )]
    };
    Ok(Plan {
        calls,
        files,
        changes,
        ..Plan::default()
    })
}

fn same_value(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

fn check_key(key: &str) -> Result<()> {
    let good = !key.is_empty()
        && !key.starts_with(['-', '.', '/'])
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if good {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{key}` is not a kernel parameter name"),
        )
        .with_found(key.to_owned()))
    }
}

/// The text of `/etc/sysctl.d/90-nurt.conf` with the line of `key` set to
/// `value`; a key without a line gets one at the end.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` when the key is not a valid name.
pub(crate) fn sysctl_file_set(text: &str, key: &str, value: &str) -> Result<String> {
    check_key(key)?;
    let line = format!("{key} = {value}");
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
    for existing in text.lines() {
        if conf_key(existing) == Some(key) {
            if !replaced {
                out.push(line.clone());
                replaced = true;
            }
        } else {
            out.push(existing.to_owned());
        }
    }
    if !replaced {
        out.push(line);
    }
    Ok(format!("{}\n", out.join("\n")))
}

fn conf_key(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
        return None;
    }
    line.split_once('=').map(|(key, _)| key.trim())
}

fn module(sys: &dyn Sys, args: &BoundArgs, load: bool) -> Result<Plan> {
    if sys.os() != Os::Linux {
        return Err(Error::new(
            ErrorCode::OsOnly,
            "kernel modules are managed on Linux only",
        ));
    }
    let name = args::required(args, "name")?;
    args::name("module name", &name, false)?;
    let params = args::list(args, "params")?.unwrap_or_default();
    for param in &params {
        args::operand("module parameter", param)?;
    }
    let loaded = is_loaded(sys, &name)?;
    if loaded == load {
        let state = if load { "already loaded" } else { "not loaded" };
        return Ok(Plan {
            unchanged: Some(format!("the module `{name}` is {state}")),
            ..Plan::default()
        });
    }
    let call = if load {
        let mut call = Call::new("modprobe", [name.as_str()]);
        call.args.extend(params);
        call
    } else {
        Call::new("modprobe", ["-r", name.as_str()])
    };
    let (before, after) = if load {
        (UNLOADED, LOADED)
    } else {
        (LOADED, UNLOADED)
    };
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(
            format!("module:{name}"),
            Some(before.to_owned()),
            Some(after.to_owned()),
        )],
        ..Plan::default()
    })
}

/// The module is listed by `lsmod`; `-` and `_` are the same character in the
/// name of a module.
pub(crate) fn is_loaded(sys: &dyn Sys, name: &str) -> Result<bool> {
    let output = sys.run(&Call::new("lsmod", Vec::<String>::new()))?;
    let wanted = name.replace('-', "_");
    Ok(output
        .stdout
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .any(|module| module == wanted))
}
