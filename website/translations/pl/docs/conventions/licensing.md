<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/conventions/licensing.md -->
<!-- Responsibility:   Documents Licensing conventions. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Konwencje licencjonowania {#licensing-conventions}

## Licencja produktu {#license-of-the-product}

Danaco Nurt jest udostępniany na licencji Apache License, Version 2.0. Pełny tekst
znajduje się w [LICENSE](../../LICENSE); nota atrybucji znajduje się w
[NOTICE](../../NOTICE) i musi być dołączona do każdej dystrybucji (sekcja
4(d) Licencji). Wkłady są przyjmowane na tej samej licencji oraz
zgodnie z Developer Certificate of Origin 1.1 (zob.
[CONTRIBUTING.md](../../CONTRIBUTING.md)).

## Nagłówki plików {#file-headers}

Licencja produktu jest podana wyłącznie w pliku `LICENSE`. Pliki źródłowe
nie zawierają wierszy z identyfikatorem licencji dla poszczególnych plików (identyfikatory SPDX ani
noty licencyjne).

Każdy plik źródłowy w formacie obsługującym komentarze zaczyna się od nagłówka producenta,
trzech wierszy w składni komentarzy danego formatu:

```text
Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
https://github.com/DanacoOfficial | support@danaco-group.pl
```

| Format | Składnia komentarza nagłówka |
|---|---|
| Rust, C, nagłówek C, JavaScript, TypeScript | każdy wiersz poprzedzony `// ` |
| TOML, YAML, powłoka, Python, PowerShell, `.gitignore`, `.gitattributes`, `.editorconfig` | każdy wiersz poprzedzony `# ` |
| Markdown, HTML, XML | komentarz HTML (`<!--` … `-->`) zawierający trzy wiersze |

Rokiem jest rok utworzenia pliku. Skrypt z wierszem interpretera
(`#!/usr/bin/env bash`) ma nagłówek bezpośrednio po nim.
Front matter YAML poprzedza nagłówek w plikach Markdown, które go potrzebują. Nagłówki
brakujące we wkładzie są uzupełniane przez opiekunów przed scaleniem.

Pliki bez składni komentarzy nie mają nagłówka i są objęte licencją
repozytorium: JSON (schematy, pliki wzorcowe), pliki blokady, binarne dane testowe,
`LICENSE` i `NOTICE`. Teksty pochodzące od osób trzecich, takie jak
[CODE_OF_CONDUCT.md](../../CODE_OF_CONDUCT.md) (Contributor Covenant 2.1,
CC BY 4.0), są przechowywane w niezmienionej postaci z własnym oświadczeniem licencyjnym.

Grupa `headers` skryptu `tools/ci/repo-hygiene.sh` zgłasza pliki w formatach
z komentarzami, w których pierwsze wiersze nie zawierają wiersza praw autorskich nagłówka producenta
(kontrola `producer-header`), pliki z wierszami identyfikatora licencji dla poszczególnych plików
(kontrola `spdx-line`) oraz pliki ze znakami narzędzia autorskiego (kontrola
`tool-mark`). Wyjątki są wymienione w `tools/ci/hygiene-allow.txt`.

## Kod stron trzecich {#third-party-code}

Kod skopiowany z innego projektu nie jest przyjmowany, o ile jego licencja nie znajduje się na
poniższej liście dozwolonych, jego pochodzenie i licencja nie są podane w komentarzu na początku
pliku, wymagane przez licencję noty nie są dodane do `NOTICE`, a
plik nie jest wymieniony z uzasadnieniem w `tools/ci/hygiene-allow.txt`.

Crate `crates/nurt-pdf-extract` jest taką kopią: źródło pdf-extract
0.12.1 (MIT), zbudowane na nowszym wydaniu lopdf, niż pozwala
wydanie nadrzędne. Tekst jego licencji to `crates/nurt-pdf-extract/LICENSE-MIT`.

## Zależności {#dependencies}

Wszystkie zależności są zadeklarowane w `[workspace.dependencies]` głównego
`Cargo.toml`, pochodzą wyłącznie z crates.io i są sprawdzane przez
[cargo-deny](https://github.com/EmbarkStudios/cargo-deny) zgodnie z polityką z
[deny.toml](../../deny.toml):

```sh
cargo deny --locked check
```

### Dozwolone licencje {#allowed-licenses}

Licencje permisywne i zgodne z Apache License 2.0:

| Licencja | Identyfikator SPDX |
|---|---|
| MIT | `MIT` |
| Apache License 2.0, także z wyjątkiem LLVM | `Apache-2.0`, `Apache-2.0 WITH LLVM-exception` |
| BSD 2-Clause i 3-Clause | `BSD-2-Clause`, `BSD-3-Clause` |
| ISC | `ISC` |
| zlib | `Zlib` |
| The Unlicense | `Unlicense` |
| Creative Commons Zero 1.0 | `CC0-1.0` |
| Zero-Clause BSD | `0BSD` |
| Unicode License v3 (dane) | `Unicode-3.0` |
| Boost Software License 1.0 | `BSL-1.0` |

Wyrażenie z alternatywami (`A OR B`) jest akceptowane, gdy jedna alternatywa jest
dozwolona; używana jest alternatywa permisywna.

### Licencje akceptowane dla poszczególnych crate'ów {#licenses-accepted-for-individual-crates}

Poniższe licencje są akceptowane wyłącznie dla wymienionych crate'ów, po
indywidualnym przeglądzie; każdy wpis w `deny.toml` podaje uzasadnienie.

| Licencja | Crate | Uzasadnienie |
|---|---|---|
| `CDLA-Permissive-2.0` | `webpki-roots` | certyfikaty główne jako dane; rozwiązanie zapasowe, gdy systemowy magazyn certyfikatów jest niedostępny |
| `bzip2-1.0.6` | `libbz2-rs-sys` | permisywna licencja implementacji bzip2 |
| `MPL-2.0` | `uluru`, `option-ext` | copyleft na poziomie plików w niezmodyfikowanych zależnościach pośrednich |
| `NCSA` | `libfuzzer-sys` | środowisko uruchomieniowe fuzzingu, niewchodzące w skład dystrybuowanych artefaktów |

### Licencje i crate'y wykluczone {#excluded-licenses-and-crates}

Licencje copyleft nie są akceptowane dla żadnej zależności: GPL, LGPL i AGPL w
każdej wersji oraz licencje ograniczające dziedzinę zastosowania. Produkt
nie osadza ani nie linkuje kodu programów GPL lub LGPL (na przykład powłok, podstawowych
narzędzi GNU, Gita, BusyBox, GNU Readline, libsystemd).

W `deny.toml` zakazane są następujące crate'y:

| Crate | Powód | Używane zamiast |
|---|---|---|
| `sequoia-openpgp` | LGPL-2.0-or-later | GnuPG, uruchamiany jako program |
| `openssl`, `openssl-sys` | systemowa biblioteka OpenSSL | `rustls` |
| `git2`, `libgit2-sys` | libgit2 (GPL-2.0 z wyjątkiem linkowania) | `gix` |
| `serde_yaml` | wycofany i niemaintainowany | `saphyr`, `saphyr-parser` |
| `ini` | podwójnie licencjonowany z LGPL-3.0-or-later | `rust-ini` i własny parser |
| `libsystemd-sys` | libsystemd (LGPL-2.1) | `zbus`; natywny czytnik plików dziennika |
| `unrar`, `unrar_sys` | licencja UnRAR zabrania tworzenia kompresora RAR | brak |

### Dodawanie zależności {#adding-a-dependency}

Pull request dodający zależność podaje crate, wersję,
licencję i uzasadnienie oraz aktualizuje `Cargo.lock`. Narzędzia czasu budowania, których
licencja nie obejmuje wytwarzanych artefaktów, są oceniane indywidualnie.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
