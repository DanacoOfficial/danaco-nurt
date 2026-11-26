---
command: help
group: core
summary: Describe commands, groups, errors, guides, the syntax and corrections.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# help

## Synopsis

```text
help [<topic>] [name: value]...
```

## Description

`help` returns documentation as text. Without arguments it prints the
introduction: the product line, a short description of programs and the
journal, and the lists of core commands, groups, guides and topics. The first
line is `Danaco Nurt <version> - command shell and language where every change
can be undone`.

A topic is resolved in this order when `kind` is not given: command, error
code, `syntax`, `corrections`, `commands`, `pipelines`, `let`, `paths`, `records`, `lists`, group, guide. The same name can denote a
command and a group (`verify`): `help verify` answers for the command,
`help verify kind: group` and `help group: verify` answer for the group.

| Topic | Answer |
|---|---|
| a command | signature, parameters with defaults, input and output kinds, effect, the first `examples` examples of the command page, typical errors, related commands |
| a group | the description of the group: its commands with parameters and examples |
| an error code | meaning, the stages in which it occurs, typical fix |
| a guide | the text of the guide |
| `syntax` | grammar, literals and expressions, and the rule on stages after the effect |
| `corrections` | classes and policy of corrections, the rule families and the result fields |
| `commands` | every command of the catalog under its group, with its summary |
| `pipelines`, `let`, `paths`, `records`, `lists` | the part of the syntax that the name says, the lines of `syntax` that apply; the hints of errors point at these topics |
| `search` | commands, error codes and guides that match the words, best first |

The topics `group`, `skill` and `search` exclude each other and the positional
topic. An unknown topic fails with `E_UNKNOWN_NAME`; the error lists up to
eight nearest names in `expected`.

The answer is text. With the call parameter `format: json` the answer is a
JSON document with the same content. `help "commands"` lists the commands of
all groups; `help group: "<group>"` describes one group.

The call parameter `language: pl`, or `output.language = "pl"` when the call
gives none, selects Polish for the fixed texts of `help` and `describe`: the
introduction, the headings and the labels. The call parameter wins over the
configuration. Command names, parameters, record fields, error codes, the
summaries of commands and the guides stay in English. The messages of
errors, the notes and the hints of a result are not translated; `language`
does not change them.

`help` never changes state and no command requires it to be read before it
runs.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `topic` | `text` | the list of commands, groups, guides and topics | Command, group, error code, syntax or corrections. |
| `kind` | `enum(command\|group\|error\|skill\|syntax\|corrections)` | `command` | Kind of the topic. |
| `group` | `text` | none | Group to describe, the same as kind: group. |
| `skill` | `text` | none | Guide to show, the same as kind: skill. |
| `search` | `text` | none | Words to look up in commands, errors and guides. |
| `examples` | `int` | `3` | Number of examples shown. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `text` |

## Effect

| Property | Value |
|---|---|
| Category | `help` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Show the introduction.

```nurt example
help
```

```text expected
command shell and language where every change can be undone
```

Show the signature of a command with its first example.

```nurt example
help "replace" examples: 1
```

```text expected
replace
```

Show the description of a group.

```nurt example
help group: "documents"
```

```text expected
GROUP documents
```

Show the meaning and the fix of an error code.

```nurt example
help "E_ASSERT"
```

```text expected
E_ASSERT
```

Look up the topics that mention a word.

```nurt example
help search: "rename symbol"
```

```text expected
code.rename
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_UNKNOWN_NAME` | the topic is not a command, group, error code, guide or topic | choose a name from `expected`, or run `help` for the list |
| `E_PARAM_CONFLICT` | more than one of `topic`, `group`, `skill` and `search` was given | keep one of them |

## Related

`describe`, `evaluate`, `help skill: "bash-migration"`.
