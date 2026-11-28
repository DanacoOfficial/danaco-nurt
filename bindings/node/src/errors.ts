// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/src/errors.ts
// Responsibility:   Source file bindings/node/src/errors.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import type { Response } from "./response.js";
import type { ReportError } from "./types.js";

/**
 * Name of a call failure, as defined by `nurt_status` in `nurt.h`.
 *
 * The process backend reports its own failures with the same names: a missing
 * executable is `NURT_E_IO`, an incompatible protocol is `NURT_E_ABI`, and a
 * crash of the server or an invalid message is `NURT_E_INTERNAL`.
 */
export type StatusName =
  | "NURT_E_TIMEOUT"
  | "NURT_E_CANCELLED"
  | "NURT_E_ARG"
  | "NURT_E_ABI"
  | "NURT_E_IO"
  | "NURT_E_INTERNAL";

/** Additional information of a {@link NurtError}. */
export interface NurtErrorOptions {
  /** Underlying error. */
  readonly cause?: unknown;
  /** JSON-RPC error code, when the server reported the failure. */
  readonly rpcCode?: number | undefined;
  /** `data` member of the JSON-RPC error, when present. */
  readonly detail?: unknown;
  /** Response of a cancelled execution, when the server returned one. */
  readonly response?: Response | undefined;
}

/**
 * Failure of a call: the server could not be started, the protocol failed, the
 * call was cancelled or timed out, or the server rejected the arguments.
 *
 * Errors of the Nurt language are not call failures; they are returned in
 * {@link Response.error}.
 */
export class NurtError extends Error {
  /** Status name from `nurt.h`. */
  readonly status: StatusName;
  /** JSON-RPC error code, when the server reported the failure. */
  readonly rpcCode: number | undefined;
  /** `data` member of the JSON-RPC error, when present. */
  readonly detail: unknown;
  /** Response of a cancelled execution, when the server returned one. */
  readonly response: Response | undefined;

  constructor(status: StatusName, message: string, options: NurtErrorOptions = {}) {
    super(message, options.cause === undefined ? undefined : { cause: options.cause });
    this.name = "NurtError";
    this.status = status;
    this.rpcCode = options.rpcCode;
    this.detail = options.detail;
    this.response = options.response;
  }
}

/**
 * Error of the Nurt language, thrown instead of returning the response when
 * a call is made with `raiseOnError: true`.
 */
export class NurtLanguageError extends Error {
  /** Error code `E_*`. */
  readonly code: string;
  /** Error object of the report. */
  readonly error: ReportError;
  /** Response that carried the error. */
  readonly response: Response;

  constructor(response: Response, error: ReportError) {
    super(`${error.code}: ${error.message}`);
    this.name = "NurtLanguageError";
    this.code = error.code;
    this.error = error;
    this.response = response;
  }

  /** Position of the error in the program text, when known. */
  get at(): ReportError["at"] {
    return this.error.at;
  }

  /** Suggested corrections, when known. */
  get fix(): ReportError["fix"] {
    return this.error.fix;
  }

  /** Help topic that describes the error, when known. */
  get help(): ReportError["help"] {
    return this.error.help;
  }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
