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
from dataclasses import replace
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

from managed_bundle_tests import ManagedBundleTests as ManagedBundleTests
from managed_install_tests import ManagedInstallTests as ManagedInstallTests
from managed_install_tests import ManagedOperationalTests as ManagedOperationalTests
from managed_update_tests import ManagedNetworkTests as ManagedNetworkTests
from managed_update_tests import ManagedUpdateTests as ManagedUpdateTests

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

    def test_publication_requires_exact_producer_platforms_and_assets(self):
        path = self.dist / nightly.release_contract.MANIFEST
        original = json.loads(path.read_bytes())
        for change in ("order", "platform", "extra-asset", "missing-asset"):
            data = json.loads(json.dumps(original))
            if change == "order":
                data["platforms"].reverse()
            elif change == "platform":
                data["platforms"].append("windows-x86_64")
            elif change == "extra-asset":
                data["artifacts"]["release-notes.txt"] = {"size": 1, "sha256": "a" * 64}
            else:
                del data["artifacts"]["ccp-server-linux-x86_64"]
            path.write_text(json.dumps(data))
            names = (*nightly.ASSETS, "build-info.json", nightly.release_contract.MANIFEST)
            (self.dist / "SHA256SUMS").write_text(
                "".join(f"{nightly.digest(self.dist / name)}  {name}\n" for name in names)
            )
            reason = (
                "Producer release platforms differ"
                if change in ("order", "platform")
                else "Producer release artifacts differ"
            )
            with (
                self.subTest(change=change),
                self.assertRaisesRegex(ValueError, "^" + reason + "$"),
            ):
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
        self.version = self.contract.source_version(ROOT)
        self.tag = "v" + self.version
        self.channel = "rc" if "-rc." in self.version else "stable"
        for name in nightly.ASSETS:
            (self.dist / name).write_bytes((name + "-exact-source").encode())
        nightly.prepare(self.dist, SHA)
        (self.dist / self.contract.PROVENANCE).write_bytes(b"fixture verifier boundary")
        self.raw = (self.dist / self.contract.MANIFEST).read_bytes()
        self.manifest = self.contract.decode(self.raw)

    def future_manifest(self):
        data = json.loads(self.raw)
        data["platforms"].append("windows-x86_64")
        item = {"size": 1, "sha256": "a" * 64}
        data["artifacts"]["ccp-client-windows-x86_64.exe"] = item
        data["artifacts"]["release-notes.txt"] = item
        return data

    def pre_compatibility_output(self, root=ROOT):
        # Independent reconstruction of 2b945c1's inventory and encoding rules.
        platforms = ["linux-x86_64", "linux-aarch64", "darwin-x86_64", "darwin-aarch64"]
        assets = [
            f"ccp-{binary}-{platform}" for platform in platforms for binary in ("client", "server")
        ] + ["ccp-mcp.tar.gz"]
        toolchain = re.search(
            r'^channel\s*=\s*"([^"]+)"', (root / "rust-toolchain.toml").read_text(), re.M
        ).group(1)
        build = {
            "source_sha": SHA,
            "rust_toolchain": toolchain,
            "platforms": platforms,
            "artifacts": assets,
        }
        output = {"build-info.json": (json.dumps(build, indent=2) + "\n").encode()}
        numbers = {
            "native_wire": ("crates/protocol/src/lib.rs", "PROTOCOL_VERSION"),
            "workspace_api": ("crates/protocol/src/workspace.rs", "API_VERSION"),
            "client_profile_read_min": (
                "crates/client/src/workspace/store.rs",
                "PROFILE_MIN_FORMAT",
            ),
            "client_profile_write": ("crates/client/src/workspace/store.rs", "PROFILE_FORMAT"),
            "server_schema_write": ("crates/server/src/init.rs", "SCHEMA_VERSION"),
        }
        bodies = {name: (self.dist / name).read_bytes() for name in assets}
        bodies.update(output)
        manifest = {
            "format": 1,
            "repository": "Squid-Proxy-Lovers/ccp",
            "source_sha": SHA,
            "package_version": self.contract.source_version(root),
            "platforms": platforms,
            "contracts": {
                key: self.contract.source_number(root, *source) for key, source in numbers.items()
            },
            "artifacts": {
                name: {"size": len(body), "sha256": hashlib.sha256(body).hexdigest()}
                for name, body in bodies.items()
            },
        }
        output["ccp-release.json"] = (
            json.dumps(manifest, sort_keys=True, indent=2) + "\n"
        ).encode()
        bodies["ccp-release.json"] = output["ccp-release.json"]
        output["SHA256SUMS"] = "".join(
            f"{hashlib.sha256(body).hexdigest()}  {name}\n" for name, body in bodies.items()
        ).encode()
        return output

    def test_producer_bytes_match_pre_forward_compatibility_output(self):
        for name, expected in self.pre_compatibility_output().items():
            with self.subTest(name=name):
                self.assertEqual((self.dist / name).read_bytes(), expected)

    def test_producer_byte_identity_survives_package_and_contract_version_bumps(self):
        source = self.dist / "release-source"
        files = {
            "crates/client/Cargo.toml",
            "crates/server/Cargo.toml",
            "crates/protocol/Cargo.toml",
            "mcp/pyproject.toml",
            "rust-toolchain.toml",
            "crates/protocol/src/lib.rs",
            "crates/protocol/src/workspace.rs",
            "crates/client/src/workspace/store.rs",
            "crates/server/src/init.rs",
        }
        for name in files:
            target = source / name
            target.parent.mkdir(parents=True, exist_ok=True)
            raw = (ROOT / name).read_text()
            raw = re.sub(r'^version = "[^"]+"$', 'version = "0.3.0"', raw, flags=re.M)
            raw = re.sub(
                r"(const (?:PROTOCOL_VERSION|API_VERSION|PROFILE_MIN_FORMAT|PROFILE_FORMAT|SCHEMA_VERSION): u32 = )([0-9]+)",
                lambda match: match[1] + str(int(match[2]) + 1),
                raw,
            )
            target.write_text(raw)
        with patch.object(nightly, "ROOT", source):
            nightly.prepare(self.dist, SHA)
        self.assertEqual(
            self.contract.decode((self.dist / self.contract.MANIFEST).read_bytes()).package_version,
            "0.3.0",
        )
        for name, expected in self.pre_compatibility_output(source).items():
            with self.subTest(name=name):
                self.assertEqual((self.dist / name).read_bytes(), expected)

    def test_future_client_only_platform_and_assets_verify_for_every_known_platform(self):
        data = self.future_manifest()
        raw = json.dumps(data).encode()
        (self.dist / self.contract.MANIFEST).write_bytes(raw)
        for platform in self.contract.PLATFORMS:
            with (
                self.subTest(platform=platform),
                patch.object(self.contract.subprocess, "run") as proof,
            ):
                result = self.contract.verify(self.dist, "nightly", TAG, platform)
                self.assertEqual(result.value(), data)
                self.assertEqual(proof.call_count, 1)
        with (
            patch.object(self.contract.subprocess, "run") as proof,
            self.assertRaisesRegex(ValueError, "^Unsupported client platform$"),
        ):
            self.contract.verify(self.dist, "nightly", TAG, "windows-x86_64")
        proof.assert_not_called()

    def test_checkout_cli_preserves_future_platforms_in_verified_output(self):
        data = self.future_manifest()
        (self.dist / self.contract.MANIFEST).write_text(json.dumps(data))
        with (
            patch.object(
                sys,
                "argv",
                [
                    "release_contract.py",
                    "--directory",
                    str(self.dist),
                    "--channel",
                    "nightly",
                    "--tag",
                    TAG,
                    "--platform",
                    "linux-x86_64",
                ],
            ),
            patch.object(self.contract.subprocess, "run"),
            patch("builtins.print") as output,
        ):
            self.contract.main()
        self.assertEqual(json.loads(output.call_args.args[0]), data)

    def test_future_manifest_keeps_provenance_source_and_version_policy_strict(self):
        data = self.future_manifest()
        path = self.dist / self.contract.MANIFEST
        path.write_text(json.dumps(data))
        with (
            patch.object(self.contract.subprocess, "run", side_effect=FileNotFoundError),
            self.assertRaisesRegex(ValueError, "provenance"),
        ):
            self.contract.verify(self.dist, "nightly", TAG, "linux-x86_64")
        for channel, tag, reason in (
            ("stable", "v0.3.0", "Tag and package version differ"),
            ("nightly", TAG.replace("a" * 12, "b" * 12), "Nightly tag and source differ"),
            ("custom", TAG, "Unsupported release channel"),
        ):
            with (
                self.subTest(channel=channel),
                patch.object(self.contract.subprocess, "run") as proof,
                self.assertRaisesRegex(ValueError, "^" + reason + "$"),
            ):
                self.contract.verify(self.dist, channel, tag, "linux-x86_64")
            proof.assert_not_called()
        for key, value, reason in (
            ("repository", "attacker/repo", "Unexpected release source"),
            ("source_sha", "b" * 40, "Nightly tag and source differ"),
            ("package_version", "malformed", "Invalid release identity"),
        ):
            path.write_text(json.dumps({**data, key: value}))
            with (
                self.subTest(key=key),
                patch.object(self.contract.subprocess, "run") as proof,
                self.assertRaisesRegex(ValueError, "^" + reason + "$"),
            ):
                self.contract.verify(self.dist, "nightly", TAG, "linux-x86_64")
            proof.assert_not_called()

    def test_future_asset_payloads_are_opaque_and_never_verified_as_installable_files(self):
        future = "ccp-client-windows-x86_64.exe"
        for payload in (
            {"size": self.contract.MAX_ARTIFACT + 1, "sha256": "a" * 64},
            {"size": 0, "sha256": "a" * 64},
            {"url": "https://attacker.invalid/client", "signature": {"future-format": 2}},
            {"size": "unknown", "sha256": None, "url": "file:///outside"},
            None,
        ):
            data = self.future_manifest()
            data["artifacts"][future] = payload
            (self.dist / self.contract.MANIFEST).write_text(json.dumps(data))
            with (
                self.subTest(payload=payload),
                patch.object(self.contract.subprocess, "run") as proof,
            ):
                manifest = self.contract.verify(self.dist, "nightly", TAG, "linux-x86_64")
                self.assertEqual(manifest.value(), data)
                self.assertNotIn(future, manifest.artifacts)
                self.assertEqual(proof.call_count, 1)
                with self.assertRaisesRegex(ValueError, "^Unexpected release artifact$"):
                    manifest.verify_files(self.dist, (future,))

    def test_known_asset_payloads_and_format_one_envelope_remain_strict(self):
        for name in self.contract.ASSETS:
            for payload, reason in (
                ({"size": 0, "sha256": "a" * 64}, "Release integer out of bounds"),
                (
                    {"size": self.contract.MAX_ARTIFACT + 1, "sha256": "a" * 64},
                    "Release integer out of bounds",
                ),
                (
                    {"size": 1, "sha256": "a" * 64, "url": "https://attacker.invalid"},
                    "Invalid release fields",
                ),
                ({"size": 1, "sha256": "a" * 64, "signature": "unused"}, "Invalid release fields"),
                ({"size": 1, "sha256": "bad"}, "Invalid release identity"),
            ):
                data = self.future_manifest()
                data["artifacts"][name] = payload
                with (
                    self.subTest(name=name, payload=payload),
                    self.assertRaisesRegex(ValueError, "^" + reason + "$"),
                ):
                    self.contract.decode(json.dumps(data).encode())
        for scope in ("top-level", "contracts"):
            data = self.future_manifest()
            parent = data if scope == "top-level" else data["contracts"]
            parent["future_key"] = {"url": "https://attacker.invalid"}
            with (
                self.subTest(scope=scope),
                self.assertRaisesRegex(ValueError, "^Invalid release fields$"),
            ):
                self.contract.decode(json.dumps(data).encode())

    def test_opaque_future_metadata_stays_bound_to_manifest_integrity_and_duplicate_checks(self):
        data = self.future_manifest()
        data["artifacts"]["release-notes.txt"] = {"url": "https://attacker.invalid/first"}
        raw = json.dumps(data).encode()
        manifest = self.contract.decode(raw)
        data["artifacts"]["release-notes.txt"]["url"] = "https://attacker.invalid/second"
        (self.dist / self.contract.MANIFEST).write_text(json.dumps(data))
        with (
            patch.object(self.contract.subprocess, "run") as proof,
            self.assertRaisesRegex(ValueError, "^Release manifest changed$"),
        ):
            self.contract.verify_provenance(self.dist, manifest, "refs/heads/main")
        proof.assert_not_called()
        for duplicate in (
            raw.replace(b'"url":', b'"url": null, "url":'),
            raw.replace(b'"release-notes.txt":', b'"ccp-mcp.tar.gz":'),
            raw.replace(b'"release-notes.txt":', b'"ccp-client-windows-x86_64.exe":'),
        ):
            with (
                self.subTest(raw=duplicate),
                self.assertRaisesRegex(ValueError, "^Duplicate release field$"),
            ):
                self.contract.decode(duplicate)

    def test_opaque_metadata_type_substitution_cannot_bypass_manifest_integrity(self):
        for before, after in ((True, 1), (1, 1.0), ({"flag": False}, {"flag": 0})):
            data = self.future_manifest()
            data["artifacts"]["release-notes.txt"] = before
            manifest = self.contract.decode(json.dumps(data).encode())
            data["artifacts"]["release-notes.txt"] = after
            (self.dist / self.contract.MANIFEST).write_text(json.dumps(data))
            with (
                self.subTest(before=before, after=after),
                patch.object(self.contract.subprocess, "run") as proof,
                self.assertRaisesRegex(ValueError, "^Release manifest changed$"),
            ):
                self.contract.verify_provenance(self.dist, manifest, "refs/heads/main")
            proof.assert_not_called()

    def test_ascii_multicomponent_platform_names_and_non_ascii_rejection(self):
        data = self.future_manifest()
        data["platforms"].append("futureos-x86_64-v3-musl")
        self.assertEqual(self.contract.decode(json.dumps(data).encode()).value(), data)
        for platform in ("9os-x86_64", "línux-x86_64", "linux-ｘ86_64", "linux-x86__64"):
            data = self.future_manifest()
            data["platforms"].append(platform)
            with (
                self.subTest(platform=platform),
                self.assertRaisesRegex(ValueError, "^Invalid release identity$"),
            ):
                self.contract.decode(json.dumps(data).encode())
        for name in ("rélease.txt", "ccp-client-ｗindows.exe"):
            data = self.future_manifest()
            data["artifacts"][name] = None
            with (
                self.subTest(asset=name),
                self.assertRaisesRegex(ValueError, "^Invalid release identity$"),
            ):
                self.contract.decode(json.dumps(data).encode())

    def test_client_assets_directly_refuses_a_declared_future_platform(self):
        manifest = self.contract.decode(json.dumps(self.future_manifest()).encode())
        with self.assertRaisesRegex(ValueError, "^Unsupported client platform$"):
            manifest.client_assets("windows-x86_64")

    def test_client_only_known_platform_does_not_require_a_server(self):
        data = json.loads(self.raw)
        del data["artifacts"]["ccp-server-linux-x86_64"]
        (self.dist / self.contract.MANIFEST).write_text(json.dumps(data))
        with patch.object(self.contract.subprocess, "run"):
            self.contract.verify(self.dist, "nightly", TAG, "linux-x86_64")

    def test_future_manifest_still_rejects_missing_local_assets_and_tampered_hashes(self):
        for platform in self.contract.PLATFORMS:
            for name in (f"ccp-client-{platform}", "ccp-mcp.tar.gz", "build-info.json"):
                for change in ("missing", "hash"):
                    data = self.future_manifest()
                    if change == "missing":
                        del data["artifacts"][name]
                    else:
                        data["artifacts"][name]["sha256"] = "0" * 64
                    (self.dist / self.contract.MANIFEST).write_text(json.dumps(data))
                    with (
                        self.subTest(platform=platform, name=name, change=change),
                        patch.object(self.contract.subprocess, "run"),
                        self.assertRaisesRegex(
                            ValueError,
                            "^Missing client release artifact$"
                            if change == "missing"
                            else "^Release artifact differs from manifest$",
                        ),
                    ):
                        self.contract.verify(self.dist, "nightly", TAG, platform)

    def test_future_manifest_rejects_duplicate_and_malformed_entries(self):
        for name in ("../windows-x86_64", "Windows-x86_64", "windows", "windows-x86_64\n", 1):
            data = self.future_manifest()
            data["platforms"].append(name)
            with (
                self.subTest(platform=name),
                self.assertRaisesRegex(
                    ValueError,
                    "^Invalid release text$"
                    if isinstance(name, int)
                    else "^Invalid release identity$",
                ),
            ):
                self.contract.decode(json.dumps(data).encode())
        data = self.future_manifest()
        data["platforms"].append("linux-x86_64")
        with self.assertRaisesRegex(ValueError, "^Duplicate release platform$"):
            self.contract.decode(json.dumps(data).encode())
        for name in ("../client", ".client", "client..exe", "client/extra", "client\n", ""):
            data = self.future_manifest()
            data["artifacts"][name] = {"size": 1, "sha256": "a" * 64}
            with (
                self.subTest(asset=name),
                self.assertRaisesRegex(ValueError, "^Invalid release identity$"),
            ):
                self.contract.decode(json.dumps(data).encode())
        raw = json.dumps(self.future_manifest()).encode()
        for key, value in (
            (b'"size": 1', b'"size": 1, "size": 1'),
            (b'"release-notes.txt":', b'"ccp-mcp.tar.gz":'),
        ):
            with self.assertRaisesRegex(ValueError, "Duplicate"):
                self.contract.decode(raw.replace(key, value))

    def test_platform_list_changes_remain_part_of_manifest_integrity(self):
        manifest = self.contract.decode(self.raw)
        data = json.loads(self.raw)
        data["platforms"].reverse()
        (self.dist / self.contract.MANIFEST).write_text(json.dumps(data))
        with (
            patch.object(self.contract.subprocess, "run") as proof,
            self.assertRaisesRegex(ValueError, "manifest changed"),
        ):
            self.contract.verify_provenance(self.dist, manifest, "refs/heads/main")
        proof.assert_not_called()

    def test_producer_binds_all_bytes_and_actual_source_versions_contracts(self):
        value = self.manifest.value()
        self.assertEqual(value["repository"], "Squid-Proxy-Lovers/ccp")
        self.assertEqual(value["source_sha"], SHA)
        self.assertEqual(value["package_version"], self.version)
        self.assertEqual(
            value["contracts"],
            {
                "native_wire": 1,
                "workspace_api": 1,
                "client_profile_read_min": 1,
                "client_profile_write": 5,
                "server_schema_write": 27,
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
        stable_version = self.version.split("-", 1)[0]
        stable = replace(self.manifest, package_version=stable_version)
        stable_tag = "v" + stable_version
        self.assertEqual(
            self.contract.source_ref("stable", stable_tag, stable), "refs/tags/" + stable_tag
        )
        self.assertEqual(
            self.contract.source_ref(self.channel, self.tag, self.manifest), "refs/tags/" + self.tag
        )
        self.assertEqual(self.contract.source_ref("nightly", TAG, self.manifest), "refs/heads/main")
        for channel, tag in [
            ("stable", "latest"),
            ("stable", "v0.3.0"),
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
                self.contract.verify(self.dist, self.channel, self.tag, "linux-x86_64")
            self.assertNotIn("PRIVATE_VERIFIER", str(caught.exception))
            self.assertIn("no checksum fallback", str(caught.exception))
        self.assertEqual({p.name: p.read_bytes() for p in self.dist.iterdir()}, before)
        (self.dist / self.contract.PROVENANCE).unlink()
        with patch.object(self.contract.subprocess, "run") as verifier, self.assertRaises(OSError):
            self.contract.verify(self.dist, self.channel, self.tag, "linux-x86_64")
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
            path.write_text('version = "0.2.0"\n')
        path = other / "mcp/pyproject.toml"
        path.parent.mkdir()
        path.write_text('version = "0.3.0"\n')
        with self.assertRaisesRegex(ValueError, "versions differ"):
            self.contract.source_version(other)
        path.write_text("const SOMETHING_ELSE: u32 = 5;\n")
        with self.assertRaisesRegex(ValueError, "source contract"):
            self.contract.source_number(other, "mcp/pyproject.toml", "MISSING")

    def test_exact_rust_and_python_version_pairs(self):
        from ccp_mcp_server.release_contract import python_version

        source = self.dist / "source"
        paths = [
            source / "crates" / name / "Cargo.toml" for name in ["client", "server", "protocol"]
        ]
        paths.append(source / "mcp/pyproject.toml")
        for path in paths:
            path.parent.mkdir(parents=True, exist_ok=True)
        for rust, python in [("0.2.0", "0.2.0"), ("0.2.0-rc.1", "0.2.0rc1")]:
            for path in paths[:3]:
                path.write_text(f'version = "{rust}"\n')
            paths[3].write_text(f'version = "{python}"\n')
            self.assertEqual(self.contract.source_version(source), rust)
            self.assertEqual(python_version(rust), python)
        for python in [
            "0.2.0-rc.1",
            "0.2.0rc2",
            "0.2.1rc1",
            "0.2.0",
            "0.2.0RC1",
            "0.2.0rc01",
            "0.2.0rc1+local",
        ]:
            paths[3].write_text(f'version = "{python}"\n')
            with self.subTest(python=python), self.assertRaises(ValueError):
                self.contract.source_version(source)
        paths[3].write_text('version = "0.2.0rc1"\n')
        paths[1].write_text('version = "0.2.1-rc.1"\n')
        with self.assertRaises(ValueError):
            self.contract.source_version(source)
        for rust in ["0.2.0-rc.01", "00.2.0-rc.1", "0.2.0rc1", "0.2.0-beta.1", "0.2.0-rc.1+local"]:
            with self.subTest(rust=rust), self.assertRaises(ValueError):
                python_version(rust)

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


class ReleaseTagTests(unittest.TestCase):
    def setUp(self):
        from importlib import import_module

        self.tags = import_module("release_tag")
        self.contract = nightly.release_contract
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.jobs = dict(
            re.findall(r"^  ([a-z-]+):\n(.*?)(?=^  [a-z-]+:\n|\Z)", self.workflow, re.M | re.S)
        )

    def source(self, rust="0.2.0-rc.1", python="0.2.0rc1"):
        for name in ["client", "server", "protocol"]:
            path = self.root / "crates" / name / "Cargo.toml"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f'version = "{rust}"\n')
        path = self.root / "mcp/pyproject.toml"
        path.parent.mkdir(exist_ok=True)
        path.write_text(f'version = "{python}"\n')

    def test_classification_matches_source_and_preserves_stable_policy(self):
        for version, python, channel, prerelease, latest in [
            ("0.2.0", "0.2.0", "stable", "false", ""),
            ("0.2.0-rc.1", "0.2.0rc1", "rc", "true", "false"),
        ]:
            self.source(version, python)
            self.assertEqual(
                self.tags.classify(self.root, "v" + version),
                {
                    "tag": "v" + version,
                    "channel": channel,
                    "prerelease": prerelease,
                    "make_latest": latest,
                },
            )

    def test_unsupported_tags_and_version_mismatches_fail_closed(self):
        self.source()
        for tag in [
            "v0.2.0",
            "v0.2.0-rc.2",
            "v0.2.1-rc.1",
            "v0.2.0rc1",
            "v0.2.0-rc.01",
            "v00.2.0-rc.1",
            "v0.2.0-beta.1",
            "v0.2.0+local",
            "v0.2.0-rc.1+local",
            "latest",
            TAG,
            "v0.2.0-rc.1\n",
        ]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                self.tags.classify(self.root, tag)
        self.source(python="0.2.0rc2")
        with self.assertRaises(ValueError):
            self.tags.classify(self.root, "v0.2.0-rc.1")

    def test_cli_emits_outputs_only_after_successful_validation(self):
        self.source()
        output = self.root / "outputs"
        command = [
            sys.executable,
            str(ROOT / "scripts/release_tag.py"),
            "--root",
            str(self.root),
            "--output",
            str(output),
            "--tag",
        ]
        # Fixed checkout CLI and owned temporary source/output paths.
        success = subprocess.run([*command, "v0.2.0-rc.1"], capture_output=True, check=False)  # noqa: S603
        self.assertEqual(success.returncode, 0, success.stderr)
        self.assertEqual(
            output.read_text(), "tag=v0.2.0-rc.1\nchannel=rc\nprerelease=true\nmake_latest=false\n"
        )
        output.unlink()
        # Same CLI must not enable publication after a mismatch.
        failure = subprocess.run([*command, "v0.2.0"], capture_output=True, check=False)  # noqa: S603
        self.assertNotEqual(failure.returncode, 0)
        self.assertFalse(output.exists())

    def test_rc_source_manifest_and_verifier_contract(self):
        self.source()
        for filename in [
            "crates/protocol/src/lib.rs",
            "crates/protocol/src/workspace.rs",
            "crates/client/src/workspace/store.rs",
            "crates/server/src/init.rs",
        ]:
            path = self.root / filename
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / filename, path)
        dist = self.root / "dist"
        dist.mkdir()
        for name in self.contract.ASSETS:
            (dist / name).write_bytes(name.encode())
        self.contract.prepare(self.root, dist, SHA)
        manifest = self.contract.decode((dist / self.contract.MANIFEST).read_bytes())
        self.assertEqual(manifest.package_version, "0.2.0-rc.1")
        (dist / self.contract.PROVENANCE).write_bytes(b"mocked verifier bundle")
        with patch.object(self.contract.subprocess, "run") as verifier:
            self.assertEqual(
                self.contract.verify(dist, "rc", "v0.2.0-rc.1", "linux-x86_64"), manifest
            )
        command = verifier.call_args.args[0]
        self.assertEqual(command[command.index("--source-ref") + 1], "refs/tags/v0.2.0-rc.1")
        self.assertTrue(
            command[command.index("--cert-identity") + 1].endswith("@refs/tags/v0.2.0-rc.1")
        )
        for channel, tag in [("stable", "v0.2.0-rc.1"), ("rc", "v0.2.0"), ("rc", "v0.2.0-rc.2")]:
            with (
                self.subTest(channel=channel, tag=tag),
                patch.object(self.contract.subprocess, "run") as verifier,
                self.assertRaises(ValueError),
            ):
                self.contract.verify(dist, channel, tag, "linux-x86_64")
            verifier.assert_not_called()

    def test_publication_jobs_require_identity_and_provenance_success(self):
        for name in ["release", "docker-publish"]:
            body = self.jobs[name]
            self.assertIn("release-identity", body.splitlines()[0])
            self.assertIn("provenance", body.splitlines()[0])
            self.assertNotIn("always()", body)
        self.assertIn("release-identity", self.jobs["provenance"].splitlines()[0])
        self.assertIn(
            'python scripts/release_tag.py --tag "${GITHUB_REF#refs/tags/}" --output "$GITHUB_OUTPUT"',
            self.jobs["release-identity"],
        )
        self.assertIn(
            "CHANNEL: ${{ needs.release-identity.outputs.channel }}", self.jobs["provenance"]
        )
        self.assertIn('--channel "$CHANNEL" --tag "$RELEASE_TAG"', self.jobs["provenance"])
        self.assertIn(
            "prerelease: ${{ needs.release-identity.outputs.prerelease == 'true' }}",
            self.jobs["release"],
        )
        self.assertIn(
            "make_latest: ${{ needs.release-identity.outputs.make_latest }}", self.jobs["release"]
        )
        self.assertIn("startsWith(github.ref, 'refs/tags/v')", self.jobs["docker-publish"])

    def test_actual_docker_step_pushes_only_sha_and_channel_tag(self):
        body = self.jobs["docker-publish"]
        script = body.split("        run: |\n", 1)[1]
        script = "\n".join(line.removeprefix("          ") for line in script.splitlines())
        docker = self.root / "docker"
        docker.write_text(
            "#!"
            + sys.executable
            + "\nimport json,os,sys\nwith open(os.environ['DOCKER_LOG'],'a') as stream: stream.write(json.dumps(sys.argv[1:])+'\\n')\n"
        )
        docker.chmod(0o700)
        log = self.root / "docker-log"
        for channel, tag, expected in [
            ("stable", "v0.2.0", "latest"),
            ("rc", "v0.2.0-rc.1", "v0.2.0-rc.1"),
        ]:
            log.write_text("")
            environment = {
                **os.environ,
                "PATH": str(self.root) + os.pathsep + os.environ["PATH"],
                "GITHUB_SHA": SHA,
                "CHANNEL": channel,
                "RELEASE_TAG": tag,
                "DOCKER_LOG": str(log),
            }
            # Execute the workflow's real shell with an owned recording Docker stub.
            result = subprocess.run(  # noqa: S603 -- fixed shell and checked-in workflow, with an owned Docker stub.
                ["/bin/bash", "-euo", "pipefail", "-c", script],
                env=environment,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            pushes = [call[1] for call in calls if call[0] == "push"]
            self.assertEqual(
                pushes,
                [
                    f"ghcr.io/squid-proxy-lovers/ccp:{SHA}",
                    f"ghcr.io/squid-proxy-lovers/ccp:{expected}",
                ],
            )


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
        stable = dict(release(), tag_name="v0.2.0", prerelease=False)
        (self.fixtures / "releases-1.json").write_text(json.dumps([stable, release(draft=True)]))
        (self.fixtures / "releases-2.json").write_text(json.dumps([release()]))
        stub = (
            f"#!{sys.executable}\n"
            + """import json, os, pathlib, shutil, sys
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
        pathlib.Path(args[args.index('-o') + 1]).write_bytes(b'partial-download')
        sys.exit(1)
    shutil.copyfile(pathlib.Path(os.environ['NIGHTLY_TEST_FIXTURES']) / fixture,
                    args[args.index('-o') + 1])
elif name == 'git':
    sys.exit('Unexpected source clone')
elif name == 'cargo':
    assert args == ['build', '--release', '--locked', '-p', 'client']
elif name == 'mktemp':
    import tempfile
    root = pathlib.Path(os.environ['NIGHTLY_TEST_FIXTURES']).parent.resolve()
    if args == ['-d']:
        print(tempfile.mkdtemp(dir=root))
    else:
        target = pathlib.Path(args[0])
        assert target.parent.resolve() == root or root in target.parent.resolve().parents
        fd, value = tempfile.mkstemp(prefix=target.name.replace('XXXXXX', ''), dir=target.parent)
        os.close(fd)
        print(value)
elif name in ('cp', 'chmod', 'mv', 'mkdir'):
    root = pathlib.Path(os.environ['NIGHTLY_TEST_FIXTURES']).parent.resolve()
    for value in args:
        if not value.startswith('-') and value not in ('+x', '0755', '0644'):
            target = pathlib.Path(value).resolve()
            assert target == root or root in target.parents
    if name == os.environ.get('NIGHTLY_TEST_FAILURE'):
        if name == 'cp': pathlib.Path(args[-1]).write_bytes(b'partial-copy')
        sys.exit(1)
    if name == 'mv' and os.environ.get('NIGHTLY_TEST_FAILURE') == 'race:' + pathlib.Path(args[-1]).name:
        destination = pathlib.Path(args[-1])
        if destination.is_file(): destination.unlink()
        destination.mkdir()
    import subprocess
    subprocess.run([shutil.which(name, path='/usr/bin:/bin'), *args], check=True)
elif name == 'pip' and os.environ.get('NIGHTLY_TEST_FAILURE') == 'pip':
    sys.exit(1)
elif name == 'cat':
    # Every "cat" call in install.sh takes no positional arguments; it
    # only ever copies a heredoc on stdin to stdout (or a redirected
    # file). Failing it here simulates a write failure that no amount
    # of privilege can bypass, unlike a chmod-based permission denial.
    data = sys.stdin.buffer.read()
    if os.environ.get('NIGHTLY_TEST_FAILURE') == 'cat':
        sys.exit(1)
    sys.stdout.buffer.write(data)
"""
        )
        for command in (
            "curl",
            "uname",
            "docker",
            "git",
            "cargo",
            "cp",
            "chmod",
            "mv",
            "mkdir",
            "mktemp",
            "cat",
        ):
            path = self.bin_dir / command
            path.write_text(stub)
            path.chmod(0o755)
        pip = self.home_dir / ".ccp-mcp/venv/bin/pip"
        pip.parent.mkdir(parents=True)
        pip.write_text(stub)
        pip.chmod(0o755)

    def source_fixture(self):
        (self.root / "Cargo.toml").write_text("# private build fixture")
        (self.root / "mcp").mkdir()
        release_dir = self.root / "target/release"
        release_dir.mkdir(parents=True)
        (release_dir / "client").write_bytes(b"replacement-client")
        (release_dir / "server").write_bytes(b"replacement-server")

    def test_stable_success_preserves_readable_executable_mode(self):
        (self.root / "mcp").mkdir()
        result, _ = self.install("--client")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        target = self.install_dir / "ccp-client"
        self.assertEqual(target.read_bytes(), b"ccp-client-linux-x86_64")
        self.assertEqual(target.stat().st_mode & 0o777, 0o755)

    def test_stable_install_refuses_directory_destination(self):
        (self.root / "mcp").mkdir()
        target = self.install_dir / "ccp-client"
        target.mkdir()
        marker = target / "do-not-move-me"
        marker.write_text("placeholder")
        result, _ = self.install("--client")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Done.", result.stdout)
        self.assertTrue(target.is_dir())
        self.assertEqual(list(target.iterdir()), [marker])

    def test_nightly_install_refuses_directory_destination(self):
        (self.root / "mcp").mkdir()
        target = self.install_dir / "ccp-client"
        target.mkdir()
        marker = target / "do-not-move-me"
        marker.write_text("placeholder")
        result, _ = self.install("--client", "--channel", "nightly", "--version", TAG)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Done.", result.stdout)
        self.assertTrue(target.is_dir())
        self.assertEqual(list(target.iterdir()), [marker])

    def install_modes(self):
        self.source_fixture()
        return (
            ("stable", ()),
            ("nightly", ("--channel", "nightly", "--version", TAG)),
            ("source", ("--from-source",)),
        )

    def clear_install_dir(self):
        for path in self.install_dir.iterdir():
            if path.is_dir() and not path.is_symlink():
                shutil.rmtree(path)
            else:
                path.unlink()

    def test_directory_created_during_publish_is_rejected_and_cleaned(self):
        for mode, args in self.install_modes():
            with self.subTest(mode=mode):
                target = self.install_dir / "ccp-client"
                target.write_bytes(b"working-client")
                # The mv double replaces this file with a directory just
                # before executing the real rename.
                result, _ = self.install("--client", *args, failure="race:ccp-client")
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn("Done.", result.stdout)
                self.assertEqual(list(target.iterdir()), [])
                self.assertEqual(list(self.install_dir.glob("*.stage.*")), [])
                self.assertEqual(list(self.install_dir.glob("*.download.*")), [])
            self.clear_install_dir()

    def test_nightly_metadata_directory_is_rejected_before_binary_publish(self):
        self.source_fixture()
        target = self.install_dir / "ccp-nightly.json"
        target.mkdir()
        marker = target / "keep"
        marker.write_text("untouched")
        client = self.install_dir / "ccp-client"
        client.write_bytes(b"working-client")
        result, _ = self.install("--client", "--channel", "nightly", "--version", TAG)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Done.", result.stdout)
        self.assertEqual(list(target.iterdir()), [marker])
        self.assertEqual(client.read_bytes(), b"working-client")

    def test_nightly_metadata_directory_race_is_rejected_and_cleaned(self):
        self.source_fixture()
        result, _ = self.install(
            "--client", "--channel", "nightly", "--version", TAG, failure="race:ccp-nightly.json"
        )
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Done.", result.stdout)
        self.assertEqual(list((self.install_dir / "ccp-nightly.json").iterdir()), [])
        self.assertEqual(list(self.install_dir.glob("*.stage.*")), [])

    def test_both_mode_preflights_every_binary_destination(self):
        for mode, args in self.install_modes():
            for invalid in ("ccp-client", "ccp-server"):
                with self.subTest(mode=mode, invalid=invalid):
                    target = self.install_dir / invalid
                    target.mkdir()
                    other = self.install_dir / (
                        "ccp-server" if invalid == "ccp-client" else "ccp-client"
                    )
                    other.write_bytes(b"working-binary")
                    result, _ = self.install(*args)
                    self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertNotIn("Done.", result.stdout)
                    self.assertEqual(other.read_bytes(), b"working-binary")
                    self.assertEqual(list(target.iterdir()), [])
                self.clear_install_dir()

    def test_symlinks_publish_to_resolved_targets_in_the_target_directory(self):
        target_dir = self.root / "targets"
        target_dir.mkdir()
        target = target_dir / "client"
        intermediate = self.install_dir / "client-link"
        intermediate.symlink_to("../targets/client")
        client = self.install_dir / "ccp-client"
        client.symlink_to("client-link")
        metadata = target_dir / "metadata.json"
        metadata.write_text("old metadata")
        metadata_link = self.install_dir / "ccp-nightly.json"
        for mode, args in self.install_modes():
            with self.subTest(mode=mode):
                self.log.unlink(missing_ok=True)
                client.unlink(missing_ok=True)
                client.symlink_to("client-link")
                target.write_bytes(b"working-client")
                if mode == "nightly":
                    metadata_link.symlink_to(metadata)
                result, calls = self.install("--client", *args)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertTrue(client.is_symlink())
                self.assertTrue(intermediate.is_symlink())
                expected = b"replacement-client" if mode == "source" else b"ccp-client-linux-x86_64"
                self.assertEqual(target.read_bytes(), expected)
                self.assertEqual(target.stat().st_mode & 0o777, 0o755)
                stages = [call[1] for call in calls if call[0] == "mktemp" and call[1] != "-d"]
                self.assertTrue(stages)
                self.assertTrue(all(Path(stage).parent == target_dir.resolve() for stage in stages))
                self.assertEqual(
                    sorted(p.name for p in target_dir.iterdir()), ["client", "metadata.json"]
                )
                if mode == "nightly":
                    self.assertTrue(metadata_link.is_symlink())
                    self.assertEqual(json.loads(metadata.read_text()), {"source_sha": SHA})
                    self.assertEqual(metadata.stat().st_mode & 0o777, 0o644)
                self.log.unlink()

    def test_symlinked_install_dir_resolves_relative_links_physically(self):
        real = self.root / "real"
        logical = self.root / "logical"
        for directory in (real / "bin", real / "lib", logical / "lib"):
            directory.mkdir(parents=True)
        (logical / "bin").symlink_to(real / "bin", target_is_directory=True)
        binary = real / "bin/ccp-client"
        binary.symlink_to("../lib/ccp-client")
        target = real / "lib/ccp-client"
        unrelated = logical / "lib/ccp-client"
        for mode, args in self.install_modes():
            with self.subTest(mode=mode), patch.object(self, "install_dir", logical / "bin"):
                target.write_bytes(b"old-client")
                unrelated.write_bytes(b"unrelated-client")
                result, _ = self.install("--client", *args)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                expected = b"replacement-client" if mode == "source" else b"ccp-client-linux-x86_64"
                self.assertEqual(target.read_bytes(), expected)
                self.assertEqual(unrelated.read_bytes(), b"unrelated-client")
                self.assertTrue(binary.is_symlink())

    def test_relative_install_dir_ignores_cdpath(self):
        self.source_fixture()
        intended = self.root / "relative-bin"
        hijack = self.root / "search-path"
        (hijack / "relative-bin").mkdir(parents=True)
        unrelated = hijack / "relative-bin/ccp-client"
        unrelated.write_bytes(b"unrelated-client")
        with (
            patch.object(self, "install_dir", Path("relative-bin")),
            patch.dict(os.environ, CDPATH=str(hijack)),
        ):
            result, _ = self.install("--client", "--from-source")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((intended / "ccp-client").read_bytes(), b"replacement-client")
        self.assertEqual(unrelated.read_bytes(), b"unrelated-client")

    def test_stable_failed_transfer_preserves_installed_bytes(self):
        target = self.install_dir / "ccp-client"
        target.write_bytes(b"working-client")
        target.chmod(0o755)
        result, _ = self.install("--client", failure="ccp-client-linux-x86_64")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(target.read_bytes(), b"working-client")
        self.assertNotIn("Done.", result.stdout)
        self.assertEqual(sorted(p.name for p in self.install_dir.iterdir()), ["ccp-client"])

    def test_source_publish_failures_preserve_old_binary_and_refuse_success(self):
        self.source_fixture()
        for failure in ("cp", "chmod", "mv", "mkdir"):
            with self.subTest(failure=failure):
                target = self.install_dir / "ccp-client"
                target.write_bytes(b"working-client")
                target.chmod(0o755)
                result, _ = self.install("--client", "--from-source", failure=failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(target.read_bytes(), b"working-client")
                self.assertNotIn("Done.", result.stdout)
                self.assertNotIn("Installed ", result.stdout)
                self.assertEqual(sorted(p.name for p in self.install_dir.iterdir()), ["ccp-client"])

    def test_stable_mcp_failure_refuses_completion(self):
        (self.root / "mcp").mkdir()
        result, calls = self.install("--client", failure="pip")
        self.assertTrue(any(call[0] == "pip" for call in calls))
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("MCP bridge installed", result.stdout)
        self.assertNotIn("Done.", result.stdout)

    def test_missing_python3_refuses_mcp_bridge_and_completion(self):
        (self.root / "mcp").mkdir()
        bash = shutil.which("bash")
        self.assertIsNotNone(bash, "bash must exist on the test host")
        # Build an allowlist directory holding only the few real binaries
        # install.sh needs before it reaches the python3 check (cat for the
        # banner heredoc, rm for the post-install cleanup); every other
        # command it runs is already one of the stub overrides in
        # self.bin_dir. A denylist approach (e.g. keeping "/bin" on PATH)
        # is not portable: on Debian/Ubuntu "/bin" is a symlink to
        # "/usr/bin", which also contains python3.
        safe_bin = self.root / "no-python-bin"
        safe_bin.mkdir()
        for tool in ("cat", "rm"):
            resolved = shutil.which(tool)
            self.assertIsNotNone(resolved, f"{tool} must exist on the test host")
            os.symlink(resolved, safe_bin / tool)
        env = dict(
            os.environ,
            HOME=str(self.home_dir),
            TMPDIR=str(self.root),
            PATH=f"{self.bin_dir}:{safe_bin}",
            NIGHTLY_TEST_LOG=str(self.log),
            NIGHTLY_TEST_FIXTURES=str(self.fixtures),
            NIGHTLY_TEST_FAILURE="",
        )
        argv = [bash, str(self.installer), "--install-dir", str(self.install_dir), "--client"]
        result = subprocess.run(  # noqa: S603
            argv, cwd=self.root, env=env, capture_output=True, text=True, start_new_session=True
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Python 3 is required", result.stdout + result.stderr)
        self.assertNotIn("Done.", result.stdout)

    def mcp_bridge_fixture(self):
        (self.root / "mcp").mkdir()
        mcp_cmd = self.home_dir / ".ccp-mcp/venv/bin/ccp-mcp-server"
        mcp_cmd.write_text("stub")
        mcp_cmd.chmod(0o755)

    def test_codex_config_write_failure_is_reported_honestly(self):
        # A chmod-based permission denial is silently bypassed when the test
        # runs as root, so this fails the "cat" that appends to the config
        # instead: a failure no privilege level can route around.
        self.mcp_bridge_fixture()
        codex_dir = self.home_dir / ".codex"
        codex_dir.mkdir(parents=True)
        codex_config = codex_dir / "config.toml"
        codex_config.write_text("")
        result, _ = self.install("--client", failure="cat")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            re.sub(r"\x1b\[[0-9;]*m", "", result.stdout).splitlines()[-1],
            "Done, with warnings: the Codex MCP config was not written; see above.",
        )
        self.assertNotIn(f"Added to {codex_config}", result.stdout)
        self.assertIn(f"Could not update {codex_config}", result.stdout)

    def test_cursor_config_create_failure_is_reported_honestly(self):
        # A directory where install.sh expects to create a file fails the
        # redirect with EISDIR regardless of permissions, unlike a
        # chmod-based denial that root silently bypasses.
        self.mcp_bridge_fixture()
        cursor_dir = self.home_dir / ".cursor"
        cursor_dir.mkdir(parents=True)
        cursor_config = cursor_dir / "mcp.json"
        cursor_config.mkdir()
        result, _ = self.install("--client")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            re.sub(r"\x1b\[[0-9;]*m", "", result.stdout).splitlines()[-1],
            "Done, with warnings: the Cursor MCP config was not written; see above.",
        )
        self.assertNotIn(f"Created {cursor_config}", result.stdout)
        self.assertIn(f"Could not create {cursor_config}", result.stdout)

    def test_source_success_publishes_executable_bytes(self):
        self.source_fixture()
        result, _ = self.install("--client", "--from-source")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        target = self.install_dir / "ccp-client"
        self.assertEqual(target.read_bytes(), b"replacement-client")
        self.assertNotEqual(target.stat().st_mode & 0o111, 0)
        self.assertEqual(target.stat().st_mode & 0o777, 0o755)
        self.assertIn("Done.", result.stdout)

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
        target = self.install_dir / "ccp-client"
        self.assertEqual(target.read_bytes(), b"ccp-client-linux-x86_64")
        self.assertEqual(target.stat().st_mode & 0o777, 0o755)
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

    def test_nightly_interrupted_copy_preserves_installed_binary(self):
        (self.install_dir / "ccp-client").write_bytes(b"working installation")
        result, calls = self.install(
            "--client", "--channel", "nightly", "--version", TAG, failure="cp"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.install_dir / "ccp-client").read_bytes(), b"working installation")
        self.assertEqual(sorted(p.name for p in self.install_dir.iterdir()), ["ccp-client"])
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
