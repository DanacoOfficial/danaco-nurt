// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Test server speaking nurt-rpc/1 over standard input and output.
//
// The program text of `run` selects the behaviour:
//   echo TEXT        response with TEXT as the rendered text
//   delay MS TEXT    response after MS milliseconds; cancellable
//   fail             response whose report carries the error E_SYNTAX
//   events N         N job.output events and exec.finished, written together
//                    with the response in one write
//   page             response with a cursor of a next page
//   crash            exit with status 3 after a message on standard error
//   noise            response written over several lines
//   split            response written in three pieces
//   binary           line that is not valid UTF-8
//
// Options: --max-inflight N (default 64), --protocol NAME (default nurt-rpc/1).

const options = { maxInflight: 64, protocol: "nurt-rpc/1" };
for (let index = 2; index < process.argv.length; index += 2) {
  const name = process.argv[index];
  const value = process.argv[index + 1];
  if (name === "--max-inflight") {
    options.maxInflight = Number(value);
  } else if (name === "--protocol") {
    options.protocol = value ?? options.protocol;
  } else {
    process.stderr.write(`fake-serve: unknown option ${name}\n`);
    process.exit(2);
  }
}

const MESSAGE_LIMIT = 1048576;
const CRASH_STATUS = 3;
const SPLIT_PIECES = 3;
const SPLIT_INTERVAL_MS = 10;
const RPC_METHOD_NOT_FOUND = -32601;
const RPC_CANCELLED = -32002;
const RPC_ARG = -32003;

const state = {
  client: null,
  session: null,
  configOverrides: null,
  events: false,
  inFlight: 0,
  maxObservedInFlight: 0,
  cancelRequests: 0,
  seq: 0,
};
const running = new Map();

function line(message) {
  return `${JSON.stringify(message)}\n`;
}

function send(message) {
  process.stdout.write(line(message));
}

function result(id, value) {
  return { jsonrpc: "2.0", id, result: value };
}

function failure(id, code, message, data) {
  return { jsonrpc: "2.0", id, error: data === undefined ? { code, message } : { code, message, data } };
}

function event(requestId, t, fields) {
  state.seq += 1;
  return {
    jsonrpc: "2.0",
    method: "nurt/event",
    params: { t, seq: state.seq, time: "2026-01-01T00:00:00Z", request_id: requestId, ...fields },
  };
}

function report(status, kind, extra) {
  return { nurt: "1.0", status, kind, ...extra };
}

function response(text, reportValue) {
  return { nurt: "1.0", report: reportValue, rendered: { format: "text", text } };
}

function begin() {
  state.inFlight += 1;
  state.maxObservedInFlight = Math.max(state.maxObservedInFlight, state.inFlight);
}

function end(id) {
  state.inFlight -= 1;
  running.delete(id);
}

function run(id, params) {
  if (state.inFlight >= options.maxInflight) {
    send(failure(id, RPC_ARG, "too many requests in progress"));
    return;
  }
  const program = typeof params.program === "string" ? params.program : "";
  const [command, ...rest] = program.split(" ");
  begin();
  switch (command) {
    case "echo": {
      end(id);
      send(result(id, response(rest.join(" "), report("ok", "echo"))));
      return;
    }
    case "delay": {
      const [ms, ...text] = rest;
      const timer = setTimeout(() => {
        end(id);
        send(result(id, response(text.join(" "), report("ok", "delay"))));
      }, Number(ms));
      running.set(id, () => {
        clearTimeout(timer);
        end(id);
        send(failure(id, RPC_CANCELLED, "the execution was cancelled", {
          response: response("", report("failed", "delay")),
        }));
      });
      return;
    }
    case "fail": {
      end(id);
      const error = {
        code: "E_SYNTAX",
        message: "unexpected end of input",
        at: { line: 1, column: 5 },
        fix: ["echo text"],
        help: "syntax",
      };
      send(result(id, response("error", report("failed", "parse", { error }))));
      return;
    }
    case "events": {
      end(id);
      const count = Number(rest[0]);
      let text = "";
      if (state.events) {
        for (let index = 1; index <= count; index += 1) {
          text += line(event(id, "job.output", { job_id: "job-1", stream: "stdout", lines: [`line ${index}`] }));
        }
        text += line(event(id, "exec.finished", {}));
      }
      text += line(result(id, response(`events ${count}`, report("ok", "events"))));
      process.stdout.write(text);
      return;
    }
    case "page": {
      end(id);
      send(result(id, response("page 1", report("ok", "page", { next_cursor: "cursor-2" }))));
      return;
    }
    case "crash": {
      process.stderr.write("fatal: simulated failure\n", () => {
        process.exit(CRASH_STATUS);
      });
      return;
    }
    case "noise": {
      end(id);
      process.stdout.write(`${JSON.stringify(result(id, response("noise", report("ok", "noise"))), null, 2)}\n`);
      return;
    }
    case "split": {
      end(id);
      const text = line(result(id, response("split", report("ok", "split"))));
      const size = Math.ceil(text.length / SPLIT_PIECES);
      for (let piece = 0; piece < SPLIT_PIECES; piece += 1) {
        setTimeout(() => {
          process.stdout.write(text.slice(piece * size, (piece + 1) * size));
        }, piece * SPLIT_INTERVAL_MS);
      }
      return;
    }
    case "binary": {
      end(id);
      process.stdout.write(Buffer.from([0xff, 0xfe, 0x0a]));
      return;
    }
    default: {
      end(id);
      const error = { code: "E_UNKNOWN_COMMAND", message: `unknown command ${command}` };
      send(result(id, response("error", report("failed", "bind", { error }))));
    }
  }
}

function handle(message) {
  const { id, method } = message;
  const params = message.params ?? {};
  switch (method) {
    case "initialize":
      state.client = params.client ?? null;
      state.session = params.session ?? null;
      state.configOverrides = params.config_overrides ?? null;
      state.events = params.capabilities?.events === true;
      send(
        result(id, {
          protocol: options.protocol,
          version: "0.0.0",
          abi: { major: 1, minor: 0 },
          language: params.language ?? "en",
          ast: "1.0",
          report: "1.0",
          catalog_rev: "test",
          features: [],
          groups: ["core"],
          limits: { max_inflight: options.maxInflight, max_message_bytes: MESSAGE_LIMIT },
        }),
      );
      return;
    case "run":
      run(id, params);
      return;
    case "cancel":
      state.cancelRequests += 1;
      running.get(params.request_id)?.();
      send(result(id, null));
      return;
    case "info":
      send(
        result(id, {
          version: "0.0.0",
          client: state.client,
          session: state.session,
          config_overrides: state.configOverrides,
          max_observed_inflight: state.maxObservedInFlight,
          cancel_requests: state.cancelRequests,
        }),
      );
      return;
    case "check":
      send(result(id, { bound: {}, effects: [], paths: [], targets: [], program: params.program ?? null }));
      return;
    case "parse":
      send(result(id, { ast: { nurt: "1.0", program: { statements: [] } }, diagnostics: [], variant: params.variant }));
      return;
    case "print":
      send(result(id, { text: `print ${params.variant}` }));
      return;
    case "correct":
      send(result(id, { program: params.program, corrections: [] }));
      return;
    case "page":
      send(result(id, response(`page ${params.cursor} ${params.limit ?? "default"}`, report("ok", "page"))));
      return;
    case "describe":
      send(result(id, { text: `${params.layer} ${params.name ?? "all"} ${params.language}` }));
      return;
    case "help":
      send(result(id, { text: params.topic ?? "index" }));
      return;
    case "catalog":
    case "schema":
    case "history":
    case "space":
      send(result(id, { method, params }));
      return;
    case "shutdown":
      process.stdout.write(line(result(id, null)), () => {
        process.exit(0);
      });
      return;
    default:
      send(failure(id, RPC_METHOD_NOT_FOUND, `method ${method} not found`));
  }
}

let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  for (;;) {
    const index = buffer.indexOf("\n");
    if (index === -1) {
      break;
    }
    const text = buffer.slice(0, index);
    buffer = buffer.slice(index + 1);
    if (text.trim() !== "") {
      handle(JSON.parse(text));
    }
  }
});
process.stdin.on("end", () => {
  process.exit(0);
});
