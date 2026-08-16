#!/usr/bin/env python3
"""Regression tests for the release-note generator."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("generate_release_notes.py")
SPEC = importlib.util.spec_from_file_location("generate_release_notes", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot import release-note generator from {SCRIPT}")
NOTES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(NOTES)


class ReleaseNotesTests(unittest.TestCase):
    def test_release_date_dereferences_annotated_tag(self) -> None:
        with mock.patch.object(NOTES, "git", return_value="2026-08-16\n") as git:
            self.assertEqual(NOTES.release_date("v0.1.2"), "2026-08-16")

        git.assert_called_once_with(
            "show", "-s", "--format=%cs", "v0.1.2^{commit}"
        )

    def test_render_uses_version_date_and_nonempty_sections(self) -> None:
        commits = [("abc123", "fix", "keep annotated tag metadata out")]
        with mock.patch.object(NOTES, "commits_between", return_value=commits):
            body = NOTES.render(
                "v0.1.2",
                "v0.1.1",
                "https://github.com/example/flash-shot",
                "2026-08-16",
            )

        self.assertTrue(body.startswith("## [0.1.2] - 2026-08-16\n"))
        self.assertIn("Bug Fixes", body)
        self.assertNotIn("Features", body)
        self.assertIn("v0.1.1...v0.1.2", body)


if __name__ == "__main__":
    unittest.main()
