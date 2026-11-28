<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           packaging/windows-installer/README.md -->
<!-- Responsibility:   Documents the setup wizard of Danaco Nurt for Windows: steps, files, switches, journal, build, signing and limits. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Setup wizard for Windows

The setup wizard is one executable, `nurt-<version>-<target>-setup.exe`, that carries the program archive `nurt-<version>-<target>.zip` of the same release. It installs `nurt.exe` for the current user or for all users, writes every change to a journal, and reverses the journal to repair or to remove the installation. The window is drawn by the program itself (Rust, egui and eframe); there is no web view and no external runtime. The interface is English only.

| Property | Value |
|---|---|
| Targets | `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` |
| Package | `nurt-setup`, outside the workspace of the repository (own `Cargo.lock`) |
| Version | equal to the version of the release; `tools/release/build-release.sh` stops when they differ |
| Windows | Windows 10 version 1809 (build 17763) or later, Windows 11, Windows Server 2019 or later |
| Architecture | the setup of a target runs on that architecture; the x86_64 setup also runs on aarch64 Windows through the emulation of the system |
| Graphics | a graphics adapter with Direct3D 12; without one, Windows provides the software rasterizer WARP and the window still opens |
| Administrator rights | only for the scope "for all users" |

## Files of the package

| Path | Content |
|---|---|
| `Cargo.toml`, `Cargo.lock` | the package `nurt-setup` and its locked dependencies |
| `build.rs` | embeds the program archive with its SHA-256 sum and compiles the Windows resources |
| `setup.rc.in`, `setup.manifest` | resource script (icon, version information) and application manifest (`asInvoker`, long paths, per-monitor DPI) |
| `assets/fonts/` | subsets of Geist and Geist Mono, and `OFL.txt` |
| `src/main.rs` | entry point: selects the wizard, the silent mode, the worker or the uninstaller |
| `src/core/` | engine without interface: `cli.rs` (switches), `engine.rs` (stages and undo), `journal.rs`, `registry.rs`, `payload.rs`, `requirements.rs`, `location.rs`, `paths.rs`, `silent.rs`, `app.rs` (elevation, background runs), `model.rs`, `error.rs`, `logfile.rs` |
| `src/ui/` | the window: `chrome.rs` (rail and caption bar), `screens.rs`, `splash.rs`, `motion.rs`, `theme.rs`, `widgets.rs`, `shape.rs`, `draw.rs`, `mark.rs` |
| `src/shortcuts.rs`, `src/terminal.rs`, `src/textedit.rs`, `src/sys.rs` | shortcuts, Windows Terminal and PowerShell integration, text edits of `settings.json` and profiles, calls of the Windows API |

## Appearance

The window is 960 x 600 points with its own caption bar (Minimize and Close) and a rail on the left that lists the steps: passed steps are silver, the step in progress is gold, steps ahead are quiet. Buttons, cards and fields are cut at the top right and at the bottom left at the slope 3 : 4, the turn of the mark. The chevron is the only marker of a choice, a list item and progress. Colors, type sizes, spacing and motion times come from the production identity 1.0.0 of the Danaco Nurt brand pack and from the design of the wizard, `design/installer/installer.md` of that pack; no value outside them is used. The faces are Geist and Geist Mono, embedded in the executable (SIL Open Font License 1.1, text in `assets/fonts/OFL.txt`); nothing is fetched from a network.

Every start of the wizard plays a splash of 6.2 seconds in the same window: the mark appears, a terminal types `nurt> setup <version>` and `nurt> undo`, and the mark moves into the lockup. A key or a click after 0.4 seconds skips it to the finished lockup. `--no-animation` opens the wizard without the splash. When Windows has animations turned off, the splash is not played and transitions between screens are cut.

Screens, the log of the installation and the dialogs are operated by keyboard (Tab, arrows, Enter, Esc, Alt+N, Alt+B, Alt+C) and are exposed to UI Automation through AccessKit.

## Steps of the wizard

| Step | Content |
|---|---|
| Welcome | The requirements: Windows build, architecture, free space, presence of the embedded archive. A failed requirement blocks the step and states that nothing was changed. |
| Licence | The full text of `LICENSE` and `NOTICE` from the archive. The button Next is enabled after the text is scrolled to the end; there is no checkbox. |
| Location | Scope ("Only for me": `%LOCALAPPDATA%\Programs\Nurt\bin`; "For all users of this PC": `%ProgramFiles%\Nurt\bin`) and the folder of `nurt.exe`, with a folder picker. The folder is checked for a full path, for writability and for free space; for all users it must lie inside `%ProgramFiles%`. |
| Options | Integration: add to `PATH`, documentation, Start menu shortcuts, desktop shortcut, Quick Launch shortcut, Windows Terminal profile (optionally the default profile), tab completion in PowerShell. First configuration: language, color and syntax variant of the written `nurt.toml`. |
| Review | The list of journal entries that the installation will write, in order. |
| Install | Six stages with progress: verify the SHA-256 sum of the embedded archive, extract, copy files, register and update `PATH`, create shortcuts and profiles, write the journal. Details are shown on request as a log in the form of a `nurt` session. An error or Cancel undoes every change made so far. |
| Finish | Two checkboxes: open a terminal and start nurt; show the documentation (available when the documentation was installed). Close applies them. |

When an installation made by this setup is found (through its entry in "Installed apps" and its journal), the wizard opens the maintenance flow instead: Repair (Update when the version differs) or Undo, then Done.

The root folder of an installation is the parent of the folder `bin` of `nurt.exe`; when the chosen folder is not named `bin`, the folder itself is the root, the documentation is not installed there, and only `LICENSE` and `NOTICE` are copied next to the executable.

## Switches

Both `--name value` and `--name=value` are accepted. Switch names are not case sensitive. The text is the `USAGE` constant of `src/core/cli.rs`.

| Switch | Effect |
|---|---|
| `/S`, `--silent` | No window and no splash; the exit code reports the result. Output goes to the console of the parent process and to the log. |
| `--scope user\|machine` | For the current user (default) or for all users. `machine` asks for administrator approval. |
| `--dir <path>`, `/D=<path>` | Folder of `nurt.exe`. Environment variables are expanded. |
| `--no-path` | Leave `PATH` unchanged. |
| `--no-docs` | Do not install the documentation. |
| `--no-start-menu` | No Start menu shortcuts. |
| `--desktop` | Shortcut on the desktop. |
| `--quick-launch` | Shortcut in the Quick Launch folder. |
| `--terminal-profile` | Add the Windows Terminal profile. |
| `--terminal-default` | Make the profile the default profile of Windows Terminal (implies `--terminal-profile`). |
| `--completions` | Tab completion in PowerShell. |
| `--no-config` | Do not write `nurt.toml`. |
| `--version <X.Y.Z>` | Version that the caller expects. The setup carries exactly one version; another value ends with exit code 2. |
| `--no-animation` | Open the wizard without the splash. |
| `--repair` | Install the same version again over an installation, with the choices recorded in the journal. |
| `--undo` | Reverse the journal of the setup. `uninstall.exe` does this without the switch. |
| `--remove-config` | Remove `nurt.toml` of the user too when undoing. |
| `--log <file>` | Log file; by default `nurt-setup.log` in the temporary folder. |
| `-h`, `--help` | The usage text. |

Defaults of the silent mode: scope `user`, path added, documentation installed, Start menu shortcuts created, no desktop, Quick Launch, Windows Terminal or completion integration, and `nurt.toml` written when it does not exist.

The switches `--elevated`, `--resume <file>`, `--worker <file>` and `--from-temp <dir>` are internal: the wizard uses them to hand work to the elevated copy and to run the uninstaller from the temporary folder.

| Exit code | Meaning |
|---|---|
| 0 | Done. |
| 1 | Failed; every change was rolled back. Also: no installation to undo, an unreadable journal, or an undo that ended with warnings. |
| 2 | Invalid arguments, an invalid folder, or a version that this setup does not carry. |
| 3 | Requirements not met. |
| 4 | Administrator approval declined. |

Examples:

```text
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --scope machine --terminal-profile --completions
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --dir=D:\Tools\Nurt\bin --no-path --version 0.1.2
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --repair
nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe /S --undo --remove-config
```

## What an installation writes

For the default folder of the scope `user` (`<root>` is `%LOCALAPPDATA%\Programs\Nurt`):

| Place | Content |
|---|---|
| `<root>\bin\nurt.exe` | the program |
| `<root>\doc\` | the documentation site, completion scripts, `README.md`, `CHANGELOG.md`, `LICENSE`, `NOTICE` |
| `<root>\uninstall.exe` | a copy of the setup; started under this name it undoes the installation |
| `<root>\nurt.ico` | the icon of the shortcuts and of the entry in "Installed apps" |
| `<root>\setup-journal.jsonl` | the journal of the setup |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\DanacoNurt` | the entry in "Installed apps": name, publisher, version, icon, size, date, `UninstallString` (`"<root>\uninstall.exe" --undo`) and `QuietUninstallString` (`... --undo /S`); `NoModify` and `NoRepair` are set |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\nurt.exe` | registration for the Run dialog |
| `HKCU\Environment`, value `Path` | the folder of `nurt.exe` is added (option) |
| `<Start menu>\Programs\Danaco Nurt\` | the shortcuts "Danaco Nurt" and "Uninstall Danaco Nurt" (option) |
| desktop, Quick Launch | the shortcut "Danaco Nurt" (options) |
| `%LOCALAPPDATA%\Microsoft\Windows Terminal\Fragments\Danaco Nurt\nurt.json` | the profile "Danaco Nurt" (option) |
| `settings.json` of Windows Terminal | the default profile, when chosen; the previous value is kept in the journal |
| PowerShell profiles | a block between `# >>> Danaco Nurt completion >>>` and `# <<< Danaco Nurt completion <<<` (option) |
| `%APPDATA%\Nurt\nurt.toml` | the first configuration, written only when the file does not exist |

For the scope `machine` the root is `%ProgramFiles%\Nurt`, the registry entries are written below `HKLM`, `PATH` is the machine `PATH`, and the Start menu, the desktop and the Windows Terminal fragments are the shared locations (`%ProgramData%`). `nurt.toml` stays in the profile of the user who ran the setup.

### Administrator rights

The wizard itself runs with the rights of the user. For the scope `machine` it starts a second copy of itself with the elevation prompt of Windows (UAC) and hands it the job (`--worker`); the window keeps following the events of that copy. In silent mode the process relaunches itself elevated and waits for its exit code. A declined prompt ends with exit code 4. Programs that the wizard starts after the installation (Windows Terminal, the documentation) run with the rights of the user, not elevated.

## Journal

The journal is the file `setup-journal.jsonl` in the root of the installation. It belongs to the setup and is not the journal of `nurt`. The format is JSON Lines, UTF-8, one object per line, identified by `nurt-setup-journal/1` in the first entry. Repair and Undo read it; nothing else depends on it.

Every line has the fields `n` (number of the entry, 0 for `begin`), `t` (UTC time, ISO 8601) and `op` (kind), followed by the fields of the kind. A change is written to the journal before the next change is made, so a run that stops in the middle leaves a journal that undoes it.

| `op` | Fields | Meaning | Undo |
|---|---|---|---|
| `begin` | `format`, `version`, `target`, `bin`, `root`, `options` | Start of a run. `options` holds every choice of the user: `scope`, `dir`, `path`, `docs`, `start_menu`, `terminal_profile`, `terminal_default`, `desktop`, `quick_launch`, `completions`, `write_config` and `config` (`language`, `color`, `syntax`). | none |
| `create_dirs` | `paths` | Folders that the run created, parents first. | removes the folders that are empty, last first |
| `write_files` | `dir`, `files` | Files written below `dir`. Each file has `path` (relative to `dir`), `sha256`, `size` and `replaced` (a file of an earlier installation was replaced). | removes the files; a rollback puts a replaced file back from its `.setup-bak` copy |
| `registry` | `scope`, `keys` | Keys written (`Uninstall\DanacoNurt`, `App Paths\nurt.exe`). Each key has `key` and `previous` (`existed` and `values`, each value with `name`, `kind` (registry type number) and `data` (hexadecimal bytes)). | rollback restores `previous`; Undo deletes the keys |
| `set_path` | `scope`, `entry`, `previous` | The folder `entry` was added to `PATH`; `previous` is the value before, or `null`. | removes the entry, or restores `previous` on rollback |
| `shortcuts` | `paths`, `dir_created` | `.lnk` files that the run created; `dir_created` is a Start menu folder that did not exist. | removes the files and the folder |
| `terminal_fragment` | `path`, `dir_created` | The fragment of Windows Terminal with the profile. | removes the file and a folder that the run created |
| `terminal_default` | `files` | The profile became the default profile. Each file has `path` of `settings.json` and `previous` (the default profile before, or `null`). | puts the previous default profile back |
| `powershell_profile` | `files` | The completion block was added to PowerShell profiles. Each file has `path` and `existed`. | removes the block; deletes a profile that the run created and that holds nothing else |
| `config_file` | `path`, `dir_created` | `nurt.toml` was written. | removes the file when `--remove-config` is given or when the run is rolled back |
| `end` | `entries`, `result` | End of a run: the number of entries before it and `complete`. | none |

Example (paths shortened by line breaks only in this document; each entry is one line in the file):

```text
{"n":0,"t":"2026-11-28T09:14:02Z","op":"begin","format":"nurt-setup-journal/1","version":"0.1.2","target":"x86_64-pc-windows-msvc","bin":"C:\\Users\\user\\AppData\\Local\\Programs\\Nurt\\bin","root":"C:\\Users\\user\\AppData\\Local\\Programs\\Nurt","options":{"scope":"user","dir":"","path":true,"docs":true,"start_menu":true,"terminal_profile":false,"terminal_default":false,"desktop":false,"quick_launch":false,"completions":false,"write_config":true,"config":{"language":"en","color":"auto","syntax":"a"}}}
{"n":1,"t":"2026-11-28T09:14:02Z","op":"create_dirs","paths":["C:\\Users\\user\\AppData\\Local\\Programs\\Nurt","C:\\Users\\user\\AppData\\Local\\Programs\\Nurt\\bin"]}
{"n":2,"t":"2026-11-28T09:14:03Z","op":"write_files","dir":"C:\\Users\\user\\AppData\\Local\\Programs\\Nurt\\bin","files":[{"path":"nurt.exe","sha256":"<64 hexadecimal digits>","size":7898112,"replaced":false}]}
{"n":3,"t":"2026-11-28T09:14:04Z","op":"registry","scope":"user","keys":[{"key":"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\DanacoNurt","previous":{"existed":false,"values":[]}},{"key":"Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\nurt.exe","previous":{"existed":false,"values":[]}}]}
{"n":4,"t":"2026-11-28T09:14:04Z","op":"set_path","scope":"user","entry":"C:\\Users\\user\\AppData\\Local\\Programs\\Nurt\\bin","previous":"C:\\Windows\\System32"}
{"n":5,"t":"2026-11-28T09:14:05Z","op":"end","entries":5,"result":"complete"}
```

### Rules

- **The file `.new`.** A run writes `setup-journal.jsonl.new` in the root. Only a run that ends with `end` replaces `setup-journal.jsonl` with it (rename). A failed or cancelled run is rolled back from its own entries and leaves the journal of an earlier installation untouched.
- **Repair.** The wizard (Maintenance) and `--repair` install the same version again. The choices come from the `options` of the `begin` entry, so the same integration is restored; switches given on the command line are applied on top of them. Every file listed in `write_files` is compared with its recorded `sha256`; the silent mode prints each file that differs or is missing (`repair: <path> differs from the journal`). The run then writes every file again from the embedded archive, which is verified against its sum before anything is extracted. Changes that the run finds already in place (the `PATH` entry, the default profile of Windows Terminal, the PowerShell profile blocks) are not repeated; their entries are carried over from the earlier journal into the new one, directly before `end`, so that Undo still reverses them. When the version in the setup differs from the installed one, the same flow is called Update. A file that is replaced keeps a `.setup-bak` copy until the run is complete; a failed run puts it back.
- **Undo.** The entries are read from the last to the first and each is reversed by its kind, so the state before the installation returns: files and folders removed, registry keys deleted, `PATH` restored, shortcuts, the Windows Terminal fragment and default profile and the PowerShell block removed. `nurt.toml` stays unless `--remove-config` is given. A step that fails becomes a warning and the remaining entries are still reversed. At the end the journal and `uninstall.exe` are deleted (the copy that runs from the root first moves itself to the temporary folder and is deleted after its exit) and the root folder is removed when it is empty.
- **Unreadable journal.** A line that is not valid JSON, an empty file or a first entry that is not `begin` of the format `nurt-setup-journal/1` makes the journal unreadable; Undo ends with the error `journal-unreadable` and changes nothing.

The setup also writes a plain text log (`nurt-setup.log` in the temporary folder, or the file of `--log`). It is meant for diagnosis and is not read by the setup.

## Build

The wizard is cross-built on Linux with `cargo-xwin`, like the other Windows targets of the release. `tools/release/build-release.sh` does it after it has packed the archive of a Windows target (see `tools/release/README.md`):

```text
NURT_SETUP_PAYLOAD=<output-dir>/nurt-<version>-<target>.zip \
NURT_SETUP_PAYLOAD_SHA256=<sum from sha256sum> \
cargo xwin build --release --locked \
  --manifest-path packaging/windows-installer/Cargo.toml --target <target>
```

| Variable | Meaning |
|---|---|
| `NURT_SETUP_PAYLOAD` | Path of the program archive that `build.rs` embeds. Without it an empty archive is embedded: the wizard builds and starts, and the requirements step reports the missing archive. |
| `NURT_SETUP_PAYLOAD_SHA256` | The sum of that archive from `checksums.txt`. `build.rs` computes the sum of the file and stops the build when the two differ, so that the archive in the release and the archive in the wizard are the same file. The sum is written next to the archive and is checked again at run time by the stage `verify`. |
| `NURT_RC` | The resource compiler. When unset, `build.rs` uses the first of `llvm-rc` and `rc` that it finds in `PATH`. |

The resources (`setup.rc.in`: icon `branding/icons/windows/nurt.ico`, version information filled from the version of the package, and `setup.manifest`) are compiled with `llvm-rc` and linked into the executable. The version of the package in `Cargo.toml` must equal the version of the release.

The fonts Geist and Geist Mono are embedded as subsets. Both are licensed under the SIL Open Font License 1.1; the license text is `assets/fonts/OFL.txt`. The design (`design/installer/installer.md`), the identity pack with the tokens and the marks, and the prototype of the splash are part of the Danaco Nurt brand pack; the repository carries the derived values in `src/ui/theme.rs` and the marks in `src/ui/mark.rs`.

Checks of the code (the unit tests of the engine need Windows; the interface and the engine compile for the Windows targets only):

```text
cargo fmt --manifest-path packaging/windows-installer/Cargo.toml --check
cargo xwin clippy --manifest-path packaging/windows-installer/Cargo.toml --target x86_64-pc-windows-msvc -- -D warnings
```

## Signing and verification

`tools/release/build-release.sh <version> <output-dir> --codesign <p12-file> <password-file>` signs with Authenticode through `osslsigncode`: SHA-256 digest, the name "Danaco Nurt", the address of the project, and an RFC 3161 time stamp when a time stamp server answers. `nurt.exe` is signed before it is packed, so the archive that the wizard embeds contains a signed program; the wizard is signed after it is built. No file of the output directory changes after `checksums.txt` is computed. The public certificate is `packaging/release/danaco-nurt-codesign.pem`. The certificate is self-signed: Windows reports a publisher that is not in its trusted list until the certificate is imported into the store "Trusted Publishers" and "Trusted Root Certification Authorities" of the machine.

Verification on Windows:

```text
Get-AuthenticodeSignature .\nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe | Format-List Status, StatusMessage, SignerCertificate, TimeStamperCertificate
```

`Status` is `Valid` when the certificate is trusted and `UnknownError` or `NotTrusted` with a self-signed certificate that is not imported; the signer subject is `CN=Danaco Nurt`. Verification on Linux:

```text
osslsigncode verify -CAfile packaging/release/danaco-nurt-codesign.pem -in nurt-0.1.2-x86_64-pc-windows-msvc-setup.exe
```

The command exits with 0 when the signature, the digest and the chain to the given certificate are valid. The SHA-256 sums of the setup files are in `checksums.txt` of the release, which is signed with GnuPG (`checksums.txt.asc`).

## Tests

- `packaging/tests/test_setup_release.py`: the arguments and the modes of `build-release.sh` that concern the wizard (`--no-setup`, `--no-windows`, `--codesign`), the check of the version, the pattern of `checksums.txt` and the order of signing, packing and building. Run like the CI: `python3 -I -m unittest discover -s packaging/tests -p 'test_setup_*.py'`.
- Unit tests in the sources of the wizard (`journal.rs`, `cli.rs`, `engine.rs`, `textedit.rs` and others): they run on Windows with `cargo test --manifest-path packaging/windows-installer/Cargo.toml`.
- Manual run on Windows 11 (x86_64 and aarch64): every step of the wizard, the scopes "for me" and "for all users" (with UAC), `nurt --version` in a new terminal, `PATH`, shortcuts, the Windows Terminal profile, the entry in "Installed apps", Undo, the silent mode, a second installation over the first, and the signature.

## Limits

- **Taskbar.** Windows gives programs no supported way to pin a shortcut to the taskbar; the wizard creates the Start menu and desktop shortcuts, and the user pins them from there.
- **Elevation.** Administrator approval is needed for the scope "for all users" only; the prompt appears when the installation starts, not when the wizard opens.
- **No WebView2.** The window does not use a web view, so the Microsoft Edge WebView2 runtime is not required.
- **Graphics.** The window is drawn with Direct3D 12 (wgpu). A computer without a Direct3D 12 adapter uses the software rasterizer WARP of Windows; if neither can start, the wizard cannot open a window and the silent mode `/S` remains available.
- **Language.** The interface is English only.
- **Trust.** The signature is made with a self-signed certificate; Windows SmartScreen can warn when the file is downloaded and run.
- **Scope of the journal.** The journal reverses what the setup wrote. Files that the user created in the installation folder, and `nurt.toml` unless `--remove-config` is given, are not removed; a folder that is not empty after the undo stays.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
