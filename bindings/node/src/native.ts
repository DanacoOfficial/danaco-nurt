// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/src/native.ts
// Responsibility:   Source file bindings/node/src/native.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import { accessSync, constants, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type { Channel, ChannelEvents } from "./channel.js";
import type { CallHooks, ConnectionLimits } from "./connection.js";
import { NurtError, type StatusName } from "./errors.js";
import { isObject, type JsonObject } from "./json.js";
import { candidatePackages, detectHost, packageName } from "./platform.js";
import type { NurtEvent } from "./types.js";

/** Environment variable with the path of the addon. */
export const LIBRARY_VARIABLE = "NURT_LIBRARY";
/** File name of the addon inside the `bin` directory of a platform package. */
export const ADDON_FILE = "nurt.node";

/** Protocol announced to the client in the result of `initialize`. */
const PROTOCOL = "nurt-rpc/1";
/** Operations of the addon that are answered with `{"result": …}` or `{"error": …}`. */
const NATIVE_OPERATIONS: ReadonlySet<string> = new Set([
  "check",
  "parse",
  "print",
  "correct",
  "page",
  "describe",
  "help",
  "catalog",
  "schema",
  "history",
  "space",
  "info",
]);
/** Failures of the addon are thrown with a message of the form `NURT_E_NAME: description`. */
const FAILURE_PATTERN = /^(NURT_E_(?:TIMEOUT|CANCELLED|ARG|ABI|IO|INTERNAL)): ([\s\S]*)$/;

/** Request started in the addon. */
export interface NativeExecution {
  /** Identifier of the request, as carried by its events. */
  readonly requestId: string;
  /** Requests cancellation. */
  cancel(): void;
  /** Whether the response is available. */
  isFinished(): boolean;
}

/**
 * Item of the stream of a request. The addon delivers events and the response
 * through one function, in order: the response is the last item.
 */
export interface NativeDelivery {
  /** `event`, `response` or `failure`. */
  readonly kind: string;
  /** Event or response as JSON text; for `failure` the text `NURT_E_NAME: description`. */
  readonly text: string;
}

/** Engine of the addon. */
export interface NativeEngine {
  /** Description of the engine as JSON text. */
  info(): string;
  /** Runs an operation other than `run`; resolves with the result as JSON text. */
  call(operation: string, input: string): Promise<string>;
  /** Starts a request; `onMessage` receives every event and then the response. */
  start(request: string, onMessage: (delivery: NativeDelivery) => void): NativeExecution;
}

/** Module exported by the addon. */
export interface NativeAddon {
  /** Opens an engine from options given as JSON text. */
  readonly NativeEngine: new (options?: string | null) => NativeEngine;
}

/** Options of {@link locateAddon}. */
export interface LocateAddonOptions {
  /** Path given by the caller; takes precedence over every other source. */
  readonly library?: string | undefined;
  /** Environment that supplies `NURT_LIBRARY`. */
  readonly env: { readonly [name: string]: string | undefined };
  /**
   * Returns the directory of an installed package, or undefined when the
   * package is not installed. Default: resolution from this package.
   */
  readonly resolvePackage?: ((name: string) => string | undefined) | undefined;
}

/**
 * Finds the addon.
 *
 * Sources, in order: the `library` option, the variable `NURT_LIBRARY` and the
 * platform package of the host. A path given by the option or the variable is
 * used without falling back to the other sources.
 *
 * @returns the path and the places checked; the path is undefined when no
 *   addon is found.
 */
export function locateAddon(options: LocateAddonOptions): { readonly path: string | undefined; readonly checked: readonly string[] } {
  const explicit = options.library ?? nonEmpty(options.env[LIBRARY_VARIABLE]);
  if (explicit !== undefined) {
    const file = resolve(explicit);
    const source = options.library === undefined ? LIBRARY_VARIABLE : "the library option";
    return isFile(file)
      ? { path: file, checked: [] }
      : { path: undefined, checked: [`${source} names ${file}, which is not a file`] };
  }
  const checked: string[] = [];
  const host = detectHost();
  const candidates = candidatePackages(host);
  if (candidates.length === 0) {
    checked.push(`no platform package exists for ${host.platform}-${host.arch}`);
  }
  const resolvePackage = options.resolvePackage ?? resolvePackageDirectory;
  for (const candidate of candidates) {
    const name = packageName(candidate);
    const directory = resolvePackage(name);
    if (directory === undefined) {
      checked.push(`package ${name} is not installed`);
      continue;
    }
    const file = join(directory, "bin", ADDON_FILE);
    if (isFile(file)) {
      return { path: file, checked };
    }
    checked.push(`package ${name} has no addon ${file}`);
  }
  return { path: undefined, checked };
}

/**
 * Loads the addon.
 *
 * @throws {NurtError} `NURT_E_IO` when the file cannot be loaded.
 */
export function loadAddon(path: string): NativeAddon {
  let loaded: unknown;
  try {
    loaded = require(path);
  } catch (error) {
    throw new NurtError("NURT_E_IO", `cannot load the library ${path}: ${error instanceof Error ? error.message : String(error)}`, {
      cause: error,
    });
  }
  if (!isObject(loaded) || typeof loaded["NativeEngine"] !== "function") {
    throw new NurtError("NURT_E_ABI", `${path} is not a Nurt library: it does not export NativeEngine`);
  }
  return loaded as unknown as NativeAddon;
}

/** Converts an exception thrown by the addon into a {@link NurtError}. */
export function nativeFailure(error: unknown): NurtError {
  if (error instanceof NurtError) {
    return error;
  }
  const message = error instanceof Error ? error.message : String(error);
  const match = FAILURE_PATTERN.exec(message);
  if (match === null) {
    return new NurtError("NURT_E_INTERNAL", message, { cause: error });
  }
  return new NurtError(match[1] as StatusName, match[2] ?? "", { cause: error });
}

interface Running {
  readonly cancel: () => void;
  readonly reject: (error: NurtError) => void;
}

/**
 * Channel to an engine inside this process, reached through the addon.
 *
 * The engine is opened by `initialize`. Calls run on the worker threads of the
 * addon and may overlap; `cancel` and `shutdown` need no round trip and are
 * accepted without effect, because a call is cancelled through its signal.
 */
export class NativeChannel implements Channel {
  readonly #addon: NativeAddon;
  readonly #events: ChannelEvents;
  readonly #running = new Set<Running>();
  #engine: NativeEngine | undefined;
  #language: string | undefined;
  #closed = false;

  /**
   * Loads the addon.
   *
   * @throws {NurtError} when the addon cannot be loaded.
   */
  constructor(path: string, events: ChannelEvents) {
    this.#addon = loadAddon(path);
    this.#events = events;
  }

  get usable(): boolean {
    return !this.#closed;
  }

  setLimits(_limits: ConnectionLimits): void {}

  async call(method: string, params: unknown, hooks: CallHooks = {}): Promise<unknown> {
    if (this.#closed) {
      throw new NurtError("NURT_E_CANCELLED", "the client is closing");
    }
    if (hooks.signal?.aborted === true && hooks.awaitCancel !== true) {
      throw cancelled(hooks.signal);
    }
    switch (method) {
      case "initialize":
        return this.#initialize(params);
      case "run":
        return this.#run(params, hooks);
      case "cancel":
      case "shutdown":
        return {};
      default:
        return this.#operation(method, params);
    }
  }

  close(_graceMs: number): Promise<void> {
    this.terminate(new NurtError("NURT_E_CANCELLED", "the client was closed"));
    return Promise.resolve();
  }

  terminate(error: NurtError): void {
    this.#closed = true;
    for (const running of [...this.#running]) {
      running.reject(error);
      running.cancel();
    }
    this.#running.clear();
  }

  #initialize(params: unknown): JsonObject {
    const options = isObject(params) ? params : {};
    const overrides = options["config_overrides"];
    if (isObject(overrides) && Object.keys(overrides).length > 0) {
      throw new NurtError("NURT_E_ARG", "configuration overrides are not supported by the library backend");
    }
    const session = options["session"];
    const language = options["language"];
    this.#language = typeof language === "string" ? language : undefined;
    const engineOptions = typeof session === "string" ? JSON.stringify({ session }) : null;
    let engine: NativeEngine;
    try {
      engine = new this.#addon.NativeEngine(engineOptions);
    } catch (error) {
      throw nativeFailure(error);
    }
    this.#engine = engine;
    const info = this.#infoOf(engine);
    const formats = isObject(info["formats"]) ? info["formats"] : {};
    return {
      protocol: PROTOCOL,
      version: info["version"] ?? null,
      abi: null,
      language: null,
      ast: formats["ast"] ?? null,
      report: formats["report"] ?? null,
      catalog_rev: null,
      features: Array.isArray(info["features"]) ? info["features"] : [],
      groups: Array.isArray(info["services"]) ? info["services"] : [],
    };
  }

  #infoOf(engine: NativeEngine): JsonObject {
    let parsed: unknown;
    try {
      parsed = JSON.parse(engine.info());
    } catch (error) {
      throw nativeFailure(error);
    }
    if (!isObject(parsed)) {
      throw new NurtError("NURT_E_INTERNAL", "the library returned an invalid description");
    }
    return parsed;
  }

  #open(): NativeEngine {
    if (this.#engine === undefined) {
      throw new NurtError("NURT_E_ARG", "the library is not initialized");
    }
    return this.#engine;
  }

  /**
   * Adds the language of the client to what names none. The engine takes only
   * the session when it is opened, so the language is written into each
   * request envelope and into the parameters of `describe` and `help`.
   */
  #withLanguage(method: string, params: unknown): unknown {
    const language = this.#language;
    if (language === undefined || !isObject(params)) {
      return params;
    }
    if (method === "describe" || method === "help") {
      return params["language"] === undefined ? { ...params, language } : params;
    }
    if (method !== "run" && method !== "check") {
      return params;
    }
    const options = isObject(params["options"]) ? params["options"] : {};
    return options["language"] === undefined ? { ...params, options: { ...options, language } } : params;
  }

  async #operation(method: string, params: unknown): Promise<unknown> {
    if (!NATIVE_OPERATIONS.has(method)) {
      throw new NurtError("NURT_E_ARG", `the library does not implement ${method}`);
    }
    const engine = this.#open();
    let text: string;
    try {
      text = await engine.call(method, JSON.stringify(this.#withLanguage(method, params) ?? null));
    } catch (error) {
      throw nativeFailure(error);
    }
    const document = parseJson(text);
    if (isObject(document) && isObject(document["error"])) {
      // The same answer as of `nurt serve`: a response whose report failed.
      return { nurt: "1.0", report: { nurt: "1.0", status: "failed", kind: method, error: document["error"] } };
    }
    return isObject(document) ? document["result"] : undefined;
  }

  async #run(params: unknown, hooks: CallHooks): Promise<unknown> {
    const engine = this.#open();
    return new Promise<unknown>((resolveCall, rejectCall) => {
      const signal = hooks.signal;
      let execution: NativeExecution | undefined;
      const running: Running = {
        reject: (error) => {
          release();
          rejectCall(error);
        },
        cancel: () => execution?.cancel(),
      };
      const onAbort = (): void => {
        if (hooks.awaitCancel === true) {
          // The response of the cancelled execution ends the call.
          running.cancel();
          return;
        }
        running.reject(cancelled(signal));
        running.cancel();
      };
      const release = (): void => {
        signal?.removeEventListener("abort", onAbort);
        this.#running.delete(running);
      };
      const onMessage = (delivery: NativeDelivery): void => {
        if (!this.#running.has(running)) {
          return;
        }
        try {
          if (delivery.kind === "event") {
            const event = parseJson(delivery.text) as NurtEvent;
            hooks.onEvent?.(event);
            this.#events.onNotification("nurt/event", event);
          } else if (delivery.kind === "response") {
            const response = parseJson(delivery.text);
            release();
            resolveCall(response);
          } else {
            release();
            rejectCall(nativeFailure(new Error(delivery.text)));
          }
        } catch (error) {
          release();
          rejectCall(nativeFailure(error));
        }
      };
      this.#running.add(running);
      signal?.addEventListener("abort", onAbort, { once: true });
      try {
        execution = engine.start(JSON.stringify(this.#withLanguage("run", params) ?? null), onMessage);
      } catch (error) {
        running.reject(nativeFailure(error));
        return;
      }
      if (hooks.awaitCancel === true && signal?.aborted === true) {
        execution.cancel();
      }
    });
  }
}

function cancelled(signal: AbortSignal | undefined): NurtError {
  return new NurtError("NURT_E_CANCELLED", "the call was cancelled", { cause: signal?.reason });
}

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new NurtError("NURT_E_INTERNAL", "the library returned text that is not JSON", { cause: error });
  }
}

function nonEmpty(value: string | undefined): string | undefined {
  return value === undefined || value === "" ? undefined : value;
}

function isFile(path: string): boolean {
  try {
    if (!statSync(path).isFile()) {
      return false;
    }
    accessSync(path, constants.R_OK);
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
