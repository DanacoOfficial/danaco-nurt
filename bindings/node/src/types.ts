// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import type { Ast } from "./generated/nurt-ast-1.js";
import type { Config } from "./generated/nurt-config-1.js";
import type { Report, ReportError } from "./generated/nurt-report-1.js";
import type { Request, RequestOptions, RequestRendered } from "./generated/nurt-request-1.js";

export type { Ast, Config, Report, ReportError, Request, RequestOptions, RequestRendered };

/** Notation variant of program text. */
export type Variant = NonNullable<Request["variant"]>;

/** Language of messages and help. */
export type Language = NonNullable<RequestOptions["language"]>;

/**
 * Program of a request: program text, or exactly one of the request forms of
 * `nurt-request/1`.
 */
export type ProgramInput =
  | string
  | { readonly program: string; readonly variant?: Variant }
  | { readonly ast: Ast }
  | { readonly cursor: string }
  | { readonly accept: string };

/** Response envelope `nurt-response/1`. */
export interface ResponseEnvelope {
  /** Format version MAJOR.MINOR. */
  readonly nurt: string;
  /** Effect report `nurt-report/1`. */
  readonly report: Report;
  /** Rendered text, present for the text formats. */
  readonly rendered?: RequestRendered;
}

/** Event of an execution or of the server (`nurt/event`). */
export interface NurtEvent {
  /** Event type, for example `exec.finished` or `job.output`. */
  readonly t: string;
  /** Sequence number, increasing within an execution. */
  readonly seq: number;
  /** Time of the event (RFC 3339). */
  readonly time: string;
  /** Identifier of the request the event belongs to. */
  readonly request_id?: number | string;
  /** Identifier of the background job the event belongs to. */
  readonly job_id?: string;
  readonly [field: string]: unknown;
}

/** Options of a call that can be cancelled. */
export interface CallOptions {
  /** Cancels the call when aborted. */
  readonly signal?: AbortSignal | undefined;
}

/**
 * Options of {@link Nurt.run} and {@link Nurt.check}: the request options of
 * `nurt-request/1` and the options of the client.
 */
export interface RunOptions extends RequestOptions, CallOptions {
  /** Throws {@link NurtLanguageError} instead of returning a response with an error. */
  readonly raiseOnError?: boolean | undefined;
}

/** Options of {@link Nurt.start}. */
export interface StartOptions extends RunOptions {
  /** Receives every event of the execution. */
  readonly onEvent?: ((event: NurtEvent) => void) | undefined;
}

/** Options of {@link Nurt.describe}. */
export interface DescribeOptions {
  /** Description layer; default `core`. */
  readonly layer?: string | undefined;
  /** Command or group to describe; default the whole layer. */
  readonly name?: string | undefined;
  /** Language of the description; default `en`. */
  readonly language?: Language | undefined;
}

/** Options of the {@link Nurt} client. */
export interface NurtOptions {
  /** Backend; `auto` and `process` both run `nurt serve --stdio`. Default `auto`. */
  readonly backend?: "auto" | "process" | undefined;
  /**
   * Path or command name of the `nurt` executable. Default: the variable
   * `NURT_EXECUTABLE`, then the platform package, then `PATH`.
   */
  readonly executable?: string | undefined;
  /** Arguments of the executable. Default `["serve", "--stdio"]`. */
  readonly args?: readonly string[] | undefined;
  /** Configuration applied on top of the configuration files (`nurt-config/1`). */
  readonly config?: Config | undefined;
  /** Default session of the requests. */
  readonly session?: string | undefined;
  /** Language of messages and help. Default `en`. */
  readonly language?: Language | undefined;
  /** Environment variables added to the environment of the server. */
  readonly env?: { readonly [name: string]: string | undefined } | undefined;
  /** Working directory of the server. Default the current directory. */
  readonly cwd?: string | undefined;
}
