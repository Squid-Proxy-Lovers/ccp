#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Checkout CLI; use the same stdlib verifier shipped in the MCP archive."""

import subprocess as subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "mcp" / "src"))
# The fixed checkout path makes this CLI work without installing the MCP package.
from ccp_mcp_server.release_contract import (
    ASSETS as ASSETS,
)
from ccp_mcp_server.release_contract import (
    CONTRACTS as CONTRACTS,
)
from ccp_mcp_server.release_contract import (
    MANIFEST as MANIFEST,
)
from ccp_mcp_server.release_contract import (
    MAX_ARTIFACT as MAX_ARTIFACT,
)
from ccp_mcp_server.release_contract import (
    PLATFORMS as PLATFORMS,
)
from ccp_mcp_server.release_contract import (
    PROVENANCE as PROVENANCE,
)
from ccp_mcp_server.release_contract import (
    REPOSITORY as REPOSITORY,
)
from ccp_mcp_server.release_contract import (
    WORKFLOW as WORKFLOW,
)
from ccp_mcp_server.release_contract import (
    Artifact as Artifact,
)
from ccp_mcp_server.release_contract import (
    Manifest as Manifest,
)
from ccp_mcp_server.release_contract import (
    decode as decode,
)
from ccp_mcp_server.release_contract import (
    digest as digest,
)
from ccp_mcp_server.release_contract import (
    fields as fields,
)
from ccp_mcp_server.release_contract import (
    main as main,
)
from ccp_mcp_server.release_contract import (
    number as number,
)
from ccp_mcp_server.release_contract import (
    prepare as prepare,
)
from ccp_mcp_server.release_contract import (
    read_file as read_file,
)
from ccp_mcp_server.release_contract import (
    require as require,
)
from ccp_mcp_server.release_contract import (
    source_number as source_number,
)
from ccp_mcp_server.release_contract import (
    source_profile_min as source_profile_min,
)
from ccp_mcp_server.release_contract import (
    source_ref as source_ref,
)
from ccp_mcp_server.release_contract import (
    source_version as source_version,
)
from ccp_mcp_server.release_contract import (
    text as text,
)
from ccp_mcp_server.release_contract import (
    unique as unique,
)
from ccp_mcp_server.release_contract import (
    verify as verify,
)
from ccp_mcp_server.release_contract import (
    verify_provenance as verify_provenance,
)

if __name__ == "__main__":
    main()
