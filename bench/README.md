<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           bench/README.md -->
<!-- Responsibility:   Documents nurt-bench. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# nurt-bench

`nurt-bench` measures whether Danaco Nurt, used as the tool of an AI agent,
completes more tasks with less harm and at lower cost than bash and
PowerShell 7. The same agent loop runs the same tasks with several models;
only the tool differs.

| Directory | Content | License |
|---|---|---|
| [`tasks/`](tasks/) | 720 tasks in 12 domains, format `nurt-bench-task/1` | CC BY 4.0 |
| [`oracles/`](oracles/) | oracle that evaluates the final state of a run | Apache-2.0 |
| [`fixtures/`](fixtures/) | shared initial file trees | CC BY 4.0 |
| [`protocol/`](protocol/) | protocol, system message, tool definitions, model configurations, schemas, container images | CC BY 4.0 |
| [`data/`](data/) | pair data set "task → Danaco Nurt program" | CC BY 4.0 |

The runner, the analysis and the data set generator are the crate
`crates/nurt-bench` (Apache-2.0), with the executable `nurt-bench`.

## Usage

```
cargo build --release -p nurt-bench
nurt-bench run --condition bash --model bench/protocol/models/openai.toml --out runs
nurt-bench run --condition nurt --nurt "nurt serve --stdio" --model bench/protocol/models/openai.toml --out runs
nurt-bench analyze runs --out results
```

The API key of a provider is read from the environment variable named in
the model configuration. `--model mock:<file>` runs the mock model, which
needs no network access.

The protocol, the measures, the statistical analysis and the reference of
every command are in [`protocol/PROTOCOL.md`](protocol/PROTOCOL.md).

## Submitting results

A result consists of the run logs, the files written by `nurt-bench
analyze`, the model configurations and the image digests of the
containers. Results are submitted as a pull request to
`github.com/DanacoOfficial/danaco-nurt` or to support@danaco-group.pl.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
