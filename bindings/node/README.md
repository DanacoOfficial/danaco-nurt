<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# nurt

[Danaco Nurt](https://nurt.danaco-group.pl) is a command shell and language
where every change can be undone. This package is the Node.js client and
command-line launcher.

The package has two backends. The `library` backend loads the Node-API addon
`nurt.node` into the process and calls the engine directly. The `process`
backend runs `nurt serve --stdio` as a child process and speaks the `nurt-rpc/1`
protocol with it. By default (`auto`) the library is used when an addon is
found and loads, and the process otherwise. The package has no run-time
dependencies and works in Node.js 18 or later, Bun and Deno (through `npm:`
specifiers; the library backend needs a runtime that supports Node-API). The
`nurt` executable and the addon come from a platform package installed as an
optional dependency.

## Installation

```sh
npm install nurt
```

Supported platforms: Linux x64 and arm64 (glibc and musl), macOS x64 and
arm64, Windows x64 and arm64. On other platforms, set `NURT_EXECUTABLE` to the
path of a `nurt` executable, or `NURT_LIBRARY` to the path of an addon.

## Usage

```js
import { Nurt } from "nurt";

const nurt = new Nurt();
const response = await nurt.run('ports state: listen |> filter port == 8080');
if (response.ok) {
  console.log(response.text);
} else {
  console.error(response.error.code, response.error.fix);
}
await nurt.close();
```

The same API is available through `require("nurt")`. `npx nurt` runs the
executable with the given arguments.

| Member | Result |
|---|---|
| `run(program, options)` | `Promise<Response>`: report (`report`), rendered text (`text`), `ok`, `error`, `nextCursor` |
| `start(program, options)` | `Execution`: `for await` over events, `on("event")`, `wait(timeoutMs)`, `cancel()` |
| `check`, `parse`, `print`, `correct`, `page` | binding, syntax tree, program text, corrections, next page |
| `describe`, `help`, `catalog`, `schema`, `history`, `space`, `info` | descriptions and state of the server |
| `close()` | ends the server |

`program` is program text or one of `{ program, variant }`, `{ ast }`,
`{ cursor }`, `{ accept }` (`nurt-request/1`). `options` holds the request
options (`preview`, `base`, `limit`, `format`, `timeout`, ...) and the client
options `signal` (an `AbortSignal` that cancels the call) and `raiseOnError`.
`Execution.cancel()` (or the `signal` of `start`) cancels the execution and
`wait()` returns its response, whose `error.code` is `E_CANCELLED`; a call of
`run` cancelled through its `signal` is rejected with `NURT_E_CANCELLED`.

Errors of the language are returned in `response.error`; with
`raiseOnError: true` they are thrown as `NurtLanguageError`. `check`, `parse`
and `correct` return a failed report (`report.error`) in the same way on both
backends; `print`, `describe`, `help`, `catalog`, `schema`, `history` and
`space` have no response to return it in, so they throw `NurtLanguageError`
(`NurtError` with `NURT_E_ARG` for an unknown argument value). Call failures are
thrown as `NurtError` with `status` set to a name from `nurt.h`
(`NURT_E_TIMEOUT`, `NURT_E_CANCELLED`, `NURT_E_ARG`, `NURT_E_ABI`, `NURT_E_IO`,
`NURT_E_INTERNAL`).

### Constructor options

| Option | Type | Default |
|---|---|---|
| `backend` | `"auto" \| "library" \| "process"` | `"auto"` |
| `library` | `string` | `NURT_LIBRARY`, then the platform package (`bin/nurt.node`) |
| `executable` | `string` | `NURT_EXECUTABLE`, then the platform package, then `PATH` |
| `args` | `string[]` | `["serve", "--stdio"]` |
| `config` | `Config` (`nurt-config/1`); not supported by the `library` backend | none |
| `session` | `string` | none |
| `language` | `"en" \| "pl"` | `"en"` |
| `env` | `Record<string, string>` | added to the environment of the server |
| `cwd` | `string` | current directory |

With the `library` backend `cwd` and `args` do not apply, `env` supplies only
`NURT_LIBRARY`, and the calls run on the worker threads of the addon. `close()`
cancels the calls in progress.

When the server process exits unexpectedly, the calls in progress fail with
`NURT_E_INTERNAL`, the client emits `exit`, and the next call starts a new
server.

## Documentation

<https://nurt.danaco-group.pl>

## License

Apache License 2.0. See `LICENSE` and `NOTICE`.
