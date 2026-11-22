// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Conformance tests of the syntax tree contracts `nurt-ast/1` and
//! `nurt-bound/1`: golden files, schemas, JSON and CBOR round trips and
//! `ast_hash`.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{self, AST_HASH_PREFIX, AST_VERSION, AstDocument, Span};
use nurt_contracts::bound::{
    BOUND_VERSION, BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundStage, BoundStatement,
    Phase,
};
use nurt_contracts::kinds::EffectKind;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const GENERATED_SCHEMA: &str = "schema.generated.json";
const AST_SCHEMA: &str = "nurt-ast-1.schema.json";
const BOUND_SCHEMA: &str = "nurt-bound-1.schema.json";

/// Smallest number of golden task programs (`task-*`).
const MIN_TASK_PROGRAMS: usize = 14;
/// Smallest number of golden example programs (`example-*`).
const MIN_EXAMPLE_PROGRAMS: usize = 48;
/// Smallest number of golden bound trees.
const MIN_BOUND_TREES: usize = 12;

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec")
}

fn golden_dir(format: &str) -> PathBuf {
    spec_dir().join("golden").join(format)
}

fn bless() -> bool {
    std::env::var_os("NURT_BLESS").is_some_and(|value| value == "1")
}

fn read_json(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Returns the golden tree documents of a format, sorted by name.
fn golden_trees(format: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(golden_dir(format))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
                && path
                    .file_name()
                    .is_some_and(|name| name != GENERATED_SCHEMA)
        })
        .collect();
    files.sort();
    files
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

fn validator(schema: &Value) -> jsonschema::Validator {
    jsonschema::validator_for(schema).unwrap()
}

fn committed_validator(file: &str) -> jsonschema::Validator {
    validator(&read_json(&spec_dir().join("schemas").join(file)))
}

fn assert_valid(validator: &jsonschema::Validator, instance: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "{what} is invalid:\n{}",
        errors.join("\n")
    );
}

fn ast_hash(document: &Value) -> String {
    let canonical = serde_json_canonicalizer::to_vec(&ast::strip_for_hash(document)).unwrap();
    format!(
        "{AST_HASH_PREFIX}{}",
        hex::encode(&Sha256::digest(&canonical)[..])
    )
}

fn check_hash(path: &Path, document: &Value) {
    let hash_path = path.with_extension("hash");
    let computed = ast_hash(document);
    if bless() {
        fs::write(&hash_path, format!("{computed}\n")).unwrap();
        return;
    }
    let recorded = fs::read_to_string(&hash_path)
        .unwrap_or_else(|error| panic!("{}: {error}", hash_path.display()));
    assert_eq!(
        recorded.trim(),
        computed,
        "{}: ast_hash differs from the recorded value",
        path.display()
    );
}

fn round_trip<T>(path: &Path, document: &Value) -> T
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let typed: T = serde_json::from_value(document.clone())
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let back = serde_json::to_value(&typed).unwrap();
    assert_eq!(
        &back,
        document,
        "{}: JSON round trip changed the document",
        path.display()
    );

    let mut cbor = Vec::new();
    ciborium::into_writer(&typed, &mut cbor).unwrap();
    let decoded: T = ciborium::from_reader(cbor.as_slice())
        .unwrap_or_else(|error| panic!("{}: CBOR: {error}", path.display()));
    assert_eq!(
        decoded,
        typed,
        "{}: CBOR round trip changed the tree",
        path.display()
    );
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        *document,
        "{}: CBOR round trip changed the document",
        path.display()
    );
    typed
}

fn check_generated_schema(schema: &schemars::Schema, format: &str) -> Value {
    let generated = serde_json::to_value(schema).unwrap();
    let path = golden_dir(format).join(GENERATED_SCHEMA);
    if bless() {
        let text = serde_json::to_string_pretty(&generated).unwrap();
        fs::write(&path, format!("{text}\n")).unwrap();
        return generated;
    }
    let committed = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing; generate it with NURT_BLESS=1",
            path.display()
        )
    });
    let committed: Value = serde_json::from_str(&committed).unwrap();
    assert_eq!(
        committed,
        generated,
        "{} differs from the schema generated from the Rust types; review the change and \
         regenerate it with NURT_BLESS=1",
        path.display()
    );
    generated
}

#[test]
fn surface_goldens_are_valid_and_round_trip() {
    let approved = committed_validator(AST_SCHEMA);
    let generated = validator(&check_generated_schema(
        &schemars::schema_for!(AstDocument),
        "ast",
    ));
    let files = golden_trees("ast");
    let tasks = files
        .iter()
        .filter(|path| stem(path).starts_with("task-"))
        .count();
    let examples = files
        .iter()
        .filter(|path| stem(path).starts_with("example-"))
        .count();
    assert!(
        tasks >= MIN_TASK_PROGRAMS,
        "expected at least {MIN_TASK_PROGRAMS} task programs, found {tasks}"
    );
    assert!(
        examples >= MIN_EXAMPLE_PROGRAMS,
        "expected at least {MIN_EXAMPLE_PROGRAMS} example programs, found {examples}"
    );

    for path in &files {
        let document = read_json(path);
        let what = path.display().to_string();
        assert_valid(&approved, &document, &what);
        assert_valid(&generated, &document, &what);
        assert_eq!(document["nurt"], AST_VERSION, "{what}");
        let _: AstDocument = round_trip(path, &document);
        check_hash(path, &document);
        let name = stem(path);
        if ["task-", "example-", "extra-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            let text = path.with_extension("nurt");
            assert!(
                text.is_file(),
                "{}: program text is missing",
                text.display()
            );
        }
    }
}

#[test]
fn bound_goldens_are_valid_and_round_trip() {
    let approved = committed_validator(BOUND_SCHEMA);
    let generated = validator(&check_generated_schema(
        &schemars::schema_for!(BoundDocument),
        "bound",
    ));
    let files = golden_trees("bound");
    assert!(
        files.len() >= MIN_BOUND_TREES,
        "expected at least {MIN_BOUND_TREES} bound trees, found {}",
        files.len()
    );

    for path in &files {
        let document = read_json(path);
        let what = path.display().to_string();
        assert_valid(&approved, &document, &what);
        assert_valid(&generated, &document, &what);
        assert_eq!(document["bound"], BOUND_VERSION, "{what}");
        let _: BoundDocument = round_trip(path, &document);
        check_hash(path, &document);
        let surface = golden_dir("ast").join(path.file_name().unwrap());
        assert!(
            surface.is_file(),
            "{}: surface tree is missing",
            surface.display()
        );
    }
}

#[test]
fn surface_schema_rejects_bound_trees() {
    let surface = committed_validator(AST_SCHEMA);
    for path in golden_trees("bound") {
        let document = read_json(&path);
        assert!(
            !surface.is_valid(&document),
            "{}: accepted as nurt-ast/1",
            path.display()
        );
        assert!(
            serde_json::from_value::<AstDocument>(document).is_err(),
            "{}: deserialized as AstDocument",
            path.display()
        );
    }

    let base = read_json(&golden_dir("ast").join("task-02-replace-identifier-expected-count.json"));
    assert!(surface.is_valid(&base));
    let mut with_root_fields = base.clone();
    with_root_fields["bound"] = json!("1.0");
    with_root_fields["catalog_rev"] = json!("cat-1");
    with_root_fields["corr_rev"] = json!("corr-1");
    let mut with_effect = base.clone();
    with_effect["program"]["statements"][0]["head"]["effect"] = json!("read");
    let mut with_default = base.clone();
    with_default["program"]["statements"][0]["head"]["args"][0]["default"] = json!(true);
    let mut with_flag = base.clone();
    with_flag["program"]["statements"][0]["head"]["args"][0]["value"] =
        json!({"t": "regex", "v": "oldName\\(", "flags": "u"});
    let mut with_typed_text = base.clone();
    with_typed_text["program"]["statements"][0]["head"]["args"][1]["value"] =
        json!({"t": "path", "v": "/srv/app/src"});
    let mut with_spread = base;
    with_spread["program"]["statements"][0]["head"]["args"][1]["value"] =
        json!({"t": "spread", "x": {"t": "ref", "name": "recent"}});
    for (what, document) in [
        ("bound, catalog_rev and corr_rev", with_root_fields),
        ("stage effect", with_effect),
        ("argument default marker", with_default),
        ("regex flag outside imsx", with_flag),
        ("typed path node", with_typed_text),
        ("spread outside a list", with_spread),
    ] {
        assert!(!surface.is_valid(&document), "nurt-ast/1 accepted {what}");
    }
}

#[test]
fn bound_schema_rejects_surface_trees() {
    let bound = committed_validator(BOUND_SCHEMA);
    for path in golden_trees("ast") {
        let document = read_json(&path);
        assert!(
            !bound.is_valid(&document),
            "{}: accepted as nurt-bound/1",
            path.display()
        );
        assert!(
            serde_json::from_value::<BoundDocument>(document).is_err(),
            "{}: deserialized as BoundDocument",
            path.display()
        );
    }
}

#[test]
fn hash_ignores_layout_spans_and_meta() {
    let surface = |name: &str| read_json(&golden_dir("ast").join(format!("{name}.json")));
    let multiline_and_single_line = [
        ("task-01-largest-go-files", "example-01-largest-go-files"),
        (
            "task-02-replace-identifier-expected-count",
            "example-02-replace-identifier-expected-count",
        ),
        (
            "task-05-rename-function",
            "example-18-rename-function-syntax",
        ),
        (
            "task-09-insert-line-after-line",
            "example-21-insert-line-after-line",
        ),
        ("task-10-move-old-logs", "example-27-move-by-date"),
        (
            "task-12-replace-verify-rollback",
            "example-26-replace-verify-rollback",
        ),
    ];
    for (one, other) in multiline_and_single_line {
        let (one_tree, other_tree) = (surface(one), surface(other));
        assert_ne!(
            one_tree, other_tree,
            "{one} and {other} must differ in spans"
        );
        assert_eq!(
            ast_hash(&one_tree),
            ast_hash(&other_tree),
            "{one} and {other}"
        );
    }
    let same_text = [
        (
            "example-02-replace-identifier-expected-count",
            "example-03-more-matches-than-expected",
        ),
        ("task-04-set-json-key", "example-17-json-key-layout"),
    ];
    for (one, other) in same_text {
        assert_eq!(
            ast_hash(&surface(one)),
            ast_hash(&surface(other)),
            "{one} and {other}"
        );
    }
    let different = [
        ("task-11-extract-archive", "example-33-archives"),
        ("task-03-remove-temporary-files", "task-01-largest-go-files"),
        ("task-14-background-job", "example-30-background-job"),
    ];
    for (one, other) in different {
        assert_ne!(
            ast_hash(&surface(one)),
            ast_hash(&surface(other)),
            "{one} and {other}"
        );
    }

    let mut with_meta = surface("task-03-remove-temporary-files");
    let plain = ast_hash(&with_meta);
    with_meta["meta"] = json!({"session": "s-1", "source": "find"});
    assert_eq!(ast_hash(&with_meta), plain);
}

#[test]
fn strip_for_hash_keeps_user_data() {
    let document = json!({
        "nurt": "1.0",
        "meta": {"session": "s-1"},
        "program": {"statements": [{
            "t": "pipeline",
            "head": {"t": "stage", "name": "edit", "args": [{
                "name": "changes",
                "value": {"t": "map", "entries": [{"k": "span", "v": {"t": "text", "v": "meta", "span": [1, 1, 1, 7]}}]},
                "span": [1, 6, 1, 20]
            }], "span": [1, 1, 1, 20]},
            "stages": [],
            "span": [1, 1, 1, 20],
            "meta": "kept below the root"
        }]}
    });
    let expected = json!({
        "nurt": "1.0",
        "program": {"statements": [{
            "t": "pipeline",
            "head": {"t": "stage", "name": "edit", "args": [{
                "name": "changes",
                "value": {"t": "map", "entries": [{"k": "span", "v": {"t": "text", "v": "meta"}}]}
            }]},
            "stages": [],
            "meta": "kept below the root"
        }]}
    });
    assert_eq!(ast::strip_for_hash(&document), expected);
}

fn pipelines(document: &BoundDocument) -> Vec<&BoundPipeline> {
    fn collect<'a>(pipeline: &'a BoundPipeline, out: &mut Vec<&'a BoundPipeline>) {
        out.push(pipeline);
        let head = match &pipeline.head {
            BoundHead::Stage(stage) => Some(stage),
            _ => None,
        };
        for stage in head.into_iter().chain(&pipeline.stages) {
            for arg in &stage.args {
                if let BoundExpr::Sub { p } = &arg.value {
                    collect(p, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for statement in &document.program.statements {
        match statement {
            BoundStatement::Let(binding) => collect(&binding.value, &mut out),
            BoundStatement::Pipeline(pipeline) => collect(pipeline, &mut out),
            _ => panic!("unknown statement kind"),
        }
    }
    out
}

/// Phase sequence of one pipeline: whether the effect stage and `verify`
/// have been seen.
#[derive(Default)]
struct PhaseState {
    effect_seen: bool,
    verify_seen: bool,
}

impl PhaseState {
    fn check(&mut self, what: &str, index: usize, stage: &BoundStage) {
        let phase = stage.phase;
        assert!(!self.verify_seen, "{what}: stage after verify");
        if !phase.is_effect() {
            assert_eq!(
                stage.effect,
                EffectKind::Read,
                "{what}: {phase:?} with an effect"
            );
        }
        match phase {
            Phase::Mutate | Phase::Run => {
                assert!(!self.effect_seen, "{what}: second effect stage");
                self.effect_seen = true;
            }
            Phase::After => assert!(self.effect_seen, "{what}: after without an effect"),
            Phase::Verify => {
                assert!(self.effect_seen, "{what}: verify without an effect");
                let check = stage.args.iter().find(|arg| arg.name == "check");
                assert!(
                    check.is_some_and(|arg| matches!(arg.value, BoundExpr::Sub { .. })),
                    "{what}: verify without a sub-pipeline"
                );
                self.verify_seen = true;
            }
            Phase::Source => assert_eq!(index, 0, "{what}: source after the head"),
            _ => assert!(!self.effect_seen, "{what}: {phase:?} after the effect"),
        }
    }
}

fn check_args(what: &str, stage: &BoundStage) {
    let mut names: Vec<&str> = stage.args.iter().map(|arg| arg.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), stage.args.len(), "{what}: duplicate parameter");
    for arg in &stage.args {
        assert!(
            !(arg.default && arg.span.is_some()),
            "{what}: default with a span"
        );
    }
}

#[test]
fn bound_goldens_follow_phase_rules() {
    for path in golden_trees("bound") {
        let document: BoundDocument = serde_json::from_value(read_json(&path)).unwrap();
        for pipeline in pipelines(&document) {
            let head = match &pipeline.head {
                BoundHead::Stage(stage) => Some(stage),
                _ => None,
            };
            let mut state = PhaseState::default();
            for (index, stage) in head.into_iter().chain(&pipeline.stages).enumerate() {
                let what = format!("{}: stage {index} `{}`", path.display(), stage.name);
                state.check(&what, index, stage);
                check_args(&what, stage);
            }
        }
    }
}

#[test]
fn after_effect_stages_are_bound_as_after() {
    let load = |name: &str| -> BoundDocument {
        serde_json::from_value(read_json(&golden_dir("bound").join(format!("{name}.json"))))
            .unwrap()
    };
    let phases = |document: &BoundDocument| -> Vec<(String, Phase)> {
        let BoundStatement::Pipeline(pipeline) = &document.program.statements[0] else {
            panic!("expected a pipeline");
        };
        let BoundHead::Stage(head) = &pipeline.head else {
            panic!("expected a stage head");
        };
        std::iter::once(head)
            .chain(&pipeline.stages)
            .map(|stage| (stage.name.clone(), stage.phase))
            .collect()
    };
    let run = load("task-07-tests-exit-code");
    assert_eq!(
        phases(&run),
        [
            ("run".to_owned(), Phase::Run),
            ("assert".to_owned(), Phase::After)
        ]
    );
    let tests = load("extra-01-test-results-after-run");
    assert_eq!(
        phases(&tests),
        [
            ("test.run".to_owned(), Phase::Run),
            ("filter".to_owned(), Phase::After)
        ]
    );
    let checked = load("task-02-replace-identifier-expected-count");
    assert_eq!(
        phases(&checked),
        [
            ("search".to_owned(), Phase::Source),
            ("assert".to_owned(), Phase::Check),
            ("replace".to_owned(), Phase::Mutate),
        ]
    );
}

#[test]
fn span_rejects_invalid_positions() {
    let span: Span = serde_json::from_value(json!([3, 7, 3, 7])).unwrap();
    assert_eq!(serde_json::to_value(span).unwrap(), json!([3, 7, 3, 7]));
    for invalid in [
        json!([0, 1, 1, 2]),
        json!([2, 1, 1, 5]),
        json!([1, 5, 1, 4]),
        json!([1, 2, 3]),
    ] {
        assert!(
            serde_json::from_value::<Span>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
}
