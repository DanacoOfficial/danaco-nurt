// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

// Package rpc implements the client side of the JSON-RPC 2.0 transport used
// by nurt serve: message framing, request identifiers, response matching and
// notification dispatch.
package rpc

import (
	"bufio"
	"bytes"
	"errors"
	"fmt"
	"io"
	"strconv"
	"strings"
	"sync"
)

// Framing selects how messages are delimited on a byte stream.
type Framing int

const (
	// NDJSON frames one message per line terminated by '\n'.
	NDJSON Framing = iota
	// ContentLength frames each message with a Content-Length header block
	// followed by an empty line, as in the Language Server Protocol.
	ContentLength
)

// The header block of a Content-Length frame is bounded by maxHeaderBytes.
const maxHeaderBytes = 4096

// ErrMessageTooLarge is returned when a frame exceeds the size limit.
var ErrMessageTooLarge = errors.New("message exceeds the size limit")

// Reader reads framed messages from a byte stream. It is not safe for
// concurrent use.
type Reader struct {
	br      *bufio.Reader
	framing Framing
	max     int
}

// NewReader returns a Reader that accepts messages of at most max bytes.
func NewReader(r io.Reader, f Framing, max int) *Reader {
	return &Reader{br: bufio.NewReader(r), framing: f, max: max}
}

// Read returns the next message. It returns io.EOF when the stream ends at a
// frame boundary and io.ErrUnexpectedEOF when it ends inside a frame.
func (r *Reader) Read() ([]byte, error) {
	if r.framing == ContentLength {
		return r.readContentLength()
	}
	return r.readLine()
}

func (r *Reader) readLine() ([]byte, error) {
	for {
		var line []byte
		for {
			chunk, err := r.br.ReadSlice('\n')
			if len(line)+len(chunk) > r.max+len("\r\n") {
				return nil, ErrMessageTooLarge
			}
			line = append(line, chunk...)
			if err == nil {
				break
			}
			if errors.Is(err, bufio.ErrBufferFull) {
				continue
			}
			if errors.Is(err, io.EOF) && len(line) > 0 {
				return nil, io.ErrUnexpectedEOF
			}
			return nil, err
		}
		line = bytes.TrimSuffix(line, []byte("\n"))
		line = bytes.TrimSuffix(line, []byte("\r"))
		if len(line) > 0 {
			return line, nil
		}
	}
}

func (r *Reader) readContentLength() ([]byte, error) {
	length := -1
	headerBytes := 0
	for {
		line, err := r.br.ReadString('\n')
		headerBytes += len(line)
		if headerBytes > maxHeaderBytes {
			return nil, fmt.Errorf("header block exceeds %d bytes", maxHeaderBytes)
		}
		if err != nil {
			if errors.Is(err, io.EOF) && headerBytes > 0 {
				return nil, io.ErrUnexpectedEOF
			}
			return nil, err
		}
		line = strings.TrimRight(line, "\r\n")
		if line == "" {
			if length < 0 {
				if headerBytes <= len("\r\n") {
					headerBytes = 0
					continue
				}
				return nil, errors.New("frame without a Content-Length header")
			}
			break
		}
		name, value, ok := strings.Cut(line, ":")
		if !ok {
			return nil, fmt.Errorf("malformed header line %q", line)
		}
		if strings.EqualFold(strings.TrimSpace(name), "Content-Length") {
			n, err := strconv.Atoi(strings.TrimSpace(value))
			if err != nil || n < 0 {
				return nil, fmt.Errorf("invalid Content-Length %q", value)
			}
			length = n
		}
	}
	if length > r.max {
		return nil, ErrMessageTooLarge
	}
	msg := make([]byte, length)
	if _, err := io.ReadFull(r.br, msg); err != nil {
		if errors.Is(err, io.EOF) {
			return nil, io.ErrUnexpectedEOF
		}
		return nil, err
	}
	return msg, nil
}

// Writer writes framed messages to a byte stream. It is safe for concurrent
// use; each message is written with a single call to the underlying writer.
type Writer struct {
	mu      sync.Mutex
	w       io.Writer
	framing Framing
}

// NewWriter returns a Writer for the given framing.
func NewWriter(w io.Writer, f Framing) *Writer {
	return &Writer{w: w, framing: f}
}

// Write frames and writes one message. With NDJSON framing a message that
// contains a line feed is rejected.
func (w *Writer) Write(msg []byte) error {
	var frame []byte
	switch w.framing {
	case ContentLength:
		frame = make([]byte, 0, len(msg)+32)
		frame = append(frame, "Content-Length: "...)
		frame = strconv.AppendInt(frame, int64(len(msg)), 10)
		frame = append(frame, "\r\n\r\n"...)
		frame = append(frame, msg...)
	default:
		if bytes.IndexByte(msg, '\n') >= 0 {
			return errors.New("message contains a line feed")
		}
		frame = make([]byte, 0, len(msg)+1)
		frame = append(frame, msg...)
		frame = append(frame, '\n')
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	_, err := w.w.Write(frame)
	return err
}
