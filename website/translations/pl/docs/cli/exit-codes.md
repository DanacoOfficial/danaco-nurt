<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cli/exit-codes.md -->
<!-- Responsibility:   Documents Exit codes. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Kody wyjścia {#exit-codes}

Kod wyjścia `nurt` odpowiada statusowi raportu programu i statusowi interfejsu jego błędu, dzięki czemu wiersz poleceń i interfejs C opisują te same wyniki.

| Kod | Nazwa | Znaczenie | Status interfejsu |
|---|---|---|---|
| 0 | sukces | Status raportu to `ok`, `planned` lub `running`, a raport nie zawiera błędu. | `NURT_OK` |
| 1 | niepowodzenie | Program zakończył się błędem języka lub wykonania albo nie dało się odczytać lub zapisać pliku. | `NURT_OK`, `NURT_E_ARG`, `NURT_E_IO` |
| 2 | użycie | Wiersz poleceń jest nieprawidłowy: nieznana opcja, brakująca lub nieprawidłowa wartość, brakujący argument. | |
| 3 | niedostępne | Podpolecenie nie wchodzi w skład tej kompilacji. | |
| 4 | przekroczenie czasu | Program przekroczył limit czasu (`E_TIMEOUT`). | `NURT_E_TIMEOUT` |
| 70 | wewnętrzny | Błąd wewnętrzny (`E_INTERNAL` lub `E_INTERNAL_CORRECTION`). | `NURT_E_INTERNAL` |
| 130 | anulowano | Program został anulowany (`E_CANCELLED`). | `NURT_E_CANCELLED` |

Raport ze statusem `partial`, `rolled_back` lub `failed` przyjmuje kod swojego błędu; bez błędu kodem jest 1. Raport ze statusem `ok`, `planned` lub `running`, który zawiera błąd, na przykład `E_ASSERT` sprawdzenia, które nie powiodło się w etapie 5a po zastosowaniu efektu, również przyjmuje kod tego błędu (1 dla `E_ASSERT`), aby skrypt nie kontynuował działania po nim. Problem dziennika, który nie przerywa operacji, nie jest błędem raportu (jest wymieniony jako `journal_error` w `irreversible`) i pozostawia kod 0. Silnik, którego nie można otworzyć, na przykład z powodu nieprawidłowej konfiguracji, przyjmuje kod swojego błędu.

Komunikaty wiersza poleceń są zapisywane na standardowe wyjście błędów w postaci `nurt: <message>`. Błąd języka jest zapisywany jako `<code>: <message>`, a po nim następują gotowe zamienniki programu.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
