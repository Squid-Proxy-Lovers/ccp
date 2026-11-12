# SPDX-License-Identifier: AGPL-3.0-or-later
"""Bounded public GitHub downloads; release metadata never supplies executable URLs."""

from __future__ import annotations

import io
import json
import time
import urllib.error
import urllib.request
from http.client import HTTPException, HTTPResponse
from pathlib import Path
from typing import BinaryIO, cast
from urllib.parse import urljoin, urlsplit

from .release_contract import REPOSITORY, require, unique

API = f"https://api.github.com/repos/{REPOSITORY}/"
DOWNLOAD = f"https://github.com/{REPOSITORY}/releases/download/"
HOSTS = {
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args: object, **_kwargs: object) -> None:
        return None


def validate_url(url: str) -> None:
    parsed = urlsplit(url)
    require(
        parsed.scheme == "https"
        and parsed.hostname in HOSTS
        and parsed.port in {None, 443}
        and parsed.username is None
        and parsed.password is None
        and not parsed.fragment
        and not any(ord(c) < 33 or c == "\\" for c in url),
        "Unexpected official download URL",
    )


def receive(url: str, sink: BinaryIO, maximum: int) -> None:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    for _ in range(6):
        validate_url(url)
        # URL is allowlisted HTTPS; no credentials, caller URLs, proxies or shell.
        request = urllib.request.Request(  # noqa: S310
            url,
            headers={
                "User-Agent": "CCP-Managed",
                "Accept": "application/vnd.github+json"
                if url.startswith(API)
                else "application/octet-stream",
                "X-GitHub-Api-Version": "2026-03-10",
            },
        )
        try:
            with cast(HTTPResponse, opener.open(request, timeout=20)) as response:
                require(response.status == 200, "Official download did not complete")
                transfer(response, sink, maximum)
                return
        except urllib.error.HTTPError as error:
            location = error.headers.get("Location")
            error.close()
            require(
                error.code in {301, 302, 303, 307, 308} and location is not None,
                "Official download is unavailable",
            )
            url = urljoin(url, cast(str, location))
        except (OSError, urllib.error.URLError, HTTPException):
            raise ValueError("Official download is unavailable") from None
    raise ValueError("Too many official download redirects")


def transfer(response: HTTPResponse, sink: BinaryIO, maximum: int) -> None:
    require(response.headers.get("Content-Encoding") is None, "Encoded download refused")
    length = response.headers.get("Content-Length")
    if length is not None:
        require(
            length.isascii() and length.isdecimal() and int(length) <= maximum,
            "Official download exceeds limit",
        )
    total, started = 0, time.monotonic()
    while chunk := response.read1(min(1024 * 1024, maximum - total + 1)):
        total += len(chunk)
        require(
            total <= maximum and time.monotonic() - started < 120,
            "Official download exceeds byte or time limit",
        )
        sink.write(chunk)
    require(total > 0 and (length is None or int(length) == total), "Incomplete official download")


def api(path: str) -> object:
    sink = io.BytesIO()
    receive(API + path, sink, 2 * 1024 * 1024)
    try:
        return json.loads(sink.getvalue(), object_pairs_hook=unique)
    except (ValueError, UnicodeDecodeError, RecursionError):
        raise ValueError("Invalid official release metadata") from None


def asset(tag: str, name: str, target: Path, maximum: int) -> None:
    with target.open("xb") as sink:
        target.chmod(0o600)
        receive(DOWNLOAD + tag + "/" + name, sink, maximum)
