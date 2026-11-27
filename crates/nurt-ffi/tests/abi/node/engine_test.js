// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ffi/tests/abi/node/engine_test.js
// Responsibility:   Source file crates/nurt-ffi/tests/abi/node/engine_test.js.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Interface test of libnurt through a Node-API module, against the engine of
// nurt-core in a build without the cargo feature `stub`.
//
// Usage: node engine_test.js <path of the compiled module>
//
// The engine of this crate has no language front end, so programs end with
// language errors in the JSON results. Every status returned by the engine
// is checked against the status table of nurt.h. The state directory of the
// engine is placed under $HOME. The script prints one line per failure and
// exits with status 1 when a check fails.

'use strict';

const nurt = require(process.argv[2]);

const [OK, , CANCELLED, ARG, ABI, IO, INTERNAL] = [0, 1, 2, 3, 4, 5, 6];
const ABI_VERSION = 1 << 16;
const WAIT_FOREVER = 0xffffffff;
const VARIANT_A = 1;

const READ = JSON.stringify({ nurt: '1.0', program: 'find "/srv/app"' });
const TWO_INPUTS = JSON.stringify({ nurt: '1.0', program: 'find "/srv/app"', cursor: 'c_91fe' });
const AST = JSON.stringify({ nurt: '1.0', program: { statements: [] } });

const USUAL = [OK, ARG, INTERNAL];
const MATRIX = {
  nurt_engine_config_get: USUAL,
  nurt_parse: USUAL,
  nurt_print: USUAL,
  nurt_correct: USUAL,
  nurt_check: USUAL,
  nurt_run_json: USUAL,
  nurt_page: USUAL,
  nurt_catalog: USUAL,
  nurt_describe: USUAL,
  nurt_help: USUAL,
  nurt_history: USUAL,
  nurt_space: USUAL,
  nurt_session_end: [...USUAL, IO],
  nurt_ext_load: [...USUAL, ABI, IO],
  nurt_ext_list: USUAL,
};

// Text argument and number of each function called through `call`.
const ARGUMENTS = {
  nurt_engine_config_get: [null, 0],
  nurt_parse: ['find "/srv/app"', VARIANT_A],
  nurt_print: [AST, VARIANT_A],
  nurt_correct: ['find "/srv/app"', VARIANT_A],
  nurt_check: [READ, 0],
  nurt_run_json: [READ, 0],
  nurt_page: ['c_91fe', 0],
  nurt_catalog: [null, 0],
  nurt_describe: [null, 0],
  nurt_help: [null, 0],
  nurt_history: [null, 0],
  nurt_space: [null, 0],
  nurt_session_end: ['s-1', 0],
  nurt_ext_load: ['/opt/nurt/ext/sample/nurt-ext.toml', 0],
  nurt_ext_list: [null, 0],
};

const failures = [];

function check(condition, what) {
  if (!condition) {
    failures.push(what);
    process.stderr.write(`check failed: ${what}\n`);
  }
}

function open(config = null) {
  const opened = nurt.engineOpen(config, false);
  check(opened.status === OK, `open the engine: ${opened.status}`);
  return opened.handle;
}

function lastError() {
  const record = nurt.lastError(false).out[0];
  return record === null ? null : JSON.parse(record);
}

function testMatrix() {
  const e = open();
  for (const [name, allowed] of Object.entries(MATRIX)) {
    const [text, number] = ARGUMENTS[name];
    const { status } = nurt.call(name, e, text, number);
    check(allowed.includes(status), `${name} returned ${status}`);
    check(nurt.call(name, null, text, number).status === ARG, `${name} with null`);
  }
  check(nurt.abiCheck(ABI_VERSION) === OK, 'nurt_abi_check');
  check(nurt.abiCheck(2 << 16) === ABI, 'nurt_abi_check ABI');
  const info = nurt.info(false);
  check(info.status === OK && JSON.parse(info.out[0]).catalog_rev.startsWith('sha256:'), 'nurt_info');
  check(nurt.info(true).status === ARG, 'nurt_info with null');
  check(nurt.engineOpen(null, true).status === ARG, 'nurt_engine_open with null');
  check(nurt.call('nurt_schema', null, 'ast', 0).status === OK, 'nurt_schema');
  check(nurt.call('nurt_schema', null, 'unknown', 0).status === ARG, 'nurt_schema with an unknown name');
  check(nurt.lastError(true).status === ARG, 'nurt_last_error with null');
  const started = nurt.execStart(e, READ);
  check(started.status === OK, 'nurt_exec_start');
  check(nurt.execCancel(started.handle) === OK, 'nurt_exec_cancel');
  check([OK, CANCELLED].includes(nurt.execWait(started.handle, WAIT_FOREVER).status), 'nurt_exec_wait');
  nurt.execFree(started.handle);
  check(nurt.execWait(null, 0).status === ARG, 'nurt_exec_wait with null');
  check(nurt.execCancel(null) === ARG, 'nurt_exec_cancel with null');
  nurt.engineClose(e);
}

function testResults() {
  const e = open();
  const read = nurt.call('nurt_run_json', e, READ, 0);
  check(read.status === OK, 'read request');
  check(JSON.parse(read.out[0]).report.error.code === 'E_UNSUPPORTED', 'read request without a front end');
  const rule = nurt.call('nurt_run_json', e, TWO_INPUTS, 0);
  check(rule.status === OK && JSON.parse(rule.out[0]).report.error.code === 'E_ARG', 'language error E_ARG');
  const invalid = nurt.call('nurt_run_json', e, 'not json', 0);
  check(invalid.status === ARG && invalid.out[0] === null, 'invalid JSON');
  check(lastError().detail.function === 'nurt_run_json', 'last error record');
  const parsed = nurt.call('nurt_parse', e, 'find "/srv/app"', VARIANT_A);
  check(parsed.status === OK && parsed.out[0] === 'null', 'no syntax tree');
  check(JSON.parse(parsed.out[1]).error.code === 'E_UNSUPPORTED', 'language error in the diagnostics');
  check(nurt.call('nurt_help', e, 'find', 0).status === ARG, 'help without a front end');
  check(lastError().detail.error.code === 'E_UNSUPPORTED', 'language error in the last error');
  nurt.engineClose(e);
}

function testEvents() {
  const e = open();
  const started = nurt.execStart(e, READ);
  check(nurt.execWait(started.handle, WAIT_FOREVER).status === OK, 'wait for completion');
  const events = nurt.execFree(started.handle).map((event) => JSON.parse(event));
  check(events.map((event) => event.t).join(',') === 'exec.started,exec.finished', 'event types');
  check(events.map((event) => event.seq).join(',') === '1,2', 'seq increases from 1');
  check(new Set(events.map((event) => event.request_id)).size === 1, 'one request identifier');
  nurt.engineClose(e);
}

function testOpen() {
  const fs = require('fs');
  const path = require('path');
  const blocker = path.join(process.env.HOME, 'blocker');
  fs.writeFileSync(blocker, '');
  const blocked = nurt.engineOpen(JSON.stringify({ state: { dir: path.join(blocker, 'state') } }), false);
  check(blocked.status === IO && blocked.handle === null, 'state directory under a file');
  check(nurt.engineOpen('{"state":{"dir":"state"}}', false).status === ARG, 'relative state directory');
}

check(nurt.abiVersion() === ABI_VERSION, 'interface version');
check(nurt.version().length > 0, 'library version');
testMatrix();
testResults();
testEvents();
testOpen();
if (failures.length > 0) {
  process.stderr.write(`${failures.length} checks failed\n`);
  process.exit(1);
}
process.stdout.write('ok\n');

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
