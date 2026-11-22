---
name: ffmpeg
description: Convert, cut and filter video and audio files. Use for transcoding, trimming and changing containers.
version: "7.1"
programs:
  - ffmpeg
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
## Invocation

```sh
ffmpeg -i input.mov -c:v libsvtav1 -crf 30 output.mkv
```

## Inspecting streams

`ffprobe -hide_banner input.mkv` lists the streams of a file.
