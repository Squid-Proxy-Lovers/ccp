"""Exercise artifact integrity, publication recovery, and the actual installer."""

import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

from managed_bundle_tests import ManagedBundleTests as ManagedBundleTests

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location("nightly", ROOT / "scripts/nightly.py")
nightly = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(nightly)
SHA = "a" * 40
TAG = "nightly-2026-10-12-" + SHA[:12]


def release(sha=SHA, *, draft=False, tag=TAG, published="2026-10-12T06:40:00Z"):
    return {
        "tag_name": tag,
        "target_commitish": sha,
        "draft": draft,
        "prerelease": True,
        "published_at": published,
    }


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.dist = Path(self.directory.name)
        for name in nightly.ASSETS:
            (self.dist / name).write_bytes(name.encode())
        nightly.prepare(self.dist, SHA)
        (self.dist / "ccp-provenance.json").write_text("fixture boundary bundle")
        self.calls = []

    def publish(self, *, releases=(), failure=(), revision=SHA, ancestor=False):
        # ARG001: the fake mirrors nightly.run's keyword interface.
        def command(*args, capture=False, check=True):  # noqa: ARG001
            self.calls.append(args)
            if failure and args[: len(failure)] == failure:
                raise subprocess.CalledProcessError(1, args)
            output = ""
            code = 0
            if args[:2] == ("gh", "api"):
                output = json.dumps([list(releases)])
            elif args[:3] == ("docker", "image", "inspect"):
                output = revision
            elif args[:2] == ("git", "merge-base"):
                code = 0 if ancestor else 1
            return subprocess.CompletedProcess(args, code, stdout=output)

        with (
            patch.dict(
                os.environ,
                GITHUB_SHA=SHA,
                GITHUB_REPOSITORY="Squid-Proxy-Lovers/ccp",
                GITHUB_REF="refs/heads/main",
                GITHUB_EVENT_NAME="schedule",
            ),
            patch.object(nightly, "run", side_effect=command),
            patch.object(nightly.release_contract.subprocess, "run") as provenance,
            patch.object(nightly, "datetime") as clock,
        ):
            clock.now.return_value = datetime(2026, 10, 12, tzinfo=timezone.utc)
            nightly.publish(self.dist, self.dist / "image.tar.gz")
            self.assertEqual(provenance.call_count, 1)

    def test_missing_platform_prevents_packaging(self):
        (self.dist / "ccp-server-darwin-aarch64").unlink()
        with self.assertRaisesRegex(ValueError, "Missing"):
            nightly.prepare(self.dist, SHA)

    def test_changed_artifact_cannot_be_published(self):
        (self.dist / "ccp-mcp.tar.gz").write_bytes(b"different commit")
        with self.assertRaisesRegex(ValueError, "Invalid release artifact"):
            self.publish()
        self.assertEqual(self.calls, [])

    def test_complete_assets_and_pinned_image_precede_release_visibility(self):
        self.publish()
        create = next(call for call in self.calls if call[:3] == ("gh", "release", "create"))
        self.assertIn("--draft", create)
        self.assertIn("--prerelease", create)
        self.assertIn("--latest=false", create)
        self.assertEqual(create[create.index("--target") + 1], SHA)
        upload = next(call for call in self.calls if call[:3] == ("gh", "release", "upload"))
        self.assertIn(str(self.dist / "ccp-mcp.tar.gz"), upload)
        self.assertIn(str(self.dist / "SHA256SUMS"), upload)
        edit = next(call for call in self.calls if call[:3] == ("gh", "release", "edit"))
        pushes = [call for call in self.calls if call[:2] == ("docker", "push")]
        self.assertLess(self.calls.index(upload), self.calls.index(edit))
        self.assertLess(self.calls.index(pushes[0]), self.calls.index(edit))
        self.assertGreater(self.calls.index(pushes[-1]), self.calls.index(edit))
        self.assertTrue(pushes[-1][-1].endswith(":nightly"))
        self.assertFalse(any(":latest" in str(call) for call in self.calls))

    def test_upload_or_image_failure_keeps_release_hidden(self):
        for failure in (("gh", "release", "upload"), ("docker", "push")):
            with self.subTest(failure=failure):
                self.calls.clear()
                with self.assertRaises(subprocess.CalledProcessError):
                    self.publish(failure=failure)
                self.assertFalse(any(call[:3] == ("gh", "release", "edit") for call in self.calls))
                self.assertFalse(any(call[-1].endswith(":nightly") for call in self.calls))

    def test_release_failure_does_not_move_alias(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.publish(failure=("gh", "release", "edit"))
        self.assertFalse(any(call[-1].endswith(":nightly") for call in self.calls))

    def test_published_release_is_reused_and_alias_can_be_repaired(self):
        self.publish(releases=(release(),))
        self.assertFalse(any(call[:2] == ("gh", "release") for call in self.calls))
        self.assertIn(("docker", "pull", f"ghcr.io/squid-proxy-lovers/ccp:{TAG}"), self.calls)
        self.assertEqual(
            self.calls[-1], ("docker", "push", "ghcr.io/squid-proxy-lovers/ccp:nightly")
        )

    def test_old_successful_rerun_does_not_rewind_alias(self):
        newer = release(
            "b" * 40, tag="nightly-2026-10-13-" + "b" * 12, published="2026-10-13T06:40:00Z"
        )
        self.publish(releases=(release(), newer))
        self.assertEqual(len(self.calls), 1)

    def test_partial_draft_resumes_without_recreating_release(self):
        self.publish(releases=(release(draft=True),))
        self.assertFalse(any(call[:3] == ("gh", "release", "create") for call in self.calls))
        upload = next(call for call in self.calls if call[:3] == ("gh", "release", "upload"))
        self.assertIn("--clobber", upload)
        self.assertTrue(any(call[:3] == ("gh", "release", "edit") for call in self.calls))

    def test_alias_failure_leaves_published_release_recoverable(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.publish(failure=("docker", "push", "ghcr.io/squid-proxy-lovers/ccp:nightly"))
        self.assertTrue(any(call[:3] == ("gh", "release", "edit") for call in self.calls))
        self.calls.clear()
        self.publish(releases=(release(),))
        self.assertFalse(any(call[:2] == ("gh", "release") for call in self.calls))
        self.assertEqual(
            self.calls[-1], ("docker", "push", "ghcr.io/squid-proxy-lovers/ccp:nightly")
        )

    def test_old_queued_commit_cannot_replace_newer_nightly(self):
        newer = release("b" * 40, tag="nightly-2026-10-13-" + "b" * 12)
        self.publish(releases=(newer,), ancestor=True)
        self.assertEqual([call[0] for call in self.calls], ["gh", "git"])

    def test_wrong_image_commit_prevents_release_creation(self):
        with self.assertRaisesRegex(ValueError, "Container image"):
            self.publish(revision="b" * 40)
        self.assertFalse(any(call[:2] == ("gh", "release") for call in self.calls))

    def test_pull_requests_cannot_invoke_publisher(self):
        with (
            patch.dict(
                os.environ,
                GITHUB_SHA=SHA,
                GITHUB_REPOSITORY="owner/repo",
                GITHUB_REF="refs/pull/10/merge",
                GITHUB_EVENT_NAME="pull_request",
            ),
            self.assertRaisesRegex(ValueError, "scheduled/manual main"),
        ):
            nightly.publish(self.dist, self.dist / "image.tar.gz")

    def test_missing_or_failed_provenance_prevents_every_publication_call(self):
        with (
            patch.dict(
                os.environ,
                GITHUB_SHA=SHA,
                GITHUB_REPOSITORY="Squid-Proxy-Lovers/ccp",
                GITHUB_REF="refs/heads/main",
                GITHUB_EVENT_NAME="schedule",
            ),
            patch.object(nightly, "run") as publisher,
            patch.object(
                nightly.release_contract.subprocess,
                "run",
                side_effect=FileNotFoundError("verifier unavailable"),
            ),
            self.assertRaises(ValueError),
        ):
            nightly.publish(self.dist, self.dist / "image.tar.gz")
        publisher.assert_not_called()
        (self.dist / "ccp-provenance.json").unlink()
        with self.assertRaises(OSError):
            self.publish()
        self.assertEqual(self.calls, [])

    def test_fork_cannot_publish_under_the_official_contract(self):
        with (
            patch.dict(
                os.environ,
                GITHUB_SHA=SHA,
                GITHUB_REPOSITORY="someone/fork",
                GITHUB_REF="refs/heads/main",
                GITHUB_EVENT_NAME="schedule",
            ),
            patch.object(nightly, "run") as command,
            self.assertRaisesRegex(ValueError, "official repository"),
        ):
            nightly.publish(self.dist, self.dist / "image.tar.gz")
        command.assert_not_called()


class ReleaseContractTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.dist = Path(self.directory.name)
        self.contract = nightly.release_contract
        for name in nightly.ASSETS:
            (self.dist / name).write_bytes((name + "-exact-source").encode())
        nightly.prepare(self.dist, SHA)
        (self.dist / self.contract.PROVENANCE).write_bytes(b"fixture verifier boundary")
        self.raw = (self.dist / self.contract.MANIFEST).read_bytes()
        self.manifest = self.contract.decode(self.raw)

    def test_producer_binds_all_bytes_and_actual_source_versions_contracts(self):
        value = self.manifest.value()
        self.assertEqual(value["repository"], "Squid-Proxy-Lovers/ccp")
        self.assertEqual(value["source_sha"], SHA)
        self.assertEqual(value["package_version"], "0.1.0")
        self.assertEqual(
            value["contracts"],
            {
                "native_wire": 1,
                "workspace_api": 1,
                "client_profile_read_min": 1,
                "client_profile_write": 5,
                "server_schema_write": 23,
            },
        )
        self.assertEqual(set(value["artifacts"]), set(nightly.ASSETS) | {"build-info.json"})
        for name, item in self.manifest.artifacts.items():
            self.assertEqual(
                item.sha256, hashlib.sha256((self.dist / name).read_bytes()).hexdigest()
            )
            self.assertEqual(item.size, (self.dist / name).stat().st_size)
        self.assertIn(self.contract.MANIFEST, (self.dist / "SHA256SUMS").read_text())

    def test_strict_manifest_rejects_unknown_duplicate_future_and_claimed_sources(self):
        original = json.loads(self.raw)
        for key, value in [
            ("format", True),
            ("format", 2),
            ("repository", "attacker/repo"),
            ("source_sha", "a" * 39),
            ("source_sha", "A" * 40),
            ("url", "https://instance.invalid/download"),
            ("platforms", ["windows"]),
            ("package_version", "--source-ref=attacker"),
        ]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                self.contract.decode(json.dumps({**original, key: value}).encode())
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            self.contract.decode(self.raw.replace(b'"format": 1', b'"format": 1, "format": 1'))
        with self.assertRaises(ValueError):
            self.contract.decode(b"x" * (65536 + 1))
        for raw in [b"\xff", b"[" * 2000 + b"]" * 2000]:
            with self.assertRaises(ValueError):
                self.contract.decode(raw)

    def test_nested_contract_and_artifact_numbers_hashes_names_are_bounded(self):
        for path, value in [
            (("contracts", "workspace_api"), True),
            (("contracts", "client_profile_write"), 0),
            (("contracts", "native_wire"), 2**32),
            (("contracts", "extra"), 1),
            (("contracts", "client_profile_read_min"), 6),
            (("artifacts", "ccp-mcp.tar.gz", "size"), -1),
            (("artifacts", "ccp-mcp.tar.gz", "size"), self.contract.MAX_ARTIFACT + 1),
            (("artifacts", "ccp-mcp.tar.gz", "sha256"), "f" * 63),
            (("artifacts", "../../client"), {"size": 1, "sha256": "a" * 64}),
        ]:
            data = json.loads(self.raw)
            parent = data
            for part in path[:-1]:
                parent = parent[part]
            parent[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.contract.decode(json.dumps(data).encode())

    def test_release_selection_pins_stable_version_or_main_nightly_source(self):
        self.assertEqual(
            self.contract.source_ref("stable", "v0.1.0", self.manifest), "refs/tags/v0.1.0"
        )
        self.assertEqual(self.contract.source_ref("nightly", TAG, self.manifest), "refs/heads/main")
        for channel, tag in [
            ("stable", "latest"),
            ("stable", "v0.2.0"),
            ("nightly", "nightly"),
            ("nightly", TAG.replace("a" * 12, "b" * 12)),
            ("custom", TAG),
        ]:
            with self.assertRaises(ValueError):
                self.contract.source_ref(channel, tag, self.manifest)

    def test_verifier_enforces_exact_publisher_ref_commit_workflow_and_hosted_runner(self):
        captured = []

        def verified(command, **options):
            captured.append(command)
            self.assertEqual(command[:3], ["gh", "attestation", "verify"])
            path = Path(command[3])
            self.assertEqual(path.read_bytes(), self.raw)
            self.assertNotEqual(path.parent, self.dist)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(
                Path(command[command.index("--bundle") + 1]).stat().st_mode & 0o777, 0o600
            )
            self.assertTrue(options["check"] and options["capture_output"])
            self.assertEqual(options["timeout"], 60)
            return subprocess.CompletedProcess(command, 0)

        with patch.object(self.contract.subprocess, "run", side_effect=verified):
            result = self.contract.verify(self.dist, "nightly", TAG, "darwin-aarch64")
        self.assertEqual(result, self.manifest)
        self.assertEqual(len(captured), 1)
        command = captured[0]
        for flag, value in [
            ("--repo", "Squid-Proxy-Lovers/ccp"),
            ("--hostname", "github.com"),
            (
                "--cert-identity",
                "https://github.com/Squid-Proxy-Lovers/ccp/.github/workflows/ci.yml@refs/heads/main",
            ),
            ("--cert-oidc-issuer", "https://token.actions.githubusercontent.com"),
            ("--source-ref", "refs/heads/main"),
            ("--source-digest", SHA),
            ("--signer-digest", SHA),
        ]:
            self.assertEqual(command[command.index(flag) + 1], value)
        self.assertIn("--deny-self-hosted-runners", command)
        self.assertNotIn(
            "--signer-workflow", command, "exact certificate identity excludes prefix signer policy"
        )
        self.assertFalse(Path(command[3]).exists(), "private verification copies are removed")

    def test_failed_missing_or_timed_out_provenance_never_falls_back_to_checksums(self):
        before = {p.name: p.read_bytes() for p in self.dist.iterdir()}
        for error in [
            FileNotFoundError("PRIVATE_VERIFIER"),
            subprocess.CalledProcessError(1, "gh", stderr=b"PRIVATE_VERIFIER"),
            subprocess.TimeoutExpired("gh", 60),
        ]:
            with (
                patch.object(self.contract.subprocess, "run", side_effect=error),
                self.assertRaises(ValueError) as caught,
            ):
                self.contract.verify(self.dist, "stable", "v0.1.0", "linux-x86_64")
            self.assertNotIn("PRIVATE_VERIFIER", str(caught.exception))
            self.assertIn("no checksum fallback", str(caught.exception))
        self.assertEqual({p.name: p.read_bytes() for p in self.dist.iterdir()}, before)
        (self.dist / self.contract.PROVENANCE).unlink()
        with patch.object(self.contract.subprocess, "run") as verifier, self.assertRaises(OSError):
            self.contract.verify(self.dist, "stable", "v0.1.0", "linux-x86_64")
        verifier.assert_not_called()

    def test_corrupt_pair_or_symlink_is_refused_even_after_publisher_verification(self):
        with patch.object(self.contract.subprocess, "run"):
            (self.dist / "ccp-mcp.tar.gz").write_bytes(b"wrong MCP revision")
            with self.assertRaisesRegex(ValueError, "differs"):
                self.contract.verify(self.dist, "nightly", TAG, "linux-x86_64")
        (self.dist / "ccp-mcp.tar.gz").unlink()
        (self.dist / "ccp-mcp.tar.gz").symlink_to(self.dist / "build-info.json")
        with (
            patch.object(self.contract.subprocess, "run"),
            self.assertRaisesRegex(ValueError, "Invalid artifact"),
        ):
            self.contract.verify(self.dist, "nightly", TAG, "linux-x86_64")

    def test_manifest_or_bundle_path_substitution_and_unknown_platform_refuse_before_verifier(self):
        for name in [self.contract.MANIFEST, self.contract.PROVENANCE]:
            path = self.dist / name
            saved = path.read_bytes()
            path.unlink()
            path.symlink_to(self.dist / "build-info.json")
            with (
                patch.object(self.contract.subprocess, "run") as verifier,
                self.assertRaises(ValueError),
            ):
                self.contract.verify(self.dist, "nightly", TAG, "linux-x86_64")
            verifier.assert_not_called()
            path.unlink()
            path.write_bytes(saved)
        with (
            patch.object(self.contract.subprocess, "run") as verifier,
            self.assertRaises(ValueError),
        ):
            self.contract.verify(self.dist, "nightly", TAG, "unsupported")
        verifier.assert_not_called()

    def test_producer_rejects_mismatched_package_versions_and_missing_constants(self):
        other = self.dist / "source"
        for name in ["client", "server", "protocol"]:
            path = other / "crates" / name / "Cargo.toml"
            path.parent.mkdir(parents=True)
            path.write_text('version = "0.1.0"\n')
        path = other / "mcp/pyproject.toml"
        path.parent.mkdir()
        path.write_text('version = "0.2.0"\n')
        with self.assertRaisesRegex(ValueError, "versions differ"):
            self.contract.source_version(other)
        path.write_text("const SOMETHING_ELSE: u32 = 5;\n")
        with self.assertRaisesRegex(ValueError, "source contract"):
            self.contract.source_number(other, "mcp/pyproject.toml", "MISSING")

    def test_provenance_publication_has_no_self_download_and_both_publishers_get_the_bundle(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        jobs = dict(re.findall(r"^  ([a-z-]+):\n(.*?)(?=^  [a-z-]+:\n|\Z)", workflow, re.M | re.S))
        proof = jobs["provenance"]
        self.assertNotIn("name: release-provenance\n          path: dist\n", proof)
        self.assertIn("path: dist/ccp-provenance.json", proof)
        for name in ["nightly", "release"]:
            self.assertIn("provenance]", jobs[name].splitlines()[0])
            self.assertIn("name: release-provenance\n          path: dist\n", jobs[name])
        for name, body in jobs.items():
            if name != "provenance":
                self.assertNotIn("id-token: write", body)
                self.assertNotIn("attestations: write", body)


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        self.home_dir = self.root / "home"
        self.install_dir = self.root / "installed"
        self.install_dir.mkdir()
        self.fixtures = self.root / "fixtures"
        self.fixtures.mkdir()
        self.log = self.root / "commands.jsonl"
        self.installer = self.root / "install.sh"
        shutil.copyfile(ROOT / "install.sh", self.installer)
        for name in ("ccp-client-linux-x86_64", "ccp-server-linux-x86_64", "ccp-mcp.tar.gz"):
            (self.fixtures / name).write_bytes(name.encode())
        (self.fixtures / "build-info.json").write_text(json.dumps({"source_sha": SHA}))
        names = [path.name for path in self.fixtures.iterdir()]
        (self.fixtures / "SHA256SUMS").write_text(
            "".join(
                f"{hashlib.sha256((self.fixtures / name).read_bytes()).hexdigest()}  {name}\n"
                for name in names
            )
        )
        # A stable release and a draft are deliberately ahead of the valid nightly.
        stable = dict(release(), tag_name="v0.1.0", prerelease=False)
        (self.fixtures / "releases-1.json").write_text(json.dumps([stable, release(draft=True)]))
        (self.fixtures / "releases-2.json").write_text(json.dumps([release()]))
        stub = """#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
from urllib.parse import urlparse, parse_qs
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['NIGHTLY_TEST_LOG'], 'a') as log:
    log.write(json.dumps([name, *args]) + '\\n')
if name == 'uname':
    print('Linux' if args == ['-s'] else 'x86_64')
elif name == 'curl':
    url = next(arg for arg in args if arg.startswith('https://'))
    parsed = urlparse(url)
    fixture = pathlib.Path(parsed.path).name
    if fixture == 'releases':
        fixture = 'releases-' + parse_qs(parsed.query)['page'][0] + '.json'
    if fixture == os.environ.get('NIGHTLY_TEST_FAILURE'):
        sys.exit(1)
    shutil.copyfile(pathlib.Path(os.environ['NIGHTLY_TEST_FIXTURES']) / fixture,
                    args[args.index('-o') + 1])
elif name == 'git':
    sys.exit('Unexpected source clone')
elif name == 'pip' and os.environ.get('NIGHTLY_TEST_FAILURE') == 'pip':
    sys.exit(1)
"""
        for command in ("curl", "uname", "docker", "git"):
            path = self.bin_dir / command
            path.write_text(stub)
            path.chmod(0o755)
        pip = self.home_dir / ".ccp-mcp/venv/bin/pip"
        pip.parent.mkdir(parents=True)
        pip.write_text(stub)
        pip.chmod(0o755)

    def install(self, *args, failure=""):
        env = dict(
            os.environ,
            HOME=str(self.home_dir),
            TMPDIR=str(self.root),
            PATH=f"{self.bin_dir}:{os.environ['PATH']}",
            NIGHTLY_TEST_LOG=str(self.log),
            NIGHTLY_TEST_FIXTURES=str(self.fixtures),
            NIGHTLY_TEST_FAILURE=failure,
        )
        argv = ["bash", str(self.installer), "--install-dir", str(self.install_dir), *args]
        # The real installer runs against isolated download, Docker, Git and pip doubles.
        result = subprocess.run(  # noqa: S603
            argv, cwd=self.root, env=env, capture_output=True, text=True, start_new_session=True
        )
        calls = (
            [json.loads(line) for line in self.log.read_text().splitlines()]
            if self.log.exists()
            else []
        )
        return result, calls

    def test_nightly_discovery_paginates_and_installs_matching_mcp(self):
        result, calls = self.install("--client", "--channel", "nightly")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.install_dir / "ccp-client").read_bytes(), b"ccp-client-linux-x86_64")
        self.assertFalse((self.install_dir / "ccp-server").exists())
        self.assertEqual(
            json.loads((self.install_dir / "ccp-nightly.json").read_text())["source_sha"], SHA
        )
        downloads = [call[2] for call in calls if call[0] == "curl"]
        self.assertTrue(any("page=2" in url for url in downloads))
        self.assertTrue(all(f"/download/{TAG}/" in url for url in downloads if "/download/" in url))
        pip = next(call for call in calls if call[0] == "pip")
        self.assertIn("--force-reinstall", pip)
        self.assertTrue(pip[-1].endswith("/ccp-mcp.tar.gz"))
        self.assertFalse(Path(pip[-1]).exists(), "staged downloads must be removed")
        self.assertFalse(any(call[0] == "git" for call in calls))

    def test_pinned_nightly_skips_discovery(self):
        result, calls = self.install("--client", "--channel", "nightly", "--version", TAG)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any("api.github.com" in str(call) for call in calls))

    def test_default_binary_install_still_uses_stable(self):
        (self.home_dir / ".ccp-mcp/src").mkdir()
        (self.install_dir / "ccp-nightly.json").write_text(json.dumps({"source_sha": SHA}))
        result, calls = self.install("--client")
        self.assertEqual(result.returncode, 0, result.stderr)
        downloads = [call[2] for call in calls if call[0] == "curl"]
        self.assertEqual(len(downloads), 1)
        self.assertIn("/releases/latest/download/ccp-client-linux-x86_64", downloads[0])
        self.assertFalse((self.install_dir / "ccp-nightly.json").exists())

    def test_no_nightly_fails_without_installing(self):
        (self.fixtures / "releases-2.json").write_text("[]")
        result, calls = self.install("--client", "--channel", "nightly")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No published nightly", result.stdout)
        self.assertFalse((self.install_dir / "ccp-client").exists())
        self.assertFalse(any(call[0] == "pip" for call in calls))

    def test_checksum_failure_preserves_binaries_and_skips_mcp(self):
        (self.install_dir / "ccp-client").write_bytes(b"working installation")
        (self.fixtures / "ccp-client-linux-x86_64").write_bytes(b"changed download")
        result, calls = self.install("--client", "--channel", "nightly", "--version", TAG)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("verification failed", result.stdout)
        self.assertEqual((self.install_dir / "ccp-client").read_bytes(), b"working installation")
        self.assertFalse(any(call[0] == "pip" for call in calls))

    def test_server_checksum_failure_preserves_both_binaries(self):
        for binary in ("ccp-client", "ccp-server"):
            (self.install_dir / binary).write_bytes(b"working installation")
        (self.fixtures / "ccp-server-linux-x86_64").write_bytes(b"changed server")
        result, _ = self.install("--channel", "nightly", "--version", TAG)
        self.assertNotEqual(result.returncode, 0)
        for binary in ("ccp-client", "ccp-server"):
            self.assertEqual((self.install_dir / binary).read_bytes(), b"working installation")

    def test_self_consistent_artifacts_from_wrong_commit_are_rejected(self):
        metadata = self.fixtures / "build-info.json"
        previous = hashlib.sha256(metadata.read_bytes()).hexdigest()
        metadata.write_text(json.dumps({"source_sha": "b" * 40}))
        updated = hashlib.sha256(metadata.read_bytes()).hexdigest()
        checksums = self.fixtures / "SHA256SUMS"
        checksums.write_text(checksums.read_text().replace(previous, updated))
        result, _ = self.install("--client", "--channel", "nightly", "--version", TAG)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("metadata does not match", result.stderr)
        self.assertFalse((self.install_dir / "ccp-client").exists())

    def test_missing_archive_preserves_existing_installation(self):
        (self.install_dir / "ccp-client").write_bytes(b"working installation")
        result, calls = self.install(
            "--client", "--channel", "nightly", "--version", TAG, failure="ccp-mcp.tar.gz"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.install_dir / "ccp-client").read_bytes(), b"working installation")
        self.assertFalse(any(call[0] == "pip" for call in calls))

    def test_mcp_failure_does_not_report_completed_installation(self):
        result, _ = self.install(
            "--client", "--channel", "nightly", "--version", TAG, failure="pip"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Done.", result.stdout)

    def test_docker_channel_and_version_selection(self):
        for args, tag in (
            ((), "latest"),
            (("--channel", "nightly"), "nightly"),
            (("--channel", "nightly", "--version", TAG), TAG),
        ):
            with self.subTest(tag=tag):
                self.log.unlink(missing_ok=True)
                result, calls = self.install("--docker", *args)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls[-1][-1], f"ghcr.io/squid-proxy-lovers/ccp:{tag}")

    def test_invalid_channel_options_do_not_start_installation(self):
        for args in (
            ("--channel", "other"),
            ("--channel",),
            ("--version", TAG),
            ("--channel", "nightly", "--from-source"),
            ("--channel", "nightly", "--version", "nightly; echo bad"),
        ):
            with self.subTest(args=args):
                result, calls = self.install(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
