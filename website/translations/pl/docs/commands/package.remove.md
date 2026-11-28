---
command: package.remove
group: package
summary: Usuwa zainstalowane pakiety.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# package.remove

## Składnia {#synopsis}

```text
package.remove <names>... [name: value]...
```

## Opis {#description}

`package.remove` usuwa wskazane pakiety za pomocą programu menedżera pakietów. Znak [P] w katalogu oznacza to użycie programu zewnętrznego. Program jest uruchamiany bez powłoki, z argumentami w postaci listy oraz ze zmiennymi `run.env_defaults`, po których następują zmienne menedżera zapewniające brak interakcji. Podczas podglądu żaden program nie jest uruchamiany.

Pliki konfiguracyjne są obsługiwane według wartości domyślnego zachowania menedżera. Zależności, które nie są już potrzebne, pozostają zainstalowane.

Polecenie wymaga uprawnienia menedżera. apt, dnf, pacman i zypper wymagają superużytkownika i w przeciwnym razie kończą się błędem `E_PRIVILEGE`; z `privilege: elevated` wywołanie jest uruchamiane przez mechanizm podniesienia uprawnień (`sudo`, `doas`, `run0` albo w tym samym procesie, gdy Nurt działa jako administrator), a samo polecenie nigdy nie zapisuje `sudo`. Homebrew odmawia uruchomienia jako superużytkownik i w takim przypadku kończy się błędem `E_PRIVILEGE`. Gdy blokada bazy pakietów jest utrzymywana, polecenie kończy się błędem `E_IN_USE` i wymienia procesy, które ją trzymają.

Niezerowy kod wyjścia menedżera kończy polecenie błędem `E_STAGE_FAILED` wraz z końcem jego wyjścia błędów. Skutek jest w takim przypadku zgłaszany jako wykonany, ponieważ menedżer mógł zmienić pakiety, zanim zawiódł.

**Uprawnienia.** apt, dnf, pacman i zypper wymagają administratora; wpisz `privilege: elevated` na etapie.

```nurt
package.remove ["jq"] privilege: elevated
```

Cofnięcie (`undo`) wpisu ponownie instaluje zapisaną wersję, o ile repozytorium nadal ją oferuje.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `names` | `list<text>` | wymagany | Pakiety do usunięcia. |
| `manager` | `enum(auto\|apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop)` | `auto` | Menedżer pakietów; auto wybiera menedżera systemu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

Rekord każdego pakietu ma `target` `package:<name>`, `action`, `before` (zainstalowana wersja, nieobecne, gdy pakiet nie był zainstalowany) i `after` (wersja odczytana z bazy pakietów po uruchomieniu).

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `package_manager` |
| Warunki odwracalności | Cofnięcie uruchamia operację odwrotną: package.install zapisanej wersji; wersja, której repozytorium już nie oferuje, pozostaje nieodwracalna. |

## Przykłady {#examples}

Usunięcie jednego pakietu.

```nurt example
package.remove "nano"
```

Usunięcie kilku pakietów za pomocą Homebrew.

```nurt example
package.remove "wget" "tree" manager: brew
```

Usunięcie dwóch pakietów w jednym wywołaniu.

```nurt example
package.remove "vim" "nano"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXTERNAL_MISSING` | Program menedżera nie jest zainstalowany. | Zainstaluj menedżera albo przekaż inny `manager`. |
| `E_PRIVILEGE` | Użytkownik nie ma uprawnienia menedżera albo menedżer odmawia superużytkownika. | Uruchom jako root, a dla Homebrew jako zwykły użytkownik. |
| `E_IN_USE` | Baza pakietów jest zablokowana przez inny proces. | Poczekaj na zakończenie wymienionych procesów. |
| `E_UNSUPPORTED` | Menedżer nie może zastosować żądania, na przykład wybranej wersji w pacman. | Pomiń parametr. |
| `E_STAGE_FAILED` | Menedżer zakończył działanie błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`package.list`, `package.search`.
