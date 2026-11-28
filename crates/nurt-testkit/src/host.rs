// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-testkit::host
// Responsibility:   Host double with fixed facts and a private environment.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Host double with fixed facts and a private environment.
//!
//! [`TestHost`] implements [`Host`] without reading the operating system.
//! Its facts default to a Linux host with the account `user`
//! ([`TestHost::linux`]); [`TestHost::windows`] and [`TestHost::macos`] give
//! the other families. Environment variables are kept by the double and can
//! be changed while tests run.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};

/// Host facts and environment for tests.
///
/// Variable names are compared as the simulated host compares them:
/// case-sensitively for Linux and macOS, ASCII case-insensitively for
/// Windows. [`Host::env_vars`] returns variables in insertion order.
///
/// ```
/// use nurt_contracts::env::{Host, Os};
/// use nurt_testkit::host::TestHost;
///
/// let host = TestHost::windows().with_var("Path", "C:/Windows");
/// assert_eq!(host.info().os, Os::Windows);
/// assert_eq!(host.env_var("PATH").as_deref(), Some("C:/Windows".as_ref()));
/// ```
#[derive(Debug)]
pub struct TestHost {
    info: HostInfo,
    vars: Mutex<Vec<(OsString, OsString)>>,
}

impl TestHost {
    /// Host with the given facts and an empty environment.
    #[must_use]
    pub fn new(info: HostInfo) -> Self {
        Self {
            info,
            vars: Mutex::new(Vec::new()),
        }
    }

    /// Linux host `testhost`, account `user`, home `/home/user`, temporary
    /// directory `/tmp`, local time rules `UTC`, locale `en_US.UTF-8`, 4
    /// processors.
    #[must_use]
    pub fn linux() -> Self {
        Self::new(HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "testhost".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from("/home/user"),
            temp: PathBuf::from("/tmp"),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: Some("en_US.UTF-8".to_owned()),
            cpus: 4,
        })
    }

    /// Host running macOS with the same account as [`TestHost::linux`] and home
    /// `/Users/user`.
    #[must_use]
    pub fn macos() -> Self {
        Self::new(HostInfo {
            os: Os::Macos,
            os_version: "15.0".to_owned(),
            architecture: "aarch64".to_owned(),
            hostname: "testhost".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from("/Users/user"),
            temp: PathBuf::from("/tmp"),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: Some("en_US.UTF-8".to_owned()),
            cpus: 4,
        })
    }

    /// Windows host with home `C:/Users/user` and temporary directory
    /// `C:/Users/user/AppData/Local/Temp`.
    #[must_use]
    pub fn windows() -> Self {
        Self::new(HostInfo {
            os: Os::Windows,
            os_version: "10.0.26100".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "testhost".to_owned(),
            user: "user".to_owned(),
            home: PathBuf::from("C:/Users/user"),
            temp: PathBuf::from("C:/Users/user/AppData/Local/Temp"),
            path_style: PathStyle::Windows,
            timezone: "UTC".to_owned(),
            locale: Some("en_US.UTF-8".to_owned()),
            cpus: 4,
        })
    }

    /// Host of the family the tests run on, with the facts of
    /// [`TestHost::linux`], [`TestHost::macos`] or [`TestHost::windows`].
    #[must_use]
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::windows()
        } else if cfg!(target_os = "macos") {
            Self::macos()
        } else {
            Self::linux()
        }
    }

    /// Adds or replaces a variable and returns the host.
    #[must_use]
    pub fn with_var(self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.set_var(name, value);
        self
    }

    /// Adds or replaces a variable. A replaced variable keeps its position
    /// and its original spelling of the name.
    pub fn set_var(&self, name: impl Into<OsString>, value: impl Into<OsString>) {
        let name = name.into();
        let value = value.into();
        let mut vars = self.lock();
        match vars
            .iter_mut()
            .find(|(known, _)| self.same_name(known, &name))
        {
            Some(slot) => slot.1 = value,
            None => vars.push((name, value)),
        }
    }

    /// Removes a variable; does nothing when it is not set.
    pub fn remove_var(&self, name: impl AsRef<OsStr>) {
        let name = name.as_ref();
        self.lock()
            .retain(|(known, _)| !self.same_name(known, name));
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(OsString, OsString)>> {
        self.vars.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn same_name(&self, left: &OsStr, right: &OsStr) -> bool {
        if self.info.os == Os::Windows {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    }
}

impl Default for TestHost {
    fn default() -> Self {
        Self::native()
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        let name = OsStr::new(name);
        self.lock()
            .iter()
            .find(|(known, _)| self.same_name(known, name))
            .map(|(_, value)| value.clone())
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use nurt_contracts::env::{Host, Os, PathStyle};

    use super::TestHost;

    #[test]
    fn linux_names_are_case_sensitive() {
        let host = TestHost::linux().with_var("HOME", "/home/user");
        assert_eq!(host.env_var("HOME"), Some(OsString::from("/home/user")));
        assert_eq!(host.env_var("home"), None);
        assert_eq!(host.info().path_style, PathStyle::Posix);
    }

    #[test]
    fn windows_names_ignore_case() {
        let host = TestHost::windows().with_var("Path", "a");
        host.set_var("PATH", "b");
        assert_eq!(host.info().os, Os::Windows);
        assert_eq!(
            host.env_vars(),
            vec![(OsString::from("Path"), OsString::from("b"))]
        );
        host.remove_var("path");
        assert!(host.env_vars().is_empty());
    }

    #[test]
    fn variables_keep_insertion_order() {
        let host = TestHost::macos()
            .with_var("B", "2")
            .with_var("A", "1")
            .with_var("B", "3");
        let names: Vec<OsString> = host.env_vars().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, vec![OsString::from("B"), OsString::from("A")]);
        assert_eq!(host.env_var("B"), Some(OsString::from("3")));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
