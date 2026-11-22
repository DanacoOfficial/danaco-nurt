// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Handlers of the subcommands and of the program shortcut.

use nurt_contracts::ast::AstDocument;
use nurt_contracts::envelope::{OutputFormat, Request, RequestOptions};
use nurt_contracts::kinds::SyntaxVariant;
use serde_json::Value as Json;

use crate::EngineSource;
use crate::app::{CliError, Context, Io};
use crate::args::{self, AST_FLAGS, CHECK_FLAGS, EXEC_FLAGS, FMT_FLAGS, Flags, SCRIPT_FLAGS};
use crate::doctor;
use crate::exit;
use crate::integrations;
use crate::output;
use crate::repl;
use crate::text;

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
    std::fs::read_to_string(path)
        .map_err(|error| CliError::Failure(format!("cannot read `{path}`: {error}")))
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

fn check(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, CHECK_FLAGS, true)?;
    let text = source_text(&flags, &words, io)?;
    let context = Context::open(io.terminal, source)?;
    let request = program_request(text, &flags);
    let report = context.engine.check(&request)?;
    if flags.format == Some(OutputFormat::Json) {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|error| CliError::Failure(format!("cannot encode the result: {error}")))?;
        writeln!(io.out, "{json}")?;
    } else {
        writeln!(io.out, "{}", context.out_style.success("ok"))?;
        writeln!(io.out, "corrections: {}", report.corrections.len())?;
        for (key, value) in &report.summary {
            writeln!(io.out, "{key}: {value}")?;
        }
    }
    Ok(exit::SUCCESS)
}

fn fmt(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let (flags, words) = args::parse_flags(args, FMT_FLAGS, true)?;
    let path = single("fmt", "one file", &words)?;
    let text = read_text(&path, io)?;
    let context = Context::open(io.terminal, source)?;
    let tree = context.engine.parse(&text, flags.variant)?;
    let printed = context.engine.print(&tree, flags.variant)?;
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
    let context = Context::open(io.terminal, source)?;
    let tree = read_tree(&text, from, &context)?;
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
            let mut bytes = Vec::new();
            ciborium::into_writer(&tree, &mut bytes)
                .map_err(|error| CliError::Failure(format!("cannot encode the tree: {error}")))?;
            io.out.write_all(&bytes)?;
        }
    }
    Ok(exit::SUCCESS)
}

fn schema(args: &[String], io: &mut Io<'_>, source: &dyn EngineSource) -> Result<u8, CliError> {
    let name = single(
        "schema",
        "a name: ast, bound, report, request, config or tool",
        args,
    )?;
    let context = Context::open(io.terminal, source)?;
    let schema = context.engine.schema(&name)?;
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
