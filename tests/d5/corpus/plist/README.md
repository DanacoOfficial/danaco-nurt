# Property list corpus

Property lists used by `crates/nurt-doc/tests/u04_plist.rs` and
`crates/nurt-doc/tests/u04_corpus.rs`. Every file is written byte for byte.

| Files | Construct |
|---|---|
| `Info.plist`, `app.entitlements`, `xml_*` | XML property lists: tabs, two and four spaces, CRLF, one line, comments, CDATA, character references, byte order mark, wrapped data, repeated keys, empty containers |
| `ascii_*` | ASCII (OpenStep) property lists: comments, strings files without braces, project files, GNUstep typed values, hexadecimal data, escapes, single quotes, trailing commas, CRLF, byte order mark |

The tests generate further documents from the XML samples: binary property
lists written by the `plist` crate, XML written again with other
indentation and line breaks, and ASCII forms in four layouts.
