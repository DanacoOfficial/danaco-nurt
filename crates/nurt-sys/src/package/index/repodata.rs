// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! RPM repository metadata of dnf and zypper (`repodata/repomd.xml`).

use std::io::BufReader;
use std::path::{Path, PathBuf};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::{EntrySink, IndexEntry, IndexReport};
use crate::package::compress::open_decoded;
use crate::package::root::HostEnv;

const DNF_CACHES: [&str; 2] = ["/var/cache/libdnf5", "/var/cache/dnf"];
const ZYPPER_CACHE: &str = "/var/cache/zypp/raw";
const REPOMD: &str = "repomd.xml";
const REPODATA: &str = "repodata";
const PRIMARY_TYPE: &str = "primary";
const SOURCE_ARCHITECTURE: &str = "src";
const MIN_HASH_LENGTH: usize = 8;

pub(super) fn read_dnf(env: &HostEnv, sink: EntrySink<'_>, report: &mut IndexReport) {
    let repositories = DNF_CACHES
        .iter()
        .flat_map(|cache| env.root().list_dir(cache))
        .collect();
    read_repositories(repositories, strip_hash, sink, report);
}

pub(super) fn read_zypper(env: &HostEnv, sink: EntrySink<'_>, report: &mut IndexReport) {
    let repositories = env.root().list_dir(ZYPPER_CACHE);
    read_repositories(repositories, str::to_owned, sink, report);
}

fn read_repositories(
    repositories: Vec<PathBuf>,
    source_name: fn(&str) -> String,
    sink: EntrySink<'_>,
    report: &mut IndexReport,
) {
    for repository in repositories {
        let repomd = repository.join(REPODATA).join(REPOMD);
        if !repomd.is_file() {
            continue;
        }
        let name = repository
            .file_name()
            .and_then(|name| name.to_str())
            .map(source_name)
            .unwrap_or_default();
        report.sources_found += 1;
        match read_repository(&repository, &repomd, &name, sink) {
            Ok(()) => report.sources_read += 1,
            Err(detail) => report.warn("corrupt_index", format!("{name}: {detail}")),
        }
    }
}

/// Removes the `-<hash>` suffix that dnf appends to the identifier of a repository.
fn strip_hash(directory: &str) -> String {
    match directory.rsplit_once('-') {
        Some((name, hash))
            if hash.len() >= MIN_HASH_LENGTH && hash.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            name.to_owned()
        }
        _ => directory.to_owned(),
    }
}

fn read_repository(
    repository: &Path,
    repomd: &Path,
    source: &str,
    sink: EntrySink<'_>,
) -> Result<(), String> {
    let primary = primary_location(repomd)?;
    let mut path = repository.join(&primary);
    if !path.is_file() {
        let file = Path::new(&primary).file_name().ok_or("empty primary location")?;
        path = repository.join(REPODATA).join(file);
    }
    let reader = open_decoded(&path).map_err(|error| error.to_string())?;
    read_primary(BufReader::new(reader), source, sink)
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.as_ref() == name)
        .map(|attribute| attribute.value.to_string())
}

fn primary_location(repomd: &Path) -> Result<String, String> {
    let reader = open_decoded(repomd).map_err(|error| error.to_string())?;
    let mut xml = Reader::from_reader(BufReader::new(reader));
    let mut buffer = Vec::new();
    let mut in_primary = false;
    loop {
        match xml.read_event_into(&mut buffer).map_err(|error| error.to_string())? {
            Event::Start(element) if element.name().as_ref() == "data" => {
                in_primary = attribute(&element, "type").as_deref() == Some(PRIMARY_TYPE);
            }
            Event::Start(element) | Event::Empty(element)
                if in_primary && element.name().as_ref() == "location" =>
            {
                if let Some(href) = attribute(&element, "href") {
                    return Ok(href);
                }
            }
            Event::End(element) if element.name().as_ref() == "data" => in_primary = false,
            Event::Eof => return Err("repomd.xml has no primary metadata".to_owned()),
            _ => {}
        }
        buffer.clear();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Text {
    Name,
    Architecture,
    Summary,
}

#[derive(Default)]
struct Package {
    name: String,
    architecture: String,
    version: String,
    summary: String,
    size: Option<u64>,
}

fn read_primary<R: std::io::BufRead>(
    source_reader: R,
    source: &str,
    sink: EntrySink<'_>,
) -> Result<(), String> {
    let mut xml = Reader::from_reader(source_reader);
    let mut buffer = Vec::new();
    let mut package: Option<Package> = None;
    let mut text: Option<Text> = None;
    let mut packages = 0_u64;
    loop {
        let event = xml.read_event_into(&mut buffer).map_err(|error| error.to_string())?;
        match event {
            Event::Start(element) => {
                text = open_element(&element, &mut package).or(text);
            }
            Event::Empty(element) => {
                open_element(&element, &mut package);
            }
            Event::Text(content) => {
                if let (Some(package), Some(target)) = (package.as_mut(), text) {
                    let content = content.xml10_content();
                    field(package, target).push_str(&content);
                }
            }
            Event::GeneralRef(reference) => {
                if let (Some(package), Some(target)) = (package.as_mut(), text) {
                    let name: &str = reference.as_ref();
                    if let Some(character) = predefined_entity(name) {
                        field(package, target).push(character);
                    }
                }
            }
            Event::End(element) => match element.name().as_ref() {
                "name" | "arch" | "summary" => text = None,
                "package" => {
                    if let Some(finished) = package.take() {
                        packages += 1;
                        emit(finished, source, sink);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if packages == 0 {
        return Err("primary metadata has no packages".to_owned());
    }
    Ok(())
}

/// Applies the attributes of an element and returns the text field it opens.
fn open_element(element: &BytesStart<'_>, package: &mut Option<Package>) -> Option<Text> {
    match element.name().as_ref() {
        "package" => *package = Some(Package::default()),
        "name" if package.is_some() => return Some(Text::Name),
        "arch" if package.is_some() => return Some(Text::Architecture),
        "summary" if package.is_some() => return Some(Text::Summary),
        "version" => {
            if let Some(package) = package.as_mut() {
                package.version = evr(element);
            }
        }
        "size" => {
            if let Some(package) = package.as_mut() {
                package.size = size(element);
            }
        }
        _ => {}
    }
    None
}

fn field(package: &mut Package, target: Text) -> &mut String {
    match target {
        Text::Name => &mut package.name,
        Text::Architecture => &mut package.architecture,
        Text::Summary => &mut package.summary,
    }
}

fn predefined_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

fn evr(element: &BytesStart<'_>) -> String {
    let epoch = attribute(element, "epoch").unwrap_or_default();
    let version = attribute(element, "ver").unwrap_or_default();
    let release = attribute(element, "rel");
    let mut text = String::new();
    if !epoch.is_empty() && epoch != "0" {
        text.push_str(&epoch);
        text.push(':');
    }
    text.push_str(&version);
    if let Some(release) = release.filter(|release| !release.is_empty()) {
        text.push('-');
        text.push_str(&release);
    }
    text
}

fn size(element: &BytesStart<'_>) -> Option<u64> {
    let number = |name| attribute(element, name).and_then(|value| value.parse::<u64>().ok());
    number("installed")
        .filter(|installed| *installed > 0)
        .or_else(|| number("package"))
}

fn emit(package: Package, source: &str, sink: EntrySink<'_>) {
    if package.name.is_empty() || package.architecture == SOURCE_ARCHITECTURE {
        return;
    }
    sink(IndexEntry {
        name: package.name,
        version: package.version,
        architecture: Some(package.architecture).filter(|arch| !arch.is_empty()),
        description: Some(package.summary).filter(|summary| !summary.is_empty()),
        size: package.size,
        source: source.to_owned(),
        phased: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_hash_removes_only_hexadecimal_suffix() {
        assert_eq!(strip_hash("fedora-0123456789abcdef"), "fedora");
        assert_eq!(strip_hash("updates-testing"), "updates-testing");
        assert_eq!(strip_hash("plain"), "plain");
    }
}
