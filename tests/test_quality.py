# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Regression coverage for tool integrity, source discovery, and CLI accounting."""

from __future__ import annotations

import ast
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

import dependency_compat

ROOT = Path(__file__).resolve().parents[1]


def load_script(filename):
    spec = importlib.util.spec_from_file_location(filename, ROOT / "scripts" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def archive(member="tool", symlink=False):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as bundle:
        info = tarfile.TarInfo(member)
        if symlink:
            info.type = tarfile.SYMTYPE
            info.linkname = "outside"
            bundle.addfile(info)
        else:
            info.size = 3
            bundle.addfile(info, io.BytesIO(b"new"))
    return output.getvalue()


def counted_function_spans(checker, name, source):
    if name.endswith(".py"):
        lines = source.splitlines(keepends=True)
        offsets = [0]
        for line in lines:
            offsets.append(offsets[-1] + len(line))
        return [
            (
                offsets[min([node.lineno, *(d.lineno for d in node.decorator_list)]) - 1],
                offsets[node.end_lineno],
            )
            for node in ast.walk(ast.parse(source))
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name.startswith("test_")
        ]
    rust = name.endswith(".rs")
    code = checker.strip_rust(source) if rust else checker.strip_mjs(source)
    pattern = checker.RUST_PATTERNS["tests"] if rust else checker.MJS_PATTERNS["tests"]
    spans = []
    matches = pattern.finditer(code) if rust else checker.mjs_test_matches(code)
    for match in matches:
        opening, closing = ("{", "}") if rust else ("(", ")")
        start = code.index(opening, match.start())
        depth = 1
        end = start + 1
        while depth:
            depth += (code[end] == opening) - (code[end] == closing)
            end += 1
        spans.append((match.start(), end))
    return spans


def comment_function(name, source, start, end):
    fragment = source[start:end]
    if name.endswith(".py"):
        indent = fragment[: len(fragment) - len(fragment.lstrip())]
        replacement = (
            indent + "pass\n" + "".join("# " + line for line in fragment.splitlines(keepends=True))
        )
    else:
        replacement = "\n".join("// " + line for line in fragment.split("\n")) + "\n"
    return source[:start] + replacement + source[end:]


class ToolIntegrityTests(unittest.TestCase):
    def install(self, payload, checksum, directory):
        installer = load_script("install-lint-tools.py")
        asset = {"url": "https://github.com/example/tool/release", "sha256": checksum}
        with (
            patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(payload)),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            installer.install("tool", asset, directory)

    def test_verified_binary_replaces_only_selected_executable(self):
        payload = archive("../../tool")
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "bin"
            self.install(payload, hashlib.sha256(payload).hexdigest(), destination)
            self.assertEqual((destination / "tool").read_bytes(), b"new")
            self.assertEqual(list(Path(directory).iterdir()), [destination])

    def test_checksum_failure_preserves_existing_tool(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory)
            (destination / "tool").write_bytes(b"old")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                self.install(archive(), "0" * 64, destination)
            self.assertEqual((destination / "tool").read_bytes(), b"old")

    def test_archive_symlink_cannot_be_installed(self):
        payload = archive(symlink=True)
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory)
            with self.assertRaisesRegex(ValueError, "exactly one"):
                self.install(payload, hashlib.sha256(payload).hexdigest(), destination)
            self.assertEqual(list(destination.iterdir()), [])


class PythonAuditTests(unittest.TestCase):
    def test_audit_requires_the_current_checkout_and_keeps_third_party_versions(self):
        auditor = load_script("audit-python.py")
        local = Mock(metadata={"Name": "ccp-mcp-server"}, version="0.1.0")
        dependency = Mock(metadata={"Name": "example-package"}, version="1.2.3")
        shadow = Mock(metadata={"Name": "ccp-mcp-server"}, version="0.1.0")
        shadow.read_text.return_value = None
        local.read_text.return_value = json.dumps({"url": (auditor.ROOT / "mcp").as_uri()})
        with patch.object(
            auditor.metadata, "distributions", return_value=[local, shadow, dependency]
        ):
            self.assertEqual(auditor.requirements(), ["example-package==1.2.3"])
            local.read_text.return_value = json.dumps({"url": "file:///another-checkout"})
            with self.assertRaisesRegex(RuntimeError, "this checkout"):
                auditor.requirements()


class SourceGateTests(unittest.TestCase):
    def run_check(self, check, files):
        scanner = load_script("check-source.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # Fixed test commands run without a shell; Git initializes only the fixture.
            subprocess.run([shutil.which("git"), "init", "--quiet", str(root)], check=True)  # noqa: S603
            for name, content in files.items():
                (root / name).write_text(content)
            shutil.copyfile(ROOT / ".gitleaks.toml", root / ".gitleaks.toml")
            with patch.object(scanner, "ROOT", root), patch.object(sys, "argv", ["check", check]):
                scanner.main()

    def test_shell_gate_rejects_unquoted_variable_in_extensionless_script(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_check("shell", {"install-helper": '#!/bin/sh\nword="two words"\necho $word\n'})

    def test_secret_gate_rejects_untracked_credentials(self):
        # Deliberately synthetic token, assembled so the test source is not a credential.
        token = "ghp_" + hashlib.sha256(b"synthetic lint regression token").hexdigest()[:36]
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_check("secrets", {"config.json": json.dumps({"token": token})})

    def test_cli_helper_preserves_parent_counts_and_raw_json(self):
        runner = (ROOT / "tests" / "run.sh").read_text()
        helpers = runner[runner.index("RED=") : runner.index("# ── Temp dir")]
        program = (
            helpers
            + """
expect_success success printf '%s' '{"ok":true}'
[[ "$OUT" == '{"ok":true}' && "$PASS_COUNT" == 1 && "$FAIL_COUNT" == 0 ]] || exit 1
expect_success failure bash -c 'exit 7'
[[ "$PASS_COUNT" == 1 && "$FAIL_COUNT" == 1 ]] || exit 1
"""
        )
        # The shell executes checked-in helper code and fixed regression commands.
        subprocess.run([shutil.which("bash"), "-c", program], check=True, capture_output=True)  # noqa: S603


class CompatSummaryTests(unittest.TestCase):
    """tests/dependency_compat.py's printed claim, without real binaries."""

    def test_describes_only_server_restarts_without_old_binaries(self):
        message = dependency_compat.summary(old_client=None, old_server=None)
        self.assertNotIn("old/new", message)
        self.assertNotIn("upgrade/downgrade", message)
        self.assertIn("server restarts, existing enrollments, and persisted content", message)

    def test_describes_old_new_clients_only_when_supplied(self):
        message = dependency_compat.summary(old_client=Path("old-client"), old_server=None)
        self.assertIn("old/new TLS clients", message)
        self.assertNotIn("upgrade/downgrade", message)

    def test_describes_server_upgrade_downgrade_only_when_supplied(self):
        message = dependency_compat.summary(old_client=None, old_server=Path("old-server"))
        self.assertNotIn("old/new", message)
        self.assertIn("server upgrade/downgrade", message)

    def test_describes_both_when_both_old_binaries_are_supplied(self):
        message = dependency_compat.summary(
            old_client=Path("old-client"), old_server=Path("old-server")
        )
        self.assertIn("old/new TLS clients", message)
        self.assertIn("server upgrade/downgrade", message)

    def test_check_prints_the_exercised_compatibility_summary(self):
        for old_client, old_server in (
            (None, None),
            (Path("old-client"), None),
            (None, Path("old-server")),
            (Path("old-client"), Path("old-server")),
        ):
            args = SimpleNamespace(
                client=Path("client"),
                server=Path("server"),
                old_client=old_client,
                old_server=old_server,
            )
            output = io.StringIO()
            with (
                self.subTest(old_client=old_client, old_server=old_server),
                patch.object(dependency_compat, "free_port", side_effect=[10001, 10002]),
                patch.object(dependency_compat, "Server") as server,
                patch.object(dependency_compat, "run") as run,
                contextlib.redirect_stdout(output),
            ):
                server.session = "test-session"
                run.return_value = json.dumps(
                    {
                        "context": "baseline content new client append old client append",
                        "total_entries": 1,
                        "frequent_labels": ["dependency"],
                    }
                )
                dependency_compat.check(args)
                self.assertEqual(
                    output.getvalue(),
                    dependency_compat.summary(old_client=old_client, old_server=old_server) + "\n",
                )


class TestIntegrityGateTests(unittest.TestCase):
    BASE = "#[test]\nfn checks() {\n    assert_eq!(1, 1);\n}\n"
    MJS_BASE = "test('checks', () => {\n  assert.equal(1, 1);\n});\n"

    def run_check(self, change, allow=False, name="lib.rs", base=None):
        checker = load_script("check-test-integrity.py")
        git = shutil.which("git")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / name).write_text(self.BASE if base is None else base)
            # Fixed Git commands initialize and commit only the fixture repository.
            for command in (
                ["init", "--quiet"],
                ["add", name],
                [
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "-m",
                    "base",
                ],
            ):
                subprocess.run([git, *command], cwd=root, check=True)  # noqa: S603
            (root / name).write_text(change)
            environment = {"CCP_ALLOW_TEST_REDUCTION": "1"} if allow else {}
            with (
                patch.object(checker, "ROOT", root),
                patch.object(sys, "argv", ["check", "--base", "HEAD"]),
                patch.dict("os.environ", environment),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                checker.main()

    def test_unchanged_and_added_checks_pass(self):
        self.run_check(self.BASE)
        self.run_check(self.BASE + "#[test]\nfn more() {\n    assert!(true);\n}\n")

    def test_removed_assertion_fails(self):
        with self.assertRaises(SystemExit):
            self.run_check("#[test]\nfn checks() {}\n")

    def test_unrelated_additions_cannot_offset_file_reductions(self):
        checker = load_script("check-test-integrity.py")
        base = {"original.rs": checker.count("original.rs", self.BASE)}
        for replacement in ("", "#[test]\nfn checks() {}\n"):
            with self.subTest(replacement=replacement), contextlib.redirect_stdout(io.StringIO()):
                head = {
                    "original.rs": checker.count("original.rs", replacement),
                    "unrelated.rs": checker.count("unrelated.rs", self.BASE),
                }
                problems = checker.compare(base, head)
                self.assertIn("original.rs: assertions 1 -> 0", problems)
                if not replacement:
                    self.assertIn("original.rs: tests 1 -> 0", problems)

    def test_removed_skip_cannot_offset_a_new_skip_elsewhere(self):
        checker = load_script("check-test-integrity.py")
        skipped = self.BASE.replace("#[test]", "#[test]\n#[ignore]")
        base = {"original.rs": checker.count("original.rs", skipped)}
        head = {
            "original.rs": checker.count("original.rs", self.BASE),
            "unrelated.rs": checker.count("unrelated.rs", skipped),
        }
        with contextlib.redirect_stdout(io.StringIO()):
            problems = checker.compare(base, head)
        self.assertIn("unrelated.rs: skips 0 -> 1", problems)

    def test_new_ignore_fails_unless_reviewed(self):
        ignored = self.BASE.replace("#[test]", "#[test]\n#[ignore]")
        with self.assertRaises(SystemExit):
            self.run_check(ignored)
        self.run_check(ignored, allow=True)

    def test_mjs_unrelated_additions_cannot_offset_file_reductions(self):
        checker = load_script("check-test-integrity.py")
        base = {"original.mjs": checker.count("original.mjs", self.MJS_BASE)}
        for replacement in ("", "test('checks', () => {});\n"):
            with self.subTest(replacement=replacement), contextlib.redirect_stdout(io.StringIO()):
                head = {
                    "original.mjs": checker.count("original.mjs", replacement),
                    "unrelated.mjs": checker.count("unrelated.mjs", self.MJS_BASE),
                }
                problems = checker.compare(base, head)
                self.assertIn("original.mjs: assertions 1 -> 0", problems)
                if not replacement:
                    self.assertIn("original.mjs: tests 1 -> 0", problems)

    def test_mjs_removed_skip_cannot_offset_a_new_skip_elsewhere(self):
        checker = load_script("check-test-integrity.py")
        skipped = self.MJS_BASE.replace("test(", "test.skip(", 1)
        base = {"original.mjs": checker.count("original.mjs", skipped)}
        head = {
            "original.mjs": checker.count("original.mjs", self.MJS_BASE),
            "unrelated.mjs": checker.count("unrelated.mjs", skipped),
        }
        with contextlib.redirect_stdout(io.StringIO()):
            problems = checker.compare(base, head)
        self.assertIn("unrelated.mjs: skips 0 -> 1", problems)

    def test_mjs_new_skip_fails_unless_reviewed(self):
        skipped = self.MJS_BASE.replace("test(", "test.skip(", 1)
        with self.assertRaises(SystemExit):
            self.run_check(skipped, name="sample.mjs", base=self.MJS_BASE)
        self.run_check(skipped, allow=True, name="sample.mjs", base=self.MJS_BASE)

    def test_mjs_skip_markers_are_caught_in_every_position_and_spelling(self):
        checker = load_script("check-test-integrity.py")
        samples = {
            "skip not first key": "test('a', {timeout: 100, skip: true}, () => {});\n",
            "skip shorthand": "const skip = true; test('a', {a, skip, b}, () => {});\n",
            "single-quoted key": "test('a', {'skip': true}, () => {});\n",
            "double-quoted key": "test('a', {\"skip\": true}, () => {});\n",
            "todo key": "test('a', {todo: true}, () => {});\n",
            "bare todo call": "test.todo('a', () => {});\n",
            "skip alias assignment": "const s = test.skip;\ns('a', () => {});\n",
        }
        for label, sample in samples.items():
            with self.subTest(label=label):
                self.assertGreaterEqual(
                    checker.count("x.mjs", sample)["skips"], 1, f"missed: {label}"
                )

    def test_mjs_subtests_and_assert_chains_are_counted(self):
        checker = load_script("check-test-integrity.py")
        sample = (
            "test('outer', async t => { await t.test('inner', () => {}); });\n"
            "assert(true);\n"
            "assert.ok(1);\n"
            "assert.strict.equal(1, 1);\n"
        )
        counts = checker.count("x.mjs", sample)
        self.assertEqual(counts["tests"], 2)
        self.assertEqual(counts["assertions"], 3)

    def test_mjs_scattered_skip_key_fails_unless_reviewed(self):
        scattered = self.MJS_BASE.replace(
            "test('checks'", "test('checks', {timeout: 100, skip: true}"
        )
        with self.assertRaises(SystemExit):
            self.run_check(scattered, name="sample.mjs", base=self.MJS_BASE)
        self.run_check(scattered, allow=True, name="sample.mjs", base=self.MJS_BASE)

    def test_commented_out_tests_and_assertions_do_not_count(self):
        checker = load_script("check-test-integrity.py")
        samples = {
            "x.rs": (
                "// #[test]\n"
                "// fn checks() { assert_eq!(1, 1); }\n"
                "/* #[test]\n"
                "fn blocked() { assert_eq!(2, 2); } */\n"
                'let url = "https://example.com"; // not a comment marker bug\n'
            ),
            "x.py": (
                "# def test_checks():\n"
                "#     self.assertEqual(1, 1)\n"
                "# self.assertEqual(2, 2)\n"
                'url = "https://example.com#fragment"\n'
            ),
            "x.mjs": (
                "// test('checks', () => { assert.equal(1, 1); });\n"
                "/* test('blocked', () => { assert.equal(2, 2); }); */\n"
                'const url = "https://example.com"; // not a comment marker bug\n'
            ),
        }
        for path, sample in samples.items():
            with self.subTest(path=path):
                counts = checker.count(path, sample)
                self.assertEqual(counts["tests"], 0, f"{path}: {counts}")
                self.assertEqual(counts["assertions"], 0, f"{path}: {counts}")

    def test_commented_out_rust_test_fails_the_real_gate_unless_reviewed(self):
        commented = "// #[test]\n// fn checks() {\n//     assert_eq!(1, 1);\n// }\n"
        with self.assertRaises(SystemExit):
            self.run_check(commented)
        self.run_check(commented, allow=True)

    def test_language_lexer_probes(self):
        checker = load_script("check-test-integrity.py")
        rust_test = "#[test]\nfn test_live() { assert!(true); }\n"
        js_test = "test('live', () => { assert(true); });\n"
        # Each row checks all categories: tests, assertions, skips.
        probes = [
            (
                "lifetime before commented test",
                ".rs",
                'const S: &\'static str = "s";\n// #[test]\n// fn test_hidden() {}\n' + rust_test,
                (1, 1, 0),
            ),
            (
                "nested block comment",
                ".rs",
                "/* outer /* inner */ #[test] fn test_hidden() {} */\n" + rust_test,
                (1, 1, 0),
            ),
            ("raw slash quotes", ".rs", 'let s = r#"a"//"#;\n' + rust_test, (1, 1, 0)),
            ("raw internal quote", ".rs", 'let s = r#"a " b"#;\n' + rust_test, (1, 1, 0)),
            (
                "raw many hashes",
                ".rs",
                'let s = br###"#[test] assert!(false);"###;\n' + rust_test,
                (1, 1, 0),
            ),
            (
                "raw no hashes",
                ".rs",
                'let s = r"#[test] assert!(false);";\n' + rust_test,
                (1, 1, 0),
            ),
            (
                "escaped bytes",
                ".rs",
                'let s = b"\\"// #[test] assert!(false)";\n' + rust_test,
                (1, 1, 0),
            ),
            (
                "chars versus lifetimes",
                ".rs",
                r"let chars = ('x', '\n', '\u{22}', '\''); fn f<'a>(s: &'a str) {}"
                + "\n"
                + rust_test,
                (1, 1, 0),
            ),
            ("quote char", ".rs", "let c = '\"';\n" + rust_test, (1, 1, 0)),
            ("slash char", ".rs", "let c = '/';\n" + rust_test, (1, 1, 0)),
            (
                "doc comments",
                ".rs",
                "/// #[test] assert!(false)\n/** #[test] assert!(false) */\n" + rust_test,
                (1, 1, 0),
            ),
            (
                "cfg_attr ignore",
                ".rs",
                '#[cfg_attr(all(unix, feature = "x"), ignore)]\n' + rust_test,
                (1, 1, 1),
            ),
            ("regex slash class", ".mjs", "const r = /[//]/;\n// " + js_test + js_test, (1, 1, 0)),
            ("regex quote", ".mjs", 'const r = /"/;\n// ' + js_test + js_test, (1, 1, 0)),
            ("regex escaped slash star", ".mjs", r"const r = /a\/*b/;" + "\n" + js_test, (1, 1, 0)),
            (
                "regex literal block opener",
                ".mjs",
                r"const r = /[/*]/;" + "\n" + js_test,
                (1, 1, 0),
            ),
            (
                "regex return keyword",
                ".mjs",
                'function f() { return /"/; }\n// ' + js_test + js_test,
                (1, 1, 0),
            ),
            ("regex after brace", ".mjs", '{} /"/.test(s);\n// ' + js_test + js_test, (1, 1, 0)),
            (
                "template comment",
                ".mjs",
                "const s = `${1 /* assert(true) */}`;\n" + js_test,
                (1, 1, 0),
            ),
            (
                "nested templates",
                ".mjs",
                "const s = `${{key: `${1 /* assert(false) */}`}}`;\n" + js_test,
                (1, 1, 0),
            ),
            ("template live code", ".mjs", "const s = `${assert(true)}`;", (0, 1, 0)),
            ("template contents", ".mjs", "const s = `test('hidden') assert(true)`;", (0, 0, 0)),
            ("computed skip key", ".mjs", "test('live', {['skip']: true}, () => {});", (1, 0, 1)),
            ("computed skip call", ".mjs", "test['skip']('live', () => {});", (1, 0, 1)),
            ("computed todo call", ".mjs", 'test["todo"]("live", () => {});', (1, 0, 1)),
            ("regex test method", ".mjs", "re.test(s);", (0, 0, 0)),
            ("spaced regex test method", ".mjs", "re . test(s);", (0, 0, 0)),
            ("subtest string", ".mjs", "t.test('live', () => {});", (1, 0, 0)),
            ("subtest template", ".mjs", "t.test(`live`, () => {});", (1, 0, 0)),
            ("spaced assert member", ".mjs", "assert . equal(1, 1);", (0, 1, 0)),
            ("JS URL", ".mjs", 'const s = "https://example.com/*";\n' + js_test, (1, 1, 0)),
            ("Rust URL", ".rs", 'let s = "https://example.com/*";\n' + rust_test, (1, 1, 0)),
            (
                "Python URL",
                ".py",
                's = "https://example.com/#"\ndef test_live(): assert True',
                (1, 1, 0),
            ),
            (
                "triple quoted tests",
                ".py",
                '"""\ndef test_hidden():\n    assert False\n"""\ndef test_live(): assert True',
                (1, 1, 0),
            ),
            (
                "f-string reused quotes",
                ".py",
                's = f"{"#"}"\ndef test_live(): assert True',
                (1, 1, 0),
            ),
            ("assert parenthesized", ".py", "def test_live(): assert(True)", (1, 1, 0)),
            ("unrelated assert member", ".py", "other.assertSomething()", (0, 0, 0)),
            (
                "Python assertion calls",
                ".py",
                "def test_live():\n self.assertEqual(1, 1)\n self.assertRaises(ValueError, f)\n with pytest.raises(ValueError): f()",
                (1, 3, 0),
            ),
            ("pytest skip", ".py", "def test_live(): pytest.skip('reason')", (1, 0, 1)),
            (
                "unittest SkipTest",
                ".py",
                "def test_live(): raise unittest.SkipTest('reason')",
                (1, 0, 1),
            ),
            ("imported SkipTest", ".py", "def test_live(): raise SkipTest", (1, 0, 1)),
            ("self skipTest", ".py", "def test_live(self): self.skipTest('reason')", (1, 0, 1)),
            ("mark skip", ".py", "@mark.skip\ndef test_live(): pass", (1, 0, 1)),
            (
                "pytest mark skipif",
                ".py",
                "@pytest.mark.skipif(True)\ndef test_live(): pass",
                (1, 0, 1),
            ),
            ("unittest skip", ".py", "@unittest.skip('reason')\ndef test_live(): pass", (1, 0, 1)),
            ("imported skip", ".py", "@skip('reason')\ndef test_live(): pass", (1, 0, 1)),
            ("imported skipIf", ".py", "@skipIf(True, 'reason')\ndef test_live(): pass", (1, 0, 1)),
            ("skip class", ".py", "@unittest.skip('reason')\nclass Tests: pass", (0, 0, 1)),
            (
                "fallback drops strings and fstrings",
                ".py",
                'future syntax\ns = f"{"#"}"\ns = "assert True"\n# assert False\ndef test_live(): assert(True)',
                (1, 1, 0),
            ),
            (
                "heredoc quoted",
                ".sh",
                ": <<'X'\nexpect_success hidden\nskip hidden\nX\nexpect_success live\n",
                (0, 1, 0),
            ),
            (
                "heredoc bare",
                ".sh",
                ": <<X\nexpect_success hidden\nskip hidden\nX\nexpect_success live\n",
                (0, 1, 0),
            ),
        ]
        probes += [
            ("always false cfg any", ".rs", "#[cfg(any())]\n" + rust_test, (1, 1, 1)),
            ("always false cfg not all", ".rs", "#[cfg(not(all()))]\n" + rust_test, (1, 1, 1)),
            (
                "always false cfg spaced",
                ".rs",
                "# [ cfg ( not ( all ( ) ) ) ]\n" + rust_test,
                (1, 1, 1),
            ),
            ("platform cfg is not skip", ".rs", "#[cfg(unix)]\n" + rust_test, (1, 1, 0)),
            (
                "cfg on ordinary function",
                ".rs",
                "#[cfg(any())]\nfn hidden() {}\n" + rust_test,
                (1, 1, 0),
            ),
            ("spaced Rust test attribute", ".rs", "# [ test ]\nfn test_live() {}", (1, 0, 0)),
            ("spaced Rust assertion", ".rs", "fn test_live() { assert ! (true); }", (0, 1, 0)),
            (
                "unittest case skip",
                ".py",
                "@unittest.case.skip('reason')\ndef test_live(): pass",
                (1, 0, 1),
            ),
            (
                "unittest case skipIf",
                ".py",
                "@unittest.case.skipIf(True, 'reason')\ndef test_live(): pass",
                (1, 0, 1),
            ),
            (
                "unittest case skipUnless",
                ".py",
                "@unittest.case.skipUnless(False, 'reason')\ndef test_live(): pass",
                (1, 0, 1),
            ),
            (
                "unittest skip flag",
                ".py",
                "class Tests:\n __unittest_skip__ = True\n def test_live(self): pass",
                (1, 0, 1),
            ),
            (
                "unittest annotated skip flag",
                ".py",
                "class Tests:\n __unittest_skip__: bool = True\n def test_live(self): pass",
                (1, 0, 1),
            ),
            (
                "unittest skip flag false",
                ".py",
                "class Tests:\n __unittest_skip__ = False\n def test_live(self): pass",
                (1, 0, 0),
            ),
            (
                "simple module skip alias",
                ".py",
                "disabled = unittest.skip('reason')\n@disabled\ndef test_live(): pass",
                (1, 0, 1),
            ),
            (
                "module case skip alias",
                ".py",
                "disabled = unittest.case.skip('reason')\n@disabled\nclass Tests:\n def test_live(self): pass",
                (1, 0, 1),
            ),
            (
                "if False removes test",
                ".py",
                "if False:\n def test_hidden(): assert False",
                (0, 0, 0),
            ),
            (
                "if False keeps else",
                ".py",
                "if False:\n def test_hidden(): assert False\nelse:\n def test_live(): assert True",
                (1, 1, 0),
            ),
            ("AST route", ".py", "def test_live(): assert True", (1, 1, 0)),
            ("tokenizer route", ".py", "future syntax\ndef test_live(): assert True", (1, 1, 0)),
            ("false skip option", ".mjs", "test('live', {skip: false}, () => {});", (1, 0, 0)),
            ("zero skip option", ".mjs", "test('live', {skip: 0}, () => {});", (1, 0, 0)),
            ("empty skip option", ".mjs", "test('live', {skip: ''}, () => {});", (1, 0, 0)),
            (
                "false computed skip option",
                ".mjs",
                "test('live', {['skip']: false}, () => {});",
                (1, 0, 0),
            ),
            ("arbitrary todo array", ".mjs", "const states = ['todo'];", (0, 0, 0)),
            ("arbitrary skip object", ".mjs", "const options = {skip: true};", (0, 0, 0)),
            (
                "arbitrary computed skip object",
                ".mjs",
                "const options = {['skip']: true};",
                (0, 0, 0),
            ),
            (
                "nested unrelated skip object",
                ".mjs",
                "test('live', {other: {skip: true}}, () => {});",
                (1, 0, 0),
            ),
            ("escaped skip key", ".mjs", r"test('live', {'s\x6bip': true}, () => {});", (1, 0, 1)),
            (
                "unicode skip key",
                ".mjs",
                r"test('live', {['s\u006bip']: true}, () => {});",
                (1, 0, 1),
            ),
            ("template skip key", ".mjs", "test('live', {[`skip`]: true}, () => {});", (1, 0, 1)),
            (
                "computed variable key is not a skip",
                ".mjs",
                "const skip = 'timeout'; test('live', {[skip]: 100}, () => {});",
                (1, 0, 0),
            ),
            (
                "line continuation inside a skip key",
                ".mjs",
                "test('live', {'sk\\\nip': true}, () => {});",
                (1, 0, 1),
            ),
            (
                "escaped template skip key",
                ".mjs",
                r"test('live', {[`s\u{6b}ip`]: true}, () => {});",
                (1, 0, 1),
            ),
            ("it options", ".mjs", "it('live', {skip: true}, () => {});", (1, 0, 1)),
            ("subtest options", ".mjs", "t.test('live', {todo: true}, () => {});", (1, 0, 1)),
            ("regex named first argument", ".mjs", "r.test('x');", (0, 0, 0)),
            ("increment then division", ".mjs", "x++ / 2; assert(true); / 3;", (0, 1, 0)),
            ("decrement then division", ".mjs", "x-- / 2; assert(true); / 3;", (0, 1, 0)),
            (
                "single quote ends at newline",
                ".mjs",
                "const s = 'unterminated\nassert(true);\n'",
                (0, 1, 0),
            ),
            (
                "double quote ends at newline",
                ".mjs",
                'const s = "unterminated\nassert(true);\n"',
                (0, 1, 0),
            ),
            (
                "heredoc double quotes",
                ".sh",
                ': <<"END"\nexpect_success hidden\nskip hidden\nEND\nexpect_success live\n',
                (0, 1, 0),
            ),
            (
                "heredoc escaped delimiter",
                ".sh",
                ": <<\\END\nexpect_success hidden\nskip hidden\nEND\nexpect_success live\n",
                (0, 1, 0),
            ),
            (
                "heredoc tab stripping",
                ".sh",
                ": <<-END\n\texpect_success hidden\n\tskip hidden\n\tEND\nexpect_success live\n",
                (0, 1, 0),
            ),
        ]
        for label, suffix, source, expected in probes:
            with self.subTest(probe=label):
                counts = checker.count("probe" + suffix, source)
                self.assertEqual(
                    tuple(counts[key] for key in ("tests", "assertions", "skips")), expected
                )
        print(f"Lexer probes: {len(probes)} rows")

    def test_python_uses_ast_before_falling_back_to_tokenize(self):
        checker = load_script("check-test-integrity.py")
        source = "def test_live(): assert True"  # AST route row in the probe table.
        with patch.object(checker, "_python_tokens", wraps=checker._python_tokens) as fallback:
            self.assertEqual(checker.count("probe.py", source)["tests"], 1)
            fallback.assert_not_called()
            self.assertEqual(checker.count("probe.py", "future syntax\n" + source)["tests"], 1)
            fallback.assert_called_once()

    def test_commenting_counted_functions_reduces_repository_counts(self):
        checker = load_script("check-test-integrity.py")
        checked = 0
        failures = []
        paths = checker.git("ls-files", "-z").decode().split("\0")
        for name in paths:
            if Path(name).suffix not in (".rs", ".py", ".mjs"):
                continue
            source = (ROOT / name).read_text()
            before = checker.count(name, source)["tests"]
            spans = counted_function_spans(checker, name, source)
            self.assertEqual(len(spans), before, name)
            if os.environ.get("CCP_FULL_INTEGRITY_SELF_CHECK") != "1":
                spans = spans[:1]  # Sample each file; full scan is an explicit validation run.
            for start, end in spans:
                changed = comment_function(name, source, start, end)
                checked += 1
                if checker.count(name, changed)["tests"] >= before:
                    failures.append(f"{name}:{source.count(chr(10), 0, start) + 1}")
        print(f"Repository integrity self-check: {checked} tests checked, {len(failures)} failures")
        self.assertGreater(checked, 0)
        self.assertEqual(failures, [])


class NativeResolverTests(unittest.TestCase):
    def test_cold_checkout_selects_each_binary_without_changing_configured_commands(self):
        from ccp_mcp_server import server

        with tempfile.TemporaryDirectory() as directory:
            for component in ("client", "server"):
                with self.subTest(component=component):
                    key = f"CCP_{component.upper()}_BIN"
                    resolver = getattr(server, f"_resolve_{component}_command")
                    with (
                        patch.dict("os.environ", {key: ""}),
                        patch.object(
                            server,
                            f"DEFAULT_{component.upper()}_BINARY",
                            Path(directory) / component,
                        ),
                        patch.object(
                            server, f"DEFAULT_{component.upper()}_MANIFEST", ROOT / "Cargo.toml"
                        ),
                        patch.object(server.shutil, "which", return_value="/trusted/cargo"),
                    ):
                        self.assertEqual(
                            resolver().argv,
                            [
                                "/trusted/cargo",
                                "run",
                                "--quiet",
                                "--manifest-path",
                                str(ROOT / "Cargo.toml"),
                                "--bin",
                                component,
                                "--",
                            ],
                        )
                        with patch.dict("os.environ", {key: "/configured/binary"}):
                            self.assertEqual(resolver().argv, ["/configured/binary"])


class CoverageGateTests(unittest.TestCase):
    def run_check(self, percent):
        checker = load_script("check-coverage.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "coverage.json"
            report.write_text(json.dumps({"data": [{"totals": {"lines": {"percent": percent}}}]}))
            baseline = root / "baseline.json"
            baseline.write_text(json.dumps({"rust_line_percent": 60.0}))
            with (
                patch.object(checker, "BASELINE", baseline),
                patch.object(checker, "ROOT", root),
                patch.object(sys, "argv", ["check", str(report)]),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                checker.main()

    def test_coverage_within_tolerance_passes(self):
        self.run_check(59.9)
        self.run_check(75.0)

    def test_coverage_drop_fails(self):
        with self.assertRaises(SystemExit):
            self.run_check(59.0)


if __name__ == "__main__":
    unittest.main()
