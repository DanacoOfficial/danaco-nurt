// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::commands
// Responsibility:   Handlers of the subcommands and of the program shortcut.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Handlers of the subcommands and of the program shortcut.

use nurt_contracts::ast::AstDocument;
use nurt_contracts::envelope::{OutputFormat, Request, RequestOptions};
use nurt_contracts::kinds::SyntaxVariant;
use serde_json::Value as Json;

use crate::EngineSource;
use crate::app::{CliError, Context, Io};
use crate::args::{
    self, AST_FLAGS, CHECK_FLAGS, EXEC_FLAGS, FMT_FLAGS, Flags, RUN_FLAGS, SCHEMA_FLAGS,
    SCRIPT_FLAGS,
};
use crate::doctor;
use crate::exit;
use crate::integrations;
use crate::output;
use crate::repl;
use crate::text;
use crate::tool_definition::{self, ToolOptions};

/// Runs the subcommand `name` with the arguments that follow it.
///
/// # Errors
///
/// Returns the reason the command did not complete.
pub fn run_subcommand(
    name: &str,
    args: &[String],
    io: &mut Io<'_>,
    source: &dyn EngineSource,
) -> Result<u8, CliError> {
    match name {
        "exec" => exec(args, io, source),
        "script" => script(args, io, source),
        "run" => run_file(args, io, source),
        "repl" => repl::run(args, io, source),
        "check" => check(args, io, source),
        "fmt" => fmt(args, io, source),
        "ast" => ast(args, io, source),
        "schema" => schema(args, io, source),
        "catalog" => catalog(args, io, source),
        "help" => help(args, io, source),
        "history" | "undo" | "redo" => journal_shortcut(name, args, io, source),
        "doctor" => doctor::run(args, io, source),
        "version" => version(args, io),
        other => integrations::run(other, args),
    }
}

fn no_arguments(command: &str, args: &[String]) -> Result<(), CliError> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(CliError::Usage(format!("`{command}` takes no arguments")))
    }
}

fn version(args: &[String], io: &mut Io<'_>) -> Result<u8, CliError> {
    no_arguments("version", args)?;
    writeln!(io.out, "{}", text::full_version())?;
    Ok(exit::SUCCESS)
}

fn options_of(flags: &Flags) -> RequestOptions {
    RequestOptions {
        preview: flags.preview.then_some(true),
        format: flags.format,
        limit: flags.limit,
        verbosity: flags.verbosity,
        privilege: flags.privilege,
        as_user: flags.as_user.clone(),
        ..RequestOptions::default()
    }
}

fn program_request(text: String, flags: &Flags) -> Request {
    let mut request = Request::program(text);
    request.variant = flags.variant;
    request.with_options(options_of(flags))
}

fn exec_request(flags: &Flags, words: &[String]) -> Result<Request, CliError> {
    match (&flags.cursor, words.is_empty()) {
        (Some(cursor), true) => Ok(Request::cursor(cursor.clone()).with_options(options_of(flags))),
        (Some(_), false) => Err(CliError::Usage(
            "the option `--cursor` cannot be combined with a program".to_owned(),
        )),
        (None, true) => Err(CliError::Usage("missing program".to_owned())),
        (None, false) => Ok(program_request(words.join(" "), flags)),
    }
}

/// Runs a program given as the arguments: `nurt exec` and the shortcut
/// `nurt <program>`.
///
/// # Errors
///
/// Returns the reason the command did not complete.
pub fn exec(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, EXEC_FLAGS, false)?;
    let request = exec_request(&flags, &words)?;
    run_request(&request, flags.format, io, source)
}

fn run_request(
    request: &Request,
    format: Option<OutputFormat>,
    io: &mut Io<'_>,
    source: &dyn EngineSource,
) -> Result<u8, CliError> {
    let context = Context::open(io.terminal, source)?;
    let response = context.engine.run(request);
    let text = output::render(&response, format.unwrap_or_default(), context.out_style);
    writeln!(io.out, "{text}")?;
    Ok(exit::for_report(&response.report))
}

fn single(command: &str, what: &str, words: &[String]) -> Result<String, CliError> {
    match words {
        [one] => Ok(one.clone()),
        _ => Err(CliError::Usage(format!("`{command}` needs {what}"))),
    }
}

fn read_text(path: &str, io: &mut Io<'_>) -> Result<String, CliError> {
    if path == "-" {
        let mut text = String::new();
        io.input
            .read_to_string(&mut text)
            .map_err(|error| CliError::Failure(format!("cannot read the input: {error}")))?;
        return Ok(text);
    }
    let unreadable = |reason: String| CliError::Failure(format!("cannot read `{path}`: {reason}"));
    let bytes = std::fs::read(path).map_err(|error| unreadable(error.to_string()))?;
    text::decode_file(&bytes).map_err(unreadable)
}

/// Replaces a leading `#!` line with an empty line, so that the positions of
/// the program stay the same.
fn without_shebang(text: &str) -> &str {
    if text.starts_with("#!") {
        text.find('\n').map_or("", |end| &text[end..])
    } else {
        text
    }
}

fn script(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, SCRIPT_FLAGS, true)?;
    let path = single("script", "one script file", &words)?;
    let text = read_text(&path, io)?;
    let mut request = program_request(without_shebang(&text).to_owned(), &flags);
    request.options.script = Some(true);
    run_request(&request, flags.format, io, source)
}

/// Whether `--help` or `-h` stands as an option of its own: not as the value of
/// a parameter written `name: --help`.
fn asks_for_help(words: &[String]) -> bool {
    words.iter().enumerate().any(|(index, word)| {
        (word == "--help" || word == "-h")
            && !index
                .checked_sub(1)
                .and_then(|before| words.get(before))
                .is_some_and(|before| before.ends_with(':'))
    })
}

/// Runs a program file with the values of its parameters:
/// `nurt run cleanup.nurt dir: "/srv/app" days: 7 dry`. Without `params` in
/// the file the program takes no values. `--help` after the file prints the
/// declaration of the parameters.
fn run_file(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, RUN_FLAGS, false)?;
    let Some((path, rest)) = words.split_first() else {
        return Err(CliError::Usage("`run` needs a program file".to_owned()));
    };
    let text = read_text(path, io)?;
    let text = without_shebang(&text).to_owned();
    if asks_for_help(rest) {
        let context = Context::open(io.terminal, source)?;
        let document = context.engine.parse(&text, flags.variant)?;
        let declaration = crate::params::declaration(&context.engine, &document, flags.variant)?;
        writeln!(
            io.out,
            "{}",
            crate::params::help(path, &document, &declaration)
        )?;
        return Ok(exit::SUCCESS);
    }
    let mut request = program_request(text, &flags);
    request.params = crate::params::from_words(rest)?;
    run_request(&request, flags.format, io, source)
}

fn source_text(flags: &Flags, words: &[String], io: &mut Io<'_>) -> Result<String, CliError> {
    match (&flags.program, words) {
        (Some(program), []) => Ok(program.clone()),
        (None, _) => {
            let path = single("check", "a file or the option `-e`", words)?;
            read_text(&path, io)
        }
        (Some(_), _) => Err(CliError::Usage(
            "`check` takes a file or the option `-e`, not both".to_owned(),
        )),
    }
}

/// Turns the outcome of a command with the explicit format `json` into a
/// document: a language error becomes `{"error": {...}}`, with
/// `"diagnostics": []` in front for `check`, written to the standard output
/// with the exit code the error has on the standard error output. Other
/// failures and every other format keep their form.
fn json_errors(
    outcome: Result<u8, CliError>,
    format: Option<OutputFormat>,
    diagnostics: bool,
    io: &mut Io<'_>,
) -> Result<u8, CliError> {
    let Err(CliError::Language(error)) = outcome else {
        return outcome;
    };
    if format != Some(OutputFormat::Json) {
        return Err(CliError::Language(error));
    }
    let document = if diagnostics {
        serde_json::json!({ "diagnostics": [], "error": error })
    } else {
        serde_json::json!({ "error": error })
    };
    let json = serde_json::to_string_pretty(&document)
        .map_err(|error| CliError::Failure(format!("cannot encode the result: {error}")))?;
    writeln!(io.out, "{json}")?;
    Ok(exit::for_error_code(error.code))
}

fn check(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, CHECK_FLAGS, true)?;
    let text = source_text(&flags, &words, io)?;
    let outcome = check_program(&flags, text, io, source);
    json_errors(outcome, flags.format, true, io)
}

fn check_program(
    flags: &Flags,
    text: String,
    io: &mut Io<'_>,
    source: &dyn EngineSource,
) -> Result<u8, CliError> {
    let context = Context::open(io.terminal, source)?;
    let request = program_request(text, flags);
    let report = context.engine.check(&request)?;
    if flags.format == Some(OutputFormat::Json) {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|error| CliError::Failure(format!("cannot encode the result: {error}")))?;
        writeln!(io.out, "{json}")?;
    } else {
        writeln!(io.out, "{}", context.out_style.success("ok"))?;
        writeln!(io.out, "corrections: {}", report.summary.corrections.len())?;
        let summary = serde_json::to_value(&report.summary)
            .map_err(|error| CliError::Failure(format!("cannot encode the result: {error}")))?;
        for (key, value) in summary.as_object().into_iter().flatten() {
            if key == "corrections" {
                continue;
            }
            match value.as_str() {
                Some(text) => writeln!(io.out, "{key}: {text}")?,
                None => writeln!(io.out, "{key}: {value}")?,
            }
        }
    }
    Ok(exit::SUCCESS)
}

fn fmt(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, FMT_FLAGS, true)?;
    let path = single("fmt", "one file", &words)?;
    let text = read_text(&path, io)?;
    let outcome = format_program(&flags, &text, io, source);
    json_errors(outcome, flags.format, false, io)
}

fn format_program(
    flags: &Flags,
    text: &str,
    io: &mut Io<'_>,
    source: &dyn EngineSource,
) -> Result<u8, CliError> {
    let context = Context::open(io.terminal, source)?;
    // The text is read in the requested variant; a text that is not in it is
    // read in the configured variant and printed in the requested one.
    let printed = match context.engine.format(text, flags.variant, flags.variant) {
        Err(first) if flags.variant.is_some() => context
            .engine
            .format(text, None, flags.variant)
            .map_err(|_| first)?,
        other => other?,
    };
    writeln!(io.out, "{}", printed.trim_end())?;
    Ok(exit::SUCCESS)
}

/// Notation of the input or the output of `ast`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notation {
    Variant(SyntaxVariant),
    Json,
    Cbor,
}

fn notation(value: Option<&str>, default: Notation, input: bool) -> Result<Notation, CliError> {
    match value {
        None => Ok(default),
        Some("json") => Ok(Notation::Json),
        Some("cbor") if !input => Ok(Notation::Cbor),
        Some(other) => args::variant(other).map(Notation::Variant).map_err(|_| {
            let accepted = if input {
                "a, b, c or json"
            } else {
                "json, cbor, a, b or c"
            };
            CliError::Usage(format!("`{other}` is not a notation; expected {accepted}"))
        }),
    }
}

fn read_tree(text: &str, from: Notation, context: &Context) -> Result<AstDocument, CliError> {
    match from {
        Notation::Variant(variant) => Ok(context.engine.parse(text, Some(variant))?),
        _ => serde_json::from_str(text)
            .map_err(|error| CliError::Failure(format!("the input is not a syntax tree: {error}"))),
    }
}

fn ast(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, AST_FLAGS, true)?;
    let from = notation(flags.from.as_deref(), Notation::Json, true)?;
    let to = notation(flags.to.as_deref(), Notation::Json, false)?;
    let input = single("ast", "one input", &words)?;
    let text = if input == "-" {
        read_text(&input, io)?
    } else {
        input
    };
    let outcome = convert_tree(&text, from, to, io, source);
    json_errors(outcome, flags.format, false, io)
}

fn convert_tree(
    text: &str,
    from: Notation,
    to: Notation,
    io: &mut Io<'_>,
    source: &dyn EngineSource,
) -> Result<u8, CliError> {
    let context = Context::open(io.terminal, source)?;
    let tree = read_tree(text, from, &context)?;
    match to {
        Notation::Variant(variant) => {
            let printed = context.engine.print(&tree, Some(variant))?;
            writeln!(io.out, "{}", printed.trim_end())?;
        }
        Notation::Json => {
            let json = serde_json::to_string_pretty(&tree)
                .map_err(|error| CliError::Failure(format!("cannot encode the tree: {error}")))?;
            writeln!(io.out, "{json}")?;
        }
        Notation::Cbor => {
            if io.terminal.stdout {
                return Err(CliError::Usage(
                    "the CBOR output is binary; redirect it to a file or a pipe".to_owned(),
                ));
            }
            let mut bytes = Vec::new();
            ciborium::into_writer(&tree, &mut bytes)
                .map_err(|error| CliError::Failure(format!("cannot encode the tree: {error}")))?;
            io.out.write_all(&bytes)?;
        }
    }
    Ok(exit::SUCCESS)
}

fn schema(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    if let [name, options @ ..] = args
        && name == "tool"
        && !options.is_empty()
        && !selects_error_format_only(options)
    {
        let options = ToolOptions::parse(options)?;
        let context = Context::open(io.terminal, source)?;
        let definition = tool_definition::render(&context.engine, &options)?;
        writeln!(io.out, "{definition}")?;
        return Ok(exit::SUCCESS);
    }
    let (flags, words) = args::parse_flags(args, SCHEMA_FLAGS, true)?;
    let name = single(
        "schema",
        "a name: ast, bound, report, request, config or tool",
        &words,
    )?;
    let outcome = print_schema(&name, io, source);
    json_errors(outcome, flags.format, false, io)
}

/// Whether the options after `schema tool` only choose the form of an error
/// (`--json`, `--format json` or `--format text`); any other option asks for
/// a rendered tool definition, whose `--format` names the definition format.
fn selects_error_format_only(options: &[String]) -> bool {
    let mut words = options.iter().map(String::as_str);
    while let Some(word) = words.next() {
        let value = match word.split_once('=') {
            Some(("--format", value)) => value,
            _ if word == "--json" => continue,
            _ if word == "--format" => match words.next() {
                Some(value) => value,
                None => return false,
            },
            _ => return false,
        };
        if value != "json" && value != "text" {
            return false;
        }
    }
    true
}

fn print_schema(name: &str, io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let context = Context::open(io.terminal, source)?;
    let schema = context.engine.schema(name)?;
    writeln!(io.out, "{}", schema.trim_end())?;
    Ok(exit::SUCCESS)
}

/// The entry of a catalog document that describes `name`: a command, or a
/// group listing it.
fn catalog_entry(catalog: &Json, name: &str) -> Option<Json> {
    catalog
        .get("commands")?
        .as_array()?
        .iter()
        .find(|entry| {
            entry.get("name").and_then(Json::as_str) == Some(name)
                || entry.get("group").and_then(Json::as_str) == Some(name)
        })
        .cloned()
}

fn catalog(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    if args.first().map(String::as_str) == Some("check-distance") {
        return Err(integrations::unavailable("catalog check-distance"));
    }
    if args.len() > 1 {
        return Err(CliError::Usage(
            "`catalog` takes at most one command or group".to_owned(),
        ));
    }
    let context = Context::open(io.terminal, source)?;
    let document = context.engine.catalog()?;
    let shown = match args.first() {
        Some(name) => catalog_entry(&document, name)
            .ok_or_else(|| CliError::Failure(format!("the catalog has no entry `{name}`")))?,
        None => document,
    };
    let json = serde_json::to_string_pretty(&shown)
        .map_err(|error| CliError::Failure(format!("cannot encode the catalog: {error}")))?;
    writeln!(io.out, "{json}")?;
    Ok(exit::SUCCESS)
}

fn help(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    if args.is_empty() {
        writeln!(io.out, "{}", text::help_intro())?;
        return Ok(exit::SUCCESS);
    }
    let context = Context::open(io.terminal, source)?;
    let topic = context.engine.help(&args.join(" "), None)?;
    writeln!(io.out, "{}", topic.trim_end())?;
    Ok(exit::SUCCESS)
}

fn journal_shortcut(
    name: &str,
    args: &[String],
    io: &mut Io<'_>,
    source: &dyn EngineSource,
) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, EXEC_FLAGS, false)?;
    let mut parts = vec![name.to_owned()];
    parts.extend(words);
    let request = program_request(parts.join(" "), &flags);
    run_request(&request, flags.format, io, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shebang_line_is_replaced_by_an_empty_line() {
        assert_eq!(
            without_shebang("#!/usr/bin/env nurt\nfind \"/srv\"\n"),
            "\nfind \"/srv\"\n"
        );
        assert_eq!(without_shebang("#!/usr/bin/env nurt"), "");
        assert_eq!(without_shebang("find \"/srv\""), "find \"/srv\"");
    }

    #[test]
    fn catalog_entries_are_found_by_command_or_group() {
        let document = serde_json::json!({"commands": [
            {"name": "find", "group": "files"},
            {"name": "remove", "group": "files"}
        ]});
        assert_eq!(
            catalog_entry(&document, "remove").and_then(|entry| entry.get("name").cloned()),
            Some(Json::from("remove"))
        );
        assert!(catalog_entry(&document, "files").is_some());
        assert!(catalog_entry(&document, "copy").is_none());
        assert!(catalog_entry(&Json::Null, "find").is_none());
    }

    #[test]
    fn notations_accept_cbor_only_as_output() {
        let default = Notation::Json;
        assert_eq!(
            notation(Some("cbor"), default, false).unwrap(),
            Notation::Cbor
        );
        assert!(notation(Some("cbor"), default, true).is_err());
        assert_eq!(
            notation(Some("b"), default, true).unwrap(),
            Notation::Variant(SyntaxVariant::B)
        );
        assert_eq!(notation(None, default, true).unwrap(), Notation::Json);
    }

    /// Runs `body` with streams that read `input` and discard the output.
    fn with_io<T>(input: &[u8], body: impl FnOnce(&mut Io<'_>) -> T) -> T {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut input = input;
        let mut io = Io {
            out: &mut out,
            err: &mut err,
            input: &mut input,
            terminal: crate::app::Terminal::default(),
        };
        body(&mut io)
    }

    #[test]
    fn request_options_are_taken_from_the_flags() {
        let none = options_of(&Flags::default());
        assert_eq!(none.preview, None);
        assert_eq!(
            (none.format, none.limit, none.verbosity),
            (None, None, None)
        );
        let flags = Flags {
            preview: true,
            format: Some(OutputFormat::Compact),
            limit: Some(7),
            verbosity: Some(nurt_contracts::envelope::Verbosity::Quiet),
            ..Flags::default()
        };
        let options = options_of(&flags);
        assert_eq!(options.preview, Some(true));
        assert_eq!(options.format, Some(OutputFormat::Compact));
        assert_eq!(options.limit, Some(7));
        assert_eq!(
            options.verbosity,
            Some(nurt_contracts::envelope::Verbosity::Quiet)
        );
    }

    #[test]
    fn program_request_carries_the_text_the_variant_and_the_options() {
        let flags = Flags {
            variant: Some(SyntaxVariant::B),
            limit: Some(3),
            ..Flags::default()
        };
        let request = program_request("sort(size)".to_owned(), &flags);
        assert_eq!(request.program.as_deref(), Some("sort(size)"));
        assert_eq!(request.variant, Some(SyntaxVariant::B));
        assert_eq!(request.options.limit, Some(3));
        assert_eq!(request.cursor, None);
    }

    #[test]
    fn program_words_are_joined_with_single_spaces() {
        let words = ["find".to_owned(), "\"/srv\"".to_owned(), "glob:".to_owned()];
        let request = exec_request(&Flags::default(), &words).unwrap();
        assert_eq!(request.program.as_deref(), Some("find \"/srv\" glob:"));
    }

    #[test]
    fn cursor_request_carries_the_options_and_no_program() {
        let flags = Flags {
            cursor: Some("c_7".to_owned()),
            limit: Some(2),
            ..Flags::default()
        };
        let request = exec_request(&flags, &[]).unwrap();
        assert_eq!(request.cursor.as_deref(), Some("c_7"));
        assert_eq!(request.program, None);
        assert_eq!(request.options.limit, Some(2));
    }

    #[test]
    fn exactly_one_word_is_accepted_where_a_single_operand_is_needed() {
        assert_eq!(
            single("fmt", "one file", &["a.nurt".to_owned()]),
            Ok("a.nurt".to_owned())
        );
        for words in [vec![], vec!["a".to_owned(), "b".to_owned()]] {
            assert_eq!(
                single("fmt", "one file", &words),
                Err(CliError::Usage("`fmt` needs one file".to_owned()))
            );
        }
    }

    #[test]
    fn program_text_and_a_file_exclude_each_other() {
        let given = Flags {
            program: Some("count".to_owned()),
            ..Flags::default()
        };
        let with_file = with_io(b"", |io| source_text(&given, &["a.nurt".to_owned()], io));
        assert!(matches!(with_file, Err(CliError::Usage(_))));
        assert_eq!(
            with_io(b"", |io| source_text(&given, &[], io)),
            Ok("count".to_owned())
        );
        let neither = with_io(b"", |io| source_text(&Flags::default(), &[], io));
        assert!(matches!(neither, Err(CliError::Usage(_))));
    }

    #[test]
    fn standard_input_is_read_as_text() {
        let text = "find \"/\u{142}\"\r\n";
        assert_eq!(
            with_io(text.as_bytes(), |io| read_text("-", io)),
            Ok(text.to_owned())
        );
        let invalid = with_io(b"\xff\xfe", |io| read_text("-", io));
        assert!(matches!(
            invalid,
            Err(CliError::Failure(message)) if message.starts_with("cannot read the input")
        ));
    }

    #[test]
    fn a_missing_file_is_a_failure_naming_the_path() {
        let missing = with_io(b"", |io| read_text("/nonexistent/nurt/program.nurt", io));
        assert!(matches!(
            missing,
            Err(CliError::Failure(message))
                if message.starts_with("cannot read `/nonexistent/nurt/program.nurt`")
        ));
    }

    #[test]
    fn shebang_is_removed_up_to_the_line_feed_only_at_the_start() {
        assert_eq!(without_shebang("#!/usr/bin/env nurt\r\ncount"), "\ncount");
        assert_eq!(without_shebang("#!"), "");
        assert_eq!(
            without_shebang(" #!/bin/nurt\ncount"),
            " #!/bin/nurt\ncount"
        );
        assert_eq!(without_shebang(""), "");
    }

    #[test]
    fn notations_name_what_is_accepted() {
        let default = Notation::Json;
        assert!(matches!(
            notation(Some("yaml"), default, false),
            Err(CliError::Usage(message)) if message.contains("json, cbor, a, b or c")
        ));
        assert!(matches!(
            notation(Some("cbor"), default, true),
            Err(CliError::Usage(message)) if message.contains("a, b, c or json")
        ));
        assert_eq!(
            notation(Some("c"), Notation::Cbor, false).unwrap(),
            Notation::Variant(SyntaxVariant::C)
        );
        assert_eq!(
            notation(None, Notation::Cbor, false).unwrap(),
            Notation::Cbor
        );
    }

    #[test]
    fn catalog_entries_match_the_first_command_or_group() {
        let document = serde_json::json!({"commands": [
            {"name": "find", "group": "files"},
            {"name": "files", "group": "other"}
        ]});
        let found = catalog_entry(&document, "files").unwrap();
        assert_eq!(found["name"], "find");
        assert!(catalog_entry(&serde_json::json!({"commands": 3}), "find").is_none());
        assert!(catalog_entry(&serde_json::json!({}), "find").is_none());
    }

    #[test]
    fn cursor_cannot_be_combined_with_a_program() {
        let flags = Flags {
            cursor: Some("c_1".to_owned()),
            ..Flags::default()
        };
        assert!(exec_request(&flags, &["find".to_owned()]).is_err());
        assert!(exec_request(&flags, &[]).is_ok());
        assert!(exec_request(&Flags::default(), &[]).is_err());
    }
}

#[cfg(test)]
mod help_option_tests {
    use super::asks_for_help;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn help_is_an_option_not_a_value() {
        assert!(asks_for_help(&words(&["--help"])));
        assert!(asks_for_help(&words(&["dir:", "/x", "-h"])));
        assert!(!asks_for_help(&words(&["msg:", "--help"])));
        assert!(!asks_for_help(&words(&["dir:", "/x"])));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
