# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Exercise byte-level text rules, exact exceptions, and Svelte suppression ownership."""

import importlib.util
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load_checker():
    spec = importlib.util.spec_from_file_location("hygiene", ROOT / "scripts/check-repo-hygiene.py")
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    return checker


class TextRuleTests(unittest.TestCase):
    def setUp(self):
        self.checker = load_checker()

    def test_each_text_rule_reports_the_affected_line(self):
        for data, expected in (
            (b"one\ntwo \n", "source.md:2: trailing whitespace"),
            (b"one\ntwo\t\n", "source.md:2: trailing whitespace"),
            (b"one\r\ntwo\n", "source.md:1: use LF line endings"),
            (b"one\rtwo\n", "source.md:1: use LF line endings"),
            (b"one\ntwo", "source.md:EOF: missing final LF"),
            (
                "one\ntwo\u2014three\n".encode(),
                "source.md:2: rewrite the em dash as natural punctuation",
            ),
        ):
            with self.subTest(data=data):
                self.assertIn(expected, self.checker.check_text("source.md", data))
        self.assertEqual(self.checker.check_text("empty.md", b""), [])

    def test_unicode_fixtures_remain_valid_without_file_or_directory_exemptions(self):
        data = "é 雪 \U0001f991 \U0001f600 \u2764\ufe0f “quotes” 1\u20132\n".encode()
        for path in (
            "crates/protocol/testdata/enrollment_links.json",
            "crates/protocol/src/enrollment_link/tests.rs",
            "tests/enrollment_link.mjs",
            "tests/new_fixture.json",
        ):
            with self.subTest(path=path):
                self.assertEqual(self.checker.check_text(path, data), [])
                self.assertTrue(self.checker.check_text(path, data + "bad\u2014copy\n".encode()))
                self.assertTrue(self.checker.check_text(path, data + b"bad \n"))

    def test_each_exact_generated_or_upstream_file_exclusion_has_a_reason(self):
        expected = {
            "Cargo.lock",
            "crates/server/web/package-lock.json",
            "crates/server/web/public/fonts/FragmentMono-OFL.txt",
            "crates/server/web/public/fonts/OverusedGrotesk-OFL.txt",
        }
        self.assertEqual(set(self.checker.EXCLUDED_FILES), expected)
        bad = "bad\u2014text \r\nmissing newline".encode()
        for path, reason in self.checker.EXCLUDED_FILES.items():
            with self.subTest(path=path):
                self.assertTrue(reason.strip())
                self.assertEqual(self.checker.check_text(path, bad), [])
                self.assertTrue(self.checker.check_text(path + ".backup", bad))
        for path in ("crates/server/web/dist/app.js", "crates/server/web/dist/fonts/upstream.txt"):
            self.assertEqual(self.checker.check_text(path, bad), [])
        for path in (
            "crates/server/web/dist-other/app.js",
            "other/Cargo.lock",
            "crates/server/web/public/fonts/NewFont-OFL.txt",
        ):
            self.assertTrue(self.checker.check_text(path, bad))

    def test_frozen_changelog_is_matched_exactly_and_other_sections_are_checked(self):
        original = (ROOT / "CHANGELOG.md").read_bytes()
        self.assertEqual(self.checker.check_text("CHANGELOG.md", original), [])
        self.assertTrue(
            self.checker.check_text("CHANGELOG.md", b"new\xe2\x80\x94copy\n" + original)
        )
        with self.assertRaisesRegex(ValueError, "frozen bytes"):
            self.checker.check_text("CHANGELOG.md", original.replace(b"No CVE", b"No assigned CVE"))
        with self.assertRaisesRegex(ValueError, "missing or renamed"):
            self.checker.check_text("CHANGELOG.md", original.replace(b"## [0.1.1]", b"## [0.1.2]"))
        self.assertTrue(self.checker.check_text("CHANGELOG.md", original + b"## Future\nbad \n"))
        self.assertEqual(
            self.checker.check_text("CHANGELOG.md", original + b"## Future\nFine.\n"), []
        )

    def test_binary_assets_are_not_text_but_invalid_source_encoding_fails(self):
        self.assertEqual(self.checker.check_text("image.png", b"\x89PNG\r\n\0data"), [])
        self.assertEqual(self.checker.check_text("font.woff2", b"\xff\xfe"), [])
        self.assertEqual(
            self.checker.check_text("code.rs", b"\xff\xfe"), ["code.rs: text must be valid UTF-8"]
        )

    def test_svelte_suppressions_require_an_adjacent_or_inline_reason(self):
        for text in (
            "<!-- svelte-ignore a11y_no_noninteractive_tabindex -->\n<div/>\n",
            "<script>\n// svelte-ignore state_referenced_locally\nconst x = 1;\n</script>\n",
            "<!-- A reason. -->\n\n<!-- svelte-ignore warning -->\n",
            "<!-- svelte-ignore first -- reason -->\n<!-- svelte-ignore second -->\n",
        ):
            with self.subTest(text=text):
                self.assertTrue(self.checker.check_text("View.svelte", text.encode()))
        for text in (
            "<!-- A keyboard-operated chart needs a focus target. -->\n<!-- svelte-ignore warning -->\n",
            "<!-- svelte-ignore warning -- Keyboard users need this target. -->\n",
            "<script>\n// Scope belongs to the mounted view.\n// svelte-ignore state_referenced_locally\n</script>\n",
            "<script>\n/* Scope belongs to the\nmounted view. */\n/* svelte-ignore warning */\n</script>\n",
            '<script>\nconst text = "// svelte-ignore warning";\n</script>\n',
            "<p>A member's view.</p>\n<!-- Reason. -->\n<!-- svelte-ignore warning -->\n",
        ):
            with self.subTest(text=text):
                self.assertEqual(self.checker.check_text("View.svelte", text.encode()), [])


class TrackedSourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.checker = load_checker()
        self.git_program = shutil.which("git")
        self.assertIsNotNone(self.git_program)
        self.git("init", "-q")
        self.git("config", "user.name", "Test Maintainer")
        self.git("config", "user.email", "maintainer@example.com")
        self.git("config", "core.hooksPath", "/dev/null")
        # Fixture commits must not inherit a developer's external signing service.
        self.git("config", "commit.gpgsign", "false")

    def git(self, *args):
        # Fixed command and separate argv operate only on the disposable repository.
        return (
            subprocess.run(  # noqa: S603 -- Isolated temporary Git repository.
                [self.git_program, *args], cwd=self.root, capture_output=True, check=True
            )
            .stdout.decode()
            .strip()
        )

    def test_tracks_index_paths_reads_working_edits_and_ignores_untracked_output(self):
        source = self.root / "source with spaces.md"
        source.write_text("good\n")
        self.git("add", "--", source.name)
        self.git("commit", "-qm", "docs: initialize")
        source.write_text("bad\u2014copy\n")
        (self.root / "untracked.md").write_text("bad \n")
        self.assertEqual(len(self.checker.findings(self.root)), 1)
        self.assertEqual(self.checker.findings(self.root, "HEAD"), [])
        source.unlink()
        self.assertEqual(self.checker.findings(self.root), [])

    def test_cli_fails_for_findings_and_unavailable_revision(self):
        (self.root / "source.md").write_text("bad \n")
        self.git("add", "source.md")
        with patch.object(self.checker, "ROOT", self.root):
            for arguments in (["checker"], ["checker", "--revision", "missing"]):
                with self.subTest(arguments=arguments), patch("sys.argv", arguments):
                    with self.assertRaises(SystemExit) as failure:
                        self.checker.main()
                    self.assertEqual(failure.exception.code, 1)


if __name__ == "__main__":
    unittest.main()
