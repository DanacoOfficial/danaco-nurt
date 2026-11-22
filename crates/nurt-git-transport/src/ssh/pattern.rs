// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Host patterns of `ssh_config` and `known_hosts`.
//!
//! A pattern matches the whole text; `*` matches any sequence of characters
//! and `?` exactly one. A list of patterns matches when at least one plain
//! pattern matches and no negated pattern (prefix `!`) does, as in OpenSSH.

/// Whether `text` matches the wildcard `pattern`, ignoring ASCII case.
pub(crate) fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some(b'*') => {
                backtrack = Some((p, t));
                p += 1;
            }
            Some(&c) if c == b'?' || c.eq_ignore_ascii_case(&text[t]) => {
                p += 1;
                t += 1;
            }
            _ => match backtrack {
                Some((star, consumed)) => {
                    p = star + 1;
                    t = consumed + 1;
                    backtrack = Some((star, consumed + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

/// Whether `text` matches a list of patterns with optional negations.
pub(crate) fn match_list<'a>(text: &str, patterns: impl IntoIterator<Item = &'a str>) -> bool {
    let mut matched = false;
    for pattern in patterns {
        match pattern.strip_prefix('!') {
            Some(negated) if wildcard_match(negated, text) => return false,
            Some(_) => {}
            None => matched |= wildcard_match(pattern, text),
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_matches_any_sequence() {
        assert!(wildcard_match("*.example.com", "git.example.com"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("a*b*c", "a-b-b-c"));
        assert!(!wildcard_match("*.example.com", "example.org"));
    }

    #[test]
    fn question_mark_matches_one_character() {
        assert!(wildcard_match("host?", "host1"));
        assert!(!wildcard_match("host?", "host"));
        assert!(!wildcard_match("host?", "host12"));
    }

    #[test]
    fn matching_ignores_ascii_case() {
        assert!(wildcard_match("GitHub.com", "github.COM"));
    }

    #[test]
    fn bracketed_port_form_is_literal() {
        assert!(wildcard_match("[git.example.com]:2222", "[git.example.com]:2222"));
        assert!(wildcard_match("[*.example.com]:*", "[git.example.com]:2222"));
    }

    #[test]
    fn negated_pattern_excludes_a_match() {
        assert!(match_list("a.example.com", ["*.example.com"]));
        assert!(!match_list("a.example.com", ["*.example.com", "!a.example.com"]));
        assert!(!match_list("b.example.org", ["!a.example.com"]));
    }
}
