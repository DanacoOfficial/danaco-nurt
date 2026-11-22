// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { EventEmitter } from "node:events";
import { Connection } from "./connection.js";
import { NurtError, NurtLanguageError } from "./errors.js";
import { Execution } from "./execution.js";
import { isObject, type JsonObject } from "./json.js";
import { locateExecutable } from "./locate.js";
import { toResponse, type Response } from "./response.js";
import type {
  Ast,
  DescribeOptions,
  NurtEvent,
  NurtOptions,
  ProgramInput,
  RunOptions,
  StartOptions,
  Variant,
} from "./types.js";

/** Protocol spoken with the server. */
const PROTOCOL = "nurt-rpc/1";
/** Accepted protocol identifiers announced by the server: major version 1, any minor version. */
const PROTOCOL_PATTERN = /^nurt-rpc\/1(\.[0-9]+)?$/;
/** Format version of the request envelope `nurt-request/1`. */
const REQUEST_FORMAT = "1.0";
/** Name of this client announced in `initialize`. */
const CLIENT_NAME = "nurt-node";
/** Version announced when the manifest of the package cannot be read. */
const UNKNOWN_VERSION = "unknown";
/** Notification that carries events. */
const EVENT_METHOD = "nurt/event";
/** Default arguments of the executable. */
const SERVE_ARGUMENTS: readonly string[] = ["serve", "--stdio"];
/** Time given to the server to finish after `shutdown`, in milliseconds. */
const SHUTDOWN_GRACE_MS = 5000;
/** Options of {@link RunOptions} that belong to the client and are not sent. */
const CLIENT_OPTIONS: ReadonlySet<string> = new Set(["signal", "raiseOnError", "onEvent"]);

interface Session {
  readonly connection: Connection;
  readonly server: JsonObject;
}

/**
 * Client of Danaco Nurt.
 *
 * The client runs `nurt serve --stdio` as a child process on the first call and
 * speaks `nurt-rpc/1` with it. Calls may run concurrently. When the server
 * exits unexpectedly, the calls in progress fail with `NURT_E_INTERNAL` and
 * the next call starts a new server.
 *
 * Events of the server are emitted as `event` ({@link NurtEvent}); the end of
 * a server that exits unexpectedly is emitted as `exit` with the
 * {@link NurtError} given to the calls in progress.
 *
 * Errors of the Nurt language are returned in {@link Response.error}; call
 * failures are thrown as {@link NurtError}.
 */
export class Nurt extends EventEmitter {
  readonly #options: NurtOptions;
  readonly #routes = new Map<string, (event: NurtEvent) => void>();
  #current: Promise<Session> | undefined;
  #session: Session | undefined;
  #closed = false;

  /**
   * Creates a client. The server is started by the first call.
   *
   * @throws {NurtError} `NURT_E_ARG` for an unknown backend.
   */
  constructor(options: NurtOptions = {}) {
    super();
    const backend: unknown = options.backend ?? "auto";
    if (backend !== "auto" && backend !== "process") {
      throw new NurtError("NURT_E_ARG", `unknown backend ${String(backend)}; use auto or process`);
    }
    this.#options = options;
  }

  /**
   * Runs a program and returns its response.
   *
   * @throws {NurtError} on a call failure.
   * @throws {NurtLanguageError} when `raiseOnError` is set and the report
   *   carries an error.
   */
  async run(program: ProgramInput, options: RunOptions = {}): Promise<Response> {
    const session = await this.#connect();
    const result = await session.connection.call("run", requestEnvelope(program, options), { signal: options.signal });
    return finish(toResponse(result), options.raiseOnError);
  }

  /**
   * Starts a program and returns its execution, which delivers the events of
   * the program and its response.
   */
  start(program: ProgramInput, options: StartOptions = {}): Execution {
    return new Execution(
      async (control) => {
        const session = await this.#connect();
        let route: string | undefined;
        try {
          const result = await session.connection.call("run", requestEnvelope(program, options), {
            signal: control.signal,
            onSent: (id) => {
              route = String(id);
              this.#routes.set(route, control.deliver);
            },
          });
          return finish(toResponse(result), options.raiseOnError);
        } finally {
          if (route !== undefined) {
            this.#routes.delete(route);
          }
        }
      },
      { signal: options.signal, onEvent: options.onEvent },
    );
  }

  /**
   * Binds a program without running it and returns its effects, paths,
   * targets and corrections (`nurt-check/1`).
   */
  async check(program: ProgramInput, options: RunOptions = {}): Promise<JsonObject> {
    const session = await this.#connect();
    const result = await session.connection.call("check", requestEnvelope(program, options), { signal: options.signal });
    return objectResult(result, "check");
  }

  /** Parses program text and returns the syntax tree and the diagnostics. */
  async parse(program: string, variant: Variant = "a"): Promise<JsonObject> {
    return objectResult(await this.#call("parse", { program, variant }), "parse");
  }

  /** Prints a syntax tree as program text in a notation variant. */
  async print(ast: Ast, variant: Variant = "a"): Promise<string> {
    return textResult(await this.#call("print", { ast, variant }), "print");
  }

  /** Corrects program text and returns the canonical program and the corrections. */
  async correct(program: string, variant: Variant = "a"): Promise<JsonObject> {
    return objectResult(await this.#call("correct", { program, variant }), "correct");
  }

  /** Returns a further page of an earlier result. */
  async page(cursor: string, limit?: number): Promise<Response> {
    return toResponse(await this.#call("page", compact({ cursor, limit })));
  }

  /** Returns the description of a layer of the language. */
  async describe(options: DescribeOptions = {}): Promise<string> {
    const params = compact({ layer: options.layer ?? "core", name: options.name, language: options.language ?? "en" });
    return textResult(await this.#call("describe", params), "describe");
  }

  /** Returns a help topic, or the index of topics. */
  async help(topic?: string): Promise<string> {
    return textResult(await this.#call("help", compact({ topic })), "help");
  }

  /** Returns the command catalog, or a part of it. */
  async catalog(what?: string): Promise<JsonObject> {
    return objectResult(await this.#call("catalog", compact({ what })), "catalog");
  }

  /** Returns a JSON schema of the interface. */
  async schema(which: string): Promise<JsonObject> {
    return objectResult(await this.#call("schema", { which }), "schema");
  }

  /** Returns operations of the journal. */
  async history(query?: string): Promise<JsonObject> {
    return objectResult(await this.#call("history", compact({ query })), "history");
  }

  /** Returns the space state of the journal volumes. */
  async space(volume?: string): Promise<JsonObject> {
    return objectResult(await this.#call("space", compact({ volume })), "space");
  }

  /** Returns the version, interface versions and features of the server. */
  async info(): Promise<JsonObject> {
    return objectResult(await this.#call("info", {}), "info");
  }

  /**
   * Closes the client: the server finishes its operations and exits. Calls in
   * progress fail with `NURT_E_CANCELLED`; later calls fail with `NURT_E_ARG`.
   */
  async close(): Promise<void> {
    this.#closed = true;
    const current = this.#current;
    this.#current = undefined;
    this.#session = undefined;
    if (current === undefined) {
      return;
    }
    let session: Session;
    try {
      session = await current;
    } catch {
      return;
    }
    await session.connection.close(SHUTDOWN_GRACE_MS);
  }

  async #call(method: string, params: JsonObject): Promise<unknown> {
    const session = await this.#connect();
    return session.connection.call(method, params);
  }

  #connect(): Promise<Session> {
    if (this.#closed) {
      return Promise.reject(new NurtError("NURT_E_ARG", "the client is closed"));
    }
    const current = this.#current;
    if (current !== undefined && (this.#session === undefined || this.#session.connection.usable)) {
      return current;
    }
    this.#session = undefined;
    const next = this.#open();
    this.#current = next;
    next.then(
      (session) => {
        if (this.#current === next) {
          this.#session = session;
        }
      },
      () => {
        if (this.#current === next) {
          this.#current = undefined;
        }
      },
    );
    return next;
  }

  async #open(): Promise<Session> {
    const options = this.#options;
    const env = options.env === undefined ? process.env : { ...process.env, ...options.env };
    const executable = locateExecutable({ executable: options.executable, env, includePath: true });
    const connection: Connection = new Connection({
      executable,
      args: options.args ?? SERVE_ARGUMENTS,
      env,
      cwd: options.cwd,
      onNotification: (method, params) => {
        this.#notification(method, params);
      },
      onClose: (error) => {
        if (this.#session?.connection === connection) {
          this.#session = undefined;
          this.#current = undefined;
          this.emit("exit", error);
        }
      },
    });
    try {
      const result = await connection.call(
        "initialize",
        compact({
          protocol: PROTOCOL,
          client: { name: CLIENT_NAME, version: clientVersion() },
          language: options.language ?? "en",
          session: options.session,
          config_overrides: options.config,
          capabilities: { events: true },
        }),
      );
      const server = checkInitialize(result);
      const limits = isObject(server["limits"]) ? server["limits"] : {};
      connection.setLimits({
        maxInFlight: positiveInteger(limits["max_inflight"]),
        maxMessageBytes: positiveInteger(limits["max_message_bytes"]),
      });
      return { connection, server };
    } catch (error) {
      const failure = error instanceof NurtError ? error : new NurtError("NURT_E_INTERNAL", String(error), { cause: error });
      connection.terminate(failure);
      throw failure;
    }
  }

  #notification(method: string, params: unknown): void {
    if (method !== EVENT_METHOD || !isObject(params) || typeof params["t"] !== "string") {
      return;
    }
    const event = params as NurtEvent;
    const requestId = event.request_id;
    if (requestId !== undefined) {
      this.#routes.get(String(requestId))?.(event);
    }
    this.emit("event", event);
  }
}

function requestEnvelope(program: ProgramInput, options: RunOptions): JsonObject {
  const wire: JsonObject = {};
  for (const [key, value] of Object.entries(options)) {
    if (value !== undefined && !CLIENT_OPTIONS.has(key)) {
      wire[key] = value;
    }
  }
  const body: JsonObject = typeof program === "string" ? { program } : { ...program };
  const envelope: JsonObject = { nurt: REQUEST_FORMAT, ...body };
  if (Object.keys(wire).length > 0) {
    envelope["options"] = wire;
  }
  return envelope;
}

function finish(response: Response, raiseOnError: boolean | undefined): Response {
  const error = response.error;
  if (raiseOnError === true && error !== undefined) {
    throw new NurtLanguageError(response, error);
  }
  return response;
}

function checkInitialize(result: unknown): JsonObject {
  if (!isObject(result)) {
    throw new NurtError("NURT_E_INTERNAL", "the server returned an invalid initialize result");
  }
  const protocol = result["protocol"];
  if (typeof protocol !== "string" || !PROTOCOL_PATTERN.test(protocol)) {
    throw new NurtError("NURT_E_ABI", `the server speaks ${String(protocol)}; this client requires ${PROTOCOL}`);
  }
  return result;
}

function objectResult(result: unknown, method: string): JsonObject {
  if (!isObject(result)) {
    throw new NurtError("NURT_E_INTERNAL", `the server returned an invalid result for ${method}`);
  }
  return result;
}

function textResult(result: unknown, method: string): string {
  if (typeof result === "string") {
    return result;
  }
  if (isObject(result) && typeof result["text"] === "string") {
    return result["text"];
  }
  throw new NurtError("NURT_E_INTERNAL", `the server returned an invalid result for ${method}`);
}

/** Returns the members of an object whose value is not undefined. */
function compact(members: { readonly [key: string]: unknown }): JsonObject {
  const result: JsonObject = {};
  for (const [key, value] of Object.entries(members)) {
    if (value !== undefined) {
      result[key] = value;
    }
  }
  return result;
}

function positiveInteger(value: unknown): number | undefined {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : undefined;
}

function clientVersion(): string {
  try {
    const manifest: unknown = JSON.parse(readFileSync(join(__dirname, "..", "package.json"), "utf8"));
    return isObject(manifest) && typeof manifest["version"] === "string" ? manifest["version"] : UNKNOWN_VERSION;
  } catch {
    return UNKNOWN_VERSION;
  }
}
