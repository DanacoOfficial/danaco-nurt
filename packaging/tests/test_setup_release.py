# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/tests/test_setup_release.py
# Responsibility:   Tests of the setup wizard step of tools/release/build-release.sh: arguments, modes, version check, signing order and checksums.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Tests of the Windows setup wizard step of tools/release/build-release.sh.

The argument tests run the script as it is. The pipeline tests put stand-ins
for cargo, rustc, rustup, curl and osslsigncode first in PATH: the stand-ins
write small files instead of building programs, so the real script, the real
pack.py and the real sha256sum run in a few seconds, and the log of the
stand-ins shows what the script passed to them.
"""

import hashlib
import os
import re
import shutil
import stat
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

import support

ROOT = support.PACKAGING.parent
SCRIPT = ROOT / "tools" / "release" / "build-release.sh"
SETUP_MANIFEST = ROOT / "packaging" / "windows-installer" / "Cargo.toml"
WINDOWS_TARGETS = ("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc")
PASSWORD = "stand-in-password-4c1f"

CARGO = r"""#!/bin/bash
echo "cargo $*" >> "$STUB_LOG"
target_dir="${CARGO_TARGET_DIR:?}"
words=" $* "
target=""
previous=""
for word in "$@"; do
  if [ "$previous" = "--target" ]; then target="$word"; fi
  previous="$word"
done
case "$words" in
  *" nurt-docgen "*)
    mkdir -p "$target_dir/release"
    cp "$STUB_DIR/nurt-docgen" "$target_dir/release/nurt-docgen"
    exit 0 ;;
esac
if [ -n "${STUB_CARGO_FAIL:-}" ]; then exit 1; fi
mkdir -p "$target_dir/$target/release"
if [ "$1" = xwin ]; then
  case "$words" in
    *" --manifest-path "*)
      echo "setup target=$target payload=$NURT_SETUP_PAYLOAD sha=$NURT_SETUP_PAYLOAD_SHA256 path0=${PATH%%:*} dir=$CARGO_TARGET_DIR" >> "$STUB_LOG"
      actual="$(sha256sum "$NURT_SETUP_PAYLOAD" | cut -d ' ' -f 1)"
      if [ "$actual" != "$NURT_SETUP_PAYLOAD_SHA256" ]; then echo "payload sum differs" >&2; exit 1; fi
      printf 'setup-wizard %s\n' "$target" > "$target_dir/$target/release/nurt-setup.exe"
      exit 0 ;;
  esac
  printf 'program-exe %s\n' "$target" > "$target_dir/$target/release/nurt.exe"
  printf 'library-dll' > "$target_dir/$target/release/nurt_ffi.dll"
  exit 0
fi
printf 'program-elf' > "$target_dir/$target/release/nurt"
printf 'library-so' > "$target_dir/$target/release/libnurt_ffi.so"
exit 0
"""

RUSTC = """#!/bin/bash
echo "host: riscv64gc-unknown-linux-gnu"
"""

RUSTUP = """#!/bin/bash
exit 0
"""

DOCGEN = r"""#!/bin/bash
out=""
previous=""
for word in "$@"; do
  if [ "$previous" = "--out" ]; then out="$word"; fi
  previous="$word"
done
mkdir -p "$out"
case "$1" in
  completions) printf 'completion' > "$out/_nurt.ps1" ;;
  site) printf 'page' > "$out/index.html" ;;
esac
"""

CURL = """#!/bin/bash
echo "curl $*" >> "$STUB_LOG"
if [ -n "${STUB_CURL_FAIL:-}" ]; then exit 7; fi
exit 0
"""

OSSLSIGNCODE = r"""#!/bin/bash
echo "osslsigncode $*" >> "$STUB_LOG"
command="$1"
shift
input=""
output=""
while [ $# -gt 0 ]; do
  case "$1" in
    -in) input="$2"; shift 2 ;;
    -out) output="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$command" in
  sign)
    if [ -n "${STUB_SIGN_FAIL:-}" ]; then echo "cannot sign" >&2; exit 1; fi
    cp "$input" "$output"
    if [ -z "${STUB_SIGN_NOOP:-}" ]; then echo "AUTHENTICODE-SIGNATURE" >> "$output"; fi
    exit 0 ;;
  verify)
    if ! grep -q '^AUTHENTICODE-SIGNATURE$' "$input"; then echo "No signature found"; exit 1; fi
    if [ -n "${STUB_UNTRUSTED:-}" ]; then echo "Signature verification: failed"; echo "Verify error: self-signed certificate"; exit 1; fi
    echo "Signature verification: ok"
    exit 0 ;;
esac
exit 2
"""


def setup_version():
    """Return the version of the package nurt-setup, read from its Cargo.toml."""
    text = SETUP_MANIFEST.read_text(encoding="utf-8")
    match = re.search(r'^\[package\](?:.*\n)*?version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if match is None:
        raise RuntimeError(f"{SETUP_MANIFEST} has no package version")
    return match.group(1)


def sha256_of(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class ScriptTextTest(unittest.TestCase):
    def setUp(self):
        self.text = SCRIPT.read_text(encoding="utf-8")

    def test_the_script_is_valid_bash(self):
        done = subprocess.run(["bash", "-n", str(SCRIPT)], capture_output=True, text=True, check=False)
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_the_usage_names_the_new_options(self):
        done = subprocess.run(["bash", str(SCRIPT), "--help"], capture_output=True, text=True, check=False)
        self.assertEqual(done.returncode, 2)
        for text in ("--codesign", "--no-setup", "--no-windows", "<p12-file>", "<password-file>"):
            self.assertIn(text, done.stderr)

    def test_the_setup_files_are_listed_in_the_checksums_before_the_signature(self):
        self.assertIn("nurt-*.tar.gz nurt-*.zip nurt-*-setup.exe", self.text)
        self.assertLess(self.text.index("for file in nurt-*.tar.gz"), self.text.index('log "sign checksums.txt"'))

    def test_the_wizard_is_built_with_the_archive_and_its_sum(self):
        for text in (
            "NURT_SETUP_PAYLOAD=",
            "NURT_SETUP_PAYLOAD_SHA256=",
            "--manifest-path",
            "packaging/windows-installer/Cargo.toml",
            "packaging/release/msvc-clang",
            'CARGO_TARGET_DIR="$target_dir"',
            "sha256sum",
        ):
            self.assertIn(text, self.text)

    def test_the_password_is_read_from_the_file_only(self):
        self.assertIn('-readpass "$codesign_pass"', self.text)
        self.assertNotIn("-pass ", self.text)
        self.assertNotIn("set -x", self.text)
        self.assertNotIn("xtrace", self.text)

    def test_the_time_stamp_servers_are_tried_in_order(self):
        servers = ("http://timestamp.digicert.com", "http://time.certum.pl", "http://timestamp.sectigo.com")
        positions = [self.text.index(server) for server in servers]
        self.assertEqual(positions, sorted(positions))

    def test_the_program_is_signed_before_it_is_packed(self):
        sign = self.text.index('sign_file "$work/signed/$target/nurt.exe"')
        pack = self.text.index('python3 -I "$root/packaging/release/pack.py"')
        setup = self.text.index('then build_setup "$target"')
        self.assertLess(sign, pack)
        self.assertLess(pack, setup)


class ArgumentTest(unittest.TestCase):
    def run_script(self, *arguments, env=None):
        return subprocess.run(
            ["bash", str(SCRIPT), *arguments], capture_output=True, text=True, check=False, env=env
        )

    def test_codesign_requires_two_values(self):
        for arguments in (("--codesign",), ("--codesign", "key.p12")):
            with self.subTest(arguments=arguments):
                done = self.run_script("1.2.3", "/nonexistent/out", *arguments)
                self.assertEqual(done.returncode, 2)
                self.assertIn("Usage", done.stderr)

    def test_a_missing_key_file_is_rejected_before_anything_is_built(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / "out"
            done = self.run_script("1.2.3", str(out), "--codesign", str(Path(directory) / "none.p12"), "none.txt")
            self.assertEqual(done.returncode, 1)
            self.assertIn("PKCS#12 file", done.stderr)
            self.assertFalse(out.exists())

    def test_a_missing_password_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            key = Path(directory) / "key.p12"
            key.write_text("x")
            done = self.run_script("1.2.3", str(Path(directory) / "out"), "--codesign", str(key), str(Path(directory) / "none.txt"))
            self.assertEqual(done.returncode, 1)
            self.assertIn("password file", done.stderr)

    def test_a_missing_osslsigncode_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            key = Path(directory) / "key.p12"
            password = Path(directory) / "pass.txt"
            key.write_text("x")
            password.write_text(PASSWORD)
            env = {**os.environ, "NURT_OSSLSIGNCODE": str(Path(directory) / "absent-osslsigncode")}
            done = self.run_script("1.2.3", str(Path(directory) / "out"), "--codesign", str(key), str(password), env=env)
            self.assertEqual(done.returncode, 1)
            self.assertIn("osslsigncode", done.stderr)
            self.assertNotIn(PASSWORD, done.stderr + done.stdout)

    def test_a_version_that_differs_from_the_wizard_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / "out"
            done = self.run_script("9.9.9", str(out))
            self.assertEqual(done.returncode, 1)
            self.assertIn(f"nurt-setup is {setup_version()}, not 9.9.9", done.stderr)
            self.assertFalse(out.exists())

    def test_the_version_of_the_wizard_equals_the_version_of_the_workspace(self):
        workspace = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        match = re.search(r'^\[workspace\.package\](?:.*\n)*?version\s*=\s*"([^"]+)"', workspace, re.MULTILINE)
        self.assertIsNotNone(match)
        self.assertEqual(setup_version(), match.group(1))


class PipelineTest(unittest.TestCase):
    """The script with stand-ins for the programs that build and sign."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)
        self.stubs = self.dir / "stubs"
        self.stubs.mkdir()
        for name, text in (
            ("cargo", CARGO),
            ("rustc", RUSTC),
            ("rustup", RUSTUP),
            ("nurt-docgen", DOCGEN),
            ("curl", CURL),
            ("osslsigncode", OSSLSIGNCODE),
        ):
            path = self.stubs / name
            path.write_text(text, encoding="utf-8")
            path.chmod(path.stat().st_mode | stat.S_IXUSR)
        self.log = self.dir / "stub.log"
        self.log.write_text("")
        self.key = self.dir / "codesign.p12"
        self.key.write_text("not a key")
        self.password = self.dir / "codesign-pass.txt"
        self.password.write_text(PASSWORD + "\n")
        self.out = self.dir / "out"
        self.version = setup_version()
        python = shutil.which("python3")
        self.assertIsNotNone(python)
        self.env = {
            "PATH": os.pathsep.join([str(self.stubs), str(Path(python).parent), "/usr/bin", "/bin"]),
            "HOME": str(self.dir),
            "CARGO_TARGET_DIR": str(self.dir / "target"),
            "SOURCE_DATE_EPOCH": "1700000000",
            "NURT_RELEASE_MUSL": "0",
            "NURT_RELEASE_AARCH64": "0",
            "STUB_LOG": str(self.log),
            "STUB_DIR": str(self.stubs),
        }

    def build(self, *arguments, **extra):
        env = {**self.env, **{key: str(value) for key, value in extra.items()}}
        return subprocess.run(
            ["bash", str(SCRIPT), self.version, str(self.out), *arguments],
            capture_output=True,
            text=True,
            check=False,
            env=env,
            cwd=self.dir,
        )

    def lines(self):
        return self.log.read_text(encoding="utf-8").splitlines()

    def checksums(self):
        sums = {}
        for line in (self.out / "checksums.txt").read_text(encoding="utf-8").splitlines():
            digest, name = line.split(None, 1)
            sums[name.lstrip("*")] = digest
        return sums

    def setup_name(self, target):
        return f"nurt-{self.version}-{target}-setup.exe"

    def archive_name(self, target):
        return f"nurt-{self.version}-{target}.zip"

    def read_program(self, target):
        with zipfile.ZipFile(self.out / self.archive_name(target)) as archive:
            return archive.read("nurt.exe")

    def test_the_wizard_is_built_and_listed_for_each_windows_target(self):
        done = self.build()
        self.assertEqual(done.returncode, 0, done.stderr)
        sums = self.checksums()
        for target in WINDOWS_TARGETS:
            setup = self.out / self.setup_name(target)
            self.assertTrue(setup.is_file(), self.setup_name(target))
            self.assertEqual(setup.read_text().strip(), f"setup-wizard {target}")
            self.assertEqual(sums[self.setup_name(target)], sha256_of(setup))
            self.assertEqual(sums[self.archive_name(target)], sha256_of(self.out / self.archive_name(target)))
        self.assertFalse((self.out / ".work").exists())

    def test_the_wizard_gets_the_archive_and_its_sum(self):
        self.assertEqual(self.build().returncode, 0)
        setups = [line for line in self.lines() if line.startswith("setup ")]
        self.assertEqual(len(setups), len(WINDOWS_TARGETS))
        sums = self.checksums()
        for target, line in zip(WINDOWS_TARGETS, setups):
            fields = dict(item.split("=", 1) for item in line.split()[1:])
            self.assertEqual(fields["target"], target)
            self.assertEqual(fields["payload"], str(self.out / self.archive_name(target)))
            self.assertEqual(fields["sha"], sums[self.archive_name(target)])
            self.assertEqual(fields["path0"], str(ROOT / "packaging" / "release" / "msvc-clang"))
            self.assertEqual(fields["dir"], str(self.dir / "target"))
        commands = [line for line in self.lines() if line.startswith("cargo xwin build") and "--manifest-path" in line]
        for command in commands:
            self.assertIn("--release --locked", command)
            self.assertIn(str(ROOT / "packaging" / "windows-installer" / "Cargo.toml"), command)

    def test_no_setup_builds_the_archives_only(self):
        done = self.build("--no-setup")
        self.assertEqual(done.returncode, 0, done.stderr)
        for target in WINDOWS_TARGETS:
            self.assertTrue((self.out / self.archive_name(target)).is_file())
            self.assertFalse((self.out / self.setup_name(target)).exists())
        self.assertFalse([line for line in self.lines() if "--manifest-path" in line])
        self.assertNotIn("-setup.exe", (self.out / "checksums.txt").read_text())

    def test_no_windows_builds_no_windows_file(self):
        done = self.build("--no-windows")
        self.assertEqual(done.returncode, 0, done.stderr)
        names = sorted(path.name for path in self.out.iterdir())
        self.assertFalse([name for name in names if "windows" in name], names)
        self.assertFalse([line for line in self.lines() if line.startswith("cargo xwin")])

    def test_no_windows_does_not_check_the_version_of_the_wizard(self):
        env = {**self.env, "STUB_CARGO_FAIL": "1"}
        for arguments in (("--no-windows",), ("--no-setup",)):
            with self.subTest(arguments=arguments):
                with tempfile.TemporaryDirectory() as directory:
                    done = subprocess.run(
                        ["bash", str(SCRIPT), "9.9.9", str(Path(directory) / "out"), *arguments],
                        capture_output=True,
                        text=True,
                        check=False,
                        env=env,
                        cwd=self.dir,
                    )
                    self.assertNotIn("nurt-setup is", done.stderr)
                    self.assertEqual(done.returncode, 1)
                    self.assertIn("build ", done.stderr)

    def test_a_failed_wizard_build_fails_the_release(self):
        done = self.build(STUB_CARGO_FAIL="1")
        self.assertEqual(done.returncode, 1)
        self.assertFalse((self.out / "checksums.txt").exists())

    def test_codesign_signs_the_program_before_the_archive_and_the_wizard_after_it(self):
        done = self.build("--codesign", str(self.key), str(self.password))
        self.assertEqual(done.returncode, 0, done.stderr)
        for target in WINDOWS_TARGETS:
            self.assertTrue(self.read_program(target).endswith(b"AUTHENTICODE-SIGNATURE\n"), target)
            setup = (self.out / self.setup_name(target)).read_bytes()
            self.assertTrue(setup.endswith(b"AUTHENTICODE-SIGNATURE\n"), target)
        # The sum of the archive that the wizard embedded is the sum that checksums.txt lists.
        sums = self.checksums()
        setups = [line for line in self.lines() if line.startswith("setup ")]
        for target, line in zip(WINDOWS_TARGETS, setups):
            self.assertIn(f"sha={sums[self.archive_name(target)]} ", line + " ")
            self.assertEqual(sums[self.setup_name(target)], sha256_of(self.out / self.setup_name(target)))
        # Order for every target: sign nurt.exe, build the wizard, sign the wizard.
        log = self.lines()
        for target in WINDOWS_TARGETS:
            program = next(i for i, line in enumerate(log) if line.startswith("osslsigncode sign") and f"signed/{target}/nurt.exe" in line)
            build = next(i for i, line in enumerate(log) if line.startswith(f"setup target={target} "))
            wizard = next(i for i, line in enumerate(log) if line.startswith("osslsigncode sign") and self.setup_name(target) in line)
            self.assertLess(program, build)
            self.assertLess(build, wizard)

    def test_the_signature_command_has_the_documented_arguments(self):
        self.assertEqual(self.build("--codesign", str(self.key), str(self.password)).returncode, 0)
        commands = [line for line in self.lines() if line.startswith("osslsigncode sign")]
        self.assertEqual(len(commands), 2 * len(WINDOWS_TARGETS))
        for command in commands:
            self.assertIn(f"-pkcs12 {self.key}", command)
            self.assertIn(f"-readpass {self.password}", command)
            self.assertIn("-h sha256", command)
            self.assertIn("-n Danaco Nurt", command)
            self.assertIn("-i https://github.com/DanacoOfficial/danaco-nurt", command)
            self.assertIn("-ts http://timestamp.digicert.com", command)
            self.assertNotIn("-pass ", command)

    def test_the_password_is_never_printed_or_passed(self):
        done = self.build("--codesign", str(self.key), str(self.password), STUB_UNTRUSTED="1")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn(PASSWORD, done.stdout + done.stderr)
        self.assertNotIn(PASSWORD, self.log.read_text())

    def test_the_first_time_stamp_server_that_answers_is_used(self):
        self.assertEqual(self.build("--codesign", str(self.key), str(self.password)).returncode, 0)
        probes = [line for line in self.lines() if line.startswith("curl ")]
        self.assertEqual(len(probes), 1)
        self.assertIn("http://timestamp.digicert.com", probes[0])
        self.assertIn("-m 8", probes[0])

    def test_without_a_time_stamp_server_the_files_are_signed_with_a_warning(self):
        done = self.build("--codesign", str(self.key), str(self.password), STUB_CURL_FAIL="1")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("without a time stamp", done.stderr)
        commands = [line for line in self.lines() if line.startswith("osslsigncode sign")]
        self.assertTrue(commands)
        for command in commands:
            self.assertNotIn(" -ts ", command)
        probes = [line for line in self.lines() if line.startswith("curl ")]
        self.assertEqual(len(probes), 3)

    def test_an_untrusted_chain_is_a_warning(self):
        done = self.build("--codesign", str(self.key), str(self.password), STUB_UNTRUSTED="1")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("chain is not trusted", done.stderr)

    def test_a_file_without_a_signature_stops_the_release(self):
        done = self.build("--codesign", str(self.key), str(self.password), STUB_SIGN_NOOP="1")
        self.assertEqual(done.returncode, 1)
        self.assertIn("no Authenticode signature", done.stderr)
        self.assertFalse((self.out / "checksums.txt").exists())

    def test_a_failed_signature_stops_the_release(self):
        done = self.build("--codesign", str(self.key), str(self.password), STUB_SIGN_FAIL="1")
        self.assertEqual(done.returncode, 1)
        self.assertIn("failed to sign", done.stderr)

    def test_codesign_with_no_setup_signs_the_program_only(self):
        done = self.build("--codesign", str(self.key), str(self.password), "--no-setup")
        self.assertEqual(done.returncode, 0, done.stderr)
        for target in WINDOWS_TARGETS:
            self.assertTrue(self.read_program(target).endswith(b"AUTHENTICODE-SIGNATURE\n"))
            self.assertFalse((self.out / self.setup_name(target)).exists())
        self.assertEqual(len([line for line in self.lines() if line.startswith("osslsigncode sign")]), len(WINDOWS_TARGETS))

    def test_without_codesign_nothing_is_signed(self):
        self.assertEqual(self.build().returncode, 0)
        self.assertFalse([line for line in self.lines() if line.startswith(("osslsigncode", "curl"))])
        for target in WINDOWS_TARGETS:
            self.assertNotIn(b"AUTHENTICODE-SIGNATURE", self.read_program(target))


if __name__ == "__main__":
    unittest.main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
