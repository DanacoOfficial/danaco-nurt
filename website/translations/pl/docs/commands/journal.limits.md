---
command: journal.limits
group: journal
summary: Pokazuje aktywne limity wygasania albo je zmienia i raportuje, ile wpisów wygaśnie.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.limits

## Składnia {#synopsis}

```text
journal.limits [name: value]...
```

## Opis {#description}

`journal.limits` pokazuje limity, które decydują, jak długo dziennik przechowuje
wpisy, albo je zmienia. Bez `steps`, `age` i `size` polecenie tylko odczytuje:
raportuje obowiązujące limity każdego woluminu albo woluminu, który zawiera
`volume`, wraz z poziomem konfiguracji, który ustawił każdy z nich.

Wpis wygasa, gdy przekroczony zostanie dowolny z trzech limitów:

| Limit | Klucz | Znaczenie |
|---|---|---|
| `steps` | `journal.max_ops` | wpis `n` wygasa, gdy liczba późniejszych operacji zmieniających osiągnie limit; operacja zbiorcza, cofnięcie i ponowienie liczą się jako jedna operacja każde |
| `age` | `journal.max_age` | wpis wygasa, gdy jego wiek osiągnie limit; wiek jest mierzony przez `journal.age_clock` |
| `size` | `journal.max_size` | gdy bajty przechowywane na woluminie przekroczą budżet, najstarsze wpisy wygasają, aż przechowywane bajty się zmieszczą |

`unlimited` wyłącza limit. Wolne miejsce dla użytkownika jest w każdym
przypadku chronione: dziennik nigdy nie przechowuje danych poniżej rezerwy
miejsca woluminu.

Każdy limit jest rozstrzygany osobno z siedmiu poziomów, w rosnącej kolejności
pierwszeństwa: wartości domyślne wbudowane, plik systemowy, plik użytkownika,
plik projektu `.nurt.toml` (tylko dla zaufanego projektu), plik wskazany przez
`NURT_CONFIG`, zmienne środowiskowe `NURT_JOURNAL_*` oraz ustawienia czasu
wykonania wprowadzone tym poleceniem. `journal.volume."<mount>".*` ma
pierwszeństwo przed kluczem ogólnym tego samego poziomu.

Dolny próg ochrony, `journal.floor.max_ops` i `journal.floor.max_age`, ustawia
administrator w pliku systemowym albo aplikacja osadzająca. Niższa wartość z
dowolnego poziomu, w tym z tego polecenia, jest podnoszona do progu i
raportowana jako `raised_to_floor`. Wartości z pliku projektu są podnoszone do
co najmniej 10 kroków i 5 minut.

Obniżenie limitu działa natychmiast: wpisy ponad nowy limit wygasają w trakcie
wywołania, od najstarszych. Podniesienie limitu nigdy nie przywraca wpisów, które
już wygasły. Ze wspólnym parametrem `preview: true` polecenie raportuje
obowiązujące limity i wpisy, które by wygasły, i niczego nie zmienia.

`scope: session` zachowuje limity do końca sesji żądania albo do upływu
`session.ttl` bez wywołań sesji. `scope: user` zapisuje sekcję `[journal]` pliku
konfiguracji użytkownika, albo tabelę `[journal.volume."<mount>"]` z `volume`,
zachowując jego komentarze i układ. Brakujące katalogi pliku są tworzone. Zmiana
jest zapisywana jako wpis dziennika rodzaju `journal.limits`, który nie liczy
się jako krok; `undo` tego wpisu przywraca poprzednią zawartość pliku.
`scope: user` wymaga skonfigurowania lokalizacji pliku konfiguracji
użytkownika dla dziennika; w przeciwnym razie wywołanie, które ustawia limit,
kończy się błędem `E_UNSUPPORTED`, a wywołanie bez `steps`, `age` i `size` tylko
raportuje.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `steps` | `int \| enum(unlimited)` | brak | Maksymalna liczba przechowywanych wpisów. |
| `age` | `duration \| enum(unlimited)` | brak | Maksymalny wiek przechowywanych wpisów. |
| `size` | `size \| percent \| enum(unlimited)` | brak | Maksymalny rozmiar dziennika jako rozmiar, taki jak 20gib, udział woluminu, taki jak "10%", albo udział wolnego miejsca, taki jak "50%free". |
| `scope` | `enum(session\|user)` | `session` | Zmienia limity dla tej sesji albo trwale dla konta. |
| `volume` | `path` | brak | Tylko dziennik woluminu, który zawiera tę ścieżkę. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `journal_state`, `effect` |

Bez `steps`, `age` i `size` wyjściem jest jeden rekord `journal_state` na
wybrany wolumin, jak z `journal.status`. W przeciwnym razie wyjściem jest jeden
rekord `effect` rodzaju `journal.limits`: `after.applied` podaje, czy nowe
limity zostały ustawione, `after.expiring` wymienia wpisy, które wygasają (albo
wygasłyby z `preview`), `after.expiring_bytes` podaje ich rozmiar logiczny, a
`after.volumes` wymienia obowiązujące limity każdego wybranego woluminu wraz z
ich źródłami.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Przywrócenie poprzednich limitów nie przywraca wpisów, które wygasły przy nowych limitach; bez parametrów polecenie tylko odczytuje. |

Limity cofa się, ustawiając je ponownie. Wpisów, które wygasły przy nowych
limitach, nie można przywrócić; wynik zawiera wtedy znacznik nieodwracalności
`journal_entries_expired`. Bez parametrów polecenie tylko odczytuje.

## Przykłady {#examples}

Pokaż limity każdego woluminu i poziomy, które je ustawiły:

```nurt example
journal.limits
```

Zachowaj ostatnie 10 operacji przez pięć minut w tej sesji:

```nurt example
journal.limits steps: 10 age: 5m
```

Zachowaj ostatnie 100 operacji przez jedną dobę w tej sesji:

```nurt example
journal.limits steps: 100 age: 24h
```

Ogranicz dziennik na woluminie projektu do jednej dziesiątej jego pojemności,
trwale dla konta:

```nurt example
journal.limits size: "10%" scope: user volume: "/home/user/project"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | wartość jest poza zakresem albo `scope: session` jest użyte przez żądanie bez sesji | użyj wartości z zakresu; ustaw opcję żądania `session` albo użyj `scope: user` |
| `E_NOT_FOUND` | żaden wolumin dziennika nie zawiera `volume` | pomiń `volume` albo wskaż ścieżkę na woluminie z dziennikiem |
| `E_CONFIG` | plik konfiguracji użytkownika nie jest prawidłowym TOML ani UTF-8 | popraw plik, a następnie powtórz wywołanie |
| `E_UNSUPPORTED` | `scope: user` i żaden plik konfiguracji użytkownika nie jest skonfigurowany dla dziennika | skonfiguruj lokalizację pliku konfiguracji użytkownika albo użyj `scope: session` |
| `E_IO` | nie można zapisać pliku dziennika ani konfiguracji | sprawdź uprawnienia katalogu stanu |

## Powiązane {#related}

`journal.status`, `journal.purge`, `journal.pin`, `journal.space`.
