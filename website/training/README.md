<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Training packages of Danaco Nurt

The directory `website/training` holds the training packages that the website offers for download. Every file is built from the repository by `website/scripts/build_training_packs.py` with the executables `nurt` and `nurt-bench` of one release; `manifest.json` records the release, the revision of the command catalog, the date, and the size, the number of records and the SHA-256 sum of every file.

```sh
python3 website/scripts/build_training_packs.py --nurt <path of nurt> --nurt-bench <path of nurt-bench>
```

## Packages

| Package | Content | Records | License |
|---|---|---|---|
| `pairs` | pairs "input → Danaco Nurt program" in the format `nurt-bench-pair/2` | `pairs.jsonl`: accepted pairs; `rejected.jsonl`: programs with the error code and the message of the engine | CC BY 4.0 |
| `chat` | the accepted pairs as chat conversations: system message, user message, the program as the reply | `train.jsonl`, `validation.jsonl` | CC BY 4.0 |
| `tool-calls` | the accepted pairs as conversations in which the reply is a call of the tool `nurt` with the member `program`; `tools.json` is the tool definition to pass with every conversation | `train.jsonl`, `validation.jsonl` | CC BY 4.0 |
| `tools` | the definition of the tool `nurt` in every format of `nurt schema tool`, in English and Polish | one JSON document per format | Apache-2.0 |
| `corpus` | the documentation and the specification as documents with a path, a section, a title and the text in Markdown | `docs.jsonl` | Apache-2.0 |

## Sources of the pairs

| Source | Input | Program |
|---|---|---|
| `task` | the description of a public task of the benchmark | the reference program of the task |
| `translation` | the reference solution of a public task in bash or PowerShell 7 | the reference program of the task |
| `docs` | the sentence that introduces an example of a command page | the example |
| `habit` | a shell command of the cheat sheets for bash and PowerShell users | the canonical Danaco Nurt program |

The candidates are produced by `nurt-bench pairs`. The held-out tasks of the benchmark are excluded, so a model trained on the pairs can be evaluated on the held-out set.

## Validation

Every candidate is sent to the method `check` of `nurt serve`: the engine parses the program, applies the corrections of the language and binds the program against the command catalog. A program that binds is accepted and carries `validation.status` `bound`. A program that the engine refuses is written to `rejected.jsonl` with the error code and the message of the engine in `validation.rejected`.

## Splits

The packages `chat` and `tool-calls` are split by the identifier of the pair: a pair whose identifier starts with a byte below 26 belongs to the validation split, which gives one pair in ten. The split is stable between builds.

## Formats of the tool definition

`anthropic`, `openai`, `openai-strict`, `openai-responses`, `openai-responses-strict`, `gemini`, `mcp` and `json-schema`. Each definition carries the description of the language, so a model that receives the definition can write programs.

## Attribution

"nurt-bench pairs, Danaco Holding Group Sp. z o.o., CC BY 4.0". Use of the data for training and for evaluating machine learning models is expressly permitted by the license of the data set.
