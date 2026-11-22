<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Differences from yq

The differential tests in `crates/nurt-doc/tests/u08_differential.rs`
compare the value that Nurt reads from every corpus file of a format with the
output of `yq` 4.53.6 for the same file. A difference is accepted only when
an entry of `known-differences.toml` covers it; an entry that covers no
difference fails the test as well.

## Records

`yq/<format>.jsonl` holds one JSON object per corpus file, in path order:

| Member | Content |
|---|---|
| `path` | path of the file relative to `tests/d5` |
| `value` | output of `yq -p <format> -o json -I0 <path>`, run in `tests/d5`; `null` when the output is empty |
| `documents` | list of the document values, instead of `value`, for a YAML stream of more than one document |
| `error` | first line of the error output, instead of `value`, when `yq` fails |

Nurt reads a YAML stream as the value of its single document, as the list of
its document values when it has more than one document, and as `null` when it
has none.

| Format | Corpus | Files |
|---|---|---|
| `xml` | `corpus/xml/` (`.xml`, `.atom`, `.csproj`, `.props`, `.rss`, `.svg`, `.targets`, `.xhtml`, `.xlf`, `.plist`) | 528 |
| `json` | `corpus/json/` | 146 |
| `toml` | `third_party/toml/toml-test/valid/` | 189 |
| `yaml` | `corpus/yaml/` (`.yaml`, `.yml`) | 347 |

A test fails when the list of corpus files differs from the records. When
`yq` 4.53.6 is in `PATH`, the tests named
`<format>_records_match_yq_when_the_recorded_yq_version_is_installed` run it
on every file and fail when its output differs from the record.

## Classes

| Class | Meaning |
|---|---|
| `convention` | both programs are correct and follow different rules |
| `yq` | a defect or a missing feature of `yq` |
| `nurt` | a defect of Nurt, reported to the format; none is open |

## Accepted differences

| Entry | Class | Difference |
|---|---|---|
| `xml-internal-entities` | `yq` | `yq` does not read the internal subset and keeps references to declared entities as written; Nurt replaces them |
| `xml-external-entities` | `convention` | Nurt never resolves external entities and reports `E_UNSUPPORTED_CONSTRUCT` `xml.external_entity` for a value that depends on one, or on a declaration in an unread external subset |
| `xml-entity-markup` | `convention` | a value that contains an entity whose replacement text has elements fails with `E_UNSUPPORTED_CONSTRUCT` `xml.entity_markup` |
| `xml-cdata-and-text-chunks` | `convention` | Nurt joins adjacent text and CDATA sections into one chunk; `yq` writes a list of trimmed sections |
| `xml-internal-subset-comment` | `yq` | `yq` removes comments from the text of the document type declaration |
| `xml-tokenized-attributes` | `yq` | `yq` does not read attribute-list declarations; Nurt normalizes the spaces of attributes declared with a tokenized type (XML 1.0, section 3.3.3) |
| `yaml-document-start-properties` | `yq` | after an anchor or a comment on the document start line, `yq` replaces the first key of the root mapping with an empty key or drops it |
| `yaml-complex-keys` | `convention` | JSON has only text keys; Nurt writes a sequence or mapping key as its flow text, `yq` writes an empty key or drops the entry |
| `yaml-merge-explicit-keys-win` | `yq` | a key written in a mapping takes precedence over a merged key; `yq` lets a later merge key override an earlier explicit key |
| `yaml-merge-inline-mapping` | `yq` | `yq` drops the keys of a mapping written in place as the value of a merge key |
| `yaml-merge-inline-mapping-in-sequence` | `yq` | `yq` drops the keys of a mapping written in place in the sequence of a merge key |
| `yaml-binary-scalar` | `convention` | Nurt reads a `!!binary` scalar as bytes, written as base64 without line breaks; `yq` writes the text of the scalar |
| `yaml-empty-scalar-with-local-tag` | `convention` | an empty scalar with a local tag is null in Nurt and an empty string in `yq` |
| `yaml-non-specific-tag` | `yq` | a scalar with the non-specific tag `!` is a string; `yq` resolves it as an untagged plain scalar |
| `yaml-empty-documents` | `yq` | `yq` writes no value for the empty documents of a stream; Nurt reads each of them as null |
| `yaml-scalar-on-document-start-line` | `yq` | `yq` joins a scalar on a document start line with the start marker of the next document |
| `yaml-version-directive` | `yq` | `yq` rejects a `%YAML` directive before a document other than the first |
| `yaml-tag-on-document-start-line` | `yq` | `yq` rejects a block mapping that follows a tag on the start line of the first document |
| `json-number-text` | `convention` | a number whose text is not the shortest form of its value, such as `19.90`, is a decimal in Nurt, written as a string in plain JSON |
| `json-byte-order-mark` | `yq` | `yq` rejects a UTF-8 byte order mark |
| `json-number-out-of-range` | `yq` | `yq` rejects numbers outside the range of a 64-bit float |
| `toml-empty-document` | `convention` | a document without key-value pairs is an empty table in Nurt; `yq` writes no value |
| `toml-local-date-time` | `yq` | `yq` does not support local dates, times and date-times |
| `toml-date-time-syntax` | `yq` | `yq` rejects a space between date and time and a lowercase `t` or `z` |
| `toml-signed-nan` | `yq` | `yq` rejects `+nan` and `-nan` |
| `toml-binary-integer` | `yq` | `yq` rejects integers with the `0b` prefix |
| `toml-json-output` | `yq` | `yq` cannot write an infinite float as JSON |
| `toml-float-rounding` | `yq` | a float read from the record may differ in the last digit, depending on the float parsing of the JSON reader |
| `toml-dotted-keys-in-inline-table` | `yq` | `yq` drops all but the last of several dotted keys with a common prefix in an inline table |
