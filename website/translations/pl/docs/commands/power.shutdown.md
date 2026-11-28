---
command: power.shutdown
group: administration
summary: Wyłącza system natychmiast lub o podanej godzinie.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# power.shutdown

## Składnia {#synopsis}

```text
power.shutdown [name: value]...
```

## Opis {#description}

`power.shutdown` wyłącza system za pomocą `shutdown`. Bez `at` i `delay` operacja jest wykonywana natychmiast i jest nieodwracalna; raport informuje o tym przed zmianą. Z `at` (chwila czasu, przeliczana na opóźnienie według zegara silnika) lub `delay` (zaokrąglane w górę do pełnych minut w systemach Linux i macOS) operacja jest planowana, a `power.cancel` ją cofa. Drugi harmonogram, gdy jeden już istnieje, kończy się w systemie Linux błędem `E_IN_USE`. `message` jest wyświetlany zalogowanym użytkownikom.

Uprawnienia: polecenie przyjmuje `privilege`; wpisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

Cofnięcie (`undo`) zaplanowanej operacji uruchamia `power.cancel`; operacji natychmiastowej nie można cofnąć.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `at` | `instant` | none | Czas wyłączenia; jest przeliczany na opóźnienie według zegara silnika. |
| `delay` | `duration` | none | Czas do wyłączenia; w systemach Linux i macOS zaokrąglany w górę do pełnych minut. |
| `message` | `text` | none | Komunikat wyświetlany zalogowanym użytkownikom. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `system_program` |
| Warunki odwracalności | Zaplanowane wyłączenie jest cofane przez power.cancel; wyłączenie bez at lub delay jest wykonywane natychmiast i jest nieodwracalne. |
| External use | `system_program`: `shutdown`; System jest wyłączany programem shutdown. |

## Przykłady {#examples}

Zaplanowanie za dziesięć minut.

```nurt example
power.shutdown delay: 10m message: "Maintenance" privilege: elevated
```

Zaplanowanie na określoną godzinę.

```nurt example
power.shutdown at: "2026-12-01T03:00:00Z" privilege: elevated
```

Wykonanie operacji natychmiastowej z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` pokazuje najpierw skutek.

```nurt example
power.shutdown privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_IN_USE` | Ponowne uruchomienie lub wyłączenie jest już zaplanowane. | Anuluj je poleceniem `power.cancel`. |
| `E_INVALID_VALUE` | `at` wskazuje przeszłość. | Podaj późniejszy czas. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`power.cancel`, `power.restart`, `power.shutdown`.
