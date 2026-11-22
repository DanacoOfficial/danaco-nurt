// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Interface test of libnurt through a Node-API module, against the engine of
// the cargo feature `stub`.
//
// Usage: node abi_test.js <path of the compiled module>
//
// Every cell of the status table of nurt.h is exercised. The script prints
// one line per failure and exits with status 1 when a check fails.

'use strict';

const nurt = require(process.argv[2]);

const [OK, TIMEOUT, CANCELLED, ARG, ABI, IO, INTERNAL] = [0, 1, 2, 3, 4, 5, 6];
const ABI_VERSION = 1 << 16;
const WAIT_FOREVER = 0xffffffff;
const VARIANT_A = 1;

const request = (program, extra = {}) => JSON.stringify({ nurt: '1.0', program, ...extra });
const READ = request('find "/srv/app" glob: "*.go"');
const SYNTAX_ERROR = request('find "/srv/app" glob: ["*.go"');
const TWO_INPUTS = request('find "/srv/app"', { cursor: 'c_91fe' });
const EVENTS = request('find "/srv/app" glob: "*.rs"');
const SLOW = request('find "/srv/archive"');
const AST = JSON.stringify({ nurt: '1.0', program: { statements: [{ t: 'pipeline', head: { t: 'stage', name: 'find', args: [{ value: { t: 'text', v: '/srv/app' } }] }, stages: [] }] } });

const MATRIX = {
  nurt_abi_check: [OK, ABI],
  nurt_info: [OK, ARG, INTERNAL],
  nurt_engine_open: [OK, ARG, IO, INTERNAL],
  nurt_engine_config_get: [OK, ARG, INTERNAL],
  nurt_parse: [OK, ARG, INTERNAL],
  nurt_print: [OK, ARG, INTERNAL],
  nurt_correct: [OK, ARG, INTERNAL],
  nurt_check: [OK, ARG, INTERNAL],
  nurt_run_json: [OK, ARG, INTERNAL],
  nurt_exec_start: [OK, ARG, INTERNAL],
  nurt_exec_wait: [OK, TIMEOUT, CANCELLED, ARG, INTERNAL],
  nurt_exec_cancel: [OK, ARG, INTERNAL],
  nurt_page: [OK, ARG, INTERNAL],
  nurt_catalog: [OK, ARG, INTERNAL],
  nurt_describe: [OK, ARG, INTERNAL],
  nurt_help: [OK, ARG, INTERNAL],
  nurt_schema: [OK, ARG, INTERNAL],
  nurt_history: [OK, ARG, INTERNAL],
  nurt_space: [OK, ARG, INTERNAL],
  nurt_session_end: [OK, ARG, IO, INTERNAL],
  nurt_ext_load: [OK, ARG, ABI, IO, INTERNAL],
  nurt_ext_list: [OK, ARG, INTERNAL],
  nurt_last_error: [OK, ARG],
};

// Text argument and number of each function called through `call`.
const ARGUMENTS = {
  nurt_engine_config_get: [null, 0],
  nurt_parse: ['find "/srv/app"', VARIANT_A],
  nurt_print: [AST, VARIANT_A],
  nurt_correct: ['fnd "/srv/app"', VARIANT_A],
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
    console.error(`check failed: ${what}`);
  }
}

function withEngine(body) {
  const opened = nurt.engineOpen(null, false);
  if (opened.status !== OK) {
    throw new Error('cannot open the stub engine');
  }
  try {
    return body(opened.handle);
  } finally {
    nurt.engineClose(opened.handle);
  }
}

function withExec(engine, json, body) {
  const started = nurt.execStart(engine, json);
  if (started.status !== OK) {
    return { status: started.status, events: [] };
  }
  const status = body(started.handle);
  return { status, events: nurt.execFree(started.handle).map((event) => JSON.parse(event)) };
}

function engineCall(name, engine) {
  if (name === 'nurt_exec_start') {
    const started = nurt.execStart(engine, READ);
    nurt.execFree(started.handle);
    return started.status;
  }
  const [text, number] = ARGUMENTS[name];
  return nurt.call(name, engine, text, number).status;
}

function okScenario(name) {
  switch (name) {
    case 'nurt_abi_check':
      return nurt.abiCheck(ABI_VERSION);
    case 'nurt_info':
      return nurt.info(false).status;
    case 'nurt_engine_open': {
      const opened = nurt.engineOpen(null, false);
      nurt.engineClose(opened.handle);
      return opened.status;
    }
    case 'nurt_schema':
      return nurt.call(name, null, 'ast', 0).status;
    case 'nurt_last_error':
      return nurt.lastError(false).status;
    case 'nurt_exec_wait':
      return withEngine((e) => withExec(e, READ, (x) => nurt.execWait(x, WAIT_FOREVER).status).status);
    case 'nurt_exec_cancel':
      return withEngine((e) => withExec(e, READ, (x) => nurt.execCancel(x)).status);
    default:
      return withEngine((e) => engineCall(name, e));
  }
}

function argScenario(name) {
  switch (name) {
    case 'nurt_info':
      return nurt.info(true).status;
    case 'nurt_engine_open':
      return nurt.engineOpen(null, true).status;
    case 'nurt_exec_wait':
      return nurt.execWait(null, 0).status;
    case 'nurt_exec_cancel':
      return nurt.execCancel(null);
    case 'nurt_schema':
      return nurt.call(name, null, 'unknown', 0).status;
    case 'nurt_last_error':
      return nurt.lastError(true).status;
    default:
      return engineCall(name, null);
  }
}

function otherScenario(name, expected) {
  return withEngine((e) => {
    const key = `${name}:${expected}`;
    switch (key) {
      case `nurt_abi_check:${ABI}`:
        return nurt.abiCheck(2 << 16);
      case `nurt_ext_load:${ABI}`:
        return nurt.call(name, e, '/opt/nurt/ext/future/nurt-ext.toml', 0).status;
      case `nurt_ext_load:${IO}`:
        return nurt.call(name, e, '/opt/nurt/ext/missing/nurt-ext.toml', 0).status;
      case `nurt_session_end:${IO}`:
        return nurt.call(name, e, 's-io', 0).status;
      case `nurt_engine_open:${IO}`: {
        process.env.NURT_STUB_DIR = '/nonexistent/nurt-stub-cases';
        try {
          const opened = nurt.engineOpen(null, false);
          check(opened.handle === null, 'failed nurt_engine_open leaves out NULL');
          return opened.status;
        } finally {
          delete process.env.NURT_STUB_DIR;
        }
      }
      case `nurt_exec_wait:${TIMEOUT}`:
        return withExec(e, SLOW, (x) => nurt.execWait(x, 0).status).status;
      case `nurt_exec_wait:${CANCELLED}`:
        return withExec(e, SLOW, (x) => {
          nurt.execCancel(x);
          return nurt.execWait(x, WAIT_FOREVER).status;
        }).status;
      default:
        throw new Error(`no scenario for ${key}`);
    }
  });
}

function scenario(name, expected) {
  if (expected === OK) {
    return okScenario(name);
  }
  if (expected === ARG) {
    return argScenario(name);
  }
  if (expected === INTERNAL) {
    process.env.NURT_STUB_PANIC = name;
    try {
      return okScenario(name);
    } finally {
      delete process.env.NURT_STUB_PANIC;
    }
  }
  return otherScenario(name, expected);
}

function testMatrix() {
  for (const [name, statuses] of Object.entries(MATRIX)) {
    for (const expected of statuses) {
      const actual = scenario(name, expected);
      check(actual === expected, `${name}: expected status ${expected}, got ${actual}`);
    }
  }
}

function testRunJson() {
  withEngine((e) => {
    const read = nurt.call('nurt_run_json', e, READ, 0);
    check(read.status === OK, 'read request');
    check(JSON.parse(read.out[0]).report.records.matched === 148, 'read response holds the records');
    for (const [json, code] of [[SYNTAX_ERROR, 'E_SYNTAX'], [TWO_INPUTS, 'E_ARG']]) {
      const response = nurt.call('nurt_run_json', e, json, 0);
      check(response.status === OK, 'language error keeps NURT_OK');
      check(JSON.parse(response.out[0]).report.error.code === code, `language error ${code}`);
    }
    const invalid = nurt.call('nurt_run_json', e, 'not json', 0);
    check(invalid.status === ARG && invalid.out[0] === null, 'invalid JSON leaves the buffer empty');
    const record = JSON.parse(nurt.lastError(false).out[0]);
    check(record.code === 'NURT_E_ARG' && record.detail.function === 'nurt_run_json', 'last error record');
  });
}

function testEvents() {
  withEngine((e) => {
    const run = withExec(e, EVENTS, (x) => nurt.execWait(x, WAIT_FOREVER).status);
    check(run.status === OK, 'wait for completion');
    const types = run.events.map((event) => event.t).join(',');
    check(types === 'exec.started,log,progress,progress,exec.finished', `event types ${types}`);
    check(run.events.every((event, index) => event.seq === index + 1), 'seq increases from 1');
  });
}

check(nurt.abiVersion() === ABI_VERSION, 'interface version');
check(nurt.version().length > 0, 'library version');
testMatrix();
testRunJson();
testEvents();
if (failures.length > 0) {
  console.error(`${failures.length} checks failed`);
  process.exit(1);
}
console.log('ok');
