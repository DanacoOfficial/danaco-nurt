// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/src/locate.ts
// Responsibility:   Source file bindings/node/src/locate.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import { accessSync, constants, statSync } from "node:fs";
import { dirname, posix, win32 } from "node:path";
import { NurtError } from "./errors.js";
import { candidatePackages, detectHost, executableName, packageName, type HostPlatform } from "./platform.js";

/** Environment variable with the path of the `nurt` executable. */
export const EXECUTABLE_VARIABLE = "NURT_EXECUTABLE";

/** Options of {@link locateExecutable}. */
export interface LocateOptions {
  /** Path or command name given by the caller; takes precedence over every other source. */
  readonly executable?: string | undefined;
  /** Environment that supplies `NURT_EXECUTABLE` and `PATH`. */
  readonly env: { readonly [name: string]: string | undefined };
  /** Whether `PATH` is searched after the platform packages. */
  readonly includePath: boolean;
  /** Host whose platform package is looked up. Default the current host. */
  readonly host?: HostPlatform | undefined;
  /**
   * Returns the directory of an installed package, or undefined when the
   * package is not installed. Default: resolution from this package.
   */
  readonly resolvePackage?: ((name: string) => string | undefined) | undefined;
}

/**
 * Finds the `nurt` executable.
 *
 * Sources, in order: the `executable` option, the variable `NURT_EXECUTABLE`,
 * the platform package of the host and, when `includePath` is set, `PATH`.
 * A path given by the option or the variable is used without falling back to
 * the other sources.
 *
 * @throws {NurtError} `NURT_E_IO` with the list of the places checked when no
 *   executable is found.
 */
export function locateExecutable(options: LocateOptions): string {
  const host = options.host ?? detectHost();
  const paths = host.platform === "win32" ? win32 : posix;
  const binary = executableName(host.platform);

  if (options.executable !== undefined) {
    return explicitExecutable(options.executable, "the executable option", options.env, host);
  }
  const variable = options.env[EXECUTABLE_VARIABLE];
  if (variable !== undefined && variable !== "") {
    return explicitExecutable(variable, EXECUTABLE_VARIABLE, options.env, host);
  }

  const checked: string[] = [];
  const resolvePackage = options.resolvePackage ?? resolvePackageDirectory;
  const candidates = candidatePackages(host);
  if (candidates.length === 0) {
    checked.push(`no platform package exists for ${host.platform}-${host.arch}`);
  }
  for (const candidate of candidates) {
    const name = packageName(candidate);
    const directory = resolvePackage(name);
    if (directory === undefined) {
      checked.push(`package ${name} is not installed`);
      continue;
    }
    const file = paths.join(directory, "bin", binary);
    if (isExecutableFile(file, host)) {
      return file;
    }
    checked.push(`package ${name} has no executable ${file}`);
  }
  if (options.includePath) {
    const found = searchPath(binary, options.env, host);
    if (found !== undefined) {
      return found;
    }
    checked.push(`${binary} is not in PATH`);
  }
  throw notFound(checked);
}

function explicitExecutable(
  value: string,
  source: string,
  env: { readonly [name: string]: string | undefined },
  host: HostPlatform,
): string {
  const paths = host.platform === "win32" ? win32 : posix;
  const isPath = value.includes("/") || (host.platform === "win32" && value.includes("\\"));
  if (isPath) {
    const file = paths.resolve(value);
    if (isExecutableFile(file, host)) {
      return file;
    }
    throw notFound([`${source} names ${file}, which is not an executable file`]);
  }
  const found = searchPath(value, env, host);
  if (found !== undefined) {
    return found;
  }
  throw notFound([`${source} names ${value}, which is not in PATH`]);
}

function notFound(checked: readonly string[]): NurtError {
  return new NurtError(
    "NURT_E_IO",
    `nurt executable not found: ${checked.join("; ")}; set ${EXECUTABLE_VARIABLE} or install the package for this platform`,
  );
}

/**
 * Searches the directories of `PATH`. Directories `node_modules/.bin` are
 * skipped: they hold the launcher of this package, which is not the server.
 */
function searchPath(
  binary: string,
  env: { readonly [name: string]: string | undefined },
  host: HostPlatform,
): string | undefined {
  const paths = host.platform === "win32" ? win32 : posix;
  const key = host.platform === "win32" ? Object.keys(env).find((name) => name.toUpperCase() === "PATH") : "PATH";
  const value = key === undefined ? undefined : env[key];
  if (value === undefined) {
    return undefined;
  }
  for (const directory of value.split(paths.delimiter)) {
    if (directory === "" || isLauncherDirectory(directory, paths)) {
      continue;
    }
    const file = paths.join(directory, binary);
    if (isExecutableFile(file, host)) {
      return file;
    }
  }
  return undefined;
}

function isLauncherDirectory(directory: string, paths: typeof posix): boolean {
  const normalized = paths.normalize(directory).split(paths.sep).filter((part) => part !== "");
  return normalized.length >= 2 && normalized[normalized.length - 2] === "node_modules" && normalized[normalized.length - 1] === ".bin";
}

function isExecutableFile(file: string, host: HostPlatform): boolean {
  try {
    if (!statSync(file).isFile()) {
      return false;
    }
    if (host.platform !== "win32") {
      accessSync(file, constants.X_OK);
    }
    return true;
  } catch {
    return false;
  }
}

function resolvePackageDirectory(name: string): string | undefined {
  try {
    return dirname(require.resolve(`${name}/package.json`));
  } catch {
    return undefined;
  }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
