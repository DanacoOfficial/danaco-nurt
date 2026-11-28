---
command: accounts.modify
group: administration
summary: Zmienia powłokę, katalog domowy, grupy, komentarz lub stan blokady konta.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# accounts.modify

## Składnia {#synopsis}

```text
accounts.modify <name> [name: value]...
```

## Opis {#description}

`accounts.modify` zmienia powłokę logowania, katalog domowy, grupy, komentarz lub stan blokady konta. Parametr `groups` jest pełną listą grup innych niż podstawowa. Wywołanie, które podaje wartości już ustawione na koncie, niczego nie zmienia i zgłasza `unchanged`. Raport zapisuje cały stan przed zmianą i po niej, dzięki czemu `undo` przywraca pola, które uległy zmianie.

Uprawnienia: polecenie przyjmuje parametr `privilege`; należy podać `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` wykonuje `accounts.modify` z zapisanymi wartościami pól, które uległy zmianie.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa konta. |
| `shell` | `text` | brak | Nowa powłoka logowania. |
| `home` | `text` | brak | Nowy katalog domowy. |
| `groups` | `list<text>` | brak | Pełna lista grup innych niż podstawowa. |
| `comment` | `text` | brak | Nowy komentarz lub imię i nazwisko. |
| `lock` | `bool` | brak | Blokuje konto przy wartości true i odblokowuje je przy wartości false. |

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
| Warunki odwracalności | Cofnięcie przywraca zapisane wartości pól, które uległy zmianie. |
| External use | `system_program`: `usermod`, `dscl`, `net`; Konta są zmieniane za pomocą usermod w systemie Linux, dscl w systemie macOS i net user w systemie Windows. |

## Przykłady {#examples}

Zmiana powłoki konta.

```nurt example
accounts.modify "deploy" shell: "/bin/zsh" privilege: elevated
```

Zablokowanie konta.

```nurt example
accounts.modify "ann" lock: true privilege: elevated
```

Zastąpienie grup konta.

```nurt example
accounts.modify "deploy" groups: ["docker", "wheel"] comment: "Deploy user" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Konto nie istnieje. | Utwórz je za pomocą `accounts.create`. |
| `E_MISSING_PARAM` | Nie podano żadnego pola do zmiany. | Podaj co najmniej jedno z pól `shell`, `home`, `groups`, `comment`, `lock`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`accounts.create`, `accounts.remove`, `accounts`.
