# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Private, short-lived request files; persisted entry intents belong to Rust."""

from __future__ import annotations

import os
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path


@contextmanager
def private_request(directory: Path, payload: bytes) -> Iterator[Path]:
    descriptor, name = tempfile.mkstemp(prefix="request-", suffix=".json", dir=directory)
    path = Path(name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
        # Close first so the Rust child can open the file on every platform.
        yield path
    finally:
        path.unlink(missing_ok=True)
