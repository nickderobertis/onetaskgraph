"""Reduce one run's raw logs to what is committed: timestamps, durations, operation names,
counts, costs and limit headers. Nothing else passes. Every field is copied by allowlist,
never by removing what looks sensitive, so a board's titles, bodies, URLs and node ids
cannot reach the output; `leaks` then scans the output for anything shaped like one.

`python3 sanitize.py RAW_RUN_DIR OUT_DIR` (the runner calls `write` itself).
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

# GitHub's global node ids (`I_kwDO…`, `PVTI_…`, `PVT_…`, `PVTSSF_…`, `IC_…`) and the
# legacy base64 ones (`MDU6…`).
NODE_ID = re.compile(
    r"\b(?:[A-Z]{1,8}_(?=[A-Za-z0-9_-]*[a-z])[A-Za-z0-9_-]{8,}|MD[A-Za-z0-9]{2}[A-Za-z0-9=+/]{8,})"
)
ISSUE_URL = re.compile(r"https?://github\.com/[^\s\"']+")
LIMIT_FIELDS = ("limit", "remaining", "used", "reset")
SPAN_ATTRS = ("op", "kind", "attempt", "limiter", "operation")
STORE_VERBS = {
    "project",
    "task",
    "document",
    "list",
    "show",
    "copy",
    "deps",
    "answers",
    "render",
    "metadata",
    "set",
    "config",
    "sources",
}


def rest_path(host: str, path: str, kind: str) -> str:
    if kind == "tunnel":
        return host
    bare = path.split("?", 1)[0]
    if bare.startswith("/graphql"):
        return "/graphql"
    if bare.startswith("/user-attachments/assets"):
        return "/user-attachments/assets"
    if re.fullmatch(r"/repos/[^/]+/[^/]+", bare):
        return "/repos/{owner}/{repo}"
    if bare.startswith("/rate_limit"):
        return "/rate_limit"
    return "other"


def limits(raw: dict) -> dict:
    out: dict = {}
    for key, value in raw.items():
        short = key.removeprefix("x-ratelimit-")
        if short in LIMIT_FIELDS or key == "retry-after":
            try:
                out[short] = int(value)
            except ValueError:
                continue
        elif short == "resource" and re.fullmatch(r"[a-z_]+", value):
            out[short] = value
    return out


def operation(record: dict) -> str:
    op = str(record.get("op", ""))
    if record.get("kind") == "rest":
        method = str(record.get("method", ""))
        return f"{method} {rest_path(record['host'], record['path'], 'rest')}"
    if record.get("kind") == "tunnel":
        return (
            f"CONNECT {record['host']}"
            if re.fullmatch(r"[a-z0-9.-]+", record["host"])
            else "CONNECT"
        )
    return op if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,80}", op) else "unnamed"


def request(record: dict) -> dict:
    return {
        "seq": int(record["seq"]),
        "start": round(float(record["start"]), 3),
        "end": round(float(record["end"]), 3),
        "ms": float(record["ms"]),
        "method": str(record["method"])
        if record["method"] in ("GET", "POST", "CONNECT")
        else "other",
        "host": record["host"]
        if re.fullmatch(r"[a-z0-9.-]+", record["host"])
        else "other",
        "path": rest_path(record["host"], record["path"], record["kind"]),
        "kind": record["kind"]
        if record["kind"] in ("query", "mutation", "rest", "tunnel")
        else "other",
        "op": operation(record),
        "req_bytes": int(record["req_bytes"]),
        "resp_bytes": int(record["resp_bytes"]),
        "status": int(record["status"]),
        "cost": record["cost"] if isinstance(record.get("cost"), int) else None,
        "errors": [
            e
            for e in record.get("errors", [])
            if re.fullmatch(r"[A-Z_]{1,40}|untyped", e)
        ],
        "limits": limits(record.get("limits", {})),
    }


def span(record: dict) -> dict:
    attrs = {
        k: v
        for k, v in record.get("attrs", {}).items()
        if k in SPAN_ATTRS and re.fullmatch(r"[A-Za-z0-9_]{1,80}", str(v))
    }
    return {
        "span": str(record["span"]),
        "start": round(record["start_us"] / 1e6, 6),
        "end": round(record["end_us"] / 1e6, 6),
        "attrs": attrs,
    }


def argv(words: list[str]) -> list[str]:
    """A command line with every id, path and value replaced by what kind of word it was."""
    out = []
    for word in words:
        if (
            word.startswith("--")
            and re.fullmatch(r"--[a-z-]+", word)
            or word in STORE_VERBS
            or re.fullmatch(r"[a-z][a-z-]{0,30}", word)
        ):
            out.append(word)
        elif "/" in word and not ":" in word:
            out.append("<path>")
        elif ":" in word:
            source = word.split(":", 1)[0]
            out.append(
                f"{source if re.fullmatch(r'[a-z-]+', source) else '<source>'}:<id>"
            )
        else:
            out.append("<value>")
    return out


def host(record: dict) -> dict:
    out = {
        "kind": record["kind"],
        "start": round(record["start"], 3),
        "end": round(record["end"], 3),
    }
    if record["kind"] == "store":
        out["argv"] = argv(record["argv"])
        out["returncode"] = record["returncode"]
    else:
        out["name"] = (
            record["name"] if re.fullmatch(r"[a-z_.]+", record["name"]) else "step"
        )
    return out


def step(record: dict) -> dict:
    words = record["argv"]
    program = Path(words[0]).name
    return {
        "step": record["step"],
        "program": program if program in ("onetaskgraph", "just") else "other",
        "argv": argv(words[1:]),
        "start": round(record["start"], 3),
        "end": round(record["end"], 3),
        "returncode": record["returncode"],
    }


def redact(value: object) -> object:
    if isinstance(value, dict):
        return {k: redact(v) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v) for v in value]
    if isinstance(value, str):
        return NODE_ID.sub("<node-id>", ISSUE_URL.sub("<url>", value))
    return value


def lines(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def dump(path: Path, records: list[dict]) -> None:
    path.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in records))


def write(run: Path, out: Path) -> Path:
    out.mkdir(parents=True, exist_ok=True)
    dump(out / "requests.jsonl", [request(r) for r in lines(run / "proxy.jsonl")])
    if (run / "spans.jsonl").exists():
        dump(out / "spans.jsonl", [span(r) for r in lines(run / "spans.jsonl")])
    if (run / "host.jsonl").exists():
        dump(out / "host.jsonl", [host(r) for r in lines(run / "host.jsonl")])
    dump(out / "steps.jsonl", [step(r) for r in lines(run / "steps.jsonl")])
    quota = {}
    for name in ("quota-before.json", "quota-after.json"):
        if (run / name).exists():
            raw = json.loads((run / name).read_text())
            quota[name.removesuffix(".json")] = {
                "at": raw["at"],
                # `readings` is how many `rateLimit` queries one reading sent, so the
                # spike's own consumption is countable from what is committed.
                "buckets": {
                    b: {k: int(v[k]) for k in LIMIT_FIELDS}
                    | (
                        {"readings": [int(r) for r in v["readings"]]}
                        if "readings" in v
                        else {}
                    )
                    for b, v in raw["buckets"].items()
                },
            }
    (out / "quota.json").write_text(json.dumps(quota, indent=1, sort_keys=True) + "\n")
    if (run / "workload.json").exists():
        sizes = json.loads((run / "workload.json").read_text())
        workload = {
            label: [int(v) for v in value]
            for label, value in sizes.items()
            if re.fullmatch(r"project|task t[1-8]|document", label)
        }
        (out / "workload.json").write_text(json.dumps(workload, indent=1) + "\n")
    for name in ("probes.json", "cleanup.json"):
        if (run / name).exists():
            (out / name).write_text(
                json.dumps(redact(json.loads((run / name).read_text())), indent=1)
                + "\n"
            )
    found = leaks(out)
    if found:
        raise SystemExit(f"sanitize: {out} would carry identifying text: {found[:5]}")
    return out


def leaks(
    directory: Path, forbidden: tuple[str, ...] = ("seeded task", "Spike workload")
) -> list[str]:
    """Every line under `directory` holding a node id, a GitHub URL or a workload title."""
    found = []
    for path in sorted(directory.rglob("*")):
        if not path.is_file():
            continue
        for number, text in enumerate(path.read_text(errors="replace").splitlines(), 1):
            if (
                NODE_ID.search(text)
                or ISSUE_URL.search(text)
                or any(f in text for f in forbidden)
            ):
                found.append(f"{path.name}:{number}")
    return found


if __name__ == "__main__":
    write(Path(sys.argv[1]), Path(sys.argv[2]))
