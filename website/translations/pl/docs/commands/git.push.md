---
command: git.push
group: git
summary: Aktualizuje odwołania w repozytorium zdalnym lokalnymi rewizjami.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.push

## Składnia {#synopsis}

```text
git.push <repo> branch: <text> [name: value]...
```

## Opis {#description}

Wysyła rewizje potrzebne repozytorium zdalnemu i aktualizuje jego odwołania
przez protokół send-pack (protokół w wersji 0 z `report-status-v2`,
`side-band-64k`, `atomic`, `push-options`, `ofs-delta`, `quiet`, `agent` i
`object-format`, gdy repozytorium zdalne je ogłasza).

`branch` jest specyfikacją odwołania (refspec):

- `main` wysyła lokalną gałąź lub tag do odwołania zdalnego o tej samej nazwie;
  `HEAD` wysyła bieżącą gałąź;
- `src:dst` wysyła wersję `src` do odwołania zdalnego `dst`; niekwalifikowane
  `dst` jest uzupełniane przedrostkiem `refs/heads/` lub `refs/tags/`, zależnie
  od tego, czego wymaga repozytorium zdalne lub źródło;
- `:dst` usuwa odwołanie zdalne;
- początkowy `+` wymusza aktualizację tej specyfikacji.

Przed żądaniem każda aktualizacja jest klasyfikowana tak, jak robi to
`git push`: aktualizacje `up_to_date` nie są wysyłane; nowe odwołanie to `new`;
aktualizacja, której stara wartość jest przodkiem nowej, to `updated`; każda
inna aktualizacja jest odrzucana jako `non_fast_forward` (stara rewizja jest
obecna lokalnie) lub `fetch_first` (nie jest), chyba że jest wymuszona. Tag
zdalny jest zastępowany tylko przez wymuszoną aktualizację
(`already_exists`). Z dzierżawą (lease) odwołanie zdalne musi nadal mieć
oczekiwaną wartość, w przeciwnym razie aktualizacja jest odrzucana jako
`stale_info`.

Wysyłane są obiekty osiągalne z nowych wartości i nieosiągalne z odwołań
repozytorium zdalnego. Wpisy zapisane jako delty w lokalnych paczkach są
kopiowane; nowe delty nie są obliczane.

Aktualizacje zaakceptowane przez repozytorium zdalne przesuwają odwołania
śledzące repozytorium zdalne wskazane przez `remote.<name>.fetch`; ich wpisy
reflogu mają postać `update by push`. Z `set_upstream` konfiguracja otrzymuje
`branch.<name>.remote` i `branch.<name>.merge`. Hak `pre-push` repozytorium jest
uruchamiany przed żądaniem; niezerowy kod zakończenia zatrzymuje push.

Repozytorium zdalne w lokalnym systemie plików (adres URL `file://` lub ścieżka)
jest obsługiwane w procesie: paczka jest zapisywana w `objects/pack` celu, nowe
końce gałęzi są sprawdzane pod kątem kompletności, obowiązują
`receive.denyCurrentBranch` (domyślnie `refuse` dla repozytorium z drzewem
roboczym), `receive.denyDeleteCurrent`, `receive.denyNonFastForwards`,
`receive.denyDeletes` i `receive.hideRefs`, a haki `pre-receive`, `update` i
`post-receive` celu są uruchamiane jako programy użytkownika. Wpisy reflogu celu
mają postać `push`.

Inne repozytoria zdalne są osiągane przez smart HTTP (`http://`, `https://`) lub
SSH (`ssh://`, `user@host:path`) po przekształceniach
`url.<base>.pushInsteadOf` i `url.<base>.insteadOf`. Obowiązują ustawienia
`http.*` repozytorium. Gdy serwer odpowiada `401`, poświadczenia dla adresu są
pobierane ze zmiennych `NURT_GIT_TOKEN_<HOST>` i `NURT_GIT_TOKEN`, ze zmiennych
dostawców hostingu albo z magazynów poświadczeń, a żądanie jest powtarzane
jednokrotnie; poświadczenia nigdy nie są wysyłane przez `http://`. Pomocniki
poświadczeń będące programami nie są uruchamiane. Transporty `git://` i `ext::`
uruchamiają programy i nie są obsługiwane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `branch` | `text` | wymagany | Gałąź do wysłania. |
| `remote` | `text` | `"origin"` | Repozytorium zdalne, do którego wysyłamy. |
| `force` | `bool \| enum(no\|lease\|yes)` | `false` | Nadpisuje gałąź zdalną, która nie jest przodkiem: lease lub true tylko dopóki nie zmieniła się od ostatniego pobrania, yes zawsze. |
| `force_with_lease` | `text` | brak | Nadpisuje gałąź zdalną tylko dopóki nadal wskazuje tę wersję. |
| `tags` | `bool` | `false` | Wysyła także tagi. |
| `set_upstream` | `bool` | `false` | Ustawia wysłaną gałąź jako upstream gałęzi lokalnej. |
| `delete` | `bool` | `false` | Usuwa gałąź w repozytorium zdalnym. |
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
| External use | `user_program`; Uruchamia hak pre-push repozytorium oraz haki pre-receive, update i post-receive repozytorium zdalnego w lokalnym systemie plików jako programy użytkownika. |

## Przykłady {#examples}

Wysłanie bieżącej gałęzi i zapisanie jej jako upstream.

```nurt example
git.push "/home/user/repo" branch: "HEAD" set_upstream: true
```

```text expected
effect git.push state: completed upstream: origin/main
git_ref_update ref: refs/heads/main status: new remote: origin remote_ref: refs/heads/main
```

Zastąpienie przepisanej gałęzi tylko wtedy, gdy repozytorium zdalne nadal ma
pobraną rewizję.

```nurt example
git.push "/home/user/repo" branch: "feature/login" force: lease
```

Wysłanie rewizji do gałęzi o innej nazwie w lokalnym lustrze repozytorium zdalnego.

```nurt example
git.push "/home/user/repo" remote: "/srv/git/mirror.git" branch: "4f2a9c1:refs/heads/release"
```

Usunięcie gałęzi w repozytorium zdalnym.

```nurt example
git.push "/home/user/repo" branch: "old-topic" delete: true
```

Wysłanie gałęzi wraz ze wszystkimi tagami.

```nurt example
git.push "/home/user/repo" branch: "main" tags: true
```

Wysłanie do usługi hostingowej przez HTTPS z tokenem ze środowiska.

```nurt example
git.push "/home/user/repo" remote: "https://git.example.com/team/app.git" branch: "main"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_PUSH_REJECTED` | Żadne odwołanie nie zostało zaktualizowane; szczegóły wymieniają każdą aktualizację z jej powodem (`non_fast_forward`, `fetch_first`, `stale_info`, `already_exists`, `checked_out`, `hook_declined`, `atomic_failed`, `deletion_prohibited`, `remote_ref_missing`). | `git.pull "/home/user/repo" remote: "origin"`, a następnie wyślij ponownie |
| `E_GIT_HOOK` | Hak `pre-push` odrzucił push. | Popraw problem zgłoszony przez hak |
| `E_GIT_REVISION` | Źródło specyfikacji odwołania nie daje się rozwiązać albo wersja nie ma celu. | Użyj `src:refs/heads/<name>` |
| `E_GIT_REF` | Specyfikacja odwołania lub nazwa odwołania jest nieprawidłowa. | Popraw nazwę |
| `E_NOT_FOUND` | Repozytorium zdalne nie jest skonfigurowane. | Przekaż adres URL lub ścieżkę albo skonfiguruj `remote.<name>.url` |
| `E_GIT_NOT_REPOSITORY` | Repozytorium lub lokalne repozytorium zdalne nie jest repozytorium Git. | Sprawdź ścieżkę i adres URL repozytorium zdalnego |
| `E_GIT_UNSUPPORTED` | Adres używa `git://` lub `ext::`, albo SSH w wersji bez SSH. | Użyj adresu `https://` lub `ssh://` |
| `E_GIT_AUTH` | Repozytorium zdalne wymaga poświadczeń, których brakuje, które zostały odrzucone albo które zostałyby wysłane przez `http://`. | Ustaw `NURT_GIT_TOKEN_<HOST>` i użyj adresu `https://` |
| `E_NETWORK` | Nie można połączyć się z repozytorium zdalnym albo odpowiedziało ono błędem HTTP. | Sprawdź adres, ustawienia proxy i połączenie |
| `E_INVALID_VALUE` | Nie można przetworzyć adresu lub ustawienia `http.*`. | Popraw adres URL repozytorium zdalnego lub ustawienie wskazane w komunikacie |
| `E_GIT_PROTOCOL` | Repozytorium zdalne naruszyło protokół albo nie ma wymaganej możliwości. | Wyślij bez opcji wskazanej w komunikacie |
| `E_GIT_LOCKED` | Istnieje blokada odwołania lub `config.lock`. | Poczekaj na inny proces albo usuń nieaktualną blokadę |
| `E_STALE` | Odwołanie zmieniło się podczas wysyłania. | Pobierz i wyślij ponownie |

## Powiązane {#related}

`git.pull`, `git.fetch`, `git.branch`, `undo`.

.
