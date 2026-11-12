# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Reject tool attribution in commit metadata and pull request titles and bodies."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = (
    r"claude(?:[ -](?:code|opus|sonnet|haiku)(?:[ -][0-9.]+)?)?|anthropic|open[ -]?ai"
    r"|codex|chatgpt|(?:(?:github|microsoft)[ -]?)?copilot"
    r"(?:[ -]swe[ -]agent)?|cursor(?:[ -]?agent)?|t3[ -]?code|gemini|aider|devin"
    r"|sweep(?:[ -]?ai)?|windsurf|codeium|tabnine|amazon[ -]q|augment(?:[ -]code)?"
    r"|cline|roo[ -]code|amp[ -]code|opencode|kiro|junie|replit[ -]agent"
)
TOOL_MENTION = re.compile(rf"(?<!\w)(?:{TOOL})(?!\w)", re.I)
TOOL_IDENTITY = re.compile(
    rf"(?:(?:anthropic|open[ -]?ai)[ -]+)?(?:{TOOL})(?:[ -]?(?:bot|agent|assistant))?", re.I
)
DEPENDABOT = ("dependabot[bot]", "49699333+dependabot[bot]@users.noreply.github.com")
# Dependabot's standard Signed-off-by trailer uses its support mailbox.
DEPENDABOT_SIGNOFF = ("dependabot[bot]", "support@github.com")
WEB_FLOW = ("github", "noreply@github.com")
TOOL_MAILBOXES = {"noreply@anthropic.com", "claude@anthropic.com", "codex@openai.com"}
TRAILER = re.compile(r"^([A-Za-z][A-Za-z0-9-]*)[ \t]*:[ \t]*(.*)$")
ATTRIBUTION_KEY = re.compile(
    r"^(?:(?:co-)?authored|signed-off|reviewed|generated|assisted|powered|helped)-by$", re.I
)
GENERATED = re.compile(
    r"^[\W_]*(?:generated|created|written|co-authored|assisted|powered)"
    r"\s+(?:with|by|using)\b",
    re.I,
)
BADGE = re.compile(
    r"\[!\[[^\]]*\]\([^\n)]*\)\]\([^\n)]*\)"
    r"|!\[[^\]]*\]\([^\n)]*(?:shields\.io|badgen\.net)[^\n)]*\)",
    re.I,
)


def normalized(text: str) -> str:
    return unicodedata.normalize("NFKC", text).casefold()


def identity_problem(name: str, email: str, role: str) -> bool:
    identity = (normalized(name.strip()), normalized(email.strip()))
    if (
        identity == DEPENDABOT
        or (role == "co-author" and identity == DEPENDABOT_SIGNOFF)
        or (role == "committer" and identity == WEB_FLOW)
    ):
        return False
    local = identity[1].split("@", 1)[0]
    bot_name = identity[0].removesuffix("[bot]").strip()
    return bool(
        TOOL_IDENTITY.fullmatch(bot_name)
        or "[bot]" in identity[0]
        or "[bot]" in local
        or identity[1] in TOOL_MAILBOXES
        or identity[1] == WEB_FLOW[1]
        or identity[0]
        in {"github-actions", "github actions", "github", "dependabot", "dependabot[bot]"}
    )


def trailer_values(message: str, commit: bool) -> list[str]:
    lines = message.rstrip("\r\n").splitlines()
    last_blank = max((i for i, line in enumerate(lines) if not line.strip()), default=-1)
    values: list[str] = []
    active = False
    for index, line in enumerate(lines):
        match = TRAILER.fullmatch(line)
        if match:
            active = bool(
                ATTRIBUTION_KEY.fullmatch(match[1]) or (commit and index > last_blank and index > 0)
            )
            if active:
                values.append(match[2])
        elif active and line.startswith((" ", "\t")):
            values[-1] += " " + line.strip()
        else:
            active = False
    return values


def attributed_trailer(value: str) -> bool:
    identity = re.fullmatch(r"\s*(.*?)\s*<([^<>]+)>\s*", value)
    if identity:
        return identity_problem(identity[1], identity[2], "co-author")
    return bool(TOOL_MENTION.search(normalized(value)))


def text_findings(text: str, *, commit: bool = False) -> list[str]:
    problems = []
    for line in text.splitlines():
        # A normal product reference or documentation link is not an attribution.
        tool = TOOL_MENTION.search(normalized(line))
        if "\U0001f916" in line:
            problems.append("robot attribution badge")
        elif tool and (GENERATED.search(line) or BADGE.search(line)):
            problems.append("generated-with/by attribution or linked tool badge")
    if any(attributed_trailer(value) for value in trailer_values(text, commit)):
        problems.append("trailer attributes work to a tool")
    return sorted(set(problems))


def run(root: Path, executable: str, *args: str) -> bytes:
    program = shutil.which(executable)
    if program is None:
        raise ValueError(f"{executable} is required for attribution checking")
    # Event text is data; only validated revisions/PR numbers enter fixed argv.
    return subprocess.run(  # noqa: S603 -- Fixed executable, argv, and no shell.
        [program, *args], cwd=root, capture_output=True, check=True
    ).stdout


def resolve(root: Path, revision: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_./-]*", revision):
        raise ValueError("invalid revision; supply a commit SHA or ref name")
    try:
        resolved = run(
            root, "git", "rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}"
        )
        return resolved.decode("ascii").strip()
    except subprocess.CalledProcessError as error:
        raise ValueError("cannot resolve commit range; fetch full base and head history") from error


def commit_ids(root: Path, revision_range: str | None) -> list[str]:
    if run(root, "git", "rev-parse", "--is-shallow-repository").strip() != b"false":
        raise ValueError("shallow history cannot establish the commit range; fetch full history")
    selection = resolve(root, "HEAD")
    if revision_range is not None:
        if revision_range.count("..") != 1 or "..." in revision_range:
            raise ValueError("range must contain two explicit endpoints: BASE..HEAD")
        base, head = revision_range.split("..")
        selection = f"{resolve(root, base)}..{resolve(root, head)}"
    return run(root, "git", "rev-list", selection).decode("ascii").splitlines()


def commit_findings(root: Path, sha: str) -> list[str]:
    raw = run(root, "git", "cat-file", "commit", sha).decode("utf-8", errors="replace")
    headers, separator, message = raw.partition("\n\n")
    if not separator:
        raise ValueError("commit metadata is incomplete")
    problems = text_findings(message, commit=True)
    for role in ("author", "committer"):
        match = re.search(rf"^{role} (.*?) <([^<>]*)> -?\d+ [+-]\d{{4}}$", headers, re.M)
        if match is None:
            raise ValueError(f"commit has no valid {role} identity")
        if identity_problem(match[1], match[2], role):
            problems.append(f"{role} uses a tool identity")
    return [f"{sha}: {problem}" for problem in problems]


def object_fields(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("event/API data must be a JSON object")
    return dict(value)


def string_field(data: dict[str, object], name: str) -> str:
    value = data.get(name)
    if not isinstance(value, str):
        raise ValueError(f"event/API field {name!r} must be a string")
    return value


def manual_pull_request(root: Path, inputs: dict[str, object]) -> dict[str, object]:
    number = string_field(inputs, "pr_number")
    repository = os.environ.get("GITHUB_REPOSITORY", "")
    if not re.fullmatch(r"[1-9][0-9]*", number):
        raise ValueError("pr_number must be a positive decimal integer")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("GITHUB_REPOSITORY must name an owner/repository")
    value: object = json.loads(run(root, "gh", "api", f"repos/{repository}/pulls/{number}"))
    pull_request = object_fields(value)
    if pull_request.get("number") != int(number):
        raise ValueError("API returned a different pull request")
    base, head = pr_endpoints(pull_request)
    run(root, "git", "fetch", "--no-tags", "origin", base, head)
    return pull_request


def pr_endpoints(pull_request: dict[str, object]) -> tuple[str, str]:
    values = []
    for field in ("base", "head"):
        sha = string_field(object_fields(pull_request.get(field)), "sha")
        if not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise ValueError(f"pull request {field} must contain a full commit SHA")
        values.append(sha)
    return values[0], values[1]


def event_selection(root: Path, path: Path) -> tuple[str, list[str]]:
    value: object = json.loads(path.read_bytes())
    event = object_fields(value)
    if "pull_request" in event or "inputs" in event:
        pr = (
            object_fields(event["pull_request"])
            if "pull_request" in event
            else manual_pull_request(root, object_fields(event["inputs"]))
        )
        base, head = pr_endpoints(pr)
        problems: list[str] = []
        for field in ("title", "body"):
            text = "" if field == "body" and pr.get(field) is None else string_field(pr, field)
            problems.extend(f"PR {field}: {finding}" for finding in text_findings(text))
        return f"{base}..{head}", problems
    if event.get("ref") != "refs/heads/main":
        raise ValueError("push attribution checks require refs/heads/main")
    endpoints = [string_field(event, field) for field in ("before", "after")]
    if any(not re.fullmatch(r"[0-9a-f]{40}", sha) or sha == "0" * 40 for sha in endpoints):
        raise ValueError("push requires resolvable nonzero before and after commit SHAs")
    return "..".join(endpoints), []


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument(
        "--range", dest="revision_range", help="check every commit in BASE..HEAD"
    )
    selection.add_argument(
        "--all-history", action="store_true", help="check all commits reaching HEAD"
    )
    selection.add_argument(
        "--event", type=Path, help="read PR/push/dispatch JSON from this event file"
    )
    args = parser.parse_args()
    try:
        revision_range, problems = (
            event_selection(ROOT, args.event) if args.event else (args.revision_range, [])
        )
        commits = commit_ids(ROOT, revision_range)
        for sha in commits:
            problems.extend(commit_findings(ROOT, sha))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Attribution could not be checked: {error}\n")
    if problems:
        parser.exit(1, "\n".join(problems) + "\n")
    print(f"Attribution passes for {len(commits)} commits and any supplied PR metadata.")


if __name__ == "__main__":
    main()
