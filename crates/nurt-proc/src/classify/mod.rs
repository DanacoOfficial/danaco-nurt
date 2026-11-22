// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classification of command lines by their effect outside the file system
//! and by the way the started program behaves.
//!
//! The functions are pure: they inspect the arguments only and never start a
//! program or read the file system. Programs started through wrappers
//! (`sudo`, `env`, `nice`, `timeout`, `npx`, `pnpm exec`, `python -m`) are
//! classified as well.

mod argv;
mod pattern;
mod table;

use std::sync::OnceLock;

use nurt_contracts::error::{Error, ErrorCode, Result};

use self::argv::{Invocation, invocations};
use self::table::{Class, Rule, Tables};

/// Rule of the effect table that matched a command line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectMatch {
    /// Name of the rule, for example `git push`; the pattern text for rules of
    /// `run.irreversible_patterns`.
    pub rule: String,
    /// Program that has the effect, after wrappers were removed.
    pub program: String,
}

/// Classifier of external effects: the built-in table and the patterns of
/// `run.irreversible_patterns`.
///
/// # Examples
///
/// ```
/// use nurt_proc::EffectClassifier;
///
/// let classifier = EffectClassifier::with_patterns(&["deploy-tool publish"]).unwrap();
/// let matches = classifier.classify(&["deploy-tool", "publish", "--force"]);
/// assert_eq!(matches[0].rule, "deploy-tool publish");
/// assert!(classifier.classify(&["deploy-tool", "status"]).is_empty());
/// ```
#[derive(Debug, Clone, Default)]
pub struct EffectClassifier {
    extra: Vec<Rule>,
}

impl EffectClassifier {
    /// Classifier with the built-in table only.
    #[must_use]
    pub fn builtin() -> Self {
        Self::default()
    }

    /// Classifier with the built-in table and additional patterns: tokens
    /// separated by spaces, the program first, `*` matching any text, `**`
    /// any number of arguments and `a|b` alternatives.
    ///
    /// # Errors
    ///
    /// `E_CONFIG` when a pattern is empty, starts with an option or has an
    /// empty alternative.
    pub fn with_patterns<S: AsRef<str>>(patterns: &[S]) -> Result<Self> {
        let extra = patterns
            .iter()
            .map(|pattern| {
                Rule::from_pattern(pattern.as_ref()).map_err(|problem| {
                    Error::new(
                        ErrorCode::Config,
                        format!("invalid entry of run.irreversible_patterns: {problem}"),
                    )
                    .with_found(pattern.as_ref())
                    .with_fix("write the program name first, for example \"deploy-tool publish\"")
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { extra })
    }

    /// Rules matched by the command line `argv` (program first), in table
    /// order, each rule once. Programs outside the table yield no match.
    #[must_use]
    pub fn classify<S: AsRef<str>>(&self, argv: &[S]) -> Vec<EffectMatch> {
        let argv = owned(argv);
        let Some(tables) = builtin() else {
            return Vec::new();
        };
        let chain = invocations(&argv, &tables.wrappers, &tables.value_options);
        let mut matches: Vec<EffectMatch> = Vec::new();
        for rule in tables.effects.iter().chain(&self.extra) {
            let Some(invocation) = chain.iter().find(|inv| rule.matches(inv).is_some()) else {
                continue;
            };
            if matches.iter().all(|found| found.rule != rule.name) {
                matches.push(EffectMatch {
                    rule: rule.name.clone(),
                    program: invocation.program.clone(),
                });
            }
        }
        matches
    }
}

/// Rules of the built-in effect table matched by `argv` (program first).
///
/// # Examples
///
/// ```
/// use nurt_proc::classify_effects;
///
/// let matches = classify_effects(&["sudo", "-u", "deploy", "git", "push", "origin"]);
/// assert_eq!(matches[0].rule, "git push");
/// assert!(classify_effects(&["git", "status"]).is_empty());
/// ```
#[must_use]
pub fn classify_effects<S: AsRef<str>>(argv: &[S]) -> Vec<EffectMatch> {
    EffectClassifier::builtin().classify(argv)
}

/// Program that needs a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interactive {
    /// Name of the matched rule, for example `less`.
    pub rule: String,
    /// The program needs a terminal only when it gets no standard input
    /// (an interpreter or a database client without a script).
    pub without_input: bool,
}

/// Interpreter given its script in the arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueScript {
    /// Name of the matched rule, for example `shell -c`.
    pub rule: String,
    /// Interpreter program.
    pub interpreter: String,
    /// Script text as given.
    pub script: String,
}

/// Classes of a command line. The console starts every command line; the
/// classes describe how the program behaves.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArgvClasses {
    /// Name of the rule when the program waits without end (`sleep`,
    /// `tail -f`, `watch`).
    pub waits: Option<String>,
    /// The program needs a terminal.
    pub interactive: Option<Interactive>,
    /// The program runs a script given in the arguments.
    pub opaque_script: Option<OpaqueScript>,
}

/// Classes of the command line `argv` (program first).
///
/// # Examples
///
/// ```
/// use nurt_proc::classify_argv;
///
/// let classes = classify_argv(&["tail", "-f", "app.log"]);
/// assert_eq!(classes.waits.as_deref(), Some("tail -f|-F|--follow*"));
///
/// let classes = classify_argv(&["bash", "-c", "make && make test"]);
/// assert_eq!(classes.opaque_script.unwrap().script, "make && make test");
/// ```
#[must_use]
pub fn classify_argv<S: AsRef<str>>(argv: &[S]) -> ArgvClasses {
    let argv = owned(argv);
    let mut classes = ArgvClasses::default();
    let Some(tables) = builtin() else {
        return classes;
    };
    let chain = invocations(&argv, &tables.wrappers, &tables.value_options);
    for invocation in &chain {
        for rule in &tables.classes {
            if let Some(end) = rule.matches(invocation) {
                apply(&mut classes, rule, invocation, end);
            }
        }
    }
    classes
}

fn apply(classes: &mut ArgvClasses, rule: &Rule, invocation: &Invocation<'_>, end: usize) {
    match rule.class {
        Some(Class::Waits) if classes.waits.is_none() => classes.waits = Some(rule.name.clone()),
        Some(Class::Interactive) if classes.interactive.is_none() => {
            classes.interactive = Some(Interactive {
                rule: rule.name.clone(),
                without_input: rule.without_input,
            });
        }
        Some(Class::OpaqueScript) if classes.opaque_script.is_none() => {
            classes.opaque_script = rule.script(invocation, end).map(|script| OpaqueScript {
                rule: rule.name.clone(),
                interpreter: invocation.program.clone(),
                script,
            });
        }
        _ => {}
    }
}

fn owned<S: AsRef<str>>(argv: &[S]) -> Vec<String> {
    argv.iter().map(|arg| arg.as_ref().to_owned()).collect()
}

/// Built-in tables, parsed once; `None` only when the embedded tables are
/// malformed, which the tests of this module exclude.
fn builtin() -> Option<&'static Tables> {
    static TABLES: OnceLock<Option<Tables>> = OnceLock::new();
    TABLES.get_or_init(|| Tables::builtin().ok()).as_ref()
}

#[cfg(test)]
mod tests {
    use super::table::Tables;

    /// Smallest numbers of effect rules required of the built-in table.
    const MIN_EFFECT_RULES: usize = 40;

    #[test]
    fn builtin_tables_are_well_formed() {
        let tables = Tables::builtin().unwrap();
        assert!(tables.effects.len() >= MIN_EFFECT_RULES);
        assert!(!tables.classes.is_empty());
        assert!(!tables.wrappers.is_empty());
    }

    #[test]
    fn rejects_unknown_format_and_misplaced_classes() {
        let unknown = "format = \"other/1\"";
        assert!(Tables::load(unknown, unknown).is_err());
        let effects_with_class =
            "format = \"nurt-argv-rules/1\"\n[[rule]]\nclass = \"waits\"\npattern = \"sleep\"";
        let empty = "format = \"nurt-argv-rules/1\"";
        assert!(Tables::load(effects_with_class, empty).is_err());
        let class_without_class = "format = \"nurt-argv-rules/1\"\n[[rule]]\npattern = \"sleep\"";
        assert!(Tables::load(empty, class_without_class).is_err());
        let bad_option = "format = \"nurt-argv-rules/1\"\n[[rule]]\npattern = \"curl x=y|\"";
        assert!(Tables::load(bad_option, empty).is_err());
    }
}
