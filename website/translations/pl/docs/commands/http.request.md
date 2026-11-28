---
command: http.request
group: network
summary: Wysyła żądanie HTTP i zwraca odpowiedź, opcjonalnie jako tekst lub Markdown.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# http.request

## Składnia {#synopsis}

```text
http.request <url> [name: value]...
```

## Opis {#description}

`http.request` wysyła jedno żądanie dla każdego adresu z `url` i zwraca jeden
rekord `response` na adres, w kolejności adresów. Lista adresów jest pobierana
z jednoczesnością do czterech żądań. Z `extract: tables` polecenie zwraca
zamiast tego rekordy `row`, po jednym na wiersz każdej wybranej tabeli.

Adres bez schematu jest żądany z `https://`, a raport zawiera uwagę
`scheme_assumed`. Nazwy hostów są konwertowane do Punycode, a adresy IPv4
zapisane w postaci dziesiętnej, ósemkowej lub szesnastkowej są przed żądaniem
konwertowane do postaci z kropkami. Akceptowane są tylko schematy `http` i
`https`.

Status 400 lub wyższy jest wynikiem, a nie błędem: status to pole `status`
rekordu. Przekierowania ze statusami 301, 302, 303, 307 i 308 są wykonywane do
`max_redirects`; przebyte adresy są wymienione w `redirects`, a ostatni to
`final_url`. Przekierowanie do innego hosta pomija nagłówki `Authorization`,
`Cookie` i `Proxy-Authorization`. Przekierowanie z `https` do `http` nie jest
wykonywane: żądanie kończy się błędem `E_NETWORK` z problemem
`scheme_downgrade`, a `follow_redirects: false` zwraca samo przekierowanie.
Status `303` zamienia żądanie w `get`, chyba że jest to `head`, które jest
wysyłane ponownie jako `head`. Z `retries` żądanie, które nie może się połączyć,
zostało zresetowane albo otrzymało status 429 lub status 500 i wyższy, jest
wysyłane ponownie po przerwie; `Retry-After` ustawia przerwę, gdy serwer ją
podaje. Żądania z metodami `post` i `patch` nie są wysyłane ponownie.

Treści typów `gzip` i `deflate` są dekompresowane. Zestaw znaków treści
tekstowej jest brany z, w kolejności: `charset`, znacznika kolejności bajtów,
parametru `charset` nagłówka `Content-Type`, deklaracji `<meta>` strony HTML i
wykrywania z bajtów. Treść, która nie jest tekstem, taka jak obraz, może być
zapisana tylko do pliku z `save`.

Z `as: text` lub `as: markdown` strona HTML jest konwertowana na zwykły tekst
lub Markdown. `main_only` zachowuje główną zawartość i pomija nawigację,
nagłówki, stopki i formularze; `links` i `images` wybierają sposób zapisu
odnośników i obrazów. Dokumenty Markdown i zwykłego tekstu są zwracane w tych
samych postaciach. Części strony wybiera się za pomocą `section` (zawartość pod
jednym nagłówkiem), `find` (akapity pasujące do wzorca, z `find_context`
akapitów wokół każdego) i `query` (`query_limit` akapitów najbardziej
istotnych dla zapytania, w kolejności dokumentu). Z `outline: auto` strona
dłuższa niż opcja `max_chars` wywołania jest zwracana jako zarys: tytuł,
długość, sekcje z ich rozmiarami, początek zawartości i ścieżka pliku, który
przechowuje pełną zawartość. `offset` pomija znaki wyniku; `total_chars` podaje
długość całego wyniku.

Pamięć podręczna odpowiedzi przechowuje udane odpowiedzi `get` na żądania bez
własnych nagłówków. `cache: use` odpowiada ze świeżej odpowiedzi w pamięci
podręcznej, `revalidate` pyta serwer, czy odpowiedź w pamięci podręcznej jest
nadal aktualna, za pomocą `If-None-Match` i `If-Modified-Since`, a `only`
odpowiada z pamięci podręcznej bez żadnego dostępu do sieci. `unchanged_since`
przyjmuje `etag` wcześniejszego wyniku; gdy treść jest taka sama, rekord ma
`unchanged: true` i pustą treść.

Konsola nie stosuje własnych reguł sieciowych. Każdy adres, w tym adresy
pętli zwrotnej i prywatne, jest kontaktowany. Kontrola programu (`nurt_check`)
wymienia każde żądanie we wpisie `network` swoich szczegółów, ze
znormalizowanym adresem, hostem, portem, schematem, serwerem proxy, nazwami
nagłówków, rodzajem treści, trybem pamięci podręcznej i, dla adresów IP, klasą
adresu. Aplikacja osadzająca, która kontroluje dostęp do sieci, może odmówić
połączeń, zanim zostaną otwarte.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `url` | `text \| list<text>` | wymagany | Adres albo kilka adresów z jednym wynikiem dla każdego. |
| `method` | `enum(get\|head\|post\|put\|patch\|delete\|options)` | `get` | Metoda HTTP. |
| `headers` | `map<text>` | brak | Nagłówki żądania. |
| `body` | `text \| path` | brak | Treść żądania jako tekst albo z pliku. |
| `json` | `any` | brak | Wartość wysyłana jako treść JSON z odpowiednim nagłówkiem Content-Type. |
| `save` | `path` | brak | Zapisuje treść odpowiedzi do tego pliku; treść odpowiedzi ze statusem spoza zakresu 200–299 nie jest zapisywana, a rekord nie ma pola `saved`. |
| `as` | `enum(raw\|text\|markdown)` | `raw` | Zwraca stronę HTML bez zmian, jako zwykły tekst albo jako Markdown z zarysem. |
| `extract` | `enum(content\|headers\|links\|headings\|meta\|tables)` | `content` | Zwracana część strony: zawartość, nagłówki, odnośniki, nagłówki sekcji, metadane lub tabele. |
| `table` | `int` | `0` | Numer tabeli zwracanej z extract ustawionym na tables, liczony od 1; 0 zwraca wszystkie tabele. |
| `main_only` | `bool` | `true` | Zachowuje tylko główną zawartość strony przy konwersji na tekst lub Markdown. |
| `links` | `enum(inline\|list\|none)` | `inline` | Zachowuje odnośniki w tekście, zbiera je na liście albo pomija przy konwersji. |
| `images` | `enum(drop\|alt)` | `alt` | Pomija obrazy albo zachowuje ich tekst alternatywny przy konwersji. |
| `section` | `text` | brak | Zwraca tylko sekcję pod nagłówkiem o tym tekście. |
| `outline` | `enum(auto\|always\|never)` | `auto` | Zwraca zarys długiej strony przed jej zawartością. |
| `query` | `text` | brak | Zwraca tylko akapity strony najbardziej istotne dla tego zapytania. |
| `query_limit` | `int` | `5` | Maksymalna liczba akapitów zwracanych dla query, od 1 do 20. |
| `find` | `regex` | brak | Zwraca tylko akapity pasujące do tego wzorca. |
| `find_context` | `int` | `1` | Liczba akapitów kontekstu wokół każdego akapitu dopasowanego przez find, do 10. |
| `links_filter` | `regex` | brak | Tylko odnośniki, których adres pasuje do tego wzorca, z extract ustawionym na links. |
| `same_host` | `bool` | `false` | Tylko odnośniki do tego samego hosta, z extract ustawionym na links. |
| `charset` | `text` | `"auto"` | Zestaw znaków używany do dekodowania treści. |
| `offset` | `int` | `0` | Liczba znaków treści do pominięcia. |
| `cache` | `enum(bypass\|use\|revalidate\|only)` | `bypass` | Ignoruje pamięć podręczną odpowiedzi, używa jej, ponownie weryfikuje odpowiedzi z pamięci podręcznej albo odpowiada tylko z pamięci podręcznej bez sieci. |
| `unchanged_since` | `text` | brak | Etag znanej wersji; niezmieniona odpowiedź nie zwraca treści. |
| `timeout` | `duration` | `30s` | Limit czasu żądania. |
| `follow_redirects` | `bool` | `true` | Wykonuje przekierowania. |
| `max_redirects` | `int` | `10` | Maksymalna liczba wykonywanych przekierowań. |
| `verify_tls` | `bool` | `true` | Weryfikuje certyfikat TLS serwera. |
| `retries` | `int` | `0` | Liczba ponowień po niepowodzeniu połączenia. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `response` |

Rekord `response` ma pola `url`, `status`, `headers` (nazwy małymi literami;
powtarzające się nagłówki połączone przez `, `, `set-cookie` jako lista),
`body`, `bytes`, `elapsed`, `final_url`, `saved`, `content_type`, `charset`,
`truncated`, `offset`, `total_chars`, `redirects`, `from_cache`, `unchanged`,
`etag`, `last_modified` i `remote_address`. `etag` to pierwsze 12 cyfr
szesnastkowych SHA-256 zwróconej treści. Rekord `row` zawiera komórki jednego
wiersza tabeli według nazwy kolumny, numer wiersza `no` i numer tabeli `table`.

Postać tekstowa wyniku pokazuje kolumny `url`, `status`, `bytes`, `content_type`
i `elapsed`, a nigdy treść. Treść i pozostałe pola pozostają w postaci JSON i
pojawiają się w tekście przy szczegółowości `detailed` albo gdy żądanie wymienia
je w `fields`, na przykład `fields: ["status", "body"]`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| Warunki odwracalności | Metody get, head i options tylko odczytują; pozostałe metody zmieniają stronę zdalną i są nieodwracalne; plik zapisany przez save jest odwracalny. |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `post` | `external` | `irreversible` | Obowiązuje, gdy method to post. |
| `put` | `external` | `irreversible` | Obowiązuje, gdy method to put. |
| `patch` | `external` | `irreversible` | Obowiązuje, gdy method to patch. |
| `delete` | `external` | `irreversible` | Obowiązuje, gdy method to delete. |
| `save` | `local` | `reversible` | Dotyczy pliku zapisanego, gdy podano save. |

## Przykłady {#examples}

Pobierz stronę dokumentacji jako Markdown:

```nurt example
http.request "https://docs.example.com/install" as: markdown
```

```text expected
# Installation
```

Zwróć trzy akapity strony najbardziej istotne dla pytania:

```nurt example
http.request "https://docs.example.com/guide" as: text query: "proxy configuration" query_limit: 3
```

Zapisz archiwum wydania do pliku:

```nurt example
http.request "https://example.com/releases/app.tar.gz" save: "/srv/downloads/app.tar.gz"
```

Odczytaj jedną sekcję strony z pamięci podręcznej wypełnionej wcześniejszym żądaniem:

```nurt example
http.request "https://docs.example.com/guide" as: markdown section: "Upgrade" cache: use
```

Wyślij treść JSON i wymagaj udanego statusu:

```nurt example
http.request "https://api.example.com/items" method: post json: {name: "demo"} |> assert all: status < 400
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_ARG` | Adres jest nieprawidłowy, parametry kolidują (`problem: param_conflict`), liczba jest poza zakresem, `offset` jest za końcem wyniku albo `save` lub metoda zmieniająca jest użyta tam, gdzie dozwolony jest tylko odczyt. | Popraw adres albo usuń kolidujący parametr. |
| `E_UNKNOWN_VALUE` | Parametr ma wartość spoza swojego wyliczenia. | Użyj jednej z wymienionych wartości. |
| `E_TYPE` | Parametr ma wartość innego typu. | Podaj wartość typu z tabeli. |
| `E_REGEX` | Wzorzec `find` lub `links_filter` jest nieprawidłowy. | Popraw wzorzec. |
| `E_NETWORK` | Żądanie nie powiodło się. `problem` mówi dlaczego: `dns`, `connect`, `reset`, `tls`, `proxy`, `redirect_limit`, `scheme_downgrade`, `bad_location`, `protocol`, `decode` lub `denied`. | Sprawdź adres, serwer proxy i certyfikat albo zwiększ `max_redirects`. |
| `E_TIMEOUT` | Żądanie nie zakończyło się w ramach `timeout`. | Zwiększ `timeout`. |
| `E_CANCELLED` | Program został anulowany w trakcie żądania. | Uruchom program ponownie. |
| `E_NOT_FOUND` | `cache: only` nie znalazło odpowiedzi w pamięci podręcznej, strona nie ma sekcji `section` albo strona ma mniej tabel niż `table`. | Najpierw pobierz stronę z `cache: use` albo użyj sekcji lub tabeli z `expected`. |
| `E_BINARY` | Treść nie jest tekstem. | Zapisz treść do pliku z `save`. |
| `E_CONFIG` | Nie można odczytać pliku `http.ca_file`. | Popraw ustawienie. |

## Powiązane {#related}

`http.download`, `network.resolve`, `undo`, `help group: "network"`.
