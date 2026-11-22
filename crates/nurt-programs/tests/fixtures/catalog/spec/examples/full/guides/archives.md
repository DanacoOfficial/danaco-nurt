---
name: archives
description: Create, list and extract tar archives compressed with Zstandard.
version: "1"
programs: [tar, zstd]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
## Create

```sh
tar --zstd -cf backup.tar.zst directory/
```

## Extract

```sh
tar --zstd -xf backup.tar.zst
```
