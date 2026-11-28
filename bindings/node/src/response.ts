// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/src/response.ts
// Responsibility:   Source file bindings/node/src/response.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import { NurtError } from "./errors.js";
import { isObject } from "./json.js";
import type { Report, ReportError, ResponseEnvelope } from "./types.js";

/** Response to a request: the effect report and, for text formats, the rendered text. */
export class Response {
  /** Response envelope `nurt-response/1` as received. */
  readonly raw: ResponseEnvelope;

  constructor(raw: ResponseEnvelope) {
    this.raw = raw;
  }

  /** Effect report `nurt-report/1`. */
  get report(): Report {
    return this.raw.report;
  }

  /**
   * Reports of the pipelines of a script that ran several operations, in
   * program order; empty for a program with a single operation. They are not
   * paged and carry no `next_cursor`.
   */
  get reports(): readonly Report[] {
    return this.raw.reports ?? [];
  }

  /** Rendered text, present for the formats `text`, `markdown` and `compact`. */
  get text(): string | undefined {
    return this.raw.rendered?.text;
  }

  /** Whether the report carries no language error. */
  get ok(): boolean {
    return this.raw.report.error === undefined;
  }

  /** Language error of the report, when present. */
  get error(): ReportError | undefined {
    return this.raw.report.error;
  }

  /** Cursor of the next page, absent on the last page. */
  get nextCursor(): string | undefined {
    return this.raw.report.next_cursor;
  }
}

/**
 * Converts a result received from the server into a {@link Response}.
 *
 * @throws {NurtError} `NURT_E_INTERNAL` when the result has no report.
 */
export function toResponse(value: unknown): Response {
  if (!isObject(value) || !isObject(value["report"])) {
    throw new NurtError("NURT_E_INTERNAL", "the server returned a result without a report");
  }
  return new Response(value as unknown as ResponseEnvelope);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
