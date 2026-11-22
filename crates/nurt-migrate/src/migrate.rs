// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of a whole script, statement by statement.

use nurt_contracts::ast::{Arg, AstDocument, Expr, Head, Pipeline, Program, Stage, Statement};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::CorrectionClass;

use crate::language::ScriptLanguage;
use crate::output::{self, Block};
use crate::report::{Item, MigrationReport, Reason, Status};
use crate::segment::{self, Join, Part, Position, Segment, SegmentKind};
use crate::translate::{LetBinding, TranslateRequest, Translation, Translator};

/// Assumption added to a statement that follows `;`.
const SEQUENCE_ASSUMPTION: &str = "the script stops on the first error; the shell runs a statement after `;` even when the previous one fails";

/// Script name used in the fallback program when the options name none.
const DEFAULT_SCRIPT_STEM: &str = "script";

/// Options of a migration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MigrateOptions {
    /// Script path written into the fallback program `run [...]`; an absolute
    /// path keeps the program independent of `base`. Without a path the
    /// program names `script.sh` or `script.ps1`.
    pub script: Option<String>,
}

impl MigrateOptions {
    /// Creates options without a script path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the script path written into the fallback program.
    #[must_use]
    pub fn with_script(mut self, path: impl Into<String>) -> Self {
        self.script = Some(path.into());
        self
    }
}

/// Result of a migration: the program file and the report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Migration {
    /// Content of the `.nurt` file.
    pub nurt: String,
    /// Report of the statements.
    pub report: MigrationReport,
}

/// Translates a bash or PowerShell script into a program file.
///
/// Each statement is translated by `translator` in script order. A list
/// joined with `&&` or `;` that the translator rejects with `E_ONE_EFFECT`
/// is translated part by part into consecutive pipelines. Loops, conditions,
/// definitions, blocks and dynamic evaluation are reported as impossible. A
/// syntax error affects only the statement that contains it. The result is
/// deterministic for a deterministic translator.
///
/// # Errors
///
/// Returns `E_INTERNAL` when the grammar of the language cannot be loaded.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::AstDocument;
/// use nurt_contracts::error::{Error, ErrorCode};
/// use nurt_migrate::{
///     MigrateOptions, ScriptLanguage, Status, TranslateRequest, Translation, Translator, migrate,
/// };
///
/// struct Untranslated;
///
/// impl Translator for Untranslated {
///     fn translate(&self, _: &TranslateRequest<'_>) -> Result<Translation, Error> {
///         Err(Error::new(ErrorCode::Uncorrectable, "no translation rule"))
///     }
///     fn print(&self, _: &AstDocument) -> String {
///         "run [\"bash\", \"/srv/a.sh\"]".to_owned()
///     }
///     fn check(&self, _: &str) -> Result<(), Error> {
///         Ok(())
///     }
/// }
///
/// let options = MigrateOptions::new().with_script("/srv/a.sh");
/// let migration = migrate("for f in *; do echo $f; done\n", ScriptLanguage::Bash, &options, &Untranslated)?;
/// assert_eq!(migration.report.items[0].status, Status::Impossible);
/// assert!(migration.nurt.contains("# untranslated: E_UNCORRECTABLE: for f in *"));
/// # Ok::<(), Error>(())
/// ```
pub fn migrate(
    source: &str,
    language: ScriptLanguage,
    options: &MigrateOptions,
    translator: &dyn Translator,
) -> Result<Migration, Error> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let normalized = source.replace("\r\n", "\n");
    let segments = segment::segment(&normalized, language)?;
    let script = options.script.clone().unwrap_or_else(|| {
        let extension = match language {
            ScriptLanguage::Bash => "sh",
            ScriptLanguage::PowerShell => "ps1",
        };
        format!("{DEFAULT_SCRIPT_STEM}.{extension}")
    });
    let mut session = Session {
        source: &normalized,
        language,
        translator,
        lets: Vec::new(),
        let_statements: Vec::new(),
        report: MigrationReport::new(language, &script),
        blocks: Vec::new(),
        directories: Vec::new(),
    };
    for segment in &segments {
        session.segment(segment);
    }
    Ok(session.finish())
}

struct Session<'a> {
    source: &'a str,
    language: ScriptLanguage,
    translator: &'a dyn Translator,
    lets: Vec<LetBinding>,
    let_statements: Vec<String>,
    report: MigrationReport,
    blocks: Vec<Block>,
    directories: Vec<String>,
}

/// Statement or part of a statement being translated.
struct Unit<'p> {
    start: Position,
    end_line: u32,
    text: &'p str,
    part: Option<&'p Part>,
    or_list: bool,
}

impl Session<'_> {
    fn segment(&mut self, segment: &Segment) {
        let text = segment.text(self.source);
        let unit = Unit {
            start: segment.start,
            end_line: segment.end_line,
            text,
            part: None,
            or_list: false,
        };
        match &segment.kind {
            SegmentKind::Comment(lines) => self.blocks.push(Block {
                start_line: segment.start.line,
                end_line: segment.end_line,
                lines: lines.clone(),
            }),
            SegmentKind::Construct(reason) => {
                let mut item =
                    Item::new(unit.start.line, unit.start.column, text, Status::Impossible);
                item.code = Some(ErrorCode::Uncorrectable);
                item.reason = Some(*reason);
                item.problem = Some(construct_problem(*reason).to_owned());
                self.push(&unit, item);
            }
            SegmentKind::Invalid(at) => {
                let mut item = Item::new(unit.start.line, unit.start.column, text, Status::Error);
                item.code = Some(ErrorCode::Syntax);
                item.reason = Some(Reason::Syntax);
                item.problem = Some(format!(
                    "the script has a syntax error at {}:{}; correct the script or run it unchanged",
                    at.line, at.column
                ));
                self.push(&unit, item);
            }
            SegmentKind::Statement(statement) => self.statement(segment, statement, unit),
        }
    }

    fn statement(&mut self, segment: &Segment, statement: &segment::Statement, unit: Unit<'_>) {
        let or_list = statement.joins.contains(&Join::Or);
        let [part] = statement.parts.as_slice() else {
            let outcome = self.translate(unit.text);
            match outcome {
                Ok(translation) => self.accept(&unit, translation, Vec::new()),
                Err(error) if error.code() == ErrorCode::OneEffect && !or_list => {
                    self.parts(segment, statement);
                }
                Err(error) => self.reject(&Unit { or_list, ..unit }, &error),
            }
            return;
        };
        let unit = Unit {
            part: Some(part),
            ..unit
        };
        match self.translate(unit.text) {
            Ok(translation) => self.accept(&unit, translation, Vec::new()),
            Err(error) => self.reject(&unit, &error),
        }
    }

    /// Translates the parts of a list into consecutive statements.
    fn parts(&mut self, segment: &Segment, statement: &segment::Statement) {
        let source = self.source;
        for (index, part) in statement.parts.iter().enumerate() {
            let after_sequence = index
                .checked_sub(1)
                .and_then(|previous| statement.joins.get(previous))
                .is_some_and(|join| *join == Join::Sequence);
            let unit = Unit {
                start: part.start,
                end_line: segment.end_line,
                text: part.text(source),
                part: Some(part),
                or_list: false,
            };
            let extra = if after_sequence {
                vec![SEQUENCE_ASSUMPTION.to_owned()]
            } else {
                Vec::new()
            };
            match self.translate(unit.text) {
                Ok(translation) => self.accept(&unit, translation, extra),
                Err(error) => self.reject(&unit, &error),
            }
        }
    }

    fn translate(&self, statement: &str) -> Result<Translation, Error> {
        self.translator.translate(&TranslateRequest {
            language: self.language,
            statement,
            lets: &self.lets,
        })
    }

    fn accept(&mut self, unit: &Unit<'_>, translation: Translation, extra: Vec<String>) {
        let output = self.translator.print(&translation.tree);
        let mut script = self.let_statements.join("\n");
        if !script.is_empty() {
            script.push('\n');
        }
        script.push_str(&output);
        if let Err(error) = self.translator.check(&script) {
            let mut item = error_item(unit, Status::Error, &error);
            item.reason = Some(Reason::Translation);
            item.output = Some(output);
            self.push(unit, item);
            return;
        }
        let probable = translation
            .rules
            .iter()
            .any(|correction| correction.class == CorrectionClass::Probable);
        let mut assumptions = translation.assumptions;
        assumptions.extend(extra);
        let status = if probable || !assumptions.is_empty() {
            Status::Assumption
        } else {
            Status::Translated
        };
        let mut item = Item::new(unit.start.line, unit.start.column, unit.text, status);
        for correction in &translation.rules {
            if !item.rules.contains(&correction.rule) {
                item.rules.push(correction.rule.clone());
            }
        }
        item.assumptions = assumptions;
        item.output = Some(output);
        self.record_lets(unit, &translation.tree);
        self.push(unit, item);
    }

    fn record_lets(&mut self, unit: &Unit<'_>, tree: &AstDocument) {
        let mut last = None;
        for statement in &tree.program.statements {
            if let Statement::Let(binding) = statement {
                let single = AstDocument::new(Program {
                    statements: vec![statement.clone()],
                });
                self.let_statements.push(self.translator.print(&single));
                last = Some(binding.name.clone());
            }
        }
        let variable = unit.part.and_then(|part| part.variable.clone());
        if let (Some(variable), Some(name)) = (variable, last) {
            self.lets.retain(|existing| existing.variable != variable);
            self.lets.push(LetBinding { variable, name });
        }
    }

    fn reject(&mut self, unit: &Unit<'_>, error: &Error) {
        let directory = unit.part.and_then(|part| part.directory.clone());
        let here_document = unit.part.is_some_and(|part| part.here_document);
        let reason = if unit.or_list {
            Reason::OrList
        } else if directory.is_some() || error.code() == ErrorCode::PathRelative {
            Reason::CurrentDirectory
        } else if here_document {
            Reason::HereDocument
        } else {
            Reason::Rule
        };
        if let Some(directory) = directory {
            self.directories.push(directory);
        }
        let mut item = error_item(unit, Status::Impossible, error);
        item.reason = Some(reason);
        self.push(unit, item);
    }

    fn push(&mut self, unit: &Unit<'_>, item: Item) {
        self.blocks.push(Block {
            start_line: unit.start.line,
            end_line: unit.end_line,
            lines: output::item_lines(&item),
        });
        self.report.items.push(item);
    }

    fn finish(mut self) -> Migration {
        let untranslated = self
            .report
            .items
            .iter()
            .any(|item| !item.status.is_translated());
        if untranslated {
            let program = fallback_program(self.language.interpreter(), &self.report.script);
            self.report.fallback = Some(self.translator.print(&program));
        }
        self.report.base = proposed_base(&self.directories);
        let nurt = output::render(&self.report, &self.blocks);
        Migration {
            nurt,
            report: self.report,
        }
    }
}

fn error_item(unit: &Unit<'_>, status: Status, error: &Error) -> Item {
    let data = error.data();
    let mut item = Item::new(unit.start.line, unit.start.column, unit.text, status);
    item.code = Some(data.code);
    item.problem = Some(data.problem.clone().unwrap_or_else(|| data.message.clone()));
    let candidates = data.candidates.iter().map(|candidate| &candidate.program);
    for program in data.fix.iter().chain(candidates) {
        if !item.fix.contains(program) {
            item.fix.push(program.clone());
        }
    }
    item
}

fn construct_problem(reason: Reason) -> &'static str {
    match reason {
        Reason::Loop => {
            "loops have no equivalent; a pattern or a pipeline over records replaces a loop"
        }
        Reason::Condition => {
            "conditional statements have no equivalent; assert checks a precondition"
        }
        Reason::Function => "definitions of functions and types have no equivalent",
        Reason::Block => "command groups and block statements have no equivalent",
        Reason::Eval => "evaluation of text as code has no equivalent",
        Reason::Source => {
            "loading a script into the current shell has no equivalent; run executes a script"
        }
        Reason::Parameters => "script parameters have no equivalent",
        Reason::Control => "flow control statements have no equivalent",
        Reason::OrList
        | Reason::HereDocument
        | Reason::CurrentDirectory
        | Reason::Rule
        | Reason::Syntax
        | Reason::Translation => "the statement has no equivalent",
    }
}

/// Program `run ["<interpreter>", "<script>"]`.
fn fallback_program(interpreter: &str, script: &str) -> AstDocument {
    let text = |value: &str| Expr::Text {
        v: value.to_owned(),
        span: None,
    };
    let stage = Stage {
        name: "run".to_owned(),
        args: vec![Arg {
            name: None,
            value: Expr::List {
                items: vec![text(interpreter), text(script)],
                span: None,
            },
            span: None,
        }],
        span: None,
    };
    AstDocument::new(Program {
        statements: vec![Statement::Pipeline(Pipeline {
            head: Head::Stage(stage),
            stages: Vec::new(),
            span: None,
        })],
    })
}

/// The only absolute target of the untranslated directory changes; none
/// when a target is relative or the targets differ.
fn proposed_base(directories: &[String]) -> Option<String> {
    let (first, rest) = directories.split_first()?;
    let unique = rest.iter().all(|directory| directory == first);
    (unique && is_absolute(first)).then(|| first.clone())
}

/// Absolute path on any system: `/a`, `//server/share`, `\\server\share`,
/// `C:/a` or `C:\a`.
fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    match bytes {
        [b'/', ..] | [b'\\', b'\\', ..] => true,
        [drive, b':', b'/' | b'\\', ..] => drive.is_ascii_alphabetic(),
        _ => false,
    }
}
