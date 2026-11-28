<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/journal.md -->
<!-- Responsibility:   Documents Journal and file system contract. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Kontrakt dziennika i systemu plików {#journal-and-file-system-contract}

Ten dokument określa kontrakt dziennika operacji (moduł
`nurt_contracts::journal`) oraz interfejsu systemu plików (moduł
`nurt_contracts::fs`). Dokumentacja Rust obu modułów jest normatywna dla
sygnatur; ten dokument jest normatywny dla protokołu, gwarancji i semantyki
odniesienia implementacji testowych.

## 1. Zasady {#1-principles}

1. Pokrycie. Każda zmiana plików, wartości dostawców lub stanu repozytorium
   wykonana przez polecenie przechodzi przez transakcję mutacji i jest
   zapisywana w dzienniku. Jedyne wyjątki są wymienione w tabeli pokrycia
   (`tables/journal-coverage.toml`, sekcja 13) i są zawsze raportowane z
   jawnym znacznikiem nieodwracalności.
2. Kolejność. Poprzednia zawartość każdego celu jest zachowywana, zanim
   cokolwiek zostanie zmienione. W każdej chwili awarii poprzednia zawartość
   istnieje w co najmniej jednym miejscu.
3. Priorytet miejsca. Wolne miejsce użytkownika ma pierwszeństwo przed
   dziennikiem. Dziennik nigdy nie powoduje niepowodzenia operacji
   użytkownika z powodu braku miejsca, kwalifikacji lub limitów: gdy
   dopuszczenie odmawia zachowania poprzedniej zawartości, operacja jest
   wykonywana bez zachowania, a cel jest oznaczany jako nieodwracalny.
   Awaria samego dziennika podczas przenoszenia usuwanego celu do dziennika
   (każdy błąd inny niż zmiana nazwy między urządzeniami, która przechodzi
   na kopię, oraz brak miejsca na tę kopię) powoduje niepowodzenie operacji
   przy celu pozostawionym na miejscu; usunięcie nigdy nie kasuje danych po
   tym, jak dziennik nie zdołał ich zachować.
4. Jawność. Brak pokrycia jest zawsze widoczny w wyniku. Wyniki nigdy nie
   podają, do kiedy operację można cofnąć.

## 2. Pojęcia {#2-concepts}

Termin zmiana jest zdefiniowany w [słowniku](glossary.md#change).

| Termin | Znaczenie |
|---|---|
| Operacja | Jeden etap skutków jednego programu; jeden numer (`OpId`), jeden wpis na konto, jedna część wpisu na wolumin. |
| Cel | Jeden adres zmieniany przez operację: wpis systemu plików albo klucz dostawcy (`TargetPath`). Drzewo katalogów usuwane lub tworzone w całości jest jednym celem. |
| Tryb zachowania | Sposób zachowania poprzedniego stanu celu (`PreserveMode`, sekcja 5). |
| Stan wpisu | Trwały stan wpisu (`EntryState`, sekcja 6). |
| Stan operacji | Stan pokazywany językowi (`OpState`): `available`, `partial`, `expired`, `undone`. |

Numery operacji tworzą jeden ciąg na konto, zaczynający się od 1. Odczyty i
podglądy nie zużywają numerów. Numery są unikalne, ale niekoniecznie gęste.

Katalog dziennika woluminu jest wspólny dla każdego konta pracującego na
woluminie. Wpis i jego nagrobek zapisują konto, które je utworzyło (katalog
stanu konta). `history`, `undo` i `redo` widzą tylko wpisy konta
wywołującego; wpisy zapisane, zanim konto zaczęto zapisywać, nie mają takiego
rekordu i są widoczne dla każdego konta. Odzyskiwanie dokańcza wpis innego
konta, ale jego wiersz indeksu i jego znacznik pozostawia temu kontu.

Konta z oddzielnymi katalogami stanu numerują swoje operacje lokalnie.
Wyłączne utworzenie znacznika `pending/<N>` rozstrzyga, kto jest właścicielem
numeru. Transakcja, która przegrywa to zajęcie (znacznik lub wpis o tym numerze
już istnieje), pobiera nowy numer i próbuje ponownie, co najwyżej osiem razy,
dopóki nie ma jeszcze otwartej żadnej części; dopiero wtedy dziennik jest
porzucany dla tej operacji, a cel jest oznaczany jako nieodwracalny z
`journal_error`.

## 3. Protokół transakcji {#3-transaction-protocol}

Wykonawca otwiera transakcję przez `JournalStore::begin`, przekazuje ją do
etapu skutków jako `&mut dyn MutationTx` i zatwierdza ją lub wycofuje po
powrocie etapu. Etap nigdy nie zatwierdza.

1. Deklaracja i zachowanie. Etap deklaruje każdy cel przez
   `MutationTx::preserve`, w jednym wywołaniu lub w partiach. Dla każdego celu
   dziennik wybiera tryb zachowania, korzystając z `Admission`; przypisuje
   numer operacji przy pierwszym wywołaniu, zapisuje wpis w stanie `prepared`
   i zachowuje poprzedni stan wszystkich zadeklarowanych celów przed
   powrotem. `preserve` nie jest dozwolone po pierwszej zmianie. W wywołaniu
   deklarującym co najmniej 64 cele poprzednia zawartość małego pliku (co
   najwyżej 4096 bajtów, jedna nazwa, bez atrybutów rozszerzonych, ACL i
   strumieni) jest przechowywana w manifeście wpisu zamiast w bloku na plik
   (magazyn `copy`, sekcja 5); manifest jest zapisywany ponownie, trwale,
   zanim `preserve` zwróci wynik.
2. Zastosowanie. Etap zmienia cele przez `MutationTx::apply`. Nowa zawartość
   jest zapisywana do pliku tymczasowego w katalogu celu, wypełnianego albo
   przez dziennik (`Mutation::WriteBytes`), albo przez etap za pomocą uchwytu
   zwróconego przez `MutationTx::stage` (`Mutation::CommitStaged`), i
   przełączana atomowo. Metadane poprzedniego pliku są przenoszone przez
   dziennik. W transakcji z co najmniej 64 celami dziennik nie opróżnia
   każdego nowego pliku i katalogu zaraz po zmianie; opróżnia je wszystkie
   jednym wywołaniem `FsWrite::sync_paths` przed jakimkolwiek innym rodzajem
   zmiany i przed zatwierdzeniem. Jest to bezpieczne, ponieważ poprzedni stan
   każdego celu jest trwały przed jego zmianą, a wpis pozostaje `prepared` do
   zatwierdzenia: awaria przed opróżnieniem jest wycofywana przez
   odzyskiwanie, które przywraca każdy cel nieposiadający zapisanego
   poprzedniego stanu, niezależnie od tego, jaka część nowej zawartości
   dotarła na dysk (sekcja 4). Nic nie jest zatwierdzane, zanim opróżnienie
   zwróci wynik; nieudane opróżnienie wycofuje transakcję.
3. Oczekiwany stan. `TargetSpec::expect` (`unchecked`, `absent`, znacznik
   encji lub skrót zawartości) jest sprawdzany atomowo ze zmianą.
   Niezgodność kończy się błędem `E_STALE` i wycofuje całą transakcję.
   Znacznik encji ma postać `size-modified-volume-low-high` w zapisie
   szesnastkowym, opcjonalnie z dopiskiem `-sha256:<digest>`; znaczniki bez
   skrótu są akceptowane. Znacznik jest nieprzejrzysty: jego postać zależy od
   sposobu jego utworzenia. Czasy modyfikacji plików mają ziarnistość do 2 s
   (FAT), a na ZFS i ext4 bez znaczników czasu multigrain kilka milisekund,
   więc przepisanie pliku o tym samym rozmiarze w obrębie tej ziarnistości
   pozostawia rozmiar i czas bez zmian. Znacznik ze skrótem jest dlatego
   porównywany także ze skrótem bieżącej zawartości. Etap odczytu, który
   odczytał zawartość (`read`, `lines`, `edit`, `convert`, `compress`,
   notatniki), umieszcza w znaczniku skrót zwróconych bajtów, więc `edit`,
   `convert` lub `compress` po takim odczycie odrzuca cudze przepisanie o tym
   samym rozmiarze z `E_STALE`. Znaczniki utworzone wyłącznie z metadanych
   (`list`, `find`, `info`, zastosowanie git, `write`/`copy`/`remove` ścieżek,
   których nie odczytały) są sprawdzane po metadanych, jak dotąd; przepisanie
   o tym samym rozmiarze w obrębie ziarnistości znaczników czasu nie jest dla
   nich wykrywane. Zegary: sprawdzenie opiera się na zegarze lokalnym tylko w
   śledzeniu zmian (patrz niżej); pamięci podręczne atrybutów NFS oraz zegar
   systemu plików różniący się od zegara lokalnego o więcej niż 3 s nie są
   objęte.
4. Zewnętrzni zapisujący. Cele zapisywane przez bibliotekę (osadzony silnik
   bazy danych, zapisujący indeks repozytorium) lub przez uruchomiony program
   są deklarowane z akcją `external_write`; dziennik zachowuje ich pełną
   poprzednią zawartość przez klon lub kopię. Po zakończeniu pracy
   zapisującego etap raportuje zmiany przez `MutationTx::observe_external`.
   Zadeklarowane cele zewnętrzne, które nie zostały zgłoszone, są usuwane z
   wpisu przy zatwierdzeniu, a ich kopie zwalniane. Zmiany wykryte skanowaniem
   bez zadeklarowanego celu są zapisywane jako nieodwracalne
   (`previous_content_not_kept`), chyba że istnieje kopia.
5. Skutki poza systemem plików. Żądania sieciowe, zatrzymane procesy, zmienione
   usługi i zainstalowane pakiety są zapisywane przez
   `MutationTx::mark_irreversible`. Transakcja może składać się wyłącznie ze
   znaczników.
6. Zapisy wyłączone. Zapisy, które z założenia są poza dziennikiem i nigdy nie
   są cofane (obiekty adresowane zawartością dodawane do magazynu obiektów
   repozytorium), są zapisywane przez `MutationTx::record_exempt`.
7. Zatwierdzenie lub wycofanie. `MutationTx::commit` czyni zmiany trwałymi, a
   wpis `committed`, i zwraca `OpHandle` dla raportu. Transakcja, która niczego
   nie zmieniła i niczego nie oznaczyła, zwraca uchwyt z `op: null` i żaden
   wpis nie istnieje. `MutationTx::rollback` przywraca każdy dotąd zmieniony
   cel, w odwrotnej kolejności. Porzucenie transakcji bez zatwierdzenia
   wycofuje ją.
8. Sprawdzenia po zmianie. Etap może sprawdzić wynik, zanim wykonawca
   zatwierdzi (walidacja, diagnostyka); w razie niepowodzenia wykonawca
   wycofuje z przyczyną `validate_failed`, a wynik ma status `rolled_back`.
   Sprawdzenie po zatwierdzeniu (`verify`) używa
   `JournalStore::rollback_committed`, które tworzy wpis rodzaju `undo` i
   oznacza wpis źródłowy jako cofnięty z przyczyną `verify_failed`.
   Niepowodzenia wykryte przed zatwierdzeniem nie tworzą nowego wpisu.

Po niepowodzeniu metody `MutationTx` transakcja jest wycofywana, a każde
dalsze wywołanie kończy się błędem `E_ROLLED_BACK`. Naruszenie protokołu
(`preserve` po zmianie, niezadeklarowany cel, mutacja niezgodna z
zadeklarowaną akcją) jest defektem wywołującego: kompilacje testowe wpadają w
panikę, kompilacje produkcyjne kończą się błędem `E_STAGE_FAILED` i wycofują.

### 3.1 Typowe operacje {#31-common-operations}

| Operacja | Deklaracja (`TargetAction`, `Expect`) | Mutacja |
|---|---|---|
| Zastąpienie zawartości pliku | `modify`, znacznik encji etapu odczytu | `write_bytes` albo `stage` i `commit_staged` dla zawartości strumieniowanej |
| Utworzenie pliku | `create`, `absent` | `write_bytes` lub `commit_staged` |
| Dopisanie | `append` | `append_bytes` |
| Zapis od przesunięcia, zmiana długości | `modify` / `truncate` | `write_at` / `set_length` |
| Usunięcie pliku lub drzewa katalogów | `delete` | `remove` |
| Zmiana nazwy lub przeniesienie, także między woluminami | `rename { to, replace }` | `rename` |
| Zastąpienie wpisu wpisem innego rodzaju | `replace` | `remove`, a następnie mutacja tworząca |
| Utworzenie katalogu lub dowiązania | `create` / `link` | `create_dir`, `create_link`, `create_hard_link` |
| Zmiana metadanych | `attributes` | `set_meta` |
| Ustawienie lub usunięcie wartości dostawcy | `modify` / `delete` na celu dostawcy | `provider_set` / `provider_remove` |
| Kopiowanie klucza dostawcy z poddrzewem albo wartości | `create`, `absent`, na miejscu docelowym | `provider_copy` z klucza źródłowego |
| Cofnięcie zmiany celu dostawcy | `modify` na zmienionym celu | `provider_restore` z zachowaną migawką |
| Biblioteka sama zapisuje plik | `external_write` | `observe_external` po zapisie |
| Nowe drzewo wypełniane przez operację | `create_tree` | zapisy wewnątrz nie są deklarowane |

Kilka plików zmienionych jednym poleceniem to jedna transakcja i jedna
operacja, raportowana jako operacja zbiorcza i cofana w całości.

### 3.2 Operacje na poziomie polecenia {#32-command-level-operations}

`FileOp` łączy deklarację i mutację każdego wiersza powyższej tabeli (`write`,
`create`, `append`, `write_at`, `set_length`, `remove`, `rename`, `copy`,
`copy_over`, `create_dir`, `create_link`, `create_hard_link`, `set_meta`), z
`expect` i `size_hint` jako ustawieniami opcjonalnymi. `apply_ops(tx, ops)`
wykonuje jeden przebieg protokołu: deklaruje i zachowuje każdy cel jednym
wywołaniem `preserve`, a następnie stosuje mutacje po kolei. Polecenie nigdy
więc nie wybiera trybu zachowania, kolejności protokołu ani strategii między
woluminami; robi to dziennik.

Zawartość strumieniowana i zapisy biblioteki zachowują jawną postać:
zadeklaruj wszystkie cele przez `preserve`, a następnie `stage` i
`commit_staged` dla każdego pliku albo `observe_external` po zapisaniu pliku
przez bibliotekę. Zastosowanie mutacji do niezadeklarowanego celu jest
naruszeniem protokołu (sekcja 3).

`AfterState.meta`, gdy jest obecne, zawiera metadane odczytane przez dziennik
po zmianie, aby wywołujący zapisujący stan plików (indeks repozytorium) nie
powtarzali `stat`.

## 4. Gwarancje awaryjne i odzyskiwanie {#4-crash-guarantees-and-recovery}

| Moment awarii | Stan po odzyskiwaniu |
|---|---|
| Przed przejściem wpisu w `prepared` | Nic się nie zmieniło; brak wpisu. |
| Po `prepared`, podczas zachowywania lub zastosowania | Wpis jest wycofywany: cele, których bieżący skrót równa się skrótowi sprzed zmiany, pozostają bez zmian; cele ze skrótem po zmianie lub nieznanym są przywracane z zachowanych danych; pliki tymczasowe są usuwane. Wpis przechodzi w `undone` z przyczyną `crash`. |
| Podczas wycofywania (`rolling_back`) | Wycofanie jest dokańczane; jest idempotentne. |
| Po `committed` | Wpis jest kompletny i można go cofnąć. |
| Podczas odzyskiwania miejsca (`expiring`) | Usuwanie danych wpisu jest dokańczane. |
| Manifest nie przechodzi kontroli integralności | Wpis przechodzi w `corrupt`; jego dane są zachowywane do ręcznego ratowania. |
| Wycofania nie da się wykonać: nie można odnaleźć poprzedniej zawartości celu albo katalog, który zawierał cel, już nie istnieje | Nic nie jest zmieniane; wpis przechodzi w `corrupt`, a jego dane są zachowywane do ręcznego ratowania. Wpis nie jest ponawiany. |

Manifest operacji z co najmniej 256 celami ma wersję formatu 2: rekordy celów
są spakowane (`targets_z`: tablica JSON skompresowana Zstandard, Base64), a
skrót obejmuje rozpakowany manifest. Elementy wpisu cofnięcia lub ponowienia
są pakowane w ten sam sposób (`items_z`), dzięki czemu wpis cofnięcia wielu
plików pozostaje proporcjonalny do ich liczby z małą stałą. Dziennik zapisany
przed tym formatem jest odczytywany bez zmian (wersja 1); czytnik wersji 1
odrzuca wersję 2 i niczego nie zmienia.

`JournalStore::recover` działa przed pierwszą transakcją procesu i przed
pierwszą operacją po restarcie; wygasza także przeterminowane wpisy. Systemy
plików bez trwałych metadanych po opróżnieniu mają dziennik domyślnie
wyłączony.

## 5. Tryby zachowania {#5-preserve-modes}

| Tryb | Zastosowanie | Cofnięcie |
|---|---|---|
| `moved` | usunięcie; nadpisany cel przeniesienia | przenosi wpis z powrotem |
| `exchange` | nadpisanie przez atomową zamianę nazw | przenosi starą zawartość z powrotem |
| `replace_backup` | nadpisanie w systemie Windows, nazwa kopii zapasowej wywołania replace | jak `exchange` |
| `hardlink` | nadpisanie, twarde dowiązanie do starej zawartości przed zastąpieniem | jak `exchange` |
| `clone` | klon starej zawartości współdzielący bloki | zapisuje klon z powrotem |
| `copy` | kopia bajtowa starej zawartości, w bloku wpisu lub, dla małych plików operacji z wieloma celami, w manifeście (`inline`, szesnastkowo) | zapisuje kopię z powrotem |
| `none` | poprzednia zawartość niezachowana; zawsze ze znacznikiem nieodwracalności | niemożliwe |
| `created` | cel nie istniał | przenosi nowy wpis do wpisu cofnięcia |
| `meta` | wyłącznie zmiana metadanych | przywraca zapisane metadane |
| `ref` | referencje i indeks repozytorium, przechowywane jako pliki | przywraca zapisane pliki |
| `value` | wartość dostawcy lub poddrzewo klucza | przywraca migawkę, łącznie z nieobecnością |
| `append` | dopisanie, dziennik referencji; poprzednia długość i skrót prefiksu | obcina |
| `rename` | zmiana nazwy w obrębie woluminu | zmienia nazwę z powrotem |
| `external` | skutek poza systemem plików; zawsze ze znacznikiem nieodwracalności | niemożliwe |

Jeden wpis może łączyć kilka trybów dla swoich celów (na przykład twardy reset:
`ref`, `append`, tryby nadpisania, `moved` i `created`). Sprawdzenie
cofnięcia obejmuje każdy cel wpisu, niezależnie od jego trybu.

Domyślną strategią nadpisania jest `exchange` z przejściem na `clone`, `copy`
i `none`. Plik z więcej niż jednym twardym dowiązaniem jest zapisywany w
miejscu, tak aby wszystkie nazwy widziały nową zawartość; jego poprzednia
zawartość jest zachowywana przez klon lub kopię, a raport pokazuje
`hardlink_set_preserved`.

## 6. Stany wpisu {#6-entry-states}

| Stan wpisu | Stan operacji | Znaczenie |
|---|---|---|
| `prepared` | nie jest wymieniany | Zachowywany lub stosowany; wycofywany po awarii. |
| `committed` | `available` | Można cofnąć. |
| `rolling_back` | nie jest wymieniany | Wycofanie w toku. |
| `partial` | `partial` | Część elementów zwolniona dla miejsca lub cofnięta pojedynczo; resztę można cofnąć. |
| `undone` | `undone` | Cofnięty w całości; przyczyna `undo`, `rolled_back`, `crash`, `verify_failed` lub `validate_failed`. |
| `expiring` | `expired` | Odzyskiwany. |
| `expired` | `expired` | Dla `history` pozostaje tylko nagrobek. |
| `corrupt` | `expired` | Nie przeszedł kontroli integralności albo nie dało się go wycofać po awarii. |

## 7. Dopuszczenie i priorytet miejsca {#7-admission-and-space-priority}

Przed zachowaniem celu dziennik pyta `Admission`. Dopuszczenie zachowuje
poprzednią zawartość tylko wtedy, gdy zachodzi wszystko naraz: wolumin
kwalifikuje się (`Admission::eligibility`), usługa miejsca działa, wolne
miejsce po zapisie pozostaje co najmniej na poziomie rezerwy plus marginesu, a
budżet rozmiaru jest zachowany. W przeciwnym razie zwalnia najstarsze wpisy w
ramach synchronicznego budżetu czasu; gdy to nie wystarcza, zwraca `skip` z
przyczyną raportowaną użytkownikowi (`journal_inactive`, `no_space_for_copy`,
`too_large_for_journal`, `network_volume`, `removable_volume`). Dopuszczenie
nigdy nie kończy się niepowodzeniem.

| Stan woluminu | Warunek | Zachowanie |
|---|---|---|
| `normal` | wolne miejsce na poziomie progu miękkiego lub powyżej | dane akceptowane; wygasanie tylko według limitów |
| `soft` | między rezerwą a progiem miękkim | dane akceptowane tylko wtedy, gdy wolne miejsce po zapisie pozostaje co najmniej na poziomie rezerwy plus marginesu; odzyskiwanie miejsca w tle |
| `hard` | poniżej rezerwy | dane nie są akceptowane; natychmiastowe odzyskiwanie miejsca |
| `emergency` | poniżej połowy rezerwy | jak `hard`, odzyskiwanie miejsca na wszystkich rdzeniach, wywołujący nie czekają |

Dziennik jest domyślnie nieaktywny na woluminach sieciowych, wymiennych i
opartych na pamięci oraz na woluminach bez trwałych metadanych; jest aktywny
na woluminach overlay, gdzie wpisy warstw niższych są zachowywane przez
kopię.

## 8. Cofnięcie i ponowienie {#8-undo-and-redo}

| Selektor | Semantyka | Wpis |
|---|---|---|
| `last` | najnowszy dostępny wpis sesji, który nie jest cofnięciem | `undo`, zakres `op` |
| `op` | jeden wpis, w całości | zakres `op` |
| `to { after, all_sessions }` | wszystkie dostępne wpisy nowsze niż `after`, od najnowszego, jako jedna transakcja; inne sesje są pomijane i raportowane, chyba że `all_sessions` | zakres `to` |
| `file { path, since }` | najnowsze wpisy, które dotknęły jednego pliku, tylko dla tego pliku; wpisy źródłowe stają się `partial` | zakres `file` |
| `paths { op, paths }` | wybrane elementy jednego wpisu; wpis pozostaje `partial`, dopóki wszystkie elementy nie zostaną cofnięte | zakres `paths` |

Kontrola konfliktów porównuje bieżący skrót każdego elementu ze skrótem
zapisanym po operacji (dla zachowanych danych: skrótem sprzed) dla całego
wyboru przed pierwszym zapisem. Przy `on_conflict: report` (domyślnie)
różnice kończą się błędem `E_JOURNAL_CONFLICT` i nic się nie zmienia; `force`
mimo to cofa i zachowuje bieżącą zawartość we wpisie cofnięcia;
`include_later` cofa także późniejsze wpisy dotykające tych samych elementów;
`partial` cofa pozostałe elementy wpisu częściowego; `abort` niczego nie
zmienia. Cofnięcie jest samo wpisem z własnym numerem; zachowuje bieżący stan
każdego przywracanego elementu, więc można je ponowić. Późniejsza zwykła
operacja na tych samych elementach unieważnia `redo` przez kontrolę
konfliktów.

### 8.1 Wpisy z podwyższonymi uprawnieniami {#81-elevated-entries}

Operacja wykonywana z `privilege: elevated` jest uruchamiana przez osobny
proces z tożsamością administratora. Jej wpisy znajdują się w dzienniku konta,
które o nią poprosiło, jak każdy inny wpis, a konto ma dostęp do ich kopii.
Istnieje jeden dziennik i jeden licznik: wpis z podwyższonymi uprawnieniami
otrzymuje następny numer konta, a `history`, `journal.status`, `undo`, `redo` i
`pin` traktują go jak każdy inny wpis. Wpis wskazuje backend w swoim `action`
(`elevated:<backend>`), a raport operacji podaje `elevated: true` oraz
`backend`.

Proces z podwyższonymi uprawnieniami zapisuje wpis, z poprzednią zawartością,
właścicielem, grupą, trybem, listą kontroli dostępu, atrybutami rozszerzonymi i
kontekstem bezpieczeństwa każdego celu (sekcje 3–7), z uprawnieniami konta, a
nie administratora: w systemie Linux może przełączyć użytkownika i grupę
systemu plików wątku wywołującego (`fsuid`, `fsgid`) na użytkownika i grupę
konta dla każdego wywołania dotyczącego katalogu stanu lub katalogu dziennika
konta, więc jądro sprawdza każdy składnik ścieżki, łącznie z dowiązaniem, z
uprawnieniami konta, a każdy plik i katalog jest tworzony jako należący do
konta. W systemie macOS i innych systemach Unix, które nie mają `fsuid`, proces
otwiera katalog dziennika bez podążania za dowiązaniami (musi to być katalog
należący do konta), tworzy każdy plik przez `openat(O_CREAT | O_EXCL |
O_NOFOLLOW)` i każdy katalog przez `mkdirat` względem tego deskryptora oraz
przekazuje każdy nowy wpis kontu przez `fchown` (`fchownat` z
`AT_SYMLINK_NOFOLLOW` dla dowiązania) i `fchmod`, zanim cokolwiek zostanie
zapisane; w systemie Windows backend `self` działa jako to samo konto i nie
potrzebuje żadnego z tych mechanizmów. W każdym przypadku proces nigdy nie
podąża za dowiązaniem z większymi uprawnieniami, niż ma konto, i nigdy nie
pozostawia pliku należącego do administratora poniżej katalogu stanu. Kopia
celu do dziennika jest odczytywana z uprawnieniami administratora i zapisywana
z uprawnieniami konta, blok po bloku; twarde dowiązanie, klon ani zmiana nazwy
celu do dziennika nie są używane (dziennik kopiuje zamiast tego). Konto
przesyła użytkownika, grupę i katalog stanu w żądaniu (`nurt-elevated/1`);
proces z podwyższonymi uprawnieniami odrzuca katalog stanu, który nie jest
istniejącym katalogiem należącym do tego użytkownika, oraz użytkownika
różniącego się od zapisanego przez backend (`SUDO_UID`). Żądanie operacji z
`as_user` nie niesie właściciela: wpis znajduje się w dzienniku tego konta.

`undo` i `redo` wpisu zapisanego przez operację z podwyższonymi uprawnieniami
przywracają cele z uprawnieniami administratora, przez ten sam backend z tą
samą regułą dostępu, i raportują `elevation_reason: elevated_entry`; bez
backendu kończą się błędem `E_PRIVILEGE` z `next_steps`. `undo` i `redo` bez
numeru najpierw znajdują wpis, który by przywróciły, tak jak robi to dziennik
(najnowszy dostępny wpis, który nie jest cofnięciem, albo najnowsze dostępne
cofnięcie sesji); gdy ten wpis zapisała operacja z podwyższonymi uprawnieniami,
kończą się błędem `E_PRIVILEGE` i następnym krokiem `undo N privilege:
elevated` (`redo N …` dla `redo`), z numerem wpisu. `undo to:` działa z
uprawnieniami konta.

`enable` lub `disable` jest cofane tylko wtedy, gdy zapisany typ uruchamiania
to `disabled` lub `automatic` (wydanie 0.1.1). Usługa, której zapisany typ
uruchamiania to `manual`, `static` lub `masked`, albo opóźniony start w
systemie Windows, czyni wpis nieodwracalnym: raport zachowuje znacznik
`service_changed`, a jego `detail` podaje przyczynę. Pakiet, którego
repozytorium nie oferuje już w zapisanej wersji, jest nieodwracalny dla
pakietu; pozostałe pakiety `upgrade` pozostają możliwe do cofnięcia.

Operacje poza systemem plików są cofane przez operacje odwrotne:

| Operacja | Zapisany stan | Cofnięcie |
|---|---|---|
| `package.install` | czy pakiet był wcześniej zainstalowany i jego wersja | `package.remove`, gdy nie był wcześniej zainstalowany; ponowna instalacja zapisanej wersji, gdy instalacja zmieniła wersję |
| `package.remove` | zainstalowana wersja | `package.install` tej wersji, gdy repozytorium nadal ją oferuje; w przeciwnym razie nieodwracalne w chwili zmiany |
| `package.upgrade` | wersje sprzed aktualizacji | instalacja zapisanych wersji, gdy repozytorium nadal je oferuje; w przeciwnym razie nieodwracalne dla pakietu |
| `start`, `stop` (usługi) | poprzedni stan | polecenie przeciwne |
| `enable`, `disable` | poprzedni typ uruchamiania | polecenie przeciwne z zapisanym typem uruchamiania |
| `restart`, `stop` procesu, `run` | brak | nieodwracalne; raport podaje `irreversible` przed zmianą |
| `firewall.allow`, `firewall.deny` | cel `firewall:<rule>`, `absent` na `present` | `firewall.remove` reguły, w backendzie i tabeli, które zostały użyte |
| `firewall.remove` | cel `firewall:<rule>`, `present` na `absent`, jedna zmiana na usuniętą regułę | `firewall.allow` lub `firewall.deny` każdej usuniętej reguły |
| `network.route_add`, `network.route_remove` | cel `route:<destination>` z `via=<gateway> dev=<interface> metric=<n>` | polecenie przeciwne z zapisaną trasą |
| `network.dns_set` | cel `dns:<interface>` z `servers=<list>;search=<list>` | `network.dns_set` z poprzednimi serwerami i domenami; `/etc/resolv.conf` jest celem plikowym przywracanym przez dziennik |
| `clock.timezone_set`, `clock.ntp` | cele `clock:timezone` i `clock:ntp` | to samo polecenie z poprzednią wartością |
| `clock.set` | cel `clock:time` | nieodwracalne; raport zachowuje znacznik `external_effect`, a jego `detail` podaje przyczynę |
| `windows.cim` z `invoke` | cel `cim:<class>.<method>` z przestrzenią nazw i warunkiem | udokumentowana metoda przeciwna na tych samych instancjach; w przeciwnym razie nieodwracalne |
| `accounts.create` | wcześniej nieobecne | `accounts.remove` z `keep_home: true` |
| `accounts.modify` | powłoka, katalog domowy, grupy, komentarz i stan blokady przed i po | `accounts.modify` z zapisanymi wartościami pól, które się zmieniły |
| `accounts.remove` | cały stan konta; katalog domowy przechodzi przez dziennik | `accounts.create` z zapisanym identyfikatorem, powłoką, katalogiem domowym, grupami i komentarzem oraz `accounts.modify` dla konta zablokowanego; dziennik przywraca katalog domowy |
| `accounts.password` | brak | nieodwracalne |
| `groups.create`, `groups.remove` | wcześniej nieobecne; identyfikator i członkowie | parami; usunięta grupa wraca przez `groups.add_member` jej członków |
| `groups.add_member`, `groups.remove_member` | członkowie przed i po | polecenie przeciwne |
| `mount`, `unmount` | wcześniej nieobecne; urządzenie, typ i opcje | parami; edycja `/etc/fstab` jest plikiem dziennika |
| `sysctl.set` | poprzednia wartość | `sysctl.set` z tą wartością; plik z `persistent: true` jest plikiem dziennika |
| `modules.load`, `modules.unload` | załadowany lub nie | polecenie przeciwne |
| `power.restart`, `power.shutdown` | harmonogram | `power.cancel` dla zaplanowanej akcji; nieodwracalne, gdy akcja wykonuje się od razu |
| `power.cancel`, `disks.partition`, `disks.format` | brak | nieodwracalne; podgląd zmiany dysku wymienia dane, które zostaną utracone |

Pliki zmienione przez program uruchomiony przez `run` z `privilege: elevated`
nie są zachowywane; wpis jest nieodwracalny, a raport to podaje.

## 9. Błędy {#9-errors}

Wszystkie metody `MutationTx` i `JournalStore` zwracają błąd języka. Szczegóły
błędów dziennika niosą typy modułu dziennika:

| Kod | Kiedy | Szczegóły |
|---|---|---|
| `E_STALE` | oczekiwany stan nie zgadza się | `path`, `expected`, `actual` |
| `E_NOT_FOUND`, `E_EXISTS`, `E_PERMISSION`, `E_PRIVILEGE`, `E_IN_USE`, `E_NO_SPACE`, `E_IO` | niepowodzenie celu | `path`, `kind` (rodzaj błędu systemu plików), `os_error`, `holders` |
| `E_CANCELLED` | operacja została anulowana; wycofana | brak |
| `E_ROLLED_BACK` | wywołanie na transakcji, która została wycofana; nieudane sprawdzenie po zmianie | `reason` |
| `E_ROLLBACK_FAILED` | wycofanie lub cofnięcie nie mogło przywrócić każdego elementu | `failed`: lista `TargetPath` |
| `E_STAGE_FAILED` | niewłaściwe użycie protokołu w kompilacjach produkcyjnych | `problem` |
| `E_JOURNAL_NOOP` | brak takiego wpisu, nic do cofnięcia ani ponowienia | `op`, `state` |
| `E_JOURNAL_EXPIRED` | wpis wyboru wygasł; nic się nie zmieniło | `expired`: lista `OpId` |
| `E_JOURNAL_CONFLICT` | kontrola cofnięcia wykryła różnice; nic się nie zmieniło | `conflicts`: lista `Conflict` |
| `E_JOURNAL_PARTIAL` | wpis częściowy, a wywołanie zażądało całego wpisu | `op`, `available`, `unavailable` |
| `E_JOURNAL_CORRUPT` | wpis nie przeszedł kontroli integralności | `op` |

Brak miejsca dla dziennika, nieaktywny dziennik i błędy wejścia/wyjścia
dziennika nigdy nie są błędami operacji użytkownika.

## 10. Retencja {#10-retention}

Wpis wygasa, gdy przekroczony zostanie dowolny z trzech limitów (kroki, wiek,
rozmiar) albo gdy użytkownik potrzebuje miejsca. Limity są oceniane względem
bieżącej konfiguracji: obniżenie limitu wygasza wpisy natychmiast, podniesienie
nigdy nie przywraca wygasłych wpisów. Operacja zbiorcza wygasa w całości; tylko
odzyskiwanie miejsca może zwolnić część wpisu, który wtedy staje się `partial`.

Stan retencji (liczba wpisów, przechowywane bajty, najstarszy wpis, obowiązujące
limity i ich źródła) jest raportowany wyłącznie przez `journal.status`
(`JournalStore::status`). `OpHandle`, `OperationRecord`, `UndoOutcome` i każdy
inny wynik nie niosą czasu wygaśnięcia ani wielkości, z której można go
wyprowadzić.

## 11. Interfejs systemu plików {#11-file-system-interface}

`Fs` jest interfejsem tylko do odczytu, przekazywanym każdemu poleceniu.
`FsWrite` wykonuje mutacje; każda metoda wymaga `TxScope`, który tworzą
wyłącznie implementacje dziennika i miejsca, więc polecenia zmieniają dane
tylko przez `MutationTx`. `Provider` i `ProviderWrite` obsługują zasoby
podobne do ścieżek, takie jak `registry:` i `env:`; dostawcy z możliwością
zapisu obsługują migawki i dokładne przywracanie.

Ścieżki są bezwzględne i leksykalnie znormalizowane (`AbsPath`); ich postać
serializowana to postać języka z separatorami `/`. Ścieżka z bajtami, które
nie są poprawnym Unicode, jest serializowana jako obiekt `{"path": <language
form with U+FFFD>, "raw_path": <native bytes, base64>}` i odczytywana z
`raw_path`, tak aby manifest wpisu prowadził z powrotem do wpisu, który został
zmieniony; lista katalogu zachowanego drzewa podaje taką nazwę jako `raw_path`
obok `path`. Błędy są wartościami `FsError` z rodzajem, który odwzorowuje się
na kod języka:

| Rodzaj | Kod |
|---|---|
| `not_found` | `E_NOT_FOUND` |
| `permission_denied`, `read_only` | `E_PERMISSION` |
| `privilege` | `E_PRIVILEGE` |
| `exists` | `E_EXISTS` |
| `in_use` | `E_IN_USE` |
| `no_space`, `quota_exceeded` | `E_NO_SPACE` |
| `stale` | `E_STALE` |
| `cross_device`, `unsupported`, `invalid_path`, `not_a_directory`, `is_a_directory`, `not_empty`, `name_too_long`, `loop`, `interrupted`, `io` | `E_IO`, z rodzajem w szczegółach |

## 12. Semantyka odniesienia dla implementacji testowych {#12-reference-semantics-for-test-implementations}

Implementacje testowe muszą zachowywać się dokładnie tak, jak opisano poniżej,
aby testy napisane względem nich obowiązywały dla implementacji rzeczywistych.

### 12.1 Dziennik przepustowy (`NoopJournal`) {#121-pass-through-journal-noopjournal}

1. `begin` zwraca transakcję, która stosuje mutacje bezpośrednio do
   bazowego `FsWrite` z zakresem o przeznaczeniu `operation`.
2. `preserve` waliduje deklaracje tak jak dziennik rzeczywisty (istnienie dla
   `create`, `modify`, `delete`; oczekiwany stan) i zwraca dla każdego celu tryb
   `none` z przyczyną `journal_inactive`.
3. Numery operacji są przypisywane od 1 w górę dla każdej instancji, przy
   pierwszym `preserve` lub, dla transakcji wyłącznie ze znacznikami, przy
   zatwierdzeniu.
4. `apply` sprawdza oczekiwany stan bezpośrednio przed zmianą i kończy się
   błędem `E_STALE` przy niezgodności; po każdym niepowodzeniu każde dalsze
   wywołanie kończy się błędem `E_ROLLED_BACK`.
5. `commit` zwraca uchwyt ze wszystkimi celami oznaczonymi `journal_inactive`;
   `rollback` niczego nie przywraca i wymienia każdy zmieniony cel w `failed`.
6. `history` zwraca pustą listę; `details`, `undo`, `redo` i
   `rollback_committed` kończą się błędem `E_JOURNAL_NOOP`; `pin` raportuje
   każdy wpis jako niedostępny; `purge`, `status` i `space` zwracają puste
   wyniki; `limits` zwraca `applied: false`; `recover` i `sweep` zwracają
   puste raporty.
7. Wariant rejestrujący dodatkowo zachowuje listę wywołań w kolejności, dla
   asercji, że każda mutacja przeszła przez transakcję.

### 12.2 Dopuszczenie {#122-admission}

Testowe dopuszczenie zwraca `eligible`, `keep`, stan `normal` i `alive`, chyba
że skonfigurowano inaczej; skonfigurowana decyzja dotyczy każdego żądania.

### 12.3 Wirtualny system plików {#123-virtual-file-system}

1. Ścieżki są porównywane bajt po bajcie na woluminach rozróżniających wielkość
   liter; na woluminach skonfigurowanych jako nierozróżniające wielkości
   liter — po normalizacji NFC i zrównaniu wielkości liter.
2. Każdy warunek niepowodzenia daje jeden rodzaj błędu: brakujący wpis lub
   element nadrzędny `not_found`; plik użyty jako składnik katalogu
   `not_a_directory`; istniejące miejsce docelowe z `no_replace` lub
   `create_new` `exists`; usunięcie niepustego katalogu `not_empty`; zmiana
   nazwy między woluminami `cross_device`; `exchange`, `replace_with_backup` i
   `clone_file` na woluminie bez tej funkcji `unsupported`; skonfigurowane
   naruszenie współdzielenia `in_use` ze skonfigurowanymi posiadaczami.
3. `read_dir` zwraca wpisy w kolejności bajtowej ich nazw.
4. Znaczniki czasu pochodzą z wstrzykniętego `Clock`; każda zmiana zawartości
   ustawia czas modyfikacji na bieżący czas zegarowy i niczego poza tym nie
   zwiększa.
5. Tożsamości plików są przypisywane od 1 w górę dla każdego woluminu; zmiana
   nazwy zachowuje tożsamość, nowy plik otrzymuje nową.
6. Znacznik encji to `(size, modified_ns, file_id)`; `content_hash` to SHA-256
   zawartości.
7. `holders` jest puste, chyba że skonfigurowano inaczej; `can_write_dir`
   podąża za bitami trybu katalogu dla skonfigurowanego konta.
8. Błędy są wstrzykiwane dla ścieżki i operacji (`permission_denied`, `io`,
   `no_space`, `in_use`, `loop`, krótkie odczyty) i dotyczą wyłącznie
   następnego pasującego wywołania, chyba że skonfigurowano je jako trwałe.

## 13. Tabela pokrycia {#13-coverage-table}

`tables/journal-coverage.toml` (format `nurt-journal-coverage/1`) jest źródłem
tabeli pokrycia. Ma jeden wiersz na polecenie, albo na polecenie i akcję,
kategorii zmiany i uruchamiania oraz poleceń dziennika ze skutkiem.

| Pole | Znaczenie |
|---|---|
| `command` | nazwa polecenia katalogu |
| `action` | opcjonalna akcja, tryb lub wariant celu; wiersz bez niej obejmuje całe polecenie |
| `group` | grupa katalogu |
| `changes` | co operacja zmienia |
| `preserve` | tryby zachowania (sekcja 5) |
| `coverage` | `full`, `partial`, `none` lub `no_effect` |
| `irreversible` | przyczyny raportowane dla wiersza, dodatkowo do `generic_irreversible` |
| `exception` | opis wyjątku; wymagany dla `partial` i `none` |
| `note` | opcjonalne wyjaśnienie |

Reguły wartości pokrycia: wiersze `full` wymieniają wyłącznie tryby
zachowujące dane i żadnych przyczyn; wiersze `partial` wymieniają co najmniej
jeden tryb zachowujący dane oraz wyjątek; wiersze `none` wymieniają `none` lub
`external` (ewentualnie z trybami dla lokalnych skutków ubocznych) oraz
wyjątek; wiersze `no_effect` nie wymieniają żadnych trybów.

Przyczyny wymienione w `generic_irreversible` mogą pogorszyć dowolny wiersz
(nieaktywny dziennik, wolumin sieciowy lub wymienny, brak miejsca na kopię,
limity rozmiaru, awaria dziennika podczas zapisu). Zestaw testów sprawdza, że
tabela jest poprawnie zbudowana, że każde polecenie katalogu i każda akcja
katalogu ze skutkiem ma wiersz, że każdy wiersz nazywa polecenie katalogu oraz
że nieodwracalne wpisy katalogu nie są oznaczone `full`, a odwracalne nie są
oznaczone `none`.

Test pokrycia implementacji dziennika wykonuje każdy wiersz: wykonuje
operację, porównuje pełny stan drzewa (skróty i metadane) przed operacją i po
cofnięciu, sprawdza tryb zachowania względem tabeli, sprawdza przyczyny
nieodwracalności wierszy `partial` i `none` oraz wykrywa mutacje poza
transakcją.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
