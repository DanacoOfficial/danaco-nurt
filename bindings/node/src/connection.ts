// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { NurtError, type StatusName } from "./errors.js";
import { encodeMessage, MessageDecoder, ProtocolError } from "./framing.js";
import { isObject, type JsonObject } from "./json.js";
import { toResponse } from "./response.js";

/** Largest message accepted from the server, in bytes. */
const MAX_INCOMING_MESSAGE_BYTES = 256 * 1024 * 1024;
/** Number of trailing characters of the standard error of the server kept for error messages. */
const STDERR_TAIL_CHARACTERS = 2048;

const JSON_RPC_VERSION = "2.0";
const RPC_INVALID_PARAMS = -32602;
const RPC_METHOD_NOT_FOUND = -32601;
/**
 * The application error codes `-32001` to `-32006` correspond one to one to
 * the values 1 to 6 of `nurt_status`: code = APPLICATION_ERROR_BASE - status.
 */
const APPLICATION_ERROR_BASE = -32000;
const STATUS_BY_VALUE: readonly StatusName[] = [
  "NURT_E_TIMEOUT",
  "NURT_E_CANCELLED",
  "NURT_E_ARG",
  "NURT_E_ABI",
  "NURT_E_IO",
  "NURT_E_INTERNAL",
];

/** Returns the status name of a JSON-RPC error code. */
export function statusOfRpcCode(code: number): StatusName {
  const value = APPLICATION_ERROR_BASE - code;
  const status = value >= 1 ? STATUS_BY_VALUE[value - 1] : undefined;
  if (status !== undefined) {
    return status;
  }
  return code === RPC_INVALID_PARAMS ? "NURT_E_ARG" : "NURT_E_INTERNAL";
}

/** Converts the `error` member of a JSON-RPC response into a {@link NurtError}. */
export function errorOfRpc(value: unknown): NurtError {
  if (!isObject(value) || typeof value["code"] !== "number" || typeof value["message"] !== "string") {
    return new NurtError("NURT_E_INTERNAL", "the server returned a malformed error");
  }
  const code = value["code"];
  const data = value["data"];
  const candidate = isObject(data) ? data["response"] : undefined;
  const response = isObject(candidate) && isObject(candidate["report"]) ? toResponse(candidate) : undefined;
  return new NurtError(statusOfRpcCode(code), value["message"], { rpcCode: code, detail: data, response });
}

/** Options of a {@link Connection}. */
export interface ConnectionOptions {
  /** Path of the executable. */
  readonly executable: string;
  /** Arguments of the executable. */
  readonly args: readonly string[];
  /** Environment of the server. */
  readonly env: NodeJS.ProcessEnv;
  /** Working directory of the server. */
  readonly cwd: string | undefined;
  /** Receives every notification of the server. */
  readonly onNotification: (method: string, params: unknown) => void;
  /** Called once when the connection ends, with the error given to the calls in progress. */
  readonly onClose: (error: NurtError) => void;
}

/** Options of {@link Connection.call}. */
export interface CallHooks {
  /** Cancels the call when aborted. */
  readonly signal?: AbortSignal | undefined;
  /** Called with the request identifier before the request is written. */
  readonly onSent?: ((id: number) => void) | undefined;
}

/** Limits announced by the server in the result of `initialize`. */
export interface ConnectionLimits {
  /** Largest number of requests in progress. */
  readonly maxInFlight?: number | undefined;
  /** Largest request, in bytes. */
  readonly maxMessageBytes?: number | undefined;
}

interface Call {
  readonly method: string;
  readonly params: unknown;
  readonly limited: boolean;
  readonly hooks: CallHooks;
  readonly resolve: (value: unknown) => void;
  readonly reject: (error: NurtError) => void;
  readonly onAbort: () => void;
  id: number | undefined;
  settled: boolean;
}

/**
 * Connection to one `nurt serve --stdio` process speaking `nurt-rpc/1`
 * (JSON-RPC 2.0, one message per line).
 *
 * Requests are answered in any order and matched by identifier. Requests
 * beyond the `max_inflight` limit of the server wait in a queue. The process
 * and its pipes hold the event loop open only while requests are in progress.
 */
export class Connection {
  readonly #options: ConnectionOptions;
  readonly #child: ChildProcessWithoutNullStreams;
  readonly #decoder = new MessageDecoder(MAX_INCOMING_MESSAGE_BYTES);
  readonly #pending = new Map<number, Call>();
  readonly #queue: Call[] = [];
  readonly #ended: Promise<void>;
  #markEnded: () => void = () => undefined;
  #nextId = 1;
  #inFlight = 0;
  #maxInFlight = Number.POSITIVE_INFINITY;
  #maxMessageBytes = Number.POSITIVE_INFINITY;
  #failure: NurtError | undefined;
  #closing = false;
  #finished = false;
  #referenced = true;
  #stderr = "";

  /** Starts the server process. Failures to start are reported to the calls. */
  constructor(options: ConnectionOptions) {
    this.#options = options;
    this.#ended = new Promise<void>((resolve) => {
      this.#markEnded = resolve;
    });
    const child = spawn(options.executable, [...options.args], {
      stdio: "pipe",
      env: options.env,
      cwd: options.cwd,
      windowsHide: true,
    });
    this.#child = child;
    child.stdout.on("data", (chunk: Uint8Array) => {
      this.#receive(chunk);
    });
    child.stdout.on("end", () => {
      this.#endOfOutput();
    });
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (text: string) => {
      this.#stderr = (this.#stderr + text).slice(-STDERR_TAIL_CHARACTERS);
    });
    child.stdin.on("error", ignoreWriteError);
    child.on("error", (error: Error) => {
      this.#fail(new NurtError("NURT_E_IO", `cannot run ${options.executable}: ${error.message}`, { cause: error }));
      this.#finish();
    });
    child.on("close", (code: number | null, signal: NodeJS.Signals | null) => {
      this.#fail(this.#exitError(code, signal));
      this.#finish();
    });
    this.#updateReference();
  }

  /** Whether the connection accepts calls. */
  get usable(): boolean {
    return this.#failure === undefined && !this.#closing;
  }

  /** Applies the limits announced by the server. */
  setLimits(limits: ConnectionLimits): void {
    if (limits.maxInFlight !== undefined) {
      this.#maxInFlight = limits.maxInFlight;
    }
    if (limits.maxMessageBytes !== undefined) {
      this.#maxMessageBytes = limits.maxMessageBytes;
    }
    this.#pump();
  }

  /**
   * Sends a request and resolves with its result.
   *
   * @throws {NurtError} the error returned by the server, `NURT_E_CANCELLED`
   *   when the signal is aborted, or the error that ended the connection.
   */
  call(method: string, params: unknown, hooks: CallHooks = {}): Promise<unknown> {
    return this.#start(method, params, hooks, true);
  }

  /**
   * Closes the connection: sends `shutdown`, closes the input of the server
   * and waits for it to exit, terminating it after `graceMs` milliseconds.
   */
  async close(graceMs: number): Promise<void> {
    if (this.#finished) {
      return;
    }
    if (this.#failure === undefined) {
      const shutdown = this.#start("shutdown", undefined, {}, false);
      this.#closing = true;
      await withDeadline(shutdown.catch(() => undefined), graceMs);
    }
    this.#closing = true;
    this.#updateReference();
    this.#child.stdin.end();
    if (!(await withDeadline(this.#ended.then(() => true), graceMs))) {
      this.#child.kill("SIGKILL");
    }
    await this.#ended;
  }

  /** Ends the connection with an error and terminates the server. */
  terminate(error: NurtError): void {
    this.#fail(error);
    this.#child.kill("SIGKILL");
  }

  #start(method: string, params: unknown, hooks: CallHooks, limited: boolean): Promise<unknown> {
    return new Promise<unknown>((resolve, reject) => {
      if (this.#failure !== undefined) {
        reject(this.#failure);
        return;
      }
      if (this.#closing) {
        reject(new NurtError("NURT_E_CANCELLED", "the client is closing"));
        return;
      }
      const signal = hooks.signal;
      if (signal?.aborted === true) {
        reject(cancelled(signal));
        return;
      }
      const call: Call = {
        method,
        params,
        limited,
        hooks,
        resolve,
        reject,
        onAbort: () => {
          this.#abort(call);
        },
        id: undefined,
        settled: false,
      };
      signal?.addEventListener("abort", call.onAbort, { once: true });
      if (limited && this.#inFlight >= this.#maxInFlight) {
        this.#queue.push(call);
        this.#updateReference();
        return;
      }
      this.#send(call);
    });
  }

  #send(call: Call): void {
    const id = this.#nextId;
    this.#nextId += 1;
    const message: JsonObject = { jsonrpc: JSON_RPC_VERSION, id, method: call.method };
    if (call.params !== undefined) {
      message["params"] = call.params;
    }
    const line = encodeMessage(message);
    if (Number.isFinite(this.#maxMessageBytes)) {
      const size = utf8.encode(line).length;
      if (size > this.#maxMessageBytes) {
        this.#settle(
          call,
          new NurtError("NURT_E_ARG", `the request of ${size} bytes exceeds the limit of ${this.#maxMessageBytes} bytes of the server`),
        );
        return;
      }
    }
    call.id = id;
    this.#pending.set(id, call);
    if (call.limited) {
      this.#inFlight += 1;
    }
    this.#updateReference();
    call.hooks.onSent?.(id);
    this.#child.stdin.write(line);
  }

  #abort(call: Call): void {
    if (call.settled) {
      return;
    }
    const reason = cancelled(call.hooks.signal);
    if (call.id === undefined) {
      const index = this.#queue.indexOf(call);
      if (index !== -1) {
        this.#queue.splice(index, 1);
      }
      this.#settle(call, reason);
      this.#updateReference();
      return;
    }
    this.#settle(call, reason);
    if (this.#failure === undefined) {
      this.#start("cancel", { request_id: call.id }, {}, false).catch(() => undefined);
    }
  }

  #settle(call: Call, outcome: { readonly value: unknown } | NurtError): void {
    if (call.settled) {
      return;
    }
    call.settled = true;
    call.hooks.signal?.removeEventListener("abort", call.onAbort);
    if (outcome instanceof NurtError) {
      call.reject(outcome);
    } else {
      call.resolve(outcome.value);
    }
  }

  #pump(): void {
    while (this.#inFlight < this.#maxInFlight && this.#failure === undefined) {
      const next = this.#queue.shift();
      if (next === undefined) {
        break;
      }
      this.#send(next);
    }
  }

  #receive(chunk: Uint8Array): void {
    if (this.#failure !== undefined) {
      return;
    }
    try {
      this.#decoder.push(chunk, (message) => {
        this.#dispatch(message);
      });
    } catch (error) {
      if (error instanceof ProtocolError) {
        this.terminate(new NurtError("NURT_E_INTERNAL", `invalid message from the server: ${error.message}`, { cause: error }));
        return;
      }
      throw error;
    }
  }

  #endOfOutput(): void {
    if (this.#failure !== undefined) {
      return;
    }
    try {
      this.#decoder.end();
    } catch (error) {
      if (error instanceof ProtocolError) {
        this.#fail(new NurtError("NURT_E_INTERNAL", `invalid message from the server: ${error.message}`, { cause: error }));
        return;
      }
      throw error;
    }
  }

  #dispatch(message: JsonObject): void {
    if (message["jsonrpc"] !== JSON_RPC_VERSION) {
      throw new ProtocolError("a message is not JSON-RPC 2.0");
    }
    const id = message["id"];
    const method = message["method"];
    if (typeof method === "string") {
      if (id === undefined) {
        this.#options.onNotification(method, message["params"]);
        return;
      }
      this.#child.stdin.write(
        encodeMessage({
          jsonrpc: JSON_RPC_VERSION,
          id,
          error: { code: RPC_METHOD_NOT_FOUND, message: `the client does not implement ${method}` },
        }),
      );
      return;
    }
    const hasError = message["error"] !== undefined;
    if (!hasError && !("result" in message)) {
      throw new ProtocolError("a message is neither a request, a notification nor a response");
    }
    if (id === null && hasError) {
      this.terminate(errorOfRpc(message["error"]));
      return;
    }
    if (typeof id !== "number") {
      throw new ProtocolError("a response has no numeric id");
    }
    const call = this.#pending.get(id);
    if (call === undefined) {
      return;
    }
    this.#pending.delete(id);
    if (call.limited) {
      this.#inFlight -= 1;
    }
    this.#settle(call, hasError ? errorOfRpc(message["error"]) : { value: message["result"] });
    this.#pump();
    this.#updateReference();
  }

  #exitError(code: number | null, signal: NodeJS.Signals | null): NurtError {
    if (this.#closing) {
      return new NurtError("NURT_E_CANCELLED", "the client was closed");
    }
    const how = signal === null ? `exited with code ${String(code)}` : `was terminated by signal ${signal}`;
    const stderr = this.#stderr.trim();
    const detail = stderr === "" ? "" : `; standard error: ${stderr}`;
    return new NurtError("NURT_E_INTERNAL", `the server ${how}${detail}`);
  }

  #fail(error: NurtError): void {
    if (this.#failure !== undefined) {
      return;
    }
    this.#failure = error;
    const calls = [...this.#queue, ...this.#pending.values()];
    this.#queue.length = 0;
    this.#pending.clear();
    this.#inFlight = 0;
    for (const call of calls) {
      this.#settle(call, error);
    }
    this.#updateReference();
  }

  #finish(): void {
    if (this.#finished) {
      return;
    }
    this.#finished = true;
    this.#markEnded();
    this.#options.onClose(this.#failure ?? new NurtError("NURT_E_INTERNAL", "the server exited"));
  }

  #updateReference(): void {
    const active = !this.#finished && (this.#closing || this.#pending.size > 0 || this.#queue.length > 0);
    if (active === this.#referenced) {
      return;
    }
    this.#referenced = active;
    for (const handle of [this.#child, this.#child.stdin, this.#child.stdout, this.#child.stderr]) {
      const target = handle as unknown as Referable;
      const method = active ? target.ref : target.unref;
      method?.call(target);
    }
  }
}

/** Handle that can hold the event loop open; the pipes of a child process are such handles. */
interface Referable {
  readonly ref?: () => void;
  readonly unref?: () => void;
}

const utf8 = new TextEncoder();

function cancelled(signal: AbortSignal | undefined): NurtError {
  return new NurtError("NURT_E_CANCELLED", "the call was cancelled", { cause: signal?.reason });
}

/**
 * Write errors on the input of the server (for example `EPIPE` after it has
 * exited) are reported through the end of the process.
 */
function ignoreWriteError(): void {}

/** Resolves with the value of `promise`, or with `false` after `ms` milliseconds. */
function withDeadline<T>(promise: Promise<T>, ms: number): Promise<T | false> {
  return new Promise<T | false>((resolve, reject) => {
    const timer = setTimeout(() => {
      resolve(false);
    }, ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}
