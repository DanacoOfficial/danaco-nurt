---
title: Nurt for bash users
---
# Nurt for bash users

Every row pairs a Bash command with its form in Nurt. The last column states whether Nurt corrects the habit automatically, rejects it with an error that carries the fix, or rejects it with an error that carries no program and asks for the program of the row.

The paths are those of a Linux host, and the call parameter `base` is `/home/user/app`.

## Files and directories

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B01 | `rm -rf /home/user/app/build` | `remove "/home/user/app/build"` | corrected automatically: recursion is implicit and a missing path is skipped |
| B02 | `rm /home/user/app/build/*.tmp` | `find "/home/user/app/build" glob: "*.tmp" depth: 1 hidden: false \|> remove on_missing: error` | corrected automatically: the glob is expanded as bash would, without recursion |
| B03 | `mv /home/user/a.txt /home/user/archive` | `move "/home/user/a.txt" to: "/home/user/archive"` | corrected automatically |
| B04 | `cp -r /home/user/app /home/user/backup` | `copy "/home/user/app" to: "/home/user/backup" preserve: false` | corrected automatically |
| B05 | `mkdir -p /home/user/app/out` | `create "/home/user/app/out" type: directory` | corrected automatically |
| B06 | `touch /home/user/app/a.txt` | `create "/home/user/app/a.txt" type: file on_exists: touch` | corrected automatically |
| B07 | `chmod -R 755 /home/user/app/bin` | `attributes "/home/user/app/bin" mode: "755" recursive: true` | corrected automatically |
| B09 | `ln -s /home/user/app/src /home/user/link` | `create "/home/user/link" type: link target: "/home/user/app/src"` | corrected automatically |
| B13 | `ls -la /home/user/app` | `list "/home/user/app"` | corrected automatically |
| B14 | `find /home/user/app -name '*.go' -type f` | `find "/home/user/app" glob: "**/*.go" type: file` | corrected automatically |
| B37 | `stat /home/user/app/a.txt` | `info "/home/user/app/a.txt"` | corrected automatically |

## Reading and searching text

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B10 | `cat /home/user/app/a.txt` | `read "/home/user/app/a.txt"` | corrected automatically |
| B11 | `tail -n 20 /home/user/app/a.log` | `read "/home/user/app/a.log" last: 20` | corrected automatically |
| B12 | `wc -l /home/user/app/a.txt` | `lines "/home/user/app/a.txt" \|> count` | corrected automatically |
| B15 | `grep -rni 'todo' /home/user/app/src` | `search /todo/ "/home/user/app/src" case_sensitive: false` | corrected automatically: a pattern in basic regular expression syntax is converted with an assumption that the report names |
| B16 | `sed -i 's/oldName/newName/g' /home/user/app/a.go` | `replace /oldName/ "newName" in: ["/home/user/app/a.go"]` | corrected automatically |
| B17 | `sed -n '10,20p' /home/user/app/a.go` | `read "/home/user/app/a.go" lines: 10..20` | corrected automatically |
| B37b | `sha256sum /home/user/app/a.tar.gz` | `hash "/home/user/app/a.tar.gz"` | corrected automatically |
| B37c | `diff /home/user/a.txt /home/user/b.txt` | `diff "/home/user/a.txt" "/home/user/b.txt"` | corrected automatically |

## Writing text

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B19 | `echo "dist/" >> /home/user/app/.gitignore` | `write "/home/user/app/.gitignore" "dist/\n" mode: append` | corrected automatically |

## Archives

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B34 | `tar -xzf /home/user/dl/data.tar.gz -C /home/user/data` | `archive.extract "/home/user/dl/data.tar.gz" into: "/home/user/data"` | corrected automatically |
| B34b | `gzip /home/user/app/a.log` | `compress "/home/user/app/a.log" keep: false` | corrected automatically |

## Processes and system

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B35a | `ps aux` | `processes` | corrected automatically |
| B35b | `df -h` | `disks` | corrected automatically |
| B35c | `ss -ltnp` | `ports protocol: tcp` | corrected automatically |
| B35d | `printenv` | `environment` | corrected automatically |
| B35e | `which git` | `program.find "git"` | corrected automatically |
| B36 | `kill -9 4132` | `processes pid: 4132 \|> stop signal: kill` | corrected automatically |

## Running programs

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B29 | `npm run build &` | `run ["npm", "run", "build"] cwd: "/home/user/app" background: true` | error without fix: `&` is not part of the language; write the program with background: true and the directory as cwd |
| B30 | `timeout 60 make test` | `run ["make", "test"] cwd: "/home/user/app" timeout: 60s` | error with fix: there is no current directory; the fix passes the directory as cwd or base |
| B20 | `cd /home/user/app && make` | `run ["make"] cwd: "/home/user/app"` | corrected automatically: only a change to an absolute directory in the same line is converted |
| B31 | `CI=1 npm test` | `run ["npm", "test"] cwd: "/home/user/app" env: {"CI": "1"}` | error with fix: there is no current directory; the fix passes the directory as cwd or base |

## Network

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B32 | `curl -s https://example.com/health` | `http.request "https://example.com/health" follow_redirects: false` | corrected automatically |
| B32b | `wget -O /home/user/a.tar.gz https://example.com/a.tar.gz` | `http.download "https://example.com/a.tar.gz" "/home/user/a.tar.gz"` | corrected automatically |

## Git

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B38 | `git -C /home/user/app status` | `git.status "/home/user/app"` | corrected automatically |

## Services and packages

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B40 | `systemctl restart nginx` | `services name: "nginx" \|> restart` | corrected automatically |
| B40b | `apt install -y ripgrep` | `package.install ["ripgrep"] manager: apt` | corrected automatically |

## Control flow

| Rule | Bash | Nurt | Result |
|---|---|---|---|
| B22 | `mkdir /home/user/out && cp /home/user/a.txt /home/user/out` | `create "/home/user/out" type: directory parents: false` | error with fix: two changing commands are not run in one program; the fix is one call per change, this is the first |
| B25 | `rm -rf "$BUILD"/*` | `find "/home/user/app/build" glob: "*" depth: 1 \|> remove` | error without fix: there are no variables in text; write the directory |
| B28 | `sudo systemctl restart nginx` | `services name: "nginx" \|> restart` | error with fix: the language has no privilege elevation |
