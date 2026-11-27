// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-sys/tests/d7_static.rs
// Responsibility:   Static checks of the system, administration, network, schedule and program.
// Dependencies:     regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Static checks of the system, administration, network, schedule and program
//! crates.
//!
//! The checks read the sources of the workspace and run none of the code under
//! test. They cover the places where the crates start programs, the
//! portability of their files, the producer header and the absence of network
//! policy keys in the configuration. Patterns that the checks reject are
//! assembled from fragments or written with escapes, so that this file passes
//! the checks it implements.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Crates whose files are subject to the checks.
const CRATES: [&str; 6] = [
    "nurt-sys",
    "nurt-syslog",
    "nurt-net",
    "nurt-sched",
    "nurt-service",
    "nurt-programs",
];

/// Catalog groups implemented by [`CRATES`].
const GROUPS: [&str; 6] = [
    "system",
    "administration",
    "package",
    "network",
    "schedule",
    "program",
];

/// Commands of the `system` catalog group implemented by other crates.
const FOREIGN_COMMANDS: [&str; 2] = ["suspend", "resume"];

/// Number of lines at the start of a file that hold the producer header.
const HEADER_LINES: usize = 15;

/// Package managers run for `package.install`, `package.remove` and
/// `package.upgrade`.
const PACKAGE_MANAGERS: [&str; 13] = [
    "apt-get", "apt", "dnf", "yum", "zypper", "pacman", "apk", "brew", "winget", "choco", "scoop",
    "flatpak", "snap",
];

/// Running package managers for changing commands is approved; reading
/// package databases stays native.
const PACKAGE_MANAGER_EXECUTION_APPROVED: bool = true;

/// Running programs with `--version` is not approved: versions come from
/// package metadata and executable file headers.
const VERSION_PROBE_APPROVED: bool = false;

/// Crates that start processes and must not be dependencies of [`CRATES`].
const PROCESS_CRATES: [&str; 7] = [
    "nurt-proc",
    "duct",
    "subprocess",
    "xshell",
    "cmd_lib",
    "process-wrap",
    "command-group",
];

/// Programs a site may start.
#[derive(Debug, Clone, Copy)]
enum Allowed {
    /// Exactly the listed programs, named by literals or constants.
    Only(&'static [&'static str]),
    /// Package managers from [`PACKAGE_MANAGERS`], also chosen at run time.
    PackageManagers,
    /// The program named by the caller; literal program names are rejected.
    DescribedProgram,
    /// The programs named by the literals of `Call::new` and `Call::owned`
    /// in the administration module, all in the listed set; the site itself
    /// runs the program of the call.
    NamedByCall(&'static [&'static str]),
}

/// Source file permitted to start programs through the process host.
#[derive(Debug, Clone, Copy)]
struct Site {
    /// Path relative to the workspace root; a directory of the same name
    /// without `.rs` is included.
    path: &'static str,
    /// Programs the site may start.
    allowed: Allowed,
    /// Catalog commands served by the site; empty for code outside stages.
    commands: &'static [&'static str],
    /// The site is approved; otherwise any execution in it is rejected.
    approved: bool,
}

/// Commands that manage services.
const SERVICE_COMMANDS: &[&str] = &["services", "start", "restart", "enable", "disable"];

/// Programs of the firewall, network and clock commands (`admin/tool.rs`).
const NETWORK_PROGRAMS: &[&str] = &[
    "nft",
    "firewall-cmd",
    "ufw",
    "pfctl",
    "ip",
    "route",
    "netstat",
    "resolvectl",
    "networksetup",
    "timedatectl",
    "systemsetup",
    "tzutil",
    "date",
    "powershell",
];

/// Commands that change the firewall, the routes, the DNS and the clock.
const NETWORK_COMMANDS: &[&str] = &[
    "firewall.rules",
    "firewall.allow",
    "firewall.deny",
    "firewall.remove",
    "network.route_add",
    "network.route_remove",
    "network.dns_set",
    "clock.timezone_set",
    "clock.set",
    "clock.ntp",
];

/// Commands that change accounts, groups, mounts, kernel parameters, power
/// and disks through system programs.
const ADMIN_COMMANDS: &[&str] = &[
    "accounts.create",
    "accounts.modify",
    "accounts.remove",
    "accounts.password",
    "groups.create",
    "groups.remove",
    "groups.add_member",
    "groups.remove_member",
    "mount",
    "unmount",
    "sysctl.set",
    "modules.load",
    "modules.unload",
    "power.restart",
    "power.shutdown",
    "power.cancel",
    "disks.partition",
    "disks.format",
];

/// Programs that the administration commands start; they build the program
/// from the plan of the command, so the site of the execution takes the
/// program named by the caller.
const ADMIN_PROGRAMS: &[&str] = &[
    "useradd",
    "sysadminctl",
    "net",
    "usermod",
    "dscl",
    "userdel",
    "chpasswd",
    "powershell",
    "groupadd",
    "dseditgroup",
    "groupdel",
    "gpasswd",
    "mount",
    "mountvol",
    "umount",
    "sysctl",
    "modprobe",
    "lsmod",
    "shutdown",
    "pkill",
    "parted",
    "lsblk",
    "diskutil",
    "mkfs",
];

/// Sites permitted to start programs.
const SITES: [Site; 7] = [
    Site {
        path: "crates/nurt-sys/src/services/macos.rs",
        allowed: Allowed::Only(&["launchctl"]),
        commands: SERVICE_COMMANDS,
        approved: true,
    },
    Site {
        path: "crates/nurt-service/src/install/macos.rs",
        allowed: Allowed::Only(&["launchctl"]),
        commands: &[],
        approved: true,
    },
    Site {
        path: "crates/nurt-sys/src/admin/sys.rs",
        allowed: Allowed::DescribedProgram,
        commands: &[],
        approved: true,
    },
    Site {
        path: "crates/nurt-sys/src/admin/mod.rs",
        allowed: Allowed::Only(ADMIN_PROGRAMS),
        commands: ADMIN_COMMANDS,
        approved: true,
    },
    Site {
        path: "crates/nurt-sys/src/package/exec.rs",
        allowed: Allowed::PackageManagers,
        commands: &["package.install", "package.remove", "package.upgrade"],
        approved: PACKAGE_MANAGER_EXECUTION_APPROVED,
    },
    Site {
        path: "crates/nurt-programs/src/help.rs",
        allowed: Allowed::DescribedProgram,
        commands: &["program.describe"],
        approved: true,
    },
    Site {
        path: "crates/nurt-sys/src/admin/tool.rs",
        allowed: Allowed::NamedByCall(NETWORK_PROGRAMS),
        commands: NETWORK_COMMANDS,
        approved: true,
    },
];

/// File outside the approved sites that runs program specifications built by
/// an approved site.
#[derive(Debug, Clone, Copy)]
struct Delegation {
    /// Path of the file relative to the workspace root.
    path: &'static str,
    /// Path of the approved site that builds the specifications.
    site: &'static str,
    /// Pattern of the call of the site that builds a specification.
    builder: &'static str,
}

/// The package stages run the command lines of the package managers in the
/// stage module, between the checks of the transaction; every specification
/// comes from the builder of the package manager site.
const DELEGATIONS: [Delegation; 1] = [Delegation {
    path: "crates/nurt-sys/src/package/mod.rs",
    site: "crates/nurt-sys/src/package/exec.rs",
    builder: r"exec\s*::\s*program_spec\s*\(",
}];

fn workspace_root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .ancestors()
    .nth(2)
    .expect("the crate lies two levels below the workspace root")
    .to_path_buf()
}

fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Regular files under `dir`, sorted; build output and hidden directories are
/// skipped.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("directory entry is readable").path();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let name = name.unwrap_or_default();
            if path.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    pending.push(path);
                }
            } else if path.is_file() {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Text of a file; `None` for files that are not UTF-8 text.
fn read_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.contains(&e))
}

fn line_of(code: &str, offset: usize) -> usize {
    code[..offset].bytes().filter(|&b| b == b'\n').count() + 1
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Rust source with comments removed and literals separated from the code.
#[derive(Debug, Default)]
struct Scanned {
    /// Code with comments and the contents of literals replaced by spaces.
    /// Line breaks are kept and other non-ASCII characters become `_`, so
    /// byte offsets equal character offsets and map to source lines.
    code: String,
    /// String literals by the offset of their first character in `code`.
    literals: BTreeMap<usize, String>,
    /// Names of modules declared as `mod name;` under `#[cfg(test)]`.
    test_modules: Vec<String>,
}

struct Scanner {
    chars: Vec<char>,
    pos: usize,
    code: String,
    literals: BTreeMap<usize, String>,
}

impl Scanner {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    fn emit(&mut self, c: char) {
        self.code.push(if c.is_ascii() { c } else { '_' });
    }

    fn blank(&mut self, c: char) {
        self.code.push(if c == '\n' { '\n' } else { ' ' });
    }

    fn run(mut self) -> (String, BTreeMap<usize, String>) {
        while let Some(c) = self.peek(0) {
            match (c, self.peek(1)) {
                ('/', Some('/')) => self.line_comment(),
                ('/', Some('*')) => self.block_comment(),
                ('"', _) => self.string(0),
                ('r', _) if self.raw_string_hashes().is_some() => {
                    let hashes = self.raw_string_hashes().unwrap_or_default();
                    self.string(hashes + 1);
                }
                ('\'', _) => self.quote(),
                _ => {
                    self.emit(c);
                    self.pos += 1;
                }
            }
        }
        (self.code, self.literals)
    }

    fn line_comment(&mut self) {
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                return;
            }
            self.blank(c);
            self.pos += 1;
        }
    }

    fn block_comment(&mut self) {
        let mut depth = 0usize;
        while let Some(c) = self.peek(0) {
            let next = self.peek(1);
            if c == '/' && next == Some('*') {
                depth += 1;
            } else if c == '*' && next == Some('/') {
                depth -= 1;
            } else {
                self.blank(c);
                self.pos += 1;
                continue;
            }
            self.code.push_str("  ");
            self.pos += 2;
            if depth == 0 {
                return;
            }
        }
    }

    /// Number of `#` after an `r` that starts a raw string literal.
    fn raw_string_hashes(&self) -> Option<usize> {
        let before = self
            .pos
            .checked_sub(1)
            .and_then(|i| self.chars.get(i))
            .copied();
        let prefix_ok = match before {
            None => true,
            Some('b' | 'c') => {
                let earlier = self
                    .pos
                    .checked_sub(2)
                    .and_then(|i| self.chars.get(i))
                    .copied();
                !earlier.is_some_and(is_ident_char)
            }
            Some(c) => !is_ident_char(c),
        };
        if !prefix_ok {
            return None;
        }
        let hashes = self.chars[self.pos + 1..]
            .iter()
            .take_while(|&&c| c == '#')
            .count();
        (self.peek(1 + hashes) == Some('"')).then_some(hashes)
    }

    /// Reads a string literal. `raw` is zero for an ordinary literal and the
    /// number of `#` plus one for a raw literal that starts with `r`.
    fn string(&mut self, raw: usize) {
        let start = self.code.len();
        let opening = if raw == 0 { 1 } else { raw + 1 };
        for _ in 0..opening {
            let c = self.peek(0).unwrap_or('"');
            self.emit(c);
            self.pos += 1;
        }
        let hashes = raw.saturating_sub(1);
        let mut value = String::new();
        while let Some(c) = self.peek(0) {
            if raw == 0 && c == '\\' {
                value.push(c);
                self.blank(c);
                self.pos += 1;
                if let Some(escaped) = self.peek(0) {
                    value.push(escaped);
                    self.blank(escaped);
                    self.pos += 1;
                }
                continue;
            }
            if c == '"' && self.closes_raw(hashes) {
                for _ in 0..=hashes {
                    let closing = self.peek(0).unwrap_or('"');
                    self.emit(closing);
                    self.pos += 1;
                }
                break;
            }
            value.push(c);
            self.blank(c);
            self.pos += 1;
        }
        self.literals.insert(start, value);
    }

    fn closes_raw(&self, hashes: usize) -> bool {
        (1..=hashes).all(|i| self.peek(i) == Some('#'))
    }

    /// Reads a character literal, or a lifetime that is kept as code.
    fn quote(&mut self) {
        let end = match (self.peek(1), self.peek(2)) {
            (Some('\\'), _) => self.chars[self.pos + 3..]
                .iter()
                .position(|&c| c == '\'')
                .map(|i| self.pos + 3 + i),
            (Some(_), Some('\'')) => Some(self.pos + 2),
            _ => None,
        };
        let Some(end) = end else {
            self.emit('\'');
            self.pos += 1;
            return;
        };
        self.emit('\'');
        for i in self.pos + 1..end {
            let c = self.chars[i];
            self.blank(c);
        }
        self.emit('\'');
        self.pos = end + 1;
    }
}

fn scan(source: &str) -> Scanned {
    let scanner = Scanner {
        chars: source.chars().collect(),
        pos: 0,
        code: String::with_capacity(source.len()),
        literals: BTreeMap::new(),
    };
    let (code, literals) = scanner.run();
    let mut scanned = Scanned {
        code,
        literals,
        test_modules: Vec::new(),
    };
    drop_test_items(&mut scanned);
    scanned
}

/// End offset (exclusive) of the item that starts at `start`: the first `;`
/// outside brackets, or the brace that closes the first block.
fn item_end(code: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut braces = 0usize;
    for (i, &b) in code.iter().enumerate().skip(start) {
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'{' => braces += 1,
            b'}' => {
                braces = braces.saturating_sub(1);
                if braces == 0 && depth == 0 {
                    return i + 1;
                }
            }
            b';' if depth == 0 && braces == 0 => return i + 1,
            _ => {}
        }
    }
    code.len()
}

/// Offset after the attributes that follow `start`.
fn skip_attributes(code: &[u8], mut start: usize) -> usize {
    loop {
        while code.get(start).is_some_and(u8::is_ascii_whitespace) {
            start += 1;
        }
        if code.get(start) != Some(&b'#') {
            return start;
        }
        let Some(open) = code[start..].iter().position(|&b| b == b'[') else {
            return start;
        };
        start = item_end_of_brackets(code, start + open);
    }
}

fn item_end_of_brackets(code: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, &b) in code.iter().enumerate().skip(open) {
        match b {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// Blanks the items compiled only for tests and records their module files.
fn drop_test_items(scanned: &mut Scanned) {
    let attribute = Regex::new(r"#\s*\[\s*cfg\s*\(\s*(?:test|all\s*\(\s*test\b[^\]]*\))\s*\)\s*\]")
        .expect("attribute pattern is valid");
    let module = Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
        .expect("module pattern is valid");
    let mut ranges = Vec::new();
    for found in attribute.find_iter(&scanned.code) {
        let bytes = scanned.code.as_bytes();
        let item = skip_attributes(bytes, found.end());
        let end = item_end(bytes, item);
        if let Some(name) = module.captures(&scanned.code[item..end]) {
            scanned.test_modules.push(name[1].to_owned());
        }
        ranges.push(found.start()..end);
    }
    let mut code = scanned.code.clone().into_bytes();
    for range in ranges {
        for b in &mut code[range.clone()] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        scanned.literals.retain(|offset, _| !range.contains(offset));
    }
    scanned.code = String::from_utf8(code).expect("code is ASCII");
}

/// Directory that holds the files of the child modules of `file`.
fn child_module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new(""));
    match file.file_name().and_then(|n| n.to_str()) {
        Some("lib.rs" | "main.rs" | "mod.rs") => parent.to_path_buf(),
        _ => parent.join(file.file_stem().unwrap_or_default()),
    }
}

/// Production source file of a crate, scanned.
struct ProductionFile {
    rel: String,
    scanned: Scanned,
}

/// Production sources of [`CRATES`]: `src/` and build scripts, without the
/// files of modules compiled only for tests.
fn production_files(root: &Path) -> Vec<ProductionFile> {
    let mut scanned: Vec<(PathBuf, Scanned)> = Vec::new();
    for krate in CRATES {
        let dir = root.join("crates").join(krate);
        let mut paths = files_under(&dir.join("src"));
        paths.push(dir.join("build.rs"));
        for path in paths {
            if !has_extension(&path, &["rs"]) {
                continue;
            }
            if let Some(text) = read_text(&path) {
                scanned.push((path, scan(&text)));
            }
        }
    }
    let mut excluded = BTreeSet::new();
    for (path, file) in &scanned {
        let dir = child_module_dir(path);
        for name in &file.test_modules {
            excluded.insert(dir.join(format!("{name}.rs")));
            excluded.insert(dir.join(name));
        }
    }
    scanned
        .into_iter()
        .filter(|(path, _)| !excluded.iter().any(|e| path.starts_with(e)))
        .map(|(path, scanned)| ProductionFile {
            rel: relative(root, &path),
            scanned,
        })
        .collect()
}

/// Program named at an execution site.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProgramName {
    /// A string literal, directly or through a constant of the same file.
    Literal(String),
    /// An expression evaluated at run time.
    Dynamic(String),
}

/// Place in production code that starts a program through the process host.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecSite {
    line: usize,
    construct: String,
    program: Option<ProgramName>,
}

fn skip_spaces(code: &str, mut offset: usize) -> usize {
    let bytes = code.as_bytes();
    while bytes.get(offset).is_some_and(u8::is_ascii_whitespace) {
        offset += 1;
    }
    offset
}

/// Program named by the expression that starts at `offset`.
fn program_at(scanned: &Scanned, offset: usize) -> ProgramName {
    let offset = skip_spaces(&scanned.code, offset);
    let rest = &scanned.code[offset..];
    if let Some(value) = scanned.literals.get(&offset) {
        return ProgramName::Literal(value.clone());
    }
    let path: String = rest
        .chars()
        .take_while(|&c| is_ident_char(c) || c == ':')
        .collect();
    let after = skip_spaces(&scanned.code, offset + path.len());
    if scanned.code.as_bytes().get(after) == Some(&b'(') && path.ends_with("from") {
        return program_at(scanned, after + 1);
    }
    let name = path.rsplit("::").next().unwrap_or_default();
    constant_value(scanned, name).map_or_else(
        || {
            ProgramName::Dynamic(
                rest.chars()
                    .take_while(|&c| c != ',' && c != '\n')
                    .collect(),
            )
        },
        ProgramName::Literal,
    )
}

/// Value of a string constant declared in the file.
fn constant_value(scanned: &Scanned, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let pattern = format!(
        r"\bconst\s+{}\s*:\s*&\s*(?:'static\s+)?str\s*=\s*",
        regex::escape(name)
    );
    let found = Regex::new(&pattern).ok()?.find(&scanned.code)?;
    scanned
        .literals
        .get(&skip_spaces(&scanned.code, found.end()))
        .cloned()
}

/// Execution sites of one file.
fn exec_sites(scanned: &Scanned) -> Vec<ExecSite> {
    let construct = Regex::new(
        r"\b(?:ProgramSpec\s*::\s*new\s*\(|(?:ProgramSpec|ServiceSpawnSpec)\s*\{|run_program\b|spawn_service\b)",
    )
    .expect("construct pattern is valid");
    let field = Regex::new(r"\bprogram\s*:").expect("field pattern is valid");
    let mut sites = Vec::new();
    for found in construct.find_iter(&scanned.code) {
        let text = found.as_str();
        let program = if text.ends_with('(') {
            Some(program_at(scanned, found.end()))
        } else if text.ends_with('{') {
            let end = item_end(scanned.code.as_bytes(), found.end() - 1);
            field
                .find(&scanned.code[found.end()..end])
                .map(|f| program_at(scanned, found.end() + f.end()))
        } else {
            None
        };
        sites.push(ExecSite {
            line: line_of(&scanned.code, found.start()),
            construct: text.split_whitespace().collect(),
            program,
        });
    }
    sites
}

/// File name of a program given by path; programs are compared by it.
fn program_file_name(program: &str) -> &str {
    program.rsplit(['/', '\\']).next().unwrap_or(program)
}

/// The `run_program` calls of a delegating file run only specifications
/// bound from the builder of an approved site.
fn runs_delegated_specs(file: &ProductionFile) -> bool {
    let Some(delegation) = DELEGATIONS.iter().find(|d| d.path == file.rel) else {
        return false;
    };
    if !site_for(delegation.site).is_some_and(|site| site.approved) {
        return false;
    }
    let call = Regex::new(r"\brun_program\s*\(\s*&?\s*([A-Za-z_][A-Za-z0-9_]*)\s*,")
        .expect("call pattern is valid");
    let arguments: Vec<String> = call
        .captures_iter(&file.scanned.code)
        .map(|c| c[1].to_owned())
        .collect();
    !arguments.is_empty()
        && arguments.iter().all(|argument| {
            let binding = format!(
                r"\blet\s+(?:mut\s+)?{}\s*=\s*{}",
                regex::escape(argument),
                delegation.builder
            );
            Regex::new(&binding).is_ok_and(|b| b.is_match(&file.scanned.code))
        })
}

/// Offset after the method calls without arguments that follow `offset`, as
/// in `"x".to_owned()`.
fn skip_method_calls(code: &str, mut offset: usize) -> usize {
    let call = Regex::new(r"^\s*\.\s*[A-Za-z_][A-Za-z0-9_]*\s*(?:::\s*<[^>]*>\s*)?\(\s*\)")
        .expect("call pattern is valid");
    while let Some(found) = call.find(&code[offset..]) {
        offset += found.end();
    }
    skip_spaces(code, offset)
}

/// The literal `--version` at `offset` ends its argument list and asks the
/// program for its own version. Followed by another argument, the option
/// selects the version of a package.
fn is_version_probe(scanned: &Scanned, offset: usize, value: &str) -> bool {
    if value != "--version" {
        return false;
    }
    let code = scanned.code.as_bytes();
    let quote = |from: usize| {
        code.get(from..)
            .and_then(|rest| rest.iter().position(|&b| b == b'"'))
            .map(|i| from + i)
    };
    let Some(close) = quote(offset).and_then(|open| quote(open + 1)) else {
        return true;
    };
    let mut next = close + 1;
    while code.get(next) == Some(&b'#') {
        next += 1;
    }
    let next = skip_method_calls(&scanned.code, next);
    if code.get(next) != Some(&b',') {
        return true;
    }
    let after = skip_spaces(&scanned.code, next + 1);
    matches!(code.get(after), Some(b']' | b')') | None)
}

fn site_for(rel: &str) -> Option<&'static Site> {
    SITES.iter().find(|site| {
        let dir = site.path.trim_end_matches(".rs");
        rel == site.path || rel.starts_with(&format!("{dir}/"))
    })
}

/// Direct process creation, outside the process host.
fn direct_process_patterns() -> Vec<(&'static str, Regex)> {
    [
        (
            "std::process types",
            r"\bprocess\s*::\s*(?:Command|Stdio|Child)\b",
        ),
        (
            "std::process types",
            r"\bprocess\s*::\s*\{[^}]*\b(?:Command|Stdio|Child)\b",
        ),
        ("command builder", r"\bCommand\s*::\s*new\b|\bCommandExt\b"),
        (
            "operating system process creation",
            r"\b(?:CreateProcess\w*|ShellExecute\w*|WinExec|posix_spawnp?)\b",
        ),
        (
            "operating system process creation",
            r"\b(?:libc|nix\s*::\s*unistd)\s*::\s*(?:system|popen|fork|vfork|exec\w*)\b",
        ),
    ]
    .into_iter()
    .map(|(label, pattern)| (label, Regex::new(pattern).expect("pattern is valid")))
    .collect()
}

fn manifest_dependencies(manifest: &toml::Table) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let sections = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut tables: Vec<&toml::Table> = vec![manifest];
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        tables.extend(targets.values().filter_map(toml::Value::as_table));
    }
    for table in tables {
        for section in sections {
            if let Some(deps) = table.get(section).and_then(toml::Value::as_table) {
                names.extend(deps.keys().cloned());
            }
        }
    }
    names
}

#[test]
fn static_no_std_process_command_in_d7_crates() {
    let root = workspace_root();
    let patterns = direct_process_patterns();
    let mut findings = Vec::new();
    for file in production_files(&root) {
        for (label, pattern) in &patterns {
            for found in pattern.find_iter(&file.scanned.code) {
                let line = line_of(&file.scanned.code, found.start());
                findings.push(format!("{}:{line}: {label}: {}", file.rel, found.as_str()));
            }
        }
    }
    for krate in CRATES {
        let path = root.join("crates").join(krate).join("Cargo.toml");
        let manifest: toml::Table = read_text(&path)
            .expect("crate manifest is readable")
            .parse()
            .expect("crate manifest is valid TOML");
        for name in manifest_dependencies(&manifest) {
            if PROCESS_CRATES.contains(&name.as_str()) {
                findings.push(format!(
                    "{}: process crate dependency: {name}",
                    relative(&root, &path)
                ));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "programs are started only through the process host:\n{}",
        findings.join("\n")
    );
}

/// Findings for the program named at one execution site of an approved file.
fn program_findings(site: &Site, rel: &str, exec: &ExecSite) -> Option<String> {
    let program = exec.program.as_ref()?;
    let location = format!("{rel}:{}", exec.line);
    match (site.allowed, program) {
        (Allowed::Only(names), ProgramName::Literal(name))
            if !names.contains(&program_file_name(name)) =>
        {
            Some(format!(
                "{location}: program `{name}` is not permitted here"
            ))
        }
        (Allowed::Only(_), ProgramName::Dynamic(expr)) => Some(format!(
            "{location}: program must be a literal or a constant, found `{expr}`"
        )),
        (Allowed::PackageManagers, ProgramName::Literal(name))
            if !PACKAGE_MANAGERS.contains(&name.as_str()) =>
        {
            Some(format!("{location}: `{name}` is not a package manager"))
        }
        (Allowed::DescribedProgram, ProgramName::Literal(name)) => Some(format!(
            "{location}: only the described program is run, found `{name}`"
        )),
        (Allowed::NamedByCall(names), ProgramName::Literal(name))
            if !names.contains(&program_file_name(name)) =>
        {
            Some(format!(
                "{location}: program `{name}` is not permitted here"
            ))
        }
        (Allowed::NamedByCall(_), ProgramName::Dynamic(expr))
            if !expr.trim_start().starts_with("call.program") =>
        {
            Some(format!(
                "{location}: the program must be the program of the call, found `{expr}`"
            ))
        }
        _ => None,
    }
}

/// Findings for the option literals of a file with execution sites.
fn option_findings(rel: &str, scanned: &Scanned, site: Option<&Site>) -> Vec<String> {
    let help_permitted = site.is_some_and(|s| matches!(s.allowed, Allowed::DescribedProgram));
    let mut findings = Vec::new();
    for (offset, value) in &scanned.literals {
        let line = line_of(&scanned.code, *offset);
        if !VERSION_PROBE_APPROVED && is_version_probe(scanned, *offset, value) {
            findings.push(format!("{rel}:{line}: `--version` probe awaits approval"));
        }
        if value.starts_with("--help") && !help_permitted {
            findings.push(format!(
                "{rel}:{line}: `--help` is run only by program descriptions"
            ));
        }
    }
    findings
}

fn file_exec_findings(file: &ProductionFile, sites: &[ExecSite]) -> Vec<String> {
    let site = site_for(&file.rel);
    let mut findings = option_findings(&file.rel, &file.scanned, site);
    let delegated = runs_delegated_specs(file);
    match site {
        None => findings.extend(
            sites
                .iter()
                .filter(|s| !(delegated && s.construct == "run_program"))
                .map(|s| {
                    format!(
                        "{}:{}: `{}` outside an approved site",
                        file.rel, s.line, s.construct
                    )
                }),
        ),
        Some(site) if !site.approved => findings.extend(sites.iter().map(|s| {
            format!(
                "{}:{}: `{}` awaits approval of the site",
                file.rel, s.line, s.construct
            )
        })),
        Some(site) => findings.extend(
            sites
                .iter()
                .filter_map(|exec| program_findings(site, &file.rel, exec)),
        ),
    }
    findings
}

#[test]
fn static_only_launchctl_and_approved_external_programs() {
    let root = workspace_root();
    let mut findings = Vec::new();
    for file in production_files(&root) {
        let sites = exec_sites(&file.scanned);
        if !sites.is_empty() {
            findings.extend(file_exec_findings(&file, &sites));
        }
    }
    assert!(
        findings.is_empty(),
        "programs are started only at approved sites:\n{}",
        findings.join("\n")
    );
}

/// Commands of the catalog groups in [`GROUPS`], by name.
fn domain_commands(root: &Path) -> BTreeMap<String, toml::Table> {
    let mut commands = BTreeMap::new();
    for group in GROUPS {
        let path = root.join("catalog").join(format!("{group}.toml"));
        let catalog: toml::Table = read_text(&path)
            .expect("catalog file is readable")
            .parse()
            .expect("catalog file is valid TOML");
        let entries = catalog.get("command").and_then(toml::Value::as_array);
        for entry in entries.into_iter().flatten() {
            let table = entry.as_table().expect("command entry is a table").clone();
            let name = table
                .get("name")
                .and_then(toml::Value::as_str)
                .unwrap_or_default();
            if !FOREIGN_COMMANDS.contains(&name) {
                commands.insert(name.to_owned(), table);
            }
        }
    }
    commands
}

fn string_field<'a>(table: &'a toml::Table, key: &str) -> Option<&'a str> {
    table.get(key).and_then(toml::Value::as_str)
}

fn external_programs(command: &toml::Table) -> Vec<String> {
    command
        .get("external")
        .and_then(|e| e.get("programs"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// Findings when `command` does not declare the external use of `site`.
fn declaration_findings(
    site: &Site,
    name: &str,
    command: &toml::Table,
    used: &[String],
) -> Vec<String> {
    let external = command.get("external").and_then(toml::Value::as_table);
    let kind = external.and_then(|e| string_field(e, "kind"));
    let declared = external_programs(command);
    let mut findings = Vec::new();
    match site.allowed {
        Allowed::Only(_) | Allowed::NamedByCall(_) => {
            if kind != Some("system_program") {
                findings.push(format!(
                    "{name}: external kind `system_program` is not declared"
                ));
            }
            for program in used.iter().filter(|p| !declared.contains(p)) {
                findings.push(format!("{name}: program `{program}` is not declared"));
            }
        }
        Allowed::PackageManagers => {
            if string_field(command, "implementation") != Some("package_manager") {
                findings.push(format!(
                    "{name}: implementation `package_manager` is not declared"
                ));
            }
        }
        Allowed::DescribedProgram => {
            if kind != Some("user_program") {
                findings.push(format!(
                    "{name}: external kind `user_program` is not declared"
                ));
            }
        }
    }
    findings
}

/// Programs named by literals at the execution sites of the files of `site`.
fn used_programs(files: &[ProductionFile], site: &Site) -> Option<Vec<String>> {
    let mut used = BTreeSet::new();
    let mut any = false;
    for file in files
        .iter()
        .filter(|f| site_for(&f.rel).is_some_and(|s| s.path == site.path))
    {
        for exec in exec_sites(&file.scanned) {
            any = true;
            if let Some(ProgramName::Literal(name)) = exec.program {
                used.insert(program_file_name(&name).to_owned());
            }
        }
    }
    any.then(|| used.into_iter().collect())
}

#[test]
fn static_admin_calls_name_approved_programs() {
    let root = workspace_root();
    let call = Regex::new(r#"Call\s*::\s*(?:new|owned)\s*\(\s*""#).expect("pattern is valid");
    let admin = Regex::new(
        r"^crates/nurt-sys/src/admin/(clock\.rs|effects\.rs|tool\.rs|firewall/|network/)",
    )
    .expect("pattern is valid");
    let mut findings = Vec::new();
    let mut named = BTreeSet::new();
    for file in production_files(&root)
        .iter()
        .filter(|file| admin.is_match(&file.rel))
    {
        for found in call.find_iter(&file.scanned.code) {
            let Some(program) = file.scanned.literals.get(&(found.end() - 1)).cloned() else {
                continue;
            };
            if !NETWORK_PROGRAMS.contains(&program.as_str()) {
                let line = line_of(&file.scanned.code, found.start());
                findings.push(format!(
                    "{}:{line}: `{program}` is not an approved program",
                    file.rel
                ));
            }
            named.insert(program);
        }
    }
    for program in NETWORK_PROGRAMS {
        if !named.contains(*program) && *program != "powershell" {
            findings.push(format!("approved program `{program}` is not used"));
        }
    }
    assert!(
        findings.is_empty(),
        "the administration module starts only approved programs:\n{}",
        findings.join("\n")
    );
}

#[test]
fn static_external_calls_have_stage_spec_external() {
    let root = workspace_root();
    let commands = domain_commands(&root);
    let files = production_files(&root);
    let mut findings = Vec::new();
    for site in &SITES {
        let Some(used) = used_programs(&files, site) else {
            continue;
        };
        for name in site.commands {
            match commands.get(*name) {
                Some(command) => findings.extend(declaration_findings(site, name, command, &used)),
                None => findings.push(format!("{name}: command is missing from the catalog")),
            }
        }
    }
    let approved: BTreeSet<&str> = SITES
        .iter()
        .filter_map(|s| match s.allowed {
            Allowed::Only(names) | Allowed::NamedByCall(names) => Some(names),
            _ => None,
        })
        .flatten()
        .copied()
        .collect();
    for (name, command) in &commands {
        for program in external_programs(command) {
            if !approved.contains(program.as_str()) {
                findings.push(format!(
                    "{name}: declared program `{program}` is not approved"
                ));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "external programs agree with the catalog:\n{}",
        findings.join("\n")
    );
}

/// Files of the crates, the command and group pages and the system tests.
fn domain_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for krate in CRATES {
        files.extend(files_under(&root.join("crates").join(krate)));
    }
    let docs = root.join("docs");
    for group in GROUPS {
        files.push(docs.join("groups").join(format!("{group}.md")));
    }
    for name in domain_commands(root).keys() {
        files.push(docs.join("commands").join(format!("{name}.md")));
    }
    files.extend(files_under(&root.join("tests").join("d7")));
    files.retain(|p| p.is_file());
    files
}

fn producer() -> String {
    let name = ["Dan", "aco"].concat();
    format!("{name} Official | {name} Holding Group Sp. z o.o. All rights reserved.")
}

/// Line of the producer header or footer, permitted to name the producer.
///
/// The lines of the fixed header and of the fixed footer that name the
/// producer are permitted at any position and behind any comment mark (`//`,
/// `#` or a one-line `<!-- -->`); among the first lines of a file the
/// copyright, license and contact lines of other forms are permitted as well.
fn is_header_line(index: usize, line: &str) -> bool {
    let name = ["Dan", "aco"].concat();
    let domain = format!("{}-group.pl", name.to_lowercase());
    let contact = format!("support@{domain}");
    let license = format!("see the LICENSE file or https://www.{domain}");
    let content = line
        .trim()
        .trim_start_matches("<!--")
        .trim_end_matches("-->")
        .trim_start_matches("//")
        .trim_start_matches('#')
        .trim();
    let fixed = [
        format!("Use is subject to license terms: {license}"),
        format!("https://github.com/{name}Official | {contact}"),
        format!("Author:           {name} Official"),
        format!("License:          {license}"),
        format!("Support: {contact}"),
    ];
    (content.starts_with("Copyright (c) ") && content.ends_with(&producer()))
        || fixed.iter().any(|text| content == text)
        || (index < HEADER_LINES
            && (line.contains("Copyright (c) ")
                || line.contains("Use is subject to license terms")
                || line.contains(&contact)))
}

/// The line names the producer other than in the product name and in the
/// address of the product repository.
fn names_producer_outside_product_name(line: &str) -> bool {
    let name = ["dan", "aco"].concat();
    let lower = line.to_lowercase();
    lower
        .replace(&format!("{name} nurt"), "")
        .replace(&format!("github.com/{name}official/{name}-nurt"), "")
        .contains(&name)
}

/// Polish letters with diacritical marks.
const POLISH_LETTERS: [char; 18] = [
    '\u{0104}', '\u{0105}', '\u{0106}', '\u{0107}', '\u{0118}', '\u{0119}', '\u{0141}', '\u{0142}',
    '\u{0143}', '\u{0144}', '\u{00D3}', '\u{00F3}', '\u{015A}', '\u{015B}', '\u{0179}', '\u{017A}',
    '\u{017B}', '\u{017C}',
];

/// Pattern of the portability check with the name of the repository hygiene
/// check whose allow list entries apply to it.
struct PortabilityRule {
    check: &'static str,
    label: &'static str,
    pattern: Regex,
}

fn portability_rules() -> Vec<PortabilityRule> {
    [
        (
            "work-marker",
            "unfinished-work marker",
            r"\b(?:T[O]DO|F[I]XME|X[X]X|H[A]CK)\b",
        ),
        (
            "process-id",
            "work item identifier",
            r"\b(?:[PDQV]|PD7|DD|LP|TC|TF|TD|TY|SEM-[A-Z]+)-\d+\b|\bU(?:0[1-9]|1[0-6])\b|\bD[1-8]\b",
        ),
        (
            "process-word",
            "design numbering",
            r"\b[A-S]\d?\.\d+(?:\.\d+)*[a-z]?\b",
        ),
        (
            "process-word",
            "design reference",
            r"(?i)\b(?:design d[o]cument|propos[a]l-v\d|decis[i]ons\.md|open-it[e]ms)\b",
        ),
        (
            "external-pattern",
            "build environment",
            r"(?i)\b(?:kvm-g[r]a\d*|g[r]a2|build-e[n]v)\b",
        ),
    ]
    .into_iter()
    .map(|(check, label, pattern)| PortabilityRule {
        check,
        label,
        pattern: Regex::new(pattern).expect("pattern is valid"),
    })
    .collect()
}

/// Parts of the default Homebrew prefix on Linux, assembled so that this file
/// does not contain the prefix.
const HOMEBREW_LINUX_PREFIX_PARTS: [&str; 2] = ["/home/", "linuxbrew/.linuxbrew"];

/// The default Homebrew prefix on Linux is product data, not the home
/// directory of an account, and is exempt from the home directory check.
fn homebrew_linux_prefix() -> String {
    HOMEBREW_LINUX_PREFIX_PARTS.concat()
}

/// Home directories other than the placeholder account `user`.
fn foreign_home_directories(line: &str) -> Vec<String> {
    let pattern = Regex::new(r"(?:/home/|/Users/|[A-Za-z]:[\\/]+Users[\\/]+)([A-Za-z0-9._-]+)")
        .expect("home pattern is valid");
    let line = line.replace(&homebrew_linux_prefix(), "");
    pattern
        .captures_iter(&line)
        .filter(|c| c[1].trim_end_matches('.') != "user")
        .map(|c| c[0].to_owned())
        .collect()
}

/// Findings of one line as pairs of hygiene check and description.
fn line_findings(
    rules: &[PortabilityRule],
    index: usize,
    line: &str,
) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    if !is_header_line(index, line) && names_producer_outside_product_name(line) {
        found.push(("external-pattern", "producer name".to_owned()));
    }
    if line.chars().any(|c| POLISH_LETTERS.contains(&c)) {
        found.push(("polish-text", "Polish text".to_owned()));
    }
    for rule in rules {
        if let Some(m) = rule.pattern.find(line) {
            found.push((rule.check, format!("{}: {}", rule.label, m.as_str())));
        }
    }
    for home in foreign_home_directories(line) {
        found.push(("user-path", format!("home directory: {home}")));
    }
    found
}

/// Groups of the repository hygiene checks used by these tests.
fn hygiene_group(check: &str) -> &'static str {
    match check {
        "producer-header" | "spdx-line" => "headers",
        _ => "content",
    }
}

/// Shell pattern match in which `*` also matches `/`.
fn glob_matches(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.split_first(), text.split_first()) {
        (None, None) => true,
        (Some((b'*', rest)), _) => {
            glob_matches(rest, text) || (!text.is_empty() && glob_matches(pattern, &text[1..]))
        }
        (Some((b'?', rest)), Some((_, tail))) => glob_matches(rest, tail),
        (Some((p, rest)), Some((t, tail))) if p == t => glob_matches(rest, tail),
        _ => false,
    }
}

/// Entries `<check> <glob>` of the allow list of the repository hygiene check.
fn hygiene_allow_list(root: &Path) -> Vec<(String, String)> {
    let text = read_text(&root.join("tools/ci/hygiene-allow.txt")).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once(char::is_whitespace))
        .map(|(check, glob)| (check.to_owned(), glob.trim().to_owned()))
        .collect()
}

fn is_allowed(allow: &[(String, String)], check: &str, rel: &str) -> bool {
    allow.iter().any(|(entry, glob)| {
        (entry == check || entry == hygiene_group(check) || entry == "*")
            && glob_matches(glob.as_bytes(), rel.as_bytes())
    })
}

#[test]
fn portability_no_forbidden_strings_in_d7_files() {
    let root = workspace_root();
    let rules = portability_rules();
    let allow = hygiene_allow_list(&root);
    let mut findings = Vec::new();
    for path in domain_files(&root) {
        let Some(text) = read_text(&path) else {
            continue;
        };
        let rel = relative(&root, &path);
        for (index, line) in text.lines().enumerate() {
            for (check, finding) in line_findings(&rules, index, line) {
                if !is_allowed(&allow, check, &rel) {
                    findings.push(format!("{rel}:{}: {finding}", index + 1));
                }
            }
        }
    }
    assert!(
        findings.is_empty(),
        "files contain no environment, process or design references:\n{}",
        findings.join("\n")
    );
}

/// Formats whose files carry the producer header.
const HEADER_FORMATS: [&str; 9] = ["rs", "toml", "sh", "bash", "ps1", "py", "yml", "yaml", "md"];

#[test]
fn portability_license_headers_present() {
    let root = workspace_root();
    let copyright = Regex::new(&format!(
        r"Copyright \(c\) \d{{4}} {}",
        regex::escape(&producer())
    ))
    .expect("copyright pattern is valid");
    let identifier = ["SPDX-License-", "Identifier"].concat();
    let allow = hygiene_allow_list(&root);
    let mut findings = Vec::new();
    for path in domain_files(&root) {
        if !has_extension(&path, &HEADER_FORMATS) {
            continue;
        }
        let Some(text) = read_text(&path) else {
            continue;
        };
        let rel = relative(&root, &path);
        let has_header = text
            .lines()
            .take(HEADER_LINES)
            .any(|l| copyright.is_match(l));
        if !has_header && !is_allowed(&allow, "producer-header", &rel) {
            findings.push(format!("{rel}: producer header is missing"));
        }
        if text.contains(&identifier) && !is_allowed(&allow, "spdx-line", &rel) {
            findings.push(format!("{rel}: per-file license identifier"));
        }
    }
    assert!(
        findings.is_empty(),
        "source files start with the producer header:\n{}",
        findings.join("\n")
    );
}

/// Configuration, error and catalog sources that define keys and codes.
fn policy_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        root.join("spec/configuration.md"),
        root.join("spec/errors.md"),
        root.join("spec/schemas/nurt-config-1.schema.json"),
        root.join("crates/nurt-contracts/src/config.rs"),
        root.join("crates/nurt-contracts/src/error.rs"),
    ];
    for group in GROUPS {
        files.push(root.join("catalog").join(format!("{group}.toml")));
    }
    for krate in CRATES {
        files.extend(files_under(&root.join("crates").join(krate).join("src")));
    }
    files
}

#[test]
fn cfg_no_policy_keys() {
    let root = workspace_root();
    let pattern = Regex::new(concat!(
        r"(?i)ss[r]f|(?:allow|block)_?priv[a]te",
        r"|(?:allow(?:ed)?|block(?:ed)?|den(?:y|ied))_?hosts?\b",
        r"|\bhosts?_?(?:allow|block|deny)|\b(?:allow|block|deny)_?list",
    ))
    .expect("policy pattern is valid");
    let mut findings = Vec::new();
    for path in policy_sources(&root) {
        let text = read_text(&path).unwrap_or_else(|| panic!("{} is readable", path.display()));
        let rel = relative(&root, &path);
        for (index, line) in text.lines().enumerate() {
            if let Some(m) = pattern.find(line) {
                findings.push(format!("{rel}:{}: {}", index + 1, m.as_str()));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "configuration, catalog and code define no network policy:\n{}",
        findings.join("\n")
    );
}

#[test]
fn scanner_removes_comments_and_keeps_literals() {
    let source = concat!(
        "// Command::new(\"sh\")\n",
        "/* outer /* inner */ Stdio */ let a = \"launchctl\";\n",
        "let q = '\"'; fn f<'a>(x: &'a str) {}\n",
        "let r = r#\"say \"hi\"\"#; let b = b\"bytes\";\n",
    );
    let scanned = scan(source);
    assert!(!scanned.code.contains("Command"));
    assert!(!scanned.code.contains("Stdio"));
    assert!(scanned.code.contains("<'a>"));
    assert_eq!(scanned.code.lines().count(), source.lines().count());
    let values: Vec<&str> = scanned.literals.values().map(String::as_str).collect();
    assert_eq!(values, ["launchctl", "say \"hi\"", "bytes"]);
}

#[test]
fn scanner_drops_items_compiled_only_for_tests() {
    let source = concat!(
        "fn run() {}\n",
        "#[cfg(test)]\n#[allow(dead_code)]\nmod tests { fn f() { Command::new(\"x\"); } }\n",
        "#[cfg(test)]\nmod fixtures;\n",
    );
    let scanned = scan(source);
    assert!(scanned.code.contains("fn run"));
    assert!(!scanned.code.contains("Command"));
    assert!(scanned.literals.is_empty());
    assert_eq!(scanned.test_modules, ["fixtures"]);
}

#[test]
fn exec_sites_resolve_programs_through_constants() {
    let source = concat!(
        "const TOOL: &str = \"launchctl\";\n",
        "fn a() { ProgramSpec::new(TOOL, vec![], cwd); }\n",
        "fn b() { ServiceSpawnSpec { program: String::from(\"brew\"), args } }\n",
        "fn c(p: &str) { ProgramSpec::new(p.to_owned(), vec![], cwd); }\n",
    );
    let sites = exec_sites(&scan(source));
    let programs: Vec<Option<ProgramName>> = sites.into_iter().map(|s| s.program).collect();
    assert_eq!(
        programs,
        [
            Some(ProgramName::Literal("launchctl".to_owned())),
            Some(ProgramName::Literal("brew".to_owned())),
            Some(ProgramName::Dynamic("p.to_owned()".to_owned())),
        ]
    );
}

#[test]
fn site_rules_reject_programs_outside_the_site() {
    let exec = |program: ProgramName| ExecSite {
        line: 1,
        construct: "ProgramSpec::new(".to_owned(),
        program: Some(program),
    };
    let launchd = &SITES[0];
    let help = &SITES[5];
    let rel = launchd.path;
    assert!(
        program_findings(
            launchd,
            rel,
            &exec(ProgramName::Literal("launchctl".to_owned()))
        )
        .is_none()
    );
    assert!(program_findings(launchd, rel, &exec(ProgramName::Literal("sh".to_owned()))).is_some());
    assert!(
        program_findings(launchd, rel, &exec(ProgramName::Dynamic("name".to_owned()))).is_some()
    );
    assert!(program_findings(help, rel, &exec(ProgramName::Dynamic("path".to_owned()))).is_none());
    assert!(program_findings(help, rel, &exec(ProgramName::Literal("man".to_owned()))).is_some());
    assert!(
        program_findings(
            launchd,
            rel,
            &exec(ProgramName::Literal("/bin/launchctl".to_owned()))
        )
        .is_none()
    );
    assert!(
        program_findings(
            launchd,
            rel,
            &exec(ProgramName::Literal("/bin/sh".to_owned()))
        )
        .is_some()
    );
    assert_eq!(
        site_for("crates/nurt-sys/src/services/macos/plist.rs").map(|s| s.path),
        Some(launchd.path)
    );
    assert!(site_for("crates/nurt-sys/src/services/linux.rs").is_none());
}

#[test]
fn allow_list_entries_apply_by_check_group_and_glob() {
    assert!(glob_matches(
        b"crates/*/data/*.toml",
        b"crates/a/data/b/c.toml"
    ));
    assert!(!glob_matches(b"crates/*.rs", b"crates/a.toml"));
    let allow = [("content".to_owned(), "docs/*".to_owned())];
    assert!(is_allowed(&allow, "polish-text", "docs/a.md"));
    assert!(!is_allowed(&allow, "producer-header", "docs/a.md"));
    assert!(!is_allowed(&allow, "polish-text", "crates/a.rs"));
}

#[test]
fn version_option_with_a_value_is_not_a_probe() {
    let source = concat!(
        "fn a(v: &str) { args.extend([\"--version\".to_owned(), v.to_owned()]); }\n",
        "fn b(v: &str) { args.extend([\"--version\", v]); }\n",
        "fn c() { ProgramSpec::new(\"tool\", vec![\"--version\".into()], cwd); }\n",
        "fn d() { run(&[\"--version\"]); }\n",
    );
    let scanned = scan(source);
    let probes: Vec<usize> = scanned
        .literals
        .iter()
        .filter(|(offset, value)| is_version_probe(&scanned, **offset, value))
        .map(|(offset, _)| line_of(&scanned.code, *offset))
        .collect();
    assert_eq!(probes, [3, 4]);
}

#[test]
fn delegated_runs_require_specs_from_the_site_builder() {
    let file = |source: &str| ProductionFile {
        rel: DELEGATIONS[0].path.to_owned(),
        scanned: scan(source),
    };
    let built = file(concat!(
        "let spec = exec::program_spec(call, &defaults, cwd, timeout);\n",
        "let outcome = process.run_program(&spec, ctx.cancel());\n",
    ));
    assert!(runs_delegated_specs(&built));
    let own = file(concat!(
        "let spec = ProgramSpec::new(name, args, cwd);\n",
        "let outcome = process.run_program(&spec, ctx.cancel());\n",
    ));
    assert!(!runs_delegated_specs(&own));
    let elsewhere = ProductionFile {
        rel: "crates/nurt-sys/src/host/mod.rs".to_owned(),
        scanned: built.scanned,
    };
    assert!(!runs_delegated_specs(&elsewhere));
}

/// Systems of the cross-system matrix.
const SYSTEMS: [&str; 3] = ["linux", "windows", "macos"];

/// Number of commands in the catalog groups of [`GROUPS`].
const COMMAND_COUNT: usize = 64;

/// Levels of a matrix case: a test on synthetic data or doubles, a test on
/// the running system, a comparison with a system program, and the
/// `E_OS_ONLY` error on a system without the command.
const LEVELS: [&str; 4] = ["native", "system", "differential", "os_only"];

/// End-to-end scenarios that the matrix assigns to tests.
const SCENARIOS: [&str; 15] = [
    "e2e_linux_cron_restart",
    "e2e_linux_schedule_every_30s_10min_with_service_restart",
    "e2e_linux_logs_unit_matches_journalctl_json",
    "e2e_linux_user_mode_linger_survives_logout",
    "e2e_windows_test_service_lifecycle",
    "e2e_windows_registry_write_remove_undo",
    "e2e_windows_logs_scm_event",
    "e2e_macos_launch_agent_lifecycle",
    "e2e_macos_logs_subsystem_filter",
    "parity_tc_real_service_subset",
    "parity_tf_td_system_subset",
    "parity_ty_01_04",
    "e2e_os_only_on_foreign_systems",
    "service_system_mode_on_three_systems",
    "windows_user_mode_task_scheduler",
];

/// Run scripts of the matrix suites.
const RUN_SCRIPTS: [&str; 3] = ["run-linux.sh", "run-windows.ps1", "run-macos.sh"];

fn matrix_dir(root: &Path) -> PathBuf {
    root.join("tests").join("d7").join("system")
}

fn matrix(root: &Path) -> toml::Table {
    read_text(&matrix_dir(root).join("matrix.toml"))
        .expect("the matrix is readable")
        .parse()
        .expect("the matrix is valid TOML")
}

fn tables<'a>(table: &'a toml::Table, key: &str) -> Vec<&'a toml::Table> {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .collect()
}

fn strings(table: &toml::Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// Names of the functions defined in the Rust files of each crate of
/// [`CRATES`], including the test targets below `tests/d7`.
fn test_index(root: &Path) -> BTreeMap<&'static str, BTreeSet<String>> {
    let function = Regex::new(r"\bfn\s+([a-z][a-z0-9_]*)\s*[(<]").expect("pattern is valid");
    let names_in = |files: Vec<PathBuf>| {
        let mut names = BTreeSet::new();
        for path in files.iter().filter(|p| has_extension(p, &["rs"])) {
            if let Some(text) = read_text(path) {
                names.extend(function.captures_iter(&text).map(|c| c[1].to_owned()));
            }
        }
        names
    };
    let shared = names_in(files_under(&root.join("tests").join("d7")));
    CRATES
        .iter()
        .map(|krate| {
            let mut names = names_in(files_under(&root.join("crates").join(krate)));
            names.extend(shared.iter().cloned());
            (*krate, names)
        })
        .collect()
}

/// Findings of one test reference `{ crate, test }` of the matrix.
fn reference_findings(
    index: &BTreeMap<&'static str, BTreeSet<String>>,
    owner: &str,
    reference: &toml::Table,
) -> Option<String> {
    let krate = string_field(reference, "crate").unwrap_or_default();
    let test = string_field(reference, "test").unwrap_or_default();
    match index.get(krate) {
        None => Some(format!(
            "{owner}: crate `{krate}` is not a crate of the matrix"
        )),
        Some(names) if !names.contains(test) => Some(format!(
            "{owner}: test `{test}` does not exist in `{krate}`"
        )),
        Some(_) => None,
    }
}

/// Findings of one case of the matrix.
fn case_findings(
    index: &BTreeMap<&'static str, BTreeSet<String>>,
    commands: &BTreeMap<String, toml::Table>,
    case: &toml::Table,
) -> Vec<String> {
    let command = string_field(case, "command").unwrap_or_default();
    let level = string_field(case, "level").unwrap_or_default();
    let owner = format!("case {command}/{level}");
    let mut findings = Vec::new();
    if !commands.contains_key(command) {
        findings.push(format!("{owner}: command is not in the catalog"));
    }
    if !LEVELS.contains(&level) {
        findings.push(format!("{owner}: unknown level"));
    }
    let systems = strings(case, "systems");
    if systems.is_empty() || systems.iter().any(|s| !SYSTEMS.contains(&s.as_str())) {
        findings.push(format!("{owner}: systems must be a subset of {SYSTEMS:?}"));
    }
    let needs_condition = matches!(level, "system" | "differential");
    if needs_condition && string_field(case, "requires").is_none_or(str::is_empty) {
        findings.push(format!("{owner}: the precondition is not described"));
    }
    findings.extend(reference_findings(index, &owner, case));
    findings
}

/// Findings of the coverage of one command on every system.
fn coverage_findings(name: &str, command: &toml::Table, cases: &[&toml::Table]) -> Vec<String> {
    let mut supported = strings(command, "os");
    if supported.is_empty() {
        supported = SYSTEMS.iter().map(|s| (*s).to_owned()).collect();
    }
    let mut findings = Vec::new();
    for system in SYSTEMS {
        let levels: Vec<&str> = cases
            .iter()
            .filter(|c| string_field(c, "command") == Some(name))
            .filter(|c| strings(c, "systems").iter().any(|s| s == system))
            .filter_map(|c| string_field(c, "level"))
            .collect();
        if supported.iter().any(|s| s == system) {
            if !levels.iter().any(|l| *l != "os_only") {
                findings.push(format!("{name}: no case on {system}"));
            }
        } else if !levels.contains(&"os_only") {
            findings.push(format!("{name}: no `E_OS_ONLY` case on {system}"));
        }
    }
    findings
}

#[test]
fn matrix_covers_all_64_commands() {
    let root = workspace_root();
    let commands = domain_commands(&root);
    assert_eq!(
        commands.len(),
        COMMAND_COUNT,
        "the catalog groups define {COMMAND_COUNT} commands"
    );
    let matrix = matrix(&root);
    let cases = tables(&matrix, "case");
    let index = test_index(&root);
    let mut findings = Vec::new();
    for case in &cases {
        findings.extend(case_findings(&index, &commands, case));
    }
    for (name, command) in &commands {
        findings.extend(coverage_findings(name, command, &cases));
    }
    assert!(
        findings.is_empty(),
        "every command has a case on every system:\n{}",
        findings.join("\n")
    );
}

#[test]
fn matrix_scenarios_name_existing_tests() {
    let root = workspace_root();
    let matrix = matrix(&root);
    let index = test_index(&root);
    let scenarios = tables(&matrix, "scenario");
    let mut findings = Vec::new();
    for name in SCENARIOS {
        let Some(scenario) = scenarios
            .iter()
            .find(|s| string_field(s, "name") == Some(name))
        else {
            findings.push(format!("scenario {name}: missing from the matrix"));
            continue;
        };
        let tests = tables(scenario, "tests");
        let gap = string_field(scenario, "gap").unwrap_or_default();
        if tests.is_empty() && gap.is_empty() {
            findings.push(format!("scenario {name}: neither tests nor a gap"));
        }
        let owner = format!("scenario {name}");
        findings.extend(
            tests
                .iter()
                .filter_map(|t| reference_findings(&index, &owner, t)),
        );
    }
    assert!(
        findings.is_empty(),
        "the scenarios of the matrix name existing tests:\n{}",
        findings.join("\n")
    );
}

#[test]
fn matrix_suites_match_the_run_scripts() {
    let root = workspace_root();
    let matrix = matrix(&root);
    let suites = tables(&matrix, "suite");
    assert!(!suites.is_empty(), "the matrix defines suites");
    let mut findings = Vec::new();
    for script in RUN_SCRIPTS {
        let path = matrix_dir(&root).join(script);
        let Some(text) = read_text(&path) else {
            findings.push(format!("{script}: missing"));
            continue;
        };
        for suite in &suites {
            let name = string_field(suite, "name").unwrap_or_default();
            let arguments = string_field(suite, "test_arguments").unwrap_or_default();
            if !text.contains(&format!("{name})")) && !text.contains(&format!("'{name}'")) {
                findings.push(format!("{script}: suite `{name}` is not handled"));
            }
            if !arguments.is_empty() && !text.contains(arguments) {
                findings.push(format!(
                    "{script}: suite `{name}` does not pass `{arguments}`"
                ));
            }
        }
        for krate in CRATES {
            if !text.contains(krate) {
                findings.push(format!("{script}: crate `{krate}` is not tested"));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "the run scripts implement the suites of the matrix:\n{}",
        findings.join("\n")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
