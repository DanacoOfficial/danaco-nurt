// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import { EventEmitter } from "node:events";
import { NurtError } from "./errors.js";
import type { Response } from "./response.js";
import type { NurtEvent } from "./types.js";

/** Means by which a running execution is cancelled and receives its events. */
export interface ExecutionControl {
  /** Aborted when the execution is cancelled. */
  readonly signal: AbortSignal;
  /** Delivers an event of the execution. */
  readonly deliver: (event: NurtEvent) => void;
}

/** Options of an {@link Execution}. */
export interface ExecutionInit {
  /** Cancels the execution when aborted. */
  readonly signal?: AbortSignal | undefined;
  /** Receives every event of the execution. */
  readonly onEvent?: ((event: NurtEvent) => void) | undefined;
}

interface Subscriber {
  readonly events: NurtEvent[];
  wake: ((result: IteratorResult<NurtEvent, undefined>) => void) | undefined;
}

/**
 * Program started by {@link Nurt.start}.
 *
 * Events of the execution are emitted as `event` and can be iterated with
 * `for await`; the iteration ends when the execution ends. The outcome is
 * given by {@link Execution.wait}.
 */
export class Execution extends EventEmitter implements AsyncIterable<NurtEvent> {
  readonly #controller = new AbortController();
  readonly #result: Promise<Response>;
  readonly #subscribers = new Set<Subscriber>();
  readonly #signal: AbortSignal | undefined;
  readonly #onSignalAbort = (): void => {
    this.cancel();
  };
  #done = false;

  /**
   * Starts an execution. Instances are created by {@link Nurt.start}.
   *
   * @param begin sends the request and resolves with its response.
   */
  constructor(begin: (control: ExecutionControl) => Promise<Response>, init: ExecutionInit = {}) {
    super();
    if (init.onEvent !== undefined) {
      this.on("event", init.onEvent);
    }
    this.#signal = init.signal;
    if (init.signal?.aborted === true) {
      this.#controller.abort(init.signal.reason);
    } else {
      init.signal?.addEventListener("abort", this.#onSignalAbort, { once: true });
    }
    this.#result = begin({
      signal: this.#controller.signal,
      deliver: (event) => {
        this.#deliver(event);
      },
    });
    this.#result.then(
      () => {
        this.#end();
      },
      () => {
        this.#end();
      },
    );
  }

  /** Whether the execution has ended. */
  get done(): boolean {
    return this.#done;
  }

  /**
   * Waits for the response.
   *
   * @param timeoutMs time limit of this wait; the execution continues after it.
   * @throws {NurtError} `NURT_E_TIMEOUT` when the time limit passes,
   *   `NURT_E_CANCELLED` after {@link Execution.cancel}, or another call
   *   failure.
   * @throws {NurtLanguageError} when started with `raiseOnError: true` and the
   *   report carries an error.
   */
  wait(timeoutMs?: number): Promise<Response> {
    if (timeoutMs === undefined) {
      return this.#result;
    }
    return new Promise<Response>((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new NurtError("NURT_E_TIMEOUT", `the execution is still running after ${timeoutMs} ms`));
      }, timeoutMs);
      this.#result.then(
        (response) => {
          clearTimeout(timer);
          resolve(response);
        },
        (error: unknown) => {
          clearTimeout(timer);
          reject(error);
        },
      );
    });
  }

  /** Cancels the execution. Has no effect after the execution has ended. */
  cancel(): void {
    if (!this.#done) {
      this.#controller.abort();
    }
  }

  [Symbol.asyncIterator](): AsyncIterator<NurtEvent, undefined> {
    const subscriber: Subscriber = { events: [], wake: undefined };
    if (!this.#done) {
      this.#subscribers.add(subscriber);
    }
    return {
      next: (): Promise<IteratorResult<NurtEvent, undefined>> => {
        const event = subscriber.events.shift();
        if (event !== undefined) {
          return Promise.resolve({ done: false, value: event });
        }
        if (!this.#subscribers.has(subscriber)) {
          return Promise.resolve({ done: true, value: undefined });
        }
        return new Promise((resolve) => {
          subscriber.wake = resolve;
        });
      },
      return: (): Promise<IteratorResult<NurtEvent, undefined>> => {
        this.#subscribers.delete(subscriber);
        subscriber.events.length = 0;
        return Promise.resolve({ done: true, value: undefined });
      },
    };
  }

  #deliver(event: NurtEvent): void {
    if (this.#done) {
      return;
    }
    this.emit("event", event);
    for (const subscriber of this.#subscribers) {
      const wake = subscriber.wake;
      if (wake === undefined) {
        subscriber.events.push(event);
      } else {
        subscriber.wake = undefined;
        wake({ done: false, value: event });
      }
    }
  }

  #end(): void {
    this.#done = true;
    this.#signal?.removeEventListener("abort", this.#onSignalAbort);
    for (const subscriber of this.#subscribers) {
      const wake = subscriber.wake;
      subscriber.wake = undefined;
      wake?.({ done: true, value: undefined });
    }
    this.#subscribers.clear();
  }
}
