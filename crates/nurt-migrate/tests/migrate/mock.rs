// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translator with fixed replies keyed by the exact statement text.
//!
//! The replies stand in for the habit tables: they name table rules and
//! programs, so the tests exercise the migration independently of the
//! table content.

use std::cell::RefCell;
use std::collections::BTreeMap;

use nurt_contracts::ast::{
    Arg, AstDocument, Expr, Head, Let, Pipeline, Program, Ref, Stage, Statement,
};
use nurt_contracts::error::{Candidate, Error, ErrorCode};
use nurt_contracts::report::{Correction, CorrectionClass};
use nurt_migrate::{TranslateRequest, Translation, Translator};

/// Member of the tree metadata that carries the program text of a reply.
const TEXT_MEMBER: &str = "text";

#[derive(Debug, Clone)]
enum Reply {
    Program {
        text: String,
        rules: Vec<(String, CorrectionClass)>,
        assumptions: Vec<String>,
        binds: Option<String>,
        requires: Option<String>,
    },
    Failure {
        code: ErrorCode,
        problem: String,
        fix: Vec<String>,
        candidates: Vec<String>,
    },
}

/// Translator answering from a table of replies.
///
/// A statement without a reply is rejected with `E_ONE_EFFECT` when it
/// contains `&&` or `;`, and with `E_UNCORRECTABLE` otherwise.
#[derive(Debug, Default)]
pub(crate) struct MockTranslator {
    replies: BTreeMap<String, Reply>,
    unbound: Vec<String>,
    requests: RefCell<Vec<String>>,
}

impl MockTranslator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Translates `statement` to `program`; a rule `B15:P` has class P, a
    /// rule without a class has class C.
    pub(crate) fn program(mut self, statement: &str, program: &str, rules: &[&str]) -> Self {
        let rules = rules
            .iter()
            .map(|rule| match rule.split_once(':') {
                Some((id, "P")) => (id.to_owned(), CorrectionClass::Probable),
                Some((id, _)) => (id.to_owned(), CorrectionClass::Certain),
                None => ((*rule).to_owned(), CorrectionClass::Certain),
            })
            .collect();
        self.replies.insert(
            statement.to_owned(),
            Reply::Program {
                text: program.to_owned(),
                rules,
                assumptions: Vec::new(),
                binds: None,
                requires: None,
            },
        );
        self
    }

    /// Adds an assumption to the program reply of `statement`.
    pub(crate) fn assume(mut self, statement: &str, assumption: &str) -> Self {
        if let Some(Reply::Program { assumptions, .. }) = self.replies.get_mut(statement) {
            assumptions.push(assumption.to_owned());
        }
        self
    }

    /// Makes the program reply of `statement` a `let` with the given name.
    pub(crate) fn binds(mut self, statement: &str, name: &str) -> Self {
        if let Some(Reply::Program { binds, .. }) = self.replies.get_mut(statement) {
            *binds = Some(name.to_owned());
        }
        self
    }

    /// Makes the program reply of `statement` depend on a bound variable.
    pub(crate) fn requires(mut self, statement: &str, variable: &str) -> Self {
        if let Some(Reply::Program { requires, .. }) = self.replies.get_mut(statement) {
            *requires = Some(variable.to_owned());
        }
        self
    }

    /// Rejects `statement` with an error.
    pub(crate) fn fail(
        mut self,
        statement: &str,
        code: ErrorCode,
        problem: &str,
        fix: &[&str],
    ) -> Self {
        self.replies.insert(
            statement.to_owned(),
            Reply::Failure {
                code,
                problem: problem.to_owned(),
                fix: fix.iter().map(|program| (*program).to_owned()).collect(),
                candidates: Vec::new(),
            },
        );
        self
    }

    /// Adds ranked candidates to the failure reply of `statement`.
    pub(crate) fn candidates(mut self, statement: &str, programs: &[&str]) -> Self {
        if let Some(Reply::Failure { candidates, .. }) = self.replies.get_mut(statement) {
            candidates.extend(programs.iter().map(|program| (*program).to_owned()));
        }
        self
    }

    /// Makes `check` fail for every script that contains `marker`.
    pub(crate) fn unbound(mut self, marker: &str) -> Self {
        self.unbound.push(marker.to_owned());
        self
    }

    /// Statements submitted for translation, in order.
    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.borrow().clone()
    }

    fn default_reply(statement: &str) -> Error {
        if statement.contains("&&") || statement.contains(';') {
            return Error::new(ErrorCode::OneEffect, "two effects in one program")
                .with_problem("the statement contains two operations");
        }
        let command = statement.split_whitespace().next().unwrap_or_default();
        Error::new(ErrorCode::Uncorrectable, "no translation rule")
            .with_problem(format!("no translation rule for `{command}`"))
    }
}

impl Translator for MockTranslator {
    fn translate(&self, request: &TranslateRequest<'_>) -> Result<Translation, Error> {
        self.requests
            .borrow_mut()
            .push(request.statement.to_owned());
        match self.replies.get(request.statement) {
            None => Err(Self::default_reply(request.statement)),
            Some(Reply::Failure {
                code,
                problem,
                fix,
                candidates,
            }) => {
                let mut error = Error::new(*code, "rejected").with_problem(problem.clone());
                for program in fix {
                    error = error.with_fix(program.clone());
                }
                for (rank, program) in (1..).zip(candidates) {
                    error = error.with_candidate(Candidate {
                        rank,
                        reference: None,
                        program: program.clone(),
                        why: None,
                    });
                }
                Err(error)
            }
            Some(Reply::Program {
                text,
                rules,
                assumptions,
                binds,
                requires,
            }) => {
                if let Some(variable) = requires
                    && !request
                        .lets
                        .iter()
                        .any(|binding| &binding.variable == variable)
                {
                    return Err(Error::new(ErrorCode::Uncorrectable, "unbound variable")
                        .with_problem(format!(
                            "`${variable}` is not bound by an earlier statement"
                        )));
                }
                Ok(Translation {
                    tree: tree(text, binds.as_deref()),
                    rules: rules
                        .iter()
                        .enumerate()
                        .map(|(index, (rule, class))| Correction {
                            id: format!("C{}", index + 1),
                            rule: rule.clone(),
                            class: *class,
                            at: None,
                            from: request.statement.to_owned(),
                            to: text.clone(),
                            kind: None,
                            alternatives: Vec::new(),
                        })
                        .collect(),
                    assumptions: assumptions.clone(),
                    records: Vec::new(),
                })
            }
        }
    }

    fn print(&self, tree: &AstDocument) -> String {
        if let Some(text) = tree
            .meta
            .as_ref()
            .and_then(|meta| meta.get(TEXT_MEMBER))
            .and_then(serde_json::Value::as_str)
        {
            return text.to_owned();
        }
        print_run(tree)
    }

    fn check(&self, script: &str) -> Result<(), Error> {
        match self
            .unbound
            .iter()
            .find(|marker| script.contains(marker.as_str()))
        {
            Some(marker) => Err(Error::new(ErrorCode::UnknownParam, "unknown parameter")
                .with_problem(format!("`{marker}` is not a parameter of the command"))),
            None => Ok(()),
        }
    }
}

fn tree(text: &str, binds: Option<&str>) -> AstDocument {
    let pipeline = Pipeline {
        head: Head::Ref(Ref {
            name: "reply".to_owned(),
            fields: Vec::new(),
            span: None,
        }),
        stages: Vec::new(),
        span: None,
    };
    let statement = match binds {
        Some(name) => Statement::Let(Let {
            name: name.to_owned(),
            value: pipeline,
            span: None,
        }),
        None => Statement::Pipeline(pipeline),
    };
    let mut document = AstDocument::new(Program {
        statements: vec![statement],
    });
    let mut meta = serde_json::Map::new();
    let program = match binds {
        Some(name) if !text.starts_with("let ") => format!("let {name} = {text}"),
        _ => text.to_owned(),
    };
    meta.insert(TEXT_MEMBER.to_owned(), serde_json::Value::String(program));
    document.meta = Some(meta);
    document
}

/// Prints the trees built by the migration itself: a single `let` taken from
/// a reply and the program `run ["a", "b"]`.
fn print_run(tree: &AstDocument) -> String {
    if let Some(Statement::Let(binding)) = tree.program.statements.first() {
        return format!("let {} = @reply", binding.name);
    }
    let Some(Statement::Pipeline(Pipeline {
        head: Head::Stage(Stage { name, args, .. }),
        ..
    })) = tree.program.statements.first()
    else {
        return String::new();
    };
    let items: Vec<String> = args
        .iter()
        .flat_map(|Arg { value, .. }| match value {
            Expr::List { items, .. } => items.clone(),
            other => vec![other.clone()],
        })
        .map(|item| match item {
            Expr::Text { v, .. } => serde_json::Value::String(v).to_string(),
            _ => String::new(),
        })
        .collect();
    format!("{name} [{}]", items.join(", "))
}
