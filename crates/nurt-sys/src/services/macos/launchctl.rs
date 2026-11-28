// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::services::macos::launchctl
// Responsibility:   Parsers of the output of `launchctl print` and `launchctl print-disabled`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsers of the output of `launchctl print` and `launchctl print-disabled`.
//!
//! The output of `launchctl` is meant for people and is not a stable
//! interface. The parsers read it as a tree of `key = value` pairs, nested
//! `key = {` … `}` blocks and plain lines, ignore keys they do not use and
//! report a missing section instead of failing.

use std::collections::BTreeMap;

use nurt_contracts::record::ServiceState;

/// Name of the block that lists the services of a domain.
const SERVICES_BLOCK: &str = "services";
/// Name of the block that lists the enabled and disabled overrides.
const DISABLED_BLOCK: &str = "disabled services";
/// Last exit status of a service that never exited.
const NEVER_EXITED: &str = "(never exited)";
/// Placeholder used in the services block for an absent value.
const ABSENT: &str = "-";

/// Raw state of a running service.
pub(crate) const STATE_RUNNING: &str = "running";
/// Raw state of a loaded service without a process.
pub(crate) const STATE_NOT_RUNNING: &str = "not running";

/// One block of the output.
#[derive(Debug, Default)]
struct Block {
    pairs: Vec<(String, String)>,
    items: Vec<String>,
    children: Vec<(String, Block)>,
}

impl Block {
    fn child(&self, key: &str) -> Option<&Self> {
        self.children
            .iter()
            .find_map(|(name, block)| (name == key).then_some(block))
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find_map(|(name, value)| (name == key).then_some(value.as_str()))
    }

    /// The block of the domain or service: the first top-level block, or
    /// the root when the output has no enclosing block.
    fn subject(&self) -> &Self {
        self.children.first().map_or(self, |(_, block)| block)
    }
}

fn parse_tree(text: &str) -> Block {
    let mut stack: Vec<(String, Block)> = vec![(String::new(), Block::default())];
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(key) = line.strip_suffix('{') {
            let key = key.trim_end().trim_end_matches("=>").trim_end_matches('=');
            stack.push((key.trim().to_owned(), Block::default()));
        } else if line == "}" {
            if stack.len() > 1
                && let Some((key, block)) = stack.pop()
                && let Some((_, parent)) = stack.last_mut()
            {
                parent.children.push((key, block));
            }
        } else if let Some((_, block)) = stack.last_mut() {
            match line.split_once(" = ") {
                Some((key, value)) => block
                    .pairs
                    .push((key.trim().to_owned(), value.trim().to_owned())),
                None => block.items.push(line.to_owned()),
            }
        }
    }
    while stack.len() > 1 {
        if let Some((key, block)) = stack.pop()
            && let Some((_, parent)) = stack.last_mut()
        {
            parent.children.push((key, block));
        }
    }
    stack.pop().map(|(_, block)| block).unwrap_or_default()
}

/// How a service last ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LastExit {
    /// The service never exited.
    Never,
    /// The service exited with a status.
    Code(i64),
    /// The service was ended by a signal.
    Signal(String),
    /// A value the parser does not know, as written.
    Other(String),
}

impl LastExit {
    /// Text of the reason, for the status of a failed service.
    pub(crate) fn describe(&self) -> Option<String> {
        match self {
            Self::Never => None,
            Self::Code(code) => Some(code.to_string()),
            Self::Signal(signal) => Some(format!("signal {signal}")),
            Self::Other(text) => Some(text.clone()),
        }
    }
}

/// A row of the services block of `launchctl print <domain>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedService {
    /// Label of the service.
    pub(crate) label: String,
    /// Process of the running service.
    pub(crate) pid: Option<u64>,
    /// Last exit status.
    pub(crate) last_exit: LastExit,
}

/// Services of a domain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DomainListing {
    /// Rows of the services block; `None` when the output has no such
    /// block.
    pub(crate) services: Option<Vec<ListedService>>,
    /// Lines of the services block that could not be read.
    pub(crate) skipped: usize,
}

/// Fields of `launchctl print <domain>/<label>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceDetail {
    /// Value of `state`.
    pub(crate) state_raw: Option<String>,
    /// Value of `pid`.
    pub(crate) pid: Option<u64>,
    /// Value of `last exit code` or `last terminating signal`.
    pub(crate) last_exit: LastExit,
    /// Value of `path`: the property list the service was loaded from.
    pub(crate) path: Option<String>,
    /// Value of `program`.
    pub(crate) program: Option<String>,
}

fn parse_status(text: &str) -> LastExit {
    if text == ABSENT {
        return LastExit::Never;
    }
    match text.parse::<i64>() {
        Ok(code) if code < 0 => LastExit::Signal(code.unsigned_abs().to_string()),
        Ok(code) => LastExit::Code(code),
        Err(_) => LastExit::Other(text.to_owned()),
    }
}

fn parse_row(line: &str) -> Option<ListedService> {
    let mut tokens = line.split_whitespace();
    let pid = match tokens.next()? {
        ABSENT => None,
        token => Some(token.parse::<u64>().ok()?).filter(|pid| *pid != 0),
    };
    let last_exit = parse_status(tokens.next()?);
    let label = tokens.collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        return None;
    }
    Some(ListedService {
        label,
        pid,
        last_exit,
    })
}

/// Reads the services block of `launchctl print <domain>`.
pub(crate) fn parse_domain(text: &str) -> DomainListing {
    let tree = parse_tree(text);
    let Some(block) = tree.subject().child(SERVICES_BLOCK) else {
        return DomainListing::default();
    };
    let mut services = Vec::with_capacity(block.items.len());
    let mut skipped = 0;
    for line in &block.items {
        match parse_row(line) {
            Some(service) => services.push(service),
            None => skipped += 1,
        }
    }
    skipped += block.pairs.len() + block.children.len();
    DomainListing {
        services: Some(services),
        skipped,
    }
}

/// Reads the overrides of `launchctl print-disabled <domain>`: `true` for
/// a disabled service, `false` for an enabled one.
pub(crate) fn parse_disabled(text: &str) -> BTreeMap<String, bool> {
    let tree = parse_tree(text);
    let block = tree
        .child(DISABLED_BLOCK)
        .or_else(|| tree.subject().child(DISABLED_BLOCK));
    let mut overrides = BTreeMap::new();
    for line in block
        .map(|block| block.items.as_slice())
        .unwrap_or_default()
    {
        let Some((label, value)) = line.split_once("=>") else {
            continue;
        };
        let disabled = match value.trim() {
            "disabled" | "true" => true,
            "enabled" | "false" => false,
            _ => continue,
        };
        overrides.insert(label.trim().trim_matches('"').to_owned(), disabled);
    }
    overrides
}

fn parse_last_exit(block: &Block) -> LastExit {
    let exit = match block.value("last exit code") {
        None | Some(NEVER_EXITED) => LastExit::Never,
        Some(text) => {
            let number = text.split(':').next().unwrap_or_default().trim();
            number
                .parse()
                .map_or_else(|_| LastExit::Other(text.to_owned()), LastExit::Code)
        }
    };
    match (&exit, block.value("last terminating signal")) {
        (LastExit::Never, Some(signal)) => LastExit::Signal(signal.to_owned()),
        _ => exit,
    }
}

/// Reads `launchctl print <domain>/<label>`.
pub(crate) fn parse_service(text: &str) -> ServiceDetail {
    let tree = parse_tree(text);
    let block = tree.subject();
    ServiceDetail {
        state_raw: block.value("state").map(str::to_owned),
        pid: block.value("pid").and_then(|pid| pid.parse().ok()),
        last_exit: parse_last_exit(block),
        path: block.value("path").map(str::to_owned),
        program: block.value("program").map(str::to_owned),
    }
}

/// Maps the raw state of launchd to the common enumeration.
///
/// A loaded service that is not running is `stopped` when it never ran,
/// exited with status 0 or was ended by a signal, and `failed` after a
/// non-zero exit status.
pub(crate) fn map_state(raw: &str, last_exit: &LastExit) -> ServiceState {
    match raw {
        STATE_RUNNING => ServiceState::Running,
        STATE_NOT_RUNNING => match last_exit {
            LastExit::Code(code) if *code != 0 => ServiceState::Failed,
            _ => ServiceState::Stopped,
        },
        "spawn scheduled" | "waiting" => ServiceState::Starting,
        _ => ServiceState::Unknown,
    }
}

/// State and raw state of a row of the services block.
pub(crate) fn listed_state(service: &ListedService) -> (ServiceState, &'static str) {
    let raw = if service.pid.is_some() {
        STATE_RUNNING
    } else {
        STATE_NOT_RUNNING
    };
    (map_state(raw, &service.last_exit), raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/d7/u07/",
                $name
            ))
        };
    }

    fn row(listing: &DomainListing, label: &str) -> ListedService {
        listing
            .services
            .as_ref()
            .unwrap()
            .iter()
            .find(|service| service.label == label)
            .unwrap()
            .clone()
    }

    #[test]
    fn launchd_parse_print_domain_services_section() {
        let fixtures = [
            fixture!("print-system-macos13.txt"),
            fixture!("print-system-macos14.txt"),
            fixture!("print-system-macos15.txt"),
        ];
        for text in fixtures {
            let listing = parse_domain(text);
            assert_eq!(listing.skipped, 0);
            assert_eq!(listing.services.as_ref().unwrap().len(), 4);
            let running = row(&listing, "com.openssh.sshd");
            assert_eq!(running.pid, Some(412));
            assert_eq!(
                listed_state(&running),
                (ServiceState::Running, STATE_RUNNING)
            );
            let idle = row(&listing, "org.example.backup");
            assert_eq!(idle.pid, None);
            assert_eq!(idle.last_exit, LastExit::Never);
            assert_eq!(listed_state(&idle).0, ServiceState::Stopped);
            let failed = row(&listing, "org.example.agentd");
            assert_eq!(failed.last_exit, LastExit::Code(78));
            assert_eq!(listed_state(&failed).0, ServiceState::Failed);
            let killed = row(&listing, "com.apple.example.killed");
            assert_eq!(killed.last_exit, LastExit::Signal("9".to_owned()));
            assert_eq!(listed_state(&killed).0, ServiceState::Stopped);
        }
        let disabled = parse_disabled(fixture!("print-system-macos13.txt"));
        assert_eq!(disabled.get("com.apple.ftpd"), Some(&true));
    }

    #[test]
    fn launchd_parse_print_disabled_overrides() {
        let overrides = parse_disabled(fixture!("print-disabled-system.txt"));
        assert_eq!(overrides.get("org.example.backup"), Some(&true));
        assert_eq!(overrides.get("com.openssh.sshd"), Some(&false));
        assert_eq!(overrides.get("org.example.legacy"), Some(&true));
        assert_eq!(overrides.get("org.example.unknown"), None);
        assert!(parse_disabled("").is_empty());
    }

    #[test]
    fn launchd_parse_print_service_state_pid_exit_code() {
        let running = parse_service(fixture!("print-service-running.txt"));
        assert_eq!(running.state_raw.as_deref(), Some(STATE_RUNNING));
        assert_eq!(running.pid, Some(412));
        assert_eq!(running.last_exit, LastExit::Never);
        assert_eq!(
            running.path.as_deref(),
            Some("/System/Library/LaunchDaemons/ssh.plist")
        );
        assert_eq!(
            running.program.as_deref(),
            Some("/usr/libexec/sshd-keygen-wrapper")
        );
        assert_eq!(
            map_state(STATE_RUNNING, &running.last_exit),
            ServiceState::Running
        );

        let failed = parse_service(fixture!("print-service-failed.txt"));
        assert_eq!(failed.pid, None);
        assert_eq!(failed.last_exit, LastExit::Code(78));
        assert_eq!(failed.last_exit.describe().as_deref(), Some("78"));
        assert_eq!(
            map_state(failed.state_raw.as_deref().unwrap(), &failed.last_exit),
            ServiceState::Failed
        );

        let stopped = parse_service(fixture!("print-service-signal.txt"));
        assert_eq!(
            stopped.last_exit,
            LastExit::Signal("Terminated: 15".to_owned())
        );
        assert_eq!(
            map_state(stopped.state_raw.as_deref().unwrap(), &stopped.last_exit),
            ServiceState::Stopped
        );
    }

    #[test]
    fn launchd_map_state_pending_and_unknown_states() {
        assert_eq!(
            map_state("spawn scheduled", &LastExit::Never),
            ServiceState::Starting
        );
        assert_eq!(
            map_state("waiting", &LastExit::Never),
            ServiceState::Starting
        );
        assert_eq!(map_state("exited", &LastExit::Never), ServiceState::Unknown);
        assert_eq!(LastExit::Never.describe(), None);
        assert_eq!(
            LastExit::Other("?".to_owned()).describe().as_deref(),
            Some("?")
        );
    }

    #[test]
    fn launchd_parse_tolerates_stray_braces_quoted_labels_and_unreadable_rows() {
        let overrides = parse_disabled(
            "}\ndisabled services = {\n\t\"org.example.a\" => disabled\n\t\"org.example.b\" => enabled\n\t\"org.example.c\" => later\n\tnot a pair\n}\n}\n",
        );
        assert_eq!(overrides.len(), 2);
        assert_eq!(overrides.get("org.example.a"), Some(&true));
        assert_eq!(overrides.get("org.example.b"), Some(&false));

        let listing = parse_domain(
            "}\nsystem = {\n\tservices = {\n\t\t-5 0 org.example.negative\n\t\t   7   -9   org.example.signal\n\t\t   0   -   org.example.zero\n\t}\n}\n}\n",
        );
        assert_eq!(listing.skipped, 1);
        assert_eq!(listing.services.as_ref().unwrap().len(), 2);
        let signalled = row(&listing, "org.example.signal");
        assert_eq!(signalled.pid, Some(7));
        assert_eq!(signalled.last_exit, LastExit::Signal("9".to_owned()));
        let zero = row(&listing, "org.example.zero");
        assert_eq!(zero.pid, None);
        assert_eq!(zero.last_exit, LastExit::Never);
        assert_eq!(
            LastExit::Signal("9".to_owned()).describe().as_deref(),
            Some("signal 9")
        );
    }

    #[test]
    fn launchd_parse_unknown_keys_tolerated_warns() {
        let text = "gui/501 = {\n\tfuture key = 7\n\tnested future = {\n\t\tx = 1\n\t}\n}\n";
        assert_eq!(parse_domain(text).services, None);

        let odd = "system = {\n\tservices = {\n\t\t     12      -     org.example.a\n\t\tgarbage\n\t\tx y\n\t\tcolumns = 3\n\t}\n\tunknown = {\n";
        let listing = parse_domain(odd);
        assert_eq!(listing.services.as_ref().unwrap().len(), 1);
        assert_eq!(listing.skipped, 3);

        let detail = parse_service(
            "svc = {\n\tstate = brand new\n\tpid = none\n\tlast exit code = 1: EPERM\n}\n",
        );
        assert_eq!(detail.state_raw.as_deref(), Some("brand new"));
        assert_eq!(detail.pid, None);
        assert_eq!(detail.last_exit, LastExit::Code(1));
        let other = parse_service("svc = {\n\tlast exit code = (unknown)\n}\n}\n");
        assert_eq!(other.last_exit, LastExit::Other("(unknown)".to_owned()));
        assert_eq!(other.state_raw, None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
