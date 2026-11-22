<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# XML corpus: elements, attributes, namespaces, CDATA, comments

Samples written for the Danaco Nurt test suite and released under the
license of the repository. Each profile has variants of indentation (two
and four spaces, three spaces, tabs), line endings (LF, CRLF), presence of
the XML declaration and of the final line break; some files start with a
UTF-8 byte order mark.

| Files | Profile |
|---|---|
| `pom-*.xml` | Maven project object model with default namespace and `xsi` attributes |
| `msbuild-*.csproj`, `*.props`, `*.targets` | MSBuild projects, conditions, items, targets |
| `AndroidManifest-*.xml` | Android manifests with attributes one per line |
| `web-*.xml` | Jakarta EE deployment descriptors |
| `spring-*.xml` | Spring bean definitions, prefixed elements and attributes, CDATA |
| `image-*.svg` | SVG with `xlink` references and nested groups |
| `page-*.xhtml` | XHTML with document type declarations, mixed content, CDATA in scripts |
| `feed-*.rss`, `atom-*.atom` | RSS 2.0 and Atom feeds |
| `messages-*.xlf` | XLIFF 1.2 translation units |
| `attributes-*.xml` | attributes split over lines and spaced around `=` |
| `cdata-*.xml` | CDATA sections, adjacent sections, empty sections |
| `mixed-*.xml` | mixed content, comments and processing instructions inside text |
| `namespaces-*.xml` | prefixes, default namespaces and undeclaring `xmlns=""` |
| `misc-*.xml` | comments and processing instructions outside the root, repeated names, deep nesting, non-ASCII text |

The output of `yq -p xml -o json` (yq 4.53.6) for every file is stored in
`tests/d5/data/u07/yq`.
