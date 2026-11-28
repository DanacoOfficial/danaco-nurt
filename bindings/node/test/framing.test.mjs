// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/framing.test.mjs
// Responsibility:   Source file bindings/node/test/framing.test.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { internal } from "./support.mjs";

const { encodeMessage, MessageDecoder, ProtocolError } = internal("framing");

const LIMIT = 1024;
const encoder = new TextEncoder();

function decodeAll(chunks, limit = LIMIT) {
  const decoder = new MessageDecoder(limit);
  const messages = [];
  for (const chunk of chunks) {
    decoder.push(typeof chunk === "string" ? encoder.encode(chunk) : chunk, (message) => {
      messages.push(message);
    });
  }
  decoder.end();
  return messages;
}

describe("encodeMessage", () => {
  it("writes one line even when strings contain line breaks", () => {
    const line = encodeMessage({ jsonrpc: "2.0", method: "run", params: { program: "a\nb\r\nc" } });
    assert.equal(line.indexOf("\n"), line.length - 1);
    assert.deepEqual(JSON.parse(line), { jsonrpc: "2.0", method: "run", params: { program: "a\nb\r\nc" } });
  });
});

describe("MessageDecoder", () => {
  it("decodes a message delivered one byte at a time", () => {
    const bytes = encoder.encode('{"jsonrpc":"2.0","id":1,"result":"ok"}\n');
    const chunks = [...bytes].map((byte) => Uint8Array.of(byte));
    assert.deepEqual(decodeAll(chunks), [{ jsonrpc: "2.0", id: 1, result: "ok" }]);
  });

  it("decodes several messages in one chunk", () => {
    const messages = decodeAll(['{"id":1}\n{"id":2}\n{"id":3}\n']);
    assert.deepEqual(messages, [{ id: 1 }, { id: 2 }, { id: 3 }]);
  });

  it("joins a multi-byte character split between chunks", () => {
    const value = "\u00e9\u4e2d\u{1f600}";
    const bytes = encoder.encode(`${JSON.stringify({ text: value })}\n`);
    // First byte of the three-byte UTF-8 sequence of U+4E2D.
    const lead = 0xe4;
    const middle = bytes.indexOf(lead) + 1;
    const messages = decodeAll([bytes.subarray(0, middle), bytes.subarray(middle)]);
    assert.deepEqual(messages, [{ text: value }]);
  });

  it("accepts a carriage return before the line feed and skips empty lines", () => {
    assert.deepEqual(decodeAll(['{"id":1}\r\n\n  \n{"id":2}\n']), [{ id: 1 }, { id: 2 }]);
  });

  it("rejects a message written over several lines", () => {
    const pretty = `${JSON.stringify({ jsonrpc: "2.0", id: 1, result: null }, null, 2)}\n`;
    assert.throws(() => decodeAll([pretty]), ProtocolError);
  });

  it("delivers the messages before an invalid line", () => {
    const decoder = new MessageDecoder(LIMIT);
    const messages = [];
    assert.throws(
      () => {
        decoder.push(encoder.encode('{"id":1}\nnot json\n{"id":2}\n'), (message) => {
          messages.push(message);
        });
      },
      { name: "ProtocolError" },
    );
    assert.deepEqual(messages, [{ id: 1 }]);
  });

  it("rejects bytes that are not UTF-8", () => {
    assert.throws(() => decodeAll([Uint8Array.of(0xff, 0xfe, 0x0a)]), /not valid UTF-8/);
  });

  it("rejects a value that is not an object", () => {
    assert.throws(() => decodeAll(["42\n"]), /not a JSON object/);
    assert.throws(() => decodeAll(["[1,2]\n"]), /not a JSON object/);
  });

  it("rejects a message longer than the limit before its end arrives", () => {
    const decoder = new MessageDecoder(16);
    assert.throws(() => {
      decoder.push(encoder.encode('{"text":"0123456789'), () => undefined);
    }, /exceeds the limit of 16 bytes/);
  });

  it("rejects a stream that ends inside a message", () => {
    const decoder = new MessageDecoder(LIMIT);
    decoder.push(encoder.encode('{"id":1}\n{"id"'), () => undefined);
    assert.throws(() => {
      decoder.end();
    }, /ended inside a message/);
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
