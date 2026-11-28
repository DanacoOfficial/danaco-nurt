---
group: package
summary: Systemowe menedżery pakietów.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Grupa package

## Zakres {#scope}

Grupa wylistowuje, wyszukuje, instaluje, usuwa i aktualizuje pakiety za
pomocą menedżera pakietów systemu i zwraca wynik jako rekordy `package`.
Polecenia nie zastępują menedżera pakietów: to on wykonuje każdą operację, a
grupa nadaje jego wyjściu postać typowaną i zapisuje zmianę w dzienniku
operacji.

| Zadanie | Polecenie | Grupa |
|---|---|---|
| Znalezienie zainstalowanego programu według nazwy | `program.find` | program |
| Opis zainstalowanego programu | `program.describe` | program |
| Wyszukanie programów ze wszystkich źródeł, w tym pakietów | `program.search` | program |
| Uruchomienie programu | `run` | core |

`package.search` odpytuje repozytoria menedżera pakietów. `program.search`
odczytuje bazy pakietów systemu i nie wymaga menedżera pakietów.

## Polecenia {#commands}

| Polecenie | Kategoria | Wejście i wyjście | Odwracalność | Strona |
|---|---|---|---|---|
| `package.list` | source | brak do `package` | read | [package.list](../commands/package.list.md) |
| `package.search` | source | brak do `package` | read | [package.search](../commands/package.search.md) |
| `package.install` | change | brak do `effect` | reversible_if_preserved | [package.install](../commands/package.install.md) |
| `package.remove` | change | brak do `effect` | reversible_if_preserved | [package.remove](../commands/package.remove.md) |
| `package.upgrade` | change | brak do `effect` | reversible_if_preserved | [package.upgrade](../commands/package.upgrade.md) |

## Rekordy {#records}

| Rekord | Wymagane pola |
|---|---|
| `package` | `name`, `version`, `manager`, `installed` |
| `effect` | `status`, `kind` |

`package.version` to wersja zainstalowana albo wersja oferowana w wyniku
wyszukiwania. Przy `outdated: true` pole `available_version` zawiera nowszą
wersję. Pola opcjonalne to `description`, `architecture`, `size`, `source` i
`files_count`.

## Menedżery pakietów {#package-managers}

Parametr `manager` wybiera menedżer pakietów. Wartością domyślną jest `auto`:
menedżer używany przez hosta jest wykrywany na podstawie systemu operacyjnego
i programów znalezionych w ścieżce wyszukiwania.

| Wartość | System |
|---|---|
| `apt` | Debian i Ubuntu |
| `dnf` | Fedora i rodzina Red Hat |
| `pacman` | Arch Linux |
| `zypper` | openSUSE i SUSE |
| `brew` | macOS i Linux z Homebrew |
| `winget` | Windows |
| `choco` | Windows z Chocolatey |
| `scoop` | Windows ze Scoop |

Pole rekordu `manager` przyjmuje także `dpkg`, `rpm` i `other` dla pakietów,
które menedżer zgłasza z innym pochodzeniem. Wartość `manager`, której host
nie ma, kończy się błędem `E_EXTERNAL_MISSING`.

Instalacja, usunięcie i aktualizacja zmieniają system poza systemem plików.
Dziennik operacji zapisuje stan przed i po dla każdego pakietu, a `undo`
uruchamia operację odwrotną: `package.remove` dla pakietu zainstalowanego
przez polecenie, `package.install` zapisanej wersji dla pakietu usuniętego
lub zaktualizowanego. Wersja, której repozytorium już nie oferuje, pozostaje
nieodwracalna. Katalog klasyfikuje te trzy polecenia jako
`reversible_if_preserved`; klasyfikacja wskazuje warunek, a operacja odwrotna
jest tym, co uruchamia `undo`.

## Różnice między platformami {#platform-differences}

| Aspekt | Linux | macOS | Windows |
|---|---|---|---|
| Menedżery | `apt`, `dnf`, `pacman`, `zypper`, `brew` | `brew` | `winget`, `choco`, `scoop` |
| Uprawnienie dla `install`, `remove`, `upgrade` | uprawnienie wymagane przez menedżer | uprawnienie wymagane przez menedżer | uprawnienie wymagane przez menedżer |
| Brak uprawnienia | `E_PRIVILEGE` | `E_PRIVILEGE` | `E_PRIVILEGE` |

Polecenie, którego menedżer nie jest zainstalowany, kończy się błędem
`E_EXTERNAL_MISSING`, a `fix` pokazuje, jak go zainstalować.

## Programy zewnętrzne {#external-programs}

Katalog oznacza implementację każdego polecenia grupy jako program menedżera
pakietów [P]:

| Polecenie | Znacznik | Program |
|---|---|---|
| `package.list` | [P] | wybrany menedżer |
| `package.search` | [P] | wybrany menedżer |
| `package.install` | [P] | wybrany menedżer |
| `package.remove` | [P] | wybrany menedżer |
| `package.upgrade` | [P] | wybrany menedżer |

Sprawdzenie programu zgłasza program przed wykonaniem.

## Przykłady {#examples}

Wylistowanie zainstalowanych pakietów o nazwie `git`:

```nurt example
package.list name: "git"
```

Zliczenie pakietów, które mają nowszą wersję:

```nurt example
package.list outdated: true |> count
```

Pokazanie zainstalowanej i dostępnej wersji nieaktualnych pakietów:

```nurt example
package.list outdated: true |> select name version available_version
```

Przeszukanie repozytoriów menedżera:

```nurt example
package.search "ripgrep"
```

Instalacja dwóch pakietów menedżerem `apt`:

```nurt example
package.install ["git", "curl"] manager: apt
```

Aktualizacja jednego pakietu:

```nurt example
package.upgrade ["openssl"]
```

## Powiązane {#related}

[Grupa program](program.md) opisuje zainstalowane programy bez menedżera
pakietów. [Grupa system](system.md) zgłasza hosta, na którym rozstrzyga
`manager: auto`.
