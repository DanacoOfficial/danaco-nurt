<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/stages.md -->
<!-- Responsibility:   Documents Stage interface. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Interfejs etapu {#stage-interface}

Ten dokument określa, jak polecenie języka Nurt jest implementowane i
wykonywane. Postacią kontraktu w Rust jest moduł `stage` skrzynki
`nurt-contracts`; fakty o zegarze i hoście znajdują się w module `env`.
Implementacja języka w innym języku programowania stosuje te same kroki i te
same reguły.

## 1. Kroki wykonania {#1-execution-steps}

Potok jest wykonywany w stałych krokach. Kroki od 0 do 4 nie mają skutku w
świecie. Numer kroku pojawia się w pozycjach błędów jako tekst.

| Krok | Nazwa | Co się dzieje | Skutek w świecie |
|---|---|---|---|
| `0` | parsowanie | tekst jest poprawiany i parsowany do drzewa powierzchniowego | brak |
| `1` | wiązanie | drzewo powierzchniowe jest wiązane z katalogiem; statyczne kontrole każdego etapu | brak |
| `2` | odczyt | źródła i transformacje tworzą rekordy; rekordy plików zawierają `etag` | tylko odczyty |
| `3` | kontrola | kontrole przed skutkiem (`assert`) | brak; niepowodzenie niczego nie zmienia |
| `4` | plan | etap skutku oblicza swoje cele i planowany skutek; `preview` kończy się tutaj | tylko odczyty |
| `5` | zastosowanie | etap skutku zmienia świat przez transakcję dziennika | zmiana |
| `5a` | po | etapy po skutku odczytują rekordy skutku | brak |
| `6` | weryfikacja | uruchamiany jest potok `verify`; niepowodzenie wycofuje skutek | wycofanie |
| `7` | raport | budowany jest raport skutku | brak |

## 2. Rozdzielanie według fazy {#2-dispatch-by-phase}

Mechanizm wiązania przypisuje każdemu powiązanemu etapowi fazę. Faza wybiera interfejs
wywoływany przez wykonawcę:

| Faza | Krok | Interfejs |
|---|---|---|
| `source`, `transform` | 2 | `ReadStage::execute` |
| `check` | 3 | `ReadStage::execute` |
| `mutate`, `run` | 4 | `EffectStage::plan` |
| `mutate`, `run` | 5 | `EffectStage::apply` |
| `after` | 5a | `ReadStage::execute` |
| `verify` | 6 | wykonawca |

Implementacja polecenia (`StageImpl`) udostępnia interfejs odczytu, interfejs
skutku albo oba. Polecenie, którego skutek zależy od argumentu (jedno działanie
odczytuje, inne zmienia), udostępnia oba; mechanizm wiązania przypisuje fazę na podstawie
argumentów.

`StageImpl::check` jest uruchamiane w kroku 1, po wiązaniu, z dostępem
wyłącznie do specyfikacji katalogu, faktów o hoście i konfiguracji. Może
odrzucać argumenty, których typy katalogu nie potrafią wyrazić, i zgłasza fakty
statyczne (`StaticInfo`): czy etap czeka, z jakimi punktami końcowymi sieci się
połączy, jakie programy zewnętrzne uruchomi, jakie ścieżki nazywa oraz szczegóły
właściwe dla polecenia. Fakty te zwraca kontrola programu bez jego wykonywania,
aby aplikacja osadzająca mogła zastosować własne reguły przed wykonaniem.

## 3. Rejestr {#3-registry}

Jedno polecenie może mieć kilka implementacji, na przykład `remove` dla wpisów
plików, dla elementów dokumentów i dla zadań, dostarczanych przez różne
skrzynki. Każda implementacja opisuje się za pomocą `StageDescriptor`: nazwa
polecenia, informacja, czy może rozpocząć potok, oraz rodzaje rekordów, które
przyjmuje. Rejestr wybiera implementację według nazwy polecenia i rodzaju
rekordu zadeklarowanego przez katalog dla wyjścia poprzedniego etapu. Gdy żadna
implementacja nie przyjmuje tego rodzaju, wiązanie kończy się błędem `E_TYPE`
wraz z listą przyjmowanych rodzajów. Strumień, którego rekordy mają rodzaje
obsługiwane przez różne implementacje, kończy się błędem `E_TYPE`, gdy do etapu
dotrze pierwszy rekord innego rodzaju; program jest dzielony przez wywołującego.

## 4. Rekordy i strumienie {#4-records-and-streams}

Rekordy przepływają między etapami w `RecordStream`, leniwym iteratorze
`Result<Record>`:

* Etap pobiera rekordy z wejścia tylko w miarę potrzeby. Potok trzyma więc całe
  wejście w pamięci tylko wtedy, gdy etap musi to robić (na przykład `sort`,
  `last`, `group`); taki etap zbiera rekordy z ograniczeniem, a przekroczenie
  `limits.max_records` kończy się błędem `E_LIMIT`. Rekordy nigdy nie są
  odrzucane po cichu.
* Element błędu kończy program. Strumień nie daje niczego po swoim pierwszym
  błędzie.
* Elementy pominięte bez błędu (nieczytelny wpis podczas listowania, niedostępny
  kanał dziennika) są zapisywane przez kontekst jako istotne wpisy raportu; nie
  kończą programu.
* Wyjściem etapu są rekordy, pojedyncza wartość (na przykład wynik `count`) albo
  nic. Może zawierać rekord nagłówka opisujący cały strumień (na przykład
  `test_run` przed rekordami `test_result`), kolejność strumienia znaną jego
  producentowi (aby następujący po nim `sort` według tych samych kluczy nie
  zbierał strumienia) oraz wskazówkę prezentacji dla renderera tekstu (`tree`,
  `sizes`). Etapy przepuszczające zachowują nagłówek, kolejność i wskazówkę.

## 5. Argumenty {#5-arguments}

Etap otrzymuje swoje powiązane argumenty w kolejności sygnatury. Każdy parametr
sygnatury jest obecny; wartości domyślne wstawione przez mechanizm wiązania są oznaczone.
Wartości literałów, wartości domyślne i odwołania do nazwanych wyników są
obliczane przed uruchomieniem etapu. Wyrażenia dla poszczególnych rekordów etap
oblicza dla każdego rekordu wejściowego za pomocą ewaluatora kontekstu. Argumenty
potokowe (kontrola `verify`, `write from:`) są uruchamiane przez wykonawcę
potoków kontekstu.

## 6. Etapy skutku {#6-effect-stages}

Etap skutku (zmiana, działanie zewnętrzne lub uruchomiony program) ma dwa kroki.

### Plan (krok 4) {#plan-step-4}

Etap pochłania swoje wejście, zapisuje znacznik encji każdego celu w stanie
odczytanym i zwraca plan skutku: planowane zmiany z różnicami, istotne skutki i
skutki nieodwracalne znane z góry, rekordy opisujące skutek bez celów plikowych
(na przykład wiersz poleceń i katalog programu), informację, czy operacja jest
masowa, oraz prywatny stan implementacji. Krok planu może odczytywać świat, w
tym żądania sieciowe tylko do odczytu potrzebne do obliczenia planu; niczego nie
tworzy, nie zmienia, nie wysyła ani nie uruchamia. Podgląd zgłasza plan i
kończy działanie. Ten sam plan jest używany przez krok zastosowania, więc podgląd
pokazuje dokładnie te cele, które zmieni wykonanie. Stan prywatny zawiera to, co
jest potrzebne do wytworzenia nowej zawartości, a nie samą zawartość.

### Zastosowanie (krok 5) {#apply-step-5}

Wykonawca dopuszcza operację i otwiera transakcję dziennika. Etap otrzymuje plan
i transakcję; każda zmiana danych użytkownika przechodzi przez transakcję:
wszystkie cele są deklarowane, a ich poprzedni stan jest zachowywany przed
pierwszą zmianą, następnie zmiany są stosowane, a dziennik sprawdza znaczniki
encji (`E_STALE` przy niezgodności). Skutki poza systemem plików są oznaczane w
transakcji jako nieodwracalne. Etap nigdy nie zatwierdza; wykonawca zatwierdza
przy powodzeniu i wycofuje przy niepowodzeniu lub anulowaniu. Etap zwraca
rekordy skutku (`effect`, `exec`, `job`, …), zmiany w postaci zastosowanej oraz
zaobserwowane istotne i nieodwracalne skutki. Zwrócone rekordy nie pożyczają
transakcji.

Gdy uruchomiony program trwa w tle po upływie swojego `wait`, wynik jest
oznaczany jako odłączony i zawiera rekord `job`; etapy po skutku nie są
uruchamiane.

## 7. Etapy po skutku {#7-stages-after-the-effect}

Etapy dozwolone po skutku (`assert`, `filter`, `select`, `first`, `last`, `skip`,
`count`, `sort`, `group`) są etapami odczytu wykonywanymi w kroku 5a na rekordach
skutku. Nigdy nie zmieniają świata. Nieudany `assert` w kroku 5a zgłasza
`E_ASSERT` z krokiem `5a` obok pełnego raportu skutku; skutek nie jest
wycofywany. Wycofuje tylko `verify` (krok 6).

## 8. Kontekst wykonania {#8-execution-context}

Etap sięga do świata wyłącznie przez swój kontekst wykonania:

| Element | Przeznaczenie |
|---|---|
| system plików | dostęp do plików tylko do odczytu; zapisy przechodzą wyłącznie przez transakcję |
| magazyn dziennika | polecenia dziennika (`history`, `undo`, `redo`, `journal.*`) |
| zegar | czas ścienny, czas monotoniczny, oczekiwanie |
| host | fakty o hoście i środowisko procesu silnika |
| konfiguracja, opcje żądania | obowiązujące ustawienia i opcje wywołania |
| anulowanie, termin | kooperacyjne anulowanie i limit czasu programu |
| limity | rekordy trzymane w pamięci, rozmiar plików tekstowych, wątki |
| katalog stanu | bufor, zadania i pamięci podręczne silnika |
| ewaluator, wykonawca potoków | wyrażenia dla poszczególnych rekordów i argumenty potokowe |
| usługi | rejestr usług innych grup poleceń (dokumenty, kod źródłowy, programy, listowanie plików, zapytania o repozytorium, kodeki tekstu, posiadacze plików, reguły przeglądu); usługa jest nieobecna, gdy jej grupa nie jest załadowana |
| rozszerzenia | dalsze obiekty współdzielone rejestrowane według typu |
| ramka | indeks, polecenie i krok uruchomionego etapu oraz zapotrzebowanie reszty potoku |
| zdarzenia | postęp i wyjście uruchomionych programów |
| uwagi | istotne wpisy raportu |

### Zapotrzebowanie {#demand}

Kontekst informuje etap, czego potrzebuje reszta potoku: pól używanych później
(kosztowne pola, które nie są używane, można pominąć), czy rekordy plików muszą
zawierać `etag` (następuje etap skutku lub kontrola znacznika encji) oraz ile
rekordów jest używanych później (etap może zakończyć działanie wcześniej).
Wyjście musi być poprawne dla dowolnego zapotrzebowania.

### Usługi {#services}

Gdy usługa jest nieobecna, etap obniża funkcjonalność tak, jak opisuje to
dokumentacja jego polecenia (na przykład zwykły widok zamiast konspektu) i
zapisuje istotny wpis; nie kończy się błędem.

### Środowisko {#environment}

Etapy odczytują zmienne środowiskowe wyłącznie przez host kontekstu, nigdy
bezpośrednio z procesu, aby aplikacja osadzająca mogła nadać każdemu wykonaniu
własne środowisko, a testy mogły je podmienić.

## 9. Anulowanie i limity {#9-cancellation-and-limits}

Anulowanie jest kooperacyjne. Kontekst przechowuje token anulowania wspólny dla
wszystkich etapów jednego wykonania; tokeny potomne są anulowane razem ze swoim
rodzicem i mogą być też anulowane samodzielnie (na przykład jedno zadanie w
tle). Wygrywa pierwsza przyczyna:

| Przyczyna | Błąd |
|---|---|
| zażądana przez wywołującego | `E_CANCELLED` |
| limit czasu programu | `E_TIMEOUT` |
| limit zasobów | `E_LIMIT` |
| zamknięcie silnika | `E_CANCELLED` |

Etap wywołuje punkt kontrolny kontekstu w długich pętlach i przed pracą
blokującą; punkt kontrolny kończy się niepowodzeniem, gdy token jest anulowany
lub termin minął (i wtedy anuluje token z przyczyną limitu czasu). Opakowanie
strumienia adapterem kontrolowanym wykonuje punkt kontrolny przed każdym
pobraniem. Oczekiwanie korzysta z zegara kontekstu i nigdy nie wykracza poza
termin. Zegar wirtualny kończy oczekiwania natychmiast, co umożliwia testowanie
limitów czasu bez czekania.

Anulowanie w trakcie kroku 5 wycofuje transakcję. Anulowanie przed krokiem 5 nie
ma skutku w świecie.

## 10. Zdarzenia {#10-events}

Zdarzenia są dostarczane w trakcie wykonywania programu. Każde zdarzenie ma swój
typ w składniku `t`:

| `t` | Zawartość |
|---|---|
| `progress` | `command`, `phase`, `done`, `total`, `bytes`, `message` |
| `job.started` | `id`, `label`, `command`, `cwd`, `pid`, `started` |
| `job.output` | `id`, `stream` (`stdout`, `stderr`), `cursor`, `lines`, `bytes` |
| `job.progress` | `id`, `percent`, `line` |
| `job.finished` | `id`, `state` (`exited`, `failed`, `stopped`, `timed_out`), `exit_code`, `signal`, `elapsed`, `output_file`, `changes` |
| `log` | `level`, `message`, `source` |

Każdy uruchomiony program ma identyfikator zadania od początku, więc te same
zdarzenia opisują program kończący się na pierwszym planie i program trwający w
tle. Silnik dodaje numer kolejny, czas i identyfikator żądania, gdy dostarcza
zdarzenie aplikacji osadzającej. Emitowanie zdarzenia nigdy nie blokuje
wykonania na długo; odbiorniki zdarzeń buforują je i dostarczają w kolejności.

## 11. Zegar i host {#11-clock-and-host}

Zegar udostępnia czas ścienny (nanosekundy od epoki Uniksa, UTC), czas
monotoniczny, który uwzględnia uśpienie i nigdy nie maleje, odczyty wszystkich
zegarów w jednej chwili (czas ścienny, identyfikator rozruchu, czas monotoniczny
z uśpieniem i bez niego, używane przez dziennik do mierzenia wieku wpisów) oraz
anulowalne oczekiwanie.

Host udostępnia fakty stałe (system operacyjny, wersja, architektura, nazwa
hosta, konto, katalogi domowy i tymczasowy, notacja ścieżek, reguły czasu
lokalnego, ustawienia regionalne, liczba procesorów) oraz zmienne środowiskowe
procesu silnika. Wartości zmieniające się w trakcie działania programu są
odczytywane przez polecenia, które je zgłaszają.

## 12. Współbieżność {#12-concurrency}

Interfejs jest synchroniczny. Każdy interfejs można bezpiecznie współdzielić
między wątkami; strumienie rekordów można przenosić między wątkami. Etap może
używać do `limits.threads` wątków do pracy równoległej i musi wytwarzać swoje
rekordy w deterministycznej kolejności, niezależnie od harmonogramowania.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
