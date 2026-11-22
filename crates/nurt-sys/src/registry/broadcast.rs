// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Notification of programs after a change of persistent environment
//! variables.
//!
//! Windows keeps persistent environment variables in two registry keys and
//! reads them when a process starts. Running programs, such as Explorer,
//! reload them when they receive `WM_SETTINGCHANGE` with the parameter
//! `"Environment"`. The provider sends the message after every change below
//! one of the two keys; the message itself cannot be undone.

use super::path::{Hive, KeyPath};

/// Key of the persistent environment variables of the current account.
pub const USER_ENVIRONMENT: &[&str] = &["Environment"];

/// Key of the persistent environment variables of the machine, below `HKLM`.
pub const MACHINE_ENVIRONMENT: &[&str] = &[
    "SYSTEM",
    "CurrentControlSet",
    "Control",
    "Session Manager",
    "Environment",
];

/// Receiver of the notification about changed environment variables.
pub trait SettingChangeNotifier: Send + Sync {
    /// Announces that persistent environment variables changed.
    fn environment_changed(&self);
}

/// Locations of the persistent environment variables: `HKCU\Environment`
/// and `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`.
pub fn environment_keys() -> [KeyPath; 2] {
    [
        KeyPath::new(Hive::CurrentUser, owned(USER_ENVIRONMENT)),
        KeyPath::new(Hive::LocalMachine, owned(MACHINE_ENVIRONMENT)),
    ]
}

/// Whether a change of `key` or of a value in it changes persistent
/// environment variables: `key` is an environment key, lies below one, or
/// contains one.
///
/// ```
/// use nurt_sys::registry::broadcast::affects_environment;
/// use nurt_sys::registry::path::RegistryPath;
///
/// let key = RegistryPath::parse("registry:/HKCU/environment")?.as_key().unwrap();
/// assert!(affects_environment(&key));
/// let other = RegistryPath::parse("registry:/HKCU/Software")?.as_key().unwrap();
/// assert!(!affects_environment(&other));
/// # Ok::<(), nurt_sys::registry::path::PathError>(())
/// ```
pub fn affects_environment(key: &KeyPath) -> bool {
    environment_keys()
        .iter()
        .any(|environment| key.starts_with(environment) || environment.starts_with(key))
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[cfg(windows)]
pub use self::win32::Win32Broadcast;

#[cfg(windows)]
mod win32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    use super::SettingChangeNotifier;

    /// Time each window may take to process the message, in milliseconds.
    const TIMEOUT_MS: u32 = 5_000;

    /// Section name carried by `WM_SETTINGCHANGE` for environment variables.
    const ENVIRONMENT_SECTION: &str = "Environment";

    /// Sends `WM_SETTINGCHANGE` to every top-level window.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct Win32Broadcast;

    impl SettingChangeNotifier for Win32Broadcast {
        fn environment_changed(&self) {
            let section: Vec<u16> = ENVIRONMENT_SECTION
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut result: usize = 0;
            // SAFETY: `section` is a NUL-terminated UTF-16 string that outlives
            // the call, `result` is a valid output location, and
            // SMTO_ABORTIFHUNG with a finite timeout bounds the wait for
            // windows that do not respond. The return value is ignored: a
            // window that does not answer does not make the change invalid.
            unsafe {
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    0,
                    section.as_ptr() as isize,
                    SMTO_ABORTIFHUNG,
                    TIMEOUT_MS,
                    &mut result,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_keys_and_their_relatives_are_detected() {
        let [user, machine] = environment_keys();
        assert!(affects_environment(&user));
        assert!(affects_environment(&machine));
        assert!(affects_environment(&user.child("x")));
        assert!(affects_environment(&KeyPath::root(Hive::CurrentUser)));
        assert!(affects_environment(&machine.prefix(2)));
        assert!(!affects_environment(&KeyPath::new(
            Hive::LocalMachine,
            vec!["SOFTWARE".to_owned()]
        )));
        assert!(!affects_environment(&KeyPath::new(
            Hive::Users,
            vec!["Environment".to_owned()]
        )));
    }
}
