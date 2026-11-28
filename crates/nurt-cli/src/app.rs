// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::app
// Responsibility:   Entry of the command line: input and output streams, engine source and the dispatcher of invocations.
// Dependencies:     crossterm, nurt_contracts, nurt_core
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Entry of the command line: input and output streams, engine source and
//! the dispatcher of invocations.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::sync::Arc;

use nurt_contracts::env::Host;
use nurt_contracts::error::Error;
use nurt_core::{Engine, EngineConfig, EngineError};

use crate::args::{self, Invocation};
use crate::commands;
use crate::elevated;
use crate::exit;
use crate::integrations;
use crate::output::Style;
use crate::system::SystemHost;
use crate::text;

/// Which standard streams are terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Terminal {
    /// The input is a terminal.
    pub stdin: bool,
    /// The output is a terminal.
    pub stdout: bool,
    /// The error output is a terminal.
    pub stderr: bool,
}

impl Terminal {
    /// Whether the session is interactive: input and output are terminals.
    #[must_use]
    pub fn is_interactive(self) -> bool {
        self.stdin && self.stdout
    }
}

/// Streams of one invocation.
pub struct Io<'a> {
    /// Standard output.
    pub out: &'a mut dyn Write,
    /// Standard error output.
    pub err: &'a mut dyn Write,
    /// Standard input.
    pub input: &'a mut dyn BufRead,
    /// Which streams are terminals.
    pub terminal: Terminal,
}

/// Provider of the engine and of the host it runs on.
///
/// The executable uses [`SystemEngine`]; tests give an engine built from
/// virtual components. The engine is opened only by the commands that need it.
pub trait EngineSource {
    /// Opens an engine.
    ///
    /// # Errors
    ///
    /// Returns the error of the engine, for example an invalid configuration.
    fn open(&self) -> Result<Engine, EngineError>;

    /// Opens the engine of the hidden subcommand `elevated-exec`, which runs
    /// with the identity of the administrator. With an `owner` the engine
    /// writes the journal of that account; without one it keeps the journal of
    /// its own account. By default it is the engine of [`open`](Self::open).
    ///
    /// # Errors
    ///
    /// Returns the error of the engine, for example an invalid configuration.
    fn open_elevated(
        &self,
        owner: Option<&nurt_core::ElevatedOwner>,
    ) -> Result<Engine, EngineError> {
        let _ = owner;
        self.open()
    }

    /// The host of the engine.
    fn host(&self) -> Arc<dyn Host>;
}

/// Engine of the running process: system clock and host, platform
/// configuration and the language front end of the build.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEngine;

impl EngineSource for SystemEngine {
    fn open(&self) -> Result<Engine, EngineError> {
        Engine::open(EngineConfig::system().with_frontend(integrations::frontend()))
    }

    fn open_elevated(
        &self,
        owner: Option<&nurt_core::ElevatedOwner>,
    ) -> Result<Engine, EngineError> {
        let config = EngineConfig::system().with_frontend(integrations::frontend());
        match owner {
            Some(owner) => Engine::open(config.with_account_journal(nurt_core::AccountJournal {
                uid: owner.uid,
                gid: owner.gid,
                state_dir: std::path::PathBuf::from(&owner.state_dir),
            })),
            None => Engine::open(config),
        }
    }

    fn host(&self) -> Arc<dyn Host> {
        Arc::new(SystemHost::detect())
    }
}

/// Reason a command did not complete.
#[derive(Debug, Clone, PartialEq)]
pub enum CliError {
    /// The command line is invalid.
    Usage(String),
    /// The subcommand is not available in this build.
    Unavailable(String),
    /// The language, the engine or the journal reported an error.
    Language(Box<Error>),
    /// Reading or writing a file or a stream failed.
    Failure(String),
}

impl From<Error> for CliError {
    fn from(error: Error) -> Self {
        Self::Language(Box::new(error))
    }
}

impl From<EngineError> for CliError {
    fn from(error: EngineError) -> Self {
        Self::Language(Box::new(error.to_error()))
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Failure(format!("cannot write the output: {error}"))
    }
}

/// An engine with the styles of the output streams.
pub struct Context {
    /// The engine.
    pub engine: Engine,
    /// Styles of the standard output.
    pub out_style: Style,
    /// Styles of the error output.
    pub err_style: Style,
}

impl Context {
    /// Opens the engine and chooses the styles from `output.color`, `NO_COLOR`
    /// and the terminals.
    ///
    /// # Errors
    ///
    /// Returns the error of the engine.
    pub fn open(terminal: Terminal, source: &dyn EngineSource) -> Result<Self, CliError> {
        let engine = source.open()?;
        let mode = engine.config().output.color;
        let no_color = source
            .host()
            .env_var("NO_COLOR")
            .is_some_and(|value| !value.is_empty());
        Ok(Self {
            out_style: Style::resolve(mode, terminal.stdout, no_color),
            err_style: Style::resolve(mode, terminal.stderr, no_color),
            engine,
        })
    }
}

/// Runs one invocation and returns the exit code.
///
/// `args` are the arguments after the name of the executable.
pub fn run(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> u8 {
    match dispatch(args, io, source) {
        Ok(code) => code,
        Err(error) => report(&error, io),
    }
}

/// Runs one invocation of the executable and returns its exit code.
///
/// The protocol servers of `serve` and `mcp` own the standard output of the process, so
/// it is started here; every other invocation runs through [`run`].
pub fn run_process(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> ExitCode {
    if args.first().map(String::as_str) == Some(nurt_core::elevation::ELEVATED_EXEC) {
        return ExitCode::from(elevated::run(io, source));
    }
    if let Invocation::Subcommand {
        name: "serve",
        args: rest,
    } = args::resolve(args)
        && !args::wants_help(&rest)
    {
        return match integrations::serve::start(&rest, source, &mut *io.input, io::stdout()) {
            Ok(code) => code,
            Err(error) => ExitCode::from(report(&error, io)),
        };
    }
    if let Invocation::Subcommand {
        name: "mcp",
        args: rest,
    } = args::resolve(args)
        && !args::wants_help(&rest)
    {
        return match integrations::mcp::start(&rest, source) {
            Ok(code) => code,
            Err(error) => ExitCode::from(report(&error, io)),
        };
    }
    if args.is_empty() {
        return ExitCode::from(session(io, source));
    }
    ExitCode::from(run(args, io, source))
}

/// Runs the invocation without arguments, the interactive session that a
/// shortcut starts, and returns its exit code.
///
/// When the session ends with an error or with a panic and this process is
/// the only one attached to its console, the window of the console would
/// close together with the message. The process then waits for a line of the
/// input, so the message can be read.
fn session(io: &mut Io<'_>, source: &dyn EngineSource) -> u8 {
    let on_a_terminal = io.terminal.stdin && io.terminal.stderr;
    let window_is_ours = on_a_terminal && nurt_core::system::console_is_private();
    if window_is_ours {
        keep_window_after_panic();
    }
    match dispatch(&[], io, source) {
        Ok(code) => code,
        Err(error) => {
            let code = report(&error, io);
            if window_is_ours {
                let _ = writeln!(io.err, "{CLOSE_NOTICE}");
                let _ = io.input.read_line(&mut String::new());
            }
            code
        }
    }
}

/// Line written before the process waits so that its window stays open.
const CLOSE_NOTICE: &str = "Press Enter to close this window.";

/// Name that the standard library gives to the first thread of a process.
const MAIN_THREAD: &str = "main";

/// Makes a panic of the main thread wait for a line of the input after its
/// message is written. The line editor may have left the console in raw
/// mode, in which the Enter key does not end a line, so that mode is turned
/// off first. A panic of another thread does not end the process and is
/// reported as before.
fn keep_window_after_panic() {
    let write_message = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_message(info);
        if std::thread::current().name() == Some(MAIN_THREAD) {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = writeln!(io::stderr(), "{CLOSE_NOTICE}");
            let _ = io::stdin().read_line(&mut String::new());
        }
    }));
}

fn dispatch(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    match args::resolve(args) {
        Invocation::Version { full } => {
            let text = if full {
                text::full_version()
            } else {
                text::short_version()
            };
            writeln!(io.out, "{text}")?;
            Ok(exit::SUCCESS)
        }
        Invocation::Help => {
            writeln!(io.out, "{}", text::usage())?;
            Ok(exit::SUCCESS)
        }
        Invocation::Subcommand { name, args } => {
            if args::wants_help(&args)
                && let Some(usage) = text::subcommand_usage(name)
            {
                writeln!(io.out, "{usage}")?;
                return Ok(exit::SUCCESS);
            }
            commands::run_subcommand(name, &args, io, source)
        }
        Invocation::Program { args } => commands::exec(&args, io, source),
    }
}

/// Style of the error output: a terminal is coloured unless `NO_COLOR` holds
/// a value.
fn error_style(stderr_is_terminal: bool, no_color: Option<std::ffi::OsString>) -> Style {
    let no_color = no_color.is_some_and(|value| !value.is_empty());
    Style::resolve(
        nurt_contracts::config::ColorMode::Auto,
        stderr_is_terminal,
        no_color,
    )
}

/// Writes the message of an error to the error output and returns its exit
/// code. A failure to write the message is ignored: nothing else can report
/// it.
fn report(error: &CliError, io: &mut Io<'_>) -> u8 {
    let style = error_style(io.terminal.stderr, std::env::var_os("NO_COLOR"));
    let (code, lines) = match error {
        CliError::Usage(message) => (
            exit::USAGE,
            vec![
                format!("nurt: {message}"),
                "run 'nurt --help' for usage".to_owned(),
            ],
        ),
        CliError::Unavailable(name) => (
            exit::UNAVAILABLE,
            vec![format!("nurt: {name} is not available in this build")],
        ),
        CliError::Failure(message) => (exit::FAILURE, vec![format!("nurt: {message}")]),
        CliError::Language(error) => {
            let mut lines = vec![style.error(&error.to_string())];
            lines.extend(error.fix.iter().map(|fix| format!("fix: {fix}")));
            (exit::for_error_code(error.code), lines)
        }
    };
    for line in lines {
        if writeln!(io.err, "{line}").is_err() {
            break;
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::*;

    fn failed(error: &CliError, stderr_is_terminal: bool) -> (u8, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut input: &[u8] = b"";
        let mut io = Io {
            out: &mut out,
            err: &mut err,
            input: &mut input,
            terminal: Terminal {
                stdin: false,
                stdout: false,
                stderr: stderr_is_terminal,
            },
        };
        let code = report(error, &mut io);
        assert!(out.is_empty());
        (code, String::from_utf8(err).unwrap())
    }

    #[test]
    fn interactive_terminal_needs_input_and_output() {
        let both = Terminal {
            stdin: true,
            stdout: true,
            stderr: false,
        };
        assert!(both.is_interactive());
        assert!(
            !Terminal {
                stdout: false,
                ..both
            }
            .is_interactive()
        );
        assert!(
            !Terminal {
                stdin: false,
                ..both
            }
            .is_interactive()
        );
        assert!(!Terminal::default().is_interactive());
    }

    #[test]
    fn usage_error_names_the_help_option() {
        let (code, text) = failed(&CliError::Usage("bad option".to_owned()), false);
        assert_eq!(code, exit::USAGE);
        assert_eq!(text, "nurt: bad option\nrun 'nurt --help' for usage\n");
    }

    #[test]
    fn unavailable_error_names_the_subcommand() {
        let (code, text) = failed(&CliError::Unavailable("lsp".to_owned()), false);
        assert_eq!(code, exit::UNAVAILABLE);
        assert_eq!(text, "nurt: lsp is not available in this build\n");
    }

    #[test]
    fn failure_error_is_reported_with_the_failure_code() {
        let (code, text) = failed(&CliError::Failure("disk full".to_owned()), false);
        assert_eq!(code, exit::FAILURE);
        assert_eq!(text, "nurt: disk full\n");
    }

    #[test]
    fn language_error_prints_the_message_and_the_fix() {
        let error = Error::new(ErrorCode::UnknownCommand, "unknown command `remvoe`")
            .with_fix("remove \"/tmp/a\"");
        let (code, text) = failed(&CliError::from(error), false);
        assert_eq!(code, exit::for_error_code(ErrorCode::UnknownCommand));
        assert!(text.contains("unknown command `remvoe`"));
        assert!(text.ends_with("fix: remove \"/tmp/a\"\n"));
    }

    #[test]
    fn io_error_becomes_a_failure_naming_the_output() {
        let error = CliError::from(io::Error::other("closed"));
        assert_eq!(
            error,
            CliError::Failure("cannot write the output: closed".to_owned())
        );
    }

    #[test]
    fn engine_error_becomes_a_language_error() {
        let error = EngineError::argument("no engine");
        assert!(matches!(CliError::from(error), CliError::Language(_)));
    }

    #[test]
    fn report_stops_at_a_broken_error_stream() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (mut out, mut err) = (Vec::new(), Closed);
        let mut input: &[u8] = b"";
        let mut io = Io {
            out: &mut out,
            err: &mut err,
            input: &mut input,
            terminal: Terminal::default(),
        };
        assert_eq!(
            report(&CliError::Failure("x".to_owned()), &mut io),
            exit::FAILURE
        );
    }

    #[test]
    fn no_color_with_a_value_turns_the_colours_of_errors_off() {
        let set = |value: &str| Some(std::ffi::OsString::from(value));
        assert!(!error_style(true, set("1")).is_enabled());
        assert!(error_style(true, set("")).is_enabled());
        assert!(error_style(true, None).is_enabled());
        assert!(!error_style(false, None).is_enabled());
        assert!(!error_style(false, set("")).is_enabled());
    }

    #[test]
    fn terminal_error_output_is_coloured_without_no_color() {
        let style = error_style(true, None);
        assert_eq!(style.error("failed"), "\u{1b}[31mfailed\u{1b}[0m");
        let plain = error_style(true, Some(std::ffi::OsString::from("yes")));
        assert_eq!(plain.error("failed"), "failed");
    }

    #[test]
    fn system_engine_reports_a_host() {
        assert!(SystemEngine.host().env_var("NURT_UNSET_VARIABLE").is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
