// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/src/channel.ts
// Responsibility:   Source file bindings/node/src/channel.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import type { CallHooks, ConnectionLimits } from "./connection.js";
import type { NurtError } from "./errors.js";

/** Receivers of what a channel reports on its own. */
export interface ChannelEvents {
  /** Receives every notification of the server. */
  readonly onNotification: (method: string, params: unknown) => void;
  /** Called once when the channel ends, with the error given to the calls in progress. */
  readonly onClose: (error: NurtError) => void;
}

/**
 * Path of the calls between the client and an engine: the child process
 * (`Connection`) or the addon loaded into this process (`NativeChannel`).
 * Both speak the methods of `nurt-rpc/1`.
 */
export interface Channel {
  /** Whether the channel accepts calls. */
  readonly usable: boolean;
  /** Applies the limits announced by the server. */
  setLimits(limits: ConnectionLimits): void;
  /**
   * Calls a method and resolves with its result.
   *
   * @throws {NurtError} the failure of the call.
   */
  call(method: string, params: unknown, hooks?: CallHooks): Promise<unknown>;
  /** Closes the channel, waiting at most `graceMs` milliseconds for the engine to finish. */
  close(graceMs: number): Promise<void>;
  /** Ends the channel with an error and releases the engine. */
  terminate(error: NurtError): void;
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
