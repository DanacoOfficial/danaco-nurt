<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/architecture/crate-graph.md -->
<!-- Responsibility:   Documents Crate graph and visibility. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Graf skrzynek i widoczność {#crate-graph-and-visibility}

Przestrzeń robocza jest podzielona na skrzynki z przedrostkiem `nurt-`. Dozwolone
zależności między nimi są danymi: `tables/crate-graph.toml`. Test
`crates/nurt-contracts/tests/workspace_graph.rs` odczytuje każdy
`crates/*/Cargo.toml` i kończy się niepowodzeniem, gdy manifest deklaruje zależność od
skrzynki przestrzeni roboczej, której tabela nie dopuszcza, gdy skrzynka nie ma wpisu lub
gdy tabela łamie jedną z poniższych reguł. Nowa zależność między
skrzynkami przestrzeni roboczej zaczyna się więc od zmiany tabeli.

## Warstwy {#layers}

Każda skrzynka należy do jednej warstwy. Skrzynka zależy wyłącznie od skrzynek tej samej
lub niższej warstwy, a graf jest acykliczny.

| Warstwa | Skrzynki | Rola |
|---|---|---|
| kontrakt | `nurt-contracts` | typy, cechy i schematy wspólne dla każdej skrzynki; nie zależy od żadnej skrzynki przestrzeni roboczej |
| fundament | `nurt-types`, `nurt-catalog` | wartości języka i rejestr poleceń |
| baza | `nurt-fs`, `nurt-syntax`, `nurt-testkit` | warstwa systemu plików, składnia, wsparcie testów |
| biblioteka | `nurt-journal`, `nurt-space`, `nurt-report`, `nurt-plan`, `nurt-correct`, `nurt-text`, `nurt-doc`, `nurt-code`, `nurt-proc`, `nurt-git-transport`, `nurt-syslog`, `nurt-pdf-extract` | usługi wielokrotnego użytku |
| komponent | `nurt-bind`, `nurt-help`, `nurt-langserver`, `nurt-migrate`, `nurt-exec`, `nurt-ext`, `nurt-archive`, `nurt-notebook`, `nurt-files`, `nurt-lspclient`, `nurt-verify`, `nurt-git`, `nurt-git-ops`, `nurt-sys`, `nurt-net`, `nurt-sched` | polecenia, wiązanie, wykonanie, punkty rozszerzeń |
| host | `nurt-core`, `nurt-service` | złożenie wszystkich komponentów: fasada silnika i usługa w tle |
| interfejs | `nurt-ffi`, `nurt-cli`, `nurt-mcp` | punkty wejścia: interfejs C, wiersz poleceń, serwer Model Context Protocol |
| narzędzie | `nurt-conformance`, `nurt-bench` | zestaw zgodności i benchmark |

```
contract     nurt-contracts
                 |
foundation   nurt-types  nurt-catalog
                 |
base         nurt-fs  nurt-syntax  nurt-testkit
                 |
library      journal space report plan correct text doc code proc git-transport syslog
                 |
component    bind help langserver migrate exec ext archive notebook files
             lspclient verify git git-ops sys net sched
                 |
host         nurt-core  nurt-service
                 |
interface    nurt-ffi  nurt-cli  nurt-mcp
                 |
tool         nurt-conformance  nurt-bench
```

## Obszary i skrzynki wspólne {#areas-and-shared-crates}

Każda skrzynka należy także do obszaru odpowiedzialności: `runtime`,
`language`, `storage`, `files`, `structured`, `process`, `system`, `git` lub
`integration`. Skrzynki różnych obszarów zależą od siebie wyłącznie przez
skrzynki wspólne:

| Skrzynka wspólna | Dlaczego jest wspólna |
|---|---|
| `nurt-contracts` | same kontrakty |
| `nurt-types` | wartości i rekordy |
| `nurt-catalog` | rejestr poleceń używany przez wiązanie, pomoc i serwer języka |
| `nurt-fs` | warstwa systemu plików każdej platformy |
| `nurt-journal` | dziennik zmian |
| `nurt-testkit` | wsparcie testów; może być także zależnością deweloperską dowolnej skrzynki warstwy bazowej lub wyższej |

Polecenia jednego obszaru nigdy nie wywołują bezpośrednio innego obszaru. Gdy polecenie potrzebuje
usługi innego obszaru (na przykład dokumentów strukturalnych w poleceniu
testowym), usługa jest cechą w `nurt-contracts`, implementowaną we własnym
obszarze i przekazywaną przez warstwę hosta. Zmiany plików przechodzą przez
interfejs transakcji dziennika w `nurt-contracts`, nigdy poza nim.

Warstwy integrujące (`host`, `interface`, `tool`) składają skrzynki wszystkich
obszarów i są zwolnione z reguły obszarów.

## Nazwy zarezerwowane {#reserved-names}

Wpisy oznaczone w tabeli `reserved = true` nazywają skrzynki, które są planowane,
ale nie ma ich jeszcze w przestrzeni roboczej: `nurt-programs`, `nurt-tooldef`, `nurt-docgen`,
`nurt-crashfs` oraz opublikowaną fasadę `nurt`. Test kończy się niepowodzeniem, gdy taka
skrzynka pojawi się w `crates/`, a oznaczenie nadal jest obecne, aby jej wpis
został przejrzany przy tworzeniu skrzynki.

## Widoczność {#visibility}

- Elementy są domyślnie prywatne. Elementy współdzielone między modułami jednej skrzynki
  są `pub(crate)`; `pub` jest zarezerwowane dla interfejsu publicznego.
- Interfejs publiczny skrzynki to to, co udostępnia `lib.rs`. Moduły wewnętrzne
  są prywatne (`mod`, nie `pub mod`), a elementy publiczne są reeksportowane
  przez `pub use`. Wyjątkiem jest `nurt-contracts`, której moduły publiczne to
  przestrzenie nazw kontraktów (`nurt_contracts::config`,
  `nurt_contracts::journal` i tak dalej).
- Lint `unreachable_pub` jest włączony dla przestrzeni roboczej: element `pub`, który
  nie jest osiągalny z `lib.rs`, jest zgłaszany.
- `#[doc(hidden)] pub` nie służy do współdzielenia elementów wewnętrznych między skrzynkami; potrzeba
  współdzielenia oznacza, że element należy do niższej skrzynki lub do kontraktu.
- Każdy element publiczny jest udokumentowany (`missing_docs` ma poziom `deny` w skrzynkach
  bibliotecznych).
- Pomocniki testowe znajdują się w modułach `#[cfg(test)]` lub w `nurt-testkit`, nigdy w
  interfejsie publicznym skrzynki produkcyjnej.
- Kod `unsafe` jest zabroniony poza skrzynkami, które z definicji wywołują interfejsy systemu operacyjnego
  (`nurt-ffi`, `nurt-fs`, `nurt-sys`); tam każdy
  blok `unsafe` zawiera komentarz `// SAFETY:`.
- Tylko `nurt-ffi` eksportuje symbole C; ich deklaracje znajdują się w
  `crates/nurt-ffi/include/nurt.h`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
