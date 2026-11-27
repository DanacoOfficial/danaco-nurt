// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::classify::argv
// Responsibility:   Splitting of a command line into the programs it starts: the program itself and every program started through a wrapper (`sudo`, `env`, `npx`).
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
/// in lower case. A backslash separates directories on Windows and in
/// Windows paths (`C:\dir\git.exe`, `\\host\share\git.exe`); in a plain word
/// on other systems it is part of the file name, so `x\ls` is not `ls`.
pub(super) fn program_name(word: &str) -> String {
    let separators: &[char] = if cfg!(windows) || is_windows_path(word) {
        &['/', '\\']
    } else {
        &['/']
    };
    let base = word.rsplit(separators).next().unwrap_or(word);
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

/// Whether `word` is a drive-letter or UNC path.
fn is_windows_path(word: &str) -> bool {
    let bytes = word.as_bytes();
    word.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn wrapper(program: &str) -> Wrapper {
        Wrapper {
            program: Words::parse(program).unwrap(),
            command: Vec::new(),
            value_options: Vec::new(),
            assignments: false,
            skip: 0,
            module_option: None,
        }
    }

    /// Programs of the chain of `argv`, with the arguments of each.
    fn chain(argv: &[&str], wrappers: &[Wrapper]) -> Vec<(String, Vec<String>)> {
        let argv = strings(argv);
        invocations(&argv, wrappers, &[])
            .into_iter()
            .map(|invocation| (invocation.program, invocation.args.to_vec()))
            .collect()
    }

    #[test]
    fn program_names_lose_directories_versions_and_extensions() {
        assert_eq!(program_name("/usr/bin/Git"), "git");
        assert_eq!(program_name(r"C:\Tools\TOOL.EXE"), "tool");
        assert_eq!(program_name(r"\\host\share\Git.exe"), "git");
        assert_eq!(
            program_name(r"x\ls"),
            if cfg!(windows) { "ls" } else { r"x\ls" }
        );
        assert_eq!(program_name("vercel@latest"), "vercel");
        assert_eq!(program_name("@scope/pkg"), "pkg");
        assert_eq!(program_name("@angular"), "@angular");
        assert_eq!(program_name("run.CMD"), "run");
        assert_eq!(program_name("a.exe.exe"), "a.exe");
        assert_eq!(program_name("./bin/"), "");
        assert_eq!(program_name(""), "");
    }

    #[test]
    fn program_names_with_multi_byte_characters_are_cut_at_character_boundaries() {
        assert_eq!(program_name("Żółć@1.0"), "Żółć");
        assert_eq!(program_name("dir/Ünï@x.exe"), "Ünï");
        assert_eq!(program_name("łódź.exe"), "łódź");
    }

    #[test]
    fn a_lone_dash_is_an_argument_and_two_dashes_are_an_option() {
        assert!(!is_option("-") && !is_option("") && !is_option("x"));
        assert!(is_option("--") && is_option("-x") && is_option("--name"));
    }

    #[test]
    fn assignments_need_a_valid_variable_name() {
        for valid in ["A=1", "_a1=", "Path=x=y"] {
            assert!(is_assignment(valid), "{valid}");
        }
        for invalid in ["1A=x", "A-B=x", "=x", "plain", "", "ź=1"] {
            assert!(!is_assignment(invalid), "{invalid}");
        }
    }

    #[test]
    fn positionals_skip_options_and_the_values_of_value_options() {
        let args = strings(&["-a", "-n", "5", "x", "--", "-y"]);
        let plain = Invocation::new("p".to_owned(), &args, &[]);
        assert_eq!(plain.positionals, [(2, "5"), (3, "x"), (5, "-y")]);
        let with_value = Invocation::new("p".to_owned(), &args, &["-n".to_owned()]);
        assert_eq!(with_value.positionals, [(3, "x"), (5, "-y")]);
        let dangling = strings(&["x", "-n"]);
        let last = Invocation::new("p".to_owned(), &dangling, &["-n".to_owned()]);
        assert_eq!(last.positionals, [(0, "x")]);
    }

    #[test]
    fn a_program_without_a_wrapper_is_the_whole_chain() {
        let chain = chain(&["git", "push"], &[wrapper("sudo")]);
        assert_eq!(chain, [("git".to_owned(), strings(&["push"]))]);
        assert!(super::invocations(&[], &[], &[]).is_empty());
    }

    #[test]
    fn a_wrapper_starts_the_next_program() {
        let mut sudo = wrapper("sudo");
        sudo.value_options = strings(&["-u"]);
        let found = chain(&["sudo", "-u", "root", "git", "push"], &[sudo]);
        assert_eq!(
            found,
            [
                ("sudo".to_owned(), strings(&["-u", "root", "git", "push"])),
                ("git".to_owned(), strings(&["push"])),
            ]
        );
    }

    #[test]
    fn wrappers_nest_and_skip_assignments_and_positionals() {
        let mut env = wrapper("env");
        env.assignments = true;
        let mut timeout = wrapper("timeout");
        timeout.skip = 1;
        let found = chain(
            &["env", "A=1", "timeout", "10", "make", "test"],
            &[env, timeout],
        );
        let programs: Vec<&str> = found.iter().map(|(program, _)| program.as_str()).collect();
        assert_eq!(programs, ["env", "timeout", "make"]);
        assert_eq!(found[2].1, strings(&["test"]));
    }

    #[test]
    fn a_wrapper_needs_its_subcommand_and_a_program_after_it() {
        let mut pnpm = wrapper("pnpm");
        pnpm.command = vec![Words::parse("exec|dlx").unwrap()];
        let wrappers = [pnpm];
        let programs = |argv: &[&str]| -> Vec<String> {
            chain(argv, &wrappers)
                .into_iter()
                .map(|(program, _)| program)
                .collect()
        };
        assert_eq!(programs(&["pnpm", "exec", "vitest"]), ["pnpm", "vitest"]);
        assert_eq!(programs(&["pnpm", "install"]), ["pnpm"]);
        assert_eq!(programs(&["pnpm", "exec"]), ["pnpm"]);
        assert_eq!(programs(&["pnpm"]), ["pnpm"]);
    }

    #[test]
    fn a_module_wrapper_applies_only_to_the_module_option() {
        let mut python = wrapper("python|python3");
        python.module_option = Some("-m".to_owned());
        python.value_options = strings(&["-c"]);
        let wrappers = [python];
        let found = chain(&["python3", "-m", "twine", "upload"], &wrappers);
        assert_eq!(found[1], ("twine".to_owned(), strings(&["upload"])));
        assert_eq!(chain(&["python3", "script.py"], &wrappers).len(), 1);
        assert_eq!(chain(&["python3", "-c", "print(1)"], &wrappers).len(), 1);
        assert_eq!(chain(&["python3", "-m"], &wrappers).len(), 1);
    }

    #[test]
    fn the_end_of_options_lets_a_wrapper_start_a_program_named_like_an_option() {
        let found = chain(&["sudo", "--", "-weird"], &[wrapper("sudo")]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].0, "-weird");
        assert_eq!(chain(&["sudo", "-x"], &[wrapper("sudo")]).len(), 1);
        assert_eq!(chain(&["sudo"], &[wrapper("sudo")]).len(), 1);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
