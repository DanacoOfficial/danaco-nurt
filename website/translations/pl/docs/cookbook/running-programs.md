---
category: running-programs
title: Uruchamianie programów i zadania w tle
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/running-programs.md -->
<!-- Responsibility:   Documents Running programs and background jobs. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Uruchamianie programów i zadania w tle {#running-programs-and-background-jobs}

## Uruchomienie programu w katalogu {#run-a-program-in-a-directory}

Uruchom budowanie w katalogu projektu.

```nurt example
run ["make", "build"] cwd: "/home/user/app"
```

```text expected
exit_code
```

```bash
cd /home/user/app && make build
```

## Błąd, gdy kod zakończenia jest różny od zera {#fail-when-the-exit-code-is-not-zero}

Uruchom program i spraw, aby zakończył się błędem, o ile nie zakończy się kodem 0.

```nurt example
run ["go", "test", "./..."] cwd: "/home/user/app" |> assert exit_code: 0
```

```text expected
exit_code
```

## Przekazanie zmiennych środowiskowych {#pass-environment-variables}

Uruchom program z dodatkową zmienną środowiskową.

```nurt example
run ["npm", "test"] cwd: "/home/user/app" env: {"CI": "1"}
```

```text expected
exit_code
```

```bash
CI=1 npm test
```

## Ograniczenie czasu działania {#limit-the-run-time}

Zatrzymaj program, który działa dłużej niż minutę.

```nurt example
run ["make", "test"] cwd: "/home/user/app" timeout: 60s
```

```text expected
timed_out
```

## Wyświetlanie samych błędów z długiego wyjścia {#show-only-the-errors-of-a-long-output}

Pokaż wiersze wyglądające na błędy, pierwsze 5 wierszy i ostatnie 10 wierszy.

```nurt example
run ["npm", "test"] cwd: "/home/user/app" show: errors head: 5 tail: 10
```

```text expected
exit_code
```

```bash
npm test 2>&1 | tail -n 50
```

## Uruchomienie programu w tle {#start-a-program-in-the-background}

Uruchom długie budowanie i natychmiast wróć.

```nurt example
run ["npm", "run", "build"] cwd: "/home/user/app" background: true
```

```text expected
job
```

```bash
npm run build > out.log 2>&1 &
```

## Odczyt wyjścia zadania w tle {#read-the-output-of-a-background-job}

Odczytaj ostatnie 20 wierszy wyjścia zadania.

```nurt example
jobs "j1" tail: 20
```

```text expected
j1
```

## Oczekiwanie na zadanie {#wait-for-a-job}

Poczekaj do 30 sekund na zakończenie zadania.

```nurt example
jobs "j1" wait: 30s
```

```text expected
j1
```

## Wyświetlanie działających zadań {#list-the-running-jobs}

Wyświetl zadania, które nadal działają.

```nurt example
jobs state: running
```

```text expected
running
```

## Zatrzymanie zadania {#stop-a-job}

Zatrzymaj działające zadanie.

```nurt example
jobs "j1" |> stop
```

```text expected
j1
```

## Ponowienie nieudanego zadania {#retry-a-failed-job}

Uruchom nieudane zadanie ponownie z tymi samymi argumentami.

```nurt example
jobs "j1" |> retry
```

```text expected
j1
```

## Wyświetlanie plików zmienionych przez program {#list-the-files-a-program-changed}

Uruchom program formatujący i wyświetl pliki, które zmienił.

```nurt example
run ["gofmt", "-w", "."] cwd: "/home/user/app" watch: ["/home/user/app"]
```

```text expected
exit_code
```

## Pauza przez podaną liczbę sekund {#pause-for-a-number-of-seconds}

Poczekaj pięć sekund przed następnym wywołaniem.

```nurt example
sleep 5s
```

```text expected
5s
```

## Uruchomienie testów i odczyt niepowodzeń {#run-the-tests-and-read-the-failures}

Uruchom testy projektu i pokaż co najwyżej 10 niepowodzeń, każde z 5 wierszami komunikatu.

```nurt example
test.run "/home/user/app" failures: 10 message_lines: 5
```

```text expected
test_run
```

## Przekazanie pliku programowi {#feed-a-file-to-a-program}

Uruchom program z zawartością pliku jako jego standardowym wejściem.

```nurt example
run ["sort", "-u"] stdin: "/home/user/app/names.txt" cwd: "/home/user/app"
```

```text expected
exit_code
```

```bash
sort -u < /home/user/app/names.txt
```

## Zatrzymanie programu, który nie generuje wyjścia {#stop-a-program-that-produces-no-output}

Zatrzymaj instalację, która przez dwie minuty nic nie wypisuje.

```nurt example
run ["npm", "install"] cwd: "/home/user/app" idle_timeout: 2m
```

```text expected
exit_code
```

## Wyświetlanie samych ostrzeżeń i błędów z kontekstem {#show-only-warnings-and-errors-with-context}

Połącz strumień błędów z wyjściem i zachowaj wiersze pasujące do wzorca wraz z dwoma wierszami kontekstu.

```nurt example
run ["make"] cwd: "/home/user/app" merge_stderr: true filter: /warning|error/ context: 2
```

```text expected
exit_code
```

```bash
make 2>&1 | grep -E -C 2 'warning|error'
```

## Uruchomienie z czystym środowiskiem {#run-with-a-clean-environment}

Uruchom program bez środowiska sesji i z jedną zmienną.

```nurt example
run ["env"] inherit_env: none env: {"LANG": "C"} cwd: "/home/user/app"
```

```text expected
LANG
```

```bash
env -i LANG=C env
```

## Ograniczenie zasobów programu {#limit-the-resources-of-a-program}

Uruchom budowanie z niskim priorytetem i limitem pamięci.

```nurt example
run ["make", "-j4"] cwd: "/home/user/app" max_memory: 2gb priority: low
```

```text expected
exit_code
```

```bash
nice -n 19 make -j4
```

## Uruchomienie programu dla każdego pliku {#run-a-program-for-each-file}

Uruchom optymalizator obrazów dla każdego pliku PNG, po cztery naraz, po 8 plików na wywołanie.

```nurt example
find "/home/user/app/img" glob: "**/*.png" |> run ["optipng", "-o2"] parallel: 4 batch: 8 cwd: "/home/user/app/img"
```

```text expected
exit_code
```

```bash
find /home/user/app/img -name '*.png' -print0 | xargs -0 -n 8 -P 4 optipng -o2
```

## Nazwanie zadania w tle {#name-a-background-job}

Uruchom budowanie wydania jako zadanie z etykietą.

```nurt example
run ["make", "release"] cwd: "/home/user/app" background: true label: "release-build" description: "Release build of the application"
```

```text expected
release-build
```

## Raportowanie postępu i podsumowania uruchomienia {#report-progress-and-a-summary-of-a-run}

Raportuj wiersze z wartościami procentowymi uruchomienia i zachowaj wiersz podsumowujący wynik.

```nurt example
run ["cargo", "test"] cwd: "/home/user/app" progress: /\d+%/ summary: [/test result/]
```

```text expected
summary_lines
```

## Odczyt błędów zadania {#read-the-errors-of-a-job}

Odczytaj wiersze wyjścia zadania pasujące do wzorca, z dwoma wierszami kontekstu.

```nurt example
jobs "j1" view: output filter: /error/ context: 2
```

```text expected
j1
```

## Odczyt wyniku zakończonego zadania {#read-the-result-of-a-finished-job}

Odczytaj kod zakończenia i czas trwania zakończonego zadania.

```nurt example
jobs "j1" view: result
```

```text expected
exit_code
```

## Łagodne zatrzymanie zadania {#stop-a-job-gently}

Przerwij zadanie i daj mu dziesięć sekund na zakończenie, zanim zostanie zabite.

```nurt example
jobs "j1" |> stop signal: interrupt grace: 10s
```

```text expected
j1
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
