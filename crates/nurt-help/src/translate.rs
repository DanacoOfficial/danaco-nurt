// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Polish texts of the messages, notes and irreversible effects of a report.
//!
//! The messages of the commands are written in English at the place where an
//! error is raised. [`localize`] replaces them in a finished report: the
//! `message` of an error becomes the Polish meaning of its code followed by
//! the parameters of the error (`found`, `expected`, `problem`), a note or an
//! irreversible effect gets the Polish text of its code. Codes, fields,
//! programs in `fix` and `next_steps` and the help topic stay as they are.
//! The English `message` is kept in the detail `message_en` of the error and
//! the English `detail` of a note follows the Polish text, so that no
//! information is lost. A code without a Polish text keeps its English one.

use nurt_contracts::envelope::Language;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::{Irreversible, Notable, Report};
use serde_json::Value;

/// Detail of an error that holds its English message after a translation.
pub const MESSAGE_EN_DETAIL: &str = "message_en";

/// Largest number of expected names written into a translated message.
const MAX_EXPECTED_SHOWN: usize = 5;

/// Polish meaning of an error code, `None` for a code without one.
#[must_use]
pub fn error_text(code: ErrorCode) -> Option<&'static str> {
    Some(match code {
        ErrorCode::Lexical => {
            "znak lub fragment programu jest niepoprawny: nieznany znak, niezamknięty tekst albo błędna liczba, rozmiar lub czas trwania"
        }
        ErrorCode::Escape => "sekwencja specjalna w tekście jest niepoprawna",
        ErrorCode::Syntax => "program nie spełnia gramatyki",
        ErrorCode::Ambiguous => "korekta jest możliwa, ale dotyka celu zmiany",
        ErrorCode::Uncorrectable => "reguły korekty nie potrafią poprawić programu",
        ErrorCode::InternalCorrection => "korekta naruszyła niezmiennik; nic nie zostało wykonane",
        ErrorCode::ExpiredCandidate => "wskazany kandydat wygasł lub jest nieznany",
        ErrorCode::AstVersion => "wersja drzewa składni jest nowsza, niż obsługuje ta instancja",
        ErrorCode::AstSchema => "drzewo składni nie jest zgodne ze schematem",
        ErrorCode::UnknownCommand => "polecenia nie ma w katalogu",
        ErrorCode::UnknownParam => "polecenie nie ma takiego parametru",
        ErrorCode::UnknownName => "nazwa nie jest wynikiem żadnego let w programie",
        ErrorCode::UnknownField => "rekordy nie mają takiego pola",
        ErrorCode::UnknownValue => "wartość nie należy do wartości wyliczenia",
        ErrorCode::MissingParam => "brakuje wymaganego parametru",
        ErrorCode::ParamConflict => "podano razem parametry, które się wykluczają",
        ErrorCode::Type => "wartość lub strumień rekordów ma typ, którego to miejsce nie przyjmuje",
        ErrorCode::Arity => "liczba argumentów jest nieprawidłowa",
        ErrorCode::InvalidValue => "wartość ma właściwy typ, ale jest niedozwolona w tym miejscu",
        ErrorCode::Config => "konfiguracja jest niepoprawna",
        ErrorCode::PathRelative => "podano ścieżkę względną bez parametru wywołania base",
        ErrorCode::PathForeign => "ścieżka jest zapisana w postaci obcej dla tego systemu",
        ErrorCode::OneEffect => {
            "program ma więcej niż jeden etap zmieniający lub uruchamiający albo niedozwolony etap po skutku"
        }
        ErrorCode::OsOnly => "polecenie istnieje tylko w innym systemie operacyjnym",
        ErrorCode::Unsupported => "parametr lub tryb nie jest dostępny w tym systemie",
        ErrorCode::Regex => "wyrażenie regularne lub szablon zamiany jest niepoprawny",
        ErrorCode::KeySyntax => "ścieżka klucza w dokumencie jest niepoprawna",
        ErrorCode::PatternSyntax => {
            "wzorzec składni nie daje się przeanalizować w języku docelowym"
        }
        ErrorCode::Query => "zapytanie do dokumentu nie kompiluje się albo kończy się błędem",
        ErrorCode::Sql => {
            "instrukcji SQL nie można przygotować lub wykonać albo zmienia dane bez write: true"
        }
        ErrorCode::Arg => "argumentu nie można przekazać bez zmiany jego znaczenia",
        ErrorCode::ExternalMissing => "program wymagany przez polecenie nie jest zainstalowany",
        ErrorCode::ExternalFailed => {
            "program zewnętrzny używany przez polecenie zakończył się błędem lub złamał protokół"
        }
        ErrorCode::ProgramNotFound => "programu do uruchomienia nie ma w ścieżce wyszukiwania",
        ErrorCode::Spawn => "nie udało się uruchomić programu",
        ErrorCode::Assert => {
            "asercja nie jest spełniona; przed skutkiem nic się nie zmieniło, po skutku skutek pozostaje"
        }
        ErrorCode::Binary => "zawartość jest binarna, a wymagany jest tekst",
        ErrorCode::DocParse => "plik nie jest poprawnym dokumentem swojego formatu",
        ErrorCode::Format => "format jest uszkodzony lub nieobsługiwany",
        ErrorCode::Encrypted => "archiwum lub dokument jest zaszyfrowany",
        ErrorCode::UnsupportedConstruct => {
            "konstrukcji dokumentu nie można zmienić bez utraty jego układu"
        }
        ErrorCode::KeyNotFound => "ścieżki klucza nie ma w dokumencie",
        ErrorCode::KeyConflict => "ścieżka klucza przechodzi przez wartość innego rodzaju",
        ErrorCode::UnsupportedLanguage => "brak gramatyki dla języka tego pliku",
        ErrorCode::SymbolNotFound => "symbol nie ma definicji",
        ErrorCode::SymbolAmbiguous => "symbol ma kilka definicji",
        ErrorCode::RenameConflict => "nowa nazwa koliduje z istniejącym symbolem w tym zakresie",
        ErrorCode::NoMatch => {
            "zamiana lub edycja nie znalazła dopasowania, a wywołanie go wymagało"
        }
        ErrorCode::PatchRejected => "fragment łatki nie pasuje do pliku",
        ErrorCode::OutOfRange => "numer wiersza, strona lub pozycja leży poza zawartością",
        ErrorCode::Stale => "plik zmienił się po odczycie; cała operacja została wycofana",
        ErrorCode::Encoding => "znaku nie można zapisać w kodowaniu pliku",
        ErrorCode::Io => "błąd wejścia lub wyjścia systemu plików",
        ErrorCode::Permission => "dostęp odrzucony przez uprawnienia",
        ErrorCode::Exists => "cel już istnieje",
        ErrorCode::NotFound => "celu nie ma",
        ErrorCode::InUse => "cel jest używany; procesy, które go trzymają, są wymienione",
        ErrorCode::Privilege => "operacja wymaga uprawnienia, którego konto nie ma",
        ErrorCode::NoSpace => "brakuje miejsca na woluminie",
        ErrorCode::Timeout => "osiągnięto limit czasu",
        ErrorCode::Cancelled => "wykonanie zostało anulowane",
        ErrorCode::Limit => "osiągnięto limit zasobu",
        ErrorCode::StageFailed => "etap zakończył się błędem",
        ErrorCode::RolledBack => "weryfikacja nie powiodła się, a skutek został wycofany",
        ErrorCode::RollbackFailed => {
            "wycofanie nie przywróciło wszystkiego; pliki są wymienione wraz z instrukcją"
        }
        ErrorCode::JournalNoop => {
            "w dzienniku nie ma takiej operacji albo nie ma czego cofnąć lub ponowić"
        }
        ErrorCode::JournalExpired => "wpis dziennika wygasł; nic się nie zmieniło",
        ErrorCode::JournalConflict => {
            "bieżący stan różni się od stanu po operacji; nic się nie zmieniło"
        }
        ErrorCode::JournalPartial => "wpis jest częściowy: część pozycji została zwolniona",
        ErrorCode::JournalCorrupt => "wpis dziennika nie przeszedł kontroli integralności",
        ErrorCode::UnknownJob => "zadanie w tle jest nieznane",
        ErrorCode::JobState => "czynność nie jest dozwolona w bieżącym stanie zadania",
        ErrorCode::TestNoFramework => "nie rozpoznano środowiska testowego projektu",
        ErrorCode::UnknownRun => "przebieg testów jest nieznany",
        ErrorCode::UnknownAnalysis => "analiza jest nieznana",
        ErrorCode::OutputParse => {
            "nie udało się odczytać wyniku środowiska testowego lub analizatora"
        }
        ErrorCode::Network => {
            "błąd sieci: rozwiązywanie nazw, połączenie, TLS, serwer pośredniczący, przekierowania lub protokół"
        }
        ErrorCode::HashMismatch => "pobrany plik nie ma oczekiwanego skrótu; nie został zachowany",
        ErrorCode::Service => "menedżer usług zgłosił błąd",
        ErrorCode::ServiceNotRunning => {
            "usługa działająca w tle dla tej instancji nie jest uruchomiona"
        }
        ErrorCode::Catalog => "jawnie skonfigurowanego katalogu programów nie można odczytać",
        ErrorCode::GitNotRepository => "ścieżka nie leży w repozytorium",
        ErrorCode::GitRevision => "rewizja jest nieznana, niejednoznaczna lub błędnie zapisana",
        ErrorCode::GitRef => {
            "nazwa referencji jest niepoprawna, istnieje, nie istnieje, nie jest scalona albo jest wyewidencjonowana gdzie indziej"
        }
        ErrorCode::GitDirty => "lokalne zmiany zostałyby nadpisane; nic się nie zmieniło",
        ErrorCode::GitConflict => {
            "scalanie zakończyło się konfliktami; skutek jest zastosowany, a konflikty są wymienione"
        }
        ErrorCode::GitInProgress => {
            "trwa scalanie lub zmiana bazy albo nie trwa żadne, a jest wymagane"
        }
        ErrorCode::GitLocked => "istnieje plik blokady repozytorium",
        ErrorCode::GitNonFastForward => {
            "historie się rozeszły, a dozwolone jest tylko przewinięcie"
        }
        ErrorCode::GitNothingToCommit => "nie ma czego zatwierdzić",
        ErrorCode::GitHook => "hak zakończył się kodem różnym od zera; zatwierdzenie nie powstało",
        ErrorCode::GitSign => "nie udało się podpisać zatwierdzenia lub znacznika",
        ErrorCode::GitAuth => "zdalne repozytorium wymaga uwierzytelnienia albo je odrzuciło",
        ErrorCode::GitHostKey => "klucz hosta zdalnego repozytorium jest nieznany lub się zmienił",
        ErrorCode::GitPushRejected => {
            "zdalne repozytorium odrzuciło aktualizację co najmniej jednej referencji"
        }
        ErrorCode::GitProtocol => {
            "zdalne repozytorium złamało protokół lub nie ma wymaganej możliwości"
        }
        ErrorCode::GitUnsupported => "możliwość repozytorium nie jest obsługiwana",
        ErrorCode::GitHunk => {
            "identyfikator fragmentu jest błędny lub nieznany albo fragment zmienił się po wypisaniu"
        }
        ErrorCode::GitCheckpoint => {
            "punkt kontrolny nie ma zapisanego indeksu albo jego zawartość nie zgadza się po przywróceniu"
        }
        ErrorCode::GitWorktree => {
            "drzewo robocze jest nieznane, jest głównym drzewem, jest zablokowane albo zawiera katalog roboczy"
        }
        ErrorCode::GitConfig => {
            "konfiguracja repozytorium jest niepoprawna: wartości nie da się odczytać albo brakuje wymaganego klucza"
        }
        ErrorCode::Internal => "błąd wewnętrzny; raport jest kompletny do tego miejsca",
        _ => return None,
    })
}

/// Polish text of a note code, `None` for a code without one.
#[must_use]
pub fn notable_text(code: Notable) -> Option<&'static str> {
    Some(match code {
        Notable::OutsideProject => "cel leży poza każdym skonfigurowanym katalogiem projektu",
        Notable::SystemPath => "cel leży w katalogu systemowym",
        Notable::HomeConfig => "cel jest plikiem lub katalogiem konfiguracji w katalogu domowym",
        Notable::MassDelete => "usuwanych jest wiele plików lub bajtów albo duża część katalogu",
        Notable::DeleteTree => "niepusty katalog jest usuwany rekurencyjnie",
        Notable::OverwroteExisting => {
            "istniejące pliki zostały zastąpione przez zapis, przeniesienie lub kopię"
        }
        Notable::FileEmptied => "plik, który miał zawartość, jest po zmianie pusty",
        Notable::SizeShrunk => "rozmiar pliku zmniejszył się co najmniej o skonfigurowany stosunek",
        Notable::ManyPerFile => "pojedynczy plik otrzymał co najmniej skonfigurowaną liczbę zamian",
        Notable::TargetCountHigh => "liczba celów osiągnęła skonfigurowany próg",
        Notable::NothingMatched => "program zmieniający nie dostał żadnych celów",
        Notable::Unchanged => "zapis dał zawartość identyczną z istniejącą",
        Notable::BinaryTarget => "zmieniono plik o zawartości binarnej",
        Notable::BinarySkipped => "pominięto pliki o zawartości binarnej",
        Notable::SpecialFile => {
            "zapisano plik specjalny (urządzenie, potok, gniazdo, pseudo system plików)"
        }
        Notable::SymlinkFollowed => {
            "zmiana przeszła przez dowiązanie symboliczne do celu poza jego katalogiem"
        }
        Notable::HardlinkBroken => {
            "plik z wieloma dowiązaniami twardymi został zastąpiony, co rozdzieliło dowiązania"
        }
        Notable::HardlinkSetPreserved => {
            "plik z wieloma dowiązaniami twardymi zapisano w miejscu, zachowując dowiązania"
        }
        Notable::AttributesChanged => {
            "zmieniły się uprawnienia, właściciel lub lista dostępu pliku"
        }
        Notable::EncodingPreserved => "plik zapisano w jego pierwotnym kodowaniu innym niż UTF-8",
        Notable::LineEndingPreserved => {
            "plik zapisano z jego pierwotnymi końcami wierszy innymi niż lf"
        }
        Notable::LineEndingMixed => "plik ma mieszane końce wierszy",
        Notable::EncodingChanged => "kodowanie pliku zostało przekonwertowane",
        Notable::LineEndingChanged => "końce wierszy pliku zostały przekonwertowane",
        Notable::BomChanged => "dodano lub usunięto znacznik kolejności bajtów",
        Notable::TruncatedLines => "długie wiersze skrócono na wyjściu",
        Notable::RegexBacktracking => {
            "wyrażenie regularne osiągnęło limit nawrotów dla części danych"
        }
        Notable::Decompressed => "skompresowaną zawartość rozpakowano w sposób przezroczysty",
        Notable::CrossDevice => {
            "przeniesienie przekroczyło granicę woluminów i wykonano je jako kopię z usunięciem"
        }
        Notable::ToIsDir => "miejsce docelowe przeniesienia lub kopii jest istniejącym katalogiem",
        Notable::CaseCollision => "nazwy różniące się tylko wielkością liter kolidują",
        Notable::CaseAlias => "nazwę dopasowano z inną wielkością liter niż podana",
        Notable::NfdName => "nazwę dopasowano w innej postaci normalizacji Unicode niż podana",
        Notable::Portability => "nazwa nie jest przenośna między systemami operacyjnymi",
        Notable::ExternalVolume => "cel leży na woluminie sieciowym lub wymiennym",
        Notable::ArchivePathEscape => "pominięto wpisy archiwum wskazujące poza katalog docelowy",
        Notable::PermissionDeniedSkipped => "pominięto pliki bez uprawnienia do odczytu",
        Notable::Ignored => "nie zgłoszono zmian w ścieżkach wyłączonych ze śledzenia",
        Notable::AliasShared => {
            "zmieniono węzeł, do którego odwołują się aliasy; zmiana jest widoczna przez każdy alias"
        }
        Notable::KeysCreated => "utworzono klucze pośrednie",
        Notable::DuplicateKey => "dokument ma powtórzony klucz; zmieniono ostatnie wystąpienie",
        Notable::SemanticFallback => "operacja na kodzie wróciła do analizy składniowej",
        Notable::ReferencesLeft => "po zmianie pozostały tekstowe odwołania do starej nazwy",
        Notable::RenameIncomplete => "zmiana nazwy nie objęła wszystkich odwołań",
        Notable::DetectionScope => {
            "zmiany wykonane przez program są wykrywane tylko w obserwowanych katalogach"
        }
        Notable::ExitNonzero => "program zakończył się kodem wyjścia różnym od zera",
        Notable::PostStagesDeferred => "pominięto etapy po skutku, bo program przeszedł do tła",
        Notable::RollbackPartial => "wycofanie przywróciło tylko część zmian",
        Notable::OutputBinary => "program zapisał dane binarne do strumienia wyjścia",
        Notable::SpoolLimit => "wyjście programu przekroczyło limit bufora",
        Notable::StdinClosedEarly => {
            "program zamknął standardowe wejście, zanim zapisano wszystkie dane"
        }
        Notable::JobLost => "zadanie w tle straciło nadzorcę",
        Notable::ToolMissing => "żądane narzędzie analizy nie jest zainstalowane",
        Notable::TestsNone => "nie znaleziono ani nie uruchomiono żadnych testów",
        Notable::TrustConfigIgnored => {
            "pominięto ustawienia zaufania w pliku konfiguracji projektu; projekt nie może sam siebie uznać za zaufany"
        }
        Notable::ProjectConfigIgnored => {
            "pominięto klucze pliku konfiguracji projektu spoza kluczy dozwolonych dla projektów"
        }
        Notable::JournalConfigIgnored => {
            "pominięto ustawienia dziennika w pliku konfiguracji niezaufanego projektu"
        }
        Notable::JournalOversize => "operacja jest większa niż limit rozmiaru dziennika",
        Notable::SourceUnavailable => {
            "źródło danych systemu operacyjnego było niedostępne; wyniki są niepełne"
        }
        Notable::SettingsBroadcast => "zmianę ustawień rozesłano do uruchomionych programów",
        Notable::HooksSkipped => "haki repozytorium nie zostały uruchomione",
        Notable::HistoryRewritten => "historia repozytorium została przepisana",
        Notable::ForcePush => "zdalna gałąź została nadpisana wymuszonym wypchnięciem",
        Notable::DetachedHead => "repozytorium jest w stanie odłączonego HEAD",
        Notable::BranchDeletedUnmerged => "usunięto gałąź, która nie była scalona",
        Notable::FastForward => "gałąź zaktualizowano przez przewinięcie",
        Notable::Conflicts => "operacja zatrzymała się z konfliktami",
        Notable::UntrackedRemoved => "usunięto pliki nieśledzone",
        Notable::LocalChangesCarried => "niezatwierdzone zmiany przeniesiono na inną gałąź",
        Notable::FilterNotApplied => "filtr zawartości repozytorium nie został zastosowany",
        Notable::CredentialHelperIgnored => {
            "skonfigurowany pomocnik poświadczeń nie został uruchomiony"
        }
        Notable::SshCommandIgnored => "skonfigurowane polecenie SSH nie zostało uruchomione",
        Notable::FsmonitorIgnored => "skonfigurowany monitor systemu plików nie został uruchomiony",
        Notable::SubmodulesNotUpdated => "podmoduły nie zostały zaktualizowane",
        Notable::SparseCheckout => "repozytorium używa rzadkiego wyewidencjonowania",
        Notable::ShallowRepository => "repozytorium ma płytką historię",
        Notable::LineEndingsConverted => {
            "końce wierszy zostały przekonwertowane przez ustawienia repozytorium"
        }
        Notable::UpstreamSet => "ustawiono gałąź nadrzędną",
        Notable::StashKept => "wpis schowka został zachowany po konflikcie",
        Notable::Signed => "zatwierdzenie lub znacznik zostały podpisane",
        Notable::AmendPublished => {
            "poprawione zatwierdzenie jest osiągalne z gałęzi śledzącej zdalnej"
        }
        Notable::CommitDetachedHead => "zatwierdzenie utworzono na odłączonym HEAD",
        Notable::OperationInProgress => {
            "zatwierdzenie utworzono w trakcie zmiany bazy, cherry-pick, cofnięcia lub bisekcji"
        }
        Notable::Excluded => "pominięto pliki dopasowane przez exclude",
        Notable::ReviewSkipped => {
            "nie przejrzano plików: nieśledzonych ponad limit rozmiaru, binarnych lub ponad limit liczby"
        }
        Notable::CheckpointWithoutIndex => {
            "punkt kontrolny utworzono bez indeksu, bo indeks ma konflikty"
        }
        Notable::LinkLoopSkipped => {
            "dowiązanie symboliczne prowadzące do własnego katalogu nadrzędnego nie zostało odwiedzone podczas przeglądania drzewa"
        }
        Notable::TlsVerificationDisabled => {
            "konfiguracja repozytorium wyłącza weryfikację certyfikatów TLS"
        }
        Notable::HttpOptionIgnored => {
            "ustawienie HTTP konfiguracji repozytorium nie jest obsługiwane i zostało pominięte"
        }
        Notable::PackWithoutDeltas => "wypchnięcie wysłało obiekty bez nowej kompresji różnicowej",
        Notable::PushStatusUnknown => {
            "zdalne repozytorium nie zgłosiło wyniku aktualizacji referencji"
        }
        Notable::HostKeyUnverified => "klucz hosta zdalnego repozytorium przyjęto bez weryfikacji",
        Notable::MatchExecIgnored => "warunek Match exec konfiguracji SSH nie został uruchomiony",
        Notable::SigningProgramIgnored => {
            "program podpisujący z konfiguracji repozytorium nie został uruchomiony; podpis utworzono bez niego"
        }
        Notable::CredentialsInUrl => "użyto poświadczeń zapisanych w adresie zdalnego repozytorium",
        Notable::LimitReached => "wyjście zatrzymano na liczbie wpisów podanej w limit",
        Notable::InteractiveProgram => {
            "uruchomiono program uznany za interaktywny: edytor, pager, monitor terminala albo interpreter bez skryptu; może czekać na wejście z terminala"
        }
        Notable::OpaqueScript => {
            "program uruchomił skrypt podany w argumentach, na przykład bash -c lub python -c; skutki skryptu nie są sklasyfikowane"
        }
        _ => return None,
    })
}

/// Polish text of the reason of an irreversible effect, `None` for a reason
/// without one.
#[must_use]
pub fn irreversible_text(reason: Irreversible) -> Option<&'static str> {
    Some(match reason {
        Irreversible::ExternalEffect => {
            "uruchomiono program ze skutkiem poza lokalnym systemem plików"
        }
        Irreversible::RemoteRefUpdated => {
            "zaktualizowano lub pobrano referencje zdalnego repozytorium"
        }
        Irreversible::Network => "wysłano żądanie sieciowe zmieniające stan zdalny",
        Irreversible::Database => "zmieniono bazę danych bez zachowanej kopii",
        Irreversible::ProcessStopped => "zatrzymano proces lub zadanie",
        Irreversible::ServiceChanged => "zmieniono stan usługi systemowej",
        Irreversible::PackageChanged => {
            "zainstalowano, usunięto lub zaktualizowano pakiety systemowe"
        }
        Irreversible::PreviousContentNotKept => {
            "program zmienił plik, którego poprzednia zawartość nie została zachowana"
        }
        Irreversible::NoSpaceForCopy => "zabrakło miejsca na kopię poprzedniej zawartości",
        Irreversible::JournalInactive => {
            "dziennik jest wyłączony albo jego usługa miejsca nie działa"
        }
        Irreversible::NetworkVolume => "cel leży na woluminie sieciowym bez dziennika",
        Irreversible::RemovableVolume => "cel leży na woluminie wymiennym bez dziennika",
        Irreversible::Purged => "wpisy dziennika zostały wyczyszczone",
        Irreversible::SpecialFile => {
            "zapisano plik specjalny (urządzenie, potok, gniazdo, pseudo system plików)"
        }
        Irreversible::TooLargeForJournal => {
            "operacja jest większa niż limit pojedynczego wpisu dziennika"
        }
        Irreversible::ReleasedForSpace => {
            "zachowaną zawartość części operacji zbiorczej zwolniono, by odzyskać miejsce na dysku"
        }
        Irreversible::JobOutputRemoved => {
            "zachowane wyjście zadania w tle usunięto razem z zadaniem"
        }
        Irreversible::ProcessStateChanged => {
            "zmieniono stan działającego procesu: wstrzymano, wznowiono, wysłano sygnał lub zmieniono priorytet"
        }
        Irreversible::JournalEntriesExpired => {
            "wpisy dziennika wcześniejszych operacji wygasły z powodu tej operacji"
        }
        Irreversible::JournalError => {
            "dziennik zawiódł podczas zapisu operacji; zmianę wykonano bez zachowania poprzedniej zawartości"
        }
        _ => return None,
    })
}

/// Joins the parts of a translated message.
fn compose(meaning: &str, error: &Error) -> String {
    let mut parts = vec![meaning.to_owned()];
    if let Some(found) = &error.found {
        parts.push(format!("znaleziono: `{found}`"));
    }
    if let Some(expected) = error.expected.as_ref().and_then(expected_text) {
        parts.push(format!("oczekiwano: {expected}"));
    }
    if let Some(problem) = &error.problem {
        parts.push(format!("problem: {problem}"));
    }
    parts.join("; ")
}

/// Text of the expected value: the first names of a list, or a scalar.
fn expected_text(expected: &Value) -> Option<String> {
    match expected {
        Value::Array(items) => {
            let names: Vec<String> = items
                .iter()
                .take(MAX_EXPECTED_SHOWN)
                .map(|item| {
                    item.as_str()
                        .map_or_else(|| item.to_string(), str::to_owned)
                })
                .collect();
            (!names.is_empty()).then(|| names.join(", "))
        }
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Translates the message of an error and of its causes into `language`.
///
/// English leaves the error as it is. The English message is kept in the
/// detail [`MESSAGE_EN_DETAIL`]; an error that was translated once is not
/// translated again.
pub fn localize_error(error: &mut Error, language: Language) {
    if language != Language::Pl || error.details.contains_key(MESSAGE_EN_DETAIL) {
        return;
    }
    if let Some(meaning) = error_text(error.code()) {
        let message = compose(meaning, error);
        let english = std::mem::replace(&mut error.message, message);
        error
            .details
            .insert(MESSAGE_EN_DETAIL.to_owned(), Value::String(english));
    }
    if let Some(cause) = error.cause.as_mut() {
        localize_error(cause, language);
    }
}

/// Appends the English detail to a Polish text.
///
/// A detail that already begins with the Polish text was translated before
/// and is kept whole, so that a second translation does not drop the English
/// part.
fn with_detail(text: &str, detail: Option<&String>) -> String {
    match detail {
        Some(detail) if detail.starts_with(text) => detail.clone(),
        Some(detail) if !detail.is_empty() => format!("{text}; {detail}"),
        _ => text.to_owned(),
    }
}

/// Translates the error, the notes and the irreversible effects of a report
/// into `language`.
///
/// English leaves the report as it is.
pub fn localize(report: &mut Report, language: Language) {
    if language != Language::Pl {
        return;
    }
    if let Some(error) = report.error.as_mut() {
        localize_error(error, language);
    }
    for entry in &mut report.notable {
        if let Some(text) = notable_text(entry.code) {
            entry.detail = Some(with_detail(text, entry.detail.as_ref()));
        }
    }
    for entry in &mut report.irreversible {
        if let Some(text) = irreversible_text(entry.reason) {
            entry.detail = Some(with_detail(text, entry.detail.as_ref()));
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::report::{IrreversibleEntry, NotableEntry, ReportStatus};

    use super::*;

    #[test]
    fn every_error_code_has_a_polish_text() {
        for code in ErrorCode::ALL {
            assert!(error_text(*code).is_some(), "{code}");
        }
    }

    #[test]
    fn every_note_code_has_a_polish_text() {
        for code in Notable::ALL {
            assert!(notable_text(*code).is_some(), "{code:?}");
        }
    }

    #[test]
    fn every_irreversible_reason_has_a_polish_text() {
        for reason in Irreversible::ALL {
            assert!(irreversible_text(*reason).is_some(), "{reason:?}");
        }
    }

    #[test]
    fn polish_texts_differ_from_the_english_meanings() {
        for note in crate::errors::NOTES {
            assert_ne!(error_text(note.code), Some(note.meaning), "{}", note.code);
        }
    }

    #[test]
    fn error_message_is_translated_with_its_parameters_and_the_code_is_kept() {
        let mut error = Error::new(ErrorCode::UnknownCommand, "unknown command `remvoe`")
            .with_found("remvoe")
            .with_expected(serde_json::json!(["remove", "rename"]))
            .with_problem("typo")
            .with_fix("remove \"/srv/a\"");
        localize_error(&mut error, Language::Pl);
        assert_eq!(error.code(), ErrorCode::UnknownCommand);
        assert_eq!(
            error.message,
            "polecenia nie ma w katalogu; znaleziono: `remvoe`; oczekiwano: remove, rename; problem: typo"
        );
        assert_eq!(
            error.details[MESSAGE_EN_DETAIL],
            Value::from("unknown command `remvoe`")
        );
        assert_eq!(error.fix, ["remove \"/srv/a\""]);
    }

    #[test]
    fn english_and_repeated_translation_change_nothing() {
        let original = Error::new(ErrorCode::Assert, "expected 12 paths, found 14");
        let mut english = original.clone();
        localize_error(&mut english, Language::En);
        assert_eq!(english, original);
        let mut polish = original;
        localize_error(&mut polish, Language::Pl);
        let once = polish.clone();
        localize_error(&mut polish, Language::Pl);
        assert_eq!(polish, once);
    }

    #[test]
    fn causes_are_translated_too() {
        let cause = Error::new(ErrorCode::Io, "disk error");
        let mut error = Error::new(ErrorCode::StageFailed, "stage failed").with_cause(cause);
        localize_error(&mut error, Language::Pl);
        let cause = error.cause.as_ref().unwrap();
        assert_eq!(cause.message, "błąd wejścia lub wyjścia systemu plików");
    }

    #[test]
    fn report_notes_and_irreversible_effects_get_polish_text_and_keep_the_detail() {
        let mut report = Report::new(ReportStatus::Ok, "find");
        let mut note = NotableEntry::new(Notable::Excluded);
        note.count = Some(3);
        note.detail = Some("3 paths left out by `exclude`".to_owned());
        report.notable.push(note);
        report
            .notable
            .push(NotableEntry::new(Notable::NothingMatched));
        report
            .irreversible
            .push(IrreversibleEntry::new(Irreversible::Network));
        localize(&mut report, Language::Pl);
        assert_eq!(
            report.notable[0].detail.as_deref(),
            Some("pominięto pliki dopasowane przez exclude; 3 paths left out by `exclude`")
        );
        assert_eq!(report.notable[0].count, Some(3));
        assert_eq!(
            report.notable[1].detail.as_deref(),
            Some("program zmieniający nie dostał żadnych celów")
        );
        assert_eq!(
            report.irreversible[0].detail.as_deref(),
            Some("wysłano żądanie sieciowe zmieniające stan zdalny")
        );
        let before = report.clone();
        localize(&mut report, Language::Pl);
        assert_eq!(report, before);
    }

    #[test]
    fn english_report_is_not_changed() {
        let mut report = Report::new(ReportStatus::Ok, "find");
        report
            .notable
            .push(NotableEntry::new(Notable::NothingMatched));
        let before = report.clone();
        localize(&mut report, Language::En);
        assert_eq!(report, before);
    }
}
