<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Language golden files

Golden files of the lexer, the parsers, the canonical printer, the tree
codecs and the binder. They complement the conformance files of
`spec/golden/`, which remain the reference for the surface and bound trees of
the 14 reference tasks and of the examples. A program of `spec/golden/ast/`
keeps its file name here: `task-NN-<name>` and `example-NN-<name>`.

## Layout

| Path | Content |
|---|---|
| `ast/a/<name>.canonical.nurt` | canonical text in word notation (variant A) of the tree `spec/golden/ast/<name>.json` |
| `ast/b/<name>.nurt` | program text in variant B; its surface tree is `spec/golden/ast/<name>.json` |
| `ast/c/<name>.nurt` | program text in variant C |
| `ast/c/<name>.json` | surface tree of `ast/c/<name>.nurt`, without spans and metadata |
| `ast/c/<name>.hash` | `ast_hash` of `ast/c/<name>.json` |
| `bound/<variant>/<name>.json`, `.hash` | bound trees; see [`bound/README.md`](bound/README.md) |
| `invalid/<case>.json` | documents that are not valid trees, or that only the published schema admits |
| `invalid/expected.tsv` | expected outcome of every document of `invalid/` |
| `hashes.tsv` | index of the hashes of every tree of this directory |

Program text is UTF-8 with a final line feed.

## Hashes

`<name>.hash` holds `sha256:` followed by 64 lowercase hexadecimal digits and
a line feed: the `ast_hash` of `<name>.json` as defined in
`spec/syntax-tree.md`, section 6.

`hashes.tsv` is generated from the trees and is never edited by hand. Its
columns, separated by tabs, are:

| Column | Content |
|---|---|
| `file` | path of the tree relative to this directory |
| `format` | `nurt-ast/1` or `nurt-bound/1` |
| `ast_hash` | hash of the tree |

## Negative cases

`invalid/expected.tsv` has the columns, separated by tabs:

| Column | Content |
|---|---|
| `file` | path of the document relative to this directory |
| `kind` | `ast` (decoded as a surface tree) or `bound` (decoded as a bound tree) |
| `code` | expected error code, or `-` when the document is accepted |
| `layer` | `schema`: the published schema rejects the document; `validate`: the schema admits it and an invariant of the decoder rejects it; `accepted`: the published contract admits the document |
| `reason` | rule that applies |

## Regeneration

The recorded hashes of this directory and `hashes.tsv` are written by the
test `hash_golden` of the crate `nurt-syntax` when the environment variable
`NURT_BLESS` is `1`:

```
NURT_BLESS=1 cargo test -p nurt-syntax --test hash_golden
```

Without the variable the tests compare. The files of `spec/golden/` are only
compared. A regenerated file is committed after review of the difference.
