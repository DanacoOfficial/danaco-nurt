<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Danaco Nurt for Java

Danaco Nurt is a command shell and language where every change can be undone. This library is the Java interface.

The artifact `pl.danaco-group:nurt` runs `nurt serve --stdio` as a child process and needs Java 17 or later. The artifact `pl.danaco-group:nurt-ffm` adds a backend that calls `libnurt` in the process through the Foreign Function and Memory API and needs Java 22 or later. Neither artifact has runtime dependencies.

## Installation

```xml
<dependency>
  <groupId>pl.danaco-group</groupId>
  <artifactId>nurt</artifactId>
  <version>0.1.0</version>
</dependency>
```

Add `pl.danaco-group:nurt-ffm` with the same version to use the library backend. The packages contain the `nurt` executable and `libnurt` for Linux, macOS and Windows on x86_64 and aarch64; each file is checked against its SHA-256 digest before it is extracted to a temporary directory.

## Usage

```java
import pl.danacogroup.nurt.Client;
import pl.danacogroup.nurt.Request;
import pl.danacogroup.nurt.Response;

try (Client nurt = Client.create()) {
    Response response = nurt.run(Request.program("find \"/home/user/src\" glob: \"*.go\""));
    response.error().ifPresentOrElse(
            error -> System.err.println(error + " fix: " + error.fix()),
            () -> System.out.println(response.text().orElse("")));
}
```

A language error is part of a successful `Response`; `Response.orThrow()` turns it into a `LanguageException`. A failure of the call itself is a `CallException` whose `status()` is the status code of the C interface.

`Client.start(Request)` returns an `Execution` with a `Flow.Publisher<Event>` of its events, a `CompletableFuture<Response>` and cancellation. `Client` is thread-safe and `AutoCloseable`.

## Backends

| `BackendKind` | Mechanism | Lookup |
|---|---|---|
| `AUTO` (default) | `LIBRARY` when `nurt-ffm` is present and the library is compatible, otherwise `PROCESS` | |
| `PROCESS` | `nurt serve --stdio`, protocol `nurt-rpc/1` | `ClientOptions.executable`, `NURT_EXECUTABLE`, bundled executable, `PATH` |
| `LIBRARY` | `libnurt`, interface version 1.0 or a later minor version | `ClientOptions.library`, `NURT_LIBRARY`, bundled library |

On the module path, the library backend needs `--enable-native-access=pl.danacogroup.nurt.ffm`. On the class path, the manifest of `nurt-ffm` enables native access for the unnamed module of executable JAR files; other launchers pass `--enable-native-access=ALL-UNNAMED`.

## Building

```sh
mvn -B verify
```

With Java 22 or later the build includes `nurt-ffm`. Tests of the library backend run when `NURT_LIBRARY` names a library; binding conformance cases run when `NURT_CONFORMANCE` names the `nurt-conformance` executable.

## Documentation

https://nurt.danaco-group.pl

## License

Apache License 2.0. See `LICENSE` and `NOTICE` in the root of the repository.
