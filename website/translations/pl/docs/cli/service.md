---
command: service
group: schedule
summary: Uruchamia, instaluje, usuwa i sprawdza usługę działającą w tle.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cli/service.md -->
<!-- Responsibility:   Documents service. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# service {#service}

## Składnia {#synopsis}

```text
nurt service run
nurt service install [--mode user|system]
nurt service uninstall [--mode user|system]
nurt service status
```

## Opis {#description}

Uprzywilejowana usługa podniesienia uprawnień (`nurt service install --elevation`, `uninstall --elevation` oraz jej wiersz w `nurt service status`) nie wchodzi w skład wydania 0.1.2; do czasu jej wydania backend podniesienia uprawnień `broker` jest zgłaszany jako `not_installed`, a wywołania z podniesionymi uprawnieniami używają `sudo`, `doas` lub `run0`.

`service` jest podpoleceniem programu `nurt`, a nie poleceniem języka potoków.
Usługa działająca w tle hostuje moduł wolnego miejsca dziennika oraz moduł
harmonogramu. Dziennik przechowuje poprzednią zawartość zmienionych plików
tylko wtedy, gdy usługa działa, a harmonogramy uruchamiają się tylko wtedy,
gdy usługa działa; bez niej harmonogram zgłasza `active: false` z `reason:
service_not_running`.

`run` uruchamia usługę na pierwszym planie. Zakłada blokadę instancji
`service.lock` w katalogu stanu, usuwa gniazdo sterujące pozostawione przez
poprzednią usługę, otwiera kanał sterujący, uruchamia każdy moduł w jego
własnym wątku i zapisuje sygnał żywotności `service.hb` co `space.heartbeat_ms`
milisekund, co najmniej raz na sekundę. Moduł, który zawiedzie, jest
restartowany po 1 s, 2 s, 4 s i tak dalej do 60 s; po pięciu restartach w ciągu
dziesięciu minut pozostaje w stanie `failed` do czasu przeładowania
konfiguracji. Awaria jednego modułu nie zatrzymuje drugiego. `SIGTERM` i
`SIGINT` zatrzymują usługę, `SIGHUP` przeładowuje konfigurację. W systemie
Windows menedżer sterowania usługami zatrzymuje usługę poleceniem `Stop` lub
`Shutdown`.

`install` rejestruje usługę w menedżerze usług systemu operacyjnego, uruchamia
ją i zapisuje manifest instalacji `service.install.json` w katalogu stanu.
`uninstall` zatrzymuje usługę, usuwa rejestrację i usuwa dokładnie pliki
wymienione w manifeście.

| System | Tryb `user` | Tryb `system` |
|---|---|---|
| Linux | jednostka użytkownika systemd `nurt-service.service` w katalogu jednostek menedżera użytkownika | jednostka systemd `/etc/systemd/system/nurt-service.service` z `User=` i `Group=` konta usługi |
| macOS | agent launchd `nurt.service` w `$HOME/Library/LaunchAgents` | demon launchd `/Library/LaunchDaemons/nurt.service.plist` z `UserName` |
| Windows | zadanie `Nurt Service` Harmonogramu zadań, uruchamiane przy logowaniu | usługa `NurtService` menedżera sterowania usługami, uruchamiana przy starcie systemu |

W trybie `user` w systemie Linux plik jednostki jest zapisywany w katalogu, który odczytuje sam menedżer użytkownika systemd: pierwszym katalogu `user` wewnątrz katalogu `systemd` we właściwości `UnitPath` menedżera (odczytywanej przez D-Bus) poza `/etc`, `/run`, `/usr` i `/lib`, korzeniami katalogów systemu, środowiska wykonawczego i dostawców. Menedżer może działać z innym `HOME` niż konto uruchamiające `install`, a jednostka zapisana pod `HOME` sesji instalującej byłaby dla niego niewidoczna (`NoSuchUnit`). Tylko gdy menedżer jest nieosiągalny lub nie zgłasza takiego katalogu, jednostka jest zapisywana w `$XDG_CONFIG_HOME/systemd/user` (domyślnie `$HOME/.config/systemd/user`). Jednostka zawiera `Environment="HOME=<home>"` oraz `Environment="XDG_STATE_HOME=<dir>"`, gdy sesja instalująca go ustawia, aby usługa używała katalogu stanu używanego przez jej klientów, niezależnie od `HOME` menedżera.

`ExecStart` nazywa program ścieżką, z którą został uruchomiony (`nurt` znalezione w `PATH`, ścieżka względna zamieniona na bezwzględną, ścieżka bezwzględna), z zachowaniem dowiązań, o ile ta ścieżka prowadzi do działającego programu; dowiązanie takie jak `current` lub `bin/nurt` nadal wskazuje więc zainstalowaną wersję po aktualizacji. Tylko gdy ścieżki nie da się ustalić lub prowadzi ona do innego pliku, zapisywany jest rozwiązany plik działającego procesu. Ścieżka pozostaje w manifeście jako `executable`.

Bez `--mode` tryb jest wybierany na podstawie hosta. Konto bez uprawnień
administratora otrzymuje tryb `user`. Przy uprawnieniach administratora host bez
sesji graficznej, którego domyślnym celem systemd jest `multi-user.target`, konto,
którego powłoką logowania jest `nologin` lub `false`, oraz wydanie Windows Server
otrzymują tryb `system`; pozostałe hosty otrzymują tryb `user`. Wynik nazywa
przyczynę wyboru.

### Utrzymywanie sesji i sesje logowania {#lingering-and-login-sessions}

W trybie `user` w systemie Linux menedżer użytkownika systemd działa tylko
wtedy, gdy konto ma sesję logowania, chyba że dla konta włączono utrzymywanie
sesji (lingering). `install` włącza utrzymywanie sesji przez
`org.freedesktop.login1.Manager.SetUserLinger` i zgłasza wynik w `linger`. Gdy
system odrzuca żądanie, na przykład ponieważ polityka polkit wymaga
administratora, instalacja nadal się udaje z `linger: false` i uwagą: usługa
uruchamia się, gdy konto się loguje, a do tego czasu dziennik nie przechowuje
poprzedniej zawartości, a harmonogramy zgłaszają `service_not_running`. Tryb
`system` nie zależy od sesji logowania.

Gdy menedżer użytkownika systemd jest w ogóle nieosiągalny (brak
`XDG_RUNTIME_DIR`, sesja bez `pam_systemd`, kontener), `install` zapisuje plik
jednostki i kończy się błędem `E_SERVICE`. Zaloguj się w sesji, w której działa
menedżer użytkownika, i powtórz polecenie albo zainstaluj w trybie `system`.

W systemie macOS bez sesji graficznej agent jest ładowany do domeny tła
`user/<uid>` i uruchamia się ponownie przy następnym logowaniu.

### Żywotność {#liveness}

`status` i każdy proces zależny od usługi stosują tę samą regułę. Usługa działa,
gdy istnieje `service.hb`, jego `beat` jest nie starszy niż
`space.heartbeat_max_age_ms` (domyślnie 2000 ms), istnieje proces `pid` sygnału
żywotności, a `ping` w kanale sterującym odpowiada w ciągu 50 ms.

### Kanał sterujący {#control-channel}

Kanał sterujący to gniazdo Unix `service.sock` w katalogu stanu (tryb `0600`,
katalog `0700`) albo, w systemie Windows, nazwany potok
`\\.\pipe\nurt-service-<SID>`, który dopuszcza wyłącznie konto i `SYSTEM`.
Każdy komunikat jest jednym obiektem JSON w jednym wierszu o długości co najwyżej
65536 bajtów.

| Komunikat | Postać |
|---|---|
| żądanie | `{"v":1,"id":<id>,"op":<op>,"body":{...}}`; `id` i `body` są opcjonalne |
| powodzenie | `{"v":1,"id":<id>,"ok":true,"result":{...}}` |
| niepowodzenie | `{"v":1,"id":<id>,"ok":false,"error":{"code":...,"message":...}}` |
| zdarzenie | `{"v":1,"event":<name>,"data":{...}}` |

| Operacja | Wynik |
|---|---|
| `ping` | `{"pid"}`; odpowiedź bez oczekiwania na jakikolwiek moduł |
| `status` | `{"pid","started","version","protocol","modules","subscribers"}` |
| `reload` | `{"modules":{<name>:"reloaded"\|"restarted"\|{"error"}}}` |
| `subscribe` | `{"subscribed":true}`; połączenie otrzymuje zdarzenia adresowane do wszystkich |
| `<module>.<op>`, `<module>/<op>` | odpowiedź modułu, na przykład `schedule.run` lub `schedule/ack` |

Błędy protokołu to `bad_request`, `unknown_op` (z `ops`),
`unsupported_version` (z `supported`), `line_too_long` (połączenie jest
zamykane) i `module_unavailable`. Błędy modułu zawierają kody `E_`. Połączenie,
które przez 30 sekund niczego nie wysyła, jest zamykane, chyba że zasubskrybowało
zdarzenia lub zarejestrowało rodzaj akcji zdalnej.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `--mode` | `user` \| `system` | `install`: wybierany na podstawie hosta; `uninstall`: zainstalowany tryb | Tryb instalacji. Tryb `system` wymaga uprawnień administratora. |

Klucze konfiguracji odczytywane przez `run`: `service.mode`, `space.heartbeat_ms`
(domyślnie `500`), `space.heartbeat_max_age_ms` (domyślnie `2000`), `state.dir`.
Klucze `service.*` w pliku `.nurt.toml` projektu są ignorowane.

## Wejście i wyjście {#input-and-output}

Podpolecenie nie czyta rekordów. `run` niczego nie wypisuje, dopóki usługa
działa. Pozostałe formy wypisują jeden obiekt.

| Forma | Wyjście |
|---|---|
| `install` | `{mode, mode_reason, backend, files, started, linger, notes}` |
| `uninstall` | `{mode, backend, removed, notes}` |
| `status` | `{installed, mode, running, reason, pid, started, version, linger, heartbeat_age_ms, modules}` |

`mode_reason` to `requested`, `no_privilege`, `server` lub `desktop`.
`backend` to `systemd_user`, `systemd_system`, `launch_agent`,
`launch_daemon`, `task_scheduler` lub `windows_service`. W `status` `reason` to
`no_heartbeat`, `stale_heartbeat`, `process_gone` lub `no_response`, gdy usługa
nie działa; `modules` odwzorowuje `space` i `schedule` na `starting`, `running`,
`degraded`, `restarting`, `failed` lub `stopped`.

## Skutek {#effect}

`run` zapisuje wyłącznie `service.lock`, `service.hb`, `service.sock` oraz, przez
moduł harmonogramu, `schedules/state.json` w katalogu stanu. Te pliki nie
przechodzą przez dziennik. Definicje harmonogramów w `schedules/definitions/` są
zmieniane wyłącznie przez konsolę.

`install` i `uninstall` zmieniają konfigurację menedżera usług. W systemie Linux
używają D-Bus (`org.freedesktop.systemd1`, `org.freedesktop.login1`), w systemie
Windows menedżera sterowania usługami i Harmonogramu zadań. W systemie macOS
uruchamiają program zewnętrzny `launchctl` (`bootstrap` i `bootout`) z limitem
czasu dziesięciu sekund.

## Przykłady {#examples}

Pokaż stan usługi zainstalowanej w trybie `user` w systemie Linux:

```nurt example
services name: "nurt-service" scope: user
```

Wymień aktywne harmonogramy, które nie uruchamiają się, ponieważ usługa nie
działa:

```nurt example
schedule.list state: active |> filter reason == "service_not_running" |> select label reason
```

Pokaż błędy zapisane w dzienniku przez jednostkę usługi:

```nurt example
logs unit: "nurt-service" level: error
```

Zainstaluj usługę dla bieżącego konta z wiersza poleceń:

```sh
nurt service install --mode user
```

```text
mode: user  mode_reason: requested  backend: systemd_user  started: true  linger: true
```

Pokaż, czy usługa jest zainstalowana i działa:

```sh
nurt service status
```

```text
installed: true  mode: user  running: true  linger: true  modules: {schedule: running, space: running}
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXISTS` | `run`: usługa już działa dla tego katalogu stanu; `details.pid` ją wskazuje | najpierw zatrzymaj działającą usługę |
| `E_PRIVILEGE` | tryb `system` bez uprawnień administratora; `details.required` to `administrator` | uruchom polecenie jako administrator albo użyj trybu `user` |
| `E_SERVICE` | menedżer usług odrzucił żądanie lub jest nieosiągalny | wykonaj krok wskazany w komunikacie, na przykład zaloguj się w sesji albo użyj trybu `system` |
| `E_EXTERNAL_MISSING` | macOS: `launchctl` jest niedostępny; `details.program` to `launchctl` | przywróć program systemowy |
| `E_NOT_FOUND` | `uninstall`: brak manifestu instalacji | usługa nie jest zainstalowana |
| `E_INVALID_VALUE` | `uninstall --mode` wskazuje inny tryb niż zainstalowany albo dwa moduły mają tę samą nazwę | użyj zainstalowanego trybu |
| `E_IO` | nie można zapisać pliku katalogu stanu ani menedżera usług | sprawdź uprawnienia katalogu w `details.path` |

## Powiązane {#related}

Klucze konfiguracji `service.mode`, `space.heartbeat_ms`,
`space.heartbeat_max_age_ms` i `state.dir`; polecenia grupy `schedule`;
`services`, `logs`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
