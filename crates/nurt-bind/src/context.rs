// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Context of a binding call.

use jiff::tz::TimeZone;
use nurt_contracts::spec::Os;

/// Correction tables revision recorded when no correction was applied.
pub const UNCORRECTED_REV: &str = "none";

/// Facts outside the program that binding depends on.
///
/// Binding is a pure function of the surface tree, the catalog and this
/// context: the same three inputs always give the same bound tree.
///
/// # Examples
///
/// ```
/// use nurt_bind::BindContext;
/// use nurt_contracts::spec::Os;
///
/// let context = BindContext::new(Os::Linux)
///     .with_base("/srv/app")
///     .with_home("/home/user")
///     .with_let("recent");
/// assert_eq!(context.base.as_deref(), Some("/srv/app"));
/// assert_eq!(context.lets, ["recent"]);
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BindContext {
    /// Operating system of the host; paths and commands foreign to it are
    /// rejected.
    pub host_os: Os,
    /// Call parameter `base`: the directory against which relative paths are
    /// resolved. Without it a relative path is an error.
    pub base: Option<String>,
    /// Home directory that `~` stands for.
    pub home: Option<String>,
    /// Names of results bound by earlier calls of the session, referenced as
    /// `@name`.
    pub lets: Vec<String>,
    /// Local time rules that convert an instant written without an offset
    /// to UTC.
    pub local_tz: TimeZone,
    /// Revision of the correction tables applied before binding, recorded as
    /// `corr_rev`.
    pub corr_rev: String,
}

impl BindContext {
    /// Creates a context for the given host without `base`, home directory
    /// and earlier results, with UTC local time and without corrections.
    #[must_use]
    pub fn new(host_os: Os) -> Self {
        Self {
            host_os,
            base: None,
            home: None,
            lets: Vec::new(),
            local_tz: TimeZone::UTC,
            corr_rev: UNCORRECTED_REV.to_owned(),
        }
    }

    /// Creates a context for the operating system the library runs on.
    #[must_use]
    pub fn for_host() -> Self {
        Self::new(host_os())
    }

    /// Sets the call parameter `base`.
    #[must_use]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }

    /// Sets the home directory.
    #[must_use]
    pub fn with_home(mut self, home: impl Into<String>) -> Self {
        self.home = Some(home.into());
        self
    }

    /// Adds the name of a result bound by an earlier call.
    #[must_use]
    pub fn with_let(mut self, name: impl Into<String>) -> Self {
        self.lets.push(name.into());
        self
    }

    /// Sets the local time rules of instants written without an offset.
    #[must_use]
    pub fn with_local_tz(mut self, local_tz: TimeZone) -> Self {
        self.local_tz = local_tz;
        self
    }

    /// Sets the revision of the correction tables applied before binding.
    #[must_use]
    pub fn with_corr_rev(mut self, corr_rev: impl Into<String>) -> Self {
        self.corr_rev = corr_rev.into();
        self
    }
}

/// Returns the operating system family the library was built for.
#[must_use]
pub fn host_os() -> Os {
    if cfg!(target_os = "windows") {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Macos
    } else {
        Os::Linux
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_without_base_and_corrections() {
        let context = BindContext::new(Os::Windows);
        assert_eq!(context.host_os, Os::Windows);
        assert!(context.base.is_none());
        assert!(context.home.is_none());
        assert_eq!(context.corr_rev, UNCORRECTED_REV);
    }

    #[test]
    fn records_the_correction_revision() {
        let context = BindContext::for_host().with_corr_rev("sha256:00");
        assert_eq!(context.corr_rev, "sha256:00");
        assert_eq!(context.host_os, host_os());
    }
}
