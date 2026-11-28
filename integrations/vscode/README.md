<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           integrations/vscode/README.md -->
<!-- Responsibility:   Documents Danaco Nurt for Visual Studio Code. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Danaco Nurt for Visual Studio Code

Language support for Danaco Nurt programs (`.nurt`): syntax highlighting, diagnostics and completion from `nurt lsp`, and commands that run and format a file.

## Requirements

The `nurt` executable, installed and on the `PATH` or referenced by the `nurt.path` setting. The extension does not install it.

## Features

| Feature | Provided by |
|---|---|
| Syntax highlighting | TextMate grammar `source.nurt`. |
| Diagnostics, completion and other language features | `nurt lsp`, started over standard input and output. |
| Comment toggling, bracket pairs | `language-configuration.json`. |

## Commands

| Command | Identifier | Action |
|---|---|---|
| Danaco Nurt: Run File | `nurt.runFile` | Saves the file and runs `nurt script <file>` in the integrated terminal. Also available as the run button of the editor title bar. |
| Danaco Nurt: Format File | `nurt.formatFile` | Saves the file and runs `nurt fmt <file>`. Text printed by the command replaces the document; when the command prints nothing, the editor reloads the rewritten file. |
| Danaco Nurt: Restart Language Server | `nurt.restartServer` | Stops `nurt lsp` and starts it with the current settings. |

## Settings

| Setting | Type | Default | Description |
|---|---|---|---|
| `nurt.path` | string | `nurt` | Path of the `nurt` executable. A bare name is looked up in the `PATH`. `${workspaceFolder}` and a leading `~` are expanded. Not applied in untrusted workspaces. |
| `nurt.lsp.enabled` | boolean | `true` | Start `nurt lsp` for Danaco Nurt files. |
| `nurt.trace.server` | `off`, `messages`, `verbose` | `off` | Trace of the messages exchanged with `nurt lsp`, written to the Danaco Nurt output channel. |

The extension does not collect telemetry and sends no data over the network.

## Workspace trust

In an untrusted workspace the extension does not start `nurt lsp` and does not run files. Both start after the workspace is trusted.

## Build

The TextMate grammar is maintained with the language server in `editors/textmate/nurt.tmLanguage.json` and is copied into `syntaxes/` unchanged by `scripts/copy-grammar.sh`. The script fails with a message when the grammar is missing or declares a scope other than `source.nurt`. `NURT_GRAMMAR_SOURCE` and `NURT_GRAMMAR_TARGET` override the paths.

```
npm install
npm run build:grammar
npm run compile
npm run package
```

`npm run package` writes `dist/nurt-<version>.vsix`.

## Tests

| Command | Requirement | Check |
|---|---|---|
| `bash scripts/test-copy-grammar.sh` | `bash` | Grammar copy script. |
| `npm run test:unit` | Node.js 20 | Executable resolution, command arguments, manifest contents, absence of telemetry. |
| `node ./out/test/runTest.js` | Node.js 20, display | Extension host tests with `@vscode/test-electron`: activation, commands, settings, formatting through a stand-in executable. |

## Publication

The workflow `.github/workflows/integrations-vscode.yml` (its copy in this directory is `ci/vscode.yml`) packages the extension and publishes it to the Visual Studio Marketplace (secret `VSCE_PAT`, publisher `danaco`) and to Open VSX (secret `OVSX_PAT`, namespace `danaco`).

## License

Apache-2.0. See the `LICENSE` file of the repository.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
