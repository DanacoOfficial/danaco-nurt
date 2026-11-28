// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.data.home-page.pl
// Responsibility:   Content of the home page in Polish, a translation of the English content with the same structure.
// Dependencies:     website.src.data.home-page.types

import type { HomeContent } from './types';

export const homePl: HomeContent = {
  hero: {
    eyebrow: 'Powłoka poleceń i język',
    title: 'Każdą zmianę można cofnąć.',
    lead: 'Danaco Nurt to powłoka poleceń z własnym językiem do pracy z plikami, tekstem, dokumentami strukturalnymi, kodem źródłowym, kontrolą wersji, procesami i systemem operacyjnym. Programy to potoki typowanych rekordów, sprawdzane przed uruchomieniem i opisane raportem skutków. Zmiany w plikach trafiają do dziennika i można je przejrzeć oraz cofnąć.',
    note: 'Dla ludzi i agentów AI. Jedno narzędzie dla Linuksa, Windows i macOS.',
    sceneLabel: 'Znak Danaco Nurt ze szczotkowanego srebra i złota: szewron wstecz i szewron naprzód',
    sceneCaption: 'Wstecz i naprzód: zmiana i jej cofnięcie',
  },
  pillars: {
    eyebrow: 'Dlaczego Nurt',
    title: 'Powłoka, która odpowiada za to, co robi',
    lead: 'Cały projekt wynika z jednej obietnicy: widzisz, co program zrobi i co zrobił, i możesz to cofnąć.',
    cards: [
      { icon: 'undo-2', title: 'Cofanie jest częścią języka', text: 'Zmiany w plikach trafiają do dziennika. Operację można przejrzeć, cofnąć i ponowić. Zmiany poza systemem plików, takie jak zmiana usługi, są zapisywane jako nieodwracalne.', path: '/product/journal-and-undo/' },
      { icon: 'scan-search', title: 'Sprawdzone przed uruchomieniem', text: 'Nazwy, parametry i typy są weryfikowane względem katalogu poleceń, zanim cokolwiek się zmieni. Błędy mają stabilny kod, położenie i poprawkę.', path: '/product/language/' },
      { icon: 'braces', title: 'Rekordy zamiast tekstu', text: 'Etapy wymieniają rekordy z nazwanymi polami (pliki, wiersze, dopasowania, procesy) zamiast nieustrukturyzowanego tekstu, więc potok nie zależy od formatu wyjścia narzędzia.', path: '/product/language/' },
      { icon: 'file-text', title: 'Raport z każdego uruchomienia', text: 'Każde uruchomienie zwraca raport skutków w postaci tekstu, JSON albo Markdown. Schemat raportu jest opublikowany w specyfikacji.', path: '/product/effect-reports/' },
      { icon: 'bot', title: 'Stworzony dla agentów AI', text: 'Ten sam silnik działa jako program wiersza poleceń, serwer Model Context Protocol, serwer JSON-RPC, serwer języka i biblioteka C.', path: '/product/ai-agents/' },
      { icon: 'boxes', title: 'Jeden plik wykonywalny, nic do doinstalowania', text: 'Operacje na plikach, tekście, archiwach, dokumentach, kodzie, kontroli wersji, sieci i systemie są zaimplementowane w programie i nie wywołują zewnętrznych narzędzi.', path: '/product/platforms/' },
    ],
  },
  how: {
    eyebrow: 'Jak to działa',
    title: 'Napisz potok. Przeczytaj raport. W razie potrzeby cofnij.',
    lead: 'Program to potok etapów połączonych operatorem potoku. Każdy etap przyjmuje rekordy i zwraca rekordy.',
    programTitle: 'nurt: 20 największych plików Go zmienionych w ciągu ostatniej doby',
    program: 'find "/home/user/project" glob: "**/*.go"\n  |> filter size > 1mb and modified > ago(24h)\n  |> sort size order: desc\n  |> first 20',
    sessionTitle: 'nurt 0.1.2: zmiana trzech plików i jej cofnięcie',
    sessionReplay: 'Odtwórz ponownie',
    sessionCaption: 'Wynik nurt 0.1.2 w sesji interaktywnej; katalog jest skrócony, a dwie z trzech różnic pominięte.',
    points: [
      { title: 'Sprawdzenie', text: 'Program jest analizowany i sprawdzany względem katalogu. Błąd w nazwie, parametrze lub typie zatrzymuje program, zanim cokolwiek się zmieni.' },
      { title: 'Uruchomienie', text: 'Etapy wymieniają typowane rekordy. Operacje są zaimplementowane w programie, na wszystkich trzech systemach.' },
      { title: 'Raport', text: 'Uruchomienie zwraca ustrukturyzowany raport skutków, w postaci tekstu, JSON albo Markdown.' },
      { title: 'Cofnięcie', text: 'Zmiany w plikach są w dzienniku. Przeglądanie, cofanie i ponawianie operacji to polecenia języka.' },
    ],
  },
  numbers: {
    eyebrow: 'W liczbach',
    title: 'Co jest w pudełku i jak to sprawdzono',
    lead: 'Każda liczba ma swoje źródło. Wartości policzone są odczytywane z repozytorium podczas budowy witryny; wartości zmierzone wskazują przebieg, który je dał.',
  },
  quality: {
    eyebrow: 'Jakość',
    title: 'Jakość jest pokazana, nie zadeklarowana',
    lead: 'Strony o jakości podają, co jest testowane, jak, z jakim wynikiem i co pozostaje otwarte.',
    cards: [
      { icon: 'flask-conical', title: 'Testy', text: 'Zestaw testów przestrzeni roboczej, platformy, na których działa, i scenariusze na czystym systemie.', path: '/quality/testing/' },
      { icon: 'scale', title: 'Zgodność', text: 'Porównanie z gitem, specyfikacja i przypadki zgodności bindingów języków.', path: '/quality/conformance/' },
      { icon: 'gauge', title: 'Wydajność', text: 'Koszty dziennika, limity uruchomienia i benchmark zadań.', path: '/quality/performance/' },
      { icon: 'shield-check', title: 'Bezpieczeństwo i łańcuch dostaw', text: 'Zgłaszanie, biuletyny, polityka zależności, podpisane commity i podpisane sumy kontrolne wydań.', path: '/quality/supply-chain/' },
    ],
  },
  docs: {
    eyebrow: 'Dokumentacja',
    title: 'Publikowana z repozytorium',
    lead: 'Dokumentacja jest tą z repozytorium, importowaną podczas budowy witryny, więc witryna i źródło nigdy się nie rozjeżdżają.',
    cards: [
      { icon: 'rocket', title: 'Przewodnik', text: 'Instalacja, pierwsze kroki, język, wiersz poleceń, raport skutków, dziennik i konfiguracja.', path: '/docs/guide/' },
      { icon: 'book-open', title: 'Książka kucharska', text: 'Gotowe programy do typowych zadań, od edycji wielu plików po administrację.', path: '/docs/cookbook/' },
      { icon: 'terminal', title: 'Polecenia', text: 'Jedna strona opisu dla każdego polecenia katalogu.', path: '/docs/commands/' },
      { icon: 'library', title: 'Specyfikacja', text: 'Język, drzewo składni, katalog, raport, dziennik, błędy i konfiguracja.', path: '/docs/specification/' },
    ],
  },
  cta: {
    title: 'Wypróbuj na katalogu, który możesz zmienić',
    text: 'Zainstaluj plik wykonywalny, uruchom program, przeczytaj raport i cofnij zmianę.',
    primary: 'Zainstaluj Danaco Nurt',
    secondary: 'Przeczytaj pierwsze kroki',
  },
};
