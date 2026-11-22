// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of a command by its table entry: flags are mapped to
//! parameters, operands fill the result template and the form is chosen by
//! the operands and flags given.

use nurt_contracts::ast::{Arg, Expr, MapEntry, Span, Stage};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::CorrectionClass;

use super::lex::{Quoting, Word, WordPart};
use super::session::{
    Piece, Session, Variants, append_args, is_absolute, is_glob, piece_text, trim_trailing_slash,
};
use super::syntax::{Located, Simple};
use super::table::{CommandEntry, OptionEntry, When};
use super::template::{
    Bindings, Bound, Conversion, fill_args, fill_pipeline, fill_stage, parse_pipeline,
};
use super::tree::is_identifier;
use super::{awk, find, pattern, produce, sed};

/// Flag that stands for a number written as a flag (`head -20`).
const NUMERIC_FLAG: &str = "-#";

/// Shortest prefix of a parameter name accepted for a long flag.
const MIN_PREFIX: usize = 3;

/// A flag given in the command.
#[derive(Debug, Clone)]
pub(crate) struct Given {
    pub(crate) index: usize,
    pub(crate) value: Option<Located<Word>>,
    pub(crate) span: Span,
}

/// Flags and operands of a command.
#[derive(Debug, Clone, Default)]
pub(crate) struct Parsed {
    pub(crate) given: Vec<Given>,
    pub(crate) operands: Vec<Located<Word>>,
    pub(crate) extra: Vec<Arg>,
    pub(crate) probable: bool,
}

impl Parsed {
    pub(crate) fn has(&self, entry: &CommandEntry, id: &str) -> bool {
        self.given
            .iter()
            .any(|given| entry.options[given.index].id == id)
    }

    pub(crate) fn value_of<'p>(
        &'p self,
        entry: &CommandEntry,
        id: &str,
    ) -> Option<&'p Located<Word>> {
        self.given
            .iter()
            .filter(|given| entry.options[given.index].id == id)
            .find_map(|given| given.value.as_ref())
    }
}

pub(crate) fn command(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Result<Variants, Error> {
    if let Some(problem) = &entry.error {
        let name = entry.words.join(" ");
        return Err(session
            .error(
                ErrorCode::Uncorrectable,
                problem,
                simple.span,
                &[("command", &name)],
            )
            .with_help(format!("help {}", entry.help)));
    }
    match entry.interpreter.as_deref() {
        Some("find") => find::translate(session, entry, simple, words, input),
        Some("grep") => pattern::grep(session, entry, simple, words, input),
        Some("sed") => sed::translate(session, entry, simple, words, input),
        Some("awk") => awk::translate(session, entry, simple, words, input),
        Some("echo" | "printf") => {
            Err(produce::content_without_file(session, entry, simple, words))
        }
        Some("test") => produce::test(session, entry, simple, words),
        Some("service") => service(session, simple, words, input),
        Some("at") => produce::at(session, entry, simple, words),
        _ => generic(session, entry, simple, words, input),
    }
}

/// Translates a command whose entry needs no interpreter. The number of
/// operands is checked only at the head of a pipeline; a stage takes its
/// input from the previous command.
pub(crate) fn generic(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Result<Variants, Error> {
    let parsed = parse_options(session, entry, words)?;
    if input.is_none() {
        check_count(session, entry, simple, &parsed)?;
    }
    let mut bindings = bind_operands(session, &parsed.operands, true)?;
    bind_values(session, entry, &parsed, &mut bindings)?;
    let directory = hook(session, entry, &parsed, &mut bindings)?;
    session.with_directory(directory, || {
        build(session, entry, simple, &parsed, &bindings, input)
    })
}

pub(crate) fn build(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    parsed: &Parsed,
    bindings: &Bindings,
    input: Option<&str>,
) -> Result<Variants, Error> {
    let mut bindings = bindings.clone();
    let choice = choose(session, entry, simple, parsed, &mut bindings, input)?;
    let mut stages = choice.stages;
    let apply_options = choice.options;
    let Some(target) = stages.len().checked_sub(1) else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "no_translation",
            simple.span,
            &[("command", &entry.words.join(" "))],
        ));
    };
    let mut class = if parsed.probable {
        CorrectionClass::Probable
    } else {
        choice.class
    };
    let mut appended = Vec::new();
    for given in parsed.given.iter().filter(|_| apply_options) {
        let option = &entry.options[given.index];
        if option.class == Some(CorrectionClass::Probable) {
            class = CorrectionClass::Probable;
        }
        if option.default {
            continue;
        }
        let mut local = bindings.clone();
        if let Some(value) = &given.value {
            local.set("value", vec![session.operand(value)?]);
        }
        append_args(
            &mut stages[target],
            fill_args(&option.args, &local, session)?,
        );
        for stage in &option.stages {
            appended.push(fill_stage(stage, &local, session, Some(simple.span))?);
        }
    }
    for option in entry
        .options
        .iter()
        .filter(|option| apply_options && !option.absent.is_empty())
    {
        let present = parsed
            .given
            .iter()
            .any(|given| entry.options[given.index].id == option.id);
        if !present {
            append_args(
                &mut stages[target],
                fill_args(&option.absent, &bindings, session)?,
            );
        }
    }
    append_args(&mut stages[target], parsed.extra.clone());
    stages.extend(appended);
    if let Some(stage) = stages.first_mut() {
        stage.span = Some(simple.span);
    }
    if class == CorrectionClass::Probable
        && let Some(key) = &choice.assumption
    {
        session.assume(&choice.rule, key);
    }
    let piece = Piece {
        stages,
        kind: choice.output,
        why: choice.assumption,
    };
    session.record(
        &choice.rule,
        class,
        simple.span,
        piece_text(&piece),
        "habit",
    );
    Ok(Variants(vec![piece]))
}

struct Choice {
    stages: Vec<Stage>,
    rule: String,
    class: CorrectionClass,
    output: String,
    assumption: Option<String>,
    options: bool,
}

/// Chooses the result template: the expansion of one unquoted pattern, the
/// stage form for the records of the previous command, the first form whose
/// condition holds, or the template of the entry.
fn choose(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    parsed: &Parsed,
    bindings: &mut Bindings,
    input: Option<&str>,
) -> Result<Choice, Error> {
    let span = Some(simple.span);
    let base = |stages, rule: &str, class, output: &str, assumption: Option<String>| Choice {
        stages,
        rule: rule.to_owned(),
        class,
        output: output.to_owned(),
        assumption,
        options: true,
    };
    let globs: Vec<&Located<Word>> = parsed
        .operands
        .iter()
        .filter(|word| is_glob(&word.value))
        .collect();
    if let Some(first) = globs.first() {
        let expandable = input.is_none() && globs.len() == 1;
        let Some(glob) = entry
            .glob
            .as_ref()
            .filter(|glob| expandable && glob.operands == parsed.operands.len())
        else {
            return Err(session.pattern_error(first));
        };
        let mut stages = expand_glob(session, entry, first, bindings, span)?;
        for stage in &glob.stages {
            stages.push(fill_stage(stage, bindings, session, span)?);
        }
        return Ok(base(
            stages,
            &glob.rule,
            entry.class,
            &entry.output,
            entry.assumption.clone(),
        ));
    }
    if let Some(kind) = input {
        let stage = entry
            .stages
            .iter()
            .find(|stage| {
                stage.input.iter().any(|accepted| accepted == kind)
                    && matches(&stage.when, entry, parsed, true, false)
            })
            .ok_or_else(|| {
                session.error(
                    ErrorCode::Uncorrectable,
                    "unsupported_stage",
                    simple.span,
                    &[("command", &entry.words.join(" "))],
                )
            })?;
        let stages = stage
            .template
            .iter()
            .map(|template| fill_stage(template, bindings, session, span))
            .collect::<Result<Vec<_>, _>>()?;
        let output = stage.output.clone().unwrap_or_else(|| kind.to_owned());
        let mut choice = base(
            stages,
            &entry.rule,
            entry.class,
            &output,
            entry.assumption.clone(),
        );
        choice.options = stage.options;
        return Ok(choice);
    }
    let followed = session.followed();
    if let Some(form) = entry
        .forms
        .iter()
        .find(|form| matches(&form.when, entry, parsed, false, followed))
    {
        let stages = fill_pipeline(&form.template, bindings, session, span)?;
        let rule = form.rule.clone().unwrap_or_else(|| entry.rule.clone());
        let output = form.output.clone().unwrap_or_else(|| entry.output.clone());
        let assumption = form.assumption.clone().or_else(|| entry.assumption.clone());
        return Ok(base(
            stages,
            &rule,
            form.class.unwrap_or(entry.class),
            &output,
            assumption,
        ));
    }
    let Some(template) = &entry.template else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "no_translation",
            simple.span,
            &[("command", &entry.words.join(" "))],
        ));
    };
    let stages = fill_pipeline(template, bindings, session, span)?;
    Ok(base(
        stages,
        &entry.rule,
        entry.class,
        &entry.output,
        entry.assumption.clone(),
    ))
}

fn matches(
    when: &When,
    entry: &CommandEntry,
    parsed: &Parsed,
    piped: bool,
    followed: bool,
) -> bool {
    let count = parsed.operands.len();
    let words_match = when.words.is_empty()
        || (when.words.len() == count
            && when
                .words
                .iter()
                .zip(&parsed.operands)
                .all(|(want, word)| Some(want) == word.value.literal().as_ref()));
    let dest_dir = count > 2
        || parsed
            .operands
            .last()
            .is_some_and(|word| word.value.raw.ends_with('/'));
    when.operands.is_none_or(|n| n == count)
        && when.min_operands.is_none_or(|n| count >= n)
        && when.dest_dir.is_none_or(|wanted| wanted == dest_dir)
        && when.piped.is_none_or(|wanted| wanted == piped)
        && when.followed.is_none_or(|wanted| wanted == followed)
        && words_match
        && when.options.iter().all(|id| parsed.has(entry, id))
        && !when.without.iter().any(|id| parsed.has(entry, id))
}

fn check_count(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    parsed: &Parsed,
) -> Result<(), Error> {
    let count = parsed.operands.len();
    let name = entry.words.join(" ");
    if count < entry.min {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            simple.span,
            &[("slot", "operand"), ("command", &name)],
        ));
    }
    if entry.max.is_some_and(|max| count > max) {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "too_many_operands",
            simple.span,
            &[("command", &name)],
        ));
    }
    Ok(())
}

/// Binds the operands to `$1` … and the operand groups. An unquoted pattern
/// is kept as written for the expansion of the entry when `allow_glob` is
/// set and is an error otherwise.
pub(crate) fn bind_operands(
    session: &Session<'_>,
    operands: &[Located<Word>],
    allow_glob: bool,
) -> Result<Bindings, Error> {
    let mut bindings = Bindings::default();
    let mut all = Vec::new();
    for (index, word) in operands.iter().enumerate() {
        let bound = if is_glob(&word.value) {
            if !allow_glob {
                return Err(session.pattern_error(word));
            }
            session.text_bound(word.value.raw.clone(), word.span)
        } else {
            session.operand(word)?
        };
        bindings.set(&(index + 1).to_string(), vec![bound.clone()]);
        all.push(bound);
    }
    bindings.set("all", all.clone());
    bindings.set("rest", all.iter().skip(1).cloned().collect());
    if let Some((last, init)) = all.split_last() {
        bindings.set("last", vec![last.clone()]);
        bindings.set("init", init.to_vec());
    }
    Ok(bindings)
}

fn bind_values(
    session: &Session<'_>,
    entry: &CommandEntry,
    parsed: &Parsed,
    bindings: &mut Bindings,
) -> Result<(), Error> {
    for given in &parsed.given {
        if let Some(value) = &given.value {
            bindings.push(&entry.options[given.index].id, session.operand(value)?);
        }
    }
    Ok(())
}

/// Parses flags and operands; flags may follow operands, `--` ends flags.
pub(crate) fn parse_options(
    session: &Session<'_>,
    entry: &CommandEntry,
    words: &[Located<Word>],
) -> Result<Parsed, Error> {
    let mut parsed = Parsed::default();
    let mut index = 0;
    let mut only_operands = false;
    while let Some(word) = words.get(index) {
        index += 1;
        let text = word
            .value
            .literal()
            .unwrap_or_else(|| word.value.raw.clone());
        if only_operands || !text.starts_with('-') || text == "-" {
            parsed.operands.push(word.clone());
            continue;
        }
        if text == "--" {
            only_operands = true;
            continue;
        }
        index = flag(session, entry, words, index, word, &text, &mut parsed)?;
    }
    Ok(parsed)
}

fn flag(
    session: &Session<'_>,
    entry: &CommandEntry,
    words: &[Located<Word>],
    next: usize,
    word: &Located<Word>,
    text: &str,
    parsed: &mut Parsed,
) -> Result<usize, Error> {
    if let Some(found) = find_option(entry, text) {
        return take(session, entry, words, next, word, found, None, parsed);
    }
    if let Some((name, value)) = text.split_once('=')
        && name.starts_with("--")
        && let Some(found) = find_option(entry, name)
    {
        let inline = located(value, word.span);
        return take(
            session,
            entry,
            words,
            next,
            word,
            found,
            Some(inline),
            parsed,
        );
    }
    let digits = &text[1..];
    if !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && let Some(found) = find_option(entry, NUMERIC_FLAG)
    {
        let inline = located(digits, word.span);
        return take(
            session,
            entry,
            words,
            next,
            word,
            found,
            Some(inline),
            parsed,
        );
    }
    if !text.starts_with("--") && text.chars().count() > 2 {
        return bundle(session, entry, words, next, word, text, parsed);
    }
    parsed.extra.push(long_flag(session, entry, word, text)?);
    Ok(next)
}

fn bundle(
    session: &Session<'_>,
    entry: &CommandEntry,
    words: &[Located<Word>],
    mut next: usize,
    word: &Located<Word>,
    text: &str,
    parsed: &mut Parsed,
) -> Result<usize, Error> {
    let letters: Vec<char> = text.chars().skip(1).collect();
    let mut position = 0;
    while let Some(&letter) = letters.get(position) {
        position += 1;
        let Some(found) = find_option(entry, &format!("-{letter}")) else {
            return Err(unknown_flag(session, entry, word, text));
        };
        if entry.options[found].value.is_some() {
            let rest: String = letters[position..].iter().collect();
            let inline = (!rest.is_empty()).then(|| located(&rest, word.span));
            return take(session, entry, words, next, word, found, inline, parsed);
        }
        next = take(session, entry, words, next, word, found, None, parsed)?;
    }
    Ok(next)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the flag parser threads its whole state"
)]
fn take(
    session: &Session<'_>,
    entry: &CommandEntry,
    words: &[Located<Word>],
    next: usize,
    word: &Located<Word>,
    index: usize,
    inline: Option<Located<Word>>,
    parsed: &mut Parsed,
) -> Result<usize, Error> {
    let option: &OptionEntry = &entry.options[index];
    if option.value.is_none() {
        parsed.given.push(Given {
            index,
            value: None,
            span: word.span,
        });
        return Ok(next);
    }
    if let Some(value) = inline {
        parsed.given.push(Given {
            index,
            value: Some(value),
            span: word.span,
        });
        return Ok(next);
    }
    let Some(value) = words.get(next) else {
        let flag = word.value.raw.clone();
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "flag_without_value",
            word.span,
            &[("flag", &flag)],
        ));
    };
    parsed.given.push(Given {
        index,
        value: Some(value.clone()),
        span: word.span,
    });
    Ok(next + 1)
}

fn find_option(entry: &CommandEntry, flag: &str) -> Option<usize> {
    entry
        .options
        .iter()
        .position(|option| option.flags.iter().any(|known| known == flag))
}

pub(crate) fn located(text: &str, span: Span) -> Located<Word> {
    Located {
        value: Word {
            raw: text.to_owned(),
            parts: vec![WordPart::Literal {
                text: text.to_owned(),
                quoting: Quoting::Single,
            }],
        },
        span,
    }
}

/// Matches a long flag to a parameter of the target command by name after
/// case folding, `kebab-case` to `snake_case` and a unique prefix.
fn long_flag(
    session: &Session<'_>,
    entry: &CommandEntry,
    word: &Located<Word>,
    text: &str,
) -> Result<Arg, Error> {
    let Some(rest) = text.strip_prefix("--") else {
        return Err(unknown_flag(session, entry, word, text));
    };
    let (name, value) = rest
        .split_once('=')
        .map_or((rest, None), |(n, v)| (n, Some(v)));
    let wanted = name.replace('-', "_").to_ascii_lowercase();
    let params = session
        .table
        .params
        .get(&entry.target)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let exact = params.iter().find(|param| **param == wanted);
    let prefixed: Vec<&String> = params
        .iter()
        .filter(|param| wanted.len() >= MIN_PREFIX && param.starts_with(&wanted))
        .collect();
    let param = match (exact, prefixed.as_slice()) {
        (Some(param), _) => param.clone(),
        (None, [param]) => (*param).clone(),
        _ => return Err(unknown_flag(session, entry, word, text)),
    };
    let span = Some(word.span);
    let value = match value {
        None => Expr::Bool { v: true, span },
        Some("true" | "false") => Expr::Bool {
            v: value == Some("true"),
            span,
        },
        Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            match digits.parse() {
                Ok(v) => Expr::Int { v, span },
                Err(_) => Expr::Text {
                    v: digits.to_owned(),
                    span,
                },
            }
        }
        Some(other) if is_identifier(other) => Expr::Ident {
            v: other.to_owned(),
            span,
        },
        Some(other) => Expr::Text {
            v: other.to_owned(),
            span,
        },
    };
    let arg = Arg {
        name: Some(param.clone()),
        value,
        span,
    };
    session.record(
        "B27",
        CorrectionClass::Certain,
        word.span,
        format!("{param}:"),
        "param",
    );
    Ok(arg)
}

fn unknown_flag(
    session: &Session<'_>,
    entry: &CommandEntry,
    word: &Located<Word>,
    text: &str,
) -> Error {
    let params = session
        .table
        .params
        .get(&entry.target)
        .cloned()
        .unwrap_or_default();
    let list = params.join(", ");
    session
        .error(
            ErrorCode::Uncorrectable,
            "flag_without_equivalent",
            word.span,
            &[
                ("flag", text),
                ("command", &entry.words.join(" ")),
                ("target", &entry.target),
                ("params", &list),
            ],
        )
        .with_expected(serde_json::json!(params))
        .with_help(format!("help {}", entry.help))
}

/// Splits an operand with a shell pattern into the directory before the
/// first pattern segment and the pattern relative to it.
fn expand_glob(
    session: &Session<'_>,
    entry: &CommandEntry,
    word: &Located<Word>,
    bindings: &mut Bindings,
    span: Option<Span>,
) -> Result<Vec<Stage>, Error> {
    let (text, first_glob) = glob_text(&word.value);
    let cut = text[..first_glob].rfind('/');
    let (directory, pattern) = match cut {
        Some(0) => ("/".to_owned(), text[1..].to_owned()),
        Some(index) => (text[..index].to_owned(), text[index + 1..].to_owned()),
        None => (".".to_owned(), text.clone()),
    };
    if pattern.contains("**") || pattern.is_empty() {
        return Err(session.pattern_error(word));
    }
    let depth = i64::try_from(pattern.matches('/').count() + 1).unwrap_or(i64::MAX);
    bindings.set(
        "dir",
        vec![Bound::Word {
            text: directory,
            span: word.span,
            bare: word.value.is_bare(),
        }],
    );
    bindings.set(
        "pattern",
        vec![Bound::Word {
            text: pattern.clone(),
            span: word.span,
            bare: false,
        }],
    );
    bindings.set(
        "depth",
        vec![Bound::Node(Expr::Int {
            v: depth,
            span: None,
        })],
    );
    let Some(source) = &session.table.glob_source else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "no_translation",
            word.span,
            &[("command", &entry.words.join(" "))],
        ));
    };
    let mut stages = fill_pipeline(source, bindings, session, span)?;
    let hidden = pattern.split('/').any(|segment| segment.starts_with('.'));
    if !hidden && let Some(first) = stages.first_mut() {
        append_args(
            first,
            fill_args(&session.table.glob_hidden, bindings, session)?,
        );
    }
    Ok(stages)
}

/// Text of a word with quoted pattern characters escaped as `[*]`, and the
/// byte offset of the first unquoted pattern character.
fn glob_text(word: &Word) -> (String, usize) {
    let mut text = String::new();
    let mut first = None;
    for part in &word.parts {
        let WordPart::Literal {
            text: piece,
            quoting,
        } = part
        else {
            continue;
        };
        for c in piece.chars() {
            match (quoting, c) {
                (Quoting::None, '*' | '?' | '[') => {
                    first.get_or_insert(text.len());
                    text.push(c);
                }
                (_, '*' | '?' | '[' | ']' | '{' | '}') => {
                    text.push('[');
                    text.push(c);
                    text.push(']');
                }
                _ => text.push(c),
            }
        }
    }
    let first = first.unwrap_or(text.len());
    (text, first)
}

/// Fills a template kept in `[values.<map>]` of the table.
pub(crate) fn value_template(
    session: &Session<'_>,
    map: &str,
    key: &str,
    bindings: &Bindings,
    span: Span,
) -> Result<Vec<Stage>, Error> {
    let Some(text) = session.table.value_str(map, key) else {
        return Err(session.error(
            ErrorCode::InternalCorrection,
            "invalid_table",
            span,
            &[("reason", key)],
        ));
    };
    let template = parse_pipeline(text).map_err(|error| {
        session.error(
            ErrorCode::InternalCorrection,
            "invalid_table",
            span,
            &[("reason", &error.0)],
        )
    })?;
    fill_pipeline(&template, bindings, session, Some(span))
}

/// Extra bindings of entries that interpret an operand or a flag value;
/// returns a working directory for relative operands when the command sets
/// one (`tar -C`).
fn hook(
    session: &Session<'_>,
    entry: &CommandEntry,
    parsed: &Parsed,
    bindings: &mut Bindings,
) -> Result<Option<String>, Error> {
    let directory = session.directory();
    match entry.interpreter.as_deref() {
        Some("chown") => {
            chown(parsed, bindings);
            Ok(directory)
        }
        Some("archive") => archive(session, entry, parsed, bindings),
        Some("curl") => {
            headers(session, entry, parsed, bindings)?;
            if let Some(url) = parsed
                .operands
                .first()
                .and_then(|word| word.value.literal())
            {
                let file = url
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let span = parsed.operands[0].span;
                bindings.set(
                    "file",
                    vec![Bound::Word {
                        text: file,
                        span,
                        bare: false,
                    }],
                );
            }
            Ok(directory)
        }
        _ => Ok(directory),
    }
}

fn chown(parsed: &Parsed, bindings: &mut Bindings) {
    let Some(first) = parsed.operands.first() else {
        return;
    };
    let text = first.value.literal().unwrap_or_default();
    let (account, group) = text
        .split_once([':', '.'])
        .map_or((text.as_str(), ""), |(a, g)| (a, g));
    if !account.is_empty() {
        bindings.set(
            "account",
            vec![Bound::Word {
                text: account.to_owned(),
                span: first.span,
                bare: false,
            }],
        );
    }
    if !group.is_empty() {
        bindings.set(
            "group",
            vec![Bound::Word {
                text: group.to_owned(),
                span: first.span,
                bare: false,
            }],
        );
    }
}

/// Archive commands: the archive path is resolved against the working
/// directory, sources against `-C` when given; `base` keeps the names that
/// the shell command would store.
fn archive(
    session: &Session<'_>,
    entry: &CommandEntry,
    parsed: &Parsed,
    bindings: &mut Bindings,
) -> Result<Option<String>, Error> {
    if let Some(word) = parsed.value_of(entry, "f") {
        let path = session_path(session, &session.operand(word)?)?;
        bindings.set("archive", vec![Bound::Node(path)]);
    }
    let explicit = match parsed.value_of(entry, "C") {
        Some(word) => match session_path(session, &session.operand(word)?)? {
            Expr::Text { v, .. } => Some(v),
            _ => None,
        },
        None => None,
    };
    let directory = explicit.or_else(|| session.directory());
    if let Some(directory) = &directory {
        bindings.set(
            "into",
            vec![Bound::Node(Expr::Text {
                v: directory.clone(),
                span: None,
            })],
        );
    }
    let format = [("z", "tar_gz"), ("j", "tar_bz2"), ("J", "tar_xz")]
        .iter()
        .find(|(id, _)| parsed.has(entry, id))
        .map_or("tar", |(_, format)| *format);
    bindings.set(
        "format",
        vec![Bound::Node(Expr::Ident {
            v: format.to_owned(),
            span: None,
        })],
    );
    let zip = entry.words[0] == "zip";
    let sources = if zip {
        parsed.operands.get(1..).unwrap_or_default()
    } else {
        parsed.operands.as_slice()
    };
    let all_absolute = !sources.is_empty()
        && sources
            .iter()
            .all(|word| word.value.literal().as_deref().is_some_and(is_absolute));
    if let Some(base) = directory
        .clone()
        .or_else(|| all_absolute.then(|| "/".to_owned()))
    {
        bindings.set(
            "base",
            vec![Bound::Node(Expr::Text {
                v: trim_trailing_slash(&base),
                span: None,
            })],
        );
    }
    let extracting = (entry.words[0] == "tar" && parsed.has(entry, "x"))
        || (entry.words[0] == "unzip" && !parsed.has(entry, "d"));
    if extracting && directory.is_none() {
        let span = parsed.operands.first().map_or_else(
            || parsed.given.first().map_or(EMPTY, |g| g.span),
            |word| word.span,
        );
        return Err(session.error(ErrorCode::Uncorrectable, "working_directory", span, &[]));
    }
    Ok(directory)
}

const EMPTY: Span = Span {
    line: 1,
    column: 1,
    end_line: 1,
    end_column: 1,
};

fn session_path(session: &Session<'_>, bound: &Bound) -> Result<Expr, Error> {
    use super::template::Convert;
    session.convert(bound, Conversion::Path)
}

fn headers(
    session: &Session<'_>,
    entry: &CommandEntry,
    parsed: &Parsed,
    bindings: &mut Bindings,
) -> Result<(), Error> {
    let mut entries = Vec::new();
    for given in parsed
        .given
        .iter()
        .filter(|given| entry.options[given.index].id == "H")
    {
        let Some(word) = &given.value else { continue };
        let text = word.value.literal().unwrap_or_default();
        let Some((name, value)) = text.split_once(':') else {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "invalid_value",
                word.span,
                &[("value", &text)],
            ));
        };
        entries.push(MapEntry {
            k: name.trim().to_owned(),
            v: Expr::Text {
                v: value.trim().to_owned(),
                span: Some(word.span),
            },
            span: None,
        });
    }
    if !entries.is_empty() {
        bindings.set(
            "headers",
            vec![Bound::Node(Expr::Map {
                entries,
                span: None,
            })],
        );
    }
    Ok(())
}

/// `service NAME ACTION` is read as `systemctl ACTION NAME`.
fn service(
    session: &Session<'_>,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Result<Variants, Error> {
    let [name, action] = words else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            simple.span,
            &[("slot", "action"), ("command", "service")],
        ));
    };
    let action_text = action.value.literal().unwrap_or_default();
    let key = vec!["systemctl".to_owned(), action_text.clone()];
    let Some(entry) = session
        .table
        .command(&key)
        .filter(|entry| entry.words.len() == 2)
    else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "no_translation",
            simple.span,
            &[("command", &format!("service {action_text}"))],
        ));
    };
    generic(session, entry, simple, std::slice::from_ref(name), input)
}

/// Translates `git [-C DIR] SUBCOMMAND …` into the built-in `git.*`
/// command; the repository comes from `-C` or `cd` in the same line and is
/// never guessed.
pub(crate) fn git(
    session: &Session<'_>,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Result<Variants, Error> {
    let mut rest = &words[1..];
    let mut repository = session.directory();
    if rest.first().is_some_and(|word| word.value.raw == "-C") {
        let Some(directory) = rest.get(1) else {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "flag_without_value",
                simple.span,
                &[("flag", "-C")],
            ));
        };
        let bound = session.operand(directory)?;
        if let Expr::Text { v, .. } = session_path(session, &bound)? {
            repository = Some(v);
        }
        rest = &rest[2..];
    }
    let without_repository = rest
        .first()
        .and_then(|word| word.value.literal())
        .is_some_and(|sub| {
            session
                .table
                .value("git_without_repository", &sub)
                .is_some()
        });
    let repository = match repository {
        Some(repository) => repository,
        None if without_repository => String::new(),
        None => return Err(git_without_repository(session, simple, words, input)),
    };
    let mut names = vec!["git".to_owned()];
    names.extend(
        rest.iter()
            .map(|word| word.value.literal().unwrap_or_default()),
    );
    let entry = session
        .table
        .command(&names)
        .filter(|entry| entry.words.len() > 1);
    let directory = (!repository.is_empty())
        .then(|| repository.clone())
        .or_else(|| session.directory());
    session.with_directory(directory, || match entry {
        Some(entry) => git_entry(
            session,
            entry,
            simple,
            &rest[entry.words.len() - 1..],
            input,
            &repository,
        ),
        None => {
            let mut program = vec![words[0].clone()];
            program.extend_from_slice(rest);
            session.run_program(simple, &program, Some(repository.clone()))
        }
    })
}

fn git_entry(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
    repository: &str,
) -> Result<Variants, Error> {
    let parsed = parse_options(session, entry, words)?;
    check_count(session, entry, simple, &parsed)?;
    let mut bindings = bind_operands(session, &parsed.operands, true)?;
    bind_values(session, entry, &parsed, &mut bindings)?;
    bindings.set(
        "repo",
        vec![Bound::Node(Expr::Text {
            v: repository.to_owned(),
            span: None,
        })],
    );
    if entry.words.get(1).is_some_and(|sub| sub == "commit") {
        commit_message(session, entry, &parsed, &mut bindings)?;
    }
    build(session, entry, simple, &parsed, &bindings, input)
}

/// Joins the paragraphs of several `-m` flags with an empty line, as git
/// does.
fn commit_message(
    session: &Session<'_>,
    entry: &CommandEntry,
    parsed: &Parsed,
    bindings: &mut Bindings,
) -> Result<(), Error> {
    let mut paragraphs = Vec::new();
    let mut span = None;
    for given in parsed
        .given
        .iter()
        .filter(|given| entry.options[given.index].id == "m")
    {
        let Some(word) = &given.value else { continue };
        let Some(text) = word.value.literal() else {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "variable_in_text",
                word.span,
                &[("name", "message"), ("lower", "message")],
            ));
        };
        span.get_or_insert(word.span);
        if !text.is_empty() {
            paragraphs.push(text);
        }
    }
    if let Some(span) = span {
        bindings.set(
            "message",
            vec![Bound::Node(Expr::Text {
                v: paragraphs.join("\n\n"),
                span: Some(span),
            })],
        );
    }
    Ok(())
}

fn git_without_repository(
    session: &Session<'_>,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Error {
    let mut error = session.error(
        ErrorCode::Uncorrectable,
        "repository_directory",
        simple.span,
        &[],
    );
    for candidate in session.directory_candidates(words) {
        let quiet = Session::new(session.table, session.context, "");
        let text = quiet.with_directory(Some(candidate), || {
            git(&quiet, simple, words, input)
                .ok()
                .map(|variants| quiet.program_text(&variants))
        });
        if let Some(text) = text {
            error = error.with_fix(text);
        }
    }
    error
}
