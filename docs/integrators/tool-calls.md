<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Tool calls

## Arguments and request

A tool call is turned into a request by placing `program`, `ast`, `cursor` and `accept` at the top level and every other argument in `options`:

<!-- schema: arguments -->
```json
{"program": "find \"/home/user/app\" glob: \"**/*.go\" |> count", "limit": 20}
```

<!-- schema: request -->
```json
{"nurt": "1.0", "program": "find \"/home/user/app\" glob: \"**/*.go\" |> count", "options": {"limit": 20}}
```

An unknown argument is a call error: the host returns it to the model as a tool error naming the argument. A request with two inputs, a relative `base` or a `limit` out of range is answered with the language error `E_ARG` in the report, with a fix.

## Result

The host returns `rendered.text` of the response to the model; for `format: "json"`, where `rendered` is absent, it returns the report as JSON. A report with `error` (`report.error`) is returned as a tool error (`is_error: true` for Anthropic, an error response for MCP), so that the model reads `fix` and corrects the call. Language errors never raise exceptions in the bindings; `raise_on_error` changes this per call.

## Corrections and `accept`

The console corrects common mistakes of notation before binding. A correction that touches a target of a change is not applied; the report carries `E_AMBIGUOUS` with ranked candidates:

<!-- schema: response -->
```json
{
  "nurt": "1.0",
  "report": {
    "nurt": "1.0",
    "status": "failed",
    "kind": "remove",
    "error": {
      "code": "E_AMBIGUOUS",
      "message": "the target of the change is ambiguous",
      "candidates": [
        {"rank": 1, "ref": "c1", "program": "remove \"/home/user/app/build\"", "why": "the directory is named by the program"},
        {"rank": 2, "ref": "c2", "program": "remove \"/home/user/app/build/*\"", "why": "the contents of the directory only"}
      ]
    }
  }
}
```

The model, or the host on behalf of the user, accepts one candidate with a call that has only `accept`:

<!-- schema: arguments -->
```json
{"accept": "c1"}
```

The candidate runs exactly as listed. A candidate that expired or is unknown gives `E_EXPIRED_CANDIDATE`; the program is sent again. A host that asks the user for confirmation shows `program` and `why` of each candidate.

## Pages

A result longer than `limit` records ends with `report.next_cursor`. The next page is read with a call that has only `cursor` (and optionally `limit`):

<!-- schema: arguments -->
```json
{"cursor": "p-7f3a-2", "limit": 100}
```

Reading a page never runs a changing program again. A host that collects all pages calls `page` (`nurt serve`) or repeats the call until `next_cursor` is absent. `max_chars` limits the length of single text fields; `save_to` writes the full result to a file instead of the response.
