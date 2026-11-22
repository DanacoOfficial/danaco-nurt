// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Matching of configuration subsections such as `http.<url>.*` and
//! `credential.<url>.*` against a remote address, with the rules of `git`:
//! the scheme must be equal, host labels may be `*`, the port defaults by
//! scheme, the path matches on whole components, and a user name in the
//! pattern must equal the user of the address. The longest path wins, then a
//! pattern with a user name.

/// Parts of an address used for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UrlParts {
    scheme: String,
    user: Option<String>,
    host: String,
    port: Option<u16>,
    path: String,
}

/// Quality of a match; a greater value is more specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Specificity {
    path_len: usize,
    has_user: bool,
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        "ssh" => Some(22),
        "git" => Some(9418),
        _ => None,
    }
}

impl UrlParts {
    /// Splits `url`; `None` for addresses without `scheme://`.
    pub(crate) fn parse(url: &str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let (authority, path) = rest.split_at(authority_end);
        let (user, host_port) = match authority.rsplit_once('@') {
            Some((userinfo, host)) => {
                let user = userinfo.split(':').next().unwrap_or_default();
                let user = percent_encoding::percent_decode_str(user)
                    .decode_utf8_lossy()
                    .into_owned();
                (Some(user), host)
            }
            None => (None, authority),
        };
        let (host, port) = match host_port.rsplit_once(':') {
            Some((host, port)) if !host.ends_with(']') || host.starts_with('[') => {
                (host, port.parse::<u16>().ok())
            }
            _ => (host_port, None),
        };
        let port = port.or_else(|| default_port(&scheme));
        Some(Self {
            scheme,
            user,
            host: host.to_ascii_lowercase(),
            port,
            path: path.to_owned(),
        })
    }

    /// How well the pattern `self` matches `url`; `None` when it does not.
    pub(crate) fn matches(&self, url: &Self) -> Option<Specificity> {
        if self.scheme != url.scheme || self.port != url.port || !host_matches(&self.host, &url.host) {
            return None;
        }
        if let Some(user) = &self.user {
            if url.user.as_ref() != Some(user) {
                return None;
            }
        }
        let path_len = path_match(&self.path, &url.path)?;
        Some(Specificity {
            path_len,
            has_user: self.user.is_some(),
        })
    }
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('.').collect();
    let host: Vec<&str> = host.split('.').collect();
    pattern.len() == host.len()
        && pattern
            .iter()
            .zip(&host)
            .all(|(p, h)| label_matches(p.as_bytes(), h.as_bytes()))
}

fn label_matches(pattern: &[u8], label: &[u8]) -> bool {
    match pattern.split_first() {
        None => label.is_empty(),
        Some((b'*', rest)) => (0..=label.len()).any(|skip| label_matches(rest, &label[skip..])),
        Some((first, rest)) => label
            .split_first()
            .is_some_and(|(l, label_rest)| l == first && label_matches(rest, label_rest)),
    }
}

fn path_match(pattern: &str, path: &str) -> Option<usize> {
    let pattern = pattern.trim_end_matches('/');
    if pattern.is_empty() {
        return Some(0);
    }
    let rest = path.strip_prefix(pattern)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(pattern.len())
}

/// Selects the most specific of `patterns` matching `url`, returning its
/// position; later patterns win ties, as later configuration entries do.
pub(crate) fn best_match<'a>(patterns: impl IntoIterator<Item = &'a str>, url: &str) -> Option<usize> {
    let url = UrlParts::parse(url)?;
    let mut best: Option<(Specificity, usize)> = None;
    for (index, pattern) in patterns.into_iter().enumerate() {
        let Some(score) = UrlParts::parse(pattern).and_then(|p| p.matches(&url)) else {
            continue;
        };
        if best.is_none_or(|(current, _)| score >= current) {
            best = Some((score, index));
        }
    }
    best.map(|(_, index)| index)
}

/// Whether `pattern` matches `url` at all.
pub(crate) fn matches(pattern: &str, url: &str) -> bool {
    match (UrlParts::parse(pattern), UrlParts::parse(url)) {
        (Some(pattern), Some(url)) => pattern.matches(&url).is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_scheme_host_port_and_path_components() {
        assert!(matches("https://example.com", "https://example.com/org/repo.git"));
        assert!(matches("https://example.com:443/org", "https://example.com/org/repo.git"));
        assert!(!matches("https://example.com/or", "https://example.com/org/repo.git"));
        assert!(!matches("http://example.com", "https://example.com/r"));
        assert!(!matches("https://example.com:8443", "https://example.com/r"));
    }

    #[test]
    fn matches_wildcard_host_labels() {
        assert!(matches("https://*.example.com", "https://git.example.com/r"));
        assert!(!matches("https://*.example.com", "https://example.com/r"));
        assert!(matches("https://g*.example.com", "https://git.example.com/r"));
    }

    #[test]
    fn requires_equal_user_when_the_pattern_has_one() {
        assert!(matches("https://bob@example.com", "https://bob:pw@example.com/r"));
        assert!(!matches("https://bob@example.com", "https://alice@example.com/r"));
        assert!(matches("https://example.com", "https://alice@example.com/r"));
    }

    #[test]
    fn prefers_longer_path_then_user_then_later_entry() {
        let url = "https://bob@example.com/org/repo";
        assert_eq!(best_match(["https://example.com/org", "https://example.com"], url), Some(0));
        assert_eq!(best_match(["https://bob@example.com", "https://example.com"], url), Some(0));
        assert_eq!(best_match(["https://example.com", "https://example.com/"], url), Some(1));
        assert_eq!(best_match(["https://other.com"], url), None);
        assert_eq!(best_match(["https://example.com"], "/local/path"), None);
    }
}
