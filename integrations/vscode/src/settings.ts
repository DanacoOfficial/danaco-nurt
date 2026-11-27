// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/vscode/src/settings.ts
// Responsibility:   Source file integrations/vscode/src/settings.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import * as vscode from "vscode";
import { resolveExecutable } from "./executable";

export const CONFIGURATION_SECTION = "nurt";

export interface Settings {
  executable: string;
  languageServerEnabled: boolean;
}

/** Reads the extension settings for a resource. */
export function readSettings(resource?: vscode.Uri): Settings {
  const configuration = vscode.workspace.getConfiguration(CONFIGURATION_SECTION, resource);
  const folder =
    resource !== undefined
      ? vscode.workspace.getWorkspaceFolder(resource)
      : vscode.workspace.workspaceFolders?.[0];
  return {
    executable: resolveExecutable(configuration.get<string>("path"), {
      workspaceFolder: folder?.uri.fsPath,
    }),
    languageServerEnabled: configuration.get<boolean>("lsp.enabled", true),
  };
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
