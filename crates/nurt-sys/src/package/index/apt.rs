// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::package::index::apt
// Responsibility:   APT package lists (`/var/lib/apt/lists/*_Packages*`).
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! APT package lists (`/var/lib/apt/lists/*_Packages*`).

use std::collections::BTreeSet;
use std::fs;
use std::io::BufReader;

use super::{EntrySink, IndexEntry, IndexReport};
use crate::package::compress::open_decoded;
use crate::package::control::for_each_stanza;
use crate::package::root::{HostEnv, SysPaths};

const LISTS_DIR: &str = "/var/lib/apt/lists";
const ARCHITECTURES_FILE: &str = "/var/lib/dpkg/arch";
const ARCHITECTURE_ALL: &str = "all";
const PACKAGES_MARKER: &str = "_Packages";
const KIBIBYTE: u64 = 1024;
const EXTENSIONS: [&str; 6] = ["", ".gz", ".xz", ".lz4", ".bz2", ".zst"];

pub(super) fn read(env: &HostEnv, sink: EntrySink<'_>, report: &mut IndexReport) {
    let architectures = architectures(env);
    for path in env.list_dir(LISTS_DIR) {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(source) = list_source(file_name) else {
            continue;
        };
        report.sources_found += 1;
        let decoded = match open_decoded(&path) {
            Ok(decoded) => decoded,
            Err(error) => {
                report.warn("corrupt_index", format!("{file_name}: {error}"));
                continue;
            }
        };
        let result = for_each_stanza(BufReader::new(decoded), |stanza| {
            let (Some(name), Some(version)) = (stanza.get("Package"), stanza.get("Version")) else {
                return;
            };
            let architecture = stanza.get("Architecture");
            if let (Some(arch), Some(allowed)) = (architecture, &architectures)
                && !allowed.contains(arch)
            {
                return;
            }
            sink(IndexEntry {
                name: name.to_owned(),
                version: version.to_owned(),
                architecture: architecture.map(str::to_owned),
                description: stanza.get("Description").map(str::to_owned),
                size: stanza
                    .get("Installed-Size")
                    .and_then(|size| size.parse::<u64>().ok())
                    .map(|kibibytes| kibibytes.saturating_mul(KIBIBYTE)),
                source: source.clone(),
                phased: stanza
                    .get("Phased-Update-Percentage")
                    .and_then(|percentage| percentage.parse::<u8>().ok()),
            });
        });
        match result {
            Ok(()) => report.sources_read += 1,
            Err(error) => report.warn("corrupt_index", format!("{file_name}: {error}")),
        }
    }
}

fn architectures(env: &HostEnv) -> Option<BTreeSet<String>> {
    let text = fs::read_to_string(env.resolve(ARCHITECTURES_FILE)).ok()?;
    let mut set: BTreeSet<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if set.is_empty() {
        return None;
    }
    set.insert(ARCHITECTURE_ALL.to_owned());
    Some(set)
}

/// Repository of a list file, from `host_path_dists_suite_component_binary-arch_Packages[.ext]`.
fn list_source(file_name: &str) -> Option<String> {
    let (stem, rest) = file_name.split_once(PACKAGES_MARKER)?;
    if !EXTENSIONS.contains(&rest) || stem.contains("_i18n_") {
        return None;
    }
    let location = stem.rsplit_once("_binary-").map_or(stem, |(head, _)| head);
    Some(match location.split_once("_dists_") {
        Some((mirror, distribution)) => {
            format!(
                "{} {}",
                mirror.replace('_', "/"),
                distribution.replace('_', "/")
            )
        }
        None => location.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_source_names_mirror_suite_and_component() {
        assert_eq!(
            list_source(
                "archive.ubuntu.com_ubuntu_dists_noble-updates_main_binary-amd64_Packages.lz4"
            )
            .as_deref(),
            Some("archive.ubuntu.com/ubuntu noble-updates/main")
        );
        assert_eq!(list_source("x_dists_s_main_i18n_Translation-en"), None);
        assert_eq!(list_source("x_Packages.diff_Index"), None);
        assert_eq!(list_source("x_Packages.gz"), Some("x".to_owned()));
        assert_eq!(list_source("lock"), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
