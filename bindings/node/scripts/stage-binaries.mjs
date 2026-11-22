// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Places the release executables into the platform packages.
//
//   node scripts/stage-binaries.mjs --assets <directory> --version <version>
//
// The assets directory holds the release archives and the file SHA256SUMS
// ("<sha256>  <file name>" per line). The archive of a platform is
// nurt-<version>-<target>.tar.gz, or .zip for Windows, where <target> is the
// Rust target triple. Every archive is checked against SHA256SUMS before it is
// extracted; a missing archive or a checksum mismatch stops the script. The
// executable found in the archive is copied to npm/<platform>/bin/, together
// with the addon nurt.node when the archive contains it. Requires
// the build (dist/) and the programs tar and unzip.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const require = createRequire(import.meta.url);
const { PLATFORM_PACKAGES, executableName } = require("../dist/platform.js");

const EXECUTABLE_MODE = 0o755;
const ADDON_FILE = "nurt.node";
const CHECKSUM_LINE = /^([0-9a-f]{64}) [ *](.+)$/;

const { values } = parseArgs({ options: { assets: { type: "string" }, version: { type: "string" } } });
if (values.assets === undefined || values.version === undefined) {
  process.stderr.write("usage: node scripts/stage-binaries.mjs --assets <directory> --version <version>\n");
  process.exit(2);
}

const packageDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");

function readChecksums(directory) {
  const checksums = new Map();
  for (const line of readFileSync(join(directory, "SHA256SUMS"), "utf8").split("\n")) {
    const match = CHECKSUM_LINE.exec(line.trim());
    if (match !== null) {
      checksums.set(match[2], match[1]);
    }
  }
  return checksums;
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function extract(archive, destination) {
  const [program, args] = archive.endsWith(".zip")
    ? ["unzip", ["-q", archive, "-d", destination]]
    : ["tar", ["-xzf", archive, "-C", destination]];
  const result = spawnSync(program, args, { stdio: "inherit" });
  if (result.status !== 0) {
    throw new Error(`${program} failed to extract ${archive}`);
  }
}

function findFile(directory, name) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      const found = findFile(path, name);
      if (found !== undefined) {
        return found;
      }
    } else if (entry.isFile() && entry.name === name) {
      return path;
    }
  }
  return undefined;
}

const checksums = readChecksums(values.assets);
const work = mkdtempSync(join(tmpdir(), "nurt-stage-"));
try {
  for (const entry of PLATFORM_PACKAGES) {
    const extension = entry.os === "win32" ? "zip" : "tar.gz";
    const name = `nurt-${values.version}-${entry.target}.${extension}`;
    const archive = join(values.assets, name);
    const expected = checksums.get(name);
    if (expected === undefined) {
      throw new Error(`SHA256SUMS has no entry for ${name}`);
    }
    const actual = sha256(archive);
    if (actual !== expected) {
      throw new Error(`checksum mismatch for ${name}: expected ${expected}, found ${actual}`);
    }
    const destination = join(work, entry.id);
    mkdirSync(destination);
    extract(archive, destination);
    const binary = executableName(entry.os);
    const source = findFile(destination, binary);
    if (source === undefined) {
      throw new Error(`${name} does not contain ${binary}`);
    }
    const target = join(packageDirectory, "npm", entry.id, "bin", binary);
    mkdirSync(dirname(target), { recursive: true });
    copyFileSync(source, target);
    chmodSync(target, EXECUTABLE_MODE);
    const addon = findFile(destination, ADDON_FILE);
    if (addon !== undefined) {
      copyFileSync(addon, join(dirname(target), ADDON_FILE));
    }
    process.stdout.write(`${entry.id}: ${name} (sha256 ${actual})\n`);
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
