// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::repl
// Responsibility:   The interactive session (`nurt repl`).
// Dependencies:     crossterm, nurt_contracts, nurt_core, reedline, serde_json
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The interactive session (`nurt repl`).
//!
//! On a terminal the session uses the line editor with a history file in the
//! state directory, completion from the command catalog and highlighting.
//! Input that is not a terminal is read line by line without the editor and
//! without the banner, so that a script can be piped into the session.
//!
//! Besides programs the session understands `help`, `jobs`, `bg <program>`,
//! `fg [id]`, `kill [id]` and `exit`.
//!
//! Ctrl-C while a program runs cancels that program and returns to the
//! prompt; the session continues. At the prompt Ctrl-C discards the line.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::cursor::SetCursorStyle;
use nurt_contracts::envelope::{OutputFormat, Request};
use nurt_core::{DeliveredEvent, Engine, EventListener, ExecHandle};
use reedline::{
    Color, ColumnarMenu, CompletionResult, CursorConfig, DefaultPrompt, DefaultPromptSegment,
    Emacs, FileBackedHistory, Highlighter, KeyCode, KeyModifiers, MenuBuilder, Prompt,
    PromptEditMode, PromptHistorySearch, Reedline, ReedlineEvent, ReedlineMenu, Signal, Span,
    StyledText, Suggestion, default_emacs_keybindings,
};

use crate::EngineSource;
use crate::app::{CliError, Context, Io, Terminal};
use crate::banner;
use crate::complete::{TokenKind, Vocabulary};
use crate::exit;
use crate::output::{self, Style as Palette};
use crate::text;
use crate::theme::Theme;

/// Number of lines kept in the history file.
const HISTORY_LINES: usize = 1000;
/// Name of the history file in the state directory.
const HISTORY_FILE: &str = "history.txt";
/// Interval at which a running program is checked for an interrupt.
const INTERRUPT_POLL: Duration = Duration::from_millis(50);
/// Name of the completion menu of the line editor.
const MENU_NAME: &str = "completion_menu";
/// Prompt of the session.
const PROMPT: &str = "nurt";
/// Notice written when the session goes on without the line editor.
const PLAIN_INPUT_NOTICE: &str = "nurt: the session continues without the line editor; \
     type a program and press Enter, `exit` ends the session";

/// One line read from the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A line of text, without the line ending.
    Line(String),
    /// The user interrupted the line (Ctrl-C); the session continues.
    Interrupted,
    /// The input ended.
    End,
}

/// Source of the lines of a session.
pub trait LineSource {
    /// Reads the next line.
    fn read_line(&mut self) -> Input;

    /// The message of the failure that ended the input, or `None` when the
    /// input ended in the regular way.
    fn failure(&self) -> Option<String> {
        None
    }
}

/// Lines read from a stream without editing.
pub struct PlainLines<'a> {
    input: &'a mut dyn BufRead,
}

impl<'a> PlainLines<'a> {
    /// Reads lines from `input`.
    #[must_use]
    pub fn new(input: &'a mut dyn BufRead) -> Self {
        Self { input }
    }
}

impl LineSource for PlainLines<'_> {
    fn read_line(&mut self) -> Input {
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            Ok(0) | Err(_) => Input::End,
            Ok(_) => Input::Line(line.trim_end_matches(['\n', '\r']).to_owned()),
        }
    }
}

/// Interrupt event of the session: set by the Ctrl-C handler, read by the
/// session while a program runs.
#[derive(Debug, Clone, Default)]
pub struct Interrupt {
    raised: Arc<AtomicBool>,
}

impl Interrupt {
    /// Creates an interrupt that is not raised.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Raises the interrupt.
    pub fn raise(&self) {
        self.raised.store(true, Ordering::SeqCst);
    }

    /// Whether the interrupt is raised and not yet taken.
    #[must_use]
    pub fn is_raised(&self) -> bool {
        self.raised.load(Ordering::SeqCst)
    }

    /// Clears the interrupt and returns whether it was raised.
    pub fn take(&self) -> bool {
        self.raised.swap(false, Ordering::SeqCst)
    }

    /// Raises this interrupt on every Ctrl-C of the process.
    ///
    /// # Errors
    ///
    /// Returns the message of the problem when the process already has a
    /// Ctrl-C handler or the handler cannot be installed.
    pub fn install_handler(&self) -> Result<(), String> {
        let interrupt = self.clone();
        ctrlc::set_handler(move || interrupt.raise()).map_err(|error| error.to_string())
    }
}

struct Suggestions(Vocabulary);

impl reedline::Completer for Suggestions {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        let (start, found) = self.0.complete(line, pos);
        let suggestions: Vec<Suggestion> = found
            .into_iter()
            .map(|value| Suggestion {
                value,
                span: Span::new(start, pos),
                append_whitespace: false,
                ..Suggestion::default()
            })
            .collect();
        CompletionResult::fresh(suggestions)
    }
}

struct Coloured(Vocabulary, Theme);

impl Highlighter for Coloured {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        for (kind, part) in self.0.tokens(line) {
            let style = match kind {
                TokenKind::Command => self.1.gold().bold(),
                TokenKind::Text => self.1.cyan().normal(),
                TokenKind::Pipe => self.1.silver().bold(),
                TokenKind::Space | TokenKind::Word => Default::default(),
            };
            styled.push((style, part));
        }
        styled
    }
}

/// Lines read with the line editor.
pub struct EditorLines {
    editor: Reedline,
    prompt: BrandPrompt,
    failure: Option<String>,
}

impl EditorLines {
    /// Creates the editor with completion and highlighting from
    /// `vocabulary`; the history is kept in `history`.
    ///
    /// # Errors
    ///
    /// Returns the message of the problem when the history file cannot be
    /// opened; the editor is then created without a saved history by
    /// [`EditorLines::without_history`].
    pub fn new(vocabulary: &Vocabulary, history: PathBuf) -> Result<Self, String> {
        let file = FileBackedHistory::with_file(HISTORY_LINES, history)
            .map_err(|error| error.to_string())?;
        Ok(Self::build(vocabulary, Some(Box::new(file))))
    }

    /// Creates the editor with a history that is not saved.
    #[must_use]
    pub fn without_history(vocabulary: &Vocabulary) -> Self {
        Self::build(vocabulary, None)
    }

    /// Turns the colours of the prompt and of the highlighting on or off.
    #[must_use]
    pub fn with_colour(self, enabled: bool) -> Self {
        Self {
            editor: self.editor.with_ansi_colors(enabled),
            prompt: self.prompt,
            failure: self.failure,
        }
    }

    fn build(vocabulary: &Vocabulary, history: Option<Box<FileBackedHistory>>) -> Self {
        let mut keybindings = default_emacs_keybindings();
        keybindings.add_binding(
            KeyModifiers::NONE,
            KeyCode::Tab,
            ReedlineEvent::UntilFound(vec![
                ReedlineEvent::Menu(MENU_NAME.to_owned()),
                ReedlineEvent::MenuNext,
            ]),
        );
        let theme = Theme::detect();
        let mut editor = Reedline::create()
            .with_completer(Box::new(Suggestions(vocabulary.clone())))
            .with_highlighter(Box::new(Coloured(vocabulary.clone(), theme)))
            .with_menu(ReedlineMenu::EngineCompleter(Box::new(
                ColumnarMenu::default().with_name(MENU_NAME),
            )))
            .with_edit_mode(Box::new(Emacs::new(keybindings)))
            .with_cursor_config(CursorConfig {
                // A blinking underscore while a line is typed; the line editor
                // restores the shape of the terminal when the line is read.
                emacs: Some(SetCursorStyle::BlinkingUnderScore),
                ..CursorConfig::default()
            });
        if let Some(history) = history {
            editor = editor.with_history(history);
        }
        Self {
            editor,
            prompt: BrandPrompt {
                inner: DefaultPrompt::new(
                    DefaultPromptSegment::Basic(PROMPT.to_owned()),
                    DefaultPromptSegment::Empty,
                ),
                theme,
            },
            failure: None,
        }
    }
}

impl LineSource for EditorLines {
    fn read_line(&mut self) -> Input {
        match self.editor.read_line(&self.prompt) {
            Ok(Signal::Success(line)) => Input::Line(line),
            Ok(Signal::CtrlC(_)) => Input::Interrupted,
            Ok(_) => Input::End,
            Err(error) => {
                self.failure = Some(error.to_string());
                Input::End
            }
        }
    }

    fn failure(&self) -> Option<String> {
        self.failure.clone()
    }
}

struct Discard;

impl EventListener for Discard {
    fn deliver(&self, _event: DeliveredEvent) {}
}

struct Job {
    id: u32,
    program: String,
    handle: ExecHandle,
}

/// State of a session: the engine, the background jobs and the exit code of
/// the last program.
pub struct Session<'a> {
    engine: &'a Engine,
    style: Palette,
    jobs: Vec<Job>,
    next_job: u32,
    last: u8,
    interrupt: Interrupt,
    id: String,
}

enum Flow {
    Continue,
    Exit,
}

fn split_word(line: &str) -> (&str, &str) {
    match line.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (line, ""),
    }
}

impl<'a> Session<'a> {
    /// Creates a session on `engine`; `style` colours the output.
    #[must_use]
    pub fn new(engine: &'a Engine, style: Palette) -> Self {
        let id = format!(
            "repl:{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        );
        engine.set_console(Some(id.clone()));
        Self {
            engine,
            style,
            jobs: Vec::new(),
            next_job: 1,
            last: exit::SUCCESS,
            interrupt: Interrupt::new(),
            id,
        }
    }

    /// Uses `interrupt` as the event that cancels the program in the
    /// foreground.
    #[must_use]
    pub fn with_interrupt(mut self, interrupt: Interrupt) -> Self {
        self.interrupt = interrupt;
        self
    }

    /// Identifier of the session in the engine.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Exit code of the last program that ran.
    #[must_use]
    pub fn last_exit_code(&self) -> u8 {
        self.last
    }

    /// Reads and runs lines until the input ends or the user exits.
    ///
    /// # Errors
    ///
    /// Returns the error of writing to the output.
    pub fn run(
        &mut self,
        lines: &mut dyn LineSource,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> io::Result<()> {
        self.read_lines(lines, out, err)?;
        self.cancel_all();
        Ok(())
    }

    /// Reads and runs the lines of `editor` and, when the editor stops with a
    /// failure, the lines of `plain`: a terminal that the line editor cannot
    /// use still gets a session, without editing, completion and history.
    ///
    /// # Errors
    ///
    /// Returns the error of writing to the output.
    pub fn run_with_plain_input(
        &mut self,
        editor: &mut dyn LineSource,
        plain: &mut dyn LineSource,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> io::Result<()> {
        self.read_lines(editor, out, err)?;
        if editor.failure().is_some() {
            writeln!(err, "{PLAIN_INPUT_NOTICE}")?;
            self.read_lines(plain, out, err)?;
        }
        self.cancel_all();
        Ok(())
    }

    fn read_lines(
        &mut self,
        lines: &mut dyn LineSource,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> io::Result<()> {
        loop {
            let line = match lines.read_line() {
                Input::Line(line) => line,
                Input::Interrupted => continue,
                Input::End => {
                    if let Some(message) = lines.failure() {
                        writeln!(err, "nurt: the line editor stopped: {message}")?;
                    }
                    break;
                }
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if matches!(self.handle(line, out, err)?, Flow::Exit) {
                break;
            }
        }
        Ok(())
    }

    fn handle(&mut self, line: &str, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<Flow> {
        let (word, rest) = split_word(line);
        match word {
            "exit" | "quit" => return Ok(Flow::Exit),
            "help" => self.help(rest, out, err)?,
            "jobs" => self.list_jobs(out)?,
            "bg" => self.background(rest, out, err)?,
            "fg" => self.foreground(rest, out, err)?,
            "kill" => self.kill(rest, out, err)?,
            _ => {
                let handle = self.start(line);
                self.finish(&handle, out)?;
            }
        }
        Ok(Flow::Continue)
    }

    fn start(&self, program: &str) -> ExecHandle {
        self.interrupt.take();
        self.engine
            .start(&Request::program(program), Arc::new(Discard))
    }

    fn finish(&mut self, handle: &ExecHandle, out: &mut dyn Write) -> io::Result<()> {
        let response = loop {
            if self.interrupt.take() {
                handle.cancel();
            }
            if let Some(response) = handle.wait(Some(INTERRUPT_POLL)) {
                break response;
            }
        };
        let text = output::render(&response, OutputFormat::Text, self.style);
        writeln!(out, "{text}")?;
        self.last = exit::for_report(&response.report);
        Ok(())
    }

    fn help(&self, topic: &str, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<()> {
        if topic.is_empty() {
            return writeln!(out, "{}", text::help_intro());
        }
        match self.engine.help(topic, None) {
            Ok(help) => writeln!(out, "{}", help.trim_end()),
            Err(error) => writeln!(err, "{}", self.style.error(&error.to_string())),
        }
    }

    fn list_jobs(&self, out: &mut dyn Write) -> io::Result<()> {
        for job in &self.jobs {
            let state = if job.handle.is_finished() {
                "finished"
            } else {
                "running"
            };
            writeln!(out, "[{}] {state} {}", job.id, job.program)?;
        }
        Ok(())
    }

    fn background(
        &mut self,
        program: &str,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> io::Result<()> {
        if program.is_empty() {
            return writeln!(err, "nurt: `bg` needs a program");
        }
        let handle = self.start(program);
        let id = self.next_job;
        self.next_job += 1;
        self.jobs.push(Job {
            id,
            program: program.to_owned(),
            handle,
        });
        writeln!(out, "[{id}] started")
    }

    /// Removes the job named by `argument`, or the newest job when the
    /// argument is empty.
    fn take_job(&mut self, argument: &str) -> Result<Job, String> {
        let index = if argument.is_empty() {
            self.jobs.len().checked_sub(1)
        } else {
            let id: u32 = argument
                .parse()
                .map_err(|_| format!("`{argument}` is not a job number"))?;
            self.jobs.iter().position(|job| job.id == id)
        };
        index
            .map(|index| self.jobs.remove(index))
            .ok_or_else(|| "no such job".to_owned())
    }

    fn foreground(
        &mut self,
        argument: &str,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> io::Result<()> {
        match self.take_job(argument) {
            Ok(job) => self.finish(&job.handle, out),
            Err(message) => writeln!(err, "nurt: {message}"),
        }
    }

    fn kill(&mut self, argument: &str, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<()> {
        match self.take_job(argument) {
            Ok(job) => {
                job.handle.cancel();
                let _response = job.handle.wait(None);
                writeln!(out, "[{}] cancelled", job.id)
            }
            Err(message) => writeln!(err, "nurt: {message}"),
        }
    }

    fn cancel_all(&mut self) {
        for job in self.jobs.drain(..) {
            job.handle.cancel();
            let _response = job.handle.wait(None);
        }
    }
}

/// The prompt of the session: the default prompt with the colours of the
/// identity, the name in gold and the indicator in silver.
struct BrandPrompt {
    inner: DefaultPrompt,
    theme: Theme,
}

impl Prompt for BrandPrompt {
    fn render_prompt_left(&self) -> std::borrow::Cow<'_, str> {
        self.inner.render_prompt_left()
    }

    fn render_prompt_right(&self) -> std::borrow::Cow<'_, str> {
        self.inner.render_prompt_right()
    }

    fn render_prompt_indicator(&self, edit_mode: PromptEditMode) -> std::borrow::Cow<'_, str> {
        self.inner.render_prompt_indicator(edit_mode)
    }

    fn render_prompt_multiline_indicator(&self) -> std::borrow::Cow<'_, str> {
        self.inner.render_prompt_multiline_indicator()
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> std::borrow::Cow<'_, str> {
        self.inner
            .render_prompt_history_search_indicator(history_search)
    }

    fn get_prompt_color(&self) -> Color {
        self.theme.gold()
    }

    fn get_indicator_color(&self) -> Color {
        self.theme.silver()
    }
}

/// Number of causes of a journal problem written below the banner.
const SHOWN_CAUSES: usize = 5;

/// Causes that the journal recorded for the entries it could not recover,
/// the first [`SHOWN_CAUSES`] of them without repetitions.
pub(crate) fn recovery_causes(problem: &nurt_contracts::error::Error) -> Vec<String> {
    let mut causes: Vec<String> = problem
        .data()
        .details
        .get("causes")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect();
    causes.dedup();
    causes.truncate(SHOWN_CAUSES);
    causes
}

/// The banner of the session; scripts, pipes and protocol servers receive
/// none, because the banner exists only on an interactive terminal.
#[must_use]
pub fn banner_for(terminal: Terminal) -> Option<String> {
    terminal.is_interactive().then(text::banner)
}

/// Runs the interactive session.
///
/// # Errors
///
/// Returns the reason the engine cannot be opened or the output cannot be
/// written.
pub fn run(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("`repl` takes no arguments".to_owned()));
    }
    let context = Context::open(io.terminal, source)?;
    let Io {
        out,
        err,
        input,
        terminal,
    } = io;
    let interrupt = Interrupt::new();
    let mut session =
        Session::new(&context.engine, context.out_style).with_interrupt(interrupt.clone());
    if banner_for(*terminal).is_some() {
        writeln!(
            out,
            "{}",
            banner::session_banner(
                text::VERSION,
                &|name| std::env::var(name).ok(),
                context.out_style.is_enabled(),
                Theme::detect(),
            )
        )?;
        if let Err(problem) = interrupt.install_handler() {
            let _ = writeln!(err, "nurt: Ctrl-C does not cancel programs: {problem}");
        }
        if let Some(problem) = context.engine.journal_problem() {
            // The session runs, but nothing a change replaces is kept until
            // the journal starts again, so the person is told at once.
            let _ = writeln!(
                err,
                "nurt: the journal did not start, so changes cannot be undone: {problem}"
            );
            for cause in recovery_causes(problem) {
                let _ = writeln!(err, "nurt: cause: {cause}");
            }
            let _ = writeln!(
                err,
                "nurt: `nurt journal salvage <volume>` lists the entries that need repair"
            );
        }
        let vocabulary = Vocabulary::from_engine(&context.engine);
        let editor = match context.engine.state_dir() {
            Some(state) => EditorLines::new(&vocabulary, state.root().join(HISTORY_FILE))
                .unwrap_or_else(|problem| {
                    // The session continues; the history is lost on exit.
                    let _ = writeln!(err, "nurt: the history is not saved: {problem}");
                    EditorLines::without_history(&vocabulary)
                }),
            None => EditorLines::without_history(&vocabulary),
        };
        let mut editor = editor.with_colour(context.out_style.is_enabled());
        session.run_with_plain_input(&mut editor, &mut PlainLines::new(*input), *out, *err)?;
    } else {
        session.run(&mut PlainLines::new(*input), *out, *err)?;
    }
    context.engine.end_session(Some(session.id()));
    Ok(session.last_exit_code())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn vocabulary() -> Vocabulary {
        Vocabulary::from_documents(&[json!({
            "format": "nurt-catalog-result/1",
            "commands": [{"name": "find", "params": [{"name": "path"}]}]
        })])
    }

    #[test]
    fn interrupt_is_taken_once() {
        let interrupt = Interrupt::new();
        assert!(!interrupt.take());
        interrupt.raise();
        assert!(interrupt.is_raised());
        assert!(interrupt.take());
        assert!(!interrupt.is_raised());
        assert!(!interrupt.take());
    }

    #[test]
    fn interrupt_clones_share_the_event() {
        let interrupt = Interrupt::new();
        interrupt.clone().raise();
        assert!(interrupt.take());
    }

    #[test]
    fn causes_of_a_journal_problem_are_listed_once_and_bounded() {
        use nurt_contracts::error::{Error, ErrorCode};
        let problem = Error::new(ErrorCode::Io, "entries could not be recovered")
            .with_detail("causes", json!(["a", "a", "b", "c", "d", "e", "f"]));
        assert_eq!(recovery_causes(&problem), ["a", "b", "c", "d", "e"]);
        let plain = Error::new(ErrorCode::Io, "the journal directory cannot be read");
        assert!(recovery_causes(&plain).is_empty());
    }

    #[test]
    fn a_line_is_split_at_its_first_white_space() {
        assert_eq!(split_word("find \"/srv\""), ("find", "\"/srv\""));
        assert_eq!(split_word("exit"), ("exit", ""));
        assert_eq!(split_word("fg   3 "), ("fg", "3"));
        assert_eq!(split_word("bg\tfind \"/srv\""), ("bg", "find \"/srv\""));
        assert_eq!(split_word("pom\u{f3}c  \u{15b}"), ("pom\u{f3}c", "\u{15b}"));
        assert_eq!(split_word(""), ("", ""));
    }

    #[test]
    fn plain_lines_keep_blank_lines_and_strip_only_line_endings() {
        let mut input: &[u8] = b"  a \r\n\n\xc5\x82\r";
        let mut lines = PlainLines::new(&mut input);
        assert_eq!(lines.read_line(), Input::Line("  a ".to_owned()));
        assert_eq!(lines.read_line(), Input::Line(String::new()));
        assert_eq!(lines.read_line(), Input::Line("\u{142}".to_owned()));
        assert_eq!(lines.read_line(), Input::End);
    }

    #[test]
    fn interrupt_is_not_raised_when_created_and_cannot_be_taken_twice() {
        let interrupt = Interrupt::default();
        assert!(!interrupt.is_raised());
        interrupt.raise();
        interrupt.raise();
        assert!(interrupt.take());
        assert!(!interrupt.take());
    }

    #[test]
    fn handler_is_installed_once_per_process() {
        let interrupt = Interrupt::new();
        assert!(interrupt.install_handler().is_ok());
        assert!(interrupt.install_handler().is_err());
    }

    #[test]
    fn completer_replaces_the_word_before_the_cursor() {
        let mut completer = Suggestions(vocabulary());
        let found = reedline::Completer::complete(&mut completer, "fi", 2);
        let values: Vec<String> = found
            .suggestions()
            .iter()
            .map(|s| s.value.clone())
            .collect();
        assert_eq!(values, vec!["find".to_owned()]);
    }

    #[test]
    fn highlighter_styles_every_part_of_the_line() {
        let highlighter = Coloured(vocabulary(), Theme::new(true));
        let styled = highlighter.highlight("find path: \"/srv\" | find", 0);
        assert_eq!(styled.raw_string(), "find path: \"/srv\" | find");
        assert!(styled.buffer.len() > 4);
    }

    #[test]
    fn editor_is_created_with_and_without_a_history_file() {
        let temp = tempfile::tempdir().unwrap();
        assert!(EditorLines::new(&vocabulary(), temp.path().join(HISTORY_FILE)).is_ok());
        let _ = EditorLines::without_history(&vocabulary());
        assert!(EditorLines::new(&vocabulary(), temp.path().to_path_buf()).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
