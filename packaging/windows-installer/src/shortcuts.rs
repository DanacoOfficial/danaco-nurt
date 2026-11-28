// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/shortcuts.rs
// Responsibility:   Shortcuts of the installation: creation of .lnk files through the scripting host of Windows.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Shortcuts of the installation.
//!
//! A `.lnk` file is written by the COM class `WScript.Shell`, driven by one
//! hidden PowerShell process that gets the list of the shortcuts as JSON in an
//! environment variable, so that no path is ever quoted into a script.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::model::Layout;

/// Flag of `CreateProcess`: no console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Name of the program in shortcuts and in the Start menu.
pub(crate) const PROGRAM_NAME: &str = "Danaco Nurt";

/// Name of the shortcut that undoes the setup.
pub(crate) const UNDO_NAME: &str = "Uninstall Danaco Nurt";

/// Shortcut to write.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Link {
    /// Path of the `.lnk` file.
    pub(crate) path: PathBuf,
    /// Target of the shortcut.
    pub(crate) target: String,
    /// Arguments of the target.
    pub(crate) arguments: String,
    /// Working directory.
    pub(crate) workdir: PathBuf,
    /// Icon, a file name or `file,index`.
    pub(crate) icon: String,
    /// Description shown by the shell.
    pub(crate) description: String,
}

/// Script of the PowerShell process. The list is always a JSON array. `ConvertFrom-Json -InputObject`
/// returns the array as one object in Windows PowerShell 5.1 and as separate objects in PowerShell 7;
/// the variable then holds the array in 5.1 and an array or the single object in 7, and `foreach`
/// reads every one of these forms. No `@()` and no pipe: in 5.1 either one nests the array.
const SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
$items = ConvertFrom-Json -InputObject $env:NURT_SETUP_LINKS; \
$shell = New-Object -ComObject WScript.Shell; \
foreach ($item in $items) { \
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $item.path) | Out-Null; \
  $link = $shell.CreateShortcut($item.path); \
  $link.TargetPath = $item.target; \
  $link.Arguments = $item.arguments; \
  $link.WorkingDirectory = $item.workdir; \
  $link.IconLocation = $item.icon; \
  $link.Description = $item.description; \
  $link.Save() \
}";

fn powershell() -> PathBuf {
    let root =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// Writes the shortcuts; the error is the message of the scripting host.
pub(crate) fn create(links: &[Link]) -> Result<(), String> {
    write_with(&powershell(), links)
}

/// Writes the shortcuts with the given PowerShell executable.
fn write_with(shell: &Path, links: &[Link]) -> Result<(), String> {
    if links.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_string(links).map_err(|error| error.to_string())?;
    let output = Command::new(shell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCRIPT,
        ])
        .env("NURT_SETUP_LINKS", json)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("powershell.exe: {error}"))?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if text.is_empty() {
            format!("powershell.exe exited with {}", output.status)
        } else {
            text
        });
    }
    for link in links {
        if !link.path.is_file() {
            return Err(format!(
                "the shortcut was not written: {}",
                link.path.display()
            ));
        }
    }
    Ok(())
}

/// The shortcut that opens a terminal with `nurt`: the profile of Windows Terminal when `terminal`
/// holds the full path of its `wt.exe`, otherwise `nurt.exe` itself, which opens a console with the
/// banner of the session. The target of the first form is always a full path: the shell resolves a
/// bare `wt.exe` through no search path and shows the dialog of a missing shortcut.
pub(crate) fn program_link(
    path: PathBuf,
    layout: &Layout,
    home: &Path,
    terminal: Option<&Path>,
) -> Link {
    let (target, arguments) = match terminal {
        Some(wt) => (wt.display().to_string(), format!("-p \"{PROGRAM_NAME}\"")),
        None => (layout.exe().display().to_string(), String::new()),
    };
    Link {
        path,
        target,
        arguments,
        workdir: home.to_path_buf(),
        icon: layout.icon().display().to_string(),
        description: "Danaco Nurt: a command shell and language where every change can be undone"
            .to_string(),
    }
}

/// The shortcut that undoes the setup.
pub(crate) fn undo_link(path: PathBuf, layout: &Layout) -> Link {
    Link {
        path,
        target: layout.uninstaller().display().to_string(),
        arguments: "--undo".to_string(),
        workdir: layout.root.clone(),
        icon: format!("{},0", layout.uninstaller().display()),
        description: UNDO_NAME.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::of(Path::new(r"C:\Users\user\Programs\Nurt\bin"), true)
    }

    fn link(path: PathBuf, name: &str) -> Link {
        Link {
            path,
            target: r"C:\Windows\System32\cmd.exe".to_string(),
            arguments: format!("/c echo \"{name}\""),
            workdir: PathBuf::from(r"C:\Windows"),
            icon: r"C:\Windows\System32\cmd.exe,0".to_string(),
            description: name.to_string(),
        }
    }

    /// Reads a shortcut back through the scripting host: target, arguments and description.
    fn read_back(path: &Path) -> String {
        let script = "$l = (New-Object -ComObject WScript.Shell).CreateShortcut($env:NURT_TEST_LINK); \
            [Console]::Out.Write($l.TargetPath + '|' + $l.Arguments + '|' + $l.Description)";
        let output = Command::new(powershell())
            .args(["-NoProfile", "-Command", script])
            .env("NURT_TEST_LINK", path)
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        output
            .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
            .unwrap_or_default()
    }

    fn shells() -> Vec<PathBuf> {
        let mut shells = vec![powershell()];
        let seven = PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe");
        if seven.is_file() {
            shells.push(seven);
        }
        shells
    }

    #[test]
    fn one_and_several_shortcuts_are_written_by_every_shell() {
        let root = std::env::temp_dir().join(format!("nurt-setup-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (index, shell) in shells().iter().enumerate() {
            let one = root.join(format!("one-{index}")).join("Danaco Nurt.lnk");
            assert_eq!(
                write_with(shell, &[link(one.clone(), "first")]),
                Ok(()),
                "{}",
                shell.display()
            );
            assert_eq!(
                read_back(&one),
                "C:\\Windows\\System32\\cmd.exe|/c echo \"first\"|first"
            );
            let many: Vec<Link> = (0..3)
                .map(|n| {
                    link(
                        root.join(format!("many-{index}"))
                            .join(format!("Link {n} \u{105}.lnk")),
                        &format!("link {n}"),
                    )
                })
                .collect();
            assert_eq!(write_with(shell, &many), Ok(()), "{}", shell.display());
            for (n, item) in many.iter().enumerate() {
                assert_eq!(
                    read_back(&item.path),
                    format!("C:\\Windows\\System32\\cmd.exe|/c echo \"link {n}\"|link {n}")
                );
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failure_of_the_host_is_returned_as_text() {
        let bad = Link {
            path: PathBuf::from("Z:\\no-such-drive\\x.lnk"),
            ..link(PathBuf::new(), "x")
        };
        assert!(create(&[bad]).is_err());
        assert_eq!(create(&[]), Ok(()));
    }

    #[test]
    fn the_program_opens_the_profile_of_windows_terminal_or_the_executable() {
        let layout = layout();
        let home = Path::new(r"C:\Users\user");
        let wt = Path::new(r"C:\Users\user\AppData\Local\Microsoft\WindowsApps\wt.exe");
        let profile = program_link(PathBuf::from("p.lnk"), &layout, home, Some(wt));
        assert_eq!(
            (profile.target.as_str(), profile.arguments.as_str()),
            (
                r"C:\Users\user\AppData\Local\Microsoft\WindowsApps\wt.exe",
                "-p \"Danaco Nurt\""
            )
        );
        let plain = program_link(PathBuf::from("p.lnk"), &layout, home, None);
        assert_eq!(plain.target, layout.exe().display().to_string());
        assert!(plain.arguments.is_empty());
        assert_eq!(plain.icon, layout.icon().display().to_string());
        assert_eq!(plain.workdir, home);
    }

    #[test]
    fn the_undo_shortcut_runs_the_uninstaller() {
        let layout = layout();
        let undo = undo_link(PathBuf::from("u.lnk"), &layout);
        assert_eq!(undo.target, layout.uninstaller().display().to_string());
        assert_eq!(undo.arguments, "--undo");
        assert_eq!(undo.description, UNDO_NAME);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
