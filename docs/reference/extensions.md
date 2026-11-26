<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Extensions

Extensions add commands, document formats and grammars to Danaco Nurt without
changing the program. An extension is a directory with a manifest
`nurt-ext.toml` and a WebAssembly component, or, when the configuration allows
it, a native library.

## Extension directory

| Item | Value |
|---|---|
| Configuration key | `ext.dir` (string, path) |
| Environment variable | `NURT_EXT_DIR` |
| Default | `ext` in the configuration directory |
| Relative value | resolved against the configuration directory |
| Layout | one subdirectory per extension, named after the extension |

The key `ext.native` (boolean, default `false`) allows native extension
libraries. Native libraries are available only in builds with the `native`
feature.

## Installation, removal and listing

| Operation | Behaviour | Errors |
|---|---|---|
| install | Reads and checks the manifest of a source directory, checks that every file it names exists, copies the directory to `<ext.dir>/<name>` through a staging directory and switches it in by rename. An installed version is replaced only on request. | `E_CATALOG` (rejected manifest), `E_NOT_FOUND` (manifest or named file missing), `E_EXISTS` (installed, replacement not requested), `E_UNSUPPORTED` (symbolic link or special file in the source, native extension not allowed), `E_INVALID_VALUE` (the source directory holds `<ext.dir>`), `E_IO` |
| remove | Deletes `<ext.dir>/<name>`, usable or not. | `E_INVALID_VALUE` (malformed name), `E_NOT_FOUND`, `E_IO` |
| list | Lists every subdirectory of `ext.dir` in name order with name, directory, version, API version, kind and full command names. An unusable extension is listed with the member `problem`. | `E_IO` |

An interrupted installation leaves the previous version in place. Directories
whose names start with `.` are staging areas and are not listed; every
installation uses a staging directory of its own, so installations of one
extension do not overwrite each other's copies.

### Trust

Installation does not verify a signature or a checksum of the source: the
account that installs an extension trusts its source directory. A WebAssembly
extension runs in a sandbox and reaches files only through the host calls
below, within `permissions.read` and `permissions.write`; the grants are
checked when the extension asks for a path and again when its writes are
applied. A native extension runs in the process of the engine without such
restrictions, which is why `ext.native` is off by default.

## Manifest

```toml
[ext]
name = "acme"
version = "1.2.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "acme.wasm"
description = "Checks of acme files"

[[stage]]
name = "lint"
summary = "Reports problems of acme files."
input = ["entry"]
output = ["finding"]

[[stage.param]]
name = "strict"
type = "bool"
summary = "Reports style problems too."

[[format]]
id = "acme-conf"
extensions = ["acmeconf"]

[[grammar]]
language = "acme"
extensions = ["acme"]
wasm = "grammar/acme.wasm"

[permissions]
read = ["${project}"]
write = []
network = []
```

Unknown keys are rejected.

### Section `[ext]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | Extension name and command prefix: lowercase ASCII letters, digits and `_`, starting with a letter, at most 64 bytes. Must not be a group, command or command prefix of the core. |
| `version` | string | required | Version of the extension, Semantic Versioning 2.0.0. |
| `api` | string | required | Required interface version, `nurt:ext@MAJOR.MINOR`; a patch number is accepted and ignored. |
| `kind` | `"wasm"` \| `"native"` | required | Distribution form. |
| `wasm` | string | none | Component file; required for `kind = "wasm"`. |
| `library` | string | none | Library file; required for `kind = "native"`. |
| `description` | string | none | One-line description; the summary of the command group. |

File names are relative paths inside the extension directory; `..`, absolute
paths and root prefixes are rejected.

### Entries `[[stage]]`

Each entry declares the command `<ext.name>.<name>`.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | Command name without the prefix; same rules as `ext.name`. |
| `summary` | string | generated | One-line description. |
| `input` | array of strings | `[]` | Accepted record kinds; `any` accepts every kind. |
| `output` | array of strings | `[]` (any kind) | Produced record kinds. |
| `source` | boolean | `false` | The command can start a pipeline. A command with an empty `input` must be a source. |
| `writes` | boolean | `false` | The command changes files; requires a non-empty `permissions.write`. |
| `network` | boolean | `false` | The command contacts the network; requires a non-empty `permissions.network`. |
| `param` | array of tables | `[]` | Parameters, with the keys of a parameter of the command catalog. |

A command without `writes` is a read stage of category `transform` (or
`source` without input). A command with `writes` is a change stage with a
reversible local effect.

### Entries `[[format]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | Format identifier: lowercase ASCII letters, digits, `_` and `-`. |
| `extensions` | array of strings | `[]` | File name extensions without the leading period. |
| `editable` | boolean | `false` | Keys can be changed with the layout preserved. |

### Entries `[[grammar]]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `language` | string | required | Language identifier; same rules as `format.id`. |
| `extensions` | array of strings | `[]` | File name extensions without the leading period. |
| `wasm` | string | required | Grammar component file. |
| `tags` | string | none | Tag query file. |
| `locals` | string | none | Local scope query file. |

### Section `[permissions]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `read` | array of strings | `[]` | Directories the extension may read. |
| `write` | array of strings | `[]` | Directories the extension may change; also readable. |
| `network` | array of strings | `[]` | Hosts the extension may contact. |

A directory is absolute or starts with a placeholder:

| Placeholder | Value during an execution |
|---|---|
| `${project}` | Base directory of the request. |
| `${home}` | Home directory of the account. |
| `${temp}` | Temporary directory. |
| `${ext}` | Directory of the extension. |

An entry whose placeholder has no value grants nothing. A path is granted when
it lies below a granted directory both as written and with symbolic links
resolved. Paths with `..` components and relative paths are refused.

## Interface versions

The host implements `nurt:ext@1.0`. A manifest is accepted when its major
version equals the major version of the host and its minor version is not
newer. Any other version fails installation with `E_CATALOG` and the message
names both versions.

## WebAssembly interface

The component implements the world `extension` of the package
`nurt:ext@1.0.0`:

```wit
package nurt:ext@1.0.0;

interface host {
    record failure {
        code: string,
        message: string,
    }

    enum log-level {
        error,
        warn,
        info,
        debug,
        trace,
    }

    read-file: func(path: string) -> result<list<u8>, failure>;
    list-dir: func(path: string) -> result<list<string>, failure>;
    write-file: func(path: string, content: list<u8>) -> result<_, failure>;
    log: func(level: log-level, message: string);
}

interface stage {
    run: func(command: string, args: string, input: list<string>) -> result<list<string>, string>;
}

world extension {
    import host;
    export stage;
}
```

### Function `stage.run`

| Parameter | Type | Value |
|---|---|---|
| `command` | string | Full command name, `<extension>.<name>`. |
| `args` | string | JSON object from parameter name to value; parameters without a value are left out. Per-record expressions are evaluated once, without a record. |
| `input` | list of strings | Input records, one JSON document each. |

Result: the output records, one JSON document each, or a message describing
why the command failed.

A record has the JSON form `{"kind": "<record kind>", "fields": {...}}` with
field values in plain JSON (texts, numbers, booleans, lists and objects).
Members other than `kind` and `fields` are rejected.

### Host calls

| Call | Access | Result |
|---|---|---|
| `read-file` | below `permissions.read` or `permissions.write` | Whole file content, at most `limits.max_file_bytes_text` bytes. |
| `list-dir` | below `permissions.read` or `permissions.write` | Entry names in byte order. |
| `write-file` | below `permissions.write`, commands with `writes = true` | The write is recorded, not performed. The preview shows it as a change; the apply step writes it through the journal, so the change can be undone. The recorded writes of one call hold at most `limits.max_file_bytes_text` bytes together; a further write is refused with `E_LIMIT`. |
| `log` | always | Adds a `log` event with the command as source. |

A refused call returns `failure` with the code `E_PERMISSION`, `E_PATH_RELATIVE`,
`E_NOT_FOUND`, `E_LIMIT` or `E_IO`. When the command then fails, the error of
the command is the refused call.

### Limits

| Limit | Value | Error |
|---|---|---|
| Execution fuel per call | 10 000 000 000 units, about one per instruction | `E_LIMIT` |
| Linear memory per instance | 512 MiB | `E_STAGE_FAILED` |

Every call runs in a new instance; no state survives between calls.

## Native libraries

A native extension is a shared library exporting three C functions:

```c
uint32_t nurt_ext_api(void);
int32_t nurt_ext_run(const uint8_t *request, size_t request_len,
                     uint8_t **response, size_t *response_len);
void nurt_ext_free(uint8_t *response, size_t response_len);
```

| Function | Contract |
|---|---|
| `nurt_ext_api` | Returns the interface version as `MAJOR << 16 \| MINOR`. |
| `nurt_ext_run` | Receives the JSON object `{"command", "args", "input"}` and stores a response allocated by the library. Status 0 with `{"records": [...]}`; any other status with `{"error": "<message>"}`. |
| `nurt_ext_free` | Releases a response; called exactly once per response. |

Native code runs with the rights of the process, so the host cannot restrict
the paths it reaches. Native extensions load only with `ext.native = true` and
cannot declare commands with `writes = true`.

## Errors

| Code | Condition |
|---|---|
| `E_CATALOG` | Rejected manifest, interface version mismatch, core name, unusable component or library, missing export `nurt:ext/stage@1.0.0`, name taken in the registry. |
| `E_PERMISSION` | A host call reached a path outside the permissions. |
| `E_STAGE_FAILED` | The command returned a failure, trapped, returned an invalid record, or planned writes without `writes = true`. |
| `E_LIMIT` | The command used up its execution fuel. |
| `E_UNSUPPORTED` | Native extension not allowed or not built, or a native command declares `writes = true`. |
| `E_NOT_FOUND`, `E_EXISTS`, `E_IO` | Installation, removal and listing. |

## Examples

A read command of an extension in a pipeline:

```
find src -name '*.acme' |> acme.lint strict=true
```

Manifest of a command that changes files:

```toml
[ext]
name = "fmt"
version = "0.3.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "fmt.wasm"

[[stage]]
name = "apply"
input = ["entry"]
writes = true

[permissions]
write = ["${project}"]
```
