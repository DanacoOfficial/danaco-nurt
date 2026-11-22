// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package rpc

import (
	"bytes"
	"errors"
	"io"
	"strings"
	"testing"
)

const testMax = 64

func readAll(t *testing.T, r *Reader) ([]string, error) {
	t.Helper()
	var got []string
	for {
		msg, err := r.Read()
		if err != nil {
			return got, err
		}
		got = append(got, string(msg))
	}
}

func TestNDJSONReadsOneMessagePerLine(t *testing.T) {
	r := NewReader(strings.NewReader("{\"a\":1}\n{\"b\":2}\r\n\n{\"c\":3}\n"), NDJSON, testMax)
	got, err := readAll(t, r)
	if !errors.Is(err, io.EOF) {
		t.Fatalf("final error = %v, want io.EOF", err)
	}
	want := []string{`{"a":1}`, `{"b":2}`, `{"c":3}`}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Fatalf("messages = %q, want %q", got, want)
	}
}

func TestNDJSONReportsTruncatedLine(t *testing.T) {
	r := NewReader(strings.NewReader("{\"a\":1}\n{\"b\""), NDJSON, testMax)
	_, err := readAll(t, r)
	if !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatalf("error = %v, want io.ErrUnexpectedEOF", err)
	}
}

func TestNDJSONRejectsOversizedLine(t *testing.T) {
	line := strings.Repeat("x", testMax+1) + "\n"
	r := NewReader(strings.NewReader(line), NDJSON, testMax)
	if _, err := r.Read(); !errors.Is(err, ErrMessageTooLarge) {
		t.Fatalf("error = %v, want ErrMessageTooLarge", err)
	}
}

func TestNDJSONAcceptsLineLongerThanReadBuffer(t *testing.T) {
	const size = 10000
	body := strings.Repeat("y", size)
	r := NewReader(strings.NewReader(body+"\n"), NDJSON, size)
	msg, err := r.Read()
	if err != nil {
		t.Fatal(err)
	}
	if len(msg) != size {
		t.Fatalf("length = %d, want %d", len(msg), size)
	}
}

func TestNDJSONWriterRejectsLineFeed(t *testing.T) {
	var buf bytes.Buffer
	w := NewWriter(&buf, NDJSON)
	if err := w.Write([]byte("{\"a\":\n1}")); err == nil {
		t.Fatal("message with a line feed was accepted")
	}
	if err := w.Write([]byte(`{"a":1}`)); err != nil {
		t.Fatal(err)
	}
	if buf.String() != "{\"a\":1}\n" {
		t.Fatalf("frame = %q", buf.String())
	}
}

func TestContentLengthRoundTrip(t *testing.T) {
	var buf bytes.Buffer
	w := NewWriter(&buf, ContentLength)
	msgs := []string{`{"a":1}`, "{\"text\":\"line\\nnext\"}", `{}`}
	for _, m := range msgs {
		if err := w.Write([]byte(m)); err != nil {
			t.Fatal(err)
		}
	}
	if !strings.HasPrefix(buf.String(), "Content-Length: 7\r\n\r\n{\"a\":1}") {
		t.Fatalf("frame = %q", buf.String())
	}
	got, err := readAll(t, NewReader(&buf, ContentLength, testMax))
	if !errors.Is(err, io.EOF) {
		t.Fatalf("final error = %v, want io.EOF", err)
	}
	if strings.Join(got, "|") != strings.Join(msgs, "|") {
		t.Fatalf("messages = %q, want %q", got, msgs)
	}
}

func TestContentLengthIgnoresOtherHeadersAndCase(t *testing.T) {
	in := "content-type: application/json\r\ncontent-length: 2\r\n\r\n{}"
	msg, err := NewReader(strings.NewReader(in), ContentLength, testMax).Read()
	if err != nil {
		t.Fatal(err)
	}
	if string(msg) != "{}" {
		t.Fatalf("message = %q", msg)
	}
}

func TestContentLengthRejectsMalformedFrames(t *testing.T) {
	cases := map[string]string{
		"missing length":   "Content-Type: x\r\n\r\n{}",
		"negative length":  "Content-Length: -1\r\n\r\n",
		"header without :": "Content-Length 2\r\n\r\n{}",
		"oversized body":   "Content-Length: 65\r\n\r\n",
	}
	for name, in := range cases {
		t.Run(name, func(t *testing.T) {
			if _, err := NewReader(strings.NewReader(in), ContentLength, testMax).Read(); err == nil {
				t.Fatal("frame was accepted")
			}
		})
	}
}

func TestContentLengthReportsTruncatedBody(t *testing.T) {
	in := "Content-Length: 10\r\n\r\n{}"
	_, err := NewReader(strings.NewReader(in), ContentLength, testMax).Read()
	if !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatalf("error = %v, want io.ErrUnexpectedEOF", err)
	}
}

func TestContentLengthRejectsUnboundedHeaders(t *testing.T) {
	in := strings.Repeat("X-Padding: "+strings.Repeat("p", 100)+"\r\n", 100)
	if _, err := NewReader(strings.NewReader(in), ContentLength, testMax).Read(); err == nil {
		t.Fatal("unbounded header block was accepted")
	}
}
