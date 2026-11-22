---
title: Security
order: 12
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Security

## Reporting a vulnerability

Suspected vulnerabilities are reported by e-mail to `support@danaco-group.pl` with a subject that starts with `[security] nurt`, not in a public issue. The policy, the scope and the handling times are in [SECURITY.md](https://github.com/DanacoOfficial/nurt/blob/main/SECURITY.md).

## Properties of the console

| Property | Description |
|---|---|
| Check before effect | Names, parameters and types are validated against the catalog before any effect takes place |
| No shell | Programs do not pass through a shell; a path is one value and is not split or expanded |
| One effect per program | A program with more than one changing or running command is rejected with `E_ONE_EFFECT` |
| Report | Each run reports its targets, unusual effects and irreversible effects |
| Journal | Changes to files are recorded and can be undone within the retention limits |
| Secrets | Parameters marked `secret` in the catalog are masked in the journal, the report and printed trees |
| Project files | A project file cannot grant itself trust; the keys that widen trust are ignored in `.nurt.toml` |

The properties that matter for a permission policy of a host program are returned by `nurt check`; see [Integration](integration.md).
