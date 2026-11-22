<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# nurt-bench fixtures

Initial file trees shared by several tasks of `../tasks/`. Each fixture is a
snapshot in the format `nurt-fs-snapshot/1`, stored as `<name>.json`. A task
refers to a fixture in `fs_before` and may replace or extend its entries:

```json
{"fixture": "web-logs", "overlay": [{"path": "logs/extra.log", "type": "file", "content_utf8": "x\n"}]}
```

| Fixture | Content |
|---|---|
| `config-mixed` | JSON, TOML, YAML and INI files under `config/` and `.env.example` |
| `docs-site` | Markdown documentation under `docs/` |
| `etc-tree` | system configuration under `etc/`: accounts, hosts, cron, logrotate, SSH, sysctl and systemd units |
| `go-service` | Go module with a server command, request handlers and a store package with tests |
| `py-inventory` | Python package `inventory/` with tests, CSV data and `pyproject.toml` |
| `ts-webapp` | TypeScript application with TSX components, `package.json` and `tsconfig.json` |
| `web-logs` | access logs of three days and an application log under `logs/` |
| `win-project` | .NET solution with C# sources, a batch file and a PowerShell script, CRLF line endings |

All content is synthetic. Host names use `example.com`; the fixtures contain
no personal data and no credentials.

The data in this directory is licensed under CC BY 4.0 (see `LICENSE`).
