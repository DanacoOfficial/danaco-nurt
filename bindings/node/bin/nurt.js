// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/bin/nurt.js
// Responsibility:   Source file bindings/node/bin/nurt.js.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

#!/usr/bin/env node
"use strict";

// Runs the nurt executable of the platform package with the arguments and
// standard streams of this process, and exits with its status.

const { spawn } = require("node:child_process");
const { locateExecutable } = require("../dist/locate.js");

const TERMINATING_SIGNALS = ["SIGHUP", "SIGTERM"];

function main() {
  let executable;
  try {
    // PATH is not searched: it may lead back to this launcher.
    executable = locateExecutable({ env: process.env, includePath: false });
  } catch (error) {
    process.stderr.write(`nurt: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
    return;
  }

  const child = spawn(executable, process.argv.slice(2), { stdio: "inherit" });
  const handlers = new Map();

  // A terminal delivers SIGINT to the whole process group, so the server
  // receives it directly; it is forwarded only when this process alone was
  // signalled. The handler also keeps this process alive until the server exits.
  handlers.set("SIGINT", () => {
    if (process.platform !== "win32" && !process.stdin.isTTY) {
      child.kill("SIGINT");
    }
  });
  if (process.platform !== "win32") {
    for (const signal of TERMINATING_SIGNALS) {
      handlers.set(signal, () => {
        child.kill(signal);
      });
    }
  }
  for (const [signal, handler] of handlers) {
    process.on(signal, handler);
  }

  child.on("error", (error) => {
    process.stderr.write(`nurt: cannot run ${executable}: ${error.message}\n`);
    process.exitCode = 1;
  });
  child.on("exit", (code, signal) => {
    for (const [name, handler] of handlers) {
      process.off(name, handler);
    }
    if (signal !== null) {
      process.kill(process.pid, signal);
      return;
    }
    process.exitCode = code === null ? 1 : code;
  });
}

main();

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
