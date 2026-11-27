// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/src/framing.ts
// Responsibility:   Source file bindings/node/src/framing.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import { isObject, type JsonObject } from "./json.js";

const LINE_FEED = 0x0a;
const CARRIAGE_RETURN = 0x0d;

/** Violation of the framing or of the message format of `nurt-rpc/1`. */
export class ProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProtocolError";
  }
}

/**
 * Encodes a message as one line of newline-delimited JSON.
 *
 * `JSON.stringify` escapes line breaks inside strings, so the encoded message
 * never contains a line feed before its terminator.
 */
export function encodeMessage(message: unknown): string {
  return `${JSON.stringify(message)}\n`;
}

/**
 * Splits a byte stream of newline-delimited JSON into messages.
 *
 * Each message is one line of UTF-8 text holding one JSON object; a carriage
 * return before the line feed is accepted and empty lines are skipped. A
 * message that spans several lines fails on its first line. Bytes are split
 * at line feeds before decoding, which is safe because a line feed byte never
 * occurs inside a multi-byte UTF-8 sequence.
 */
export class MessageDecoder {
  readonly #maxBytes: number;
  readonly #utf8 = new TextDecoder("utf-8", { fatal: true });
  #parts: Uint8Array[] = [];
  #length = 0;

  /** Creates a decoder that rejects messages longer than `maxBytes` bytes. */
  constructor(maxBytes: number) {
    this.#maxBytes = maxBytes;
  }

  /**
   * Appends bytes and passes every message they complete to `onMessage`, in
   * order.
   *
   * @throws {ProtocolError} when a line is not one JSON object in UTF-8 or a
   *   message exceeds the size limit. Messages before the failing line have
   *   been passed to `onMessage`.
   */
  push(chunk: Uint8Array, onMessage: (message: JsonObject) => void): void {
    let start = 0;
    for (;;) {
      const end = chunk.indexOf(LINE_FEED, start);
      if (end === -1) {
        break;
      }
      this.#append(chunk.subarray(start, end));
      const message = this.#take();
      start = end + 1;
      if (message !== undefined) {
        onMessage(message);
      }
    }
    this.#append(chunk.subarray(start));
  }

  /**
   * Checks the end of the stream.
   *
   * @throws {ProtocolError} when the stream ends inside a message.
   */
  end(): void {
    if (this.#length > 0) {
      this.#parts = [];
      this.#length = 0;
      throw new ProtocolError("the stream ended inside a message");
    }
  }

  #append(bytes: Uint8Array): void {
    if (bytes.length === 0) {
      return;
    }
    if (this.#length + bytes.length > this.#maxBytes) {
      this.#parts = [];
      this.#length = 0;
      throw new ProtocolError(`a message exceeds the limit of ${this.#maxBytes} bytes`);
    }
    this.#parts.push(bytes.slice());
    this.#length += bytes.length;
  }

  #take(): JsonObject | undefined {
    const line = new Uint8Array(this.#length);
    let offset = 0;
    for (const part of this.#parts) {
      line.set(part, offset);
      offset += part.length;
    }
    this.#parts = [];
    this.#length = 0;
    const length = line.length > 0 && line[line.length - 1] === CARRIAGE_RETURN ? line.length - 1 : line.length;
    let text: string;
    try {
      text = this.#utf8.decode(line.subarray(0, length));
    } catch {
      throw new ProtocolError("a message is not valid UTF-8");
    }
    if (text.trim() === "") {
      return undefined;
    }
    let value: unknown;
    try {
      value = JSON.parse(text);
    } catch {
      throw new ProtocolError(`a line is not a complete JSON message: ${preview(text)}`);
    }
    if (!isObject(value)) {
      throw new ProtocolError(`a message is not a JSON object: ${preview(text)}`);
    }
    return value;
  }
}

/** Number of characters of an invalid line quoted in an error message. */
const PREVIEW_LENGTH = 80;

function preview(text: string): string {
  return text.length > PREVIEW_LENGTH ? `${text.slice(0, PREVIEW_LENGTH)}...` : text;
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
