---
command: power.restart
group: administration
summary: Uruchamia system ponownie teraz lub o wskazanej godzinie.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# power.restart

## Składnia {#synopsis}

```text
power.restart [name: value]...
```

## Opis {#description}

`power.restart` uruchamia system ponownie za pomocą `shutdown`. Bez `at` i `delay` akcja jest wykonywana natychmiast i jest nieodwracalna; raport informuje o tym przed zmianą. Z `at` (chwila, przeliczana na opóźnienie według zegara silnika) lub `delay` (zaokrąglane w górę do pełnych minut w systemach Linux i macOS) akcja jest planowana, a `power.cancel` ją cofa. Drugi harmonogram, gdy jeden już istnieje, daje w systemie Linux `E_IN_USE`. `message` jest pokazywane zalogowanym użytkownikom.

Uprawnienia: polecenie przyjmuje `privilege`; wpisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je po podniesieniu uprawnień. Wpis z podniesionymi uprawnieniami jest cofany przez ten sam proces z podniesionymi uprawnieniami.

Cofnięcie (`undo`) zaplanowanej akcji uruchamia `power.cancel`; akcji natychmiastowej nie można cofnąć.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `at` | `instant` | brak | Czas ponownego uruchomienia; przeliczany na opóźnienie według zegara silnika. |
| `delay` | `duration` | brak | Czas do ponownego uruchomienia; zaokrąglany w górę do pełnych minut w systemach Linux i macOS. |
| `message` | `text` | brak | Komunikat pokazywany zalogowanym użytkownikom. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `system_program` |
| Warunki odwracalności | Zaplanowane ponowne uruchomienie jest cofane przez power.cancel; ponowne uruchomienie bez at lub delay jest wykonywane natychmiast i jest nieodwracalne. |
| External use | `system_program`: `shutdown`; System jest uruchamiany ponownie za pomocą programu shutdown. |

## Przykłady {#examples}

Zaplanowanie za dziesięć minut.

```nurt example
power.restart delay: 10m message: "Maintenance" privilege: elevated
```

Zaplanowanie na wskazaną godzinę.

```nurt example
power.restart at: "2026-12-01T03:00:00Z" privilege: elevated
```

Uruchomienie akcji natychmiastowej z podniesionymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
power.restart privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_IN_USE` | Ponowne uruchomienie lub wyłączenie jest już zaplanowane. | Anuluj je poleceniem `power.cancel`. |
| `E_INVALID_VALUE` | `at` jest w przeszłości. | Podaj późniejszy czas. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wymieniony w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`power.cancel`, `power.restart`, `power.shutdown`.
