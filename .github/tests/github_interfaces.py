#!/usr/bin/env python3
"""Hold the release's GitHub calls, and the double that stands GitHub in, to the pinned interface.

Called as `github_interfaces.py [<pin>]`, the pin defaulting to config/github-interfaces.toml,
whose header says what is checked. Quiet and exit 0 when everything agrees; otherwise one line
per disagreement on stderr, naming the call and the field, and exit 1.
"""

import copy
import importlib.util
import json
import re
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
problems: list[str] = []


def problem(message: str) -> None:
    problems.append(message)


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def segments(path: str) -> list[str]:
    """`assets[].id` -> ["assets", "[]", "id"]; `[].head.ref` -> ["[]", "head", "ref"]."""
    return [part for part in re.split(r"\.|(\[\])", path) if part]


def without(value: object, path: list[str]) -> object:
    """A copy of `value` with the field at `path` removed from every element a `[]` reaches."""
    value = copy.deepcopy(value)

    def strip(node: object, rest: list[str]) -> None:
        head, *tail = rest
        if head == "[]":
            for item in node if isinstance(node, list) else []:
                strip(item, tail)
        elif isinstance(node, dict) and head in node:
            if tail:
                strip(node[head], tail)
            else:
                del node[head]

    strip(value, path)
    return value


def shape(value: object, prefix: str = "") -> set[str]:
    """Every field path a JSON value carries, list members spelled `[]`."""
    paths: set[str] = set()
    if isinstance(value, dict):
        for key, member in value.items():
            path = f"{prefix}.{key}" if prefix else key
            paths.add(path)
            paths |= shape(member, path)
    elif isinstance(value, list):
        for member in value:
            paths |= shape(member, f"{prefix}[]")
    return paths


def holds_reads(call: str, answer: object, reads: list[str], parse) -> None:
    """The parser accepts the pinned answer, and each field it is said to read changes its result when gone."""
    try:
        whole = parse(answer)
    except Exception as error:  # noqa: BLE001 - any refusal of the pinned answer is the finding
        problem(f"{call}: the release's parser refuses GitHub's pinned answer: {error}")
        return
    pinned = shape(answer)
    for read in reads:
        spelled = read.replace(".[]", "[]")
        if spelled.lstrip("[]").lstrip(".") and spelled not in pinned and read != "errors":
            problem(f"{call}: `reads` names {read}, which GitHub's pinned answer does not carry")
            continue
        try:
            changed = parse(without(answer, segments(read))) != whole
        except Exception:  # noqa: BLE001 - a refusal is the parser reading the field
            changed = True
        if not changed and read != "errors":
            problem(f"{call}: removing {read} from GitHub's answer leaves the parser's result unchanged, so it does not read what the pin says it does")


def check_document(graphql: dict, document: str) -> None:
    """Walk the document's selections over the pinned schema slice."""
    objects, inputs, enums, arguments = graphql["objects"], graphql["inputs"], graphql["enums"], graphql["arguments"]
    if document.strip() != graphql["document"].strip():
        problem("graphql: scripts/enable-release-automerge.py sends a document other than the pinned one")
    body = document[document.index("{") :]
    tokens = re.findall(r"[A-Za-z_][A-Za-z0-9_]*|\$[A-Za-z_]+|[{}():,]", body)
    stack, current, last, at = [], "Mutation", None, 0
    while at < len(tokens):
        token = tokens[at]
        if token == "{":
            if last is not None:
                stack.append(current)
                current = objects.get(current, {}).get(last, "")
            last = None
        elif token == "}":
            current = stack.pop() if stack else current
        elif token == "(":
            # field(name: value, …): every name an argument of the field, an input object's
            # keys the input type's fields, and a bare value of an enum-typed field one of its values.
            depth, end = 1, at + 1
            while depth:
                depth += {"(": 1, ")": -1}.get(tokens[end], 0)
                end += 1
            declared = arguments.get(f"{current}.{last}", {})
            inner = tokens[at + 1 : end - 1]
            index = 0
            while index < len(inner):
                if index + 1 < len(inner) and inner[index + 1] == ":" and inner[index] not in "{}":
                    name = inner[index]
                    if name not in declared:
                        problem(f"graphql: {current}.{last} takes no argument {name}")
                    elif inner[index + 2] == "{":
                        fields = inputs.get(declared[name], {})
                        close = inner.index("}", index)
                        pairs = inner[index + 3 : close]
                        for key, value in zip(pairs[0::4], pairs[2::4], strict=False):
                            if key not in fields:
                                problem(f"graphql: {declared[name]} has no field {key}")
                            elif fields[key] in enums and value not in enums[fields[key]]:
                                problem(f"graphql: {value} is not a value of {fields[key]}")
                        index = close
                index += 1
            at = end
            continue
        elif token not in (":", ","):
            if current not in objects or token not in objects[current]:
                problem(f"graphql: {current} has no field {token} in the pinned schema slice")
            last = token
        at += 1


def double_answers(pin: dict, release: dict) -> dict:
    """What the loopback double answers each call with, on the paths the release journeys drive."""
    tag = release["tag"]
    with tempfile.TemporaryDirectory() as state:
        pulls = json.loads(next(entry for entry in pin["rest"] if entry["call"] == "open-pulls")["answer"])
        (Path(state) / "control.json").write_text(json.dumps({"releases": [tag], "pulls": [{"number": pull["number"], "node_id": pull["node_id"]} for pull in pulls]}))
        double = subprocess.Popen([sys.executable, str(ROOT / "scripts/loopback-release-registries.py"), state], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(200):
                if (Path(state) / "ports.json").exists():
                    break
                time.sleep(0.05)
            base = f"http://127.0.0.1:{json.loads((Path(state) / 'ports.json').read_text())['main']}/github/api"

            def ask(method: str, url: str, body: bytes | None = None) -> object:
                with urllib.request.urlopen(urllib.request.Request(url, data=body, method=method), timeout=30) as answer:
                    return json.loads(answer.read() or b"null")

            listed = ask("GET", f"{base}/repos/owner/name/releases/tags/{tag}")
            upload = listed["upload_url"].split("{", 1)[0]
            uploaded = ask("POST", f"{upload}?name=asset.tar.gz", b"bytes")
            return {
                "release-by-tag": ask("GET", f"{base}/repos/owner/name/releases/tags/{tag}"),
                "upload-asset": uploaded,
                "graphql": ask("POST", f"{base}/graphql", json.dumps({"query": pin["graphql"]["document"], "variables": {"pullRequestId": pulls[0]["node_id"]}}).encode()),
            }
        finally:
            double.terminate()
            double.wait(timeout=30)


def main() -> int:
    pin = tomllib.loads(Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "config/github-interfaces.toml").read_text(encoding="utf-8"))
    rest = {entry["call"]: entry for entry in pin["rest"]}
    answers = {call: json.loads(entry["answer"]) for call, entry in rest.items()} | {"graphql": json.loads(pin["graphql"]["answer"])}
    sys.path.insert(0, str(ROOT / "scripts"))
    release_cd = load("release_cd", ROOT / "scripts/release_cd.py")
    automerge = load("enable_release_automerge", ROOT / "scripts/enable-release-automerge.py")

    tag = rest["release-by-tag"]["tag"]
    holds_reads("release-by-tag", answers["release-by-tag"], rest["release-by-tag"]["reads"], lambda answer: release_cd.Release.parse(answer, tag))
    holds_reads("upload-asset", answers["upload-asset"], rest["upload-asset"]["reads"], release_cd.ReleaseAsset.parse)
    holds_reads("open-pulls", answers["open-pulls"], rest["open-pulls"]["reads"], lambda answer: [automerge.PullRequest.parse(pull) for pull in answer])

    check_document(pin["graphql"], automerge.ENABLE)
    try:
        pull = automerge.PullRequest.parse(answers["open-pulls"][0])
    except automerge.Refused:
        pull = None  # open-pulls has already reported it; the auto-merge answer needs its pull request
    if pull is not None:

        def enabled(answer: object) -> bool:
            automerge.confirm_enabled(answer, pull)
            return True

        holds_reads("graphql", answers["graphql"], pin["graphql"]["reads"], enabled)
        try:
            enabled({"errors": [{"message": "refused"}]})
            problem("graphql: an answer carrying `errors` is taken as auto-merge enabled")
        except automerge.Refused:
            pass

    try:
        served = double_answers(pin, rest["release-by-tag"])
    except (OSError, KeyError, ValueError, StopIteration, subprocess.SubprocessError) as error:
        problem(f"scripts/loopback-release-registries.py could not be asked what it answers: {error!r}")
        served = {}
    for call, answer in served.items():
        for field in sorted(shape(answer) - shape(answers[call])):
            problem(f"{call}: scripts/loopback-release-registries.py answers with {field}, which GitHub's pinned answer does not carry")

    for line in problems:
        print(f"github-interfaces: {line}", file=sys.stderr)
    if problems:
        print("github-interfaces: next: re-observe the call in config/github-interfaces.toml if GitHub moved, otherwise bring the script or the double back to the pin", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
