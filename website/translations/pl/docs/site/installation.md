---
title: Instalacja
order: 2
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/installation.md -->
<!-- Responsibility:   Documents Installation. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Instalacja {#installation}

Danaco Nurt to jeden plik wykonywalny dla każdego systemu operacyjnego: `nurt` w Linuksie i macOS, `nurt.exe` w Windows. Kompilacja musl dla Linuksa i kompilacja dla Windows nie mają zależności uruchomieniowych. Kompilacja glibc dla Linuksa dołącza dynamicznie biblioteki systemowe `libc.so.6`, `libm.so.6` i `libgcc_s.so.1`.

## Systemy i archiwa {#systems-and-archives}

Kod jest budowany i testowany w systemach z pierwszej tabeli. Druga tabela wymienia archiwa wydania 0.1.2.

| System | Architektury kodu | Archiwum wydania | Testy |
|---|---|---|---|
| Linux | x86_64, aarch64 | glibc i musl, każde dla x86_64 i aarch64 | ciągła integracja |
| Windows | x86_64, aarch64 | MSVC, x86_64 i aarch64 | ciągła integracja |
| macOS | x86_64, aarch64 | brak; zbuduj ze źródeł | ciągła integracja na środowiskach macOS; dla wydania 0.1.2 nie opublikowano archiwum, zobacz „Known issues” wydania 0.1.2 w `CHANGELOG.md` |

| Cel archiwum | System |
|---|---|
| `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` | Linux z glibc |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` | Linux, statyczny plik wykonywalny |
| `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | Windows 10 lub nowszy |

Program instalacyjny dla Windows wymaga Windows 10 w wersji 1809 (kompilacja 17763) lub nowszej i sprawdza to po uruchomieniu. `nurt.exe` jest budowany zestawem narzędzi Rusta, którego cele Windows obsługują Windows 10 i nowsze.

## Artefakty wydania {#release-artifacts}

| Kanał | Identyfikator | Stan |
|---|---|---|
| Wydanie GitHub | `github.com/DanacoOfficial/danaco-nurt` | archiwa i `checksums.txt` |
| Obraz kontenera | `ghcr.io/danacoofficial/nurt` | publikowany przez przepływ pracy `integrations-containers.yml` |
| winget | `Danaco.Nurt` | zgłaszany przez przepływ pracy `release.yml` |
| NuGet (powiązanie biblioteki) | `Danaco.Nurt` | planowane; repozytorium nie ma etapu publikacji |
| Maven (powiązanie biblioteki) | `pl.danaco-group:nurt` | planowane; repozytorium nie ma etapu publikacji |

Każde wydanie publikuje sumy SHA-256 swoich artefaktów. Instalatory `install.sh` i `install.ps1` weryfikują sumę, zanim umieszczą plik wykonywalny.

## Instalacja w Linuksie i macOS {#install-on-linux-and-macos}

```text
curl -fsSL https://nurt.danaco-group.pl/install.sh | sh
```

Skrypt instaluje `nurt` w `$HOME/.local/bin` i nie wymaga uprawnień. W Linuksie pobiera statyczne archiwum musl, gdy wydanie je zawiera, a w przeciwnym razie archiwum glibc. Aby zainstalować w całym systemie, uruchom skrypt jako root z `--bin-dir /usr/local/bin`. Opcja `--public-key` weryfikuje podpis `checksums.txt`; klucz i procedura znajdują się w `SECURITY.md`.

## Instalacja w Windows {#install-on-windows}

```text
irm https://nurt.danaco-group.pl/install.ps1 | iex
```

Domyślnie skrypt instaluje program dla bieżącego użytkownika. `nurt.exe` trafia do `%LOCALAPPDATA%\Programs\Nurt\bin`, dokumentacja do `%LOCALAPPDATA%\Programs\Nurt\doc`, a katalog pliku wykonywalnego jest dodawany do `PATH` użytkownika. Uprawnienia administratora nie są potrzebne.

Aby zainstalować dla wszystkich użytkowników, uruchom PowerShell przez „Uruchom jako administrator” i ustaw zakres przed wywołaniem:

```text
$env:NURT_SCOPE = 'machine'
irm https://nurt.danaco-group.pl/install.ps1 | iex
```

Zakres `machine` instaluje w `%ProgramFiles%\Nurt` i zmienia `PATH` komputera. Zmienna `NURT_INSTALL_DIR` wybiera inny katalog dla pliku wykonywalnego.

### Instalator systemu Windows {#windows-setup}

Każde wydanie zawiera kreator instalacji dla każdej architektury Windows, `nurt-<version>-<target>-setup.exe`, z celem `x86_64-pc-windows-msvc` lub `aarch64-pc-windows-msvc`. Plik zawiera archiwum programu z tego samego wydania, więc nie wymaga sieci. Kreator działa w systemie Windows 10 w wersji 1809 lub nowszej oraz w Windows 11; jego interfejs jest w języku angielskim i nie wymaga środowiska webowego.

1. Pobierz plik dla architektury komputera i sprawdź jego sumę względem `checksums.txt` wydania.
2. Uruchom plik. Odtwarzana jest krótka animacja; klawisz lub kliknięcie ją pomija.
3. Powitanie: kreator sprawdza wersję Windows, architekturę i wolne miejsce.
4. Licencja: przeczytaj `LICENSE` i `NOTICE` do końca; przycisk Next jest włączany na końcu tekstu.
5. Lokalizacja: wybierz „Only for me” (`%LOCALAPPDATA%\Programs\Nurt`, bez uprawnień administratora) lub „For all users of this PC” (`%ProgramFiles%\Nurt`; Windows prosi o zatwierdzenie administratora po rozpoczęciu instalacji) oraz folder `nurt.exe`.
6. Opcje: dodaj folder do `PATH`, zainstaluj dokumentację, utwórz skróty w menu Start, na pulpicie i na pasku szybkiego uruchamiania, dodaj profil Windows Terminal (opcjonalnie jako profil domyślny), dodaj uzupełnianie klawiszem Tab do PowerShell i zapisz pierwszy `nurt.toml`.
7. Przegląd, następnie Install: kreator weryfikuje sumę SHA-256 osadzonego archiwum, kopiuje pliki i wprowadza zmiany. Błąd lub Cancel cofa wszystkie dotychczas wprowadzone zmiany.
8. Zakończenie: wybierz, czy otworzyć terminal i uruchomić `nurt` oraz czy pokazać dokumentację, a następnie zamknij kreator.

Każda zmiana jest zapisywana w dzienniku `setup-journal.jsonl` w folderze instalacji (format `nurt-setup-journal/1`). Program jest wymieniony w „Zainstalowane aplikacje”; jego wpis, skrót „Uninstall Danaco Nurt” i `uninstall.exe` w folderze instalacji odwracają dziennik i usuwają program, jego skróty, jego wpis w `PATH` i jego profil Windows Terminal. `nurt.toml` pozostaje, chyba że podano `--remove-config`. Ponowne uruchomienie instalatora na komputerze z zainstalowanym programem oferuje Repair (lub Update, gdy instalator zawiera inną wersję) i Undo.

Tryb cichy instaluje bez okna:

```text
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --scope machine --terminal-profile --completions
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --dir=D:\Tools\Nurt\bin --no-path
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --undo
```

| Przełącznik | Skutek |
|---|---|
| `/S`, `--silent` | Bez okna; kod wyjścia zgłasza wynik. |
| `--scope user\|machine` | Dla bieżącego użytkownika (wartość domyślna) lub dla wszystkich użytkowników. |
| `--dir <path>`, `/D=<path>` | Folder `nurt.exe`. |
| `--no-path`, `--no-docs`, `--no-start-menu`, `--no-config` | Pomiń `PATH`, dokumentację, skróty menu Start lub `nurt.toml`. |
| `--desktop`, `--quick-launch` | Utwórz skrót na pulpicie lub na pasku szybkiego uruchamiania. |
| `--terminal-profile`, `--terminal-default`, `--completions` | Dodaj profil Windows Terminal, ustaw go jako profil domyślny, dodaj uzupełnianie klawiszem Tab do PowerShell. |
| `--version <X.Y.Z>` | Zakończ niepowodzeniem, jeśli instalator nie zawiera tej wersji. |
| `--repair`, `--undo`, `--remove-config` | Zainstaluj ponownie na istniejącej instalacji, odwróć dziennik, usuń także `nurt.toml`. |
| `--no-animation`, `--log <file>`, `-h` | Otwórz okno bez animacji, nazwij plik dziennika, wypisz sposób użycia. |

Kody wyjścia: 0 wykonano, 1 niepowodzenie i wycofanie zmian, 2 nieprawidłowe argumenty, 3 niespełnione wymagania, 4 odrzucone zatwierdzenie administratora.

Wydanie podpisuje `nurt.exe` i pliki instalatora podpisem Authenticode. Certyfikat jest samopodpisany (`packaging/release/danaco-nurt-codesign.pem` w repozytorium), więc Windows zgłasza nieznanego wydawcę, dopóki certyfikat nie zostanie uznany za zaufany. `Get-AuthenticodeSignature` pokazuje podpisującego `CN=Danaco Nurt`. Instalator nie może przypiąć `nurt` do paska zadań, ponieważ Windows nie ma do tego obsługiwanego interfejsu; przypnij skrót menu Start ręcznie. Pełny opis plików, dziennika i budowania znajduje się w `packaging/windows-installer/README.md` w repozytorium.

### Uprawnienia administratora {#administrator-rights}

`nurt.exe` nie żąda podniesienia uprawnień. Jego manifest deklaruje poziom wykonania `asInvoker`, więc program działa z uprawnieniami terminala, który go uruchamia. Operacje wymagające uprawnień administratora, takie jak `nurt service install`, zmiany w części komputerowej rejestru i zapisy do katalogów systemowych, działają w terminalu uruchomionym jako administrator. Manifest deklaruje także obsługę długich ścieżek i UTF-8 jako stronę kodową procesu.

## Zawartość archiwów {#contents-of-the-archives}

| Archiwum | Pliki |
|---|---|
| Program, Linux i macOS, katalog `nurt-<version>-<target>/` | `nurt`, `LICENSE`, `NOTICE`, `README.md`, `CHANGELOG.md`, `man/`, `completions/`, `doc/` |
| Program, Windows, pliki na najwyższym poziomie | `nurt.exe`, `LICENSE`, `NOTICE`, `README.md`, `CHANGELOG.md`, `completions/`, `doc/` |
| Kreator instalacji, Windows, jeden plik `nurt-<version>-<target>-setup.exe` | archiwum programu dla Windows i kreator, który je instaluje |
| Biblioteka C, Linux i macOS, katalog `nurt-lib-<version>-<target>/` | `libnurt.so.1` lub `libnurt.1.dylib`, `nurt.h`, `nurt.pc`, `LICENSE`, `NOTICE` |
| Biblioteka C, Linux musl, katalog `nurt-lib-<version>-<target>/` | `libnurt.a` (statyczna), `nurt.h`, `nurt.pc`, `LICENSE`, `NOTICE` |
| Biblioteka C, Windows, pliki na najwyższym poziomie | `nurt.dll`, `nurt.h`, `LICENSE`, `NOTICE` |

Archiwa biblioteki C są publikowane dla celów, które mają bibliotekę; instalatory `install.sh` i `install.ps1` instalują wyłącznie archiwum programu.

Katalog `doc/` zawiera tę dokumentację jako witrynę działającą bez sieci; otwórz `doc/index.html`. Katalog `completions/` zawiera skrypty uzupełniania dla Bash, Zsh, Fish, PowerShell i Elvish.

## Budowanie ze źródeł {#build-from-source}

Budowanie wymaga łańcucha narzędzi Rust zainstalowanego przez rustup oraz kompilatora C. Wersja łańcucha narzędzi jest przypięta w `rust-toolchain.toml`.

```text
git clone https://github.com/DanacoOfficial/danaco-nurt.git
cd danaco-nurt
cargo build --release --locked
```

Plik wykonywalny jest zapisywany w `target/release/nurt` (`target\release\nurt.exe` w Windows).

## Weryfikacja {#verification}

```text
nurt --version
```

Polecenie wypisuje wersję, wiersz z prawami autorskimi i licencję; format opisuje [Wiersz poleceń](command-line.md). `nurt doctor` raportuje właściwości systemu plików, uprawnienia, usługę i serwery języka, które znalazł.

## Licencja {#license}

Produkt jest objęty licencją Apache License, Version 2.0. Dystrybucje zawierają plik `NOTICE`. Zasady używania nazwy opisuje [Zgodność](compatibility.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
