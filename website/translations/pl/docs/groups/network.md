---
group: network
summary: Żądania HTTP, pobieranie, rozwiązywanie nazw, ping i interfejsy sieciowe.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Grupa network

## Zakres {#scope}

Grupa wysyła żądania HTTP, pobiera pliki, rozwiązuje nazwy DNS, mierzy czasy
odpowiedzi i wylicza interfejsy sieciowe hosta. Wszystkie polecenia są częścią
Danaco Nurt; żadne z nich nie uruchamia programu.

| Zadanie | Polecenie | Grupa |
|---|---|---|
| Odczyt lub zapis pliku lokalnego | `read`, `write` | core |
| Rozpakowanie pobranego archiwum | `archive.extract` | archive |
| Cofnięcie pobrania | `undo` | core |
| Sprawdzenie, z którymi hostami łączy się program | `nurt_check` | wszystkie grupy |

Sprawdzenie programu (`nurt_check`) wylicza cele sieciowe `http.request` i
`http.download` przed uruchomieniem programu, więc aplikacja osadzająca może
zastosować własne reguły.

## Polecenia {#commands}

| Polecenie | Kategoria | Wejście i wyjście | Odwracalność | Strona |
|---|---|---|---|---|
| `http.request` | change | brak do `response` | read, według metody | [http.request](../commands/http.request.md) |
| `http.download` | change | brak do `effect` | reversible | [http.download](../commands/http.download.md) |
| `network.ping` | source | brak do `ping_result` | read | [network.ping](../commands/network.ping.md) |
| `network.resolve` | source | brak do `dns_record` | read | [network.resolve](../commands/network.resolve.md) |
| `network.interfaces` | source | brak do `network_interface` | read | [network.interfaces](../commands/network.interfaces.md) |

## Rekordy {#records}

| Rekord | Wymagane pola |
|---|---|
| `response` | `url`, `status`, `headers`, `body`, `bytes`, `elapsed`, `final_url` |
| `ping_result` | `host`, `address`, `sent`, `received`, `loss` |
| `dns_record` | `name`, `type`, `value`, `ttl` |
| `network_interface` | `name`, `addresses`, `up` |
| `effect` | `status`, `kind` |

Czasy odpowiedzi `ping_result` (`rtt_min`, `rtt_avg`, `rtt_max`,
`rtt_stddev`) są podawane w milisekundach. `loss` jest wartością procentową.

## Żądania HTTP {#http-requests}

Status odpowiedzi HTTP jest daną. Odpowiedź ze statusem 404 lub 503 jest
pomyślnym wynikiem `http.request`; status jest polem `status` rekordu, a
program sprawdza go za pomocą `filter` lub `assert`. Błędem jest tylko
niepowodzenie uzyskania odpowiedzi.

Parametr `as` wybiera postać treści: `raw` (wartość domyślna), `text` albo
`markdown` dla strony skonwertowanej z HTML. Parametr `extract` wybiera, co
jest zwracane ze strony: `content`, `headers`, `links`, `headings`, `meta`
lub `tables`. Przy kilku adresach w `url` polecenie daje jeden rekord na
adres.

Odwracalność `http.request` zależy od metody:

| Metoda | Skutek | Odwracalność |
|---|---|---|
| `get`, `head`, `options` | read | read |
| `post`, `put`, `patch`, `delete` | external | irreversible |
| dowolna, z `save` | zapisany plik | reversible, przywracany przez `undo` |

Nieodwracalne żądanie jest wymienione wśród skutków nieodwracalnych raportu
skutków.

`http.download` zapisuje treść odpowiedzi w ścieżce z `to` i wznawia
częściowe pobieranie, gdy serwer to obsługuje (`resume: true`). Z `hash`
pobrany plik jest weryfikowany względem podanego skrótu, na przykład `sha256:`
a po nim skrót. Plik z innym skrótem nie jest zachowywany, a polecenie kończy
się błędem `E_HASH_MISMATCH`. Zapis pliku przechodzi przez dziennik operacji,
więc `undo` usuwa pobrany plik albo przywraca plik, który został zastąpiony.

## Różnice między platformami {#platform-differences}

| Polecenie | Linux | macOS | Windows |
|---|---|---|---|
| `http.request`, `http.download` | identyczne zachowanie | identyczne zachowanie | identyczne zachowanie |
| `network.ping` | echo ICMP przez system | echo ICMP przez system | echo ICMP przez API systemu |
| `network.resolve` | resolver systemu albo host w `server` | tak samo | tak samo |
| `network.interfaces` | interfejsy systemu | interfejsy systemu | adaptery systemu |

Polecenia zgłaszają następujące błędy:

| Kod | Znaczenie |
|---|---|
| `E_NETWORK` | Niepowodzenie rozwiązywania nazwy, połączenia, TLS, proxy, przekierowania lub protokołu. |
| `E_HASH_MISMATCH` | Skrót pobranego pliku różni się od `hash`. |
| `E_TIMEOUT` | Upłynął `timeout` polecenia lub programu. |
| `E_CANCELLED` | Wykonanie zostało anulowane. |

Wartość domyślna `timeout` polecenia `http.download` to `300s`, dłużej niż
domyślny limit czasu programu. Gdy program nie ustawia `timeout`, jego limit
jest podnoszony do najdłuższego jawnego limitu jego poleceń.

## Programy zewnętrzne {#external-programs}

Żadne polecenie grupy nie uruchamia programu. Grupa nie ma znacznika [X], [P]
ani [U].

## Przykłady {#examples}

Wysłanie żądania i sprawdzenie, że nadeszła dokładnie jedna odpowiedź:

```nurt example
http.request "https://api.example.com/health" |> assert count: 1
```

Sprawdzenie, że status oznacza powodzenie. Status 400 lub wyższy kończy
niepowodzeniem `assert`, a nie żądanie:

```nurt example
http.request "https://example.com/health" |> assert all: status < 400
```

Odczyt strony jako Markdown:

```nurt example
http.request "https://example.com/docs" as: markdown
```

Wysłanie JSON metodą `post`:

```nurt example
http.request "https://example.com/api/items" method: post json: {name: "demo"}
```

Pobranie pliku i weryfikacja jego skrótu:

```nurt example
http.download "https://example.com/a.tar.gz" "/home/user/dl/a.tar.gz" hash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
```

Pomiar trzech czasów odpowiedzi hosta:

```nurt example
network.ping "example.com" count: 3
```

Rozwiązanie serwerów poczty domeny:

```nurt example
network.resolve "example.com" type: mx
```

Wylistowanie interfejsów, które są włączone:

```nurt example
network.interfaces |> filter up |> select name addresses
```

## Powiązane {#related}

[Grupa schedule](schedule.md) uruchamia żądania i pobierania o ustalonych
porach. [Grupa system](system.md) wylicza porty i procesy, które je
utrzymują.
