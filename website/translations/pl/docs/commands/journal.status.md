---
command: journal.status
group: journal
summary: Raportuje liczbę wpisów, zajęte miejsce, najstarszy wpis oraz aktywne limity wraz z ich źródłami.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.status

## Składnia {#synopsis}

```text
journal.status [name: value]...
```

## Opis {#description}

`journal.status` raportuje stan przechowywania dziennika: jeden rekord
`journal_state` na wolumin albo dla woluminu, który zawiera `volume`. Jest to
jedyne polecenie raportujące stan przechowywania; wyniki innych poleceń podają,
czy operację można cofnąć, nigdy na jak długo.

Obowiązujące limity są rozstrzygane z siedmiu poziomów konfiguracji, w rosnącej
kolejności pierwszeństwa: wartości domyślne wbudowane, plik systemowy, plik
użytkownika, plik projektu `.nurt.toml` (tylko dla zaufanego projektu), plik
wskazany przez `NURT_CONFIG`, zmienne środowiskowe `NURT_JOURNAL_*` oraz
ustawienia czasu wykonania z `journal.limits`. Każdy limit jest raportowany
wraz z poziomem, który go ustawił. Dolny próg ochrony (`journal.floor.max_ops`,
`journal.floor.max_age`) podnosi niższe wartości z dowolnego poziomu; podniesiony
limit ma źródło `floor`.

| Pole | Znaczenie |
|---|---|
| `active` | dziennik przechowuje dane na woluminie |
| `entries` | wpisy, które można cofnąć, łącznie z wpisami częściowymi |
| `bytes` | bajty logiczne przechowywane przez dziennik |
| `limits` | limity `steps`, `age` i `size`, każdy z `name`, `value`, `source`, `scope` i `floor` |
| `volume` | punkt montowania woluminu |
| `oldest` | czas zatwierdzenia najstarszego wpisu, który można cofnąć |
| `oldest_id` | numer tego wpisu |
| `reason` | dlaczego dziennik jest wyłączony na woluminie, na przykład `service_down` lub `network_volume`; nieobecne, gdy jest włączony |

`source` wskazuje poziom, który ustawił wartość: `default`, `system`, `user`,
`project`, `config_file`, `environment`, `session` (ustawione przez
`journal.limits` dla sesji, wtedy `scope` ma wartość `session`) lub `floor`.
`floor` podaje dolny próg ochrony dla `steps` i `age`; jest nieobecny, gdy
obowiązuje tylko minimum projektu. `unlimited` jest wartością limitu, który jest
wyłączony.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `volume` | `path` | brak | Tylko dziennik woluminu, który zawiera tę ścieżkę. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `journal_state` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zaraportuj dziennik każdego woluminu:

```nurt example
journal.status
```

Zaraportuj dziennik woluminu, który przechowuje projekt:

```nurt example
journal.status volume: "/home/user/project"
```

Wyświetl woluminy, na których dziennik jest wyłączony:

```nurt example
journal.status |> filter active == false
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | żaden wolumin dziennika nie zawiera `volume` | pomiń `volume` albo wskaż ścieżkę na woluminie z dziennikiem |
| `E_CONFIG` | plik konfiguracji zawiera nieprawidłowe ustawienie dziennika | popraw klucz wskazany w błędzie |
| `E_IO` | nie można odczytać katalogu dziennika | sprawdź uprawnienia katalogu dziennika |

## Powiązane {#related}

`journal.limits`, `journal.space`, `journal.purge`, `history`.
