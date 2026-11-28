<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/tool-calls.md -->
<!-- Responsibility:   Documents Tool calls. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Wywołania narzędzia {#tool-calls}

## Argumenty i żądanie {#arguments-and-request}

Wywołanie narzędzia jest zamieniane na żądanie przez umieszczenie `program`, `ast`, `cursor` i `accept` na najwyższym poziomie, a każdego innego argumentu w `options`:

<!-- schema: arguments -->
```json
{"program": "find \"/home/user/app\" glob: \"**/*.go\" |> count", "limit": 20}
```

<!-- schema: request -->
```json
{"nurt": "1.0", "program": "find \"/home/user/app\" glob: \"**/*.go\" |> count", "options": {"limit": 20}}
```

Nieznany argument jest błędem wywołania: host zwraca go modelowi jako błąd narzędzia z nazwą argumentu. Żądanie z dwoma wejściami, względnym `base` lub `limit` spoza zakresu otrzymuje w raporcie błąd języka `E_ARG` wraz ze wskazówką naprawy.

## Wynik {#result}

Host zwraca modelowi `rendered.text` odpowiedzi; dla `format: "json"`, gdzie `rendered` nie występuje, zwraca raport jako JSON. Raport z `error` (`report.error`) jest zwracany jako błąd narzędzia (`is_error: true` dla Anthropic, odpowiedź błędu dla MCP), aby model odczytał `fix` i poprawił wywołanie. Błędy języka nigdy nie zgłaszają wyjątków w wiązaniach; `raise_on_error` zmienia to dla pojedynczego wywołania.

## Poprawki i `accept` {#corrections-and-accept}

Konsola poprawia typowe błędy zapisu przed wiązaniem. Poprawka dotykająca celu zmiany nie jest stosowana; raport zawiera `E_AMBIGUOUS` z uszeregowanymi kandydatami. Dotyczy to błędnie napisanej nazwy polecenia, które zmienia system plików lub stan zdalny albo uruchamia program: `remvoe "/home/user/app/build"` nie jest uruchamiane, a pierwszym kandydatem jest `remove "/home/user/app/build"`. Błędnie napisana nazwa polecenia, które tylko odczytuje (`fnid`), jest poprawiana i program się wykonuje. To samo dotyczy tekstu z nieznaną sekwencją ucieczki, który dane miejsce odczytuje jako wyrażenie regularne lub ścieżkę Windows (`"\d+"`, `"C:\temp\new"`): program odczytujący jest uruchamiany z pokazaną poprawką, a program zmieniający jest proponowany jako kandydat.

<!-- schema: response -->
```json
{
  "nurt": "1.0",
  "report": {
    "nurt": "1.0",
    "status": "failed",
    "kind": "remove",
    "error": {
      "code": "E_AMBIGUOUS",
      "message": "the target of the change is ambiguous",
      "candidates": [
        {"rank": 1, "ref": "c1", "program": "remove \"/home/user/app/build\"", "why": "the directory is named by the program"},
        {"rank": 2, "ref": "c2", "program": "remove \"/home/user/app/build/*\"", "why": "the contents of the directory only"}
      ]
    }
  }
}
```

Model lub host w imieniu użytkownika akceptuje jednego kandydata wywołaniem zawierającym wyłącznie `accept`:

<!-- schema: arguments -->
```json
{"accept": "c1"}
```

Kandydat jest uruchamiany dokładnie w takiej postaci, w jakiej został wymieniony. Kandydat, który wygasł lub jest nieznany, daje `E_EXPIRED_CANDIDATE`; program należy wysłać ponownie. Host, który prosi użytkownika o potwierdzenie, pokazuje `program` i `why` każdego kandydata.

## Strony {#pages}

Wynik dłuższy niż `limit` rekordów kończy się `report.next_cursor`. Następną stronę odczytuje się wywołaniem zawierającym wyłącznie `cursor` (i opcjonalnie `limit`):

<!-- schema: arguments -->
```json
{"cursor": "p-7f3a-2", "limit": 100}
```

Odczyt strony nigdy nie uruchamia ponownie programu zmieniającego. Host zbierający wszystkie strony wywołuje `page` (`nurt serve`) lub powtarza wywołanie, aż `next_cursor` przestanie występować. `max_chars` ogranicza długość pojedynczych pól tekstowych; `save_to` zapisuje pełny wynik do pliku zamiast do odpowiedzi.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
