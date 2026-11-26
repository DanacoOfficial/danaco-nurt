# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of tools/ci/check-commits.sh and tools/ci/repo-hygiene.sh."""

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

TOOLS = support.PACKAGING.parent / "tools" / "ci"
CHECK_COMMITS = TOOLS / "check-commits.sh"
REPO_HYGIENE = TOOLS / "repo-hygiene.sh"
AUTHOR = "tester@example.com"


@unittest.skipIf(shutil.which("bash") is None, "bash is not installed")
class RepoHygieneArgumentsTest(unittest.TestCase):
    def run_script(self, *arguments):
        return subprocess.run(
            ["bash", str(REPO_HYGIENE), *arguments],
            capture_output=True,
            text=True,
            env=support.clean_environment(),
            check=False,
        )

    def test_rejects_a_selection_without_any_check(self):
        result = self.run_script("--only", ",")
        self.assertEqual(result.returncode, 2)
        self.assertIn("no checks selected", result.stderr)

    def test_rejects_skipping_every_check(self):
        result = self.run_script("--skip", "content,style,headers")
        self.assertEqual(result.returncode, 2)
        self.assertIn("no checks selected", result.stderr)

    def test_rejects_an_unknown_check(self):
        result = self.run_script("--only", "no-such-check")
        self.assertEqual(result.returncode, 2)

    def test_rejects_an_additional_pattern_that_is_not_a_regular_expression(self):
        result = subprocess.run(
            ["bash", str(REPO_HYGIENE), "--only", "external-pattern"],
            capture_output=True,
            text=True,
            env=support.clean_environment(NURT_HYGIENE_PATTERNS="secret(unclosed"),
            check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("not a valid extended regular expression", result.stderr)
        self.assertNotIn("secret", result.stderr)

    def test_lists_the_checks(self):
        result = self.run_script("--list")
        self.assertEqual(result.returncode, 0)
        self.assertIn("producer-header", result.stdout)


@unittest.skipIf(shutil.which("bash") is None or shutil.which("git") is None, "bash or git is not installed")
class CheckCommitsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repository = Path(self.temp.name) / "repository"
        self.repository.mkdir()
        self.environment = support.clean_environment(
            HOME=self.temp.name,
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_AUTHOR_NAME="Tester",
            GIT_AUTHOR_EMAIL=AUTHOR,
            GIT_COMMITTER_NAME="Tester",
            GIT_COMMITTER_EMAIL=AUTHOR,
        )
        self.git("init", "-q")
        self.commit("chore: initial commit")

    def git(self, *arguments):
        return subprocess.run(
            ["git", *arguments],
            cwd=self.repository,
            capture_output=True,
            text=True,
            env=self.environment,
            check=True,
        )

    def commit(self, subject, signed=True):
        message = subject
        if signed:
            message += f"\n\nSigned-off-by: Tester <{AUTHOR}>"
        self.git("commit", "-q", "--allow-empty", "-m", message)

    def check(self, *arguments):
        return subprocess.run(
            ["bash", str(CHECK_COMMITS), *arguments],
            cwd=self.repository,
            capture_output=True,
            text=True,
            env=self.environment,
            check=False,
        )

    def test_requires_a_revision_range(self):
        result = self.check()
        self.assertEqual(result.returncode, 2)
        self.assertIn("Usage", result.stderr)

    def test_accepts_a_signed_conventional_commit(self):
        self.commit("fix(docs): correct the example")
        result = self.check("HEAD~1..HEAD")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_accepts_a_one_character_summary(self):
        self.commit("fix: a")
        result = self.check("HEAD~1..HEAD")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_a_summary_ending_with_a_period(self):
        self.commit("fix: correct the example.")
        result = self.check("HEAD~1..HEAD")
        self.assertEqual(result.returncode, 1)
        self.assertIn("does not follow", result.stderr)

    def test_rejects_a_summary_consisting_of_a_period(self):
        self.commit("fix: .")
        result = self.check("HEAD~1..HEAD")
        self.assertEqual(result.returncode, 1)

    def test_rejects_a_commit_without_sign_off(self):
        self.commit("fix: correct the example", signed=False)
        result = self.check("HEAD~1..HEAD")
        self.assertEqual(result.returncode, 1)
        self.assertIn("Signed-off-by", result.stderr)

    def test_rejects_an_unknown_scope(self):
        self.commit("fix(nonexistent): correct the example")
        result = self.check("HEAD~1..HEAD")
        self.assertEqual(result.returncode, 1)
        self.assertIn("unknown scope", result.stderr)

    def test_rejects_a_subject_longer_than_72_characters(self):
        self.commit("fix: " + "x" * 80)
        result = self.check("HEAD~1..HEAD")
        self.assertEqual(result.returncode, 1)
        self.assertIn("longer than 72", result.stderr)


class WorkflowCopiesTest(unittest.TestCase):
    def test_installed_workflows_match_their_sources_in_the_packaging_directory(self):
        workflows = support.PACKAGING.parent / ".github" / "workflows"
        for name in ("packaging.yml", "release.yml"):
            source = support.PACKAGING / "ci" / name
            installed = workflows / name
            self.assertEqual(
                source.read_text(encoding="utf-8"), installed.read_text(encoding="utf-8"), name
            )

    def test_release_jobs_that_commit_to_a_repository_can_be_run_again(self):
        lines = (support.PACKAGING / "ci" / "release.yml").read_text(encoding="utf-8").splitlines()
        commits = [number for number, line in enumerate(lines) if line.strip().startswith("git commit")]
        self.assertEqual(len(commits), 2)
        for number in commits:
            self.assertIn("git diff --cached --quiet", lines[number - 1])

    def test_binding_workflows_name_the_library_the_ffi_crate_builds(self):
        # The crate nurt-ffi has no [lib] name, so cargo builds libnurt_ffi.
        manifest = (support.PACKAGING.parent / "crates" / "nurt-ffi" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertNotIn("[lib]\nname", manifest)
        for language in ("go", "dotnet", "java"):
            text = (support.PACKAGING.parent / "bindings" / language / "ci" / "workflow.yml").read_text(
                encoding="utf-8"
            )
            self.assertNotIn("target/release/libnurt.so", text, language)
            self.assertNotRegex(text, r"target/release -lnurt\s*$", language)

    def test_binding_workflows_fail_for_an_archive_without_a_checksum(self):
        for language in ("dotnet", "java", "python"):
            text = (support.PACKAGING.parent / "bindings" / language / "ci" / "workflow.yml").read_text(
                encoding="utf-8"
            )
            self.assertIn("is not listed in checksums.txt", text, language)
            self.assertIn("--ignore-missing --strict checksums.txt", text, language)

    def test_rpm_build_takes_its_times_from_source_date_epoch(self):
        text = (support.PACKAGING / "ci" / "packaging.yml").read_text(encoding="utf-8")
        self.assertIn('--define "use_source_date_epoch_as_buildtime 1"', text)
        self.assertIn('--define "clamp_mtime_to_source_date_epoch 1"', text)

    def test_only_the_job_that_uploads_to_the_release_can_write_to_the_repository(self):
        text = (support.PACKAGING / "ci" / "release.yml").read_text(encoding="utf-8")
        self.assertEqual(text.count("contents: write"), 1)
        self.assertLess(text.index("contents: read"), text.index("jobs:"))
        self.assertGreater(text.index("contents: write"), text.index("  prepare:"))

    def test_apt_upload_keeps_the_packages_of_older_releases(self):
        text = (support.PACKAGING / "ci" / "release.yml").read_text(encoding="utf-8")
        self.assertIn('--delete-after --filter="P /pool/"', text)

    def test_release_checks_use_the_first_line_of_nurt_version(self):
        workflows = support.PACKAGING.parent / ".github" / "workflows"
        for name in ("packaging.yml", "release.yml"):
            self.assertNotIn("Danaco Nurt", (workflows / name).read_text(encoding="utf-8"), name)


class DependencyStatementTest(unittest.TestCase):
    def test_readme_names_the_libraries_that_the_glibc_build_links(self):
        readme = (support.PACKAGING.parent / "README.md").read_text(encoding="utf-8")
        self.assertNotIn("without runtime dependencies", readme)
        build = (support.PACKAGING / "deb" / "build-deb.sh").read_text(encoding="utf-8")
        for library in ("libc.so.6", "libgcc_s.so.1", "liblzma.so.5"):
            self.assertIn(library, readme)
        for package in ("libgcc-s1", "liblzma5"):
            self.assertIn(package, build)


class WorkflowSupplyChainTest(unittest.TestCase):
    ROOT = support.PACKAGING.parent

    def workflow_files(self):
        patterns = (
            ".github/workflows/*.yml",
            "integrations/*/ci/*.yml",
            "bindings/*/ci/*.yml",
            "packaging/ci/*.yml",
            "bench/oracles/ci/*.yml",
        )
        files = sorted(path for pattern in patterns for path in self.ROOT.glob(pattern))
        self.assertGreaterEqual(len(files), 25)
        return files

    def test_every_action_is_pinned_to_a_commit_with_its_tag_in_a_comment(self):
        reference = re.compile(r"^\s*(?:-\s+)?uses:\s+(\S+)(.*)$")
        for path in self.workflow_files():
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
                match = reference.match(line)
                if match is None or match.group(1).startswith(("./", "docker://")):
                    continue
                where = f"{path.relative_to(self.ROOT)}:{number}"
                self.assertRegex(match.group(1), r"^[\w./-]+@[0-9a-f]{40}$", where)
                self.assertRegex(match.group(2), r"^\s+# \S+$", where)

    def test_the_toolchain_action_is_told_which_toolchain_to_install(self):
        # Pinned to a commit, the action no longer takes the toolchain from the name of the branch.
        for path in self.workflow_files():
            lines = path.read_text(encoding="utf-8").splitlines()
            for number, line in enumerate(lines):
                if "dtolnay/rust-toolchain@" in line:
                    self.assertTrue(
                        any("toolchain:" in following for following in lines[number + 1 : number + 4]),
                        f"{path.relative_to(self.ROOT)}:{number + 1}",
                    )

    def test_global_npm_tools_are_installed_with_an_exact_version(self):
        found = 0
        for path in self.workflow_files():
            for line in path.read_text(encoding="utf-8").splitlines():
                if "npm install --global" not in line:
                    continue
                packages = [word for word in line.split() if re.match(r"^@?[a-z][\w./-]*(@[^\s]*)?$", word)]
                packages = [word for word in packages if "/" in word or word.count("@") or word == "ovsx"]
                for package in packages:
                    found += 1
                    self.assertRegex(package, r"@\d+\.\d+\.\d+$", f"{path.relative_to(self.ROOT)}: {line.strip()}")
        self.assertGreaterEqual(found, 3)

    def test_the_publishing_job_of_the_vscode_extension_does_not_resolve_a_dependency_tree(self):
        text = (self.ROOT / "integrations" / "vscode" / "ci" / "vscode.yml").read_text(encoding="utf-8")
        publish = text[text.index("  publish:") :]
        self.assertNotIn("npm install --no-audit --no-fund\n", publish)
        self.assertIn("npm install --global --no-audit --no-fund --ignore-scripts @vscode/vsce@", publish)

    def test_workspace_wide_test_runs_fail_when_no_test_runs(self):
        text = (self.ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        for line in text.splitlines():
            if "--workspace" in line and "nextest" in line:
                self.assertNotIn("--no-tests=warn", line)
                self.assertIn("--no-tests=fail", line)

    def test_base_images_of_the_container_scenarios_are_named_by_digest(self):
        text = (self.ROOT / "integrations" / "devcontainer" / "test" / "nurt" / "scenarios.json").read_text(
            encoding="utf-8"
        )
        for image in re.findall(r'"image":\s*"([^"]+)"', text):
            if image.startswith("alpine"):
                self.assertRegex(image, r"@sha256:[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
