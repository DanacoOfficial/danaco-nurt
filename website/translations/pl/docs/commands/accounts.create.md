---
command: accounts.create
group: administration
summary: Tworzy konto użytkownika.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts.create

## Składnia {#synopsis}

```text
accounts.create <name> [name: value]...
```

## Opis {#description}

`accounts.create` tworzy konto użytkownika za pomocą `useradd` w systemie Linux, `sysadminctl` w systemie macOS i `net user` w systemie Windows. Konto jest najpierw odczytywane: istniejąca nazwa daje błąd `E_EXISTS`. Parametr `groups` wymienia grupy inne niż podstawowa, a `create_home: false` lub `system: true` pomija katalog domowy. Raport zapisuje stan po zmianie (`uid`, `shell`, `home`, `groups`, `comment`, `locked`) jako `after` celu `account:<name>`.

Uprawnienia: polecenie przyjmuje parametr `privilege`; należy podać `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` wykonuje `accounts.remove name: "<name>" keep_home: true`; katalog domowy zostaje zachowany.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa konta. |
| `uid` | `int` | brak | Identyfikator liczbowy; system wybiera go, gdy zostanie pominięty. |
| `shell` | `text` | brak | Powłoka logowania. |
| `home` | `text` | brak | Katalog domowy. |
| `groups` | `list<text>` | brak | Grupy inne niż podstawowa. |
| `comment` | `text` | brak | Komentarz lub imię i nazwisko. |
| `system` | `bool` | `false` | Tworzy konto systemowe bez katalogu domowego. |
| `create_home` | `bool` | `true` | Tworzy katalog domowy. |

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
| Warunki odwracalności | Cofnięcie usuwa konto i zachowuje jego katalog domowy. |
| External use | `system_program`: `useradd`, `sysadminctl`, `net`; Konta są tworzone za pomocą useradd w systemie Linux, sysadminctl w systemie macOS i net user w systemie Windows. |

## Przykłady {#examples}

Utworzenie konta z powłoką i grupą.

```nurt example
accounts.create "deploy" shell: "/bin/bash" groups: ["docker"] privilege: elevated
```

Utworzenie konta systemowego bez katalogu domowego.

```nurt example
accounts.create "svc-backup" system: true privilege: elevated
```

Utworzenie konta ze stałym identyfikatorem i komentarzem.

```nurt example
accounts.create "ann" uid: 2001 comment: "Ann Smith" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXISTS` | Konto istnieje. | Użyj `accounts.modify`. |
| `E_INVALID_VALUE` | Nazwa lub tekst zaczyna się od `-`, jest pusty albo zawiera znak sterujący, `:` lub `/`. | Podaj poprawną nazwę. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`accounts.modify`, `accounts.remove`, `groups.create`, `accounts`.
