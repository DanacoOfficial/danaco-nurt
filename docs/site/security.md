---
title: Security
order: 12
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Security

## Reporting a vulnerability

Suspected vulnerabilities are reported by e-mail to `support@danaco-group.pl` with a subject that starts with `[security] nurt`, not in a public issue. The policy, the scope and the handling times are in [SECURITY.md](https://github.com/DanacoOfficial/danaco-nurt/blob/main/SECURITY.md).

## Properties of the shell

| Property | Description |
|---|---|
| Check before effect | Names, parameters and types are validated against the catalog before any effect takes place |
| No shell | Programs do not pass through a shell; a path is one value and is not split or expanded |
| One effect per program | A program with more than one changing or running command is rejected with `E_ONE_EFFECT` |
| Report | Each run reports its targets, unusual effects and irreversible effects |
| Journal | Changes to files are recorded and can be undone within the retention limits |
| Secrets | Parameters marked `secret` in the catalog are masked in the journal, the report and printed trees |
| Project files | A project file cannot grant itself trust; the keys that widen trust are ignored in `.nurt.toml` |

## Privilege elevation

A call may ask for the rights of the administrator with the call parameter `privilege` (`account`, `elevated`, `auto`) and for another account with `as_user`. Nurt applies no policy of its own: the decision belongs to the embedding engine, which sets `privilege.allowed = false` to refuse `elevated` and `auto` with `E_PRIVILEGE` and the reason `disabled_by_configuration`. A project file cannot set `privilege.*`.

| Backend | How it works |
|---|---|
| `self` | the process already is the administrator; the operation runs in the same process |
| `sudo`, `doas`, `run0` | `nurt elevated-exec` is started with the rights of the administrator and reads one request in the request format of the protocol on its standard input; no shell text is built and no password is asked (`-n`, `nopass`, `--no-ask-password`) |
| `broker` | the privileged service `nurt-elevation`; reported as `not_installed` until it is released |

The order and the set of backends come from `privilege.backends`. Access control is that of the operating system: an account that may run `nurt` through `sudo`, `doas` or `run0` is administrator-equivalent, as with those programs. The elevated process refuses to run unless its identity is the administrator (or the account named by `as_user`), writes its journal entry to the journal of the account that asked, with the rights of that account (on Linux through the file system identity of the thread, on macOS through descriptors that are checked and handed to the account with `fchown` before anything is written; on Windows it is the same account), and returns the report, which states `elevated: true` and the `backend`.

The properties that matter for a permission policy of a host program are returned by `nurt check`; see [Integration](integration.md).
