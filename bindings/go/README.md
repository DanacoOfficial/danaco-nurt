<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Danaco Nurt for Go

[Danaco Nurt](https://github.com/DanacoOfficial/nurt) is a command shell and
language where every change can be undone. This module is the Go binding.

## Installation

```sh
go get github.com/DanacoOfficial/nurt/bindings/go
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

## Backends

| Backend | Mechanism | Build |
|---|---|---|
| `process` (default) | `nurt serve --stdio`, JSON-RPC 2.0 | pure Go, any target |
| `library` | `libnurt` through cgo | `-tags nurt_cgo`, `libnurt` registered with `pkg-config` |

## Documentation

API reference: [pkg.go.dev](https://pkg.go.dev/github.com/DanacoOfficial/nurt/bindings/go).
Language and command reference: <https://nurt.danaco-group.pl>.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
