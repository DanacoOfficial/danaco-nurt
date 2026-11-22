# Test corpus licenses

Every file under `tests/d5/corpus/` belongs to exactly one of the sets listed
below. A set records its directory, upstream project, license and copyright
holder. Files of unknown origin or license are not accepted into the corpus.

## Files authored for Danaco Nurt

| Field | Value |
|---|---|
| Directories | every directory not listed in another section |
| Origin | written for the Danaco Nurt test suite |
| License | Apache-2.0, the license of the repository (`LICENSE`) |

## Third-party conformance suites

Each third-party suite keeps the upstream `LICENSE` file in its directory.
Files are copied without modification unless the section states otherwise.

### YAML Test Suite

| Field | Value |
|---|---|
| Directory | `yaml/yaml-test-suite/` |
| Upstream | <https://github.com/yaml/yaml-test-suite> (source format, one file per test identifier) |
| License | MIT (`yaml/yaml-test-suite/LICENSE`) |
| Copyright | Copyright (c) 2016-2020 Ingy döt Net |
| Cases | 351 files, 403 test cases |
| Modifications | none |

## Suites not included

### W3C XML Conformance Test Suite

| Field | Value |
|---|---|
| Upstream | <https://www.w3.org/XML/Test/> (archive `xmlts20130923.tar.gz`) |
| Reason | the archive has no single open license: the James Clark collection may be redistributed only as the unmodified `xmltest.zip`, and the IBM and Sun collections are published with "All rights reserved" |
| Use | the ignored integration test `u08_xml_w3c_conformance` downloads the archive into a temporary directory, verifies its SHA-256 digest and runs the cases; no file of the suite is stored in the repository |
