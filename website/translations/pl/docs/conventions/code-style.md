<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/conventions/code-style.md -->
<!-- Responsibility:   Documents Code style. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Styl kodu {#code-style}

## Zestaw narzędzi i formatowanie {#toolchain-and-formatting}

- Rust edycja 2024. Zestaw narzędzi jest przypięty w `rust-toolchain.toml`;
  minimalna obsługiwana wersja Rust to `rust-version` obszaru roboczego.
- Kod jest formatowany za pomocą `rustfmt` z ustawieniami z `rustfmt.toml`
  (maksymalna szerokość wiersza 100, zakończenia wierszy w stylu Unix). `cargo fmt --all --check`
  musi zakończyć się powodzeniem.
- Pliki tekstowe używają UTF-8 i zakończeń wierszy LF (`.editorconfig`, `.gitattributes`).

## Reguły lintera {#lints}

Lints są skonfigurowane w `[workspace.lints]` głównego `Cargo.toml` oraz w
`clippy.toml`. Każdy crate dziedziczy je przez `[lints] workspace = true`.
Ciągła integracja traktuje każde ostrzeżenie jako błąd:

```sh
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Lint jest wyłączany wyłącznie na możliwie najmniejszym elemencie, z uzasadnieniem
wyjaśniającym, dlaczego kod jest poprawny: `#[expect(lint, reason = "...")]` albo
`#[allow(lint, reason = "...")]`, gdy lint nie jest wyzwalany na każdym
celu. Przed `#[allow(...)]` bez argumentu `reason` stoi
komentarz podający uzasadnienie.

## Dokumentacja {#documentation}

- Crate'y biblioteczne deklarują `#![deny(missing_docs)]`; każdy element publiczny ma
  dokumentację rustdoc.
- Pierwszy akapit to jedno zdanie, które określa, czym element jest lub co robi.
  Sekcje `# Errors`, `# Panics` (tylko testy i przykłady) oraz `# Safety`
  są używane tam, gdzie mają zastosowanie.
- Przykłady w dokumentacji kompilują się i są uruchamiane jako testy dokumentacji.
- Dokumentacja jest budowana bez ostrzeżeń:
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.

## Błędy i paniki {#errors-and-panics}

- Kod produkcyjny nie wywołuje `unwrap`, `expect`, `panic!`, `todo!` ani
  `unimplemented!` i nie indeksuje wycinków w sposób, który może wywołać panikę
  na danych zewnętrznych. Operacje mogące zawieść zwracają `Result` z typami błędów
  crate'a `nurt-contracts`; każdy błąd niesie stabilny kod błędu.
- Testy mogą używać `unwrap`, `expect` i `panic!`. Pliki testów integracyjnych w
  `tests/` zaczynają się od
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]`.
- Komunikaty błędów są krótkie i rzeczowe, zaczynają się małą literą, nie mają
  kropki na końcu i wskazują, co poprawić.

## Kod unsafe {#unsafe-code}

- `#![forbid(unsafe_code)]` w każdym crate'cie, z wyjątkiem crate'ów, które z definicji wywołują
  interfejsy systemu operacyjnego lub zewnętrzne (interfejs C, warstwa systemu
  plików, warstwa informacji o systemie).
- W tych crate'ach każdy blok `unsafe` jest możliwie najmniejszy i jest
  poprzedzony komentarzem `// SAFETY:` wyjaśniającym, dlaczego operacja jest poprawna.

## Struktura {#structure}

- Złożoność kognitywna funkcji wynosi co najwyżej 15 (`clippy.toml`).
- Elementy są prywatne, o ile nie należą do interfejsu crate'a;
  `pub(crate)` jest używane dla elementów współdzielonych w obrębie crate'a.
- Crate'y zależą od siebie wyłącznie zgodnie z grafem crate'ów w dokumentacji
  architektury; typy współdzielone znajdują się w `nurt-contracts`.
- Kod specyficzny dla platformy jest wydzielony za atrybutami `cfg` w dedykowanych
  modułach, tak aby każdy crate budował się w systemach Linux, macOS i Windows.

- Scalony kod nie zawiera martwego kodu, zastępczych implementacji
  (`todo!`, `unimplemented!`, puste ciała) ani abstrakcji wykraczających poza
  specyfikację.
- Literały liczbowe mające znaczenie są nazwanymi stałymi.

## Komentarze i wyjście {#comments-and-output}

- Komentarz podaje to, czego kod nie pokazuje: niezmiennik, uzasadnienie
  nieoczywistej decyzji, odwołanie do standardu, RFC lub udokumentowanego
  zachowania systemu operacyjnego. Nie pisze się komentarzy, które powtarzają kod lub wyliczają
  jego kroki.
- Komentarze nie opisują historii kodu ani nie zwracają się do recenzenta.
  Zakomentowany kod nie jest zatwierdzany.
- Znaczniki niedokończonej pracy nie są zatwierdzane; otwarte prace są śledzone w zgłoszeniach.
- Kod biblioteczny nie wypisuje na standardowe wyjście ani standardowe wyjście błędów; rodziny
  `dbg!`, `print!` i `eprint!` są zarezerwowane dla programu wiersza poleceń
  oraz dla testów. Diagnostyka używa `tracing`.

## Język {#language}

Kod, komentarze, dokumentacja, komunikaty programu i komunikaty rewizji
są pisane po angielsku, w formalnym, technicznym i bezosobowym rejestrze:

- krótkie, oznajmujące, jednoznaczne zdania; terminologia
  specyfikacji, stosowana spójnie;
- bez przymiotników promocyjnych lub oceniających, bez asekuracji, bez zwrotów skierowanych do
  czytelnika, bez wykrzyknień, pytań retorycznych, emoji i ozdobnych
  symboli;
- bez nagłówków i wstępów powtarzających treść dokumentu;
- wyróżnienia i listy tylko tam, gdzie zdanie nie wystarcza.

Grupa `style` skryptu `tools/ci/repo-hygiene.sh` sprawdza te reguły tam, gdzie
można je wykryć automatycznie.

## Testy {#tests}

- Nazwy testów opisują testowane zachowanie, na przykład
  `rejects_path_outside_root`.
- Dane testowe są minimalne, a ich cel jest podany.
- Testy jednostkowe znajdują się w module `tests` pliku; testy integracyjne w
  katalogu `tests/` crate'a.
- Testy są deterministyczne i samodzielne: tworzą pliki w katalogach tymczasowych
  (`tempfile`), używają ścieżek względnych względem katalogu testu i
  nie zależą od strefy czasowej, ustawień regionalnych ani konta użytkownika maszyny.
- Testy nie korzystają z sieci. Testy wymagające sieci lub programu zewnętrznego
  są oznaczone `#[ignore]` z uzasadnieniem i uruchamiane jawnie.
- Pliki wzorcowe (oczekiwane wyjścia) są przechowywane w `spec/golden/` i są
  porównywane jako wartości, a nie jako tekst; są regenerowane wyłącznie celowo.

## Przykłady i dane testowe {#examples-and-test-data}

Przykładowe ścieżki używają `/home/user` i `C:/Users/user`; przykładowe domeny używają
`example.com`, `example.org` i `example.net`; przykładowe adresy IPv4 używają
zakresów dokumentacyjnych `192.0.2.0/24`, `198.51.100.0/24` i `203.0.113.0/24`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
