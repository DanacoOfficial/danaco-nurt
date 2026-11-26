// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Redirect policy of smart HTTP, as applied by `git`.
//!
//! A redirect may change the scheme only from `http` to `https`, and when it
//! changes the authority, the new address must keep the request tail
//! (`info/refs?service=…`) so that the repository base can be derived.

use super::failure::{NetworkFailure, NetworkProblem};

/// Maximum number of redirects followed for one request; the default of curl.
pub(crate) const MAX_REDIRECTS: usize = 50;

fn parse(url: &str) -> Option<url::Url> {
    url::Url::parse(url).ok()
}

/// Whether a redirect from `original` to `redirect` keeps transport security.
pub(crate) fn scheme_is_safe(redirect: &str, original: &str) -> bool {
    let (Some(redirect), Some(original)) = (parse(redirect), parse(original)) else {
        return false;
    };
    match (original.scheme(), redirect.scheme()) {
        (from, to) if from == to => matches!(to, "http" | "https"),
        ("http", "https") => true,
        _ => false,
    }
}

/// Whether two addresses share scheme, host and port.
pub(crate) fn same_authority(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (parse(a), parse(b)) else {
        return false;
    };
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Repository base after a redirect of `url` (which is `base_url` followed by
/// a tail) to `redirect`.
pub(crate) fn base_url(
    redirect: &str,
    base_url: &str,
    url: &str,
) -> Result<String, NetworkFailure> {
    let refuse = || {
        NetworkFailure::new(
            NetworkProblem::Redirect,
            url,
            format!(
                "refusing redirect to '{}': the scheme is insecure or the request path changed",
                super::failure::redact_url(redirect)
            ),
        )
    };
    let tail = url.strip_prefix(base_url).ok_or_else(refuse)?;
    if !scheme_is_safe(redirect, base_url) {
        return Err(refuse());
    }
    redirect
        .strip_suffix(tail)
        .map(ToOwned::to_owned)
        .ok_or_else(refuse)
}

/// Replaces the `base_url` prefix of `url` with the effective base after a
/// redirect.
pub(crate) fn swap_tails(effective_base: Option<&str>, base_url: &str, url: &str) -> String {
    match (effective_base, url.strip_prefix(base_url)) {
        (Some(effective), Some(tail)) => format!("{effective}{tail}"),
        _ => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = "https://original/a/info/refs?service=git-upload-pack";

    #[test]
    fn derives_base_of_redirect_with_same_tail() {
        let base = base_url(
            "https://redirected.org/b/info/refs?service=git-upload-pack",
            "https://original/a",
            ORIGINAL,
        );
        assert_eq!(base.ok().as_deref(), Some("https://redirected.org/b"));
    }

    #[test]
    fn allows_upgrade_from_http_to_https() {
        let base = base_url(
            "https://original/b/info/refs?service=git-upload-pack",
            "http://original/a",
            "http://original/a/info/refs?service=git-upload-pack",
        );
        assert_eq!(base.ok().as_deref(), Some("https://original/b"));
    }

    #[test]
    fn rejects_downgrade_other_schemes_and_changed_tails() {
        for redirect in [
            "http://original/b/info/refs?service=git-upload-pack",
            "ftp://original/b/info/refs?service=git-upload-pack",
            "https://original/b/other",
        ] {
            let error = base_url(redirect, "https://original/a", ORIGINAL).err();
            assert_eq!(
                error.map(|e| e.problem()),
                Some(NetworkProblem::Redirect),
                "{redirect}"
            );
        }
    }

    #[test]
    fn judges_scheme_changes_of_a_redirect() {
        assert!(scheme_is_safe("https://b/x", "https://a/x"));
        assert!(scheme_is_safe("http://b/x", "http://a/x"));
        assert!(scheme_is_safe("https://a/x", "http://a/x"));
        assert!(!scheme_is_safe("http://a/x", "https://a/x"));
        assert!(!scheme_is_safe("ftp://a/x", "ftp://a/x"));
        assert!(!scheme_is_safe("not a url", "https://a/x"));
    }

    #[test]
    fn compares_authorities_with_default_ports() {
        assert!(same_authority("https://H:443/a", "https://h/b"));
        assert!(!same_authority("https://h:444/a", "https://h/a"));
        assert!(!same_authority("http://h/a", "https://h/a"));
        assert!(!same_authority("not a url", "https://h/a"));
    }

    #[test]
    fn swaps_base_only_after_a_redirect() {
        assert_eq!(
            swap_tails(None, "https://o/a", "https://o/a/x"),
            "https://o/a/x"
        );
        assert_eq!(
            swap_tails(Some("https://r/b"), "https://o/a", "https://o/a/x"),
            "https://r/b/x"
        );
    }
}
