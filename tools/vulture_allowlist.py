# Cephalopod Coordination Protocol
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Names that vulture cannot see being used. Each entry needs a reason.

Vulture parses this file without running it; the bare names count as uses.
"""
# ruff: noqa: B018, F821

# Server-administration and destructive MCP functions exist, but are not
# registered with @mcp.tool(). Exposing or deleting them needs a maintainer
# decision, so they stay listed until that is made.
_runtime_metadata_path
_run_server_admin
server_sessions
stop_server
restart_server
tail_server_logs
rename_session
delete_session
delete_entry
restore_entry
import_bundle
revoke_certificate

# tarfile.TarInfo fields set by tests to build fixture archives.
_.type
_.linkname
_.size
