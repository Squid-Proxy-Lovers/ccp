"""Load the bridge with a local decorator double for focused unit tests."""

import importlib.util
import json  # Load its decoder outside the temporary module registry.
import sys
import types
from pathlib import Path


class FakeFastMCP:
    def __init__(self, *_args, **_kwargs):
        pass

    @staticmethod
    def tool():
        return lambda function: function

    @staticmethod
    def resource(_uri):
        return lambda function: function


def load_unit_server():
    fake_fastmcp = types.ModuleType("fastmcp")
    fake_fastmcp.FastMCP = FakeFastMCP
    name = "ccp_mcp_unit_server"
    path = Path(__file__).resolve().parents[1] / "src/ccp_mcp_server/server.py"
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    previous = sys.modules.get("fastmcp")
    sys.modules["fastmcp"] = fake_fastmcp
    sys.modules[name] = module
    try:
        spec.loader.exec_module(module)
    finally:
        if previous is None:
            del sys.modules["fastmcp"]
        else:
            sys.modules["fastmcp"] = previous
    return module
