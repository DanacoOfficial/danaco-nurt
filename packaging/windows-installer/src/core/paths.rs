// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/paths.rs
// Responsibility:   Directories of the installation: defaults of the scopes, folders of the user, places of shortcuts and of the configuration.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Directories of the installation.

use std::path::{Path, PathBuf};

use crate::model::{Scope, UserPaths};
use crate::registry;

/// Expands `%NAME%` references to environment variables; unknown names stay as written.
pub(crate) fn expand_env(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('%') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('%') {
            Some(close) if close > 0 => match std::env::var(&after[..close]) {
                Ok(value) => {
                    out.push_str(&value);
                    rest = &after[close + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn variable(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_default()
}

impl UserPaths {
    /// Directories of the user of the current process.
    pub(crate) fn current() -> Self {
        let profile = variable("USERPROFILE");
        let appdata = variable("APPDATA");
        let local_appdata = variable("LOCALAPPDATA");
        let documents =
            registry::shell_folder("Personal").unwrap_or_else(|| profile.join("Documents"));
        let desktop = registry::shell_folder("Desktop").unwrap_or_else(|| profile.join("Desktop"));
        Self {
            profile,
            appdata,
            local_appdata,
            documents,
            desktop,
        }
    }
}

/// Directories below which an installation for all users is allowed.
pub(crate) fn program_files_roots() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .into_iter()
        .map(variable)
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

/// Default folder of `nurt.exe` for a scope.
pub(crate) fn default_bin(scope: Scope, user: &UserPaths) -> PathBuf {
    match scope {
        Scope::User => user.local_appdata.join("Programs").join("Nurt").join("bin"),
        Scope::Machine => {
            let root = variable("ProgramW6432");
            let root = if root.as_os_str().is_empty() {
                variable("ProgramFiles")
            } else {
                root
            };
            root.join("Nurt").join("bin")
        }
    }
}

/// Directory of the Start menu programs of a scope.
pub(crate) fn start_menu(scope: Scope, user: &UserPaths) -> PathBuf {
    match scope {
        Scope::User => user.appdata.join(r"Microsoft\Windows\Start Menu\Programs"),
        Scope::Machine => variable("ProgramData").join(r"Microsoft\Windows\Start Menu\Programs"),
    }
}

/// Directory of the desktop of a scope.
pub(crate) fn desktop(scope: Scope, user: &UserPaths) -> PathBuf {
    match scope {
        Scope::User => user.desktop.clone(),
        Scope::Machine => {
            let public = variable("PUBLIC");
            if public.as_os_str().is_empty() {
                user.desktop.clone()
            } else {
                public.join("Desktop")
            }
        }
    }
}

/// Directory of the Quick Launch shortcuts of the user.
pub(crate) fn quick_launch(user: &UserPaths) -> PathBuf {
    user.appdata
        .join(r"Microsoft\Internet Explorer\Quick Launch")
}

/// Directory of the fragment of Windows Terminal of a scope.
pub(crate) fn terminal_fragments(scope: Scope, user: &UserPaths) -> PathBuf {
    let base = match scope {
        Scope::User => user
            .local_appdata
            .join(r"Microsoft\Windows Terminal\Fragments"),
        Scope::Machine => variable("ProgramData").join(r"Microsoft\Windows Terminal\Fragments"),
    };
    base.join("Danaco Nurt")
}

/// Files `settings.json` of the Windows Terminal variants that exist for the user.
pub(crate) fn terminal_settings(user: &UserPaths) -> Vec<PathBuf> {
    let packages = user.local_appdata.join("Packages");
    [
        packages.join(r"Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"),
        packages.join(r"Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json"),
        user.local_appdata
            .join(r"Microsoft\Windows Terminal\settings.json"),
    ]
    .into_iter()
    .filter(|path| path.is_file())
    .collect()
}

/// Windows Terminal is installed for the user.
pub(crate) fn terminal_installed(user: &UserPaths) -> bool {
    user.local_appdata
        .join(r"Microsoft\WindowsApps\wt.exe")
        .exists()
        || user
            .local_appdata
            .join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe")
            .exists()
        || user
            .local_appdata
            .join(r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe")
            .exists()
}

/// Profile files of PowerShell 7 and of Windows PowerShell that apply to every host of the user.
pub(crate) fn powershell_profiles(user: &UserPaths) -> Vec<PathBuf> {
    vec![
        user.documents.join(r"PowerShell\profile.ps1"),
        user.documents.join(r"WindowsPowerShell\profile.ps1"),
    ]
}

/// File `nurt.toml` of the user.
pub(crate) fn user_config(user: &UserPaths) -> PathBuf {
    user.appdata.join(r"Nurt\nurt.toml")
}

/// Directory of the temporary files of the setup.
pub(crate) fn temp() -> PathBuf {
    std::env::temp_dir()
}

/// Nearest existing ancestor of a path, the path itself when it exists.
pub(crate) fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|candidate| candidate.exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_variables_only() {
        let known = std::env::vars().find(|(name, value)| {
            !name.is_empty() && !name.contains('%') && !value.is_empty() && !name.contains('=')
        });
        if let Some((name, value)) = known {
            assert_eq!(
                expand_env(&format!("%{name}%\\Nurt")),
                format!("{value}\\Nurt")
            );
        }
        assert_eq!(
            expand_env("%NURT_SETUP_UNKNOWN%\\x"),
            "%NURT_SETUP_UNKNOWN%\\x"
        );
        assert_eq!(expand_env("100%"), "100%");
        assert_eq!(expand_env("a%%b"), "a%%b");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
