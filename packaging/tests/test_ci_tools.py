# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of tools/ci/check-commits.sh and tools/ci/repo-hygiene.sh."""

import os
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

    def test_apt_upload_keeps_the_packages_of_older_releases(self):
        text = (support.PACKAGING / "ci" / "release.yml").read_text(encoding="utf-8")
        self.assertIn('--delete-after --filter="P /pool/"', text)

    def test_release_checks_use_the_first_line_of_nurt_version(self):
        workflows = support.PACKAGING.parent / ".github" / "workflows"
        for name in ("packaging.yml", "release.yml"):
            self.assertNotIn("Danaco Nurt", (workflows / name).read_text(encoding="utf-8"), name)


if __name__ == "__main__":
    unittest.main()
