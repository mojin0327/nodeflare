#!/usr/bin/env python3
"""Write a secret-free, machine-readable receipt for the Builder CI run."""

import json
import os
from datetime import datetime, timezone
from pathlib import Path


def integer_env(name: str) -> int:
    value = os.environ.get(name, "")
    return int(value) if value.isdigit() else 0


receipt = {
    "schema_version": "1.0",
    "repository": os.environ.get("REPOSITORY", ""),
    "workflow": os.environ.get("WORKFLOW", ""),
    "run_id": integer_env("RUN_ID"),
    "run_attempt": integer_env("RUN_ATTEMPT"),
    "run_url": os.environ.get("RUN_URL", ""),
    "event": os.environ.get("EVENT_NAME", ""),
    "pr_number": integer_env("PR_NUMBER") or None,
    "head_sha": os.environ.get("HEAD_SHA", ""),
    "base_sha": os.environ.get("BASE_SHA", ""),
    "ref": os.environ.get("REF", ""),
    "command": "cargo test --locked -p mcp-builder",
    "scope": ["mcp-builder unit tests"],
    "status": os.environ.get("TEST_OUTCOME", "not_run"),
    "boundaries": [
        "Does not deploy an MCP server",
        "Does not exercise NodeFlare production services",
        "Does not prove other workspace packages pass",
    ],
    "generated_at": datetime.now(timezone.utc).isoformat(),
}

Path("verification-receipt.json").write_text(
    json.dumps(receipt, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
