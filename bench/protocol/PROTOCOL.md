<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# nurt-bench protocol

Protocol version 1.0.0. This document defines how `nurt-bench` compares
Danaco Nurt with bash and PowerShell 7 as the tool of an AI agent: the
conditions, the models, the agent loop, the execution of a task, the
measures, the statistical analysis, the entry rule and the publication of
results. The task set is described in [`../tasks/README.md`](../tasks/README.md)
and the oracles in [`../oracles/`](../oracles/).

The code of the benchmark is licensed under Apache-2.0. The task set, the
protocol files and the data sets are licensed under CC BY 4.0 (see the
`LICENSE` file of each data directory).

## 1. Conditions

Every task runs under the conditions that apply to its platform. The agent
loop, the system message, the limits and the model settings are identical in
every condition; only the tool and its description differ.

| Condition | Tool shown to the model | Description of the tool | Systems |
|---|---|---|---|
| `bash` | `bash` | [`tools/bash.json`](tools/bash.json) | Linux, macOS |
| `pwsh` | `pwsh` | [`tools/pwsh.json`](tools/pwsh.json) | Windows |
| `nurt` | `nurt` | description of the core returned by `describe` of the server | Linux, macOS, Windows |
| `nurt-groups` | `nurt` | core description; the description of a command group is added to the result of the first call that uses a command of the group | Linux, macOS, Windows |

The shell conditions use the programs of a standard installation of the
system image (see [`containers/`](containers/)). The Danaco Nurt conditions
use one `nurt serve --stdio` process per run.

## 2. Models

The panel has at least six models of at least four producers, including
models with open weights. Each model is described by one configuration file
in [`models/`](models/); the configuration is data and is recorded in every
run log.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `format` | text | required | `"nurt-bench-model/1"` |
| `id` | text | required | label of the model in run logs and results |
| `provider` | `anthropic`, `openai`, `gemini`, `openai-compatible` | required | API of the provider; `openai-compatible` serves models with open weights through the Chat Completions API |
| `model` | text | required | model name in the API of the provider |
| `producer` | text | required | organization that trained the model |
| `weights` | `open`, `closed` | required | availability of the weights |
| `base_url` | text | the public API of the provider | base address of the API; required for `openai-compatible` |
| `api_key_env` | text | required except for `openai-compatible` | name of the environment variable holding the API key |
| `temperature` | number 0 to 2 | required | sampling temperature; 0 in the published panel |
| `max_output_tokens` | integer | required | output token limit of one response |
| `timeout_seconds` | integer | `300` | time limit of one request |
| `retries` | integer | `4` | repeated attempts after a rate limit, a server error or a connection failure |
| `retry_delay_ms` | integer | `2000` | delay before the first repeated attempt; doubled for every further attempt |

API keys are read only from the environment variable named in the
configuration. They are never written to run logs.

Each task runs three times per model and condition, with the seeds 1, 2 and
3. The seed is passed to the providers that accept one (OpenAI, Gemini,
servers with the Chat Completions API) and is recorded for all.

## 3. Agent loop

1. The conversation starts with one user message: the description `nl` of
   the task.
2. The system message is [`system-message.txt`](system-message.txt) with
   `{os}` replaced by `Linux`, `Windows` or `macOS` and `{max_steps}` by the
   step limit of the task.
3. The model receives one tool. Its arguments are defined by the tool
   definition (`input_schema`); the argument named by `argument` carries the
   command or the program.
4. The runner sends the conversation to the model. A response without a
   tool call ends the run with the outcome `answered`; its text is the final
   answer.
5. The tool calls of a response are executed in order. Each result is
   returned to the model in one user message, marked as an error when the
   call failed.
6. One model call is one step. After the step limit the run ends with the
   outcome `step_limit`; when the time limit is exceeded, with
   `time_limit`. The time limit covers model calls and tool calls.

### Tool execution

| Condition | Execution of one call | Text returned to the model |
|---|---|---|
| `bash` | new process `bash --noprofile --norc -c <command>` in the task root | standard output; standard error after the line `[stderr]`; `[exit code N]` for a non-zero code; `[killed after the time limit of N s]` |
| `pwsh` | new process `pwsh -NoProfile -NonInteractive -Command <command>` in the task root | as for `bash` |
| `nurt`, `nurt-groups` | `run` request of `nurt serve` with the arguments of the call as the request envelope; `options.base` is the task root and `options.timeout` the remaining time | the rendered text of the response, or the report as JSON when no text is rendered |

Every process of a run has `LC_ALL=C.UTF-8`, `TZ=UTC` and a home directory
outside the task root. Standard input of shell commands is empty. Shell
state does not persist between calls; only the file system does. The time
limit of a call is the remaining time of the run; on expiry the process
group is killed.

A tool result longer than `max_tool_output_chars` of
[`protocol.toml`](protocol.toml) keeps its first and last halves and names
the number of omitted characters. The limit is the same in every condition.

In the condition `nurt-groups` every program is first sent to `check`. For
each group named in the result that was not shown before, the description
returned by `describe` is appended to the tool result after the line
`[description of the group <name>]`.

## 4. Execution of a task

1. The runner creates a temporary directory with the task root, a home
   directory and the state directory of the server.
2. `fs_before` is materialized in the task root.
3. The `setup` script of the shell of the system runs in the task root.
4. The snapshot `before` of the task root is taken.
5. The agent loop runs.
6. The probes of the task are collected; the `teardown` script runs.
7. The snapshot `after` is taken and the oracle evaluates the run.
8. The run log is completed and the temporary directory is removed.

The Danaco Nurt server of a run gets `NURT_STATE_DIR` and `NURT_CONFIG` in
the temporary directory, so that a run never reads or changes the
configuration and the journal of the user.

Linux and macOS tasks run in a temporary directory of the host or in the
container image of [`containers/linux/`](containers/linux/); Windows tasks
run in a virtual machine or the image of
[`containers/windows/`](containers/windows/). Published results state the
image digest of every series.

### Oracle

The oracle is a separate process. It reads one JSON object on standard
input:

```json
{"task": {}, "before": {}, "after": {}, "observation": {"answer": "", "probes": {}}}
```

`task` is the task document, `before` and `after` are snapshots
`nurt-fs-snapshot/1`, `answer` is the final answer and `probes` maps probe
identifiers to their results. The oracle writes the verdict
`nurt-bench-verdict/1` on standard output and exits with code 0. The default
command is `python3 -m nurt_bench_oracle`, run in `bench/oracles`.

## 5. Measures

| Measure | Definition |
|---|---|
| Completion | the verdict reports `completed`: the expected final state holds |
| Harm | the verdict reports `harmed`: an entry changed outside the permitted set, an entry of the initial tree was lost, the encoding or the line ending style of an existing file changed, or a forbidden change happened |
| Lost entries | number of entries of the initial tree missing after the run |
| Encoding changes | number of files with a changed encoding or line ending style |
| Input and output tokens | sums of the token counts reported by the provider over all model calls of the run |
| Description tokens | tokens of the tool description and of the group descriptions added during the run, counted by the token counting interface of the provider (Anthropic, Gemini); characters are recorded for all providers |
| Steps | number of model calls |
| Wall clock time | duration of the agent loop |
| Intermediate script | the run contains a tool call that starts an interpreter of another language (`python`, `perl`, `ruby`, `node`, `php`, `deno`, `bun`) as the first word of a shell command, or, in Danaco Nurt, a `run` of such an interpreter or of a shell |
| Command corrections | in tasks with `platform: "cross"`: tool calls that directly follow a failed tool call |
| FV | share of Danaco Nurt calls without a correction and without a notation error (an error code that arises only while parsing or binding) |
| FC | share of Danaco Nurt calls that ran after a correction of the notation |
| FE | FV + FC |
| FCR | share of corrected calls whose report names a changed path that the oracle reports as unexpected, lost or forbidden |
| AMB | share of Danaco Nurt calls that ended with `E_AMBIGUOUS` |

Costs (tokens, steps, time) are compared on runs completed in both compared
conditions.

### Provocation tasks

Tasks of domain 12 provoke known failure classes of shells (field
`provocation`). They are reported separately per class and condition. For a
shell, a run counts as harmful exactly as in daily use: a removed wrong
directory, a truncated input or a corrupted encoding is harm even when the
model reports success. For Danaco Nurt, the expected results are no effect,
or an effect with the unusual consequence named in the report; any harm
counts as for the shells.

## 6. Analysis

`nurt-bench analyze` reads every complete run log. Runs with the outcome
`error` (provider failure after all repeated attempts, failure of the
server process or of the oracle) are excluded and counted; they are
repeated before publication.

A run of a Danaco Nurt condition is paired with the shell run of the same
model, task and seed: `bash` on Linux and macOS, `pwsh` on Windows.

| Quantity | Method |
|---|---|
| Completion, paired | exact two-sided McNemar test on the pairs with different outcomes |
| Costs, paired | two-sided Wilcoxon signed-rank test; zero differences are dropped; exact distribution without ties and with at most 50 non-zero differences, otherwise the normal approximation with tie correction and continuity correction |
| Intervals | 95% percentile bootstrap with 10 000 resamples of runs (rates) or pairs (differences); generator SplitMix64 with a recorded seed |
| Significance level | 0.05 |

Results are given per model, pooled over models, per domain and condition,
for the provocation classes and for cross-system tasks. The files of an
analysis are `summary.json` (format `nurt-bench-results/1`), `tables.md` and
`runs.csv` (one row per run).

### Criteria

| Criterion | Holds when |
|---|---|
| No harm | no run of the Danaco Nurt condition is harmed (100% of runs) |
| Completion | the completion rate of Danaco Nurt on the pairs is at least that of the shell; the McNemar p-value and the interval of the difference are reported with it |
| Cost | on the pairs completed in both conditions, the medians of tokens (input + output), steps and wall clock time of Danaco Nurt are at most those of the shell |

### Entry rule

The result supports further work on a Danaco Nurt condition when the
completion and cost criteria hold pooled over the models and for at least
two thirds of the models (4 of 6), and the no-harm criterion holds in every
run.

## 7. Publication

A published result includes the raw run logs, the analysis files, the
version of the protocol and of `nurt-bench`, the model configurations, and
the container image digests. Results are published whether they favour
Danaco Nurt or not; tasks in which Danaco Nurt does worse than a shell are
listed in the result.

The held-out tasks (see `../tasks/README.md`) are run with the public tasks
and reported separately; a significant difference between the public and
the held-out part indicates leakage of the task set into training data.
Their content is not published.

## 8. Run log `nurt-bench-run/1`

One file per run, `<out>/<model>/<condition>/<task>.s<seed>.jsonl`, one JSON
object per line, schema [`schema/nurt-bench-run-1.schema.json`](schema/nurt-bench-run-1.schema.json).
The member `event` names the kind of the line:

| `event` | Content |
|---|---|
| `run` | first line: run identifier, task, condition, model, seed, protocol version, SHA-256 of the system message, tool name and description size, limits, system, start time, version of `nurt-bench` |
| `model` | one model call: step, time, latency, response text, tool calls, token counts, stop reason |
| `tool` | one tool call: step, call identifier, arguments, returned text, error flag, exit code, time limit flag; for Danaco Nurt the report status, error code, correction rules and changed paths |
| `group` | a group description added in the condition `nurt-groups`: group, characters, tokens |
| `oracle` | final answer, probe results and verdict |
| `end` | last line: outcome, steps, tool calls, token totals, wall clock time, failure description for the outcome `error` |

A log is written as `<file>.partial` and renamed when the run ends; a run
whose complete log exists is not repeated.

## 9. Mock model

`--model mock:<file>` replaces the provider by scripted responses. The file
is either a script `nurt-bench-mock/1` (schema
[`schema/nurt-bench-mock-1.schema.json`](schema/nurt-bench-mock-1.schema.json))
or a run log, whose model responses are replayed in order. A script lists
`scripts` (task identifier or pattern with `*`, optional condition and seed,
`steps`) and `default` steps for other runs. Token counts not given by a
step are estimated as one token per four characters. The mock model is used
to test the runner and the analysis without network access; its runs are
not results.

## 10. Relation to the syntax test

The syntax test of the Danaco Nurt language and this benchmark are separate
plans with different questions.

| | Syntax test | `nurt-bench` |
|---|---|---|
| Question | which notation variant, description length, description language and correction setting give the most programs written correctly | does Danaco Nurt let an agent complete more tasks with less harm and lower cost than bash and PowerShell |
| Tasks | 540 tasks, 45 per domain, taken from the task set | 720 tasks, 60 per domain, including 72 held-out tasks |
| Models | 4 models | at least 6 models of at least 4 producers |
| Conditions | notation variants A, B and C, description length, description language, correction on and off, bash as control | `bash`, `pwsh`, `nurt`, `nurt-groups` |
| Execution | parser, correction, binding and a reduced executor on a virtual file system, without the journal and without the operating system | the released executable on a real file system in a container or virtual machine |
| Main measures | FV, FC, FE, FCR, AMB, steps to completion, notation tokens | completion, harm, tokens, steps, time, scripts, command corrections, and FV to AMB |
| Use of the result | choice of the notation variant and of the description | public comparison and entry rule |

`nurt-bench variants` produces the variants B and C of the reference
programs used by the syntax test.

## 11. Commands

### `nurt-bench run`

```
nurt-bench run --condition <condition> --model <model> [options]
```

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `--tasks` | path | `bench/tasks` | directory of the task set |
| `--condition` | `bash`, `pwsh`, `nurt`, `nurt-groups` | required | condition of every run |
| `--model` | path or `mock:<path>` | required | model configuration or mock model |
| `--seeds` | integer | `3` | runs per task, with the seeds 1 to N |
| `--out` | path | `runs` | output directory of the run logs |
| `--protocol` | path | `bench/protocol` | directory of the protocol files |
| `--fixtures` | path | `bench/fixtures` | directory of the shared initial trees |
| `--oracles` | path | `bench/oracles` | working directory of the oracle command |
| `--oracle-command` | text | `python3 -m nurt_bench_oracle` | oracle command line |
| `--nurt` | text | none; required for `nurt` and `nurt-groups` | command line of the server, for example `"nurt serve --stdio"` |
| `--filter` | text, repeatable | all tasks | tasks whose identifier contains the text or matches the pattern with `*` |
| `--jobs` | integer | `1` | runs executed in parallel |

Only tasks whose platform runs on the current system are executed. Each run
is reported on standard error; a summary is written to standard output.
Exit code 0 when every run was executed or already complete, 1 when a run
could not be prepared or the invocation failed, 2 for invalid arguments.

```
nurt-bench run --condition bash --model bench/protocol/models/deepseek.toml --out /home/user/runs
nurt-bench run --condition nurt --nurt "nurt serve --stdio" --model mock:/home/user/mock.json --filter "search-*"
```

### `nurt-bench analyze`

```
nurt-bench analyze <runs> [--out <path>] [--resamples <n>] [--seed <n>]
```

| Parameter | Type | Default | Meaning |
|---|---|---|---|
| `<runs>` | path | required | directory of the run logs |
| `--out` | path | `results` | output directory |
| `--resamples` | integer | `10000` | bootstrap resamples |
| `--seed` | integer | `1` | bootstrap seed |

Writes `summary.json`, `tables.md` and `runs.csv`. Exit code 1 when a log
cannot be read.

### `nurt-bench variants`

```
nurt-bench variants --nurt <command> --out <path> [--tasks <path>]
```

Parses the reference program of every task in variant A, prints the tree in
variants B and C and parses each back; a variant is kept only when its tree
equals the tree of variant A. Writes one record `nurt-bench-variants/1` per
task (`id`, `a`, `b`, `c` or `error`). Exit code 1 when a task has no
variants.

### `nurt-bench pairs`

```
nurt-bench pairs --out <path> (--nurt <command> | --list-only) [--tasks <path>] [--fixtures <path>] [--docs <path>] [--habits <path>]... [--runs <path>]
```

Builds the pair data set described in [`../data/DATASHEET.md`](../data/DATASHEET.md).
With `--nurt`, every candidate is validated and the output holds
`pairs.jsonl`, `rejected.jsonl` and `summary.json`; with `--list-only` it
holds `candidates.jsonl` and `summary.json`.

## 12. Versions

The protocol version is `version` of [`protocol.toml`](protocol.toml) and is
recorded in every run. A change of the system message, of a tool definition,
of the limits or of the analysis methods changes the version; results of
different versions are not pooled.
