// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";
import { serverArguments } from "./executable";
import { CONFIGURATION_SECTION, readSettings } from "./settings";

export const LANGUAGE_ID = "nurt";

/** Owns the connection to `nurt lsp`. */
export class LanguageServer implements vscode.Disposable {
  private client: LanguageClient | undefined;
  private readonly output = vscode.window.createOutputChannel("Danaco Nurt");

  /** Starts the server unless it is disabled or already running. */
  async start(): Promise<void> {
    if (this.client !== undefined) {
      return;
    }
    const settings = readSettings();
    if (!settings.languageServerEnabled) {
      return;
    }
    const serverOptions: ServerOptions = {
      command: settings.executable,
      args: serverArguments(),
      transport: TransportKind.stdio,
    };
    const clientOptions: LanguageClientOptions = {
      documentSelector: [
        { scheme: "file", language: LANGUAGE_ID },
        { scheme: "untitled", language: LANGUAGE_ID },
      ],
      outputChannel: this.output,
      synchronize: {
        configurationSection: CONFIGURATION_SECTION,
      },
    };
    const client = new LanguageClient("nurt", "Danaco Nurt", serverOptions, clientOptions);
    this.client = client;
    try {
      await client.start();
    } catch (error) {
      this.client = undefined;
      const reason = error instanceof Error ? error.message : String(error);
      this.output.appendLine(`cannot start "${settings.executable} lsp": ${reason}`);
      const action = await vscode.window.showErrorMessage(
        `Cannot start "${settings.executable} lsp". Install Danaco Nurt or set nurt.path.`,
        "Open Settings",
      );
      if (action === "Open Settings") {
        await vscode.commands.executeCommand("workbench.action.openSettings", "nurt.path");
      }
    }
  }

  /** Stops the server if it is running. */
  async stop(): Promise<void> {
    const client = this.client;
    this.client = undefined;
    if (client !== undefined) {
      await client.stop();
    }
  }

  /** Stops the server and starts it again with the current settings. */
  async restart(): Promise<void> {
    await this.stop();
    await this.start();
  }

  /** Reports whether a client object exists. */
  get running(): boolean {
    return this.client !== undefined;
  }

  dispose(): void {
    void this.stop();
    this.output.dispose();
  }
}
