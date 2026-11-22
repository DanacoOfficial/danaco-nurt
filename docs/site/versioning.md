---
title: Versioning and support
order: 11
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Versioning and support

| Element | Rule |
|---|---|
| Program | Semantic Versioning 2.0.0. Releases of one major version do not break compatibility. Minor releases appear every 8 weeks and patch releases as needed |
| Language | `language MAJOR.MINOR`. Within a major version the language only grows. Synonyms and aliases of the corrections are kept. A deprecation is marked in `help` and the removal comes no earlier than 24 months later, in a major version |
| Syntax tree | `nurt-ast/MAJOR`. A new minor version only adds. The previous major version is accepted for 24 months |
| C interface | `NURT_ABI_MAJOR.MINOR` and the library name `libnurt.so.<MAJOR>`. Within a major version functions are only added |
| Catalog | The catalog only grows within `nurt-catalog/1`: new commands, new optional parameters with a default that keeps the previous behavior, new enumeration values. Removing or renaming a command or parameter, or changing a type or default, is a change of the major version |
| Long-term support | One release a year is a long-term support release. It receives fixes of defects and vulnerabilities for 36 months and no new features |

## Supported systems

Linux (x86_64, aarch64), macOS (x86_64, aarch64) and Windows (x86_64, aarch64).

## Support

Questions and defects are reported to `support@danaco-group.pl`. Vulnerabilities follow [Security](security.md).
