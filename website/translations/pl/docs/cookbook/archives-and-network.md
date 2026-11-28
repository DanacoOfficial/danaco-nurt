---
category: archives-and-network
title: Archiwa i sieć
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/archives-and-network.md -->
<!-- Responsibility:   Documents Archives and network. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Archiwa i sieć {#archives-and-network}

## Rozpakowanie archiwum {#extract-an-archive}

Rozpakuj skompresowane archiwum tar do katalogu.

```nurt example
archive.extract "/home/user/dl/data.tar.gz" into: "/home/user/data"
```

```text expected
/home/user/data
```

```bash
tar -xzf /home/user/dl/data.tar.gz -C /home/user/data
```

## Wyświetlanie zawartości archiwum {#list-the-content-of-an-archive}

Wyświetl wpisy archiwum bez jego rozpakowywania.

```nurt example
archive.list "/home/user/dl/data.zip"
```

```text expected
data
```

## Rozpakowanie tylko wybranych wpisów {#extract-only-some-entries}

Rozpakuj wpisy pasujące do wzorca.

```nurt example
archive.extract "/home/user/dl/data.zip" into: "/home/user/data" only: "**/*.csv"
```

```text expected
/home/user/data
```

## Tworzenie archiwum {#create-an-archive}

Utwórz archiwum ZIP katalogu.

```nurt example
find "/home/user/app/dist" |> archive.create "/home/user/out/dist.zip" base: "/home/user/app"
```

```text expected
/home/user/out/dist.zip
```

```bash
cd /home/user/app && zip -r /home/user/out/dist.zip dist
```

## Kompresja pliku {#compress-a-file}

Skompresuj plik programem gzip i zachowaj oryginał.

```nurt example
compress "/home/user/app/app.log"
```

```text expected
app.log
```

## Dekompresja pliku {#decompress-a-file}

Zdekompresuj plik gzip.

```nurt example
decompress "/home/user/app/app.log.gz"
```

```text expected
app.log
```

## Wysyłanie żądania HTTP {#send-an-http-request}

Wyślij żądanie GET i wymagaj jednej odpowiedzi.

```nurt example
http.request "https://example.com/health" |> assert count: 1
```

```text expected
200
```

```bash
curl -fsS https://example.com/health
```

## Wysyłanie JSON metodą POST {#post-json}

Wyślij treść JSON w żądaniu POST.

```nurt example
http.request "https://example.com/api/items" method: post json: {"name": "item"}
```

```text expected
200
```

## Wysyłanie żądania z nagłówkiem {#send-a-request-with-a-header}

Wyślij żądanie z nagłówkiem autoryzacji.

```nurt example
http.request "https://example.com/api/me" headers: {"Accept": "application/json"}
```

```text expected
200
```

## Odczyt strony jako Markdown {#read-a-page-as-markdown}

Odczytaj główny tekst strony internetowej jako Markdown.

```nurt example
http.request "https://example.com/docs" as: markdown
```

```text expected
example
```

## Pobranie pliku i weryfikacja jego skrótu {#download-a-file-and-verify-its-hash}

Pobierz plik i zweryfikuj jego skrót SHA-256.

```nurt example
http.download "https://example.com/a.tar.gz" "/home/user/dl/a.tar.gz" hash: "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
```

```text expected
/home/user/dl/a.tar.gz
```

```bash
curl -fLo a.tar.gz https://example.com/a.tar.gz && sha256sum a.tar.gz
```

## Sprawdzenie, czy host odpowiada {#check-that-a-host-answers}

Wyślij do hosta cztery żądania echo.

```nurt example
network.ping "example.com" count: 4
```

```text expected
example.com
```

## Rozwiązywanie nazwy {#resolve-a-name}

Rozwiąż adresy IPv6 podanej nazwy.

```nurt example
network.resolve "example.com" type: aaaa
```

```text expected
example.com
```

## Wyświetlanie interfejsów sieciowych {#list-the-network-interfaces}

Wyświetl interfejsy sieciowe hosta.

```nurt example
network.interfaces
```

```text expected
interface
```

## Rozpakowanie archiwum bez katalogu najwyższego poziomu {#extract-an-archive-without-its-top-directory}

Rozpakuj archiwum źródłowe i pomiń pierwszy składnik ścieżki każdego wpisu.

```nurt example
archive.extract "/home/user/dl/src.tar.gz" into: "/home/user/src" strip_components: 1
```

```text expected
/home/user/src
```

```bash
tar -xzf /home/user/dl/src.tar.gz -C /home/user/src --strip-components=1
```

## Rozpakowanie bez nadpisywania plików {#extract-without-overwriting-files}

Rozpakuj archiwum i zachowaj pliki, które już istnieją.

```nurt example
archive.extract "/home/user/dl/data.zip" into: "/home/user/data" on_conflict: skip
```

```text expected
/home/user/data
```

## Wyświetlanie wpisów pasujących do wzorca {#list-the-entries-that-match-a-pattern}

Wyświetl tylko wpisy JSON archiwum.

```nurt example
archive.list "/home/user/dl/data.zip" glob: "**/*.json"
```

```text expected
json
```

```bash
unzip -l /home/user/dl/data.zip '*.json'
```

## Tworzenie archiwum tar Zstandard {#create-a-zstandard-tar-archive}

Spakuj katalog źródłowy do archiwum tar skompresowanego algorytmem Zstandard.

```nurt example
find "/home/user/app/src" |> archive.create "/home/user/out/src.tar.zst" format: tar_zst base: "/home/user/app"
```

```text expected
/home/user/out/src.tar.zst
```

```bash
tar --zstd -cf /home/user/out/src.tar.zst -C /home/user/app src
```

## Kompresja pliku wybranym algorytmem {#compress-a-file-with-a-chosen-algorithm}

Skompresuj log algorytmem Zstandard na poziomie 9 i usuń oryginał.

```nurt example
compress "/home/user/app/app.log" algorithm: zstd level: 9 keep: false
```

```text expected
app.log
```

```bash
zstd -9 --rm /home/user/app/app.log
```

## Kompresja starych logów {#compress-the-old-logs}

Skompresuj pliki logów starsze niż siedem dni.

```nurt example
find "/home/user/logs" glob: "*.log" depth: 1 |> filter modified < ago(7d) |> compress
```

```text expected
nothing_matched
```

## Żądanie samych nagłówków {#request-only-the-headers}

Wyślij żądanie HEAD, aby odczytać nagłówki zasobu.

```nurt example
http.request "https://example.com/a.tar.gz" method: head
```

```text expected
200
```

```bash
curl -fsSI https://example.com/a.tar.gz
```

## Zapis odpowiedzi do pliku {#save-a-response-to-a-file}

Zapisz treść odpowiedzi do pliku.

```nurt example
http.request "https://example.com/data.json" save: "/home/user/dl/data.json"
```

```text expected
/home/user/dl/data.json
```

## Wyświetlanie odnośników strony {#list-the-links-of-a-page}

Wyświetl odnośniki strony prowadzące do tego samego hosta.

```nurt example
http.request "https://example.com/docs" extract: links same_host: true
```

```text expected
example.com
```

## Ograniczenie czasu i ponawianie żądania {#limit-the-time-and-retry-a-request}

Zakończ żądanie błędem po pięciu sekundach i ponów je do trzech razy.

```nurt example
http.request "https://example.com/health" timeout: 5s retries: 3
```

```text expected
200
```

```bash
curl -fsS --max-time 5 --retry 3 https://example.com/health
```

## Rozwiązywanie serwera poczty {#resolve-a-mail-exchanger}

Rozwiąż serwery poczty domeny przez wybrany serwer DNS.

```nurt example
network.resolve "example.com" type: mx server: "9.9.9.9"
```

```text expected
example.com
```

```bash
dig @9.9.9.9 example.com MX
```

## Ping z krótkim limitem czasu {#ping-with-a-short-timeout}

Wyślij jedno żądanie echo i czekaj najwyżej jedną sekundę.

```nurt example
network.ping "example.com" count: 1 timeout: 1s
```

```text expected
example.com
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
