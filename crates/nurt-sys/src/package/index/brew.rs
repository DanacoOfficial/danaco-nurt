// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Homebrew API cache (`api/formula.jws.json`, `api/cask.jws.json`).

use std::fs::File;
use std::io::BufReader;

use nurt_programs::sources::brew::cache_dir;
use serde_json::Value;

use super::{EntrySink, IndexEntry, IndexReport};
use crate::package::root::HostEnv;

const API_DIR: &str = "api";
const FORMULA_FILE: &str = "formula.jws.json";
const CASK_FILE: &str = "cask.jws.json";
const FORMULA_SOURCE: &str = "homebrew/core";
const CASK_SOURCE: &str = "homebrew/cask";

pub(super) fn read(env: &HostEnv, sink: EntrySink<'_>, report: &mut IndexReport) {
    let Some(cache) = cache_dir(env) else {
        report.warn("no_index", "the Homebrew cache directory is unknown");
        return;
    };
    for (file, source, is_cask) in [
        (FORMULA_FILE, FORMULA_SOURCE, false),
        (CASK_FILE, CASK_SOURCE, true),
    ] {
        let path = env.sys_path(cache.join(API_DIR).join(file));
        if !path.is_file() {
            continue;
        }
        report.sources_found += 1;
        match read_payload(&path) {
            Ok(items) => {
                for item in &items {
                    if let Some(entry) = entry(item, source, is_cask) {
                        sink(entry);
                    }
                }
                report.sources_read += 1;
            }
            Err(detail) => report.warn("corrupt_index", format!("{file}: {detail}")),
        }
    }
}

fn read_payload(path: &std::path::Path) -> Result<Vec<Value>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let document: Value =
        serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
    let payload = document.get("payload").unwrap_or(&document);
    let items = match payload {
        Value::String(text) => serde_json::from_str(text).map_err(|error| error.to_string())?,
        other => other.clone(),
    };
    match items {
        Value::Array(items) => Ok(items),
        _ => Err("the payload is not a list".to_owned()),
    }
}

fn entry(item: &Value, source: &str, is_cask: bool) -> Option<IndexEntry> {
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    if is_cask {
        let token = text(item.get("token"))?;
        let display = item
            .get("name")
            .and_then(Value::as_array)
            .and_then(|names| names.first())
            .and_then(Value::as_str)
            .map(str::to_owned);
        return Some(IndexEntry {
            name: token,
            version: text(item.get("version"))?,
            description: text(item.get("desc")).or(display),
            source: source.to_owned(),
            ..IndexEntry::default()
        });
    }
    let stable = text(
        item.get("versions")
            .and_then(|versions| versions.get("stable")),
    )?;
    let revision = item.get("revision").and_then(Value::as_u64).unwrap_or(0);
    let version = if revision > 0 {
        format!("{stable}_{revision}")
    } else {
        stable
    };
    Some(IndexEntry {
        name: text(item.get("name"))?,
        version,
        description: text(item.get("desc")),
        source: source.to_owned(),
        ..IndexEntry::default()
    })
}
