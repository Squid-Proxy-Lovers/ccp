#!/usr/bin/env python3
"""Exercise the image's default command, TLS-pinned setup and persistent restart."""

import argparse
import hashlib
import json
import re
import shutil
import ssl
import subprocess
import time
import urllib.error
import urllib.request
import uuid


# Fixed Docker argv and generated fixture names; never invoke a shell or print secrets.
def docker(*args: str) -> str:
    executable = shutil.which("docker")
    if executable is None:
        raise RuntimeError("Docker is required for this integration test")
    result = subprocess.run(  # noqa: S603 -- fixed Docker argv and unique fixture names
        [executable, *args], capture_output=True, text=True, check=True, timeout=45
    )
    return result.stdout.strip()


def startup(name: str, *, fresh: bool) -> tuple[str, str]:
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        logs = docker("logs", name)
        opened = re.search(r"^Open: (https://\S+)$", logs, re.MULTILINE)
        fingerprint = re.search(r"^CA SHA-256: ([0-9a-f:]+)$", logs, re.MULTILINE)
        if opened and fingerprint:
            assert ("#code=" in opened[1]) == fresh
            return opened[1], fingerprint[1].replace(":", "")
        time.sleep(0.1)
    raise AssertionError("container did not print its startup URL and CA fingerprint")


def request(
    opener: urllib.request.OpenerDirector, origin: str, path: str, body: dict | None = None
) -> tuple[int, bytes]:
    headers = {"Origin": origin, "X-CCP-CSRF": "setup"}
    data = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode()
    # origin is constructed from https://localhost and Docker's numeric mapped port.
    query = urllib.request.Request(origin + path, data=data, headers=headers)  # noqa: S310
    try:
        with opener.open(query, timeout=10) as response:
            return response.status, response.read(1024 * 1024)
    except urllib.error.HTTPError as error:
        return error.code, error.read(1024 * 1024)


def pinned_client(origin: str, pin: str) -> urllib.request.OpenerDirector:
    # Test-only public CA discovery: no credentials/cookies; startup output is the
    # independent trust anchor. Only the pinned CA enters the normal TLS client.
    discovery = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    discovery.check_hostname = False
    discovery.verify_mode = ssl.CERT_NONE
    probe = urllib.request.build_opener(
        urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=discovery)
    )
    status, pem = request(probe, origin, "/api/v1/ca.pem")
    assert status == 200 and b"PRIVATE KEY" not in pem
    der = ssl.PEM_cert_to_DER_cert(pem.decode())
    assert hashlib.sha256(der).hexdigest() == pin
    strict = ssl.create_default_context(cadata=pem.decode())
    return urllib.request.build_opener(
        urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=strict)
    )


def run(image: str) -> None:
    suffix = uuid.uuid4().hex
    name, volume = "ccp-first-boot-" + suffix, "ccp-first-boot-data-" + suffix
    docker("volume", "create", volume)
    try:
        # Same default command, container ports and named-volume mount as the
        # documented one-liner; ephemeral host ports avoid disturbing local CCP.
        docker(
            "run",
            "-d",
            "--name",
            name,
            "-p",
            "127.0.0.1::8443",
            "-p",
            "127.0.0.1::8444",
            "-v",
            volume + ":/var/lib/ccp/server",
            image,
        )
        link, pin = startup(name, fresh=True)
        port = docker("port", name, "8443/tcp").rsplit(":", 1)[1]
        assert docker("port", name, "8444/tcp")
        origin = "https://localhost:" + port
        client = pinned_client(origin, pin)
        status, page = request(client, origin, "/admin/setup")
        assert status == 200 and b"/admin/app.js" in page
        assert request(client, origin, "/api/v1/setup/verify", {"code": "wrong"})[0] == 401
        code = link.split("#code=", 1)[1]
        assert request(client, origin, "/api/v1/setup/verify", {"code": code})[0] == 200
        claim = {"code": code, "operator": "Docker owner", "workspace": "Docker workspace"}
        status, receipt = request(client, origin, "/api/v1/setup/claim", claim)
        assert status == 200
        keys = json.loads(receipt)["data"]
        assert keys["instance_admin_key"]["secret"] != keys["workspace_owner_key"]["secret"]
        assert request(client, origin, "/api/v1/setup/claim", claim)[0] == 409
        docker("stop", "--time", "10", name)
        docker("rm", name)
        docker(
            "run",
            "-d",
            "--name",
            name,
            "-p",
            "127.0.0.1::8443",
            "-p",
            "127.0.0.1::8444",
            "-v",
            volume + ":/var/lib/ccp/server",
            image,
        )
        _, next_pin = startup(name, fresh=False)
        assert next_pin == pin
        port = docker("port", name, "8443/tcp").rsplit(":", 1)[1]
        origin = "https://localhost:" + port
        client = pinned_client(origin, pin)
        assert request(client, origin, "/api/v1/setup/claim", claim)[0] == 409
        # Same fixed HTTPS loopback origin; invitation/setup secrets never enter the URL.
        login = urllib.request.Request(  # noqa: S310
            origin + "/api/v1/admin/session/login",
            data=json.dumps({"key": keys["workspace_owner_key"]["secret"]}).encode(),
            headers={"Origin": origin, "X-CCP-CSRF": "login", "Content-Type": "application/json"},
        )
        with client.open(login, timeout=10) as response:
            assert response.status == 200 and response.headers.get("Set-Cookie")
        print(
            "PASS: default Docker startup, pinned HTTPS claim, one-use code and same-volume key/CA restart"
        )
    finally:
        # Only unique names created by this run; failure to clean up is an error.
        docker("rm", "-f", name)
        docker("volume", "rm", volume)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    run(parser.parse_args().image)
