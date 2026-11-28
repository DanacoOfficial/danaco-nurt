// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.section-pages.pl
// Responsibility:   The Polish content of the landing pages of the sections. Every statement summarises a page of the section; figures that the build counts come from website.src.data.facts.
// Dependencies:     website.src.data.section-pages.types

import type { SectionPagesContent } from './types';

export const sectionPagesPl: SectionPagesContent = {
  common: { indexEyebrow: 'Wszystkie strony', indexTitle: 'Każda strona tego działu' },

  product: {
    program: {
      eyebrow: 'Czym jest',
      title: 'Powłoka, w której program jest potokiem typowanych rekordów',
      lead: 'Jedno narzędzie do plików, tekstu, dokumentów strukturalnych, kodu źródłowego, kontroli wersji, procesów i systemu operacyjnego, dostarczane jako jeden plik wykonywalny na system.',
      points: [
        { title: 'Sprawdzone przed uruchomieniem', text: 'Nazwy, parametry i typy są weryfikowane względem katalogu poleceń, zanim cokolwiek się zmieni. Błąd ma stabilny kod, położenie i poprawkę.' },
        { title: 'Cofanie jest częścią języka', text: 'Zmiany w plikach są zapisywane w dzienniku. Operację przegląda się, cofa i ponawia poleceniami języka.' },
        { title: 'Raport z każdego uruchomienia', text: 'Każde uruchomienie zwraca raport skutków w postaci tekstu, JSON albo Markdown, ze schematem opublikowanym w specyfikacji.' },
        { title: 'Operacje wbudowane w program', text: 'Operacje na plikach, tekście, archiwach, dokumentach, kodzie, kontroli wersji, sieci i systemie są zaimplementowane w programie i nie wywołują zewnętrznych narzędzi.' },
      ],
      programs: [
        {
          title: 'nurt: 20 największych plików Go zmienionych w ciągu ostatniej doby',
          code: 'find "/home/user/project" glob: "**/*.go"\n  |> filter size > 1mb and modified > ago(24h)\n  |> sort size order: desc\n  |> first 20',
          caption: 'Każdy etap przyjmuje rekordy i zwraca rekordy, więc filter i sort odwołują się do pola po nazwie.',
        },
        {
          title: 'nurt: zamiana, która podaje oczekiwaną liczbę plików',
          code: 'search /oldName\\(/ "/srv/app/src" glob: "**/*.go"\n  |> assert paths: 12\n  |> replace "newName("',
          caption: 'Program zatrzymuje się z błędem E_ASSERT przed jakąkolwiek zmianą, chyba że wyszukiwanie znalazło dokładnie 12 plików.',
        },
        {
          title: 'nurt: przegląd dziennika i cofnięcie operacji',
          code: 'history limit: 5\nundo 18',
          caption: 'Dziennik wypisuje operacje; undo przywraca stan sprzed operacji 18.',
        },
      ],
    },
    numbers: { eyebrow: 'W pudełku', title: 'Jeden plik wykonywalny, cały katalog' },
    map: {
      eyebrow: 'Na czym pracują polecenia',
      title: 'Jedenaście obszarów pracy, jedna składnia',
      lead: 'Katalog poleceń jest pogrupowany według obiektu pracy. Każde polecenie ma stronę w dokumentacji.',
      action: 'Wszystkie możliwości',
      cells: [
        { icon: 'folder-tree', title: 'Pliki i katalogi', code: 'list find info read lines write copy move remove rename create attributes', note: 'Każda zmiana przechodzi przez dziennik, więc można ją cofnąć.' },
        { icon: 'file-pen', title: 'Tekst', code: 'search replace insert edit diff patch hash convert', note: 'Tekst dosłowny, wyrażenia regularne lub wzorce składni; edit stosuje listę zmian do jednego pliku w trybie „wszystko albo nic”.' },
        { icon: 'table', title: 'Rekordy', code: 'filter select sort first last skip count group summarize distinct', note: 'Pracują na rekordach tworzonych przez dowolne polecenie źródłowe.' },
        { icon: 'braces', title: 'Dokumenty strukturalne', code: 'set query sql', note: 'JSON, YAML, TOML, XML, INI, CSV i inne według ścieżki klucza; set zachowuje układ dokumentu.' },
        { icon: 'file-code', title: 'Kod źródłowy', code: 'code.symbols code.definition code.references code.rename', note: 'Trzynaście poleceń, które wypisują symbole, znajdują definicje i odwołania oraz zmieniają nazwy według składni.' },
        { icon: 'git-branch', title: 'Kontrola wersji', code: 'git.status git.diff git.commit git.rebase git.push', note: 'Dwadzieścia dwa polecenia, które czytają i zmieniają repozytoria wewnątrz programu.' },
        { icon: 'archive', title: 'Archiwa', code: 'archive.list archive.extract archive.create compress decompress', note: 'Archiwa i pliki skompresowane.' },
        { icon: 'globe', title: 'Sieć', code: 'http.request http.download network.ping network.resolve', note: 'Pobranie zapisuje plik i można je cofnąć; żądanie zmieniające jest w raporcie oznaczone jako nieodwracalne.' },
        { icon: 'activity', title: 'Procesy', code: 'run jobs retry stop processes ports suspend resume', note: 'run uruchamia program bez powłoki i kontynuuje go jako zadanie w tle.' },
        { icon: 'server', title: 'Administracja systemem', code: 'services start restart enable disable windows.cim', note: 'Trzydzieści cztery polecenia: usługi, konta, grupy, montowania, dyski, zapora, trasy, DNS i zegar.' },
        { icon: 'calendar', title: 'Harmonogramy', code: 'schedule.create schedule.list schedule.run schedule.history', note: 'Jedna składnia zastępuje crona, timery systemd, zadania launchd i Harmonogram zadań Windows.' },
      ],
    },
    audiences: {
      eyebrow: 'Dla kogo',
      title: 'Trzy wejścia do produktu',
      panels: [
        {
          icon: 'terminal',
          title: 'Ludzie przy terminalu',
          points: ['Błędne remove lub replace można cofnąć.', 'Program, który nie może być poprawny, jest odrzucany, zanim dotknie pliku.', 'Siedem konkretnych zadań z rzeczywistymi programami z książki kucharskiej.'],
          action: 'Przykłady zastosowań',
          path: '/product/use-cases/',
        },
        {
          icon: 'bot',
          title: 'Agenci AI i ich platformy',
          points: ['Ten sam silnik jako narzędzie przez Model Context Protocol, serwer JSON-RPC albo bibliotekę.', 'Każde wywołanie zwraca raport, który program potrafi odczytać.', 'Polityka i kontrola uprawnień przed uruchomieniem programu.'],
          action: 'Nurt dla agentów AI',
          path: '/product/ai-agents/',
        },
        {
          icon: 'route',
          title: 'Zespoły odchodzące od basha i PowerShella',
          points: ['Dokumentacja przypisuje nawyki obu powłok do ich postaci w Nurcie.', 'nurt migrate tłumaczy warunki, pętle, funkcje i parametry skryptów.', 'Tabela par poleceń i ściągawki dla każdej platformy.'],
          action: 'Migracja',
          path: '/product/migration/',
        },
      ],
    },
  },

  ai: {
    numbers: { eyebrow: 'Co jest dostarczane', title: 'Dane i definicje, które przychodzą z narzędziem' },
    audiences: {
      eyebrow: 'Dwie grupy odbiorców',
      title: 'Jedno narzędzie, którego model się uczy, a silnik agentowy je uruchamia',
      lead: 'Danaco Nurt daje modelowi jedno narzędzie w miejsce wielu narzędzi powłoki. Silnik sprawdza program, wykonuje skutek, zapisuje go i raportuje.',
      panels: [
        {
          icon: 'cpu',
          title: 'Dla laboratoriów modeli',
          text: 'Co otrzymuje laboratorium, które trenuje lub ocenia modele.',
          points: ['Definicja narzędzia w ośmiu formatach, po angielsku i po polsku.', 'Stabilne kody błędów z położeniem i poprawką, którą model stosuje.', 'Pięć paczek treningowych sprawdzonych przez silnik oraz zbiór zadań wstrzymanych.', 'Wersjonowane interfejsy, na które celuje wytrenowany model.'],
          action: 'Dla laboratoriów modeli',
          path: '/ai/model-labs/',
        },
        {
          icon: 'bot',
          title: 'Dla silników agentów',
          text: 'Co otrzymuje twórca silnika agentów lub platformy agentowej.',
          points: ['Cztery drogi integracji, które przyjmują to samo żądanie i zwracają tę samą odpowiedź.', 'Sesje, zdarzenia i anulowanie.', 'Polityka i kontrola uprawnień, praca wielu agentów i użycie zdalne.', 'Przykłady integracji i akcja instalacyjna dla ciągłej integracji.'],
          action: 'Dla silników agentów',
          path: '/ai/agent-engines/',
        },
      ],
    },
    call: {
      eyebrow: 'Jedno wywołanie narzędzia',
      title: 'Co się dzieje, gdy model wywołuje narzędzie',
      lead: 'Te same cztery rzeczy dzieją się niezależnie od drogi, którą program dociera do silnika.',
      steps: [
        { index: '1', name: 'check', text: 'Program jest analizowany i wiązany z katalogiem poleceń, zanim cokolwiek zrobi. Błąd zwraca stabilny kod, położenie w programie i poprawkę.', effect: 'bez skutku' },
        { index: '2', name: 'run', text: 'Etapy wymieniają typowane rekordy. Ten sam tekst programu działa w systemach Linux, Windows i macOS.', effect: 'zmiana', change: true },
        { index: '3', name: 'report', text: 'Uruchomienie zwraca raport nurt-report/1 w postaci tekstu, JSON, Markdown lub zwartej, ze stronicowaniem i ograniczonym rozmiarem; next zwraca dalszą część długiego wyniku.', effect: 'bez skutku' },
        { index: '4', name: 'undo', text: 'Zmiany w plikach są w dzienniku, więc pomyłkę modelu przegląda się i cofa poleceniem języka.', effect: 'wycofanie' },
      ],
    },
    routes: {
      eyebrow: 'Integracja',
      title: 'Cztery drogi do tego samego silnika',
      lead: 'Każda droga przyjmuje żądanie nurt-request/1 i zwraca odpowiedź nurt-response/1 z raportem skutków nurt-report/1.',
      table: {
        columns: ['Droga', 'Uruchomienie', 'Zastosowanie'],
        rows: [
          ['Wywołanie narzędzia', '`nurt exec "<program>"`', 'host lub zestaw narzędzi uruchamia jeden proces na wywołanie'],
          ['Serwer Model Context Protocol', '`nurt mcp` lub `nurt mcp --http <address>`', 'uruchamia go klient MCP; kod integracyjny nie jest pisany'],
          ['Serwer JSON-RPC', '`nurt serve --stdio`', 'silnik prowadzi własną pętlę agenta; jeden proces na sesję lub przestrzeń roboczą'],
          ['Interfejs C i powiązania', '`libnurt`, `nurt.h` oraz `bindings/`', 'krótkie wywołania bez skutków w procesie hosta; powiązania z językami Python, TypeScript, Go, Java i .NET'],
        ],
      },
    },
    data: {
      eyebrow: 'Do trenowania i oceny',
      title: 'Paczki treningowe i benchmark zadań',
      panels: [
        {
          icon: 'database',
          title: 'Paczki treningowe',
          points: ['pairs: 1457 przyjętych par zadania i programu oraz 629 odrzuconych programów z kodem błędu silnika.', 'chat i tool-calls: przyjęte pary jako rozmowy, 1309 do trenowania i 148 do walidacji.', 'tools: definicja narzędzia w 8 formatach i 2 językach.', 'corpus: 251 dokumentów dokumentacji i specyfikacji.'],
          action: 'Paczki treningowe',
          path: '/ai/training-packages/',
        },
        {
          icon: 'gauge',
          title: 'Benchmark zadań',
          points: ['nurt-bench porównuje Danaco Nurt z bashem i PowerShell 7 jako narzędzie agenta AI.', '720 zadań w 12 dziedzinach: 648 publicznych i 72 wstrzymane.', 'O wyniku każdego zadania rozstrzyga wyrocznia.', 'Protokół i polecenia, które laboratorium uruchamia na własnych modelach.'],
          action: 'Benchmark',
          path: '/ai/benchmark/',
        },
      ],
    },
  },

  quality: {
    numbers: { eyebrow: 'Zmierzone', title: 'Wyniki przebiegów testów' },
    checks: {
      eyebrow: 'Co jest sprawdzane',
      title: 'Każda zmiana przechodzi te same kontrole',
      lead: 'Przepływy formatują kod i sprawdzają go lintem, budują i testują przestrzeń roboczą na Linuksie, macOS i Windows, mierzą pokrycie wierszy, budują dokumentację i oceniają politykę zależności.',
      action: 'Pełna mapa kontroli',
      table: {
        columns: ['Kontrola', 'Co weryfikuje', 'Wynik'],
        rows: [
          ['Testy przestrzeni roboczej', 'zachowanie skrzynek, poziom jednostkowy i integracyjny', '13 965 zaliczonych (wersja 0.1.1, Linux, 27 listopada 2026)'],
          ['Testy na Windows 11', 'ten sam zestaw na Windows', '13 455 zaliczonych, żaden niezaliczony (wersja 0.1.1)'],
          ['Testy różnicowe względem git', 'polecenia `git.*` dają stan, który daje `git`, a ich cofnięcie przywraca stan początkowy', '775 z 775 zgodnych (wersja 0.1.1)'],
          ['Zgodność wiązań', 'Go, Node, .NET, Java i Python wywołują silnik tak, jak opisuje specyfikacja', 'wiązania przechodzą; Python 136 z 136 (wersja 0.1.1)'],
          ['Scenariusze na czystym systemie', 'archiwa wydania działają na minimalnym Debianie 13', '19 scenariuszy zaliczonych; archiwa `gnu` i `musl` zaliczone (wersja 0.1.1)'],
          ['Pokrycie wierszy', 'udział wierszy wykonywanych przez testy', 'zadanie wymaga 80 procent dla przestrzeni roboczej i 90 procent dla sześciu skrzynek krytycznych'],
          ['Lint i format', 'formatowanie, ostrzeżenia kompilatora i clippy', '`fmt` i `clippy -D warnings` bez uwag na Linuksie i Windows (wersja 0.1.1)'],
          ['Polityka zależności', 'licencje, zakazy, źródła i zalecenia bezpieczeństwa', '`cargo deny` bez błędów; `cargo audit` kod wyjścia 0 (wersja 0.1.1)'],
          ['Testy pakietów', 'instalatory, pakiety i manifesty', '174 z 174 (wersja 0.1.1)'],
        ],
      },
    },
    areas: {
      eyebrow: 'Siedem obszarów',
      title: 'Gdzie opisany jest każdy rodzaj dowodu',
      cells: [
        { slug: 'testing', icon: 'flask-conical', title: 'Testowanie', note: 'Zestawy, platformy, testy z uprawnieniami i woluminami, aarch64 oraz scenariusze na archiwach wydania.' },
        { slug: 'conformance', icon: 'check-check', title: 'Zgodność', note: 'Porównanie z gitem, specyfikacja, zestaw zgodności wiązań i program zgodności.' },
        { slug: 'performance', icon: 'gauge', title: 'Wydajność', note: 'Zmierzone zmiany w 0.1.2 i 0.1.1, limity egzekwowane przez testy i testy skali.' },
        { slug: 'reliability', icon: 'hard-drive', title: 'Niezawodność', note: 'Co dziennik gwarantuje po awarii i na pełnym dysku oraz które skutki są oznaczane jako nieodwracalne.' },
        { slug: 'security', icon: 'shield-check', title: 'Bezpieczeństwo', note: 'Model bezpieczeństwa, polityka uprawnień, zgłaszanie podatności i kontrole w ciągłej integracji.' },
        { slug: 'supply-chain', icon: 'package-check', title: 'Łańcuch dostaw i wydania', note: 'Polityka zależności, sposób budowy, sum kontrolnych i podpisywania archiwów oraz ich weryfikacja.' },
        { slug: 'engineering-standards', icon: 'list-checks', title: 'Standardy inżynierskie', note: 'Styl kodu, konwencja commitów, nagłówki licencyjne i kontrole, które je egzekwują.' },
      ],
    },
  },

  architecture: {
    flow: {
      eyebrow: 'Jeden kierunek',
      title: 'Program przechodzi przez silnik w dziewięciu krokach',
      lead: 'Tekst jest parsowany do drzewa składni, drzewo jest wiązane z katalogiem poleceń i sprawdzane, powstaje plan, moduł wykonawczy uruchamia etapy, polecenia zmieniają świat przez dziennik, a raport opisuje skutki.',
      action: 'Silnik, krok po kroku',
      steps: [
        { index: '0', name: 'parse', text: 'Tekst jest korygowany i parsowany do drzewa powierzchniowego.', effect: 'brak' },
        { index: '1', name: 'bind', text: 'Drzewo jest wiązane z katalogiem; statyczne sprawdzenie każdego etapu.', effect: 'brak' },
        { index: '2', name: 'read', text: 'Źródła i przekształcenia tworzą rekordy.', effect: 'tylko odczyt' },
        { index: '3', name: 'check', text: 'Sprawdzenia przed skutkiem (assert).', effect: 'brak' },
        { index: '4', name: 'plan', text: 'Etap ze skutkiem wylicza swoje cele; tu kończy się preview.', effect: 'tylko odczyt' },
        { index: '5', name: 'apply', text: 'Etap ze skutkiem zmienia świat przez transakcję dziennika.', effect: 'zmiana', change: true },
        { index: '5a', name: 'after', text: 'Etapy po skutku odczytują rekordy skutku.', effect: 'brak' },
        { index: '6', name: 'verify', text: 'Uruchamia się potok verify; niepowodzenie cofa skutek.', effect: 'wycofanie' },
        { index: '7', name: 'report', text: 'Budowany jest raport skutków.', effect: 'brak' },
      ],
    },
    entries: {
      eyebrow: 'Punkty wejścia',
      title: 'Pięć wejść, jeden silnik',
      lead: 'Punkty wejścia same nie implementują języka. Żądanie, które przychodzi dowolnym z nich, jest tym samym dokumentem, nurt-request/1.',
      items: [
        { command: 'nurt exec', name: 'Program wiersza poleceń', note: 'Plik wykonywalny nurt i interfejs Rust produktu.' },
        { command: 'nurt mcp', name: 'Serwer Model Context Protocol', note: 'Narzędzia i zasoby MCP, definicje narzędzia nurt.' },
        { command: 'nurt serve', name: 'Serwer protokołu', note: 'Protokół nurt-rpc/1 dla hostów, które prowadzą własną pętlę.' },
        { command: 'nurt lsp', name: 'Serwer języka', note: 'Diagnostyka, uzupełnianie i formatowanie plików .nurt.' },
        { command: 'libnurt', name: 'Interfejs C i wiązania', note: 'ABI języka C, nurt.h oraz wiązania dla Go, Node.js, .NET, Javy i Pythona.' },
      ],
      core: { name: 'Engine · nurt-core', text: 'Każdy punkt wejścia wywołuje tę samą fasadę Rust, Engine ze skrzynki nurt-core.' },
    },
    layers: {
      eyebrow: 'Przestrzeń robocza',
      title: 'Warstwy i ich skrzynki',
      lead: 'Danaco Nurt to przestrzeń robocza Cargo. Dozwolone zależności między jej skrzynkami są danymi w tables/crate-graph.toml, a test kończy się niepowodzeniem, gdy skrzynka deklaruje zależność, której tabela nie dopuszcza.',
      action: 'Każda skrzynka z jej opisem',
      table: {
        columns: ['Warstwa', 'Rola', 'Skrzynki'],
        rows: [
          ['Warstwa wejściowa języka', 'analizator leksykalny, parsery trzech wariantów zapisu, drukarka kanoniczna, korekta zapisu, pomoc, migracja skryptów powłoki', '`nurt-syntax` `nurt-correct` `nurt-help` `nurt-migrate`'],
          ['Katalog, sprawdzanie i planowanie', 'rejestr poleceń, wiązanie drzew składni z katalogiem, planowanie wykonania', '`nurt-catalog` `nurt-bind` `nurt-plan`'],
          ['Moduł wykonawczy', 'strumieniowy moduł wykonawczy potoków i etapy rekordów', '`nurt-exec`'],
          ['Dziennik i system plików', 'dziennik operacji, wolne miejsce dla dziennika, warstwa systemu plików zależna od platformy', '`nurt-journal` `nurt-space` `nurt-fs`'],
          ['Raport', 'model raportu skutków i jego renderery', '`nurt-report`'],
          ['Silnik i usługa', 'fasada silnika, usługa w tle', '`nurt-core` `nurt-service`'],
          ['Program wiersza poleceń', 'plik wykonywalny `nurt` i interfejs Rust produktu', '`nurt-cli` `nurt`'],
          ['Serwer Model Context Protocol', 'narzędzia i zasoby MCP, definicje narzędzia `nurt`', '`nurt-mcp` `nurt-tooldef`'],
          ['Serwer języka', 'diagnostyka, uzupełnianie i formatowanie plików `.nurt`', '`nurt-langserver`'],
          ['Interfejs C i wiązania', 'ABI języka C i dodatek Node.js', '`nurt-ffi` `nurt-node`'],
        ],
      },
    },
  },

  design: {
    idea: {
      eyebrow: 'Idea',
      title: 'Wstecz w srebrze, naprzód w złocie',
      lead: 'Znak składa się z dwóch identycznych szewronów. Złoty szewron to srebrny obrócony o 180 stopni, więc oba są jednym rysunkiem użytym dwa razy.',
      imageAlt: 'Znak Danaco Nurt: srebrny szewron wskazujący wstecz i złoty szewron wskazujący naprzód.',
      imageCaption: 'Render znaku, identyfikacja produkcyjna 1.0.0.',
      action: 'Projekt wizualny Nurt',
      points: [
        { title: 'Srebro: krok wstecz', text: 'Lewy szewron jest srebrny i wskazuje wstecz: cofnięcie.' },
        { title: 'Złoto: krok naprzód', text: 'Prawy szewron jest złoty i wskazuje naprzód: zmiana.' },
        { title: 'Nurt między nimi', text: 'Kanał między równoległymi ramionami to nurt, szeroki na 5,5 jednostki na całej wstędze, czyli połowę kreski równej 11 jednostek.' },
        { title: 'Jeden kąt', text: 'Ramiona obu szewronów mają nachylenie 3 : 4 (36,87 stopnia), kąt każdej ukośnej linii identyfikacji.' },
      ],
    },
    colours: {
      eyebrow: 'Kolor',
      title: 'Sześć kolorów niesie identyfikację',
      lead: 'Każdy kolor ma jedną rolę. Na graficie srebro ma kontrast 14,9 : 1, a złoto 8,1 : 1.',
      action: 'Znak, kolor i krój pisma',
      swatches: [
        { name: 'Grafit', value: '#0B0C0E', role: 'tło główne' },
        { name: 'Srebro', value: '#DDE1E6', role: 'krok wstecz' },
        { name: 'Złoto', value: '#C9A15A', role: 'krok naprzód' },
        { name: 'Atrament', value: '#2B2F36', role: 'atrament na jasnym tle' },
        { name: 'Złoto głębokie', value: '#A3722A', role: 'złoto na jasnym tle, znaki i linie' },
        { name: 'Papier', value: '#F6F7F8', role: 'tło jasne' },
      ],
    },
    type: {
      eyebrow: 'Krój pisma',
      title: 'Firma się cofa, produkt idzie naprzód',
      lead: 'Nazwa jest złożona krojem Geist, neutralną geometryczną groteską na licencji SIL Open Font License. Danaco jest złożone w grubości 400, a Nurt w grubości 700, tak jak szewrony.',
      mono: 'find "/home/user/app" |> first 5',
      note: 'Geist Mono jest krojem terminala i kodu.',
    },
    files: {
      eyebrow: 'Trzy strony',
      title: 'Od idei do plików',
      panels: [
        { icon: 'eye', title: 'Projekt wizualny Nurt', text: 'Nazwa, znak z dwóch szewronów, kąt nurtu, krój pisma, tła i miejsca, w których identyfikacja występuje w produkcie.', action: 'Czytaj', path: '/design/overview/' },
        { icon: 'sliders-horizontal', title: 'Znak, kolor i krój pisma', text: 'Konstrukcja znaku na siatce 96 x 96 jednostek, rysunki dla małych rozmiarów, paleta kolorów z kontrastem i krój pisma.', action: 'Czytaj', path: '/design/mark-and-colour/' },
        { icon: 'download', title: 'Pliki identyfikacji', text: 'Księga identyfikacji oraz pliki wektorowe i rastrowe znaku do pobrania.', action: 'Otwórz', path: '/design/identity/' },
      ],
    },
  },

  project: {
    release: {
      eyebrow: 'Bieżące wydanie',
      title: 'Wersja 0.1.2',
      version: '0.1.2',
      date: '28 listopada 2026',
      text: 'Kreator instalacji dla Windows, wyszukiwanie i zamiana wyrażeniem regularnym w czasie proporcjonalnym do liczby dopasowań, kontrola statyczna przyjmująca każdy poprawny literał oraz dziennik działający na woluminie danych macOS.',
      actions: [
        { label: 'Wydania', path: '/project/releases/' },
        { label: 'Stan bieżący', path: '/project/status/' },
      ],
      timeline: [
        { label: '2026-11-28', title: 'Wersja 0.1.2', text: 'Kreator instalacji dla Windows, szybsze wyszukiwanie i zamiana, dziennik na woluminie danych macOS.' },
        { label: '2026-11-27', title: 'Wersja 0.1.1', text: 'Konstrukcje sterujące, polecenia administracyjne, podniesienie uprawnień, polecenie next i szybszy dziennik przy wielu małych plikach.' },
        { label: '2026-11-22', title: 'Wersja 0.1.0', text: 'Pierwsze wydanie: silnik, język, katalog poleceń, dziennik, program wiersza poleceń, serwer Model Context Protocol, serwer języka i interfejs C.' },
      ],
    },
    numbers: { eyebrow: 'Repozytorium', title: 'Z czego składa się projekt' },
    systems: {
      eyebrow: 'Platformy',
      title: 'Trzy systemy operacyjne, po jednym pliku wykonywalnym',
      lead: 'nurt w Linuksie i macOS, nurt.exe w Windows. Plik wykonywalny nie ma zależności uruchomieniowych poza samym systemem.',
      table: {
        columns: ['System', 'Architektury', 'Archiwa wydania 0.1.2'],
        rows: [
          ['Linux', 'x86-64, aarch64', 'glibc i statyczne musl'],
          ['Windows', 'x86-64, aarch64', 'MSVC, jako archiwum i jako kreator instalacji'],
          ['macOS', 'x86-64, aarch64', 'budowa ze źródeł'],
        ],
      },
    },
    pages: {
      eyebrow: 'Projekt',
      title: 'Wydania, zasady i ludzie',
      groups: [
        { title: 'Wydanie', slugs: ['releases', 'status', 'versioning'] },
        { title: 'Udział', slugs: ['contributing', 'support'] },
        { title: 'Warunki i ludzie', slugs: ['license', 'about'] },
      ],
    },
  },
};
