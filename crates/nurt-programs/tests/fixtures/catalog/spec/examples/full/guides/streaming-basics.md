---
name: streaming-basics
description: >-
  Choose between DASH and HLS, select segment duration
  and prepare one set of files for both.
version: "2"
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
## Formats

DASH uses an MPD manifest; HLS uses M3U8 playlists. Both accept fragmented MP4 segments.

## Segment duration

Segments of 2 to 6 seconds balance start-up delay and request overhead.
