<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Differential tests

Differential tests compare the output of a Nurt command with the output of a reference program on the same generated corpus. They are compiled only with the `differential` feature:

```
cargo test -p nurt-files --features differential
```

A test starts by locating its reference program in `PATH`. When the program is missing, the test prints `skipped: program ... not found in PATH` and returns without failing. Reference programs are never part of the product.

## Normalization

Before comparison both results are converted to the same form: UTF-8 text, line ends `\n`, paths relative to the corpus root and separated by `/`, names in Unicode normalization form C, entries sorted by path.

## Option mapping

| Nurt command and option | Reference |
|---|---|
| `search` with `case_sensitive: false` | `rg -i` |
| `search` with `whole_word` | `rg -w` |
| `search` with `multiline` | `rg -U` |
| `search` with `invert` | `rg -v` |
| `search` with `only_matching` | `rg -o` |
| `search` with `output: counts` | `rg -c` |
| `search` with `output: files` | `rg -l` |
| `search` with `output: unmatched` | `rg --files-without-match` |
| `search` with `glob` | `rg -g` |
| `search` with `hidden: true` | `rg --hidden` |
| `search` with `gitignore: false` | `rg --no-ignore` |
| `search` with `context` | `rg -C` |
| `search` with `file_type` | `rg -t` |
| `find` with `type` | `fd -t` |
| `find` with `depth` | `fd -d` |
| `replace` with a regular expression | `sed -E` |
| `diff` | `diff -u` |
| `patch` | `patch -p1` |
| `hash` with `sha256`, `sha512`, `md5`, `blake3` | `sha256sum`, `sha512sum`, `md5sum`, `b3sum` |
| `archive.*` | `tar`, `unzip`, `bsdtar`, `7z` |
| `convert` | `iconv` |

## Intended differences

| Difference | Nurt | Reference |
|---|---|---|
| Hidden entries | included by default (`hidden: true`) | excluded by default |
| Ignore files | not applied by default (`gitignore: false`) | applied by default |
| Name form | Unicode normalization form C | bytes as stored |
| Binary files | skipped with a count | searched or reported as binary |
| Diff without git headers that empties a file, new side stamped with the epoch | the `patch` command removes the file; the library `nurt_text::patch` plans the file as kept and empty | file removed by `patch` |
| Hunk at the start of a file when the target gained leading lines | hunk placed by offset search | `patch` may report a rejected hunk |

## Test files

| File | Library | Reference programs |
|---|---|---|
| `differential_codec.rs` | `nurt_text::codec` | `iconv` |
| `differential_diff.rs` | `nurt_text::diff` | `diff -u` |
| `differential_patch.rs` | `nurt_text::patch` | `patch -p1`, `diff -urN` |
| `differential_hash.rs` | `nurt_text::hash` | `sha256sum`, `sha512sum`, `sha1sum`, `md5sum`, `b3sum` |
| `differential_archive.rs` | `nurt_archive` zip, tar, 7z | `unzip -l`, `zip`, `tar -tvf`, `bsdtar -tf`, `7z l` |
| `differential_search.rs` | `nurt_text::search` with the walker | `rg` |
| `differential_walk.rs` | walker | `fd` |
| `perf_search.rs`, `perf_walk.rs` | search over 1 GiB, walk over 10^6 entries | `rg`, `fd`; ignored tests, ratio at most 1.3 |

Ignored tests run with `--ignored`; the corpus directory is set with `NURT_PERF_DIR`.

A test that depends on an intended difference asserts the difference itself, not only the equal part. `differential_patch.rs` compares the library `nurt_text::patch`, so it asserts the empty file; the `patch` command removes such a file when it turns the plan into file operations, as the reference does.
