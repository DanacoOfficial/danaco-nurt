<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           bindings/go/README.md -->
<!-- Responsibility:   Documents Danaco Nurt for Go. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Danaco Nurt for Go

[Danaco Nurt](https://github.com/DanacoOfficial/danaco-nurt) is a command shell and
language where every change can be undone. This module is the Go binding.

## Installation

```sh
go get github.com/DanacoOfficial/danaco-nurt/bindings/go
```

The module requires Go 1.23 or later and has no dependencies outside the
standard library. It runs the `nurt` executable, which is located through
`ClientOptions.Executable`, the `NURT_EXECUTABLE` environment variable or
`PATH`.

## Usage

```go
c, err := nurt.New(ctx, nurt.ClientOptions{})
if err != nil {
	return err
}
defer c.Close()

resp, err := c.Run(ctx, nurt.Request{Program: `find "/home/user/project" glob: "**/*.go" |> first 20`})
if err != nil {
	return err
}
fmt.Println(resp.Text())
```

A language error, such as a syntax error, is part of the response:
`resp.OK()` reports false and `resp.Err()` returns a `*nurt.LanguageError`.
Failures of the call itself are returned as `*nurt.Error`.

Both backends report the language error of `Check` and `Parse` in the `Error`
member of the result, and `Correct`, `Catalog`, `History` and `Space` return the
failure response `nurt-response/1` of the server (status `failed`, the error in
`report.error`) as their raw result.

## Backends

| Backend | Mechanism | Build |
|---|---|---|
| `process` (default) | `nurt serve --stdio`, JSON-RPC 2.0 | pure Go, any target |
| `library` | `libnurt` through cgo | `-tags nurt_cgo`, `libnurt` registered with `pkg-config` |

## Testing

`go test ./...` runs the unit tests of the module. The binding cases of the
repository (`conformance/bindings`, test `TestConformanceBindings`) run
against the implementation named by `NURT_TEST_COMMAND`, a JSON array with the
command that starts a server of the form `nurt serve --stdio`, for example
`["/path/to/nurt-conformance","serve","--stdio"]`, and against `libnurt` when
the module is built with `-tags nurt_cgo`. The test sets
`NURT_CONFORMANCE_CASES` and `NURT_CONFORMANCE_CASE` for each case; without
`NURT_TEST_COMMAND` and without the tag the cases are skipped.
`NURT_CONFORMANCE_BINDINGS` overrides the directory of the cases.

## Documentation

API reference: [pkg.go.dev](https://pkg.go.dev/github.com/DanacoOfficial/danaco-nurt/bindings/go).
Language and command reference: <https://nurt.danaco-group.pl>.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
