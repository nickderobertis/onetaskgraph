"""Policy subprocess input validation from the pinned onevcs contract."""

import json
from pathlib import Path
from typing import Literal

SCHEMA = (
    Path(__file__).resolve().parents[3]
    / "crates/onetaskgraph-core/src/engine/boundary/onevcs-boundary-schema-v4.json"
)


def policy_input(command: Literal["terms", "sanitize"]) -> str:
    """Build stdin validation using the producer's schema, without a second wire model."""
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))[command]["input"]
    return f"""
import json, sys
from jsonschema import Draft202012Validator
schema = json.loads({json.dumps(schema)!r})
Draft202012Validator.check_schema(schema)
try:
    received = json.loads(sys.stdin.read())
except (ValueError, TypeError):
    sys.exit(2)
if not Draft202012Validator(schema).is_valid(received):
    sys.exit(2)
"""
