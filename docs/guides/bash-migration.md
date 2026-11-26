---
name: bash-migration
description: Habits from bash and how Nurt corrects, translates or rejects them.
commands: [remove, move, copy, create, read, find, search, replace, write, run, list, lines, attributes]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# bash-migration

## What is corrected

A line that starts with a bash command is read with the rules of the POSIX shell and translated to a Nurt program. The result shows `corrected` and `ran`, the program that was executed. A translation that rests on an assumption and touches the target of a change is not applied: the answer is E_AMBIGUOUS with ready programs.

A line is a bash command when its first word is a bash command and it shows notation that only the shell has: a flag such as `-rf`, an operator such as `&&` or `>`, a quoted `'text'`, `$NAME`, an assignment before the program (`CI=1 npm test`) or an unquoted pattern. A first word that Nurt does not know at all (`touch`, `gzip`, `sha256sum`) needs none of these, and a word that Nurt reads as a synonym (`cat`, `ps`, `which`) is a bash command when its argument is a bare word that Nurt cannot take (`ps aux`, `which git`). A line with Nurt notation, a pipe `|>` or a named argument such as `glob:`, is always a Nurt program.

```text
rm -rf /srv/app/build
```

The line is corrected to `remove "/srv/app/build"`. The form to write directly uses a pattern, and the console lists the targets it computed:

```nurt
find "/srv/app" glob: "**/*.tmp" |> remove
```

## Commands and flags

`mv A B` becomes `move "A" to: "B"`, `cp -r A B` becomes `copy "A" to: "B"`, `mkdir -p D` becomes `create "D" type: directory`, `touch F` becomes `create "F" type: file on_exists: touch`, `cat F` becomes `read "F"`, and `ls D` becomes `list "D"`. Recursion and the creation of missing parents are the default, so `-r` and `-p` need no counterpart.

A long flag or a short flag becomes the named parameter of the command with the same meaning: `grep -i` becomes `case_sensitive: false`, `grep -v` becomes `invert: true`, `grep -l` becomes `output: files`. A flag without a counterpart is an error that lists the parameters of the command.

## Redirection and variables

`echo "x" > F` becomes `write "F" "x\n" mode: overwrite` and `>>` becomes `mode: append`. A here-document written to a file becomes `write` with a raw text.

`$NAME` becomes `@NAME` when the program has a matching `let`. Inside a text the substitution is not translated: write `@name.path` as an argument instead.

```nurt
let files = find "/srv/app" glob: "**/*.go"
run ["wc", "-l", ...@files.path] cwd: "/srv/app"
```

## Pipes, lists and sequences

A pipe `A | B` becomes `A |> B` when both sides translate. A sequence `A && B` or `A ; B` with two changing commands is not executed: the error E_ONE_EFFECT lists the two calls in `next_steps`. The alternative `A || B` has no counterpart; the correction proposes `A` with an `assert` and `B` as a separate call.

`find D -name P -delete` becomes `find "D" glob: "**/P" |> remove`. `xargs` and `find -exec` become a changing stage after `find`, or a `run` that receives `...@files.path`.

## What is rejected

Loops, conditionals, functions and `case` are rejected with an explanation: patterns and pipelines replace loops. `sudo` is rejected because the console does not raise privileges. `cd D && CMD` is translated only for an absolute path in the same line, as the `cwd` of `run` or as paths resolved against `D`; other uses of `cd` are rejected with a note on the call parameter `base`.

## Scripts

An existing shell script is run unchanged through `run ["bash", "script.sh"] cwd: "/srv/app"`. The command `nurt migrate` translates a script to a Nurt program and reports what it could not translate.
