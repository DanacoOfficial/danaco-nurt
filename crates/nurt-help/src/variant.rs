// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-help::variant
// Responsibility:   Programs of examples in the three syntax variants.
// Dependencies:     nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Programs of examples in the three syntax variants.
//!
//! Every example is stored once, in variant A. The other variants are
//! produced by parsing the example and printing the tree with the printer of
//! `nurt-syntax` (a pipeline whose first stage has arguments is printed one
//! stage per line), so the three notations of an example always denote the
//! same program. In variant C positional arguments are given the names of
//! the parameters from the catalog, and the arguments of a repeated
//! parameter are written as one list.

use nurt_contracts::ast::{Arg, AstDocument, Expr, Head, Pipeline, Stage, Statement};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print::{self, PositionalNames};

use crate::catalog::Catalog;

pub use nurt_syntax::parse::Variant;

/// Fence that opens a block with a program in a guide.
const PROGRAM_FENCE: &str = "```nurt";

/// Marker of a fenced block.
const FENCE: &str = "```";

/// Names of the positional parameters taken from the catalog.
struct CatalogNames<'a>(&'a Catalog);

impl PositionalNames for CatalogNames<'_> {
    fn name(&self, command: &str, index: usize) -> Option<String> {
        self.0
            .command(command)?
            .positional()
            .get(index)
            .map(|param| param.name.clone())
    }
}

/// Joins the positional arguments of a repeated parameter into one list
/// argument named after the parameter, in the stages of the top-level
/// pipelines. Variant C has no notation for several positional arguments of
/// one parameter.
fn collect_repeated(catalog: &Catalog, mut document: AstDocument) -> AstDocument {
    for statement in &mut document.program.statements {
        match statement {
            Statement::Let(binding) => collect_in_pipeline(catalog, &mut binding.value),
            Statement::Pipeline(pipeline) => collect_in_pipeline(catalog, pipeline),
            _ => {}
        }
    }
    document
}

fn collect_in_pipeline(catalog: &Catalog, pipeline: &mut Pipeline) {
    if let Head::Stage(stage) = &mut pipeline.head {
        collect_in_stage(catalog, stage);
    }
    for stage in &mut pipeline.stages {
        collect_in_stage(catalog, stage);
    }
}

fn collect_in_stage(catalog: &Catalog, stage: &mut Stage) {
    let Some(spec) = catalog.command(&stage.name) else {
        return;
    };
    let Some((index, param)) = spec
        .positional()
        .into_iter()
        .enumerate()
        .find(|(_, param)| param.repeated)
    else {
        return;
    };
    let unnamed: Vec<usize> = stage
        .args
        .iter()
        .enumerate()
        .filter(|(_, arg)| arg.name.is_none())
        .map(|(at, _)| at)
        .collect();
    if unnamed.len() <= index {
        return;
    }
    let taken = &unnamed[index..];
    let first = taken[0];
    let items: Vec<Expr> = taken
        .iter()
        .map(|at| stage.args[*at].value.clone())
        .collect();
    let mut position = 0;
    stage.args.retain(|_| {
        position += 1;
        !taken.contains(&(position - 1))
    });
    stage.args.insert(
        first,
        Arg {
            name: Some(param.name.clone()),
            value: Expr::List { items, span: None },
            span: None,
        },
    );
}

fn conversion_error(program: &str, variant: Variant, reason: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Internal,
        format!("example `{program}` does not convert to variant {variant:?}: {reason}"),
    )
}

/// Writes a program given in variant A in `variant`.
///
/// Variant A returns the text unchanged. For the other variants the result
/// is the canonical text of the printer, without the final line break.
///
/// # Errors
///
/// Fails with `E_INTERNAL` when the text is not a program of variant A or
/// has no text form in `variant`.
///
/// # Examples
///
/// ```no_run
/// use nurt_help::catalog::Catalog;
/// use nurt_help::variant::{Variant, convert_program};
///
/// fn show(catalog: &Catalog) -> Result<(), nurt_contracts::error::Error> {
///     let text = convert_program(catalog, "find \"/srv\" |> first 5", Variant::B)?;
///     assert_eq!(text, "find(\"/srv\")\n|> first(5)");
///     Ok(())
/// }
/// ```
pub fn convert_program(
    catalog: &Catalog,
    program: &str,
    variant: Variant,
) -> Result<String, Error> {
    if variant == Variant::A {
        return Ok(program.to_owned());
    }
    let document = parse(program, Variant::A, &ParseOptions::default())
        .map_err(|error| conversion_error(program, variant, error))?;
    let document = if variant == Variant::C {
        let document = collect_repeated(catalog, document);
        print::name_positional(&document, &CatalogNames(catalog))
            .map_err(|error| conversion_error(program, variant, error))?
    } else {
        document
    };
    let text = print::try_print(&document, variant)
        .map_err(|error| conversion_error(program, variant, error))?;
    Ok(text.trim_end().to_owned())
}

/// Writes the programs of the fenced `nurt` blocks of a text in `variant`.
///
/// Text outside the blocks and blocks of other kinds are kept as they are.
///
/// # Errors
///
/// Fails like [`convert_program`] for the first block that does not convert.
pub fn convert_fenced(catalog: &Catalog, text: &str, variant: Variant) -> Result<String, Error> {
    if variant == Variant::A {
        return Ok(text.to_owned());
    }
    let mut out: Vec<String> = Vec::new();
    let mut block: Option<Vec<&str>> = None;
    for line in text.lines() {
        let fence = line.trim_start().starts_with(FENCE);
        match block.take() {
            Some(program) if fence => {
                out.push(convert_program(catalog, &program.join("\n"), variant)?);
                out.push(line.to_owned());
            }
            Some(mut program) => {
                program.push(line);
                block = Some(program);
            }
            None => {
                if line.trim_end() == PROGRAM_FENCE {
                    block = Some(Vec::new());
                }
                out.push(line.to_owned());
            }
        }
    }
    if let Some(program) = block {
        out.extend(program.into_iter().map(str::to_owned));
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::Docs;

    const INDEX: &str = "format = \"nurt-catalog-index/1\"\ntotal = 2\n[[file]]\nname = \"core\"\nfile = \"core.toml\"\nsummary = \"Core.\"\ncommands = 2\n";
    const CORE: &str = "format = \"nurt-catalog/1\"\ngroup = \"core\"\nsummary = \"Core.\"\n\
[[command]]\nname = \"find\"\ncategory = \"source\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"none\"]\noutput = [\"entry\"]\nsummary = \"Find.\"\n\
[[command.param]]\nname = \"path\"\ntype = \"path\"\nposition = 1\nrequired = true\nsummary = \"Root.\"\n\
[[command]]\nname = \"first\"\ncategory = \"transform\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"any\"]\noutput = [\"same\"]\nsummary = \"First.\"\n\
[[command.param]]\nname = \"count\"\ntype = \"int\"\nposition = 1\nrequired = true\nsummary = \"How many.\"\n";

    const SELECT: &str = "[[command]]\nname = \"select\"\ncategory = \"transform\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"any\"]\noutput = [\"any\"]\nsummary = \"Select.\"\n\
[[command.param]]\nname = \"fields\"\ntype = \"list<field | named_expression>\"\nposition = 1\nrequired = true\nrepeated = true\nsummary = \"Fields.\"\n";

    fn catalog_with_select() -> Catalog {
        let index = INDEX
            .replace("total = 2", "total = 3")
            .replace("commands = 2", "commands = 3");
        let core = format!("{CORE}{SELECT}");
        Catalog::from_sources(&index, &[("core.toml", &core)], Docs::empty()).unwrap()
    }

    const SEARCH: &str = "[[command]]\nname = \"search\"\ncategory = \"source\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"none\"]\noutput = [\"match\"]\nsummary = \"Search.\"\n\
[[command.param]]\nname = \"pattern\"\ntype = \"text\"\nposition = 1\nrequired = true\nsummary = \"Pattern.\"\n\
[[command.param]]\nname = \"path\"\ntype = \"list<path>\"\nposition = 2\nrequired = true\nrepeated = true\nsummary = \"Paths.\"\n";

    fn catalog_with_search() -> Catalog {
        let index = INDEX
            .replace("total = 2", "total = 3")
            .replace("commands = 2", "commands = 3");
        let core = format!("{CORE}{SEARCH}");
        Catalog::from_sources(&index, &[("core.toml", &core)], Docs::empty()).unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::from_sources(INDEX, &[("core.toml", CORE)], Docs::empty()).unwrap()
    }

    #[test]
    fn variant_a_is_returned_unchanged() {
        let text = "find   \"/srv\"";
        assert_eq!(convert_program(&catalog(), text, Variant::A).unwrap(), text);
    }

    #[test]
    fn variant_b_uses_call_notation() {
        let text = convert_program(&catalog(), "find \"/srv\" |> first 5", Variant::B).unwrap();
        assert_eq!(text, "find(\"/srv\")\n|> first(5)");
    }

    #[test]
    fn variant_c_names_positional_arguments_from_the_catalog() {
        let text = convert_program(&catalog(), "find \"/srv\" |> first 5", Variant::C).unwrap();
        assert_eq!(text, "find path=\"/srv\"\n|> first count=5");
    }

    #[test]
    fn repeated_parameter_is_written_as_one_list_in_variant_c() {
        let catalog = catalog_with_select();
        let text =
            convert_program(&catalog, "find \"/srv\" |> select path size", Variant::C).unwrap();
        assert_eq!(text, "find path=\"/srv\"\n|> select fields=[path, size]");
    }

    #[test]
    fn repeated_parameter_stays_after_the_positional_parameters_before_it_in_variant_c() {
        let catalog = catalog_with_search();
        let text = convert_program(&catalog, "search \"x\" \"/a\" \"/b\"", Variant::C).unwrap();
        let pattern = text.find("pattern=\"x\"").unwrap();
        let path = text.find("path=[").unwrap();
        assert!(pattern < path, "{text}");
        assert!(text.contains("\"/a\"") && text.contains("\"/b\""), "{text}");
    }

    #[test]
    fn single_item_of_a_repeated_parameter_is_still_a_list_in_variant_c() {
        let catalog = catalog_with_search();
        let text = convert_program(&catalog, "search \"x\" \"/a\"", Variant::C).unwrap();
        assert!(text.contains("path=[\"/a\"]"), "{text}");
    }

    #[test]
    fn text_that_is_not_a_program_is_an_error() {
        let error = convert_program(&catalog(), "find |>", Variant::B).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
    }

    #[test]
    fn unterminated_block_is_kept_as_written() {
        let text = "Before.\n\n```nurt\nfind \"/srv\" |> first 5\nmore text";
        let converted = convert_fenced(&catalog(), text, Variant::B).unwrap();
        assert_eq!(converted, text);
    }

    #[test]
    fn text_without_blocks_is_unchanged_in_every_variant() {
        let text = "Only prose.\n\n    indented\n";
        for variant in [Variant::A, Variant::B, Variant::C] {
            assert_eq!(
                convert_fenced(&catalog(), text.trim_end(), variant).unwrap(),
                text.trim_end()
            );
        }
    }

    #[test]
    fn fenced_blocks_are_converted_and_prose_is_kept() {
        let text = "Before.\n\n```nurt\nfind \"/srv\" |> first 5\n```\n\nAfter.\n\n```text\nfind \"/srv\"\n```";
        let converted = convert_fenced(&catalog(), text, Variant::B).unwrap();
        assert_eq!(
            converted,
            "Before.\n\n```nurt\nfind(\"/srv\")\n|> first(5)\n```\n\nAfter.\n\n```text\nfind \"/srv\"\n```"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
