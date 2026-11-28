// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::jq::guard
// Responsibility:   Protection of the process against filters that recurse without end.
// Dependencies:     jaq_core
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Protection of the process against filters that recurse without end.
//!
//! The library that runs `jq` filters evaluates a call of a filter defined by
//! the user on the stack of the calling thread, to a depth that follows the
//! recursion of the filter, and a filter such as `def f: 1 + f; f` ends the
//! process with a stack overflow that no code can catch. A filter that
//! defines a filter of its own therefore runs on a thread with a stack of
//! known size, and every definition of the filter gets a call of the native
//! filter `__guard` at its start, which measures the stack in use and stops
//! the filter with `E_LIMIT` before the stack is used up.

use std::ptr;
use std::thread;

use jaq_core::load::lex::{Lexer, StrPart, Tok, Token};

/// Name of the native filter that measures the stack.
pub(crate) const GUARD_NAME: &str = "__guard";

/// Text inserted after the colon of every definition of the filter.
const GUARD_CALL: &str = " __guard |";

/// Size of the stack of the thread that runs a guarded filter.
const STACK_BYTES: usize = 1280 * 1024 * 1024;

/// Stack that a guarded filter may use; the rest is left for the frames
/// between two guards and for the release of the structures of the filter
/// when it stops.
const STACK_BUDGET: usize = 1024 * 1024 * 1024;

/// Stack that a filter may use on the calling thread when the thread of a
/// guarded filter cannot be started: the part of the stack of any thread of
/// the process that is surely free.
const FALLBACK_BUDGET: usize = 256 * 1024;

/// Address of a variable on the stack of the calling thread.
pub(crate) fn stack_address() -> usize {
    let marker = 0_u8;
    ptr::from_ref(&marker).addr()
}

/// Lowest stack address a guarded filter may reach when its run starts at the
/// address `start` and may use `budget` bytes.
fn floor_below(start: usize, budget: usize) -> usize {
    start.saturating_sub(budget)
}

/// Runs `work` on a thread with a large stack and returns its result.
///
/// `work` receives the lowest stack address that a guarded filter may reach.
/// When the thread cannot be started, `work` runs on the calling thread with
/// a budget that a stack of any size covers.
pub(crate) fn on_large_stack<R: Send>(work: impl Fn(usize) -> R + Sync) -> R {
    let started = thread::scope(|scope| {
        thread::Builder::new()
            .name("nurt-jq".to_owned())
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, || work(floor_below(stack_address(), STACK_BUDGET)))
            .map(|handle| handle.join())
    });
    match started {
        Ok(Ok(result)) => result,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => work(floor_below(stack_address(), FALLBACK_BUDGET)),
    }
}

/// The filter `expression` with a call of `__guard` at the start of every
/// definition; `None` when the filter defines nothing or does not lex.
pub(crate) fn guarded(expression: &str) -> Option<String> {
    let tokens = Lexer::new(expression).lex().ok()?;
    let mut offsets = Vec::new();
    collect(expression, &tokens, &mut offsets);
    if offsets.is_empty() {
        return None;
    }
    offsets.sort_unstable();
    let mut source = String::with_capacity(expression.len() + offsets.len() * GUARD_CALL.len());
    let mut from = 0;
    for offset in offsets {
        source.push_str(&expression[from..offset]);
        source.push_str(GUARD_CALL);
        from = offset;
    }
    source.push_str(&expression[from..]);
    Some(source)
}

/// Offsets after the colon of every definition in `tokens`, nested blocks and
/// interpolated strings included.
fn collect(code: &str, tokens: &[Token<&str>], offsets: &mut Vec<usize>) {
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token("def", Tok::Word) => {
                if let Some(colon) = definition_colon(&tokens[index + 1..]) {
                    offsets.push(colon.as_ptr().addr() - code.as_ptr().addr() + colon.len());
                }
            }
            Token(_, Tok::Block(inner)) => collect(code, inner, offsets),
            Token(_, Tok::Str(parts)) => {
                for part in parts {
                    if let StrPart::Term(term) = part {
                        collect(code, std::slice::from_ref(term), offsets);
                    }
                }
            }
            Token(..) => {}
        }
    }
}

/// The colon that follows the name and the optional parameters of a
/// definition, given the tokens after `def`.
fn definition_colon<'a>(rest: &[Token<&'a str>]) -> Option<&'a str> {
    let Some(Token(_, Tok::Word)) = rest.first() else {
        return None;
    };
    let mut position = 1;
    if let Some(Token(text, Tok::Block(_))) = rest.get(position)
        && text.starts_with('(')
    {
        position += 1;
    }
    match rest.get(position) {
        Some(Token(colon, Tok::Sym)) if *colon == ":" => Some(*colon),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_definition_gets_a_guard() {
        assert_eq!(
            guarded("def f: 1 + f; f").as_deref(),
            Some("def f: __guard | 1 + f; f")
        );
        assert_eq!(
            guarded("def f(g; $x): g; def h: 2; f(.; 1) | h").as_deref(),
            Some("def f(g; $x): __guard | g; def h: __guard | 2; f(.; 1) | h")
        );
        assert_eq!(
            guarded("[.[] | def inner: def deeper: 1; deeper; inner]").as_deref(),
            Some("[.[] | def inner: __guard | def deeper: __guard | 1; deeper; inner]")
        );
        assert_eq!(
            guarded(r#""\(def f: 1; f)""#).as_deref(),
            Some(r#""\(def f: __guard | 1; f)""#)
        );
    }

    #[test]
    fn filters_without_definitions_are_left_alone() {
        for expression in [
            ".a | map(.b)",
            "{def: 1} | .def",
            "\"def f: 1;\"",
            ". # def f: 1;",
        ] {
            assert_eq!(guarded(expression), None, "{expression}");
        }
        assert_eq!(guarded("def ("), None);
    }

    #[test]
    fn the_floor_lies_below_the_start_of_the_run() {
        assert_eq!(floor_below(1000, 400), 600);
        assert_eq!(floor_below(100, 400), 0);
    }

    #[test]
    fn work_runs_on_a_stack_of_the_known_size() {
        let (start, floor) = on_large_stack(|floor| (stack_address(), floor));
        let room = start - floor;
        assert!(room <= STACK_BUDGET, "{room}");
        assert!(room > STACK_BUDGET - 64 * 1024, "{room}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
