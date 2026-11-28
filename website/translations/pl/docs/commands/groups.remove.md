---
command: groups.remove
group: administration
summary: Usuwa grupę.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# groups.remove

## Składnia {#synopsis}

```text
groups.remove <name>
```

## Opis {#description}

`groups.remove` usuwa grupę. Identyfikator i członkowie są najpierw odczytywani i zapisywani. Grupa o identyfikatorze 0 nigdy nie jest usuwana.

Uprawnienia: polecenie akceptuje `privilege`; zapisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis wykonany z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` uruchamia `groups.create` z zapisanym identyfikatorem oraz `groups.add_member` dla każdego zapisanego członka.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa grupy. |

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
| Warunki odwracalności | Cofnięcie tworzy grupę ponownie z jej identyfikatorem i członkami. |
| External use | `system_program`: `groupdel`, `dseditgroup`, `net`; Grupy są usuwane przez groupdel w systemie Linux, dseditgroup w systemie macOS i net localgroup w systemie Windows. |

## Przykłady {#examples}

Usuń grupę.

```nurt example
groups.remove "ops" privilege: elevated
```

Usuń grupę i pokaż skutek.

```nurt example
groups.remove "ops" privilege: elevated |> select target before
```

Wykonaj usunięcie z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
groups.remove "ops" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Grupa nie istnieje. | Sprawdź nazwę poleceniem `accounts kind: group`. |
| `E_INVALID_VALUE` | Grupa ma identyfikator 0. | Grupy nie można usunąć. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`groups.create`, `groups.remove_member`.
