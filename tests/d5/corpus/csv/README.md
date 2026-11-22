# CSV and TSV corpus

Delimited files used by `crates/nurt-doc/tests/u04_csv.rs` and
`crates/nurt-doc/tests/u04_corpus.rs`. Every file is written byte for byte;
the name states the construct it holds.

| Files | Construct |
|---|---|
| `rfc4180_*` | RFC 4180 quoting, `""` inside quoted fields, CRLF inside a field |
| `*_quoted*`, `all_quoted`, `quoted_header` | quoted fields, every field quoted |
| `multiline_fields`, `tab_quoted.tsv` | fields over several lines |
| `semicolon_*`, `pipe_*`, `*.tsv`, `*.tab` | separators `;`, `\|` and tab |
| `bom_*` | UTF-8 byte order mark |
| `*crlf*`, `mixed_line_endings`, `cr_line_endings` | CRLF, mixed and CR line breaks |
| `crlf_no_final_break`, `rfc4180_embedded_crlf` | no final line break |
| `noheader_*` | data without a header row |
| `ragged_rows`, `trailing_delimiter`, `empty_fields` | rows with different numbers of fields |
| `blank_lines`, `trailing_blank_lines`, `empty`, `header_only` | blank lines and empty tables |
| `single_quote_char`, `irregular_quote`, `only_quote_field`, `escaped_only` | quote characters |

The tests generate 96 further files: every combination of separator,
quoting, line break, byte order mark and final line break.
