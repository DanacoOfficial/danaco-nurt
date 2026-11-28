// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::install::detect
// Responsibility:   Choice of the installation mode from facts about the host.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Choice of the installation mode from facts about the host.
//!
//! Without administrator rights the mode is `user`. With them, a host that
//! looks like a server gets mode `system`, which starts the service at boot
//! without a login session; a desktop gets mode `user`.
//!
//! | Fact | Linux | macOS | Windows |
//! |---|---|---|---|
//! | graphical session | `DISPLAY`, `WAYLAND_DISPLAY` or a graphical `XDG_SESSION_TYPE` | the account owns `/dev/console` | always |
//! | default target | `org.freedesktop.systemd1.Manager.GetDefaultTarget` | none | none |
//! | service account | login shell `nologin` or `false` | login shell `nologin` or `false` | none |
//! | server edition | none | none | `ProductType` of `RtlGetVersion` is not a workstation |

use nurt_contracts::config::ServiceMode;
use serde::Serialize;

const SERVER_TARGET: &str = "multi-user.target";
const NON_LOGIN_SHELLS: &[&str] = &["nologin", "false"];

/// Why an installation mode was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeReason {
    /// The caller asked for the mode.
    Requested,
    /// The account has no administrator rights.
    NoPrivilege,
    /// The host looks like a server.
    Server,
    /// The host has a graphical session or boots into one.
    Desktop,
}

/// Chosen installation mode with its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModeChoice {
    /// Mode.
    pub mode: ServiceMode,
    /// Reason.
    pub reason: ModeReason,
}

/// Facts about the host used to choose the mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostFacts {
    /// The process has administrator rights.
    pub elevated: bool,
    /// A graphical session of the account is present.
    pub graphical_session: bool,
    /// Default systemd target, when known.
    pub default_target: Option<String>,
    /// The account has a login shell that refuses logins.
    pub service_account: bool,
    /// The operating system is a server edition.
    pub server_edition: bool,
}

/// Chooses the mode for the facts.
///
/// ```
/// use nurt_contracts::config::ServiceMode;
/// use nurt_service::install::detect::{HostFacts, ModeReason, choose_mode};
///
/// let facts = HostFacts {
///     elevated: true,
///     default_target: Some("multi-user.target".to_owned()),
///     ..HostFacts::default()
/// };
/// let choice = choose_mode(&facts);
/// assert_eq!((choice.mode, choice.reason), (ServiceMode::System, ModeReason::Server));
/// ```
#[must_use]
pub fn choose_mode(facts: &HostFacts) -> ModeChoice {
    if !facts.elevated {
        return ModeChoice {
            mode: ServiceMode::User,
            reason: ModeReason::NoPrivilege,
        };
    }
    let headless_target = facts
        .default_target
        .as_deref()
        .is_none_or(|target| target == SERVER_TARGET);
    let server = facts.server_edition
        || facts.service_account
        || (!facts.graphical_session && headless_target);
    if server {
        ModeChoice {
            mode: ServiceMode::System,
            reason: ModeReason::Server,
        }
    } else {
        ModeChoice {
            mode: ServiceMode::User,
            reason: ModeReason::Desktop,
        }
    }
}

/// Whether a login shell refuses logins.
#[must_use]
pub fn is_non_login_shell(shell: &str) -> bool {
    std::path::Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| NON_LOGIN_SHELLS.contains(&name))
}

/// Whether the environment variables describe a graphical session (Linux).
#[must_use]
pub fn graphical_from_env(var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|value| !value.is_empty());
    set("DISPLAY")
        || set("WAYLAND_DISPLAY")
        || var("XDG_SESSION_TYPE")
            .is_some_and(|kind| matches!(kind.as_str(), "x11" | "wayland" | "mir"))
}

/// Facts about the running host.
#[must_use]
pub fn host_facts() -> HostFacts {
    imp::host_facts()
}

#[cfg(unix)]
fn account_shell_refuses_login() -> bool {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .and_then(|user| user.shell.to_str().map(is_non_login_shell))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{HostFacts, account_shell_refuses_login, graphical_from_env};

    pub(super) fn host_facts() -> HostFacts {
        HostFacts {
            elevated: crate::process::is_elevated(),
            graphical_session: graphical_from_env(|name| std::env::var(name).ok()),
            default_target: super::super::linux::default_target(),
            service_account: account_shell_refuses_login(),
            server_edition: false,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{HostFacts, account_shell_refuses_login};
    use std::os::unix::fs::MetadataExt;

    const CONSOLE: &str = "/dev/console";

    pub(super) fn host_facts() -> HostFacts {
        let uid = nix::unistd::getuid().as_raw();
        let console_owner = std::fs::metadata(CONSOLE)
            .map(|metadata| metadata.uid())
            .ok();
        HostFacts {
            elevated: crate::process::is_elevated(),
            graphical_session: console_owner == Some(uid),
            default_target: None,
            service_account: account_shell_refuses_login(),
            server_edition: false,
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::HostFacts;

    pub(super) fn host_facts() -> HostFacts {
        HostFacts {
            elevated: crate::process::is_elevated(),
            graphical_session: true,
            default_target: None,
            service_account: false,
            server_edition: crate::process::is_server_edition(),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    use super::HostFacts;

    pub(super) fn host_facts() -> HostFacts {
        HostFacts::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_detect_mode_matrix() {
        let base = HostFacts {
            elevated: true,
            ..HostFacts::default()
        };
        let cases = [
            (
                HostFacts {
                    elevated: false,
                    server_edition: true,
                    ..base.clone()
                },
                ServiceMode::User,
                ModeReason::NoPrivilege,
            ),
            (
                HostFacts {
                    default_target: Some("multi-user.target".into()),
                    ..base.clone()
                },
                ServiceMode::System,
                ModeReason::Server,
            ),
            (
                HostFacts {
                    default_target: Some("graphical.target".into()),
                    ..base.clone()
                },
                ServiceMode::User,
                ModeReason::Desktop,
            ),
            (
                HostFacts {
                    graphical_session: true,
                    default_target: Some("multi-user.target".into()),
                    ..base.clone()
                },
                ServiceMode::User,
                ModeReason::Desktop,
            ),
            (
                HostFacts {
                    graphical_session: true,
                    service_account: true,
                    ..base.clone()
                },
                ServiceMode::System,
                ModeReason::Server,
            ),
            (
                HostFacts {
                    graphical_session: true,
                    server_edition: true,
                    ..base.clone()
                },
                ServiceMode::System,
                ModeReason::Server,
            ),
            (base.clone(), ServiceMode::System, ModeReason::Server),
        ];
        for (facts, mode, reason) in cases {
            assert_eq!(
                choose_mode(&facts),
                ModeChoice { mode, reason },
                "{facts:?}"
            );
        }
    }

    #[test]
    fn shells_and_session_variables() {
        assert!(is_non_login_shell("/usr/sbin/nologin"));
        assert!(is_non_login_shell("/bin/false"));
        assert!(!is_non_login_shell("/bin/bash"));
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert!(graphical_from_env(env(&[("WAYLAND_DISPLAY", "wayland-0")])));
        assert!(graphical_from_env(env(&[("XDG_SESSION_TYPE", "x11")])));
        assert!(!graphical_from_env(env(&[
            ("XDG_SESSION_TYPE", "tty"),
            ("DISPLAY", "")
        ])));
        let _ = host_facts();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
