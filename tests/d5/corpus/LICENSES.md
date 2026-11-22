# Test data licenses

Every file under `tests/d5/` belongs to exactly one of the sets listed below.
A set records its directory, origin, license and copyright holder. Files of
unknown origin or license are not accepted. Directories are relative to
`tests/d5/`.

## Files authored for Danaco Nurt

| Field | Value |
|---|---|
| Directories | `corpus/`, `data/`, `differential/` |
| Origin | written for the Danaco Nurt test suite; `differential/yq/xml.jsonl`, `differential/yq/json.jsonl` and `differential/yq/yaml.jsonl` record the output of `yq` for these files |
| License | Apache-2.0, the license of the repository (`LICENSE`) |

## Third-party conformance suites

Each third-party suite is stored under `third_party/` with the upstream
`LICENSE` file in its directory. Files are copied without modification.

### JSONTestSuite

| Field | Value |
|---|---|
| Directory | `third_party/json/jsontestsuite/` |
| Upstream | <https://github.com/nst/JSONTestSuite> (directory `test_parsing`) |
| License | MIT (`third_party/json/jsontestsuite/LICENSE`) |
| Copyright | Copyright (c) 2016 Nicolas Seriot |
| Cases | 318 files: `y_` accepted, `n_` rejected, `i_` implementation-defined |
| Modifications | none |

### toml-test

| Field | Value |
|---|---|
| Directory | `third_party/toml/toml-test/` |
| Upstream | <https://github.com/toml-lang/toml-test> (directory `tests`, subdirectories `valid` and `invalid`) |
| License | MIT (`third_party/toml/toml-test/LICENSE`) |
| Copyright | Copyright (c) 2013 TOML authors; Copyright (c) 2018 TOML authors |
| Cases | 756 files; each valid `.toml` file has the expected value in the `.json` file of the same name |
| Modifications | none |
| Derived files | `differential/yq/toml.jsonl` holds the values of the valid files as written by `yq`; it is covered by the same license |

### YAML Test Suite

| Field | Value |
|---|---|
| Directory | `third_party/yaml/yaml-test-suite/` |
| Upstream | <https://github.com/yaml/yaml-test-suite> (source format, one file per test identifier) |
| License | MIT (`third_party/yaml/yaml-test-suite/LICENSE`) |
| Copyright | Copyright (c) 2016-2020 Ingy döt Net |
| Cases | 351 files, 403 test cases |
| Modifications | none |

## Suites not included

### W3C XML Conformance Test Suite

| Field | Value |
|---|---|
| Upstream | <https://www.w3.org/XML/Test/> (archive `xmlts20130923.tar.gz`) |
| Reason | the archive has no single open license: the James Clark collection may be redistributed only as the unmodified `xmltest.zip`, and the IBM and Sun collections are published with "All rights reserved" |
| Use | the ignored test `w3c_suite_classifies_well_formedness` in `crates/nurt-doc/tests/u08_xml_conformance.rs` downloads the archive into a temporary directory, verifies its SHA-256 digest and runs the cases; no file of the suite is stored in the repository |
