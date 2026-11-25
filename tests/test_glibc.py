# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Exercise the glibc floor check against realistic readelf output without readelf."""

import contextlib
import importlib.util
import io
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load_checker():
    spec = importlib.util.spec_from_file_location("glibc", ROOT / "scripts/check-glibc.py")
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    return checker


def readelf(libc_tags, definitions=(), extra_libs=(), declared=None, counts=None):
    """Build `readelf -W -V` output with the sections in their real order."""
    lines = [
        "",
        "Version symbols section '.gnu.version' contains 3 entries:",
        " Addr: 0x0000000000001000  Offset: 0x001000  Link: 5 (.dynsym)",
        "  000:   0 (*local*)       2 (GLIBC_2.2.5)    3 (GLIBC_2.34)",
        "",
    ]
    if definitions:
        lines += [
            "Version definition section '.gnu.version_d' contains 2 entries:",
            "  Addr: 0x0000000000002000  Offset: 0x002000  Link: 6 (.dynstr)",
            "  000000: Rev: 1  Flags: BASE   Index: 1  Cnt: 1  Name: ccp-client",
        ]
        lines += [
            f"  0x{index * 28:04x}: Rev: 1  Flags: none  Index: {index + 1}  Cnt: 1  Name: {name}"
            for index, name in enumerate(definitions, start=1)
        ]
        lines.append("")
    libs = [("libc.so.6", list(libc_tags)), *extra_libs]
    lines += [
        f"Version needs section '.gnu.version_r' contains {declared or len(libs)} entries:",
        " Addr: 0x0000000000003000  Offset: 0x003000  Link: 6 (.dynstr)",
    ]
    for number, (library, tags) in enumerate(libs):
        lines.append(
            f"  {number * 32:06x}: Version: 1  File: {library}  Cnt: {counts[number] if counts else len(tags)}"
        )
        lines += [
            f"  0x{16 + 16 * offset:04x}:   Name: {tag}  Flags: none  Version: {offset + 2}"
            for offset, tag in enumerate(tags)
        ]
    return "\n".join(lines) + "\n"


X86_64 = readelf(["GLIBC_2.2.5", "GLIBC_2.3.4", "GLIBC_2.14", "GLIBC_2.28"])
AARCH64 = readelf(["GLIBC_2.17"], extra_libs=[("ld-linux-aarch64.so.1", ["GLIBC_2.17"])])


def completed(stdout="", stderr="", returncode=0):
    return subprocess.CompletedProcess(["readelf"], returncode, stdout, stderr)


class GlibcCheckTests(unittest.TestCase):
    def setUp(self):
        self.checker = load_checker()
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.binary = Path(directory.name) / "ccp-client"
        self.binary.write_bytes(b"\x7fELF")

    def check(self, output, ceiling="2.28"):
        with patch.object(self.checker.subprocess, "run", return_value=completed(output)):
            return self.checker.check_binary(self.binary, ceiling)

    def assertRejected(self, output, *fragments, ceiling="2.28"):
        with self.assertRaises(self.checker.CheckError) as raised:
            self.check(output, ceiling)
        message = str(raised.exception)
        self.assertIn(str(self.binary), message)
        for fragment in fragments:
            self.assertIn(fragment, message)

    def test_x86_64_at_the_floor_passes(self):
        self.assertEqual(self.check(X86_64), f"{self.binary}: GLIBC_2.28")

    def test_aarch64_baseline_passes(self):
        self.assertEqual(self.check(AARCH64), f"{self.binary}: GLIBC_2.17")

    def test_reported_2_38_requirement_fails(self):
        output = readelf(["GLIBC_2.2.5", "GLIBC_2.34", "GLIBC_2.38"])
        self.assertRejected(output, "GLIBC_2.38", "GLIBC_2.28")

    def test_patch_release_above_the_floor_fails(self):
        self.assertRejected(readelf(["GLIBC_2.28.1"]), "GLIBC_2.28.1")

    def test_missing_patch_equals_zero_patch(self):
        self.assertEqual(self.check(readelf(["GLIBC_2.28.0"])), f"{self.binary}: GLIBC_2.28.0")
        self.assertEqual(
            self.check(readelf(["GLIBC_2.28"]), ceiling="2.28.0"), f"{self.binary}: GLIBC_2.28"
        )

    def test_versions_compare_numerically(self):
        self.assertEqual(
            self.check(readelf(["GLIBC_2.9", "GLIBC_2.17"])), f"{self.binary}: GLIBC_2.17"
        )
        self.assertEqual(
            self.check(readelf(["GLIBC_2.9", "GLIBC_2.28"])), f"{self.binary}: GLIBC_2.28"
        )
        self.assertRejected(readelf(["GLIBC_2.9", "GLIBC_2.100"]), "GLIBC_2.100")

    def test_every_needed_library_counts(self):
        output = readelf(["GLIBC_2.17"], extra_libs=[("libm.so.6", ["GLIBC_2.29"])])
        self.assertRejected(output, "GLIBC_2.29")

    def test_version_definitions_are_ignored(self):
        output = readelf(["GLIBC_2.17"], definitions=["GLIBC_2.38", "GLIBC_2.40"])
        self.assertEqual(self.check(output), f"{self.binary}: GLIBC_2.17")

    def test_other_version_namespaces_are_ignored(self):
        extra = [("libgcc_s.so.1", ["GCC_3.0"]), ("libstdc++.so.6", ["GLIBCXX_3.4.30"])]
        output = readelf(["GLIBC_2.17"], extra_libs=extra)
        self.assertEqual(self.check(output), f"{self.binary}: GLIBC_2.17")

    def test_unknown_glibc_tags_are_rejected(self):
        for tag in ("GLIBC_PRIVATE", "GLIBC_ABI_DT_RELR", "GLIBC_2.x"):
            with self.subTest(tag=tag):
                self.assertRejected(readelf(["GLIBC_2.17", tag]), tag)

    def test_empty_and_malformed_output_is_rejected(self):
        for output in ("", "readelf: nothing here\n", readelf([]), readelf(["GCC_3.0"])):
            with self.subTest(output=output):
                self.assertRejected(output, "no GLIBC version requirements")

    def failure_message(self, binary=None, **run_options):
        with (
            patch.object(self.checker.subprocess, "run", **run_options) as run,
            self.assertRaises(self.checker.CheckError) as raised,
        ):
            self.checker.check_binary(binary or self.binary, "2.28")
        return str(raised.exception), run

    def test_readelf_missing(self):
        message, _ = self.failure_message(side_effect=FileNotFoundError("readelf"))
        self.assertIn("cannot run readelf", message)

    def test_readelf_failure(self):
        failure = completed(stderr="readelf: Error: Not an ELF file\n", returncode=1)
        message, _ = self.failure_message(return_value=failure)
        self.assertIn("Not an ELF file", message)

    def test_readelf_failure_without_message_reports_status(self):
        message, _ = self.failure_message(return_value=completed(returncode=2))
        self.assertIn("exit status 2", message)

    def test_missing_file_fails_without_running_readelf(self):
        missing = self.binary.with_name("absent")
        message, run = self.failure_message(binary=missing)
        run.assert_not_called()
        self.assertIn(str(missing), message)

    def test_warning_on_stderr_fails_even_with_exit_zero(self):
        warning = "readelf: Warning: Unable to read entry 3 in version needs section\n"
        message, _ = self.failure_message(return_value=completed(X86_64, stderr=warning))
        self.assertIn("Unable to read entry 3", message)
        self.assertIn(str(self.binary), message)

    def test_count_larger_than_the_names_present_fails(self):
        output = readelf(["GLIBC_2.17"], counts=[2])
        self.assertRejected(output, "libc.so.6 declares 2 versions, found 1")

    def test_count_smaller_than_the_names_present_fails(self):
        output = readelf(["GLIBC_2.17", "GLIBC_2.28"], counts=[1])
        self.assertRejected(output, "declares 1 versions, found 2")

    def test_later_library_with_missing_names_fails(self):
        output = readelf(["GLIBC_2.17"], extra_libs=[("libm.so.6", ["GLIBC_2.2.5"])], counts=[1, 3])
        self.assertRejected(output, "libm.so.6 declares 3 versions, found 1")

    def test_section_entry_count_mismatch_fails(self):
        output = readelf(["GLIBC_2.17"], declared=3)
        self.assertRejected(output, "declares 3 libraries, found 1")

    def test_name_before_any_library_fails(self):
        output = readelf(["GLIBC_2.17"]).replace("File: libc.so.6  Cnt: 1", "Flags: none")
        self.assertRejected(output, "appears before any library")

    def test_readelf_runs_without_a_shell_in_the_c_locale(self):
        with patch.object(self.checker.subprocess, "run", return_value=completed(X86_64)) as run:
            self.checker.check_binary(self.binary, "2.28")
        arguments, options = run.call_args
        self.assertEqual(arguments[0], ["readelf", "-W", "-V", str(self.binary)])
        self.assertEqual(options["env"]["LC_ALL"], "C")
        self.assertNotIn("shell", options)


class GlibcMainTests(unittest.TestCase):
    def setUp(self):
        self.checker = load_checker()
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.good = Path(directory.name) / "good"
        self.bad = Path(directory.name) / "bad"
        self.good.write_bytes(b"good")
        self.bad.write_bytes(b"bad")

    def run_main(self, arguments, outputs):
        def fake_run(command, **_options):
            return completed(outputs[Path(command[-1]).name])

        stdout, stderr = io.StringIO(), io.StringIO()
        with (
            patch("sys.argv", ["check-glibc.py", *arguments]),
            patch.object(self.checker.subprocess, "run", side_effect=fake_run),
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            status = self.checker.main()
        return status, stdout.getvalue(), stderr.getvalue()

    def test_success_prints_one_line_per_file(self):
        outputs = {"good": X86_64, "bad": AARCH64}
        status, stdout, stderr = self.run_main(
            ["--max-glibc", "2.28", str(self.good), str(self.bad)], outputs
        )
        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertEqual(
            stdout.splitlines(), [f"{self.good}: GLIBC_2.28", f"{self.bad}: GLIBC_2.17"]
        )

    def test_one_bad_binary_fails_overall_and_all_are_checked(self):
        outputs = {"good": X86_64, "bad": readelf(["GLIBC_2.38"])}
        status, stdout, stderr = self.run_main(
            ["--max-glibc", "2.28", str(self.bad), str(self.good)], outputs
        )
        self.assertEqual(status, 1)
        self.assertIn(f"{self.good}: GLIBC_2.28", stdout)
        self.assertIn("GLIBC_2.38", stderr)
        self.assertIn(str(self.bad), stderr)

    def test_invalid_ceiling_is_rejected(self):
        for ceiling in ("2", "2.x", "GLIBC_2.28", "2.28.", "-2.28", ""):
            with self.subTest(ceiling=ceiling):
                with self.assertRaises(SystemExit) as raised:
                    self.run_main(["--max-glibc", ceiling, str(self.good)], {})
                self.assertEqual(raised.exception.code, 2)

    def test_ceiling_and_binaries_are_required(self):
        for arguments in ([str(self.good)], ["--max-glibc", "2.28"]):
            with self.subTest(arguments=arguments):
                with self.assertRaises(SystemExit) as raised:
                    self.run_main(arguments, {})
                self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
