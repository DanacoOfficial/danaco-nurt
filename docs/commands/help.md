---
command: help
group: core
summary: Describe commands, groups, errors, guides, the syntax and corrections.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/help.md -->
<!-- Responsibility:   Documents help. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

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

With `kind` given, the topic is looked up among that kind only. `kind: syntax`
accepts the topic `syntax` and the topics of its parts (`pipelines`, `let`,
`paths`, `records`, `lists`); `kind: corrections` accepts the topic
`corrections`. Any other name fails with `E_UNKNOWN_NAME` and the names that
the kind accepts.

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

With `kind: syntax` the topic is empty, `syntax` or one of the topics
`pipelines`, `let`, `paths`, `records`, `lists`; with `kind: corrections` it is
empty or `corrections`. Any other name fails with `E_UNKNOWN_NAME` and is not
ignored.

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
summaries of commands and the guides stay in English.

`language: pl` also puts the messages of a result into Polish: the `message`
of an error is the Polish meaning of its code followed by `found`, `expected`
and `problem`; the English message is kept in the error detail `message_en`.
The `detail` of every note and of every irreversible effect starts with the
Polish text of its code, followed by the English detail when there is one. The
codes, the programs in `fix` and `next_steps` and the help topic are never
translated. A code without a Polish text keeps its English message.

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

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
