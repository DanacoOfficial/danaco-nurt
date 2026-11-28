---
command: undo
group: core
summary: Cofa operacje z dziennika.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# undo

## Składnia {#synopsis}

```text
undo [<id>] [name: value]...
```

## Opis {#description}

`undo` przywraca stan sprzed jednej lub kilku operacji zapisanych w
dzienniku. Naprawia pomyłkę; nie zastępuje sprawdzenia wyniku operacji, zanim
się na nim oprze.

Dokładnie jedna forma wybiera operacje:

| Forma | Cofane operacje | Zapisany zakres |
|---|---|---|
| brak wyboru | najnowsza dostępna operacja sesji (konta bez sesji), która sama nie jest cofnięciem | `op` |
| `id` | operacja `id`, wszystkie jej elementy; operacja zbiorcza jest cofana w całości | `op` |
| `to` | każda dostępna operacja nowsza niż `to` w sesji, od najnowszej, jako jedna transakcja | `to` |
| `file` | każda dostępna operacja, która zmieniła plik, nowsza niż `since` albo wszystkie, tylko dla tego pliku; plik wraca do stanu sprzed najstarszej z nich | `file` |
| `id` z `paths` | tylko wskazane elementy operacji `id` | `paths` |

Cofnięcie cofnięcia jest stosowane ponownie za pomocą `redo`; domyślny wybór
nigdy nie wybiera cofnięcia. Operacje innych sesji w zakresie `to` są
pomijane i wymieniane w `skipped`, chyba że `all_sessions` ma wartość true.

Raporty nazywają operację `op`, natomiast parametr, który ją wybiera, to `id`.
Wywołanie zapisane jako `undo op: N` lub `undo operation: N` jest
poprawiane na `undo id: N` (reguła L06) i wykonywane; poprawka jest
wymieniana w wyniku.

Zanim cokolwiek zostanie zapisane, każdy element całego wyboru jest
porównywany ze stanem zapisanym w dzienniku po operacji: skrót zawartości,
rodzaj, rozmiar i bity uprawnień, brak usuniętych wpisów, zawartość
zachowana dla nadpisanych i usuniętych plików oraz późniejsze operacje, które
zmieniły te same elementy. Różnica niczego nie zmienia i nie zużywa numeru
operacji; zamiast tego o dalszym przebiegu decyduje `on_conflict`:

| `on_conflict` | Zachowanie |
|---|---|
| `report` | kończy się błędem `E_JOURNAL_CONFLICT` wymieniającym każdą różnicę; kończy się błędem `E_JOURNAL_PARTIAL`, gdy operacja ma elementy zwolnione dla miejsca |
| `force` | cofa mimo to; bieżąca zawartość każdego elementu jest zachowana w nowym wpisie dziennika; różnice są wymienione w `forced`; zwolnione elementy są pomijane jak przy `partial` |
| `include_later` | cofa także, od najnowszej, późniejsze operacje, które zmieniły te same elementy, oraz ich późniejsze operacje, aż nie pozostanie żadna późniejsza zmiana; kończy się błędem `E_JOURNAL_EXPIRED`, gdy któraś z nich wygasła; z `to` zachowuje się jak `report`, ponieważ zakres jest już cofany od najnowszej |
| `partial` | cofa pozostałe elementy operacji z elementami zwolnionymi dla miejsca i wymienia zwolnione elementy w `unavailable`; różnice są zgłaszane jak przy `report` |
| `abort` | kończy się błędem `E_JOURNAL_CONFLICT` zgłaszającym tylko liczbę różnic albo błędem `E_JOURNAL_PARTIAL` |

Operacja jest w stanie `partial`, gdy niektóre jej elementy zostały zwolnione
w celu odzyskania miejsca na dysku albo zostały cofnięte pojedynczo za pomocą
`file` lub `paths`; elementy nadal zachowane pozostają możliwe do cofnięcia.
Elementy cofnięte pojedynczo są przywracane tylko raz; operacja, której
wszystkie elementy są cofnięte, otrzymuje stan `undone`.

W zakresie `to` lub `file` cofnięcie, którego operacja leży w tym samym
zakresie, znosi ją i żadne z nich nie jest cofane; `redo` takiego cofnięcia
jest cofane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `int` | najnowsza operacja | Operacja do cofnięcia. |
| `to` | `int` | brak | Cofa każdą operację nowszą niż ta, od najnowszej, w jednej transakcji. |
| `file` | `path` | brak | Cofa każdą zmianę tego pliku od podanego punktu. |
| `since` | `int \| instant` | brak | Operacja lub czas, od którego zmiany pliku są cofane. |
| `paths` | `list<path>` | brak | Cofa tylko zmiany tych ścieżek w obrębie operacji. |
| `all_sessions` | `bool` | `false` | Obejmuje operacje innych sesji; tylko z to. |
| `on_conflict` | `enum(report\|force\|include_later\|partial\|abort)` | `report` | Zachowanie, gdy późniejsze zmiany dotykają tych samych plików. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `effect` |

Podgląd `undo id` i `undo id paths:` wylicza elementy, które cofnięcie
przywróci; pozostałe formy mają podgląd bez elementów.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Cofnięcie można odwrócić za pomocą redo. |

## Przykłady {#examples}

Cofnięcie najnowszej operacji sesji:

```nurt example
undo
```

Cofnięcie operacji 18 wraz z późniejszymi operacjami, które zmieniły te same
pliki:

```nurt example
undo 18 on_conflict: include_later
```

Przywrócenie pliku do stanu sprzed operacji 12:

```nurt example
undo file: "/home/user/app/config.toml" since: 12
```

Cofnięcie dwóch elementów usunięcia zbiorczego:

```nurt example
undo 21 paths: ["/home/user/app/a.go", "/home/user/app/b.go"]
```

Cofnięcie wszystkiego, co zrobiono po operacji 30 w tej sesji:

```nurt example
undo to: 30
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_JOURNAL_NOOP` | operacja nie istnieje, jest już cofnięta lub nadal trwa, ścieżka nie jest elementem operacji albo nie ma nic do cofnięcia | wylistuj operacje za pomocą `history` |
| `E_INVALID_VALUE` | argumenty mieszają formy: więcej niż jedno z `id`, `to` i `file`, `since` bez `file`, `paths` bez `id`, puste `paths` albo `all_sessions` bez `to` | użyj jednej formy ze składni |
| `E_JOURNAL_EXPIRED` | operacja wyboru wygasła; nic się nie zmieniło; `expired` je wylicza | wybierz późniejsze operacje |
| `E_JOURNAL_CONFLICT` | elementy zmieniły się po operacji; nic się nie zmieniło; `conflicts` wylicza każdą różnicę | użyj jednego z programów w `fix` albo `on_conflict: force` lub `include_later` |
| `E_JOURNAL_PARTIAL` | elementy operacji zostały zwolnione dla miejsca; nic się nie zmieniło | `on_conflict: partial` |
| `E_JOURNAL_CORRUPT` | operacja nie przeszła kontroli integralności; jej dane są zachowane do ratowania | `nurt journal salvage` |
| `E_UNSUPPORTED` | dziennik nie może przywrócić elementu (zmiana nazwy między woluminami lub nad istniejącym wpisem); nic się nie zmieniło | przywróć element z danych wymienionych przez `history N details: true` albo cofnij pozostałe elementy za pomocą `paths` |
| `E_STALE` | element zmienił się podczas cofania; cofnięcie zostało wycofane | powtórz cofnięcie |
| `E_ROLLBACK_FAILED` | cofnięcie, które zawiodło, nie mogło przywrócić każdego elementu; `failed` je wylicza | przywróć wymienione ścieżki |

## Powiązane {#related}

`redo`, `history`, `journal.pin`, `journal.status`, `help skill: "journal"`.
