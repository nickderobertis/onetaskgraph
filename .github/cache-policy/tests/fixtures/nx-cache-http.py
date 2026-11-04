"""GitHub's HTTP boundary; Nx, the sidecar and production CLIs run unchanged."""

from __future__ import annotations

import json
import sys
import threading
import xml.etree.ElementTree as ET
from dataclasses import asdict, dataclass, field
from enum import Enum
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import NewType, TypeAlias
from urllib.parse import parse_qs, urlparse

CacheKey = NewType("CacheKey", str)
CacheVersion = NewType("CacheVersion", str)
RefScope = NewType("RefScope", str)
UploadToken = NewType("UploadToken", str)
DownloadToken = NewType("DownloadToken", str)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Operation(str, Enum):
    DOWNLOAD = "GetCacheEntryDownloadURL"
    CREATE = "CreateCacheEntry"
    FINALIZE = "FinalizeCacheEntryUpload"
    DELETE = "DELETE"


class StoreMode(Enum):
    LIVE = "live"
    FAULT = "fault"
    THROTTLED = "throttled"


@dataclass(frozen=True)
class EntryKey:
    key: CacheKey
    version: CacheVersion


@dataclass(frozen=True)
class ScopedKey:
    scope: RefScope
    entry: EntryKey


@dataclass(frozen=True)
class Call:
    method: Operation
    scope: RefScope
    key: CacheKey
    version: CacheVersion


@dataclass
class Upload:
    scope: RefScope
    entry: EntryKey
    blocks: dict[str, bytes] = field(default_factory=dict)
    body: bytes | None = None


class GithubCache:
    def __init__(self, directory: Path) -> None:
        self.entries: dict[ScopedKey, bytes] = {}
        self.uploads: dict[UploadToken, Upload] = {}
        self.downloads: dict[DownloadToken, bytes] = {}
        self.calls: list[Call] = []
        self.mode = StoreMode.LIVE
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *args: object) -> None:
                pass

            def reply(self, data: Json | bytes, status: int = 200) -> None:
                body = data if isinstance(data, bytes) else json.dumps(data).encode()
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Content-Type", "application/octet-stream" if isinstance(data, bytes) else "application/json")
                self.end_headers()
                self.wfile.write(body)

            def body(self) -> bytes:
                size = self.headers.get("Content-Length", "")
                if not size.isdecimal() or len(size) > 10 or int(size) > 128 * 1024 * 1024:
                    raise ValueError("invalid Content-Length")
                body = self.rfile.read(int(size))
                if len(body) != int(size):
                    raise ValueError("incomplete request body")
                return body

            def do_POST(self) -> None:
                try:
                    body = json.loads(self.body())
                    if not isinstance(body, dict):
                        raise ValueError("expected JSON object")
                    if self.path == "/control":
                        owner.mode = StoreMode(body.get("mode"))
                        self.reply({"ok": True})
                        return
                    key, version = body.get("key"), body.get("version")
                    if not isinstance(key, str) or not key or not isinstance(version, str) or not version:
                        raise ValueError("cache key and version must be nonempty strings")
                    scope = RefScope(self.headers.get("Authorization", "").removeprefix("Bearer "))
                    if not scope.startswith("refs/"):
                        raise ValueError("missing cache scope")
                    entry = EntryKey(CacheKey(key), CacheVersion(version))
                    method = Operation(self.path.rsplit("/", 1)[-1])
                    owner.calls.append(Call(method, scope, entry.key, entry.version))
                    if owner.mode is StoreMode.FAULT:
                        self.reply({"code": "internal", "msg": "store unavailable"}, 500)
                        return
                    match method:
                        case Operation.DOWNLOAD:
                            bases_file = directory / "bases.json"
                            bases = json.loads(bases_file.read_text()) if bases_file.exists() else {}
                            if not isinstance(bases, dict) or not all(isinstance(k, str) and isinstance(v, str) for k, v in bases.items()):
                                raise ValueError("invalid fixture base scopes")
                            found = next((owner.entries[ScopedKey(RefScope(s), entry)] for s in [scope, bases.get(scope, "refs/heads/main")] if ScopedKey(RefScope(s), entry) in owner.entries), None)
                            if found is None:
                                self.reply({"ok": False, "signedDownloadUrl": "", "matchedKey": ""})
                            else:
                                token = DownloadToken(str(len(owner.downloads)))
                                owner.downloads[token] = found
                                self.reply({"ok": True, "signedDownloadUrl": owner.url + "download/" + token, "matchedKey": key})
                        case Operation.CREATE:
                            if owner.mode is StoreMode.THROTTLED:
                                self.reply({"code": "resource_exhausted", "msg": "rate limited"}, 429)
                            else:
                                token = UploadToken(str(len(owner.uploads)))
                                owner.uploads[token] = Upload(scope, entry)
                                self.reply({"ok": True, "signedUploadUrl": owner.url + "upload/" + token})
                        case Operation.FINALIZE:
                            uploads = [u for u in owner.uploads.values() if u.scope == scope and u.entry == entry]
                            if not uploads or uploads[-1].body is None:
                                raise ValueError("finalize requires a completed upload")
                            owner.entries[ScopedKey(scope, entry)] = uploads[-1].body
                            self.reply({"ok": True, "entryId": "1"})
                        case _:
                            self.reply({"error": "unknown method"}, 404)
                except (ValueError, TypeError) as error:
                    self.reply({"error": str(error)}, 400)

            def do_PUT(self) -> None:
                try:
                    parsed = urlparse(self.path)
                    if not parsed.path.startswith("/upload/"):
                        raise ValueError("unknown upload route")
                    token = UploadToken(parsed.path.rsplit("/", 1)[-1])
                    if token not in owner.uploads:
                        raise ValueError("unknown upload token")
                    upload = owner.uploads[token]
                    body = self.body()
                    query = parse_qs(parsed.query)
                    match query.get("comp"):
                        case ["block"]:
                            block = query.get("blockid")
                            if not block or len(block) != 1 or not block[0]:
                                raise ValueError("missing block id")
                            upload.blocks[block[0]] = body
                        case ["blocklist"]:
                            ids = [node.text for node in ET.fromstring(body)]
                            if not ids or any(name not in upload.blocks for name in ids):
                                raise ValueError("unknown upload block")
                            upload.body = b"".join(upload.blocks[name] for name in ids)
                        case None:
                            upload.body = body
                        case _:
                            raise ValueError("unsupported upload operation")
                    self.reply(b"", 201)
                except (ValueError, ET.ParseError) as error:
                    self.reply({"error": str(error)}, 400)

            def do_GET(self) -> None:
                try:
                    parsed = urlparse(self.path)
                    if parsed.path.startswith("/repos/"):
                        metadata_file = directory / "routes.json"
                        routes = json.loads(metadata_file.read_text()) if metadata_file.exists() else {}
                        if not isinstance(routes, dict):
                            raise ValueError("fixture routes must be an object")
                        value = routes.get(self.path, routes.get(parsed.path))
                        if value is None:
                            self.reply({"message": "No fixture route: " + self.path}, 404)
                            return
                        query = parse_qs(parsed.query)
                        page, size = query.get("page", ["1"]), query.get("per_page", ["100"])
                        if any(len(v) != 1 or not v[0].isdecimal() or not 0 < int(v[0]) <= 1000 for v in [page, size]):
                            raise ValueError("invalid pagination")
                        if self.path not in routes and "page" in query:
                            start, count = (int(page[0]) - 1) * int(size[0]), int(size[0])
                            match value:
                                case list():
                                    value = value[start : start + count]
                                case dict():
                                    value = {k: v[start : start + count] if isinstance(v, list) else v for k, v in value.items()}
                        if isinstance(value, dict) and set(value) == {"fixture_file"}:
                            file = value["fixture_file"]
                            if not isinstance(file, str) or Path(file).name != file:
                                raise ValueError("invalid fixture file")
                            self.reply((directory / file).read_bytes())
                        else:
                            self.reply(value)
                        return
                    if not parsed.path.startswith("/download/"):
                        raise ValueError("unknown download route")
                    token = DownloadToken(parsed.path.rsplit("/", 1)[-1])
                    if token not in owner.downloads:
                        raise ValueError("unknown download token")
                    self.reply(owner.downloads[token])
                except (ValueError, OSError) as error:
                    self.reply({"error": str(error)}, 400)

            def do_DELETE(self) -> None:
                owner.calls.append(Call(Operation.DELETE, RefScope(""), CacheKey(""), CacheVersion("")))
                self.reply(b"", 405)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}/"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def main() -> None:
    if len(sys.argv) != 2 or not Path(sys.argv[1]).is_dir():
        raise SystemExit("usage: nx-cache-http.py EXISTING_FIXTURE_DIRECTORY")
    directory = Path(sys.argv[1])
    cache = GithubCache(directory)
    (directory / "url").write_text(cache.url)
    try:
        sys.stdin.read()
    finally:
        cache.close()
        (directory / "calls.json").write_text(json.dumps([asdict(call) for call in cache.calls]))
        (directory / "entries.json").write_text(json.dumps([asdict(key) for key in cache.entries]))


if __name__ == "__main__":
    main()
