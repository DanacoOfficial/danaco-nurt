<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Command documentation

Every command of the catalog has one page: the reference description for
people and the source of the examples and notes that the built-in help and the
model description show. Signatures are not written by
hand: names, types, positions, defaults and enumeration values come from the
catalog (`catalog/*.toml`, format `nurt-catalog/1`, see `spec/catalog.md`), and
the build checks that every page agrees with it.

## 1. Files

| Path | Content |
|---|---|
| `docs/commands/<command>.md` | page of one command; the file name is the full command name (`replace.md`, `git.status.md`) |
| `docs/commands/_template.md` | template of a new page |
| `docs/commands/README.md` | these conventions |

Every command of the catalog has exactly one page, and every page belongs to a
command of the catalog.

## 2. Front matter

A page starts with a YAML front matter block:

```yaml
---
command: replace
group: core
summary: Replace text in files or in the found text.
since: 0.1.0
---
```

| Field | Rule |
|---|---|
| `command` | full command name; equal to the file name without `.md` |
| `group` | group of the command in the catalog |
| `summary` | equal to the `summary` of the command in the catalog |
| `since` | version of Danaco Nurt that introduced the command |

## 3. Sections

The sections appear in this order, with these headings. Sections marked
"generated" are written by the generator from the catalog and are checked, not
edited.

| Heading | Content | Source |
|---|---|---|
| `# <command>` | title: the command name | front matter |
| `## Synopsis` | the forms of the command in variant A, one per line | generated |
| `## Description` | what the command does, in a few paragraphs; behaviour on empty input; how targets are computed | written |
| `## Parameters` | table: name, type, default, description; required parameters first | generated, descriptions may be extended |
| `## Input and output` | accepted and produced record kinds, per form | generated |
| `## Effect` | category, effect, reversibility and its conditions, external programs, journal behaviour | generated, notes written |
| `## Examples` | at least three examples executed by the build (section 4) | written |
| `## Errors` | error codes the command reports with their typical fix | written |
| `## Related` | related commands and guides | written |

A parameter description in the page may be longer than the catalog
`summary`, but must not contradict it. A page may add a `## Notes` section
before `## Examples` for platform differences.

## 4. Examples

Every page has at least three examples. Each example is a fenced block with
the info string `nurt example`, written in variant A of the syntax, optionally
followed by a block with the info string `fs-before` and a block with the
info string `text expected`:

````markdown
Replace a call in one file:

```nurt example
replace "oldName(" "newName(" in: ["/home/user/app/a.go"] occurrence: 1
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "x := oldName(1)\n" }
] }
```

```text expected
1 file changed, 1 replacement
```
````

| Block | Rule |
|---|---|
| `nurt example` | one program; it must parse, bind against the catalog and run |
| `fs-before` | optional snapshot of the virtual file system before the example, in the format `nurt-fs-snapshot/1`; without it the example runs on an empty file system |
| `text expected` | optional fragment that must appear in the text result of the example |

The sentence before an example says what it does. Examples use the paths
`/home/user/…` and `C:/Users/user/…` and the host `example.com`; they never
depend on the machine that builds them. The first three examples of a page are
the examples shown by `help <command>`; `help <command> examples: N` shows
more.

## 5. Documentation layers generated from the pages

| Layer | Content taken from the catalog | Content taken from the page |
|---|---|---|
| 1, core description for the model | name, positional parameters, defaults, enumeration values, summary | the first example of selected core commands |
| 2, group description | the same for the commands of the group | the first example of selected commands |
| 3, guides | command names only | none; guides are separate documents |
| 4, built-in help (`help <command>`) | synopsis, parameters with defaults, input and output, effect | the first three examples, `## Errors`, `## Related` |
| 5, reference (`nurt help --format man\|html\|markdown`) | everything | the whole page |

Reading any layer is optional; no command requires its documentation to be
read before it runs.

## 6. Checks

The build fails when:

1. a command of the catalog has no page, or a page has no command;
2. the front matter differs from the catalog (`command`, `group`, `summary`);
3. a generated section differs from the text generated from the catalog;
4. a page has fewer than three `nurt example` blocks;
5. an example does not parse, does not bind, fails to run on the virtual file
   system, or its result does not contain the `text expected` fragment;
6. a parameter or command name mentioned in backticks does not exist in the
   catalog.

## 7. Style

Pages are written in English, in the present tense and the impersonal style
of reference documentation ("`insert` adds text before or after …"), without tutorial
narrative. Commands, parameters, values and record fields are
written in backticks. Lines are wrapped at about 80 characters, except tables
and code.
