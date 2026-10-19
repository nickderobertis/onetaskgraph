#!/usr/bin/env python3
"""Hold the release's GitHub calls, and the double that stands GitHub in, to the pinned interface.

Called as `github_interfaces.py [<pin>]`, the pin defaulting to config/github-interfaces.toml,
whose header says what is checked. Quiet and exit 0 when everything agrees; otherwise one line
per disagreement on stderr, naming the call and the field, and exit 1.

Called as `github_interfaces.py --answer <call>`, it prints that call's pinned answer as JSON,
which is how the release journeys build what their double serves from the pin.
"""

# llmlint: ignore-file[async_typed_clients_at_boundaries] Every request here goes to a loopback server this check starts itself — the double, or the recording stand-in the release scripts are pointed at — strictly one after another, each answer read before the next is sent; the scripts under check use the same standard-library client for the same reason their own directives give.

import contextlib
import copy
import importlib.util
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import urllib.parse
import urllib.request
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]
CALLS = ("release-by-tag", "delete-asset", "upload-asset", "open-pulls", "graphql")
# The order a release's GitHub calls come in: an asset upload that was interrupted is taken
# back before it is sent again, and auto-merge is asked for the pull request the list named.
SEQUENCE = list(CALLS)

Json = None | bool | int | float | str | list | dict  # a decoded JSON value


class PinError(Exception):
    """The pin itself is not what this check can read."""


@dataclass(frozen=True)
class Call:
    name: str
    host: str
    method: str
    path: str
    status: int
    reads: tuple[str, ...]
    answer: Json

    def matches(self, method: str, target: str) -> bool:
        pattern = re.sub(r"\\\{[a-z_]+\\\}", "[^/?&]+", re.escape(self.path))
        return method == self.method and re.fullmatch(pattern, target) is not None


@dataclass(frozen=True)
class Sent:
    """One request a release script made of the recording stand-in."""

    method: str
    target: str
    body: bytes


@dataclass(frozen=True)
class Served:
    """What the loopback double answered one call with."""

    status: int
    answer: Json


@dataclass(frozen=True)
class Schema:
    objects: dict[str, dict[str, str]]
    arguments: dict[str, dict[str, str]]
    inputs: dict[str, dict[str, str]]
    enums: dict[str, tuple[str, ...]]


@dataclass(frozen=True)
class Pin:
    calls: dict[str, Call]
    tag: str
    document: str
    schema: Schema


def required(table: object, key: str, kind: type, where: str):
    if not isinstance(table, dict) or not isinstance(table.get(key), kind) or isinstance(table.get(key), bool) is not (kind is bool):
        raise PinError(f"{where}: `{key}` must be of type {kind.__name__}")
    return table[key]


def names(table: object, where: str) -> dict[str, str]:
    if not isinstance(table, dict) or not all(isinstance(value, str) for value in table.values()):
        raise PinError(f"{where}: must map names to type names")
    return dict(table)


def call(table: object, name: str) -> Call:
    reads = required(table, "reads", list, name)
    if not all(isinstance(read, str) for read in reads):
        raise PinError(f"{name}: `reads` must list field paths")
    text = required(table, "answer", str, name)
    try:
        answer = json.loads(text) if text.strip() else None
    except json.JSONDecodeError as error:
        raise PinError(f"{name}: `answer` is not JSON: {error}") from error
    for key in ("documentation", "observed", "observed_from"):
        required(table, key, str, name)
    return Call(name, required(table, "host", str, name), required(table, "method", str, name), required(table, "path", str, name), required(table, "status", int, name), tuple(reads), answer)


def read_pin(path: Path) -> Pin:
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise PinError(f"{path}: {error}") from error
    calls = {}
    for table in required(raw, "rest", list, "the pin"):
        name = required(table, "call", str, "a [[rest]] entry")
        calls[name] = call(table, name)
    graphql = required(raw, "graphql", dict, "the pin")
    calls["graphql"] = call(graphql, "graphql")
    if sorted(calls) != sorted(CALLS):
        raise PinError(f"the pin describes {sorted(calls)}, not the release's calls {sorted(CALLS)}")
    enums = required(graphql, "enums", dict, "graphql")
    if not all(isinstance(values, list) and all(isinstance(value, str) for value in values) for values in enums.values()):
        raise PinError("graphql.enums: must map each enum to its values")
    schema = Schema(
        {name: names(fields, f"graphql.objects.{name}") for name, fields in required(graphql, "objects", dict, "graphql").items()},
        {name: names(fields, f"graphql.arguments.{name}") for name, fields in required(graphql, "arguments", dict, "graphql").items()},
        {name: names(fields, f"graphql.inputs.{name}") for name, fields in required(graphql, "inputs", dict, "graphql").items()},
        {name: tuple(values) for name, values in enums.items()},
    )
    tag = required(next(table for table in raw["rest"] if table.get("call") == "release-by-tag"), "tag", str, "release-by-tag")
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise PinError(f"release-by-tag: `tag` is {tag!r}, not a release tag such as v0.7.1")
    # What drive_scripts and double_answers index into, held here so they can.
    match calls["release-by-tag"].answer:
        case {"upload_url": str(), "assets": [{"name": str()}, *_]}:
            pass
        case _:
            raise PinError("release-by-tag: `answer` must be a release with an upload_url and at least one named asset")
    match calls["upload-asset"].answer:
        case {"name": str(), "size": int()}:
            pass
        case _:
            raise PinError("upload-asset: `answer` must be a release asset with its name and size")
    match calls["open-pulls"].answer:
        case [{"node_id": str(), "head": {"ref": str(), "repo": {"full_name": str()}}}, *rest] if all(isinstance(pull, dict) for pull in rest):
            pass
        case _:
            raise PinError("open-pulls: `answer` must list at least one pull request, the first with its node_id, head ref and head repository's full_name")
    if not isinstance(calls["graphql"].answer, dict) or calls["delete-asset"].answer is not None:
        raise PinError("graphql: `answer` must be a response object, and delete-asset's empty")
    for name, entry in calls.items():
        if not re.fullmatch(r"[a-z0-9.-]+\.github\.com", entry.host) or not entry.path.startswith("/"):
            raise PinError(f"{name}: `host` must be a github.com host and `path` start with /")
    return Pin(calls, tag, required(graphql, "document", str, "graphql"), schema)


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def segments(path: str) -> list[str]:
    """`assets[].id` -> ["assets", "[]", "id"]; `[].head.ref` -> ["[]", "head", "ref"]."""
    return [part for part in re.split(r"\.|(\[\])", path) if part]


def without(value: Json, path: list[str]) -> Json:
    """A copy of `value` with the field at `path` removed from every element a `[]` reaches."""
    value = copy.deepcopy(value)

    def strip(node: Json, rest: list[str]) -> None:
        match rest, node:
            case ["[]", *tail], list():
                for item in node:
                    strip(item, tail)
            case [head], dict() if head in node:
                del node[head]
            case [head, *tail], dict() if head in node:
                strip(node[head], tail)

    strip(value, path)
    return value


def shape(value: Json, prefix: str = "") -> set[str]:
    """Every field path a JSON value carries, list members spelled `[]`."""
    match value:
        case dict():
            return set().union(*({f"{prefix}.{key}".lstrip("."), *shape(member, f"{prefix}.{key}".lstrip("."))} for key, member in value.items()))
        case list():
            return set().union(*(shape(member, f"{prefix}[]") for member in value))
        case _:
            return set()


class Findings:
    def __init__(self) -> None:
        self.lines: list[str] = []

    def add(self, message: str) -> None:
        self.lines.append(message)

    def holds_reads(self, call: Call, parse: Callable[[Json], object]) -> None:
        """The parser accepts the pinned answer, every field `reads` names changes what it
        answers when gone, and no other field does."""
        try:
            whole = parse(call.answer)
        except Exception as error:  # noqa: BLE001 - any refusal of the pinned answer is the finding
            self.add(f"{call.name}: the release's parser refuses GitHub's pinned answer: {error}")
            return
        carried = shape(call.answer)
        named = {read.replace(".[]", "[]") for read in call.reads}
        for read in sorted(named - carried - {"errors"}):
            self.add(f"{call.name}: `reads` names {read}, which GitHub's pinned answer does not carry")
        for field in sorted(carried | (named & {"errors"})):
            ancestor = any(read.startswith((f"{field}.", f"{field}[]")) for read in named)
            try:
                same = parse(without(call.answer, segments(field))) == whole
            except Exception:  # noqa: BLE001 - a refusal is the parser reading the field
                same = False
            if field in named and same and field != "errors":
                self.add(f"{call.name}: removing {field} from GitHub's answer leaves the parser's result unchanged, so it does not read what the pin says it does")
            elif field not in named and not ancestor and not same:
                self.add(f"{call.name}: the parser reads {field}, which `reads` does not name")

    def check_document(self, pin: Pin, document: str) -> None:
        """Walk the document's selections over the pinned schema slice."""
        schema = pin.schema
        if document.strip() != pin.document.strip():
            self.add("graphql: scripts/enable-release-automerge.py sends a document other than the pinned one")
        tokens = re.findall(r"[A-Za-z_][A-Za-z0-9_]*|\$[A-Za-z_]+|[{}():,]", document[document.index("{") :])
        stack, current, last, at = [], "Mutation", None, 0
        while at < len(tokens):
            match tokens[at]:
                case "{":
                    if last is not None:
                        stack.append(current)
                        current = schema.objects.get(current, {}).get(last, "")
                    last = None
                case "}":
                    current = stack.pop() if stack else current
                case "(":
                    at = self.check_arguments(schema, current, last, tokens, at)
                    continue
                case ":" | ",":
                    pass
                case token:
                    if token not in schema.objects.get(current, {}):
                        self.add(f"graphql: {current} has no field {token} in the pinned schema slice")
                    last = token
            at += 1

    def check_arguments(self, schema: Schema, owner: str, field: str | None, tokens: list[str], at: int) -> int:
        """field(name: value, …): each name an argument of the field, an input object's keys
        the input type's fields, and a bare value of an enum-typed field one of its values."""
        depth, end = 1, at + 1
        while depth:
            depth += {"(": 1, ")": -1}.get(tokens[end], 0)
            end += 1
        declared, inner, index = schema.arguments.get(f"{owner}.{field}", {}), tokens[at + 1 : end - 1], 0
        while index + 2 < len(inner):
            if inner[index + 1] == ":" and inner[index] not in "{}":
                if inner[index] not in declared:
                    self.add(f"graphql: {owner}.{field} takes no argument {inner[index]}")
                elif inner[index + 2] == "{":
                    fields, close = schema.inputs.get(declared[inner[index]], {}), inner.index("}", index)
                    pairs = inner[index + 3 : close]
                    for key, value in zip(pairs[0::4], pairs[2::4], strict=False):
                        if key not in fields:
                            self.add(f"graphql: {declared[inner[index]]} has no field {key}")
                        elif fields[key] in schema.enums and value not in schema.enums[fields[key]]:
                            self.add(f"graphql: {value} is not a value of {fields[key]}")
                    index = close
            index += 1
        return end


def served_pulls(pin: Pin) -> list[dict]:
    """The pinned pull-request list, which read_pin has held to a non-empty list of objects."""
    return pin.calls["open-pulls"].answer


def check_hosts(pin: Pin, findings: Findings, release_cd, automerge) -> None:
    """Each pinned call goes to the host the scripts send it to by default: the API host they
    default to, and the upload host the pinned release names, which release_cd trusts."""
    for name, default in (("release_cd.py", release_cd.DEFAULT_GITHUB_API), ("enable-release-automerge.py", automerge.DEFAULT_API)):
        for call in pin.calls.values():
            if call.name != "upload-asset" and urllib.parse.urlsplit(default).netloc != call.host:
                findings.add(f"{call.name}: scripts/{name} sends it to {urllib.parse.urlsplit(default).netloc}, where the pin says {call.host}")
    upload_url = pin.calls["release-by-tag"].answer.get("upload_url", "") if isinstance(pin.calls["release-by-tag"].answer, dict) else ""
    if urllib.parse.urlsplit(upload_url).netloc != pin.calls["upload-asset"].host:
        findings.add(f"upload-asset: the pinned release names {upload_url!r} to upload to, where the pin says {pin.calls['upload-asset'].host}")
    github = release_cd.GitHub(release_cd.DEFAULT_GITHUB_API, release_cd.RepositoryName("owner/name"), release_cd.Token("pinned-interface"))
    if not github.trusts(upload_url.split("{", 1)[0]):
        findings.add(f"upload-asset: scripts/release_cd.py would not send its token to {upload_url!r}, the upload host the pin says GitHub names")


class Recorder(BaseHTTPRequestHandler):
    """GitHub as the pin describes it: each call answered with its pinned status and an answer
    `answers` shapes from the pinned one, every request recorded."""

    answers: dict[str, Json] = {}
    calls: list[Call] = []
    sent: list[Sent] = []

    def log_message(self, *_: object) -> None:
        pass

    def answer(self) -> None:
        # Only a decimal length is read: a negative one would read to the end of the stream
        # and a malformed one would raise, so either is refused before any body is read.
        declared = self.headers.get("Content-Length") or "0"
        if not declared.isdigit():
            self.send_error(400, "unusable Content-Length")
            return
        body = self.rfile.read(int(declared))
        self.sent.append(Sent(self.command, self.path, body))
        call = next((call for call in self.calls if call.matches(self.command, self.path)), None)
        payload = b"" if call is None or self.answers[call.name] is None else json.dumps(self.answers[call.name]).encode()
        self.send_response(404 if call is None else call.status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = do_POST = do_DELETE = answer


@contextlib.contextmanager
def environment(**values: str | None):
    saved = {name: os.environ.get(name) for name in values}
    os.environ.update({name: value for name, value in values.items() if value is not None})
    for name in (name for name, value in values.items() if value is None):
        os.environ.pop(name, None)
    try:
        yield
    finally:
        for name, value in saved.items():
            os.environ.pop(name, None) if value is None else os.environ.__setitem__(name, value)


def drive_scripts(pin: Pin, findings: Findings, release_cd, automerge) -> None:
    """Run both scripts' real GitHub code against a server answering as the pin does, and hold
    every request they send to a call the pin describes, in the release's order."""
    server = ThreadingHTTPServer(("127.0.0.1", 0), Recorder)
    base = f"http://127.0.0.1:{server.server_port}"
    calls = pin.calls
    release = copy.deepcopy(calls["release-by-tag"].answer)
    pull = copy.deepcopy(served_pulls(pin)[0])
    try:
        repository = pull["head"]["repo"]["full_name"]
        upload = re.sub(r"^https?://[^/]+", base, release["upload_url"])
        release["upload_url"] = upload
        pull["head"]["ref"] = f"release-plz-{pin.tag}"
    except (KeyError, TypeError) as error:
        findings.add(f"the pinned release or pull request lacks what the release reads: {error!r}")
        return
    with tempfile.TemporaryDirectory() as scratch:
        built = Path(scratch) / f"onetaskgraph-{pin.tag}-x86_64-unknown-linux-gnu.tar.gz"
        built.write_bytes(b"one build of the archive")
        # The release already holds this file, short and not `uploaded`, as an interrupted upload
        # leaves it: so the step takes it back and sends it again, making every call it can make.
        release["assets"][0] |= {"name": built.name, "state": "starter"}
        Recorder.calls, Recorder.sent = list(calls.values()), []
        Recorder.answers = {
            "release-by-tag": release,
            "delete-asset": None,
            "upload-asset": calls["upload-asset"].answer | {"name": built.name, "size": built.stat().st_size, "state": "uploaded"},
            "open-pulls": [pull],
            "graphql": calls["graphql"].answer,
        }
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        quiet = io.StringIO()
        try:
            with environment(GH_TOKEN="pinned-interface", GITHUB_TOKEN=None, GITHUB_REPOSITORY=repository, GITHUB_API_URL=base, GITHUB_GRAPHQL_URL=None), contextlib.redirect_stderr(quiet), contextlib.redirect_stdout(quiet):
                try:
                    release_cd.publish_github(release_cd.ReleaseTag(pin.tag), [built])
                except release_cd.ReleaseStepFailed as error:
                    findings.add(f"scripts/release_cd.py refuses GitHub answering as pinned: {error}")
                if automerge.main() != 0:
                    findings.add(f"scripts/enable-release-automerge.py refuses GitHub answering as pinned: {quiet.getvalue().strip()[-300:]}")
        finally:
            server.shutdown()
            server.server_close()
    made = [next((call.name for call in calls.values() if call.matches(sent.method, sent.target)), f"{sent.method} {sent.target}") for sent in Recorder.sent]
    for sent in Recorder.sent:
        if not any(call.matches(sent.method, sent.target) for call in calls.values()):
            findings.add(f"a release script sent {sent.method} {sent.target}, which the pin describes as no call of GitHub's")
    if made != SEQUENCE:
        findings.add(f"the release scripts made {made}, not the pinned calls in order {SEQUENCE}")
    graphql = next((json.loads(sent.body) for sent in Recorder.sent if calls["graphql"].matches(sent.method, sent.target)), None)
    if graphql is not None and graphql != {"query": pin.document.strip(), "variables": {"pullRequestId": pull["node_id"]}}:
        findings.add(f"graphql: scripts/enable-release-automerge.py sent {json.dumps(graphql)[:200]}, not the pinned document for the listed pull request")


def double_answers(pin: Pin) -> dict[str, Served]:
    """What the loopback double answers each call with, on the paths the release journeys drive."""
    with tempfile.TemporaryDirectory() as state:
        control = {"releases": [pin.tag], "pulls": served_pulls(pin)}
        (Path(state) / "control.json").write_text(json.dumps(control))
        double = subprocess.Popen([sys.executable, str(ROOT / "scripts/loopback-release-registries.py"), state], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(200):
                if (Path(state) / "ports.json").exists():
                    break
                time.sleep(0.05)
            base = f"http://127.0.0.1:{json.loads((Path(state) / 'ports.json').read_text())['main']}/github/api"

            def ask(method: str, url: str, body: bytes | None = None) -> Served:
                with urllib.request.urlopen(urllib.request.Request(url, data=body, method=method), timeout=30) as answer:
                    return Served(answer.status, json.loads(answer.read() or b"null"))

            listed = ask("GET", f"{base}/repos/owner/name/releases/tags/{pin.tag}").answer
            uploaded = ask("POST", f"{listed['upload_url'].split('{', 1)[0]}?name=asset.tar.gz", b"bytes")
            answers = {
                "release-by-tag": ask("GET", f"{base}/repos/owner/name/releases/tags/{pin.tag}"),
                "upload-asset": uploaded,
                "open-pulls": ask("GET", f"{base}/repos/owner/name/pulls?state=open&per_page=100"),
                "graphql": ask("POST", f"{base}/graphql", json.dumps({"query": pin.document, "variables": {"pullRequestId": served_pulls(pin)[0]["node_id"]}}).encode()),
            }
            answers["delete-asset"] = ask("DELETE", f"{base}/repos/owner/name/releases/assets/{uploaded.answer['id']}")
            return answers
        finally:
            double.terminate()
            double.wait(timeout=30)


def main(arguments: list[str]) -> int:
    findings = Findings()
    match arguments:
        case ["--answer", name]:
            path, answer = ROOT / "config/github-interfaces.toml", name
        case [path]:
            path, answer = Path(path), None
        case []:
            path, answer = ROOT / "config/github-interfaces.toml", None
        case _:
            print("github-interfaces: usage: github_interfaces.py [<pin> | --answer <call>]", file=sys.stderr)
            return 2
    try:
        pin = read_pin(Path(path))
    except PinError as error:
        print(f"github-interfaces: {error}", file=sys.stderr)
        print("github-interfaces: next: bring config/github-interfaces.toml back to the shape its header describes", file=sys.stderr)
        return 1
    if answer is not None:
        if answer not in pin.calls:
            print(f"github-interfaces: no pinned call {answer}; the pin describes {sorted(pin.calls)}", file=sys.stderr)
            return 2
        print(json.dumps(pin.calls[answer].answer))
        return 0

    sys.path.insert(0, str(ROOT / "scripts"))
    release_cd = load("release_cd", ROOT / "scripts/release_cd.py")
    automerge = load("enable_release_automerge", ROOT / "scripts/enable-release-automerge.py")
    calls = pin.calls

    findings.holds_reads(calls["release-by-tag"], lambda answer: release_cd.Release.parse(answer, pin.tag))
    findings.holds_reads(calls["upload-asset"], release_cd.ReleaseAsset.parse)
    findings.holds_reads(calls["open-pulls"], lambda answer: [automerge.PullRequest.parse(pull) for pull in answer])
    findings.check_document(pin, automerge.ENABLE)
    check_hosts(pin, findings, release_cd, automerge)
    try:
        pull = automerge.PullRequest.parse(served_pulls(pin)[0])
    except (automerge.Refused, IndexError, TypeError):
        pull = None  # open-pulls has already reported it; the auto-merge answer needs its pull request
    if pull is not None:

        def enabled(answer: Json) -> bool:
            automerge.confirm_enabled(answer, pull)
            return True

        findings.holds_reads(calls["graphql"], enabled)
        drive_scripts(pin, findings, release_cd, automerge)

    try:
        served = double_answers(pin)
    except (OSError, KeyError, ValueError, TypeError, subprocess.SubprocessError) as error:
        findings.add(f"scripts/loopback-release-registries.py could not be asked what it answers: {error!r}")
        served = {}
    for name, answer in served.items():
        if answer.status != calls[name].status:
            findings.add(f"{name}: scripts/loopback-release-registries.py answers HTTP {answer.status}, where GitHub answers {calls[name].status}")
        for field in sorted(shape(answer.answer) - shape(calls[name].answer)):
            findings.add(f"{name}: scripts/loopback-release-registries.py answers with {field}, which GitHub's pinned answer does not carry")

    for line in findings.lines:
        print(f"github-interfaces: {line}", file=sys.stderr)
    if findings.lines:
        print("github-interfaces: next: re-observe the call in config/github-interfaces.toml if GitHub moved, otherwise bring the script or the double back to the pin", file=sys.stderr)
    return 1 if findings.lines else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
