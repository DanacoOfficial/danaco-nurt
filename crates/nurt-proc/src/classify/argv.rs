// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Splitting of a command line into the programs it starts: the program
//! itself and every program started through a wrapper (`sudo`, `env`, `npx`).

use super::pattern::Words;

/// Extensions of executable files removed from program names.
const EXECUTABLE_EXTENSIONS: [&str; 4] = [".exe", ".cmd", ".bat", ".com"];

/// Whether `arg` is an option (`-x`, `--name`); a lone `-` is an argument.
pub(super) fn is_option(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-')
}

/// Program name compared with patterns: the last path component, without a
/// package version (`vercel@latest`) and without an executable extension,
/// in lower case.
pub(super) fn program_name(word: &str) -> String {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let base = match base.char_indices().skip(1).find(|&(_, c)| c == '@') {
        Some((at, _)) => base.get(..at).unwrap_or(base),
        None => base,
    };
    let lower = base.to_ascii_lowercase();
    EXECUTABLE_EXTENSIONS
        .iter()
        .find_map(|extension| lower.strip_suffix(extension))
        .map_or_else(|| lower.clone(), str::to_owned)
}

/// Whether `arg` is an environment assignment `NAME=VALUE`.
fn is_assignment(arg: &str) -> bool {
    let Some((name, _)) = arg.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Program that starts another program given in its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Wrapper {
    /// Names of the wrapper.
    pub(super) program: Words,
    /// Subcommand that must follow (`exec` of `pnpm exec`).
    pub(super) command: Vec<Words>,
    /// Options followed by a value.
    pub(super) value_options: Vec<String>,
    /// `NAME=VALUE` words are skipped.
    pub(super) assignments: bool,
    /// Positional arguments before the started program.
    pub(super) skip: usize,
    /// Option naming a module run as the program; without it the wrapper does
    /// not apply.
    pub(super) module_option: Option<String>,
}

impl Wrapper {
    /// Index in `args` of the started program, or `None` when the arguments
    /// do not start a program through this wrapper.
    fn started(&self, args: &[String]) -> Option<usize> {
        let mut command = self.command.iter();
        let mut skip = self.skip;
        let mut options_end = false;
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if !options_end && arg == "--" {
                options_end = true;
            } else if !options_end && is_option(arg) {
                if self.module_option.as_deref() == Some(arg.as_str()) {
                    return (index + 1 < args.len()).then_some(index + 1);
                }
                if self.value_options.iter().any(|option| option == arg) {
                    index += 1;
                }
            } else if !(self.assignments && !options_end && is_assignment(arg)) {
                if let Some(words) = command.next() {
                    if !words.matches(arg) {
                        return None;
                    }
                } else if skip > 0 {
                    skip -= 1;
                } else {
                    return self.module_option.is_none().then_some(index);
                }
            }
            index += 1;
        }
        None
    }
}

/// Options of a program that are followed by a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValueOptions {
    /// Program names.
    pub(super) program: Words,
    /// Options followed by a value.
    pub(super) options: Vec<String>,
}

/// One program of a command line with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Invocation<'a> {
    /// Program name as compared with patterns.
    pub(super) program: String,
    /// Arguments after the program.
    pub(super) args: &'a [String],
    /// Positional arguments with their index in `args`.
    pub(super) positionals: Vec<(usize, &'a str)>,
}

impl<'a> Invocation<'a> {
    fn new(program: String, args: &'a [String], value_options: &[String]) -> Self {
        let mut positionals = Vec::new();
        let mut options_end = false;
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if options_end || !is_option(arg) {
                positionals.push((index, arg.as_str()));
            } else if arg == "--" {
                options_end = true;
            } else if value_options.iter().any(|option| option == arg) {
                index += 1;
            }
            index += 1;
        }
        Self {
            program,
            args,
            positionals,
        }
    }
}

/// The program of `argv` and every program it starts through wrappers, outer
/// first.
pub(super) fn invocations<'a>(
    argv: &'a [String],
    wrappers: &[Wrapper],
    value_options: &[ValueOptions],
) -> Vec<Invocation<'a>> {
    let mut chain = Vec::new();
    let mut start = 0;
    while let (Some(first), Some(args)) = (argv.get(start), argv.get(start + 1..)) {
        let program = program_name(first);
        let started = wrappers
            .iter()
            .filter(|wrapper| wrapper.program.matches(&program))
            .find_map(|wrapper| wrapper.started(args));
        let options = value_options
            .iter()
            .find(|entry| entry.program.matches(&program))
            .map_or(&[][..], |entry| entry.options.as_slice());
        chain.push(Invocation::new(program, args, options));
        match started {
            Some(offset) => start += 1 + offset,
            None => break,
        }
    }
    chain
}
