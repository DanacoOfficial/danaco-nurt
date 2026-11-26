// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Command-line patterns: words with wildcards and alternatives, positional
//! tokens and option tokens.

use super::argv::{Invocation, is_option};

/// Alternatives separated by `|`; `*` in a word matches any text. Letter
/// case is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Words(Vec<String>);

impl Words {
    /// Parses `a|b*`.
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let words: Vec<String> = text.split('|').map(str::to_ascii_lowercase).collect();
        if words.iter().any(String::is_empty) {
            return Err(format!("`{text}` has an empty alternative"));
        }
        Ok(Self(words))
    }

    /// Whether `text` matches one of the alternatives.
    pub(super) fn matches(&self, text: &str) -> bool {
        let text = text.to_ascii_lowercase();
        self.0
            .iter()
            .any(|word| wildcard(word.as_bytes(), text.as_bytes()))
    }
}

/// Matches `text` against `pattern`, where `*` stands for any sequence of
/// bytes. Backtracks only to the last `*`, which is sufficient for this
/// pattern language.
fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut resume: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some(b'*') => {
                resume = Some((p, t));
                p += 1;
            }
            Some(&byte) if text.get(t) == Some(&byte) => {
                p += 1;
                t += 1;
            }
            _ => match resume {
                Some((star, start)) => {
                    p = star + 1;
                    t = start + 1;
                    resume = Some((star, start + 1));
                }
                None => return false,
            },
        }
    }
    pattern
        .get(p..)
        .is_some_and(|rest| rest.iter().all(|&byte| byte == b'*'))
}

/// One name of an option token.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OptionName {
    /// `-x`: also found within a group of short options (`-xvf`); letter case
    /// is significant.
    Short(char),
    /// `--name`, `-name`: compared with the option name before `=`.
    Word(Words),
}

/// Occurrence of an option in the arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OptionHit {
    /// Index of the option in the arguments.
    pub(super) index: usize,
    /// Value attached to the option or the next argument.
    pub(super) value: Option<String>,
}

/// Option that must be present, optionally with a value: `-X|--request=POST`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OptionToken {
    names: Vec<OptionName>,
    value: Option<Words>,
}

impl OptionToken {
    /// Parses `-a|--bee=V1|V2`.
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let (names, value) = match text.split_once('=') {
            Some((names, value)) => (names, Some(Words::parse(value)?)),
            None => (text, None),
        };
        let names = names
            .split('|')
            .map(option_name)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { names, value })
    }

    /// First occurrence of the option before `--` with a matching value.
    ///
    /// With `exact`, names are compared as whole words and single letters are
    /// not looked up within groups of short options.
    pub(super) fn find(&self, args: &[String], exact: bool) -> Option<OptionHit> {
        args.iter()
            .take_while(|arg| arg.as_str() != "--")
            .enumerate()
            .filter(|(_, arg)| is_option(arg))
            .filter_map(|(index, _)| self.hit(args, index, exact))
            .find(|hit| self.value_matches(hit))
    }

    fn value_matches(&self, hit: &OptionHit) -> bool {
        match &self.value {
            None => true,
            Some(words) => hit
                .value
                .as_deref()
                .is_some_and(|value| words.matches(value)),
        }
    }

    fn hit(&self, args: &[String], index: usize, exact: bool) -> Option<OptionHit> {
        let arg = args.get(index)?;
        let next = || args.get(index + 1).cloned();
        self.names.iter().find_map(|name| {
            let value = match name {
                OptionName::Short(letter) if !exact => short_in_group(arg, *letter)?.or_else(next),
                OptionName::Short(letter) => {
                    let mut short = String::from('-');
                    short.push(*letter);
                    if !arg.eq_ignore_ascii_case(&short) {
                        return None;
                    }
                    next()
                }
                OptionName::Word(words) => {
                    let (name, attached) = match arg.split_once('=') {
                        Some((name, attached)) => (name, Some(attached.to_owned())),
                        None => (arg.as_str(), None),
                    };
                    if !words.matches(name) {
                        return None;
                    }
                    attached.or_else(next)
                }
            };
            Some(OptionHit { index, value })
        })
    }
}

fn option_name(name: &str) -> Result<OptionName, String> {
    let mut chars = name.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some('-'), Some(letter), None) if letter != '-' => Ok(OptionName::Short(letter)),
        (Some('-'), Some(_), _) => Ok(OptionName::Word(Words::parse(name)?)),
        _ => Err(format!("option `{name}` must start with `-`")),
    }
}

/// Looks up `letter` among the leading letters of a group of short options.
///
/// Returns `None` when the letter is absent, `Some(None)` when it ends the
/// group (its value is the next argument) and `Some(Some(text))` with the
/// text attached after it (`-n10`, `-XPOST`).
fn short_in_group(arg: &str, letter: char) -> Option<Option<String>> {
    let group = arg.strip_prefix('-')?;
    if group.starts_with('-') {
        return None;
    }
    let letters = group
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(group.len());
    let position = group.get(..letters)?.find(letter)?;
    let attached = group
        .get(position + letter.len_utf8()..)
        .unwrap_or_default();
    Some((!attached.is_empty()).then(|| attached.to_owned()))
}

/// Positional token of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Positional {
    /// One positional argument.
    Words(Words),
    /// `**`: any number of positional arguments.
    Gap,
}

/// Pattern tokens after the program: positional tokens matched from the
/// first positional argument on, and options required anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Tail {
    positionals: Vec<Positional>,
    options: Vec<OptionToken>,
}

impl Tail {
    /// Parses tokens separated by white space.
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let mut tail = Self::default();
        for token in text.split_whitespace() {
            if token == "**" {
                tail.positionals.push(Positional::Gap);
            } else if token.starts_with('-') {
                tail.options.push(OptionToken::parse(token)?);
            } else {
                tail.positionals
                    .push(Positional::Words(Words::parse(token)?));
            }
        }
        Ok(tail)
    }

    /// Whether the invocation matches; returns the index in the arguments
    /// after the last positional argument consumed by the pattern.
    pub(super) fn matches(&self, invocation: &Invocation<'_>, exact: bool) -> Option<usize> {
        let consumed = prefix(&self.positionals, &invocation.positionals)?;
        if !self
            .options
            .iter()
            .all(|option| option.find(invocation.args, exact).is_some())
        {
            return None;
        }
        Some(match consumed.checked_sub(1) {
            Some(last) => invocation
                .positionals
                .get(last)
                .map_or(0, |(index, _)| index + 1),
            None => 0,
        })
    }
}

/// Number of positional arguments consumed when `tokens` match a prefix of
/// `positionals`.
fn prefix(tokens: &[Positional], positionals: &[(usize, &str)]) -> Option<usize> {
    let Some((token, rest)) = tokens.split_first() else {
        return Some(0);
    };
    match token {
        Positional::Gap => (0..=positionals.len()).find_map(|skip| {
            let remaining = positionals.get(skip..)?;
            prefix(rest, remaining).map(|count| skip + count)
        }),
        Positional::Words(words) => {
            let ((_, first), remaining) = positionals.split_first()?;
            if !words.matches(first) {
                return None;
            }
            prefix(rest, remaining).map(|count| count + 1)
        }
    }
}

/// Program and tail of a rule pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pattern {
    /// Program names.
    pub(super) program: Words,
    /// Remaining tokens.
    pub(super) tail: Tail,
}

impl Pattern {
    /// Parses `program tokens…`.
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let trimmed = text.trim();
        let (program, rest) = trimmed
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed, ""));
        if program.is_empty() {
            return Err("the pattern is empty".to_owned());
        }
        if program.starts_with('-') || program == "**" {
            return Err(format!("pattern `{text}` must start with a program name"));
        }
        Ok(Self {
            program: Words::parse(program)?,
            tail: Tail::parse(rest)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::argv::invocations;
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// End index of the positional tokens of `pattern` in `argv`, `None`
    /// when the command line does not match.
    fn matched(pattern: &str, argv: &[&str], exact: bool) -> Option<usize> {
        let argv = strings(argv);
        let chain = invocations(&argv, &[], &[]);
        let invocation = chain.first()?;
        let pattern = Pattern::parse(pattern).unwrap();
        if !pattern.program.matches(&invocation.program) {
            return None;
        }
        pattern.tail.matches(invocation, exact)
    }

    #[test]
    fn words_ignore_letter_case_and_reject_empty_alternatives() {
        let words = Words::parse("Git|HG").unwrap();
        assert!(words.matches("git") && words.matches("hg") && words.matches("GIT"));
        assert!(!words.matches("svn"));
        assert!(Words::parse("a||b").is_err());
        assert!(Words::parse("").is_err());
        assert!(Words::parse("a|").is_err());
    }

    #[test]
    fn a_star_matches_any_text_within_a_word() {
        let words = Words::parse("python3.*").unwrap();
        assert!(words.matches("python3.12") && words.matches("python3."));
        assert!(!words.matches("python3") && !words.matches("python"));
        assert!(Words::parse("*").unwrap().matches(""));
        assert!(Words::parse("p*n").unwrap().matches("python"));
        assert!(!Words::parse("p*n").unwrap().matches("pyth"));
    }

    #[test]
    fn a_star_backtracks_to_the_last_star() {
        assert!(wildcard(b"a*b*c", b"aXbXbXc"));
        assert!(wildcard(b"*ab", b"aab"));
        assert!(wildcard(b"a**b", b"ab"));
        assert!(!wildcard(b"*ab", b"aba"));
        assert!(!wildcard(b"a*b", b"a"));
        assert!(wildcard(b"", b""));
        assert!(!wildcard(b"", b"x"));
    }

    #[test]
    fn multi_byte_text_is_compared_by_bytes() {
        let words = Words::parse("za*ć").unwrap();
        assert!(words.matches("zażółć"));
        assert!(!words.matches("zażółd"));
    }

    #[test]
    fn short_options_are_found_within_groups_and_with_attached_values() {
        let token = OptionToken::parse("-n=10").unwrap();
        for found in [&["-n", "10"][..], &["-n10"], &["-vn", "10"], &["-vn10"]] {
            assert!(token.find(&strings(found), false).is_some(), "{found:?}");
        }
        for missing in [&["-n", "11"][..], &["-m", "10"], &["--n", "10"], &["n", "10"]] {
            assert!(token.find(&strings(missing), false).is_none(), "{missing:?}");
        }
    }

    #[test]
    fn exact_matching_ignores_groups_and_letter_case() {
        let token = OptionToken::parse("-c").unwrap();
        assert!(token.find(&strings(&["-lc", "x"]), false).is_some());
        assert!(token.find(&strings(&["-lc", "x"]), true).is_none());
        assert!(token.find(&strings(&["-C", "x"]), false).is_none());
        assert!(token.find(&strings(&["-C", "x"]), true).is_some());
    }

    #[test]
    fn long_options_take_their_value_attached_or_from_the_next_argument() {
        let token = OptionToken::parse("-X|--request=POST|PUT").unwrap();
        for found in [
            &["-X", "post"][..],
            &["-XPUT"],
            &["--request=POST"],
            &["--request", "put"],
        ] {
            assert!(token.find(&strings(found), false).is_some(), "{found:?}");
        }
        for missing in [&["--request=GET"][..], &["--request"], &["-X"], &["--other=POST"]] {
            assert!(token.find(&strings(missing), false).is_none(), "{missing:?}");
        }
    }

    #[test]
    fn options_after_the_end_of_options_are_not_options() {
        let token = OptionToken::parse("-f").unwrap();
        assert!(token.find(&strings(&["--", "-f"]), false).is_none());
        assert!(token.find(&strings(&["x", "-f"]), false).is_some());
        assert_eq!(token.find(&strings(&["x", "-f"]), false).map(|hit| hit.index), Some(1));
    }

    #[test]
    fn malformed_option_tokens_are_refused() {
        assert!(OptionToken::parse("x").is_err());
        assert!(OptionToken::parse("-a|b").is_err());
        assert!(OptionToken::parse("-a=").is_err());
        assert!(OptionToken::parse("-a=|b").is_err());
    }

    #[test]
    fn positional_tokens_match_from_the_first_positional_argument() {
        assert_eq!(matched("git push", &["git", "push", "origin"], false), Some(1));
        assert_eq!(matched("git push", &["git", "--no-pager", "push"], false), Some(2));
        assert_eq!(matched("git push", &["git", "-C", "x", "push"], false), None);
        assert_eq!(matched("git push", &["git", "status"], false), None);
        assert_eq!(matched("git push", &["git"], false), None);
        assert_eq!(matched("git push", &["git", "origin", "push"], false), None);
        assert_eq!(matched("git", &["git", "anything"], false), Some(0));
    }

    #[test]
    fn alternatives_and_gaps_consume_positional_arguments() {
        assert_eq!(matched("npm publish|pack", &["npm", "PACK"], false), Some(1));
        assert_eq!(
            matched("cmd ** /c|/k", &["cmd", "a", "b", "/C", "dir"], false),
            Some(3)
        );
        assert_eq!(matched("cmd ** /c", &["cmd", "/c"], false), Some(1));
        assert_eq!(matched("cmd ** /c", &["cmd", "a"], false), None);
        assert_eq!(matched("ssh *", &["ssh", "host", "ls"], false), Some(1));
    }

    #[test]
    fn options_of_a_pattern_are_required_anywhere() {
        assert_eq!(matched("rm -r", &["rm", "-rf", "x"], false), Some(0));
        assert_eq!(matched("rm -r", &["rm", "x", "-r"], false), Some(0));
        assert_eq!(matched("rm -r", &["rm", "x"], false), None);
        assert_eq!(matched("rm -r|--recursive", &["rm", "--recursive"], false), Some(0));
        assert_eq!(matched("rm -r -f", &["rm", "-r"], false), None);
    }

    #[test]
    fn patterns_start_with_a_program_name() {
        assert!(Pattern::parse("").is_err());
        assert!(Pattern::parse("   ").is_err());
        assert!(Pattern::parse("-x").is_err());
        assert!(Pattern::parse("** x").is_err());
        assert!(Pattern::parse("git a||b").is_err());
        let parsed = Pattern::parse("  git   push  ").unwrap();
        assert!(parsed.program.matches("git"));
    }
}
