// Test double of the addon `nurt.node`. It has the interface of the module
// exported by the library and answers from the request text.
//
// The program text of a request selects the behaviour:
//   echo TEXT        response with TEXT as the rendered text, after two events
//   slow             response only after cancellation, with the error E_CANCELLED
//   bad              start throws NURT_E_ARG
//   broken           the stream ends with the failure NURT_E_INTERNAL
//
// The operations answer with their name and input, except `parse` of the
// program `bad`, which answers with a language error, and `help`, which
// throws NURT_E_IO.

"use strict";

/** Observations of the tests. */
/** @type {{ engines: any[], requests: any[], cancelled: number, started: number, running: number, maxRunning: number }} */
const observed = { engines: [], requests: [], cancelled: 0, started: 0, running: 0, maxRunning: 0 };

function report(status, error) {
  return error === undefined ? { status, kind: "program" } : { status, kind: "program", error };
}

class NativeExecution {
  constructor(program, onMessage) {
    observed.started += 1;
    this.requestId = `r${observed.started}`;
    this.finished = false;
    this.cancelRequested = false;
    this.program = program;
    this.onMessage = onMessage;
    this.release = undefined;
    observed.running += 1;
    observed.maxRunning = Math.max(observed.maxRunning, observed.running);
    setImmediate(() => {
      this.emit(1, "exec.started");
      this.emit(2, "job.output", { text: "line" });
      if (program.startsWith("echo ")) {
        this.finish(JSON.stringify({ nurt: "1.0", report: report("ok"), rendered: { format: "text", text: program.slice(5) } }));
      } else if (program === "broken") {
        this.finished = true;
        observed.running -= 1;
        this.onMessage({ kind: "failure", text: "NURT_E_INTERNAL: the engine failed" });
      }
    });
  }

  emit(seq, t, fields) {
    const event = { seq, time: "2026-01-01T00:00:00Z", request: this.requestId, t, ...fields };
    this.onMessage({ kind: "event", text: JSON.stringify(event) });
  }

  finish(text) {
    if (this.finished) {
      return;
    }
    this.finished = true;
    observed.running -= 1;
    this.emit(3, "exec.finished");
    this.onMessage({ kind: "response", text });
  }

  cancel() {
    observed.cancelled += 1;
    this.cancelRequested = true;
    const error = { code: "E_CANCELLED", message: "cancelled" };
    this.finish(JSON.stringify({ nurt: "1.0", report: report("failed", error) }));
  }

  isFinished() {
    return this.finished;
  }
}

class NativeEngine {
  constructor(options) {
    this.options = options === undefined || options === null ? null : JSON.parse(options);
    observed.engines.push(this.options);
  }

  info() {
    return JSON.stringify({
      version: "0.1.0",
      formats: { ast: "nurt-ast/1", bound: "nurt-bound/1", report: "nurt-report/1", envelope: "1.0", config: "1" },
      features: ["fixtures"],
      os: "linux",
      architecture: "x86_64",
      services: ["files"],
      state_dir: null,
    });
  }

  call(operation, input) {
    const params = JSON.parse(input);
    observed.requests.push({ operation, params });
    if (operation === "help") {
      return Promise.reject(new Error("NURT_E_IO: the help files cannot be read"));
    }
    if (operation === "parse" && params.program === "bad") {
      return Promise.resolve(JSON.stringify({ error: { code: "E_SYNTAX", message: "unexpected token" } }));
    }
    if (operation === "describe") {
      if (params.subject === "structured") {
        return Promise.resolve(JSON.stringify({ result: { layer: "core", commands: [] } }));
      }
      return Promise.resolve(JSON.stringify({ result: `${params.subject} ${params.language}` }));
    }
    return Promise.resolve(JSON.stringify({ result: { operation, params } }));
  }

  start(request, onMessage) {
    const body = JSON.parse(request);
    observed.requests.push({ operation: "run", params: body });
    if (body.program === "bad") {
      throw new Error("NURT_E_ARG: the request is not valid");
    }
    return new NativeExecution(body.program, onMessage);
  }
}

module.exports = { NativeEngine, observed };
