// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Approximate call hierarchy: the calls written in the body of each
//! function.
//!
//! A call is attributed to the innermost function or method whose
//! definition contains it and is named by the last identifier of its callee
//! expression. Calls are matched to functions by name only, so a call of a
//! method of another type with the same name is counted as well.

use nurt_contracts::services::{SymbolInfo, SymbolKind, TextSpan};

use super::{LocalAnalysis, advance, rules};
use crate::registry::{ParsedSource, to_u64};

/// Call written in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// Name of the called function.
    pub callee: String,
    /// Region of the callee name.
    pub span: TextSpan,
    /// Index of the calling function among the symbols; `None` for a call
    /// outside any function.
    pub caller: Option<usize>,
}

pub(super) fn collect(parsed: &ParsedSource<'_>, symbols: &[SymbolInfo]) -> Vec<CallSite> {
    let callers = Callers::new(symbols);
    let mut cursor = parsed.tree.root_node().walk();
    let mut calls = Vec::new();
    loop {
        let node = cursor.node();
        if rules::is_call(node)
            && let Some(name) = rules::callee_name(node)
        {
            let start = to_u64(node.start_byte());
            let text = parsed.text(name);
            calls.push(CallSite {
                callee: text.strip_prefix('$').unwrap_or(&text).to_owned(),
                span: parsed.span(name),
                caller: callers.of(start),
            });
        }
        if cursor.goto_first_child() {
            continue;
        }
        if !advance(&mut cursor) {
            break;
        }
    }
    calls
}

/// Functions and methods of a file, searched by offset.
///
/// The symbols are sorted by start and nest like syntax nodes, so the last
/// function that starts at or before an offset either contains it or has the
/// innermost containing function among its ancestors.
struct Callers<'s> {
    symbols: &'s [SymbolInfo],
    /// Indexes of the functions and methods among the symbols.
    callable: Vec<usize>,
    /// Position in `callable` of the enclosing function of each function.
    parent: Vec<Option<usize>>,
}

impl<'s> Callers<'s> {
    fn new(symbols: &'s [SymbolInfo]) -> Self {
        let callable: Vec<usize> = symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method))
            .map(|(index, _)| index)
            .collect();
        let mut parent = Vec::with_capacity(callable.len());
        let mut open: Vec<usize> = Vec::new();
        for (position, index) in callable.iter().enumerate() {
            let bytes = symbols[*index].span.bytes;
            while let Some(top) = open.last() {
                let outer = symbols[callable[*top]].span.bytes;
                if outer.start <= bytes.start && bytes.end <= outer.end {
                    break;
                }
                open.pop();
            }
            parent.push(open.last().copied());
            open.push(position);
        }
        Self {
            symbols,
            callable,
            parent,
        }
    }

    /// Index of the innermost function whose definition contains `offset`;
    /// of several with the same span, the first.
    fn of(&self, offset: u64) -> Option<usize> {
        let upper = self
            .callable
            .partition_point(|index| self.symbols[*index].span.bytes.start <= offset);
        let mut current = upper.checked_sub(1);
        while let Some(position) = current {
            let bytes = self.symbols[self.callable[position]].span.bytes;
            if offset < bytes.end {
                let mut found = position;
                while let Some(outer) = self.parent[found] {
                    if self.symbols[self.callable[outer]].span.bytes != bytes {
                        break;
                    }
                    found = outer;
                }
                return Some(self.callable[found]);
            }
            current = self.parent[position];
        }
        None
    }
}

impl LocalAnalysis<'_> {
    /// Calls made in the body of the symbol with index `symbol`, nested
    /// functions excluded, in source order.
    ///
    /// ```
    /// use nurt_code::local::analyze;
    /// use nurt_code::registry::Registry;
    /// use nurt_contracts::services::LanguageId;
    ///
    /// let source = b"def load():\n    data = read()\n    return parse(data)\n";
    /// let analysis = analyze(Registry::builtin(), source, &LanguageId::new("python")?)?;
    /// let callees: Vec<&str> = analysis.outgoing(0).iter().map(|call| call.callee.as_str()).collect();
    /// assert_eq!(callees, ["read", "parse"]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn outgoing(&self, symbol: usize) -> Vec<&CallSite> {
        self.calls()
            .iter()
            .filter(|call| call.caller == Some(symbol))
            .collect()
    }

    /// Calls of functions named `name`, in source order.
    #[must_use]
    pub fn incoming(&self, name: &str) -> Vec<&CallSite> {
        self.calls()
            .iter()
            .filter(|call| call.callee == name)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::services::{LanguageId, SymbolKind};

    use super::Callers;
    use crate::local::analyze;
    use crate::registry::Registry;

    #[test]
    fn calls_are_attributed_to_the_innermost_function() {
        let source = "fn outer() { inner(); fn nested() { leaf(); } }\nfn inner() {}\n";
        let analysis = analyze(
            Registry::builtin(),
            source.as_bytes(),
            &LanguageId::new("rust").unwrap(),
        )
        .unwrap();
        let outer = analysis
            .symbols()
            .iter()
            .position(|s| s.name == "outer")
            .unwrap();
        let nested = analysis.symbols().iter().position(|s| s.name == "nested");
        let callees: Vec<&str> = analysis
            .outgoing(outer)
            .iter()
            .map(|c| c.callee.as_str())
            .collect();
        assert_eq!(callees, ["inner"]);
        if let Some(nested) = nested {
            assert_eq!(analysis.outgoing(nested)[0].callee, "leaf");
        }
        let callers: Vec<Option<usize>> = analysis
            .incoming("inner")
            .iter()
            .map(|c| c.caller)
            .collect();
        assert_eq!(callers, [Some(outer)]);
    }

    #[test]
    fn caller_search_agrees_with_a_scan_of_every_function() {
        let source = "fn a() { fn b() { fn c() {} c(); } b(); }\nfn d() { a(); }\nstruct S;\nimpl S { fn e(&self) { d(); } fn f(&self) { self.e(); } }\nfn g() {}\n";
        let analysis = analyze(
            Registry::builtin(),
            source.as_bytes(),
            &LanguageId::new("rust").unwrap(),
        )
        .unwrap();
        let symbols = analysis.symbols();
        let callers = Callers::new(symbols);
        for offset in 0..=source.len() as u64 {
            let expected = symbols
                .iter()
                .enumerate()
                .filter(|(_, symbol)| {
                    matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
                        && symbol.span.bytes.start <= offset
                        && offset < symbol.span.bytes.end
                })
                .min_by_key(|(_, symbol)| symbol.span.bytes.len())
                .map(|(index, _)| index);
            assert_eq!(callers.of(offset), expected, "offset {offset}");
        }
        assert!(callers.of(0).is_some());
        assert_eq!(callers.of(source.len() as u64), None);
    }

    #[test]
    fn method_calls_use_the_method_name() {
        let source =
            "class A { void run() { this.helper(); Util.log(\"x\"); } void helper() {} }\n";
        let analysis = analyze(
            Registry::builtin(),
            source.as_bytes(),
            &LanguageId::new("java").unwrap(),
        )
        .unwrap();
        let names: Vec<&str> = analysis.calls().iter().map(|c| c.callee.as_str()).collect();
        assert_eq!(names, ["helper", "log"]);
        assert_eq!(analysis.incoming("helper").len(), 1);
    }
}
