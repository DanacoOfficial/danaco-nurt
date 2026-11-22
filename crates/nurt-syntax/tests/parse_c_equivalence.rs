// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Equivalence of the variant A and variant C notations.
//!
//! The parser does not know the catalog, so a variant A surface tree keeps
//! positional arguments while the variant C tree names every argument. Once
//! the positional arguments of the variant A tree receive their parameter
//! names, both trees must have the same `ast_hash`. The reference tasks have
//! hand-written variant C texts; the variant C text of an example is the
//! variant C print of its named variant A tree. With `NURT_BLESS=1` a missing
//! example text and its tree are written and must be reviewed before they are
//! kept. After binding, the trees of both notations are equal: the bound
//! golden trees of one program under `bound/a` and `bound/c` have one hash.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{
    Arg, AstDocument, Expr, Head, MapEntry, Pipeline, Stage, Statement, strip_for_hash,
};
use nurt_syntax::hash::ast_hash;
use nurt_syntax::json::{decode_bound, encode_pretty};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use nurt_syntax::print;

/// Positional parameters of the commands used by the golden programs, in
/// signature order, as declared in `catalog/*.toml`.
///
/// A command with several forms has one entry per form; a stage takes the
/// first entry with at least as many parameters as it has positional
/// arguments. `replace` on `match` records takes only the replacement text
/// `with`; on lines and as a source it takes `pattern` and `with`.
const POSITIONAL_PARAMETERS: &[(&str, &[&str])] = &[
    ("archive.create", &["to"]),
    ("archive.extract", &["path"]),
    ("code.references", &["path"]),
    ("code.rename", &["to"]),
    ("code.symbols", &["name"]),
    ("copy", &["from", "to"]),
    ("create", &["path"]),
    ("edit", &["path"]),
    ("filter", &["condition"]),
    ("find", &["path", "glob"]),
    ("first", &["count"]),
    ("git.commit", &["repo"]),
    ("git.status", &["repo"]),
    ("http.download", &["url", "to"]),
    ("http.request", &["url"]),
    ("info", &["path"]),
    ("insert", &["text"]),
    ("jobs", &["id"]),
    ("lines", &["path", "range"]),
    ("move", &["from", "to"]),
    ("notebook.replace", &["path"]),
    ("program.search", &["query"]),
    ("read", &["path"]),
    ("remove", &["path"]),
    ("replace", &["with"]),
    ("replace", &["pattern", "with"]),
    ("run", &["command"]),
    ("schedule.create", &["label"]),
    ("search", &["pattern", "path"]),
    ("select", &["fields"]),
    ("set", &["path"]),
    ("sort", &["by"]),
    ("sql", &["path", "query"]),
    ("test.run", &["path"]),
    ("undo", &["id"]),
    ("verify", &["check"]),
    ("write", &["path", "text"]),
];

/// Repeated positional parameters (`repeated = true` in the catalog). A
/// repeated parameter collects the positional arguments left over by the
/// other positions into one list.
const REPEATED_PARAMETERS: &[(&str, &str)] = &[
    ("copy", "from"),
    ("create", "path"),
    ("git.status", "repo"),
    ("info", "path"),
    ("read", "path"),
    ("remove", "path"),
    ("search", "path"),
    ("select", "fields"),
    ("sort", "by"),
];

/// Number of reference tasks.
const TASK_COUNT: usize = 14;
/// Number of examples.
const EXAMPLE_COUNT: usize = 48;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root exists")
}

fn variant_c_dir() -> PathBuf {
    workspace_root().join("tests/d1/golden/ast/c")
}

fn word_notation_dir() -> PathBuf {
    workspace_root().join("spec/golden/ast")
}

fn bound_dir(variant: &str) -> PathBuf {
    workspace_root().join("tests/d1/golden/bound").join(variant)
}

fn blessing() -> bool {
    std::env::var_os("NURT_BLESS").is_some_and(|value| value == "1")
}

/// Stems of the files of `directory` with `extension` whose name starts with
/// `prefix` and has no further dot, sorted.
fn stems(directory: &Path, extension: &str, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|found| found == extension))
        .map(|path| {
            path.file_stem()
                .expect("file stem")
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with(prefix) && !name.contains('.'))
        .collect();
    names.sort();
    names
}

fn task_names() -> Vec<String> {
    stems(&variant_c_dir(), "nurt", "task-")
}

fn example_names() -> Vec<String> {
    stems(&variant_c_dir(), "nurt", "example-")
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    serde_json::from_str(&read_text(path))
        .unwrap_or_else(|error| panic!("cannot decode {}: {error}", path.display()))
}

fn read_hash(path: &Path) -> String {
    read_text(path).trim_end().to_owned()
}

fn hash_of(document: &AstDocument) -> String {
    ast_hash(document).expect("tree hashes")
}

/// The golden file text of a surface tree: indented JSON without spans and
/// metadata, with a final line feed.
fn golden_tree_text(document: &AstDocument) -> String {
    let value = serde_json::to_value(document).expect("tree serializes");
    format!(
        "{}\n",
        encode_pretty(&strip_for_hash(&value)).expect("tree encodes")
    )
}

fn parse_c(text: &str, name: &str) -> AstDocument {
    parse(text, Variant::C, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{name} does not parse: {error:?}\n{text}"))
}

fn positional_parameters(command: &str, count: usize) -> &'static [&'static str] {
    let mut forms = POSITIONAL_PARAMETERS
        .iter()
        .filter(|(name, _)| *name == command)
        .map(|(_, parameters)| *parameters);
    let first = forms.clone().next().unwrap_or(&[]);
    forms
        .find(|parameters| parameters.len() >= count)
        .unwrap_or(first)
}

fn repeated_parameter(command: &str) -> Option<&'static str> {
    REPEATED_PARAMETERS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, parameter)| *parameter)
}

fn name_document(document: &mut AstDocument) {
    for statement in &mut document.program.statements {
        match statement {
            Statement::Let(binding) => name_pipeline(&mut binding.value),
            Statement::Pipeline(pipeline) => name_pipeline(pipeline),
            other => panic!("unexpected statement {other:?}"),
        }
    }
}

fn named(document: &AstDocument) -> AstDocument {
    let mut copy = document.clone();
    name_document(&mut copy);
    copy
}

fn name_pipeline(pipeline: &mut Pipeline) {
    if let Head::Stage(stage) = &mut pipeline.head {
        name_stage(stage);
    }
    for stage in &mut pipeline.stages {
        name_stage(stage);
    }
}

/// Gives the k-th positional argument of a stage the name of the k-th
/// positional parameter of its command, keeping the written order. When a
/// stage has more positional arguments than its command has positions, the
/// repeated parameter takes the surplus arguments as one list.
fn name_stage(stage: &mut Stage) {
    for argument in &mut stage.args {
        name_expr(&mut argument.value);
    }
    let count = stage.args.iter().filter(|arg| arg.name.is_none()).count();
    if count == 0 {
        return;
    }
    let parameters = positional_parameters(&stage.name, count);
    let surplus = count.saturating_sub(parameters.len());
    let repeated = (surplus > 0).then(|| {
        let parameter = repeated_parameter(&stage.name).unwrap_or_else(|| {
            panic!(
                "`{}` takes {} positional arguments, found {count}",
                stage.name,
                parameters.len()
            )
        });
        parameters
            .iter()
            .position(|name| *name == parameter)
            .expect("the repeated parameter is positional")
    });
    let mut args: Vec<Arg> = Vec::with_capacity(stage.args.len());
    let mut list_index: Option<usize> = None;
    let mut position = 0;
    for mut argument in std::mem::take(&mut stage.args) {
        if argument.name.is_some() {
            args.push(argument);
            continue;
        }
        let slot = match repeated {
            Some(start) if position > start + surplus => position - surplus,
            Some(start) if position >= start => start,
            _ => position,
        };
        position += 1;
        let name = parameters
            .get(slot)
            .unwrap_or_else(|| panic!("no positional parameter {} for `{}`", slot + 1, stage.name));
        if repeated == Some(slot) {
            match list_index {
                Some(index) => match &mut args[index].value {
                    Expr::List { items, .. } => items.push(argument.value),
                    other => panic!("repeated parameter holds {other:?}"),
                },
                None => {
                    list_index = Some(args.len());
                    args.push(Arg {
                        name: Some((*name).to_owned()),
                        value: Expr::List {
                            items: vec![argument.value],
                            span: None,
                        },
                        span: None,
                    });
                }
            }
            continue;
        }
        argument.name = Some((*name).to_owned());
        args.push(argument);
    }
    stage.args = args;
}

fn name_expr(expr: &mut Expr) {
    match expr {
        Expr::Sub { p, .. } => name_pipeline(p),
        Expr::List { items, .. } => items.iter_mut().for_each(name_expr),
        Expr::Map { entries, .. } => entries
            .iter_mut()
            .for_each(|MapEntry { v, .. }| name_expr(v)),
        Expr::Spread { x, .. } | Expr::Un { x, .. } => name_expr(x),
        Expr::Field { of: Some(of), .. } => name_expr(of),
        Expr::Call { args, .. } => args.iter_mut().for_each(name_expr),
        Expr::Bin { l, r, .. } => {
            name_expr(l);
            name_expr(r);
        }
        _ => {}
    }
}

fn stage_args(document: &AstDocument) -> Vec<&Arg> {
    fn collect<'a>(pipeline: &'a Pipeline, out: &mut Vec<&'a Arg>) {
        let head = match &pipeline.head {
            Head::Stage(stage) => Some(stage),
            _ => None,
        };
        for stage in head.into_iter().chain(&pipeline.stages) {
            for argument in &stage.args {
                out.push(argument);
                if let Expr::Sub { p, .. } = &argument.value {
                    collect(p, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for statement in &document.program.statements {
        match statement {
            Statement::Let(binding) => collect(&binding.value, &mut out),
            Statement::Pipeline(pipeline) => collect(pipeline, &mut out),
            other => panic!("unexpected statement {other:?}"),
        }
    }
    out
}

fn word_notation_tree(name: &str) -> AstDocument {
    read_json(&word_notation_dir().join(format!("{name}.json")))
}

/// Writes the variant C text of an example and its tree when they are
/// missing; returns `false` when a file is missing and blessing is off.
fn write_example(name: &str) -> bool {
    let text_path = variant_c_dir().join(format!("{name}.nurt"));
    let tree_path = variant_c_dir().join(format!("{name}.json"));
    if text_path.exists() && tree_path.exists() {
        return true;
    }
    if !blessing() {
        return false;
    }
    if !text_path.exists() {
        let text = print::try_print(&named(&word_notation_tree(name)), Variant::C)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        fs::write(&text_path, text).expect("variant C text is written");
    }
    let document = parse_c(&read_text(&text_path), name);
    fs::write(&tree_path, golden_tree_text(&document)).expect("variant C tree is written");
    true
}

#[test]
fn every_reference_task_has_a_variant_c_golden_tree() {
    assert_eq!(task_names().len(), TASK_COUNT);
}

#[test]
fn every_example_has_a_variant_c_text() {
    let examples = stems(&word_notation_dir(), "json", "example-");
    assert_eq!(examples.len(), EXAMPLE_COUNT);
    let missing: Vec<&String> = examples
        .iter()
        .filter(|name| !write_example(name))
        .collect();
    assert!(
        missing.is_empty(),
        "no variant C text for {missing:?}; run the test with NURT_BLESS=1 and review the result"
    );
}

#[test]
fn variant_c_golden_trees_name_every_argument() {
    for name in task_names().iter().chain(&example_names()) {
        let document: AstDocument = read_json(&variant_c_dir().join(format!("{name}.json")));
        for argument in stage_args(&document) {
            assert!(
                argument.name.is_some(),
                "{name}: positional argument {:?}",
                argument.value
            );
        }
    }
}

#[test]
fn variant_c_golden_trees_match_recorded_hashes() {
    for name in task_names().iter().chain(&example_names()) {
        let document: AstDocument = read_json(&variant_c_dir().join(format!("{name}.json")));
        let recorded = read_hash(&variant_c_dir().join(format!("{name}.hash")));
        assert_eq!(hash_of(&document), recorded, "{name}");
    }
}

#[test]
fn variant_c_golden_tree_files_have_the_golden_layout() {
    for name in task_names().iter().chain(&example_names()) {
        let document = parse_c(
            &read_text(&variant_c_dir().join(format!("{name}.nurt"))),
            name,
        );
        assert_eq!(
            golden_tree_text(&document),
            read_text(&variant_c_dir().join(format!("{name}.json"))),
            "{name}"
        );
    }
}

#[test]
fn named_variant_a_trees_equal_variant_c_trees() {
    for name in task_names() {
        let variant_a = named(&word_notation_tree(&name));
        let variant_c: AstDocument = read_json(&variant_c_dir().join(format!("{name}.json")));
        assert_eq!(hash_of(&variant_a), hash_of(&variant_c), "{name}");
    }
}

#[test]
fn variant_c_examples_parse_to_named_variant_a_trees() {
    let names = example_names();
    assert_eq!(names.len(), EXAMPLE_COUNT);
    for name in &names {
        let variant_a = named(&word_notation_tree(name));
        let variant_c = parse_c(
            &read_text(&variant_c_dir().join(format!("{name}.nurt"))),
            name,
        );
        let expected = serde_json::to_value(&variant_a).expect("tree serializes");
        let actual = serde_json::to_value(&variant_c).expect("tree serializes");
        assert_eq!(strip_for_hash(&actual), strip_for_hash(&expected), "{name}");
        assert_eq!(hash_of(&variant_c), hash_of(&variant_a), "{name}");
    }
}

#[test]
fn variant_c_examples_are_the_canonical_print_of_their_trees() {
    for name in example_names() {
        let text = read_text(&variant_c_dir().join(format!("{name}.nurt")));
        let document = parse_c(&text, &name);
        assert_eq!(print::print(&document, Variant::C), text, "{name}");
    }
}

#[test]
fn naming_changes_the_hash_of_trees_with_positional_arguments() {
    let variant_a = word_notation_tree("task-03-remove-temporary-files");
    assert_ne!(hash_of(&variant_a), hash_of(&named(&variant_a)));
}

#[test]
fn surplus_positional_arguments_fill_the_repeated_parameter() {
    let document = parse(
        "copy \"/a\" \"/b\" \"/dst\"\n",
        Variant::A,
        &ParseOptions::default(),
    )
    .expect("variant A text parses");
    let text = print::try_print(&named(&document), Variant::C).expect("named tree prints");
    assert_eq!(text, "copy from=[\"/a\", \"/b\"] to=\"/dst\"\n");
}

#[test]
fn every_bound_task_of_variant_a_is_bound_in_variant_c() {
    let variant_a = stems(&bound_dir("a"), "json", "task-");
    assert_eq!(variant_a.len(), TASK_COUNT);
    assert_eq!(stems(&bound_dir("c"), "json", "task-"), variant_a);
}

#[test]
fn bound_trees_of_variant_c_have_the_hashes_of_variant_a() {
    let names = stems(&bound_dir("c"), "json", "");
    assert!(names.len() >= TASK_COUNT, "found {} trees", names.len());
    for name in &names {
        let mut hashes = Vec::new();
        for variant in ["a", "c"] {
            let path = bound_dir(variant).join(format!("{name}.json"));
            let tree = decode_bound(&read_text(&path))
                .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()));
            let hash = ast_hash(&tree).expect("bound tree hashes");
            let recorded = read_hash(&bound_dir(variant).join(format!("{name}.hash")));
            assert_eq!(hash, recorded, "{}", path.display());
            hashes.push(hash);
        }
        assert_eq!(
            hashes[0], hashes[1],
            "{name}: variant C binds to another tree"
        );
    }
}
