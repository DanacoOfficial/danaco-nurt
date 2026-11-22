Protocol transcripts of nurt serve in the stub mode.

methods.requests.ndjson  requests sent in order, one per line; the message
                         after shutdown is never read.
methods.replies.ndjson   expected replies, compared by id. An expected
                         object matches when each of its members matches;
                         the string "*" matches any value.

The answers come from the stub cases of crates/nurt-ffi/tests/fixtures/stub.
