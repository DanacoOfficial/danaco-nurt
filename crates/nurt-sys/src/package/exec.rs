// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Package changes through the program of the manager.
//!
//! This module is the only place where the package commands start a program.
//! The programs are started through the process host of the execution
//! context, with the arguments as a list and without a shell.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::PackageManager;
use nurt_contracts::services::{EnvVar, ProgramSpec};

/// Kind of change requested from a manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Install packages.
    Install,
    /// Remove packages.
    Remove,
    /// Upgrade packages.
    Upgrade,
}

impl Action {
    /// Name of the action as reported in the effect record.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Remove => "remove",
            Self::Upgrade => "upgrade",
        }
    }
}

/// One program invocation of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// Program name.
    pub program: String,
    /// Arguments without the program name.
    pub args: Vec<String>,
    /// Variables set on top of the configured defaults.
    pub env: Vec<(String, String)>,
}

/// Invocations that perform a change, with remarks for the report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangePlan {
    /// Invocations in execution order.
    pub invocations: Vec<Invocation>,
    /// Remarks about the change.
    pub warnings: Vec<String>,
}

fn invocation(program: &str, args: &[&str], names: &[String], env: &[(&str, &str)]) -> Invocation {
    Invocation {
        program: program.to_owned(),
        args: args
            .iter()
            .map(|arg| (*arg).to_owned())
            .chain(names.iter().cloned())
            .collect(),
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn unsupported_version(manager: PackageManager) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!("{manager} cannot install a chosen version; omit `version`"),
    )
}

const APT_ENV: [(&str, &str); 2] = [
    ("DEBIAN_FRONTEND", "noninteractive"),
    ("APT_LISTCHANGES_FRONTEND", "none"),
];
const BREW_ENV: [(&str, &str); 2] = [("HOMEBREW_NO_AUTO_UPDATE", "1"), ("NONINTERACTIVE", "1")];
const WINGET_QUIET: [&str; 2] = ["--silent", "--disable-interactivity"];
const WINGET_AGREEMENTS: [&str; 2] = [
    "--accept-package-agreements",
    "--accept-source-agreements",
];

/// Builds the invocations that apply `action` to `names` through `manager`.
///
/// `names` is empty only for an upgrade of all packages.
///
/// # Errors
///
/// Returns `E_UNSUPPORTED` when the manager cannot install the requested
/// `version`.
pub fn build(
    manager: PackageManager,
    action: Action,
    names: &[String],
    version: Option<&str>,
) -> Result<ChangePlan> {
    let mut plan = ChangePlan::default();
    let pinned = |separator: &str| -> Vec<String> {
        names
            .iter()
            .map(|name| match version {
                Some(version) => format!("{name}{separator}{version}"),
                None => name.clone(),
            })
            .collect()
    };
    if version.is_some() && action != Action::Install {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "`version` applies to install only",
        ));
    }
    let invocations = match (manager, action) {
        (PackageManager::Apt, Action::Install) => vec![invocation(
            "apt-get",
            &[
                "install",
                "-y",
                "-q",
                "-o",
                "Dpkg::Options::=--force-confdef",
                "-o",
                "Dpkg::Options::=--force-confold",
            ],
            &pinned("="),
            &APT_ENV,
        )],
        (PackageManager::Apt, Action::Remove) => {
            vec![invocation("apt-get", &["remove", "-y", "-q"], names, &APT_ENV)]
        }
        (PackageManager::Apt, Action::Upgrade) if names.is_empty() => {
            vec![invocation("apt-get", &["upgrade", "-y", "-q"], &[], &APT_ENV)]
        }
        (PackageManager::Apt, Action::Upgrade) => vec![invocation(
            "apt-get",
            &["install", "--only-upgrade", "-y", "-q"],
            names,
            &APT_ENV,
        )],
        (PackageManager::Dnf, Action::Install) => {
            vec![invocation("dnf", &["install", "-y"], &pinned("-"), &[])]
        }
        (PackageManager::Dnf, Action::Remove) => {
            vec![invocation("dnf", &["remove", "-y"], names, &[])]
        }
        (PackageManager::Dnf, Action::Upgrade) => {
            vec![invocation("dnf", &["upgrade", "-y"], names, &[])]
        }
        (PackageManager::Pacman, Action::Install) => {
            if version.is_some() {
                return Err(unsupported_version(manager));
            }
            vec![invocation("pacman", &["-S", "--noconfirm", "--needed"], names, &[])]
        }
        (PackageManager::Pacman, Action::Remove) => {
            vec![invocation("pacman", &["-R", "--noconfirm"], names, &[])]
        }
        (PackageManager::Pacman, Action::Upgrade) if names.is_empty() => {
            vec![invocation("pacman", &["-Syu", "--noconfirm"], &[], &[])]
        }
        (PackageManager::Pacman, Action::Upgrade) => {
            plan.warnings.push(
                "pacman upgrades the named packages without the rest of the system, which is a partial upgrade"
                    .to_owned(),
            );
            vec![invocation("pacman", &["-S", "--noconfirm"], names, &[])]
        }
        (PackageManager::Zypper, Action::Install) => vec![invocation(
            "zypper",
            &["--non-interactive", "install"],
            &pinned("="),
            &[],
        )],
        (PackageManager::Zypper, Action::Remove) => vec![invocation(
            "zypper",
            &["--non-interactive", "remove"],
            names,
            &[],
        )],
        (PackageManager::Zypper, Action::Upgrade) => vec![invocation(
            "zypper",
            &["--non-interactive", "update"],
            names,
            &[],
        )],
        (PackageManager::Brew, Action::Install) => {
            vec![invocation("brew", &["install"], &pinned("@"), &BREW_ENV)]
        }
        (PackageManager::Brew, Action::Remove) => {
            vec![invocation("brew", &["uninstall"], names, &BREW_ENV)]
        }
        (PackageManager::Brew, Action::Upgrade) => {
            vec![invocation("brew", &["upgrade"], names, &BREW_ENV)]
        }
        (PackageManager::Choco, Action::Install) => {
            let mut built = invocation("choco", &["install"], names, &[]);
            built.args.extend(["-y", "--no-progress"].map(str::to_owned));
            if let Some(version) = version {
                built.args.extend(["--version".to_owned(), version.to_owned()]);
            }
            vec![built]
        }
        (PackageManager::Choco, Action::Remove) => {
            let mut built = invocation("choco", &["uninstall"], names, &[]);
            built.args.extend(["-y", "--no-progress"].map(str::to_owned));
            vec![built]
        }
        (PackageManager::Choco, Action::Upgrade) => {
            let targets = if names.is_empty() { vec!["all".to_owned()] } else { names.to_vec() };
            let mut built = invocation("choco", &["upgrade"], &targets, &[]);
            built.args.extend(["-y", "--no-progress"].map(str::to_owned));
            vec![built]
        }
        (PackageManager::Scoop, Action::Install) => {
            vec![invocation("scoop", &["install"], &pinned("@"), &[])]
        }
        (PackageManager::Scoop, Action::Remove) => {
            vec![invocation("scoop", &["uninstall"], names, &[])]
        }
        (PackageManager::Scoop, Action::Upgrade) => {
            let targets = if names.is_empty() { vec!["*".to_owned()] } else { names.to_vec() };
            vec![invocation("scoop", &["update"], &targets, &[])]
        }
        (PackageManager::Winget, Action::Upgrade) if names.is_empty() => {
            let mut args = vec!["upgrade", "--all"];
            args.extend(WINGET_QUIET);
            args.extend(WINGET_AGREEMENTS);
            vec![invocation("winget", &args, &[], &[])]
        }
        (PackageManager::Winget, _) => names
            .iter()
            .map(|name| {
                let verb = match action {
                    Action::Install => "install",
                    Action::Remove => "uninstall",
                    Action::Upgrade => "upgrade",
                };
                let mut args = vec![verb, "--id", name.as_str(), "--exact"];
                args.extend(WINGET_QUIET);
                if action != Action::Remove {
                    args.extend(WINGET_AGREEMENTS);
                }
                if let Some(version) = version {
                    args.extend(["--version", version]);
                }
                invocation("winget", &args, &[], &[])
            })
            .collect(),
        _ => {
            return Err(Error::new(
                ErrorCode::Unsupported,
                format!("{manager} cannot change packages"),
            ));
        }
    };
    plan.invocations = invocations;
    Ok(plan)
}

/// Program specification of `invocation` with the configured default
/// variables followed by the variables of the manager.
#[must_use]
pub(crate) fn program_spec(
    invocation: &Invocation,
    defaults: &[(String, String)],
    cwd: &str,
    timeout_ms: Option<u64>,
) -> ProgramSpec {
    let mut spec = ProgramSpec::new(invocation.program.clone(), invocation.args.clone(), cwd);
    spec.env = defaults
        .iter()
        .chain(invocation.env.iter())
        .map(|(name, value)| EnvVar {
            name: name.clone(),
            value: Some(value.clone()),
        })
        .collect();
    spec.timeout_ms = timeout_ms;
    spec
}

/// Program started by `manager` for package changes.
#[must_use]
pub(crate) fn program_name(manager: PackageManager) -> &'static str {
    match manager {
        PackageManager::Apt | PackageManager::Dpkg => "apt-get",
        PackageManager::Dnf | PackageManager::Rpm => "dnf",
        PackageManager::Pacman => "pacman",
        PackageManager::Zypper => "zypper",
        PackageManager::Brew => "brew",
        PackageManager::Winget => "winget",
        PackageManager::Choco => "choco",
        PackageManager::Scoop => "scoop",
        _ => "",
    }
}

/// Lock files of the managers, under the system root.
#[must_use]
pub(crate) fn lock_files(manager: PackageManager) -> &'static [&'static str] {
    match manager {
        PackageManager::Apt => &["/var/lib/dpkg/lock-frontend"],
        PackageManager::Pacman => &["/var/lib/pacman/db.lck"],
        PackageManager::Zypper => &["/var/run/zypp.pid"],
        _ => &[],
    }
}

/// Privilege the manager needs on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Privilege {
    /// The effective user must be the superuser.
    Root,
    /// The program refuses to run as the superuser.
    NotRoot,
    /// The manager asks for elevation by itself.
    None,
}

/// Privilege required by `manager` on a Unix system.
#[must_use]
pub(crate) fn privilege_of(manager: PackageManager) -> Privilege {
    match manager {
        PackageManager::Apt
        | PackageManager::Dnf
        | PackageManager::Pacman
        | PackageManager::Zypper => Privilege::Root,
        PackageManager::Brew => Privilege::NotRoot,
        _ => Privilege::None,
    }
}

/// Checks the effective user against the privilege `manager` needs.
///
/// # Errors
///
/// Returns `E_PRIVILEGE` when the user does not fit.
pub(crate) fn check_privilege(manager: PackageManager, is_root: bool, unix: bool) -> Result<()> {
    if !unix {
        return Ok(());
    }
    match (privilege_of(manager), is_root) {
        (Privilege::Root, false) => Err(Error::new(
            ErrorCode::Privilege,
            format!("{manager} needs the superuser; run the program as root"),
        )
        .with_detail("required", serde_json::json!("root"))),
        (Privilege::NotRoot, true) => Err(Error::new(
            ErrorCode::Privilege,
            format!("{manager} refuses to run as the superuser; run the program as a regular user"),
        )
        .with_detail("required", serde_json::json!("user"))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn install_apt_argv_and_env_noninteractive() {
        let plan = build(PackageManager::Apt, Action::Install, &names(&["curl", "jq"]), Some("1.0")).expect("plan");
        let call = &plan.invocations[0];
        assert_eq!(call.program, "apt-get");
        assert_eq!(call.args[..3], ["install", "-y", "-q"]);
        assert_eq!(call.args[7..], ["curl=1.0", "jq=1.0"]);
        assert!(call.env.contains(&("DEBIAN_FRONTEND".to_owned(), "noninteractive".to_owned())));
        assert!(call.env.contains(&("APT_LISTCHANGES_FRONTEND".to_owned(), "none".to_owned())));
    }

    #[test]
    fn install_pacman_dnf_zypper_brew_choco_scoop_argv() {
        let one = names(&["tool"]);
        let line = |manager, action, version| {
            let plan = build(manager, action, &one, version).expect("plan");
            let call = &plan.invocations[0];
            format!("{} {}", call.program, call.args.join(" "))
        };
        assert_eq!(line(PackageManager::Pacman, Action::Install, None), "pacman -S --noconfirm --needed tool");
        assert_eq!(line(PackageManager::Pacman, Action::Remove, None), "pacman -R --noconfirm tool");
        assert_eq!(line(PackageManager::Dnf, Action::Install, Some("2")), "dnf install -y tool-2");
        assert_eq!(line(PackageManager::Zypper, Action::Install, Some("2")), "zypper --non-interactive install tool=2");
        assert_eq!(line(PackageManager::Zypper, Action::Upgrade, None), "zypper --non-interactive update tool");
        assert_eq!(line(PackageManager::Brew, Action::Install, Some("2")), "brew install tool@2");
        assert_eq!(line(PackageManager::Brew, Action::Remove, None), "brew uninstall tool");
        assert_eq!(line(PackageManager::Choco, Action::Install, Some("2")), "choco install tool -y --no-progress --version 2");
        assert_eq!(line(PackageManager::Choco, Action::Remove, None), "choco uninstall tool -y --no-progress");
        assert_eq!(line(PackageManager::Scoop, Action::Install, Some("2")), "scoop install tool@2");
        assert_eq!(line(PackageManager::Scoop, Action::Remove, None), "scoop uninstall tool");
        assert_eq!(
            line(PackageManager::Winget, Action::Install, None),
            "winget install --id tool --exact --silent --disable-interactivity --accept-package-agreements --accept-source-agreements"
        );
        let brew = build(PackageManager::Brew, Action::Install, &one, None).expect("plan");
        assert!(brew.invocations[0].env.contains(&("NONINTERACTIVE".to_owned(), "1".to_owned())));
    }

    #[test]
    fn upgrade_all_and_partial_pacman_upgrade() {
        let all = build(PackageManager::Apt, Action::Upgrade, &[], None).expect("plan");
        assert_eq!(all.invocations[0].args, ["upgrade", "-y", "-q"]);
        let some = build(PackageManager::Apt, Action::Upgrade, &names(&["a"]), None).expect("plan");
        assert_eq!(some.invocations[0].args[..2], ["install", "--only-upgrade"]);
        let pacman_all = build(PackageManager::Pacman, Action::Upgrade, &[], None).expect("plan");
        assert_eq!(pacman_all.invocations[0].args, ["-Syu", "--noconfirm"]);
        assert!(pacman_all.warnings.is_empty());
        let partial = build(PackageManager::Pacman, Action::Upgrade, &names(&["a"]), None).expect("plan");
        assert_eq!(partial.warnings.len(), 1);
        let scoop = build(PackageManager::Scoop, Action::Upgrade, &[], None).expect("plan");
        assert_eq!(scoop.invocations[0].args, ["update", "*"]);
        let winget = build(PackageManager::Winget, Action::Upgrade, &names(&["a", "b"]), None).expect("plan");
        assert_eq!(winget.invocations.len(), 2);
    }

    #[test]
    fn rejects_version_for_pacman_and_for_removal() {
        let error = build(PackageManager::Pacman, Action::Install, &names(&["a"]), Some("1")).expect_err("unsupported");
        assert_eq!(error.code(), ErrorCode::Unsupported);
        assert!(build(PackageManager::Apt, Action::Remove, &names(&["a"]), Some("1")).is_err());
    }

    #[test]
    fn install_without_root_privilege_error() {
        let error = check_privilege(PackageManager::Apt, false, true).expect_err("needs root");
        assert_eq!(error.code(), ErrorCode::Privilege);
        assert!(check_privilege(PackageManager::Apt, true, true).is_ok());
        assert_eq!(
            check_privilege(PackageManager::Brew, true, true).expect_err("refuses root").code(),
            ErrorCode::Privilege
        );
        assert!(check_privilege(PackageManager::Brew, false, true).is_ok());
        assert!(check_privilege(PackageManager::Choco, false, false).is_ok());
    }

    #[test]
    fn program_spec_orders_defaults_before_manager_variables() {
        let call = invocation("apt-get", &["x"], &[], &APT_ENV);
        let spec = program_spec(&call, &[("A".to_owned(), "1".to_owned())], "/", Some(5));
        assert_eq!(spec.env[0].name, "A");
        assert_eq!(spec.env[1].name, "DEBIAN_FRONTEND");
        assert_eq!(spec.timeout_ms, Some(5));
    }
}
