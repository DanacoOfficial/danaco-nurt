// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"context"
	"encoding/json"
	"iter"
	"sync"
)

// EventFinished is the type of the last event of an execution.
const EventFinished = "exec.finished"

// Event is an event of an execution nurt-event/1: progress, output of
// programs, journal changes or completion.
type Event struct {
	// Type is the event type, for example "job.output" or "exec.finished".
	Type string `json:"t"`
	// Seq orders the events of one execution.
	Seq uint64 `json:"seq"`
	// Time is the time of the event in RFC 3339 format.
	Time string `json:"time,omitempty"`
	// JobID identifies the background job the event belongs to, if any.
	JobID string `json:"job_id,omitempty"`

	// Raw is the complete event as JSON.
	Raw json.RawMessage `json:"-"`
}

func decodeEvent(raw []byte) (Event, bool) {
	var ev Event
	if err := json.Unmarshal(raw, &ev); err != nil {
		return Event{}, false
	}
	ev.Raw = append(json.RawMessage(nil), raw...)
	return ev, true
}

// The queue never blocks the producer, so a slow consumer does not hold up
// the execution or other calls on the same connection.
type eventQueue struct {
	mu     sync.Mutex
	items  []Event
	closed bool
	signal chan struct{}
}

func newEventQueue() *eventQueue {
	return &eventQueue{signal: make(chan struct{}, 1)}
}

func (q *eventQueue) push(ev Event) {
	q.mu.Lock()
	if q.closed {
		q.mu.Unlock()
		return
	}
	q.items = append(q.items, ev)
	if ev.Type == EventFinished {
		q.closed = true
	}
	q.mu.Unlock()
	q.wake()
}

func (q *eventQueue) close() {
	q.mu.Lock()
	q.closed = true
	q.mu.Unlock()
	q.wake()
}

func (q *eventQueue) wake() {
	select {
	case q.signal <- struct{}{}:
	default:
	}
}

func (q *eventQueue) pop() (Event, bool) {
	for {
		q.mu.Lock()
		if len(q.items) > 0 {
			ev := q.items[0]
			q.items[0] = Event{}
			q.items = q.items[1:]
			q.mu.Unlock()
			return ev, true
		}
		closed := q.closed
		q.mu.Unlock()
		if closed {
			return Event{}, false
		}
		<-q.signal
	}
}

// Each backend implements execHandle for one running request.
type execHandle interface {
	wait(ctx context.Context) (json.RawMessage, error)
	cancel(ctx context.Context) error
	close()
}

// Execution is a request started by Client.Start. Its methods are safe for
// concurrent use.
type Execution struct {
	handle    execHandle
	queue     *eventQueue
	failOnErr bool
	closeOnce sync.Once
}

// Events returns the events of the execution in the order of Seq. The
// sequence ends after the event of type EventFinished, or when the
// execution ends without it. Events are buffered without limit until they
// are read; the sequence can be iterated by one consumer.
func (x *Execution) Events() iter.Seq[Event] {
	return func(yield func(Event) bool) {
		for {
			ev, ok := x.queue.pop()
			if !ok || !yield(ev) {
				return
			}
		}
	}
}

// Wait waits for the response. When ctx ends first, Wait returns an *Error
// with StatusTimeout or StatusCancelled and the execution continues; Wait
// may be called again. After Cancel, Wait returns the response of the
// cancelled execution together with an *Error with StatusCancelled.
func (x *Execution) Wait(ctx context.Context) (*Response, error) {
	raw, err := x.handle.wait(ctx)
	if raw == nil {
		return nil, err
	}
	resp, decodeErr := decodeResponse(raw)
	if decodeErr != nil {
		return nil, decodeErr
	}
	if err == nil && x.failOnErr {
		err = resp.Err()
	}
	return resp, err
}

// Cancel requests cancellation of the execution. It is idempotent and
// succeeds also when the execution has already finished. Changes already
// committed stay in the journal and can be undone.
func (x *Execution) Cancel(ctx context.Context) error {
	return x.handle.cancel(ctx)
}

// Close releases the execution. A running execution is cancelled first.
// Close is idempotent.
func (x *Execution) Close() error {
	x.closeOnce.Do(func() {
		x.handle.close()
		x.queue.close()
	})
	return nil
}
