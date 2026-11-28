---
name: bash-migration
description: Nawyki z basha i sposób, w jaki Nurt je poprawia lub tłumaczy, w tym warunki, pętle, funkcje, parametry skryptu i polecenia administracyjne.
commands: [remove, move, copy, create, read, find, search, replace, write, run, list, lines, attributes]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/bash-migration.md -->
<!-- Responsibility:   Documents bash-migration. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# migracja z basha {#bash-migration}

## Co jest poprawiane {#what-is-corrected}

Wiersz, który zaczyna się od polecenia basha, jest odczytywany według reguł powłoki POSIX i tłumaczony na program Nurt. Wynik pokazuje `corrected` i `ran`, czyli wykonany program. Tłumaczenie, które opiera się na założeniu i dotyka celu zmiany, nie jest stosowane: odpowiedzią jest E_AMBIGUOUS z gotowymi programami.

Wiersz jest poleceniem basha, gdy jego pierwsze słowo jest poleceniem basha i występuje w nim zapis właściwy wyłącznie powłoce: flaga, taka jak `-rf`, operator, taki jak `&&` lub `>`, cytowany `'text'`, `$NAME`, przypisanie przed programem (`CI=1 npm test`) lub niecytowany wzorzec. Pierwsze słowo, którego Nurt w ogóle nie zna (`touch`, `gzip`, `sha256sum`), nie wymaga żadnego z nich, a słowo, które Nurt odczytuje jako synonim (`cat`, `ps`, `which`), jest poleceniem basha, gdy jego argumentem jest samo słowo, którego Nurt nie może przyjąć (`ps aux`, `which git`). Wiersz z zapisem Nurt, potokiem `|>` lub argumentem nazwanym, takim jak `glob:`, jest zawsze programem Nurt.

```text
rm -rf /srv/app/build
```

Wiersz jest poprawiany do `remove "/srv/app/build"`. Postać do bezpośredniego zapisu używa wzorca, a konsola wymienia obliczone przez siebie cele:

```nurt
find "/srv/app" glob: "**/*.tmp" |> remove
```

## Polecenia i flagi {#commands-and-flags}

`mv A B` staje się `move "A" to: "B"`, `cp -r A B` staje się `copy "A" to: "B"`, `mkdir -p D` staje się `create "D" type: directory`, `touch F` staje się `create "F" type: file on_exists: touch`, `cat F` staje się `read "F"`, a `ls D` staje się `list "D"`. Rekurencja i tworzenie brakujących katalogów nadrzędnych są wartością domyślną, więc `-r` i `-p` nie potrzebują odpowiednika.

Flaga długa lub krótka staje się parametrem nazwanym polecenia o tym samym znaczeniu: `grep -i` staje się `case_sensitive: false`, `grep -v` staje się `invert: true`, `grep -l` staje się `output: files`. Flaga bez odpowiednika jest błędem, który wymienia parametry polecenia.

## Podwyższanie uprawnień {#privilege-elevation}

`sudo CMD`, `doas CMD` i `run0 CMD` stają się tłumaczeniem `CMD` z parametrem wywołania `privilege: elevated`, a `sudo -u NAME CMD` (także `--user NAME`, `--user=NAME`) staje się tłumaczeniem `CMD` z `as_user: "NAME"`. Opcje `-n`, `-H`, `-E`, `-k`, `-b` i `--` są pomijane. `sudo systemctl restart nginx` staje się `services name: "nginx" |> restart` z `privilege: elevated`; `sudo PROGRAM ARGS`, gdzie `PROGRAM` nie ma odpowiednika, staje się `run ["PROGRAM", ARGS...] cwd: "..."` z `privilege: elevated`. `sudo -i`, `sudo -s`, `su` i `su -` otwierają sesję z podwyższonymi uprawnieniami, która nie jest poleceniem: kończą się błędem `E_AMBIGUOUS` i następnym krokiem podającym polecenie z `privilege: elevated`. Grupa (`sudo -g`) i inne opcje bez odpowiednika są zgłaszane wraz z opcją. Poprawka `run ["sudo", ...]` zapisanego w Nurt jest zastępowana tym samym `run` bez `sudo` i z `privilege: elevated`.

## Przekierowania i zmienne {#redirection-and-variables}

`echo "x" > F` staje się `write "F" "x\n" mode: overwrite`, a `>>` staje się `mode: append`. Dokument here zapisany do pliku staje się `write` z tekstem surowym.

`$NAME` staje się `@NAME`, gdy program ma pasujące `let`. Wewnątrz tekstu podstawienie nie jest tłumaczone: zamiast tego należy zapisać `@name.path` jako argument.

```nurt
let files = find "/srv/app" glob: "**/*.go"
run ["wc", "-l", ...@files.path] cwd: "/srv/app"
```

## Potoki, listy i sekwencje {#pipes-lists-and-sequences}

Potok `A | B` staje się `A |> B`, gdy obie strony się tłumaczą. `find D -name P -delete` staje się `find "D" glob: "**/P" |> remove`. `xargs` i `find -exec` stają się etapem zmieniającym po `find` albo `run`, które otrzymuje `...@files.path`.

Lista `A && B && C` lub `A ; B` z kilkoma poleceniami zmieniającymi staje się jednym blokiem `steps`: kroki są wykonywane po kolei, blok jest jednym wpisem dziennika, a niepowodzenie dowolnego kroku wycofuje wcześniejsze (krok `run` jest wymieniany jako nieodwracalny). W pojedynczym wywołaniu odpowiedzią jest `E_ONE_EFFECT`; `candidates` ma blok `steps` jako pierwszy program, a `next_steps` ma osobne wywołania.

```text
mkdir -p /srv/out && cp -a /srv/in/data.csv /srv/out/
```

```nurt
steps {
    create "/srv/out" type: directory
    copy "/srv/in/data.csv" into: "/srv/out"
}
```

Lista z jednym poleceniem zmieniającym pozostaje tym poleceniem. `cd D && make && make install` staje się blokiem `steps` z dwóch kroków `run` z `cwd: "D"`.

## Warunki {#conditions}

`if`, `elif` i `else` stają się `when`, `otherwise when` i `otherwise`. Testy `[ … ]`, `[[ … ]]` i `test` stają się wyrażeniem `bool`: `-e`, `-f`, `-d`, `-L`, `-s` (testy plików, sprawdzane przez `exists` i `info`), `-z`, `-n`, `=`, `==`, `!=`, `=~` (wyrażenie regularne), `-eq`, `-ne`, `-gt`, `-ge`, `-lt`, `-le`, z `!`, `-a`, `-o`, `&&` i `||` jako `not`, `and` i `or`. Polecenie użyte jako warunek (`if systemctl is-active x`) ma tłumaczenie tylko wtedy, gdy jest `run`; jego kodem zakończenia jest `r.exit_code == 0` po `let r = run …`.

```text
if [ -f /srv/a ]; then rm /srv/a; fi
```

```nurt
when exists("/srv/a") and ((info "/srv/a" |> first 1)).type == "file" {
    remove "/srv/a" on_missing: error
}
```

`case` staje się tym samym łańcuchem: każdy wzorzec to `==` dla literału i `~` dla wzorca z `*`, `?` lub `[…]`, `a|b)` to `or`, a `*)` to `otherwise`.

`test -f F || CMD` i `[ … ] || CMD` stają się `when not (…) { CMD }`. `CMD || true` to `CMD` z założeniem, że jego niepowodzenie nie jest już ignorowane. Każde inne `A || B` nie ma tłumaczenia: błąd wymienia `A` i `B` jako osobne programy. `[ … ] && CMD` to `when … { CMD }`.

## Pętle {#loops}

`for x in PATTERN; do … done` staje się `each (find … ) as x { … }`: zmienna jest rekordem, a `$x` staje się `@x.path`. `for x in $(CMD)` to `each (CMD) as x`, gdy `CMD` odczytuje rekordy. `for x in a b c` to `each ["a", "b", "c"] as x`. `for x in "$@"` to `each @args as x` z parametrem skryptu `args`. `while [ … ]; do … done` staje się `while … { … }`, a `until [ … ]; do …; sleep N; done` staje się `repeat until: … every: Ns { … }` (treść wykonuje się raz przed pierwszym testem, co tłumaczenie zgłasza jako założenie). `break` i `continue` pozostają bez zmian.

Nie są tłumaczone: arytmetyczne `for ((…))`, `select`, treść pętli używająca `$x` wewnątrz dłuższego tekstu (należy zapisać `@x.path`) oraz potok do `while read`.

## Funkcje i parametry skryptu {#functions-and-script-parameters}

`f() { … }` i `function f { … }` stają się `define f(…) { … }`. Parametr jest nazywany od zmiennej, która przyjmuje `$1`, `$2` (`dir="$1"`, `days=${2:-30}`), a w przeciwnym razie nazywa się `arg1`, `arg2`, z adnotacją w raporcie. Wywołanie `f x y` staje się `f("x", "y")`.

W skrypcie `$1`, `$2` i `$@` poza funkcjami stają się blokiem `params`, który rozpoczyna program, typ jest wyprowadzany z użycia (`path` dla operandu `rm`, `cp` i testów plików, `int` dla `-eq`, `-gt` i innych testów liczbowych, w przeciwnym razie `text`), wartość domyślna `${2:-30}` jest wartością domyślną parametru, `shift` przesuwa numerację, `$@` jest listą `args`, a `getopts` deklaruje jeden parametr na opcję: `bool` dla flagi, `text` dla opcji z argumentem. Program jest wywoływany przez `nurt run script.nurt dir: "/srv" days: 7`.

```text
dir="$1"
keep=${2:-7}
rm -rf "$dir"
```

```nurt
params {
    dir: path
    keep: text = "7"
}
remove @dir
```

## Administracja kontami, montowaniami i jądrem {#administration-of-accounts-mounts-and-the-kernel}

| Bash | Nurt |
|---|---|
| `mkfifo [-m MODE] P` | `create "P" type: fifo mode: "MODE"` |
| `mknod P p`, `mknod P c|b MAJOR MINOR` | `create "P" type: fifo`, `create "P" type: device kind: char|block major: MAJOR minor: MINOR` |
| `useradd [-m] [-s SHELL] [-d HOME] [-G G1,G2] [-c TEXT] [-u UID] [-r] NAME` | `accounts.create name: "NAME" …` |
| `usermod [-s] [-d] [-G] [-c] [-L or -U] NAME` | `accounts.modify name: "NAME" …` (`-L` to `lock: true`, `-U` to `lock: false`) |
| `usermod -aG GROUP NAME` | `groups.add_member name: "GROUP" member: "NAME"` |
| `userdel [-r] NAME` | `accounts.remove name: "NAME" keep_home: true`; z `-r`, `keep_home: false` |
| `groupadd [-g GID] [-r] NAME`, `groupdel NAME` | `groups.create name: "NAME" …`, `groups.remove name: "NAME"` |
| `gpasswd -a USER GROUP`, `gpasswd -d USER GROUP` | `groups.add_member`, `groups.remove_member` |
| `passwd NAME` | `accounts.password name: "NAME"`; hasło pochodzi z odwołania do sekretu silnika, nigdy z programu |
| `mount [-t T] [-o O1,O2] DEVICE TARGET`, `umount TARGET` | `mount device: "DEVICE" target: "TARGET" type: "T" options: ["O1", "O2"]`, `unmount target: "TARGET"` |
| `sysctl -w KEY=VALUE` (jeden klucz na wywołanie) | `sysctl.set key: "KEY" value: "VALUE"` |

## Moduły i zapora {#modules-and-firewall}

| Bash | Nurt |
|---|---|
| `modprobe MODULE [PARAM…]`, `modprobe -r MODULE`, `rmmod MODULE` | `modules.load name: "MODULE" params: […]`, `modules.unload name: "MODULE"` |
| `ufw allow or deny PORT[/PROTO]`, `ufw allow from SOURCE to any port PORT proto PROTO` | `firewall.allow` lub `firewall.deny` z `port: PORT protocol: "PROTO" source: "SOURCE"` |
| `firewall-cmd [--zone=Z] --add-port=PORT/PROTO`, `--remove-port=…` | `firewall.allow port: PORT protocol: "PROTO" zone: "Z"`, `firewall.remove …` |
| `nft add rule … input or output tcp or udp dport PORT accept or drop`, `iptables -A INPUT or OUTPUT -p tcp or udp --dport PORT -j ACCEPT or DROP [-s SOURCE]` | `firewall.allow` lub `firewall.deny` z `port: PORT protocol: "tcp" direction: "in"` lub `"out"` |

## Trasy, serwery nazw, zegar, zasilanie i dyski {#routes-name-servers-clock-power-and-disks}

| Bash | Nurt |
|---|---|
| `ip route add DEST via GW dev IF metric N`, `ip route del DEST` | `network.route_add destination: "DEST" gateway: "GW" interface: "IF" metric: N`, `network.route_remove destination: "DEST"` |
| `resolvectl dns IF SERVER…` | `network.dns_set interface: "IF" servers: ["SERVER", …]` |
| `timedatectl set-timezone ZONE`, `date -s "TIME"` | `clock.timezone_set "ZONE"`, `clock.set "TIME"` |
| `shutdown -r or -h [now or +MINUTES] [MESSAGE]`, `shutdown -c`, `reboot`, `poweroff`, `halt` | `power.restart` lub `power.shutdown` z `delay: Nm` i `message: "…"`, `power.cancel`, `power.restart`, `power.shutdown` |
| `parted -s DEVICE rm N`, `parted -s DEVICE mkpart … START END` | `disks.partition device: "DEVICE" action: "delete" number: N`, `action: "create" start: "START" end: "END"` |
| `mkfs.FS [-L LABEL] DEVICE`, `mkfs -t FS DEVICE` | `disks.format device: "DEVICE" fs: "FS" label: "LABEL"` |
| `sqlite3 DB "SQL"` | `sql "DB" "SQL"` |

Reguła wykraczająca poza regułę portu (`iptables -t nat …`, reguła z modułem dopasowania) jest uruchamiana przez `run ["iptables", …] cwd: "/"`, a z `privilege: elevated`, gdy została zapisana z `sudo`. Postać z opcją, która nie ma odpowiednika, jest błędem wskazującym tę opcję. `sudo` przed którymkolwiek z tych poleceń dodaje `privilege: elevated`, jak przy każdym poleceniu.

## Co jest odrzucane {#what-is-rejected}

Tylko postacie zależne od samej powłoki nie mają tłumaczenia: `eval`, podpowłoki `( … )` i grupy `{ …; }`, arytmetyczne `for ((…))`, `select`, wejście interaktywne (`read`) oraz dokumenty here inne niż `cat > file`. `cd D && CMD` jest tłumaczone tylko dla ścieżki bezwzględnej w tym samym wierszu, jako `cwd` polecenia `run` lub jako ścieżki rozwiązywane względem `D`; inne użycia `cd` są odrzucane z adnotacją o parametrze wywołania `base`. Skrypt, który potrzebuje którejś z nich, działa bez zmian przez `run ["bash", "script.sh"] cwd: "/srv/app"`.

## Skrypty {#scripts}

Istniejący skrypt powłoki jest uruchamiany bez zmian przez `run ["bash", "script.sh"] cwd: "/srv/app"`. Polecenie `nurt migrate` tłumaczy skrypt na program Nurt z warunkami, pętlami, funkcjami i `params` oraz zgłasza to, czego nie udało się przetłumaczyć. Funkcja zdefiniowana w skrypcie jest `define`, a jej późniejsze wywołania są wywołaniami.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
