<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/examples/swe-agent/README.md -->
<!-- Responsibility:   Documents SWE-agent. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# SWE-agent

Tool bundle that gives SWE-agent the `nurt` command.

## Files

| File | Content |
|---|---|
| `tools/nurt/config.yaml` | signature, description and argument of the `nurt` command |
| `tools/nurt/bin/nurt` | command of the bundle; runs `nurt exec` with the program |
| `agent.yaml` | agent configuration that loads the bundle and uses function calling |

## Usage

1. Install the `nurt` executable in the environment image of the agent at `/usr/local/bin/nurt`, or set `NURT_EXECUTABLE` to its path.
2. Run `sweagent run --config agent.yaml` together with the settings of the model and the problem.

The bundle directory comes first in `PATH`, so the command of the bundle calls the console executable by its path.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
