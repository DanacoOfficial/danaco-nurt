---
command: accounts.remove
group: administration
summary: Usuwa konto użytkownika.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts.remove

## Składnia {#synopsis}

```text
accounts.remove <name> [name: value]...
```

## Opis {#description}

`accounts.remove` usuwa konto za pomocą `userdel` w systemie Linux, `sysadminctl` w systemie macOS i `net user` w systemie Windows. Katalog domowy nie jest usuwany przez program systemowy: jeżeli `keep_home` nie ma wartości true, zostaje usunięty za pośrednictwem dziennika, więc `undo` przywraca go wraz z zawartością. Konto o identyfikatorze 0 nigdy nie jest usuwane. Raport zapisuje cały stan konta sprzed zmiany.

Uprawnienia: polecenie przyjmuje parametr `privilege`; należy podać `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` wykonuje `accounts.create` z zapisanymi identyfikatorem, powłoką, katalogiem domowym, grupami i komentarzem (`create_home: false`), a następnie `accounts.modify` dla konta zablokowanego; katalog domowy przywraca dziennik.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa konta. |
| `keep_home` | `bool` | `false` | Zachowuje katalog domowy; w przeciwnym razie jest on usuwany za pośrednictwem dziennika. |

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
| Warunki odwracalności | Cofnięcie tworzy konto ponownie z jego identyfikatorem, powłoką, grupami i komentarzem; katalog domowy wraca wraz z dziennikiem. |
| External use | `system_program`: `userdel`, `sysadminctl`, `net`; Konta są usuwane za pomocą userdel w systemie Linux, sysadminctl w systemie macOS i net user w systemie Windows. |

## Przykłady {#examples}

Usunięcie konta wraz z jego katalogiem domowym.

```nurt example
accounts.remove "olduser" privilege: elevated
```

Usunięcie konta z zachowaniem katalogu domowego.

```nurt example
accounts.remove "olduser" keep_home: true privilege: elevated
```

Wykonanie usunięcia z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
accounts.remove "olduser" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Konto nie istnieje. | Sprawdź nazwę za pomocą `accounts`. |
| `E_INVALID_VALUE` | Konto jest kontem administratora głównego. | Konta administratora głównego nie można usunąć. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`accounts.create`, `accounts.modify`, `accounts`.
