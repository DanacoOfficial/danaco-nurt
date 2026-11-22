# Key-value format corpus

The corpus of the TOML, INI, property and environment formats is in
`tests/d5/corpus/{toml,ini,properties,env}/`. The test
`crates/nurt-doc/tests/u03_roundtrip.rs` opens every file with the harness of
`nurt_doc::testing` and applies every harness operation to every key.

## Sets

| Directory | Files | Content |
|---|---|---|
| `corpus/toml/own/` | 7 | `Cargo.toml`, `pyproject.toml`, `rust-toolchain.toml`, `netlify.toml`, Hugo `config.toml`, dates and numbers, strings and tables |
| `corpus/toml/own/generated/` | 30 | combinations of three of ten fragments: the four date kinds, integer radixes and special floats, arrays of tables, the four string styles, inline tables, dotted keys, multi-line arrays with comments, nested arrays of tables, empty tables |
| `third_party/toml/toml-test/` | 189 valid, 371 invalid | `toml-test`; valid documents are also run through the harness |
| `corpus/ini/<profile>/` | 8 | one or two files per profile |
| `corpus/ini/generated/<profile>/` | 25 per profile | `generic`, `systemd`, `gitconfig`, `desktop`, `php`, `editorconfig`: sections and keys of the profile, repeated keys, continuation lines (`systemd`), trailing comments (`gitconfig`, `php`), localized keys (`desktop`), `key[]` (`php`), glob sections (`editorconfig`), keys before the first section |
| `corpus/properties/` | 3 | Spring configuration, messages, ISO-8859-1 |
| `corpus/properties/generated/` | 100 | the separators `=`, `:` and white space, continuation lines, `\uHHHH` and escaped separators in keys, `#` and `!` comments; every tenth file has an ISO-8859-1 byte, four files have CRLF line breaks |
| `corpus/env/` | 3 | application, Docker Compose and local override files; stored under `*.env` names because `.env` and `.env.*` are excluded by `.gitignore` |
| `corpus/env/generated/` | 100 | `export`, single and double quotes, escapes, multi-line values, `${NAME}` references, trailing comments, empty values; ten files without a final line break, four files with CRLF line breaks |

A generated file combines fragments chosen by its number, so that every
fragment occurs in several files and in several positions. The INI profile
of a file follows from its name when the name implies a dialect
(`.service`, `.gitconfig`, `.desktop`, `php.ini`, `.editorconfig`, `.ini`) and
from its content otherwise.

## Recorded results of other implementations

| Path | Content | Used by |
|---|---|---|
| `git-config/gitconfig.list`, `git-config/generated/NNN.list` | output of `git config --list --null --file` (Git 2.53.0) for `corpus/ini/gitconfig/.gitconfig` and `corpus/ini/generated/gitconfig/NNN.ini`; records are `key\nvalue\0`, a key without `=` has no value | `u03_ini_differential.rs` |
| `git-config/edge.gitconfig`, `git-config/edge.list` | one file with every value rule of the Git syntax (quotes, escapes, continuation, trailing comments, key without `=`, subsections with differing case) and its Git output | `u03_ini_differential.rs` |
| `java-properties.json` | `java.util.Properties.load` (OpenJDK 21.0.12.1) for the 103 files of `corpus/properties/`: 524 keys; files are read as UTF-8 when valid and as ISO-8859-1 otherwise, a byte order mark is removed | `u03_ini_differential.rs` |
| `roundtrip-stats.json` | refusals with `E_UNSUPPORTED_CONSTRUCT` by construct name for each corpus set of the harness run | `u03_roundtrip.rs` |

`rust-ini` is compared live; no results of it are recorded.
