---
category: journal-and-undo
title: Journal, undo and safety
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Journal, undo and safety

## Show the history of operations

Show the five latest operations of the journal.

```nurt example
history limit: 5
```

```text expected
id
```

## Undo an operation

Undo one operation by its number.

```nurt example
undo 18
```

```text expected
18
```

## Undo with later changes included

Undo an operation together with the later operations that touched the same files.

```nurt example
undo 18 on_conflict: include_later
```

```text expected
18
```

## Undo the changes of one file

Undo the changes of one file made since an operation.

```nurt example
undo since: 18 paths: ["/home/user/app/src/a.go"]
```

```text expected
/home/user/app/src/a.go
```

## Redo an undone operation

Apply an undone operation again.

```nurt example
redo 18
```

```text expected
18
```

## Show the details of an operation

Show the files of one operation.

```nurt example
history 18 details: true
```

```text expected
18
```

## Find the operations that touched a path

Show the operations that changed a path.

```nurt example
history path: "/home/user/app/src/a.go"
```

```text expected
/home/user/app/src/a.go
```

## Pin an operation

Keep an operation in the journal for 24 hours.

```nurt example
journal.pin 18 until: 24h
```

```text expected
18
```

## Show the space of the journal

Show the free space and the space held by the journal of a volume.

```nurt example
journal.space
```

```text expected
free
```

## Show the state of the journal

Show whether the journal is on and how many operations it holds.

```nurt example
journal.status
```

```text expected
journal
```

## Set the retention limits

Keep the last 100 operations of the session.

```nurt example
journal.limits steps: 100
```

```text expected
100
```

## Purge expired entries

Remove the entries up to an operation from the journal.

```nurt example
journal.purge up_to: 17
```

```text expected
17
```

## Guard a change with a count

Stop before any change when a command targets more files than expected.

```nurt example
find "/home/user/app/build" glob: "**/*.o" |> assert count: 12 |> remove
```

```text expected
E_ASSERT
```

## Check a change with a build

Run a build after a change and roll back when it fails.

```nurt example
search /oldName\(/ "/home/user/app/src" glob: "**/*.go" |> replace "newName(" |> verify (run ["go", "build", "./..."] cwd: "/home/user/app" |> assert exit_code: 0)
```

```text expected
newName
```
