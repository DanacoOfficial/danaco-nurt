// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of programs, statements and pipelines.

use nurt_contracts::ast::{
    AST_VERSION, Arg, AstDocument, Expr, Head, Pipeline, Ref, Span, Stage, Statement,
};
use nurt_contracts::bound::{
    BoundDocument, BoundHead, BoundLet, BoundPipeline, BoundProgram, BoundRef, BoundStatement,
};
use nurt_contracts::error::{Error, ErrorCode, Stage as ExecStage};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::spec::IoSpec;
use nurt_correct::distance::{EXPECTED_LIMIT, NameEntry, NameIndex, NameKind, expected};
use nurt_syntax::print::try_print;

use crate::catalog::Catalog;
use crate::context::BindContext;
use crate::correct::{Applied, Corrector};
use crate::error::BindError;
use crate::phase::Walk;

/// Kind of the records that flow into a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Flow {
    /// The stage starts a pipeline and has no input.
    Start,
    /// The stage starts a sub-pipeline: it has no input when it accepts none,
    /// and otherwise receives records of any kind.
    Open,
    /// The stage receives records of one of the kinds, in order of preference.
    Records(Vec<IoSpec>),
}

impl Flow {
    /// Records whose kind is not known when the program is bound.
    pub(crate) fn any() -> Self {
        Self::Records(vec![IoSpec::Any])
    }
}

/// Role of a pipeline in the program.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mode<'m> {
    /// A pipeline statement.
    Statement,
    /// The pipeline of `let name = …`.
    Let(&'m str),
    /// A sub-pipeline in a place of type `pipeline`.
    Sub,
}

/// Position of a top-level stage in the surface program, used to build ready
/// replacement programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Location {
    statement: usize,
    stage: Option<usize>,
}

/// State of one binding call.
pub(crate) struct Binder<'a> {
    pub(crate) catalog: &'a Catalog,
    pub(crate) context: &'a BindContext,
    document: &'a AstDocument,
    lets: Vec<(String, Flow)>,
    location: Option<Location>,
    /// Correction of names and literals; absent for a binding without
    /// correction.
    pub(crate) corrector: Option<&'a Corrector<'a>>,
    /// Depth of the parts of the binding where corrections are suppressed.
    pub(crate) strict: usize,
    /// Corrections made so far.
    pub(crate) applied: Vec<Applied>,
    /// Fields of the records that flow into the stage being bound; absent
    /// when their kind is not known.
    pub(crate) fields: Option<Vec<&'static str>>,
}

/// Major version of the surface tree format accepted by the binder.
const AST_MAJOR: &str = "1";

/// Binds a surface tree.
pub(crate) fn bind_document(
    document: &AstDocument,
    catalog: &Catalog,
    context: &BindContext,
) -> Result<BoundDocument, BindError> {
    run(document, catalog, context, None).map(|(bound, _)| bound)
}

/// Binds a surface tree with correction of names and literals and returns
/// the corrections made.
pub(crate) fn bind_corrected(
    document: &AstDocument,
    catalog: &Catalog,
    context: &BindContext,
    corrector: &Corrector<'_>,
) -> Result<(BoundDocument, Vec<Applied>), BindError> {
    run(document, catalog, context, Some(corrector))
}

fn run(
    document: &AstDocument,
    catalog: &Catalog,
    context: &BindContext,
    corrector: Option<&Corrector<'_>>,
) -> Result<(BoundDocument, Vec<Applied>), BindError> {
    check_version(&document.nurt)?;
    let mut binder = Binder {
        catalog,
        context,
        document,
        lets: Vec::new(),
        location: None,
        corrector,
        strict: 0,
        applied: Vec::new(),
        fields: None,
    };
    let statements = binder.statements()?;
    let bound = BoundDocument::new(
        catalog.revision(),
        context.corr_rev.clone(),
        BoundProgram { statements },
    );
    Ok((bound, binder.applied))
}

fn check_version(version: &str) -> Result<(), BindError> {
    let major = version.split_once('.').map_or(version, |(major, _)| major);
    if major == AST_MAJOR {
        return Ok(());
    }
    Err(BindError::from(
        Error::new(
            ErrorCode::AstVersion,
            format!(
                "tree version {version} is not supported; send a tree of version {AST_VERSION}"
            ),
        )
        .with_stage(ExecStage::Parse),
    ))
}

impl<'a> Binder<'a> {
    fn statements(&mut self) -> Result<Vec<BoundStatement>, BindError> {
        let document = self.document;
        if document.program.statements.is_empty() {
            return Err(BindError::from(
                Error::new(
                    ErrorCode::AstSchema,
                    "the program has no statement; write at least one pipeline",
                )
                .with_stage(ExecStage::Parse),
            ));
        }
        let mut bound = Vec::with_capacity(document.program.statements.len());
        for (index, statement) in document.program.statements.iter().enumerate() {
            bound.push(self.statement(index, statement)?);
        }
        Ok(bound)
    }

    fn statement(
        &mut self,
        index: usize,
        statement: &Statement,
    ) -> Result<BoundStatement, BindError> {
        match statement {
            Statement::Let(binding) => {
                let (value, flow) =
                    self.pipeline(&binding.value, Mode::Let(&binding.name), Some(index))?;
                self.lets.push((binding.name.clone(), flow));
                Ok(BoundStatement::Let(BoundLet {
                    name: binding.name.clone(),
                    value,
                    span: binding.span,
                }))
            }
            Statement::Pipeline(pipeline) => self
                .pipeline(pipeline, Mode::Statement, Some(index))
                .map(|(pipeline, _)| BoundStatement::Pipeline(pipeline)),
            _ => Err(unsupported("statement")),
        }
    }

    /// Binds a pipeline and returns it with the kind of its output records.
    pub(crate) fn pipeline(
        &mut self,
        pipeline: &Pipeline,
        mode: Mode<'_>,
        statement: Option<usize>,
    ) -> Result<(BoundPipeline, Flow), BindError> {
        let let_name = match mode {
            Mode::Let(name) => Some(name),
            Mode::Statement | Mode::Sub => None,
        };
        let mut walk = Walk::new(pipeline, let_name);
        let mut stages = Vec::new();
        let (head, mut flow) = match &pipeline.head {
            Head::Ref(reference) => {
                let (name, flow) = self.resolve_let(reference)?;
                (
                    BoundHead::Ref(BoundRef {
                        name,
                        fields: reference.fields.clone(),
                    }),
                    flow,
                )
            }
            Head::Stage(stage) => {
                self.location = statement.map(|statement| Location {
                    statement,
                    stage: None,
                });
                let start = if matches!(mode, Mode::Sub) {
                    Flow::Open
                } else {
                    Flow::Start
                };
                let mut bound = self.command(stage, &start)?.into_iter();
                let Some(mut first) = bound.next() else {
                    return Err(internal("binding a stage produced no stage"));
                };
                walk.place(&mut first, 0, true)?;
                let mut flow = first.output.clone();
                for mut next in bound {
                    walk.place(&mut next, 0, false)?;
                    flow = next.output.clone();
                    stages.push(next.stage);
                }
                (BoundHead::Stage(first.stage), flow)
            }
            _ => return Err(unsupported("pipeline head")),
        };
        for (position, stage) in pipeline.stages.iter().enumerate() {
            let index = position + 1;
            let spec = self.spec(stage)?;
            walk.admit(spec, stage, index)?;
            self.location = statement.map(|statement| Location {
                statement,
                stage: Some(position),
            });
            for mut bound in self.command(stage, &flow)? {
                walk.place(&mut bound, index, false)?;
                flow = bound.output.clone();
                stages.push(bound.stage);
            }
        }
        Ok((
            BoundPipeline {
                head,
                stages,
                span: pipeline.span,
            },
            flow,
        ))
    }

    /// Binds a sub-pipeline in a place of type `pipeline`.
    pub(crate) fn sub_pipeline(&mut self, pipeline: &Pipeline) -> Result<BoundPipeline, BindError> {
        let location = self.location.take();
        let fields = self.fields.take();
        let bound = self.pipeline(pipeline, Mode::Sub, None);
        self.location = location;
        self.fields = fields;
        bound.map(|(pipeline, _)| pipeline)
    }

    /// Binds a reference to a named result.
    pub(crate) fn reference(&mut self, reference: &Ref) -> Result<BoundRef, BindError> {
        self.resolve_let(reference).map(|(name, _)| BoundRef {
            name,
            fields: reference.fields.clone(),
        })
    }

    /// Returns the kind of the records of a named result.
    pub(crate) fn let_flow(&self, name: &str) -> Option<Flow> {
        self.lets
            .iter()
            .rev()
            .find(|(known, _)| known == name)
            .map(|(_, flow)| flow.clone())
            .or_else(|| {
                self.context
                    .lets
                    .iter()
                    .any(|known| known == name)
                    .then(Flow::any)
            })
    }

    /// Returns the names of the results known at this point.
    pub(crate) fn let_names(&self) -> Vec<String> {
        self.lets
            .iter()
            .map(|(name, _)| name.clone())
            .chain(self.context.lets.iter().cloned())
            .collect()
    }

    /// Error for a reference to a name that is not known.
    pub(crate) fn unknown_name(&self, reference: &Ref) -> BindError {
        let index = NameIndex::new(
            self.let_names()
                .into_iter()
                .map(|name| NameEntry::name(name, NameKind::Let)),
        );
        BindError::new(
            ErrorCode::UnknownName,
            format!(
                "unknown name `@{}`; define it first with `let {} = …`",
                reference.name, reference.name
            ),
        )
        .at(reference.span)
        .found(format!("@{}", reference.name))
        .expected(&expected(
            &reference.name,
            &index,
            NameKind::Let,
            EXPECTED_LIMIT,
        ))
        .help("let")
    }

    /// Returns the program with a named argument added to the stage being
    /// bound, printed in variant A; `None` inside a sub-pipeline.
    pub(crate) fn program_with(&self, param: &str, value: &str) -> Option<String> {
        self.program_edited(|stage| {
            stage.args.push(Arg {
                name: Some(param.to_owned()),
                value: Expr::Ident {
                    v: value.to_owned(),
                    span: None,
                },
                span: None,
            });
            true
        })
    }

    /// Returns the program with the bare word written at `span` in the stage
    /// being bound replaced by a text with the same characters, printed in
    /// variant A; `None` inside a sub-pipeline or when no argument has that
    /// span.
    pub(crate) fn program_quoting(&self, span: Option<Span>) -> Option<String> {
        let span = span?;
        self.program_edited(|stage| {
            stage.args.iter_mut().any(|arg| {
                let Expr::Ident { v, span: written } = &arg.value else {
                    return false;
                };
                if *written != Some(span) {
                    return false;
                }
                arg.value = Expr::Text {
                    v: v.clone(),
                    span: None,
                };
                true
            })
        })
    }

    /// Prints the program after `edit` changed the stage being bound; `edit`
    /// returns `false` when it changed nothing.
    fn program_edited(&self, edit: impl FnOnce(&mut Stage) -> bool) -> Option<String> {
        let location = self.location?;
        let mut document = self.document.clone();
        document.meta = None;
        let statement = document.program.statements.get_mut(location.statement)?;
        let pipeline = match statement {
            Statement::Let(binding) => &mut binding.value,
            Statement::Pipeline(pipeline) => pipeline,
            _ => return None,
        };
        let stage = match location.stage {
            None => match &mut pipeline.head {
                Head::Stage(stage) => stage,
                _ => return None,
            },
            Some(position) => pipeline.stages.get_mut(position)?,
        };
        if !edit(stage) {
            return None;
        }
        try_print(&document, SyntaxVariant::A)
            .ok()
            .map(|text| text.trim_end().to_owned())
    }
}

/// Error for a node kind this version of the binder does not know.
pub(crate) fn unsupported(what: &str) -> BindError {
    BindError::from(
        Error::new(
            ErrorCode::AstVersion,
            format!("unsupported {what} kind; send a tree of version {AST_VERSION}"),
        )
        .with_stage(ExecStage::Parse),
    )
}

/// Error for a broken invariant of the binder or the catalog.
pub(crate) fn internal(message: impl Into<String>) -> BindError {
    BindError::new(ErrorCode::Internal, message)
}

/// Prints a pipeline as a program of one statement in variant A.
pub(crate) fn print_pipeline(pipeline: Pipeline) -> Option<String> {
    let document = AstDocument::new(nurt_contracts::ast::Program {
        statements: vec![Statement::Pipeline(pipeline)],
    });
    try_print(&document, SyntaxVariant::A)
        .ok()
        .map(|text| text.trim_end().to_owned())
}
