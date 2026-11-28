---
command: http.download
group: network
summary: Pobiera plik przez HTTP, wznawiając częściowe pobieranie, gdy to możliwe.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# http.download

## Składnia {#synopsis}

```text
http.download <url> <to> [name: value]...
```

## Opis {#description}

`http.download` wysyła żądanie `GET` pod adres `url` i zapisuje treść odpowiedzi
w pliku `to`. Treść jest zapisywana tak, jak została odebrana: żądanie prosi o
`Accept-Encoding: identity`, a przekierowania są wykonywane.

Treść jest najpierw zapisywana w pliku częściowym `<to>.part` w katalogu celu.
Plik `<to>.part.meta` zapisuje adres i walidator odpowiedzi: silny `ETag`, a w
przeciwnym razie `Last-Modified`. Gdy pobieranie kończy się przedwcześnie
(zamknięte połączenie, limit czasu, anulowanie), oba pliki pozostają. Następne
uruchomienie dla tego samego adresu i celu z `resume: true` wysyła
`Range: bytes=<size of the partial file>-` z `If-Range: <validator>`:

| Odpowiedź serwera | Wynik |
|---|---|
| `206` od rozmiaru pliku częściowego | Reszta jest dopisywana; `resumed_from` to rozmiar pliku częściowego. |
| `200` | Zasób się zmienił albo serwer ignoruje zakresy; pobieranie zaczyna się od nowa, a `resumed_from` wynosi 0. |
| `416` z rozmiarem pliku częściowego jako pełną długością | Plik częściowy zawiera już całą zawartość; żadne bajty nie są przesyłane. |

Z `hash` skrót całej zawartości jest obliczany w trakcie napływania bajtów, w
tym bajtów wznowionego pliku częściowego. Niezgodność usuwa plik częściowy i
pozostawia cel bez zmian.

Po pełnym przesłaniu ze zgodnym skrótem zawartość zastępuje cel w jednym kroku, a
plik częściowy jest usuwany. Katalog `to` musi istnieć.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `url` | `text` | wymagany | Adres pliku. |
| `to` | `path` | wymagany | Plik do zapisania. |
| `resume` | `bool` | `true` | Kontynuuje częściowe pobieranie. |
| `hash` | `text` | brak | Oczekiwany skrót pliku, na przykład sha256:<hex>; niezgodność pozostawia cel bez zmian. |
| `headers` | `map<text>` | brak | Nagłówki żądania. |
| `timeout` | `duration` | `300s` | Limit czasu pobierania. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

Rekord `effect` ma `kind` równe `http.download`, jeden wpis w `changes` dla celu
(`action` równe `create` lub `modify`, `size_before`, `size_after`, `hash_after`
jako `<algorithm>:<hex>`) oraz obiekt pobierania w `after`:

Zdarzenia postępu fazy `receive` raportują odebrane bajty.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Zapisany plik jest odwracalny; samo żądanie tylko odczytuje z sieci. |

Żądanie tylko odczytuje z sieci i nie jest zapisywane jako skutek nieodwracalny.
Plik częściowy i jego opis są zapisywane poza dziennikiem i nie są cofane; są
usuwane po udanym pobraniu oraz po niezgodności skrótu.

## Przykłady {#examples}

Pobierz archiwum i sprawdź jego skrót SHA-256:

```nurt example
http.download "https://example.org/dist/tool-1.4.2.tar.gz" "/home/user/tool-1.4.2.tar.gz" hash: "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
```

```text expected
effect  http.download  ok  /home/user/tool-1.4.2.tar.gz  create
```

Pobierz obraz ponownie od pierwszego bajtu, z dłuższym limitem czasu:

```nurt example
http.download "https://example.org/images/disk.img" "/srv/images/disk.img" resume: false timeout: 30m
```

Pobierz artefakt kompilacji z dodatkowym nagłówkiem żądania:

```nurt example
http.download "https://artifacts.example.org/builds/42/app.zip" "/home/user/app.zip" headers: {accept: "application/zip"}
```

Pokaż obiekt pobierania, który mówi, czy wcześniejsze częściowe pobieranie
zostało kontynuowane:

```nurt example
http.download "https://example.org/data.csv" "/home/user/data.csv" |> select after
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_ARG` | `url` nie jest prawidłowym adresem HTTP ani HTTPS. | Podaj adres taki jak `https://example.org/file`. |
| `E_INVALID_VALUE` | `hash` nie ma znanego algorytmu albo ma błędną liczbę cyfr, `headers` ustawia `Range` lub `If-Range`, `timeout` wynosi zero albo `to` nie jest zwykłym plikiem. | Popraw wartość zgodnie z opisem w komunikacie. |
| `E_NOT_FOUND` | Katalog `to` nie istnieje. | Utwórz katalog, a następnie uruchom polecenie ponownie. |
| `E_NETWORK` | Nie można połączyć się z serwerem, uzgadnianie TLS się nie powiodło, połączenie zamyka się przed końcem treści (`problem` `incomplete`) albo serwer odpowiada statusem 400 lub wyższym (`problem` `http_status`, status w `status`). | Sprawdź adres i sieć; uruchom polecenie ponownie, aby wznowić niepełne pobieranie. |
| `E_HASH_MISMATCH` | Skrót zawartości różni się od `hash`. | Sprawdź oczekiwany skrót i adres. |
| `E_TIMEOUT` | Pobieranie nie zakończyło się w ramach `timeout`. | Zwiększ `timeout`; następne uruchomienie wznawia częściowe pobieranie. |
| `E_CANCELLED` | Wykonanie zostało anulowane. | Uruchom polecenie ponownie, aby wznowić. |
| `E_PERMISSION` | Nie można zapisać pliku częściowego ani celu. | Sprawdź uprawnienia i wolne miejsce woluminu. |
| `E_NO_SPACE` | Nie można zapisać pliku częściowego ani celu. | Sprawdź uprawnienia i wolne miejsce woluminu. |
| `E_IO` | Nie można zapisać pliku częściowego ani celu. | Sprawdź uprawnienia i wolne miejsce woluminu. |
| `E_STALE` | Cel został utworzony przez inny proces w trakcie pobierania. | Uruchom polecenie ponownie. |

## Powiązane {#related}

`http.request`, `hash`, `undo`.
