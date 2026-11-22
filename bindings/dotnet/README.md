<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Danaco.Nurt

Danaco Nurt is a command shell and language where every change can be undone.
This package is the .NET client; it works through the `libnurt` library or the
`nurt` executable.

## Installation

```
dotnet add package Danaco.Nurt
```

The package targets .NET 8 and .NET Standard 2.0 and contains `libnurt` and `nurt`
for Linux (x64, Arm64, x64 with musl), macOS (x64, Arm64) and Windows (x64, Arm64).
It has no dependencies on .NET 8; on .NET Standard 2.0 it depends on
`System.Text.Json`.

## Usage

```csharp
using Danaco.Nurt;

await using var nurt = new Client();
Response response = await nurt.RunAsync(
    "find \"src\" glob: \"*.cs\"",
    new RunOptions { Base = "/home/user/project", Format = OutputFormat.Text });
Console.WriteLine(response.Ok
    ? response.Text
    : response.Error!.Code + ": " + response.Error.Message);
```

An error of the language, for example `E_SYNTAX`, is part of the response:
`Response.Ok` is `false` and `Response.Error` holds the code, the position and the
ways to fix it. `RunOptions.ThrowOnLanguageError` raises `LanguageException`
instead. A failure of the call itself raises `CallException`, whose `Status` is a
status of `nurt.h` (`StatusName` gives its name, for example `NURT_E_ARG`).

A long program is started with `StartAsync` and waited for, cancelled or followed
through its events:

```csharp
await using Execution execution = await nurt.StartAsync("test \"/home/user/project\"");
await foreach (ExecutionEvent item in execution.ReadEventsAsync())
{
    Console.WriteLine(item.Kind);
}

Response result = await execution.WaitAsync(TimeSpan.FromMinutes(5));
```

A `CancellationToken` passed to `RunAsync` cancels the execution in Danaco Nurt.
A `Client` may be used from several threads at once.

## Backends

| `ClientOptions.Backend` | Mechanism |
|---|---|
| `Auto` (default) | `Library` when `libnurt` is found and implements interface version 1.x; otherwise `Process`. |
| `Library` | `libnurt` loaded into the process. |
| `Process` | Child process `nurt serve --stdio`; it is started again by the next call after it exits. |

The library is searched for in `ClientOptions.LibraryPath`, the variable
`NURT_LIBRARY` and the file of the package for the current platform; the
executable in `ClientOptions.ExecutablePath`, the variable `NURT_EXECUTABLE`, the
file of the package and the directories of `PATH`. When nothing is found,
`CallException` with the status `Io` lists the checked locations.

## Building and testing

```
dotnet build Binding.sln -c Release
dotnet test Binding.sln -c Release
```

The tests of the process backend use a scripted server built with the tests.
Further tests run when the following variables are set:

| Variable | Value |
|---|---|
| `NURT_TEST_LIBRARY` | Path of a build of `libnurt`; enables the tests of the library backend and the binding cases on it. |
| `NURT_TEST_SERVE` | Path of an executable that implements `nurt serve --stdio`; enables the binding cases of `conformance/bindings` on the process backend. |
| `NURT_TEST_SERVE_ARGS` | Arguments of that executable, separated by spaces; default `serve --stdio`. |

## Documentation

https://nurt.danaco-group.pl

## License

Apache License 2.0. See the files `LICENSE` and `NOTICE`.
