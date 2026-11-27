---
command: windows.cim
group: admin
summary: Query CIM instances through WMI on Windows, or call a method of the selected instances with invoke.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# windows.cim

## Synopsis

```text
windows.cim <class> [name: value]...
```

## Description

`windows.cim` reads instances of a CIM class from the local Windows
Management Instrumentation service and returns one `cim_object` record per
instance. The command builds the WQL statement
`SELECT <properties> FROM <class> [WHERE <where>]` from its parameters; all
properties are read when `properties` is omitted or empty.

`class` and every element of `properties` must be CIM names
(`[A-Za-z_][A-Za-z0-9_]*`), so they cannot change the statement. `where` is a
WQL condition on the properties of the class. It must be one line with
balanced `'` and `"` string literals and must not contain the association
query forms `ASSOCIATORS OF` or `REFERENCES OF`. `namespace` is written with
`/` separators, for example `root/Microsoft/Windows/Storage`.

Property values are converted to language values by their CIM type:

| CIM type | Value |
|---|---|
| `string`, `char16`, `reference` | `text` |
| `sint8` to `uint64` | `int`; values of `uint64` above the range of `int` keep all digits |
| `real32`, `real64` | `number` |
| `boolean` | `bool` |
| `datetime`, point in time | `instant` in UTC; the offset of the value is applied |
| `datetime`, interval | `duration` |
| arrays | `list` |
| embedded objects | `map` with the field `__class`, up to four levels deep |

Properties without a value are left out of `properties`. A `datetime` value
with unknown fields (`*`) is returned as `text`.

`path` holds the object path reported by WMI. When `properties` leaves out
the key properties of the class, WMI does not report the object path and
`path` is empty.

The rest of the pipeline may limit the number of records, for example with
`first`; the query then stops reading after that number of instances.

## Calling a method

With `invoke` the command calls the method of the class instead of reading.
`where` selects the instances that receive the call; without `where` the
method of the class is called, as for the static method `Create` of
`Win32_Process`. `arguments` holds the input parameters of the method by name;
a value is text, an integer or a boolean. `invoke` and the names in
`arguments` must be CIM names. The call is an effect of the program: a preview
reads the selected instances and shows the call without making it, and a
method that returns a `ReturnValue` other than 0 ends the call with
`E_STAGE_FAILED`; the effect record holds `return_value`.

A call is a change of the target `cim:<class>.<method>`. The call is
irreversible unless the class documents an opposite method, which `undo` calls
on the same instances; a call without `where` has no instances to repeat the
opposite on, so it stays irreversible.

| Class | Method | Opposite |
|---|---|---|
| `Win32_Service` | `StartService`, `StopService` | each other |
| `Win32_NetworkAdapter`, `Win32_PnPEntity` | `Enable`, `Disable` | each other |
| `Win32_Printer` | `Pause`, `Resume` | each other |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `class` | `text` | required | CIM class name, for example Win32_Process. |
| `where` | `text` | none | WQL condition on instance properties. |
| `namespace` | `text` | `"root/cimv2"` | CIM namespace. |
| `properties` | `list<text>` | none | Properties to return; all when omitted. |
| `invoke` | `text` | none | Method to call on every selected instance, or on the class when where is omitted (static methods such as Win32_Process.Create). |
| `arguments` | `map<any>` | none | Input parameters of the method by name: text, int or bool values. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `cim_object`, `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |
| Operating systems | `windows` |
| Reversibility conditions | A query only reads. A call with invoke is irreversible unless the method has a documented opposite (StartService and StopService, Enable and Disable, Pause and Resume), which undo calls on the same instances. |

| Action | Effect | Reversibility | Note |
|---|---|---|---|
| `invoke` | `external` | `irreversible` | Applies when invoke is given. |

## Examples

Read the name and version of the operating system.

```nurt example
windows.cim "Win32_OperatingSystem" properties: ["Caption", "Version"]
```

List the running services.

```nurt example
windows.cim "Win32_Service" where: "State = 'Running'" properties: ["Name", "ProcessId"]
```

List the disks known to the storage management provider.

```nurt example
windows.cim "MSFT_Disk" namespace: "root/Microsoft/Windows/Storage"
```

Find processes by name and keep the command lines.

```nurt example
windows.cim "Win32_Process" where: "Name = 'svchost.exe'" properties: ["ProcessId", "CommandLine"] |> first 5
```

Stop a service; `undo` starts it again.

```nurt example
windows.cim "Win32_Service" where: "Name = 'Spooler'" invoke: "StopService" privilege: elevated
```

Start a process through the static method of the class.

```nurt example
windows.cim "Win32_Process" invoke: "Create" arguments: {CommandLine: "notepad.exe"} privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_OS_ONLY` | The command runs only on Windows. | Use `host`, `processes` or `services` on other systems. |
| `E_INVALID_VALUE` | A class, property or namespace is not a CIM name, the condition is malformed, or WMI rejected the query. | Correct the name or the condition. |
| `E_NOT_FOUND` | The class or the namespace does not exist. | Check the class name and the namespace. |
| `E_PRIVILEGE` | Reading the namespace requires the `administrator` privilege. | Run the console as an administrator. |
| `E_STAGE_FAILED` | The method returned a `ReturnValue` other than 0; `return_value` holds it. | Read the documentation of the method for the code. |
| `E_LIMIT` | The query returns more instances than a stage may hold. | Narrow the query with `where`. |
| `E_IO` | WMI or COM failed; `os_error` holds the `HRESULT`. | Check that the WMI service is running. |

## Related

`services`, `host`, `processes`, `disks`.
