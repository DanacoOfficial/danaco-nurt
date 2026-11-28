---
command: accounts.password
group: administration
summary: Ustawia hasło konta.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts.password

## Składnia {#synopsis}

```text
accounts.password <name> <value>
```

## Opis {#description}

`accounts.password` ustawia hasło konta. `value` jest parametrem tajnym: jest maskowany w dzienniku, w raporcie i w wypisywanym drzewie, w systemie Linux jest zapisywany na standardowe wejście `chpasswd`, a w systemie Windows w zmiennej środowiskowej `Set-LocalUser`; nigdy nie pojawia się na liście argumentów programów systemu Linux. W systemie macOS `dscl -passwd` przyjmuje wartość jako argument. Poprzednie hasło nie jest zapisywane, więc zmiana jest nieodwracalna, a raport to zaznacza.

Uprawnienia: polecenie przyjmuje parametr `privilege`; należy podać `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

Hasło jest nieodwracalne: `undo` zgłasza, że nie istnieje operacja odwrotna.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa konta. |
| `value` | `text` | wymagany | Nowe hasło; maskowane w dzienniku i w raporcie. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `system_program` |
| Warunki odwracalności | Poprzednie hasło nie jest zapisywane. |
| External use | `system_program`: `chpasswd`, `dscl`, `powershell`; Hasło jest przekazywane do chpasswd na jego standardowym wejściu w systemie Linux, a w systemie Windows w zmiennej środowiskowej Set-LocalUser. |

## Przykłady {#examples}

Ustawienie hasła.

```nurt example
accounts.password "deploy" "correct horse battery" privilege: elevated
```

Ustawienie hasła z użyciem parametru nazwanego.

```nurt example
accounts.password name: "ann" value: "s3cret" privilege: elevated
```

Ustawienie hasła i sprawdzenie konta po zmianie.

```nurt example
accounts.password "ann" "s3cret" privilege: elevated |> select target
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Konto nie istnieje. | Utwórz je za pomocą `accounts.create`. |
| `E_INVALID_VALUE` | Hasło zawiera znak sterujący. | Użyj innego hasła. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`accounts.create`, `accounts.modify`.
