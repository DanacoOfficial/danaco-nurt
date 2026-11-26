// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution environment of `jq` filters.
//!
//! Filters reach the world only through the execution context: `env` and
//! `$ENV` read the environment of the host, `now` reads the context clock,
//! `debug` and `stderr` emit log events. Generators that can run without end
//! (`range`, `repeat`, `recurse`, `while`, `until`) pass a checkpoint that
//! stops the filter when the execution is cancelled or its time limit
//! elapses; a stopped filter cannot be resumed by `try`.

use std::cell::RefCell;

use jaq_core::box_iter::box_once;
use jaq_core::data::HasLut;
use jaq_core::load::parse::Def;
use jaq_core::native::{Fun, v};
use jaq_core::{DataT, Exn, Lut, Native, ValXs};
use jaq_json::{Map, Val};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::{Event, ExecCtx, LogEvent, LogLevel};

use super::guard;

/// Generator steps between two checks of cancellation and the deadline.
const CHECK_INTERVAL: u64 = 256;

/// Calls of definitions that one run may make. A recursion that the evaluator
/// runs without growing the stack, such as `def f: f; f`, is not seen by the
/// stack measurement and ends only at this count.
const CALL_LIMIT: u64 = 1_000_000;

/// Exit code of the halt used internally to stop a filter at a checkpoint.
const STOP_EXIT_CODE: i32 = i32::MIN;

/// Source of the log events of `debug` and `stderr`.
const LOG_SOURCE: &str = "query";

/// Seconds per nanosecond of the context clock.
const SECONDS_PER_NANOSECOND: f64 = 1e-9;

/// Definitions of the core library replaced by checkpointed versions, as
/// name and number of arguments.
const REPLACED_DEFS: [(&str, usize); 6] = [
    ("repeat", 1),
    ("recurse", 1),
    ("recurse", 0),
    ("recurse", 2),
    ("while", 2),
    ("until", 2),
];

/// Native filters of the libraries replaced by filters of this module.
const REPLACED_FUNS: [&str; 6] = [
    "range",
    "env",
    "now",
    "debug_empty",
    "stderr_empty",
    "decode_base64",
];

/// Checkpointed generators; `repeat` follows `jq` 1.8, which applies `f` to
/// the same input again and again.
const CHECKED_DEFS: &str = r"
def repeat(f): def rec: __checkpoint | f, rec; rec;
def recurse(f): def rec: __checkpoint | ., (f | rec); rec;
def recurse: recurse(.[]?);
def recurse(f; cond): recurse(f | select(cond));
def while(cond; update): def rec: __checkpoint | if cond then ., (update | rec) else empty end; rec;
def until(cond; update): def rec: __checkpoint | if cond then . else update | rec end; rec;
";

/// Definitions of `jq` that the libraries lack or define differently,
/// written as in the `jq` builtin library.
const JQ_DEFS: &str = r#"
def IN(s): . as $x | first((s | select(. == $x) | true), false);
def IN(src; s): first((src | IN(s) | select(.)), false);
def INDEX(stream; idx_expr): reduce stream as $row ({}; . + {($row | idx_expr | tostring): $row});
def INDEX(idx_expr): INDEX(.[]; idx_expr);
def tostream: path(def r: (.[]? | r), .; r) as $p | getpath($p) | reduce path(.[]?) as $q ([$p, .]; [$p + $q]);
def fromstream(i): {x: null, e: false} as $init |
  foreach i as $i ($init
  ; if .e then $init else . end
  | if $i | length == 2
    then setpath(["e"]; $i[0] | length == 0) | setpath(["x"] + $i[0]; $i[1])
    else setpath(["e"]; $i[0] | length == 1) end
  ; if .e then .x else empty end);
def delpaths($paths): reduce ($paths | sort | reverse | .[]) as $path (.; getpath($path) |= empty);
def del(f): delpaths([path(f)]);
def from_entries: reduce .[] as $x ({};
  . + {($x | if .key == null then .k // .name // .Name // .K // .Key else .key end
          | if isstring then . else tojson end):
       ($x | if has("value") then .value else .v end)});
def @csv:
  if isarray | not then error("\(type) (\(tojson)) cannot be csv-formatted, only an array can be") end
  | map(if isstring then "\"" + (split("\"") | join("\"\"")) + "\""
        elif . == null then ""
        elif isboolean or isnumber then tojson
        else error("\(type) (\(tojson)) is not valid in a csv row") end)
  | join(",");
def @tsv:
  if isarray | not then error("\(type) (\(tojson)) cannot be tsv-formatted, only an array can be") end
  | map(if isstring then split("\\") | join("\\\\") | split("\t") | join("\\t")
          | split("\n") | join("\\n") | split("\r") | join("\\r")
        elif . == null then ""
        elif isboolean or isnumber then tojson
        else error("\(type) (\(tojson)) is not valid in a tsv row") end)
  | join("\t");
"#;

/// Name of the global variable holding the host environment.
pub(crate) const ENV_VAR: &str = "$ENV";

/// Data type of filters run by this crate.
pub(crate) struct Doc;

impl DataT for Doc {
    type V<'a> = Val;
    type Data<'a> = &'a Runtime<'a>;
}

/// Per-run state reachable from native filters.
pub(crate) struct Runtime<'a> {
    lut: &'a Lut<Doc>,
    ctx: &'a ExecCtx,
    /// Generator steps allowed in one run, from `limits.max_records`.
    max_steps: u64,
    /// Lowest stack address a guarded filter may reach; 0 for a run that is
    /// not guarded.
    floor: usize,
    state: RefCell<RunState>,
}

#[derive(Default)]
struct RunState {
    steps: u64,
    calls: u64,
    stopped: Option<Error>,
    buffered: Option<Vec<LogEvent>>,
    last_stderr: Option<String>,
}

impl<'a> HasLut<'a, Doc> for &'a Runtime<'a> {
    fn lut(&self) -> &'a Lut<Doc> {
        self.lut
    }
}

impl<'a> Runtime<'a> {
    /// Runtime of one run; log events are held back while `buffered`. A
    /// `floor` other than 0 is the lowest stack address that the filter may
    /// reach before a guard stops it.
    pub(crate) fn new(lut: &'a Lut<Doc>, ctx: &'a ExecCtx, buffered: bool, floor: usize) -> Self {
        let state = RunState {
            buffered: buffered.then(Vec::new),
            ..RunState::default()
        };
        Self {
            lut,
            ctx,
            max_steps: ctx.limits().max_records,
            floor,
            state: RefCell::new(state),
        }
    }

    /// Error of a checkpoint that stopped the filter.
    pub(crate) fn take_stop(&self) -> Option<Error> {
        self.state.borrow_mut().stopped.take()
    }

    /// Text last written by `stderr`, used for the message of `halt_error`.
    pub(crate) fn last_stderr(&self) -> Option<String> {
        self.state.borrow().last_stderr.clone()
    }

    /// Emits the log events held back during the run.
    pub(crate) fn release_logs(&self) {
        let events = self.state.borrow_mut().buffered.take().unwrap_or_default();
        for event in events {
            self.ctx.emit(Event::Log(event));
        }
    }

    /// Environment of the host as an object, the value of `$ENV`.
    pub(crate) fn environment(&self) -> Val {
        let mut map = Map::default();
        for (name, value) in self.ctx.host().env_vars() {
            map.insert(
                Val::from(name.to_string_lossy().into_owned()),
                Val::from(value.to_string_lossy().into_owned()),
            );
        }
        Val::obj(map)
    }

    /// Counts a generator step and checks the context at every
    /// [`CHECK_INTERVAL`] steps; a failed check stops the filter with a halt
    /// that `try` does not catch.
    fn step(&self) -> Result<(), Exn<'a, Val>> {
        let due = {
            let mut state = self.state.borrow_mut();
            state.steps += 1;
            if state.stopped.is_none() && state.steps > self.max_steps {
                state.stopped = Some(Error::new(
                    ErrorCode::Limit,
                    format!(
                        "the filter generated more than {} values; narrow the filter",
                        self.max_steps
                    ),
                ));
            }
            state.stopped.is_some() || state.steps.is_multiple_of(CHECK_INTERVAL)
        };
        if !due {
            return Ok(());
        }
        let stopped = self.state.borrow().stopped.is_some();
        if !stopped {
            if let Err(error) = self.ctx.checkpoint() {
                self.state.borrow_mut().stopped = Some(error);
            } else {
                return Ok(());
            }
        }
        Err(Exn::halt(STOP_EXIT_CODE))
    }

    /// Guards the call of a definition of the filter: stops the filter, as
    /// [`Runtime::step`] does, when the stack in use reaches the floor of the
    /// run or when the calls reach [`CALL_LIMIT`], and checks the context at
    /// every [`CHECK_INTERVAL`] calls. Calls
    /// are not counted against the steps, which bound the values a filter
    /// generates.
    fn guard(&self) -> Result<(), Exn<'a, Val>> {
        let mut state = self.state.borrow_mut();
        if state.stopped.is_none() {
            if self.floor != 0 && guard::stack_address() < self.floor {
                state.stopped = Some(Error::new(
                    ErrorCode::Limit,
                    "the filter recurses deeper than the stack of the run allows; give the recursion an end",
                ));
            } else if state.calls >= CALL_LIMIT {
                state.stopped = Some(Error::new(
                    ErrorCode::Limit,
                    format!(
                        "the filter recurses without end: more than {CALL_LIMIT} calls of definitions; give the recursion an end"
                    ),
                ));
            } else {
                state.calls += 1;
                if state.calls.is_multiple_of(CHECK_INTERVAL)
                    && let Err(error) = self.ctx.checkpoint()
                {
                    state.stopped = Some(error);
                }
            }
        }
        if state.stopped.is_some() {
            Err(Exn::halt(STOP_EXIT_CODE))
        } else {
            Ok(())
        }
    }

    fn log(&self, level: LogLevel, message: String) {
        let event = LogEvent {
            level,
            message,
            source: Some(LOG_SOURCE.to_owned()),
        };
        let mut state = self.state.borrow_mut();
        if level == LogLevel::Info {
            state.last_stderr = Some(event.message.clone());
        }
        match state.buffered.as_mut() {
            Some(buffer) => buffer.push(event),
            None => {
                drop(state);
                self.ctx.emit(Event::Log(event));
            }
        }
    }

    /// Seconds since the Unix epoch on the context clock.
    fn now(&self) -> Val {
        #[expect(
            clippy::cast_precision_loss,
            reason = "jq represents time as a floating point number of seconds"
        )]
        let nanoseconds = self.ctx.clock().now().0 as f64;
        Val::from(nanoseconds * SECONDS_PER_NANOSECOND)
    }
}

/// Returns `true` for the exit code used by checkpoints.
pub(crate) const fn is_stop_code(code: i32) -> bool {
    code == STOP_EXIT_CODE
}

/// Definitions available to every filter: the libraries of `jq` with the
/// generators replaced by checkpointed versions, followed by the
/// definitions of `jq` that the libraries lack, which take precedence.
///
/// Returns `None` when the definitions of this module do not parse, which is
/// a defect of this crate.
pub(crate) fn prelude() -> Option<Vec<Def<&'static str>>> {
    let compatible = jaq_core::load::parse(JQ_DEFS, |parser| parser.defs())?;
    let checked = jaq_core::load::parse(CHECKED_DEFS, |parser| parser.defs())?;
    let mut defs = Vec::new();
    let mut checked = Some(checked);
    for def in jaq_core::defs() {
        if REPLACED_DEFS.contains(&(def.name, def.args.len())) {
            defs.extend(checked.take().unwrap_or_default());
        } else {
            defs.push(def);
        }
    }
    defs.extend(checked.unwrap_or_default());
    defs.extend(jaq_std::defs());
    defs.extend(jaq_json::defs());
    defs.extend(compatible);
    Some(defs)
}

/// Native filters available to every filter.
pub(crate) fn funs() -> Vec<Fun<Doc>> {
    let library = jaq_core::funs::<Doc>()
        .chain(jaq_std::funs::<Doc>())
        .chain(jaq_json::funs::<Doc>())
        .filter(|(name, _, _)| !REPLACED_FUNS.contains(name));
    let checkpoint = Native::<Doc>::new(|cv| pass(cv.0.data(), cv.1))
        .with_paths(|cv| match cv.0.data().step() {
            Ok(()) => box_once(Ok(cv.1)),
            Err(exn) => box_once(Err(exn)),
        })
        .with_update(|cv, update| match cv.0.data().step() {
            Ok(()) => update(cv.1),
            Err(exn) => box_once(Err(exn)),
        });
    let guard_filter = Native::<Doc>::new(|cv| pass_guarded(cv.0.data(), cv.1))
        .with_paths(|cv| match cv.0.data().guard() {
            Ok(()) => box_once(Ok(cv.1)),
            Err(exn) => box_once(Err(exn)),
        })
        .with_update(|cv, update| match cv.0.data().guard() {
            Ok(()) => update(cv.1),
            Err(exn) => box_once(Err(exn)),
        });
    let own: [Fun<Doc>; 8] = [
        (
            "decode_base64",
            v(0),
            Native::new(|cv| box_once(decode_base64(&cv.1).map_err(Exn::from))),
        ),
        (
            "range",
            v(3),
            Native::new(|mut cv| {
                let by = cv.0.pop_var();
                let to = cv.0.pop_var();
                let from = cv.0.pop_var();
                range(cv.0.data(), from, to, by)
            }),
        ),
        (
            "env",
            v(0),
            Native::new(|cv| box_once(Ok(cv.0.data().environment()))),
        ),
        (
            "now",
            v(0),
            Native::new(|cv| box_once(Ok(cv.0.data().now()))),
        ),
        (
            "debug_empty",
            v(0),
            Native::new(|cv| {
                cv.0.data()
                    .log(LogLevel::Debug, format!("[\"DEBUG:\",{}]", cv.1));
                Box::new(std::iter::empty())
            }),
        ),
        (
            "stderr_empty",
            v(0),
            Native::new(|cv| {
                cv.0.data().log(LogLevel::Info, stderr_text(&cv.1));
                Box::new(std::iter::empty())
            }),
        ),
        ("__checkpoint", v(0), checkpoint),
        (guard::GUARD_NAME, v(0), guard_filter),
    ];
    library.chain(own).collect()
}

/// `@base64d` as `jq` 1.8 decodes it: padding ends the text, a last group of
/// two or three characters gives one or two bytes, and bytes that are not
/// UTF-8 become the replacement character.
fn decode_base64(input: &Val) -> Result<Val, jaq_core::Error<Val>> {
    let Val::TStr(text) = input else {
        return Err(jaq_core::Error::str(format!(
            "{input} cannot be base64-decoded, only a string can be"
        )));
    };
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3 + 2);
    let mut group = 0_u32;
    let mut count = 0_u8;
    for &byte in text.iter().take_while(|byte| **byte != b'=') {
        let sextet = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => {
                return Err(jaq_core::Error::str(format!(
                    "string ({input}) is not valid base64 data"
                )));
            }
        };
        group = group << 6 | u32::from(sextet);
        count += 1;
        if count == 4 {
            bytes.extend_from_slice(&group.to_be_bytes()[1..]);
            group = 0;
            count = 0;
        }
    }
    match count {
        0 => {}
        1 => {
            return Err(jaq_core::Error::str(format!(
                "string ({input}) trailing base64 byte found"
            )));
        }
        2 => bytes.push((group >> 4).to_be_bytes()[3]),
        _ => bytes.extend_from_slice(&(group >> 2).to_be_bytes()[2..]),
    }
    Ok(Val::from(String::from_utf8_lossy(&bytes).into_owned()))
}

fn pass<'a>(runtime: &&'a Runtime<'a>, value: Val) -> ValXs<'a, Val> {
    match runtime.step() {
        Ok(()) => box_once(Ok(value)),
        Err(exn) => box_once(Err(exn)),
    }
}

fn pass_guarded<'a>(runtime: &&'a Runtime<'a>, value: Val) -> ValXs<'a, Val> {
    match runtime.guard() {
        Ok(()) => box_once(Ok(value)),
        Err(exn) => box_once(Err(exn)),
    }
}

/// `stderr` writes text values without quotes, as `jq` does.
fn stderr_text(value: &Val) -> String {
    match value {
        Val::TStr(text) => String::from_utf8_lossy(text).into_owned(),
        other => other.to_string(),
    }
}

/// `range($from; $to; $by)` with the semantics of `jq`: the sequence stops
/// before `$to`; with a step of zero it is endless unless `$from == $to`.
fn range<'a>(runtime: &&'a Runtime<'a>, from: Val, to: Val, by: Val) -> ValXs<'a, Val> {
    let runtime: &'a Runtime<'a> = runtime;
    if [&from, &to, &by]
        .iter()
        .any(|bound| !matches!(bound, Val::Num(_)))
    {
        return box_once(Err(Exn::from(jaq_core::Error::<Val>::str(
            "Range bounds must be numeric",
        ))));
    }
    let direction = by.cmp(&Val::from(0_usize));
    let mut next = Some(from);
    Box::new(std::iter::from_fn(move || {
        let current = next.take()?;
        let more = match direction {
            std::cmp::Ordering::Greater => current < to,
            std::cmp::Ordering::Less => current > to,
            std::cmp::Ordering::Equal => current != to,
        };
        if !more {
            return None;
        }
        if let Err(exn) = runtime.step() {
            return Some(Err(exn));
        }
        match current.clone() + by.clone() {
            Ok(following) => next = Some(following),
            Err(error) => return Some(Err(Exn::from(error))),
        }
        Some(Ok(current))
    }))
}
