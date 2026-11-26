// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Well-formedness conformance of XML reading.
//!
//! The repository suite `tests/d5/data/u08/conformance.toml` runs by default.
//! The W3C XML Conformance Test Suite is not redistributable; the ignored
//! test `w3c_suite_classifies_well_formedness` downloads it into a temporary
//! directory, checks its SHA-256 digest and applies the same criteria. It
//! needs network access and the programs `curl`, `sha256sum` and `tar`.
// Helper functions of this test target are not `#[test]` functions, which
// `allow-unwrap-in-tests` covers.
#![allow(clippy::unwrap_used)]

mod u08_support;

use std::path::{Path, PathBuf};
use std::process::Command;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_doc::formats::xml::encoding::parse_document;
use u08_support::Codec;

/// Minimum number of cases of the repository suite.
const MIN_CASES: usize = 200;

const W3C_ARCHIVE_URL: &str = "https://www.w3.org/XML/Test/xmlts20130923.tar.gz";
const W3C_ARCHIVE_SHA256: &str = "9b61db9f5dbffa545f4b8d78422167083a8568c59bd1129f94138f936cf6fc1f";

/// Test catalogs of XML 1.0 in the W3C archive. The Japanese collection is
/// left out because it uses encodings that are not ASCII-compatible.
const W3C_CATALOGS: [&str; 13] = [
    "xmltest/xmltest.xml",
    "sun/sun-valid.xml",
    "sun/sun-invalid.xml",
    "sun/sun-not-wf.xml",
    "oasis/oasis.xml",
    "ibm/ibm_oasis_invalid.xml",
    "ibm/ibm_oasis_not-wf.xml",
    "ibm/ibm_oasis_valid.xml",
    "eduni/errata-2e/errata2e.xml",
    "eduni/errata-3e/errata3e.xml",
    "eduni/errata-4e/errata4e.xml",
    "eduni/misc/ht-bh.xml",
    "eduni/namespaces/1.0/rmt-ns10.xml",
];

/// Reads a document the way the product does: decoding, parsing and
/// installing the entities of the document type declaration, then reading
/// every value. A value that cannot be read without external entities does
/// not make the document malformed.
fn read(bytes: &[u8]) -> Result<()> {
    let (_, tree) = parse_document(bytes, Some(&Codec))?;
    match tree.to_value() {
        Err(error) if error.code() == ErrorCode::UnsupportedConstruct => Ok(()),
        other => other.map(drop),
    }
}

struct Case {
    id: String,
    wf: bool,
    xml: String,
}

fn repository_cases() -> Vec<Case> {
    let path = u08_support::data_dir().join("conformance.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let document: toml_edit::DocumentMut = text.parse().unwrap();
    let cases = document["case"].as_array_of_tables().unwrap();
    cases
        .iter()
        .map(|case| Case {
            id: case["id"].as_str().unwrap().to_owned(),
            wf: case["wf"].as_bool().unwrap(),
            xml: case["xml"].as_str().unwrap().to_owned(),
        })
        .collect()
}

/// Describes a misclassified case, or returns `None` for a correct one.
/// Tests of the catalog of Namespaces in XML 1.0 that expect a namespace
/// error. The parser leaves a name with an undeclared prefix or an invalid
/// qualified name without a namespace instead of failing, so that documents
/// written for tools without namespace support stay editable (see the module
/// documentation of `formats::xml::ns`). The suite pins this decision: each of
/// these documents must be accepted, and a change of the behavior fails the
/// test.
const NAMESPACE_LENIENT: [&str; 20] = [
    "rmt-ns10-009",
    "rmt-ns10-010",
    "rmt-ns10-011",
    "rmt-ns10-012",
    "rmt-ns10-013",
    "rmt-ns10-014",
    "rmt-ns10-015",
    "rmt-ns10-016",
    "rmt-ns10-023",
    "rmt-ns10-025",
    "rmt-ns10-026",
    "rmt-ns10-029",
    "rmt-ns10-030",
    "rmt-ns10-031",
    "rmt-ns10-032",
    "rmt-ns10-033",
    "rmt-ns10-036",
    "rmt-ns10-042",
    "rmt-ns10-043",
    "rmt-ns10-044",
];

fn misclassified(id: &str, wf: bool, result: &Result<()>) -> Option<String> {
    if NAMESPACE_LENIENT.contains(&id) {
        return result
            .is_err()
            .then(|| format!("{id}: the documented acceptance of a namespace error changed"));
    }
    match (wf, result) {
        (true, Ok(())) => None,
        (false, Err(error)) if error.code() == ErrorCode::DocParse => None,
        (true, Err(error)) => Some(format!("{id}: well-formed document rejected: {error}")),
        (false, Ok(())) => Some(format!("{id}: malformed document accepted")),
        (false, Err(error)) => Some(format!(
            "{id}: rejected with {error} instead of E_DOC_PARSE"
        )),
    }
}

#[test]
fn repository_suite_has_unique_identifiers_and_both_classes() {
    let cases = repository_cases();
    assert!(cases.len() >= MIN_CASES, "{} cases", cases.len());
    let mut ids: Vec<&str> = cases.iter().map(|case| case.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), cases.len());
    assert!(cases.iter().any(|case| case.wf));
    assert!(cases.iter().any(|case| !case.wf));
    for case in &cases {
        assert_eq!(case.id.starts_with("wf-"), case.wf, "{}", case.id);
    }
}

#[test]
fn repository_suite_accepts_well_formed_and_rejects_malformed_documents() {
    let failures: Vec<String> = repository_cases()
        .iter()
        .filter_map(|case| misclassified(&case.id, case.wf, &read(case.xml.as_bytes())))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn malformed_documents_are_rejected_with_a_location() {
    for case in repository_cases().iter().filter(|case| !case.wf) {
        if let Err(error) = read(case.xml.as_bytes()) {
            let location = error.data().location.as_ref();
            assert!(
                location.is_some_and(|location| location.byte.is_some()),
                "{}: {error}",
                case.id
            );
        }
    }
}

struct W3cTest {
    id: String,
    kind: String,
    path: PathBuf,
}

/// Selects the tests that apply to a processor that reads no external
/// entity: XML 1.0, fifth edition, namespaces, no external entities.
fn applies(attributes: &[(String, String)]) -> bool {
    let get = |name: &str| {
        attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    get("ENTITIES").is_none_or(|entities| entities == "none")
        && get("VERSION").is_none_or(|version| version.split(' ').any(|v| v == "1.0"))
        && get("EDITION").is_none_or(|edition| edition.split(' ').any(|e| e == "5"))
        && get("NAMESPACE") != Some("no")
        && matches!(get("TYPE"), Some("valid" | "invalid" | "not-wf"))
}

/// The Sun catalogs are sequences of `TEST` elements without a root element:
/// the suite includes them as external entities of `xmlconf.xml`. A sequence
/// is wrapped in a `TESTCASES` element, placed after the XML declaration, so
/// that it is a document; a catalog that has the element is left as it is.
fn with_root(bytes: Vec<u8>) -> Vec<u8> {
    if bytes.windows(10).any(|window| window == b"<TESTCASES") {
        return bytes;
    }
    let start = if bytes.starts_with(b"<?xml") {
        bytes
            .windows(2)
            .position(|window| window == b"?>")
            .map_or(0, |end| end + 2)
    } else {
        0
    };
    let mut wrapped = bytes[..start].to_vec();
    wrapped.extend_from_slice(b"<TESTCASES>");
    wrapped.extend_from_slice(&bytes[start..]);
    wrapped.extend_from_slice(b"</TESTCASES>");
    wrapped
}

fn catalog_tests(root: &Path, catalog: &str) -> Vec<W3cTest> {
    let path = root.join(catalog);
    let base = path.parent().unwrap().to_path_buf();
    let bytes = with_root(std::fs::read(&path).unwrap());
    let (_, tree) = parse_document(&bytes, Some(&Codec)).unwrap();
    let mut tests = Vec::new();
    for id in tree.node_ids() {
        let Some(element) = tree.element(id).filter(|_| tree.name(id) == "TEST") else {
            continue;
        };
        let attributes: Vec<(String, String)> = element
            .attributes
            .iter()
            .map(|attribute| {
                (
                    tree.text()[attribute.name.clone()].to_owned(),
                    tree.attribute_value(attribute).unwrap(),
                )
            })
            .collect();
        if applies(&attributes) {
            let get = |name: &str| {
                attributes
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
                    .unwrap()
            };
            tests.push(W3cTest {
                id: get("ID"),
                kind: get("TYPE"),
                path: base.join(get("URI")),
            });
        }
    }
    tests
}

fn run(program: &str, args: &[&str]) -> String {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(output.status.success(), "{program} failed: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
#[ignore = "downloads the W3C XML Conformance Test Suite; needs network access, curl, sha256sum and tar"]
fn w3c_suite_classifies_well_formedness() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("xmlts.tar.gz");
    let archive_path = archive.to_str().unwrap();
    run("curl", &["-sSfL", "-o", archive_path, W3C_ARCHIVE_URL]);
    let digest = run("sha256sum", &[archive_path]);
    assert!(
        digest.starts_with(W3C_ARCHIVE_SHA256),
        "unexpected digest {digest}"
    );
    run(
        "tar",
        &["-xzf", archive_path, "-C", dir.path().to_str().unwrap()],
    );
    let root = dir.path().join("xmlconf");
    let mut failures = Vec::new();
    let mut count = 0;
    for catalog in W3C_CATALOGS {
        for test in catalog_tests(&root, catalog) {
            count += 1;
            let bytes = std::fs::read(&test.path).unwrap();
            let result = read(&bytes);
            if let Some(failure) = misclassified(&test.id, test.kind != "not-wf", &result) {
                failures.push(failure);
            }
        }
    }
    u08_support::write_summary(
        "u08-w3c-conformance.json",
        &serde_json::json!({ "tests": count, "failures": failures }),
    );
    assert!(count > 0);
    assert!(
        failures.is_empty(),
        "{} of {count} tests:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
