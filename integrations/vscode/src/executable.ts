// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/vscode/src/executable.ts
// Responsibility:   Source file integrations/vscode/src/executable.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import * as os from "os";
import * as path from "path";

export const DEFAULT_EXECUTABLE = "nurt";

export interface ExpansionContext {
  workspaceFolder?: string;
  homeDirectory?: string;
}

/**
 * Returns the command to start for the `nurt.path` setting. An empty setting
 * selects the bare name, which the operating system resolves through the
 * `PATH`. `${workspaceFolder}` and a leading `~` are expanded.
 */
export function resolveExecutable(
  configured: string | undefined,
  context: ExpansionContext = {},
): string {
  const value = (configured ?? "").trim();
  if (value === "") {
    return DEFAULT_EXECUTABLE;
  }
  const home = context.homeDirectory ?? os.homedir();
  let expanded = value;
  if (expanded === "~" || expanded.startsWith("~/") || expanded.startsWith("~\\")) {
    expanded = path.join(home, expanded.slice(1));
  }
  if (expanded.includes("${workspaceFolder}")) {
    if (context.workspaceFolder === undefined) {
      throw new Error("nurt.path uses ${workspaceFolder}, but no folder is open");
    }
    expanded = expanded.split("${workspaceFolder}").join(context.workspaceFolder);
  }
  return expanded;
}

/** Arguments that start the language server. */
export function serverArguments(): string[] {
  return ["lsp"];
}

/** Arguments that run a program file. */
export function runArguments(file: string): string[] {
  return ["script", file];
}

/** Arguments that print or rewrite a program file in canonical form. */
export function formatArguments(file: string): string[] {
  return ["fmt", file];
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
