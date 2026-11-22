// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import * as vscode from "vscode";
import { LanguageServer } from "./client";
import { registerCommands } from "./commands";
import { CONFIGURATION_SECTION } from "./settings";

/** Starts the language server and registers the commands. */
export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const server = new LanguageServer();
  context.subscriptions.push(server, ...registerCommands(server));
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (
        event.affectsConfiguration(`${CONFIGURATION_SECTION}.path`) ||
        event.affectsConfiguration(`${CONFIGURATION_SECTION}.lsp.enabled`)
      ) {
        void server.restart();
      }
    }),
  );
  if (vscode.workspace.isTrusted) {
    await server.start();
  } else {
    context.subscriptions.push(
      vscode.workspace.onDidGrantWorkspaceTrust(() => {
        void server.start();
      }),
    );
  }
}

export function deactivate(): void {}
