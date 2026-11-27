// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::credentials::program
// Responsibility:   Credential helpers run as programs of the user through the process host.
// Dependencies:     gix_credentials, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Credential helpers run as programs of the user through the process host
//! of the console, with the credential protocol of `git` on standard input
//! and output and without a terminal.

use gix_credentials::protocol::{Context, ContextOptions};
use nurt_contracts::services::{EnvVar, ProgramInput, ProgramSpec};

use super::ProgramRunner;
use super::config::ProgramHelper;

/// Shell used for helpers defined as `!<snippet>`, as by `git`.
const SHELL: &str = "sh";

/// Builds the command line for `action` (`get`, `store` or `erase`).
pub(crate) fn command(helper: &ProgramHelper, action: &str) -> (String, Vec<String>) {
    match helper {
        ProgramHelper::Command { program, args } => {
            let mut args = args.clone();
            args.push(action.to_owned());
            (program.clone(), args)
        }
        ProgramHelper::Script(script) => (
            SHELL.to_owned(),
            vec![
                "-c".to_owned(),
                format!("{script} \"$@\""),
                script.clone(),
                action.to_owned(),
            ],
        ),
    }
}

/// Runs `helper` for `action` with `context` on standard input.
///
/// Returns the answer for `get` and `None` for the other actions. A helper
/// that exits with a code other than 0 gives no answer.
pub(crate) fn run(
    runner: &ProgramRunner,
    helper: &ProgramHelper,
    action: &str,
    context: &Context,
) -> Result<Option<Context>, String> {
    let (program, args) = command(helper, action);
    let mut input = Vec::new();
    context.write_to(&mut input).map_err(|e| e.to_string())?;
    let mut spec = ProgramSpec::new(program, args, runner.cwd.clone());
    spec.stdin = ProgramInput::Bytes(input);
    spec.env.push(EnvVar {
        name: "GIT_TERMINAL_PROMPT".to_owned(),
        value: Some("0".to_owned()),
    });
    let outcome = runner
        .host
        .run_program(&spec, &runner.cancel)
        .map_err(|e| e.data().message.clone())?;
    if outcome.cancelled || outcome.timed_out {
        return Err("stopped before it answered".to_owned());
    }
    if outcome.exit_code != Some(0) {
        let code = outcome
            .exit_code
            .map_or_else(|| "a signal".to_owned(), |c| format!("code {c}"));
        return Err(format!("exited with {code}"));
    }
    if action != "get" {
        return Ok(None);
    }
    Context::from_bytes(&outcome.stdout, ContextOptions::default())
        .map(Some)
        .map_err(|e| format!("answered with invalid output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_the_action_to_program_arguments() {
        let helper = ProgramHelper::Command {
            program: "git-credential-netrc".to_owned(),
            args: vec!["-f".to_owned(), "x".to_owned()],
        };
        assert_eq!(
            command(&helper, "get"),
            (
                "git-credential-netrc".to_owned(),
                vec!["-f".to_owned(), "x".to_owned(), "get".to_owned()]
            )
        );
    }

    #[test]
    fn runs_scripts_through_the_shell_with_positional_action() {
        let helper = ProgramHelper::Script("gh auth git-credential".to_owned());
        let (program, args) = command(&helper, "store");
        assert_eq!(program, "sh");
        assert_eq!(
            args,
            [
                "-c",
                "gh auth git-credential \"$@\"",
                "gh auth git-credential",
                "store"
            ]
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
