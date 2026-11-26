# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Tests of the scripts that render the identity of the product.

The scripts need rsvg-convert, Blender and fonts, which the tests do not
require: the calls to the renderer are recorded, and the script for Blender,
which imports bpy, is checked as text.

Run:
    python3 -I -m unittest discover -s branding -p 'test_*.py'
"""

import importlib.util
import re
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

BRANDING = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, BRANDING / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class ExportTest(unittest.TestCase):
    def setUp(self):
        sys.path.insert(0, str(BRANDING))
        self.addCleanup(sys.path.remove, str(BRANDING))
        self.export = load("export")

    def commands(self):
        recorded = []

        def run(command, check):
            recorded.append(list(command))
            Path(command[command.index("-o") + 1]).write_bytes(b"png")

        with tempfile.TemporaryDirectory() as temp:
            with mock.patch.object(self.export.subprocess, "run", side_effect=run):
                with mock.patch.object(self.export, "social_card"):
                    self.assertEqual(self.export.main(["--svg", str(BRANDING / "svg"), "--out", temp]), 0)
        return recorded

    def test_touch_icon_is_rendered_on_the_colour_of_its_tile(self):
        touch = [command for command in self.commands() if command[-1].endswith("apple-touch-icon.png")]
        self.assertEqual(len(touch), 1)
        command = touch[0]
        self.assertIn("-b", command)
        self.assertEqual(command[command.index("-b") + 1], self.export.APPLE_TOUCH_BACKGROUND)
        tile = (BRANDING / "svg" / "app-icon-graphite.svg").read_text(encoding="utf-8")
        self.assertIn(f'fill="{self.export.APPLE_TOUCH_BACKGROUND}"', tile)

    def test_the_other_icons_keep_their_transparent_corners(self):
        others = [c for c in self.commands() if not c[-1].endswith("apple-touch-icon.png")]
        self.assertTrue(others)
        self.assertTrue(all("-b" not in command for command in others))


class ScriptSourceTest(unittest.TestCase):
    def test_the_book_has_no_condition_that_is_always_true(self):
        text = (BRANDING / "book.py").read_text(encoding="utf-8")
        self.assertIsNone(re.search(r"(?m)^\s*if True:", text))

    def test_the_blender_scene_fixes_the_seed_of_the_sampling(self):
        text = (BRANDING / "render" / "two_materials.py").read_text(encoding="utf-8")
        self.assertRegex(text, r"scene\.cycles\.seed = RENDER_SEED")
        self.assertRegex(text, r"scene\.cycles\.use_animated_seed = False")
        self.assertRegex(text, r"(?m)^RENDER_SEED = \d+$")


if __name__ == "__main__":
    unittest.main()
