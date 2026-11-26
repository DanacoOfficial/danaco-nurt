// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Matching of configuration subsections such as `http.<url>.*` and
//! `credential.<url>.*` against a remote address, with the rules of `git`:
//! the scheme must be equal, host labels may be `*`, the port defaults by
//! scheme, the path matches on whole components, and a user name in the
//! pattern must equal the user of the address. The longest path wins, then a
//! pattern with a user name.

use bstr::ByteSlice;

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
    path_length: usize,
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
            Some((host, port)) if !port.contains(']') => {
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
        if self.scheme != url.scheme
            || self.port != url.port
            || !host_matches(&self.host, &url.host)
        {
            return None;
        }
        if let Some(user) = &self.user
            && url.user.as_ref() != Some(user)
        {
            return None;
        }
        let path_length = path_match(&self.path, &url.path)?;
        Some(Specificity {
            path_length,
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

/// Whether `pattern` matches `url` at all.
pub(crate) fn matches(pattern: &str, url: &str) -> bool {
    match (UrlParts::parse(pattern), UrlParts::parse(url)) {
        (Some(pattern), Some(url)) => pattern.matches(&url).is_some(),
        _ => false,
    }
}

/// Value of `<section>.<key>` from the most specific section applying to
/// `url`, where `<section>.<url>.<key>` is more specific than `<section>.<key>`;
/// later sections win ties.
pub(crate) fn best_value(
    config: &gix::config::File,
    section: &str,
    key: &str,
    url: &str,
) -> Option<String> {
    let target = UrlParts::parse(url);
    let mut best: Option<(Option<Specificity>, String)> = None;
    for candidate in config.sections_by_name(section).into_iter().flatten() {
        let score = match candidate.header().subsection_name() {
            None => None,
            Some(pattern) => {
                let pattern = UrlParts::parse(&pattern.to_str_lossy());
                match pattern.zip(target.as_ref()).and_then(|(p, u)| p.matches(u)) {
                    Some(score) => Some(score),
                    None => continue,
                }
            }
        };
        if let Some(value) = candidate.value(key)
            && best.as_ref().is_none_or(|(current, _)| score >= *current)
        {
            best = Some((score, value.to_str_lossy().into_owned()));
        }
    }
    best.map(|(_, value)| value)
}

/// All values of `<section>.<key>` from sections applying to `url`, in
/// configuration order; an empty value clears the values before it.
pub(crate) fn all_values(
    config: &gix::config::File,
    section: &str,
    key: &str,
    url: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for candidate in config.sections_by_name(section).into_iter().flatten() {
        if let Some(pattern) = candidate.header().subsection_name()
            && !matches(&pattern.to_str_lossy(), url)
        {
            continue;
        }
        for value in candidate.values(key) {
            if value.is_empty() {
                out.clear();
            } else {
                out.push(value.to_str_lossy().into_owned());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> gix::config::File {
        text.parse().expect("valid configuration")
    }

    #[test]
    fn matches_scheme_host_port_and_path_components() {
        assert!(matches(
            "https://example.com",
            "https://example.com/org/repo.git"
        ));
        assert!(matches(
            "https://example.com:443/org",
            "https://example.com/org/repo.git"
        ));
        assert!(!matches(
            "https://example.com/or",
            "https://example.com/org/repo.git"
        ));
        assert!(!matches("http://example.com", "https://example.com/r"));
        assert!(!matches(
            "https://example.com:8443",
            "https://example.com/r"
        ));
    }

    #[test]
    fn distinguishes_ipv6_hosts_with_and_without_a_port() {
        assert!(matches("https://[2001:db8::1]", "https://[2001:db8::1]/r"));
        assert!(!matches("https://[2001:db8::1]", "https://[2001:db8::2]/r"));
        assert!(matches(
            "https://[2001:db8::1]:8443",
            "https://[2001:db8::1]:8443/r"
        ));
        assert!(!matches(
            "https://[2001:db8::1]:8443",
            "https://[2001:db8::1]/r"
        ));
    }

    #[test]
    fn matches_wildcard_host_labels() {
        assert!(matches(
            "https://*.example.com",
            "https://git.example.com/r"
        ));
        assert!(!matches("https://*.example.com", "https://example.com/r"));
        assert!(matches(
            "https://g*.example.com",
            "https://git.example.com/r"
        ));
    }

    #[test]
    fn requires_equal_user_when_the_pattern_has_one() {
        assert!(matches(
            "https://bob@example.com",
            "https://bob:pw@example.com/r"
        ));
        assert!(!matches(
            "https://bob@example.com",
            "https://alice@example.com/r"
        ));
        assert!(matches(
            "https://example.com",
            "https://alice@example.com/r"
        ));
    }

    #[test]
    fn prefers_longer_path_then_user_then_later_section() {
        let url = "https://bob@example.com/org/repo";
        let longer = config(
            "[s \"https://example.com/org\"]\n\tk = path\n[s \"https://example.com\"]\n\tk = host\n",
        );
        assert_eq!(best_value(&longer, "s", "k", url).as_deref(), Some("path"));
        let user = config(
            "[s \"https://bob@example.com\"]\n\tk = user\n[s \"https://example.com\"]\n\tk = host\n",
        );
        assert_eq!(best_value(&user, "s", "k", url).as_deref(), Some("user"));
        let later = config(
            "[s \"https://example.com\"]\n\tk = first\n[s \"https://example.com/\"]\n\tk = second\n",
        );
        assert_eq!(best_value(&later, "s", "k", url).as_deref(), Some("second"));
        let plain = config("[s]\n\tk = plain\n[s \"https://other.com\"]\n\tk = other\n");
        assert_eq!(best_value(&plain, "s", "k", url).as_deref(), Some("plain"));
        assert_eq!(best_value(&plain, "s", "missing", url), None);
    }

    #[test]
    fn collects_values_of_matching_sections_in_order() {
        let text = "[s]\n\tk = a\n[s \"https://other.com\"]\n\tk = b\n[s \"https://example.com\"]\n\tk = c\n";
        assert_eq!(
            all_values(&config(text), "s", "k", "https://example.com/r"),
            ["a", "c"]
        );
    }
}
