---
name: mp4box
description: "MP4 ohne Neukodierung verpacken: Untertitel als Spur mit Sprache, Kapitel, DASH."
version: "26.07"
programs: [MP4Box]
language: de
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
## Aufruf

```sh
MP4Box -add film.mp4 -add untertitel.srt:lang=deu -new film-untertitel.mp4
```

## Fallstricke

- Ohne `-new` wird die erste Eingabedatei direkt geändert.
