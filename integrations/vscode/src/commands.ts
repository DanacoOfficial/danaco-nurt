// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/vscode/src/commands.ts
// Responsibility:   Source file integrations/vscode/src/commands.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import { execFile } from "child_process";
import * as path from "path";
import * as vscode from "vscode";
import { LANGUAGE_ID, LanguageServer } from "./client";
import { formatArguments, runArguments } from "./executable";
import { readSettings } from "./settings";

export const COMMAND_RUN_FILE = "nurt.runFile";
export const COMMAND_FORMAT_FILE = "nurt.formatFile";
export const COMMAND_RESTART_SERVER = "nurt.restartServer";

const FORMAT_TIMEOUT_MS = 30_000;
const FORMAT_MAX_OUTPUT_BYTES = 16 * 1024 * 1024;

/** Returns the document of a Danaco Nurt file saved on disk, after saving pending edits. */
async function savedDocument(resource?: vscode.Uri): Promise<vscode.TextDocument | undefined> {
  if (!vscode.workspace.isTrusted) {
    void vscode.window.showErrorMessage("Trust the workspace to run or format Danaco Nurt files.");
    return undefined;
  }
  const document =
    resource !== undefined
      ? await vscode.workspace.openTextDocument(resource)
      : vscode.window.activeTextEditor?.document;
  if (document === undefined || document.languageId !== LANGUAGE_ID) {
    void vscode.window.showErrorMessage("Open a Danaco Nurt file first.");
    return undefined;
  }
  if (document.isUntitled) {
    void vscode.window.showErrorMessage("Save the file before running or formatting it.");
    return undefined;
  }
  if (document.isDirty && !(await document.save())) {
    void vscode.window.showErrorMessage("The file could not be saved.");
    return undefined;
  }
  return document;
}

/** Runs the file with `nurt script` in the integrated terminal. */
async function runFile(resource?: vscode.Uri): Promise<void> {
  const document = await savedDocument(resource);
  if (document === undefined) {
    return;
  }
  const settings = readSettings(document.uri);
  const folder = vscode.workspace.getWorkspaceFolder(document.uri);
  const quoted = runArguments(document.uri.fsPath).map((value) => ({
    value,
    quoting: vscode.ShellQuoting.Strong,
  }));
  const execution = new vscode.ShellExecution(
    { value: settings.executable, quoting: vscode.ShellQuoting.Strong },
    quoted,
    { cwd: folder?.uri.fsPath ?? path.dirname(document.uri.fsPath) },
  );
  const task = new vscode.Task(
    { type: "nurt" },
    folder ?? vscode.TaskScope.Workspace,
    `Run ${path.basename(document.uri.fsPath)}`,
    "Danaco Nurt",
    execution,
  );
  task.presentationOptions = { reveal: vscode.TaskRevealKind.Always, clear: true };
  await vscode.tasks.executeTask(task);
}

function runFormatter(executable: string, file: string): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      executable,
      formatArguments(file),
      {
        cwd: path.dirname(file),
        timeout: FORMAT_TIMEOUT_MS,
        maxBuffer: FORMAT_MAX_OUTPUT_BYTES,
        encoding: "utf8",
      },
      (error, stdout, stderr) => {
        if (error !== null) {
          const detail = stderr.trim() !== "" ? stderr.trim() : error.message;
          reject(new Error(detail));
          return;
        }
        resolve(stdout);
      },
    );
  });
}

/**
 * Formats the file with `nurt fmt`. Canonical text printed by the command
 * replaces the document; when the command prints nothing, it has rewritten
 * the file and the editor reloads it.
 */
async function formatFile(resource?: vscode.Uri): Promise<void> {
  const document = await savedDocument(resource);
  if (document === undefined) {
    return;
  }
  const settings = readSettings(document.uri);
  let output: string;
  try {
    output = await runFormatter(settings.executable, document.uri.fsPath);
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    void vscode.window.showErrorMessage(`nurt fmt failed: ${reason}`);
    return;
  }
  if (output === "") {
    await vscode.commands.executeCommand("workbench.action.files.revert", document.uri);
    return;
  }
  if (output === document.getText()) {
    return;
  }
  const edit = new vscode.WorkspaceEdit();
  const whole = new vscode.Range(
    document.positionAt(0),
    document.positionAt(document.getText().length),
  );
  edit.replace(document.uri, whole, output);
  await vscode.workspace.applyEdit(edit);
}

/** Registers the commands of the extension. */
export function registerCommands(server: LanguageServer): vscode.Disposable[] {
  return [
    vscode.commands.registerCommand(COMMAND_RUN_FILE, runFile),
    vscode.commands.registerCommand(COMMAND_FORMAT_FILE, formatFile),
    vscode.commands.registerCommand(COMMAND_RESTART_SERVER, () => server.restart()),
  ];
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
