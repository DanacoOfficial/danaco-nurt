<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Datasheet: nurt-bench pairs

Data set of pairs "task → Danaco Nurt program", format `nurt-bench-pair/1`
(schema [`../protocol/schema/nurt-bench-pair-1.schema.json`](../protocol/schema/nurt-bench-pair-1.schema.json)),
produced by `nurt-bench pairs`.

## Version

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Language of the descriptions | English |
| Notation of the programs | canonical notation (variant A) of Danaco Nurt 1 |
| Files | `pairs.jsonl` (accepted pairs), `rejected.jsonl` (rejected candidates with the reason), `summary.json` (counts per source) |
| Target size | at least 10 000 accepted pairs |

The number of pairs per source is given in `summary.json` of each release.

## License

The data set is licensed under the Creative Commons Attribution 4.0
International license (CC BY 4.0), see [`LICENSE`](LICENSE). Use of the data
set for training and for evaluating machine learning models, including
language models, is expressly permitted under this license. Attribution:
"nurt-bench pairs, Danaco Holding Group Sp. z o.o., CC BY 4.0".

## Content

| Source | Input | Program | Initial tree |
|---|---|---|---|
| `task` | description `nl` of a public task | reference program of the task | `fs_before` of the task |
| `translation` | reference solution of the task in bash or PowerShell 7 | reference program of the task | `fs_before` of the task |
| `docs` | sentence introducing an example of `docs/commands` | the example | the `fs-before` block of the example, if any |
| `habit` | shell command from a habit table `nurt-bench-habits/1` | canonical Danaco Nurt program | none |
| `run` | description of a public task | programs of the tool calls without an error of a passed run of a Danaco Nurt condition, one per line | `fs_before` of the task |

`input_kind` is `nl`, `bash` or `pwsh`. Pairs with the same input kind,
input and program appear once.

A habit table is a TOML file:

```toml
format = "nurt-bench-habits/1"

[[pair]]
shell = "bash"
habit = "mv notes.txt archive/"
canonical = "move \"notes.txt\" to: \"archive/\""
rule = "B03"
```

## Validation

Every candidate is sent to `check` of `nurt serve`: the program must parse
and bind against the catalog. A candidate with an initial tree is then run
on that tree in a temporary directory; it is accepted when the report has
no error and its status is neither `failed` nor `rolled_back`.
`validation` of a pair records the result.

## Provenance

The tasks, the documentation and the habit tables are original work of the
Danaco Nurt project. Tasks inspired by tldr-pages (CC BY 4.0) are attributed
in `../ATTRIBUTION.md`. The runs come from the published series of
`nurt-bench`; their programs were written by the models named in the run
logs. No pair contains personal data, credentials or addresses of real
services; example hosts use `example.com`.

## Limitations

- Pairs from runs reflect the solutions of the models of the panel and may
  contain programs that are valid but not the shortest form.
- Validation proves that a program binds and runs without an error, not
  that it fulfils the description; pairs from runs also passed the oracle of
  the task.
- Held-out tasks of the task set are excluded, so the data set can be used
  to train models that are later evaluated on the held-out set.
- The programs follow version 1 of the language; later versions may accept
  other forms.
