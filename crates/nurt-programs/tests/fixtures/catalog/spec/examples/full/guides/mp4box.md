---
name: mp4box
description: "Package MP4 without re-encoding: subtitle tracks with a language, chapters, fragmented MP4, DASH. Use when subtitles must stay a separate track or DASH is needed."
version: "26.07"
programs: [MP4Box]
language: en
license: CC-BY-4.0
metadata:
  maintainer: media team
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
MP4Box changes the structure of MP4 files and leaves the encoded streams intact.

## Invocation

```sh
MP4Box -add film.mp4 -add subs.srt:lang=deu -new film-subs.mp4
## This line is inside a code block and does not start a section.
```

## Pitfalls

- Without `-new` the first input file is modified in place.
- Track identifiers start at 1.
