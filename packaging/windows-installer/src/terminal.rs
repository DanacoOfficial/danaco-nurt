// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/terminal.rs
// Responsibility:   Integration with terminals: the profile of Windows Terminal, its default profile and the tab completion of PowerShell.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Integration with terminals.
//!
//! Every function that changes a file returns what the journal needs to undo the
//! change, and every undo function takes those records back. The files of the user
//! are edited as text and written back with the bytes outside the edit unchanged.

use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::journal::{ProfileRecord, SettingsRecord};
use crate::model::UserPaths;
use crate::paths;
use crate::shortcuts::CREATE_NO_WINDOW;
use crate::textedit::{
    POWERSHELL_GUID, PROFILE_GUID, default_profile, profile_with_block, profile_without_block,
    remove_default_profile, set_default_profile, terminal_fragment,
};

/// Name of the file of the fragment.
pub(crate) const FRAGMENT_FILE: &str = "nurt.json";

/// Byte order mark of UTF-8.
const BOM: char = '\u{feff}';

/// Writes the fragment with the profile; `directory` is the folder `Danaco Nurt` below `Fragments`.
pub(crate) fn write_fragment(directory: &Path, exe: &Path, icon: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    fs::write(
        directory.join(FRAGMENT_FILE),
        terminal_fragment(&exe.display().to_string(), &icon.display().to_string()),
    )
}

/// The error of a file that cannot be edited as text.
fn invalid(path: &Path, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{}: {reason}", path.display()),
    )
}

/// Result of the work on several files: what changed, or the first error when nothing changed.
fn settle<T>(changed: Vec<T>, failure: Option<io::Error>) -> io::Result<Vec<T>> {
    match failure {
        Some(error) if changed.is_empty() => Err(error),
        _ => Ok(changed),
    }
}

/// Makes the profile the default profile in one `settings.json`; `None` when it already is.
fn set_default_in(path: &Path) -> io::Result<Option<SettingsRecord>> {
    let text = fs::read_to_string(path)?;
    let previous = default_profile(&text);
    if previous.as_deref() == Some(PROFILE_GUID) {
        return Ok(None);
    }
    let updated = set_default_profile(&text, PROFILE_GUID)
        .ok_or_else(|| invalid(path, "the settings are not a JSON object"))?;
    fs::write(path, updated)?;
    Ok(Some(SettingsRecord {
        path: path.display().to_string(),
        previous,
    }))
}

/// Makes the profile the default profile in every `settings.json` of Windows Terminal that exists.
///
/// A file that cannot be edited does not stop the others; its error is returned only when no
/// file changed.
pub(crate) fn set_default(user: &UserPaths) -> io::Result<Vec<SettingsRecord>> {
    let mut changed = Vec::new();
    let mut failure = None;
    for path in paths::terminal_settings(user) {
        match set_default_in(&path) {
            Ok(Some(record)) => changed.push(record),
            Ok(None) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    settle(changed, failure)
}

/// Gives the default profile back in one file, where it is still the profile of the program.
fn release_default_in(record: &SettingsRecord) -> io::Result<()> {
    let path = Path::new(&record.path);
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(());
    };
    if default_profile(&text).as_deref() != Some(PROFILE_GUID) {
        return Ok(());
    }
    let updated = match record.previous.as_deref() {
        Some(previous) => set_default_profile(&text, previous),
        None => remove_default_profile(&text, PROFILE_GUID)
            .or_else(|| set_default_profile(&text, POWERSHELL_GUID)),
    };
    match updated {
        Some(updated) => fs::write(path, updated),
        None => Ok(()),
    }
}

/// Gives the default profile back where it is still the profile of the program.
///
/// When the file named no default profile before, the line that the setup inserted is removed;
/// when the user has edited the file since, the default becomes Windows PowerShell.
pub(crate) fn release_default(files: &[SettingsRecord]) -> io::Result<()> {
    let mut failure = None;
    for record in files {
        if let Err(error) = release_default_in(record) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Text of a profile of PowerShell, with the way to write it back.
///
/// A profile that is not UTF-8 is read byte by byte (each byte is one character), so that the
/// bytes outside the block come back unchanged; the block itself is ASCII.
struct ProfileText {
    /// The text to edit.
    text: String,
    /// Each byte of the file is one character of the text.
    bytes: bool,
}

impl ProfileText {
    /// Reads a profile; a file in UTF-16 is refused, because the block would corrupt it.
    fn read(path: &Path) -> io::Result<Self> {
        let raw = fs::read(path)?;
        if raw.starts_with(&[0xFF, 0xFE]) || raw.starts_with(&[0xFE, 0xFF]) {
            return Err(invalid(path, "the profile is encoded in UTF-16"));
        }
        Ok(match String::from_utf8(raw) {
            Ok(text) => Self { text, bytes: false },
            Err(error) => Self {
                text: error.into_bytes().into_iter().map(char::from).collect(),
                bytes: true,
            },
        })
    }

    /// The bytes of `text` in the encoding that the profile was read in.
    fn encode(&self, text: &str) -> Vec<u8> {
        if self.bytes {
            text.chars()
                .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
                .collect()
        } else {
            text.as_bytes().to_vec()
        }
    }
}

/// Adds the block to one profile; `None` when the profile has it already.
fn add_completion_in(path: &Path, script: &str) -> io::Result<Option<ProfileRecord>> {
    let existing = if path.is_file() {
        Some(ProfileText::read(path)?)
    } else {
        None
    };
    let before = existing
        .as_ref()
        .map_or("", |profile| profile.text.as_str());
    let updated = profile_with_block(before, script);
    if existing.is_some() && updated == before {
        return Ok(None);
    }
    let bytes = match &existing {
        Some(profile) => profile.encode(&updated),
        // A new file gets a byte order mark: Windows PowerShell 5.1 reads a file without it as ANSI.
        None => format!("{BOM}{updated}").into_bytes(),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(Some(ProfileRecord {
        path: path.display().to_string(),
        existed: existing.is_some(),
    }))
}

/// Adds the block that loads the completion to the given profiles of PowerShell.
///
/// A profile that cannot be edited does not stop the others; its error is returned only when no
/// profile changed.
pub(crate) fn add_completion_to(
    profiles: &[std::path::PathBuf],
    script: &Path,
) -> io::Result<Vec<ProfileRecord>> {
    let script = script.display().to_string();
    let mut changed = Vec::new();
    let mut failure = None;
    for path in profiles {
        match add_completion_in(path, &script) {
            Ok(Some(record)) => changed.push(record),
            Ok(None) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    settle(changed, failure)
}

/// The execution policy lets PowerShell run an unsigned profile of the user.
fn policy_runs_profiles(policy: &str) -> bool {
    matches!(
        policy.trim().to_ascii_lowercase().as_str(),
        "remotesigned" | "unrestricted" | "bypass"
    )
}

/// Tells whether PowerShell runs the profile `profile` of the user. The host of
/// `Documents\WindowsPowerShell` is Windows PowerShell, the host of `Documents\PowerShell` is
/// PowerShell 7; the effective execution policy of the host must allow an unsigned script. Windows
/// sets Restricted by default, and then every start of PowerShell reports that the profile cannot be
/// loaded. A PowerShell 7 that is not installed never reads its profile, so the block is harmless
/// there.
pub(crate) fn profile_runs(profile: &Path) -> bool {
    let windows_powershell = profile
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name.eq_ignore_ascii_case("WindowsPowerShell"));
    let host = if windows_powershell {
        crate::shortcuts::powershell()
    } else {
        std::path::PathBuf::from("pwsh.exe")
    };
    let output = Command::new(host)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-ExecutionPolicy).ToString()",
        ])
        // The policy of a process scope comes from this variable of the parent; a PowerShell that
        // the user opens has none, so the probe asks without it.
        .env_remove("PSExecutionPolicyPreference")
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            policy_runs_profiles(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(_) => false,
        Err(_) => !windows_powershell,
    }
}

/// Removes the block from one profile; a profile that the setup created and that holds nothing
/// else is deleted.
fn remove_completion_in(record: &ProfileRecord) -> io::Result<()> {
    let path = Path::new(&record.path);
    let Ok(profile) = ProfileText::read(path) else {
        return Ok(());
    };
    let stripped = profile_without_block(&profile.text);
    if stripped == profile.text {
        return Ok(());
    }
    if !record.existed && stripped.trim_start_matches(BOM).trim().is_empty() {
        fs::remove_file(path)?;
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
        return Ok(());
    }
    fs::write(path, profile.encode(&stripped))
}

/// Removes the block of the completion from the profiles of PowerShell.
pub(crate) fn remove_completion(files: &[ProfileRecord]) -> io::Result<()> {
    let mut failure = None;
    for record in files {
        if let Err(error) = remove_completion_in(record) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A directory of its own for a test, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("nurt-setup-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            let _ = fs::create_dir_all(&path);
            Self(path)
        }

        fn user(&self) -> UserPaths {
            UserPaths {
                profile: self.0.clone(),
                appdata: self.0.join("AppData\\Roaming"),
                local_appdata: self.0.join("AppData\\Local"),
                documents: self.0.join("Documents"),
                desktop: self.0.join("Desktop"),
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, bytes);
    }

    fn settings_path(user: &UserPaths) -> PathBuf {
        user.local_appdata
            .join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json")
    }

    #[test]
    fn fragment_is_written_below_the_directory() {
        let scratch = Scratch::new("fragment");
        let directory = scratch.0.join("Fragments").join("Danaco Nurt");
        assert!(
            write_fragment(
                &directory,
                Path::new(r"C:\n\bin\nurt.exe"),
                Path::new(r"C:\n\nurt.ico")
            )
            .is_ok()
        );
        let text = fs::read_to_string(directory.join(FRAGMENT_FILE)).unwrap_or_default();
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
        assert!(text.contains(PROFILE_GUID));
    }

    /// Writes the fragment to the folder named by `NURT_SETUP_FRAGMENT_DIR`, for the check by hand
    /// on a machine with Windows Terminal: `NURT_SETUP_FRAGMENT_EXE` and `NURT_SETUP_FRAGMENT_ICON`
    /// name the program and the icon.
    #[test]
    #[ignore = "writes into the folder of the fragments of the user"]
    fn fragment_is_written_for_the_check_by_hand() {
        let variable = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .unwrap_or_default()
        };
        let directory = variable("NURT_SETUP_FRAGMENT_DIR");
        assert!(
            write_fragment(
                &directory,
                &variable("NURT_SETUP_FRAGMENT_EXE"),
                &variable("NURT_SETUP_FRAGMENT_ICON")
            )
            .is_ok()
        );
    }

    #[test]
    fn default_profile_is_set_and_given_back() {
        let scratch = Scratch::new("default");
        let user = scratch.user();
        let path = settings_path(&user);
        let original = "\u{feff}{\r\n    // keep\r\n    \"defaultProfile\": \"{old}\",\r\n    \"profiles\": { \"list\": [ ], },\r\n}\r\n";
        write(&path, original.as_bytes());
        let records = set_default(&user).unwrap_or_default();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].previous.as_deref(), Some("{old}"));
        let text = fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(default_profile(&text).as_deref(), Some(PROFILE_GUID));
        assert!(set_default(&user).unwrap_or_default().is_empty());
        assert!(release_default(&records).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap_or_default(), original);
    }

    #[test]
    fn missing_default_is_inserted_and_removed_again() {
        let scratch = Scratch::new("missing");
        let user = scratch.user();
        let path = settings_path(&user);
        let original = "{\n    \"profiles\": {}\n}\n";
        write(&path, original.as_bytes());
        let records = set_default(&user).unwrap_or_default();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].previous, None);
        assert!(release_default(&records).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap_or_default(), original);
    }

    #[test]
    fn default_is_left_when_the_user_chose_another_profile() {
        let scratch = Scratch::new("chosen");
        let user = scratch.user();
        let path = settings_path(&user);
        write(&path, b"{ \"defaultProfile\": \"{old}\" }");
        let records = set_default(&user).unwrap_or_default();
        write(&path, b"{ \"defaultProfile\": \"{mine}\" }");
        assert!(release_default(&records).is_ok());
        assert_eq!(
            fs::read_to_string(&path).unwrap_or_default(),
            "{ \"defaultProfile\": \"{mine}\" }"
        );
    }

    #[test]
    fn a_broken_settings_file_is_an_error_only_when_nothing_changed() {
        let scratch = Scratch::new("broken");
        let user = scratch.user();
        write(&settings_path(&user), b"[ not an object ]");
        assert!(set_default(&user).is_err());
        write(
            &user
                .local_appdata
                .join(r"Microsoft\Windows Terminal\settings.json"),
            b"{ }",
        );
        assert_eq!(set_default(&user).unwrap_or_default().len(), 1);
    }

    #[test]
    fn only_policies_that_run_an_unsigned_profile_allow_the_completion() {
        for policy in ["RemoteSigned", "Unrestricted\r\n", "Bypass"] {
            assert!(policy_runs_profiles(policy), "{policy}");
        }
        for policy in ["Restricted", "AllSigned", "Undefined", ""] {
            assert!(!policy_runs_profiles(policy), "{policy}");
        }
    }

    #[test]
    fn completion_is_added_to_both_profiles_and_removed_byte_for_byte() {
        let scratch = Scratch::new("completion");
        let user = scratch.user();
        let profiles = paths::powershell_profiles(&user);
        let utf8_bom = "\u{feff}Set-Alias ll ls\r\n# \u{105}\u{119}\r\n"
            .as_bytes()
            .to_vec();
        let ansi = b"Set-Alias ll ls\r\n# \xb1\xea no newline".to_vec();
        write(&profiles[0], &utf8_bom);
        write(&profiles[1], &ansi);
        let script = Path::new(r"C:\Users\user\Programs\Nurt\completions\_nurt.ps1");
        let records = add_completion_to(&profiles, script).unwrap_or_default();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| record.existed));
        assert!(
            fs::read(&profiles[0])
                .unwrap_or_default()
                .starts_with(&utf8_bom)
        );
        assert!(
            fs::read(&profiles[1])
                .unwrap_or_default()
                .starts_with(&ansi)
        );
        assert!(
            add_completion_to(&profiles, script)
                .unwrap_or_default()
                .is_empty()
        );
        assert!(remove_completion(&records).is_ok());
        assert_eq!(fs::read(&profiles[0]).unwrap_or_default(), utf8_bom);
        assert_eq!(fs::read(&profiles[1]).unwrap_or_default(), ansi);
    }

    #[test]
    fn a_profile_created_by_the_setup_is_deleted_with_its_block() {
        let scratch = Scratch::new("created");
        let user = scratch.user();
        let script = Path::new(r"C:\n\completions\_nurt.ps1");
        let profiles = paths::powershell_profiles(&user);
        let records = add_completion_to(&profiles, script).unwrap_or_default();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| !record.existed));
        let created = fs::read(&records[0].path).unwrap_or_default();
        assert!(created.starts_with("\u{feff}".as_bytes()));
        assert!(remove_completion(&records).is_ok());
        assert!(
            records
                .iter()
                .all(|record| !Path::new(&record.path).exists())
        );
    }

    #[test]
    fn a_profile_in_utf16_is_not_touched() {
        let scratch = Scratch::new("utf16");
        let user = scratch.user();
        let profiles = paths::powershell_profiles(&user);
        let utf16 = [0xFF, 0xFE, b'a', 0, b'\n', 0];
        write(&profiles[0], &utf16);
        let records =
            add_completion_to(&profiles, Path::new(r"C:\n\_nurt.ps1")).unwrap_or_default();
        assert_eq!(records.len(), 1);
        assert_eq!(fs::read(&profiles[0]).unwrap_or_default(), utf16);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
