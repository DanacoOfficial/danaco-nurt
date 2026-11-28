---
command: git.fetch
group: git
summary: Pobiera obiekty i odwołania z repozytorium zdalnego przez HTTPS lub SSH.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.fetch

## Składnia {#synopsis}

```text
git.fetch <repo> [name: value]...
```

## Opis {#description}

Pobiera obiekty i odwołania z repozytorium zdalnego do repozytorium, tak jak
robi to `git fetch`.

`remote` to nazwa skonfigurowanego repozytorium zdalnego albo adres. Nazwa
wybiera specyfikacje odwołań z `remote.<name>.fetch`; adres pobiera `HEAD`
repozytorium zdalnego tylko do `FETCH_HEAD`. Repozytorium zdalne jest osiągane
przez ścieżkę lokalną lub adres `file://`, smart HTTP (`http://`, `https://`)
albo SSH (`ssh://`, `user@host:path`) po przekształceniach `url.<base>.insteadOf`.
Używana jest wersja 2 protokołu, chyba że `protocol.version` ma wartość `0`
lub `1`.

Dla każdego odwołania zdalnego wybranego przez specyfikację odwołań odwołanie
lokalne jest tworzone, przesuwane w trybie fast-forward, a gdy specyfikacja
zaczyna się od `+`, przesuwane bez sprawdzania fast-forward. Odwołanie, które
przesunęłoby się bez wymuszenia, jest odrzucane i zgłaszane ze stanem
`rejected`. Gałąź wyewidencjonowana w drzewie roboczym nigdy nie jest
aktualizowana przez specyfikację odwołań.

Tagi wskazujące na pobraną historię są pobierane, chyba że
`remote.<name>.tagOpt` ma wartość `--no-tags`. Z `tags: true` pobierany jest
każdy tag repozytorium zdalnego. Z `prune: true` albo gdy ustawiono
`fetch.prune` lub `remote.<name>.prune`, usuwane są odwołania lokalne pasujące
do prawej strony specyfikacji odwołań, które już nie istnieją w repozytorium
zdalnym.

`depth` ogranicza pobraną historię do tylu rewizji od każdego pobranego końca
gałęzi i zapisuje granicę w `shallow`; większa głębokość pogłębia płytkie
repozytorium (shallow).

`FETCH_HEAD` jest zapisywany w formacie `git`: jeden wiersz na pobrane
odwołanie, przy czym głowice, które pull scala w pierwszej kolejności, są na
początku, a pozostałe są oznaczone `not-for-merge`. Komunikat reflogu każdego
zaktualizowanego odwołania to `fetch <remote>: fast-forward`,
`fetch <remote>: forced-update` lub `fetch <remote>: storing head`.

Poświadczenia dla HTTP są pobierane z adresu, ze zmiennych
`NURT_GIT_TOKEN_<HOST>` i `NURT_GIT_TOKEN`, ze zmiennych dostawców hostingu albo
z magazynów poświadczeń. Pomocniki poświadczeń będące programami nie są
uruchamiane i są zgłaszane jako `credential_helper_ignored`. Hasło w adresie
jest używane do transferu i nigdy nie jest zapisywane w wynikach.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `remote` | `text` | `"origin"` | Repozytorium zdalne, z którego pobieramy. |
| `prune` | `bool` | `false` | Usuwa odwołania śledzące repozytorium zdalne, które już w nim nie istnieją. |
| `tags` | `bool` | `false` | Pobiera wszystkie tagi repozytorium zdalnego. |
| `depth` | `int` | brak | Ogranicza pobraną historię do tylu rewizji. |
| `accept_host_key` | `text` | brak | Odcisk SHA-256 (SHA256:<base64>) klucza hosta SSH do zaakceptowania, gdy host nie jest znany. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect`, `git_ref_update` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Pobranie skonfigurowanych specyfikacji odwołań `origin`.

```nurt example
git.fetch "/home/user/app"
```

```text expected
effect  git.fetch  ok  remote:origin  state: completed
```

Pobranie wszystkich tagów i usunięcie odwołań śledzących usunięte gałęzie
repozytorium zdalnego.

```nurt example
git.fetch "/home/user/app" remote: "upstream" prune: true tags: true
```

Pogłębienie płytkiego repozytorium do 50 rewizji.

```nurt example
git.fetch "/home/user/app" depth: 50
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium. | Podaj ścieżkę repozytorium. |
| `E_GIT_REF` | Repozytorium zdalne nie jest skonfigurowane (`unknown_remote`), specyfikacja odwołań niczego nie wybiera (`no_match`) albo specyfikacja odwołań zaktualizowałaby wyewidencjonowaną gałąź (`checked_out`). | `git.branch "<repo>" action: list` |
| `E_INVALID_VALUE` | `depth` wynosi 0 albo specyfikacja odwołań jest nieprawidłowa. | Przekaż `depth` równe 1 lub więcej. |
| `E_GIT_LOCKED` | Istnieje plik blokady odwołania. | Usuń nieaktualny plik `.lock` po sprawdzeniu, że nie działa żaden proces Git. |
| `E_GIT_AUTH` | Repozytorium zdalne wymaga poświadczeń, których brakuje lub które zostały odrzucone. | Ustaw `NURT_GIT_TOKEN_<HOST>` i użyj adresu `https://`. |
| `E_GIT_HOST_KEY` | Klucz hosta SSH jest nieznany lub się zmienił. | Przekaż `accept_host_key` z odciskiem hosta. |
| `E_GIT_PROTOCOL` | Repozytorium zdalne odpowiedziało niezgodnie z protokołem albo jego format obiektów jest inny. | Sprawdź, czy adres wskazuje repozytorium Git. |
| `E_NETWORK` | Połączenie nie powiodło się; problem to `dns`, `connect`, `tls`, `timeout`, `proxy`, `http_status` lub `insecure_credentials` (nagłówek `Authorization` albo token zostałby wysłany przez `http://` do hosta innego niż ta maszyna). | Sprawdź adres, ustawienia proxy i sieć; użyj adresu `https://`. |
| `E_CANCELLED` | Transfer został przerwany; żadne odwołanie nie zostało zmienione. | Powtórz wywołanie. |
| `E_TIMEOUT` | Transfer został przerwany; żadne odwołanie nie zostało zmienione. | Powtórz wywołanie. |

## Powiązane {#related}

`git.pull`, `git.push`, `git.branch`, `git.clone`, `undo`.
