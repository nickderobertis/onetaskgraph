#!/usr/bin/env python3
"""Every registry a release publishes to, on loopback, for the release journeys.

Called as `loopback-release-registries.py <state directory>`: it binds two ports the kernel
picks — `main`, which serves everything, and `checksums`, which serves the GitHub Release
downloads again so scripts/install.sh sees the archive and its checksum come from two
origins, as it does from github.com — writes them to `<state>/ports.json`, and answers until
it is killed. Each registry sits under its own prefix of `main`:

  /github/api        the REST calls a release makes: a release by tag, its assets, deleting
                     one, the open pull requests, and GraphQL's enablePullRequestAutoMerge
  /github/uploads    the asset upload a release's `upload_url` names
  /github/download   <tag>/<asset>, as github.com/<repo>/releases/download serves them
  /api/v3            the read release-plz makes of a forge it finds at a repository URL
  /pypi              the legacy upload API, the simple index (HTML and JSON) and its files
  /npm               packuments, publication and tarballs, as registry.npmjs.org
  /crates            crates.io's publish and version API, and a sparse index whose `dl`
                     serves what was published; any other index file is fetched from
                     index.crates.io, so cargo resolves the published crates' own
                     dependencies exactly as it would against crates.io — except a crate
                     named onetaskgraph*, which is this repository's and the double's alone

What the double answers is controlled per request by `<state>/control.json`, which the
journeys rewrite between runs: `releases` (tags that have a GitHub Release), `pulls` (the
open pull requests), `refuse` (uploads and calls answered with an error: entries name a
`registry` and a `file`, `package` or `action`, and a GitHub upload marked `partial` leaves a
short asset behind, as an interrupted one does), `hide` (names read as absent), `corrupt`
(files served as garbage), and `relabel` (`{"from": v1, "to": v2}`: every read lists v1 as
v2), and `release_answer`, `upload_host` and `automerge_answer`, which replace what GitHub
answers. Every request is one JSON line in `<state>/requests.jsonl`, with the file or package a
publication carried, so a journey can assert exactly what was sent.
"""

# llmlint: ignore-file[boundary_inputs_validated] The one argument is a directory the release journeys create and pass. The wire input is each registry's own protocol, read only as far as the double needs to answer it, and a request it cannot read is answered 400 — the journeys exercise it with the real uv, npm, cargo and pip clients rather than hand-built bodies.
# llmlint: ignore-file[async_typed_clients_at_boundaries] The one outbound call, the index.crates.io proxy, answers one cargo request at a time inside a ThreadingHTTPServer handler; the double is a test fixture whose fidelity is to the registry protocols, so it uses the standard library rather than a service client.
# llmlint: ignore-file[modern_domain_modeling] What the shapes below model is four registries' own JSON and form protocols, which this file stands in for; a typed model here would restate those protocols inside the double whose fidelity to them is the whole point.

import base64
import email
import email.policy
import hashlib
import html
import itertools
import json
import re
import socketserver
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

state = Path(sys.argv[1])
lock = threading.Lock()
github_assets = {}  # (tag, name) -> {"id", "bytes"}
github_ids = {}  # asset id -> (tag, name)
pypi_files = {}  # filename -> {"project", "bytes", "sha256"}
npm_packages = {}  # name -> {version -> {"manifest", "file", "bytes"}}
crates = {}  # name -> {version -> {"entry", "bytes"}}
proxied_index = {}
ports = {}
asset_ids = itertools.count(1)


def control():
    try:
        return json.loads((state / "control.json").read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def refused(registry, **names):
    for entry in control().get("refuse", []):
        if entry.get("registry") == registry and all(entry.get(key) in (None, value) for key, value in names.items()):
            return entry.get("status", 500)
    return None


def partial(name):
    return any(entry.get("partial") and entry.get("file") == name for entry in control().get("refuse", []))


def hidden(name):
    return name in control().get("hide", [])


def relabel(text):
    rule = control().get("relabel")
    return text.replace(rule["from"], rule["to"]) if rule else text


def served_bytes(name, payload):
    return b"this is not the file that was published\n" if name in control().get("corrupt", []) else payload


def normalise(project):
    return re.sub(r"[-_.]+", "-", project).lower()


def index_path(name):
    name = name.lower()
    if len(name) <= 2:
        return f"{len(name)}/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        """Quiet: the journeys read requests.jsonl."""

    def record(self, **fields):
        with lock, open(state / "requests.jsonl", "a", encoding="utf-8") as log:
            log.write(json.dumps({"method": self.command, "path": self.path, **fields}) + "\n")

    def body(self):
        length = self.headers.get("Content-Length", "0")
        return self.rfile.read(int(length)) if length.isdigit() else b""

    def send(self, status, payload=b"", content_type="application/json", headers=None):
        match payload:
            case dict() | list():
                payload = json.dumps(payload).encode()
            case str():
                payload = payload.encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    def base(self):
        return f"http://127.0.0.1:{ports['main']}"


    def route(self):
        path = urllib.parse.urlsplit(self.path).path
        for prefix, handler in (
            ("/github/api", self.github_api),
            ("/github/uploads", self.github_upload),
            ("/github/download/", self.github_download),
            ("/api/v3", self.github_enterprise),
            ("/pypi", self.pypi),
            ("/npm", self.npm),
            ("/crates", self.crates),
        ):
            if path.startswith(prefix):
                try:
                    handler(path[len(prefix):])
                except (ValueError, KeyError, json.JSONDecodeError) as error:
                    self.send(400, {"message": f"unreadable request: {error}"})
                return
        self.send(404, {"message": "no registry here"})

    do_GET = do_PUT = do_POST = do_DELETE = do_HEAD = route


    def github_api(self, path):
        if path == "/graphql" and self.command == "POST":
            query = json.loads(self.body() or b"{}")
            pull = query.get("variables", {}).get("pullRequestId")
            self.record(registry="github", action="graphql", query=query.get("query", ""), pull_request_id=pull)
            status = refused("github", action="automerge")
            if status:
                self.send(200, {"errors": [{"type": "UNPROCESSABLE", "message": "Pull request is not in the correct state to enable auto-merge"}]})
                return
            answer = control().get("automerge_answer")
            if answer is None:
                number = next((p["number"] for p in control().get("pulls", []) if isinstance(p, dict) and p.get("node_id") == pull), None)
                answer = {"data": {"enablePullRequestAutoMerge": {"pullRequest": {"number": number, "autoMergeRequest": {"mergeMethod": "SQUASH"}}}}}
            self.send(200, answer)
            return
        match = re.fullmatch(r"/repos/[^/]+/[^/]+/pulls", path)
        if match and self.command == "GET":
            self.record(registry="github", action="list-pulls")
            if refused("github", action="list-pulls"):
                self.send(502, {"message": "Server Error"})
                return
            self.send(200, control().get("pulls", []))
            return
        match = re.fullmatch(r"/repos/([^/]+/[^/]+)/releases/tags/(.+)", path)
        if match and self.command == "GET":
            repository, tag = match.group(1), urllib.parse.unquote(match.group(2))
            self.record(registry="github", action="read-release", tag=tag)
            if tag not in control().get("releases", []):
                self.send(404, {"message": "Not Found"})
                return
            release_id = abs(hash(tag)) % 100000
            with lock:
                assets = [
                    {"id": asset["id"], "name": relabel(name), "size": len(asset["bytes"]), "state": asset.get("state", "uploaded")}
                    for (asset_tag, name), asset in sorted(github_assets.items())
                    if asset_tag == tag and not hidden(name)
                ]
            upload = control().get("upload_host", f"{self.base()}/github/uploads")
            release = {"id": release_id, "tag_name": tag, "upload_url": f"{upload}/repos/{repository}/releases/{release_id}/assets{{?name,label}}", "assets": assets}
            self.send(200, control().get("release_answer", release))
            return
        match = re.fullmatch(r"/repos/[^/]+/[^/]+/releases/assets/(\d+)", path)
        if match and self.command == "DELETE":
            with lock:
                key = github_ids.pop(int(match.group(1)), None)
                if key:
                    github_assets.pop(key, None)
            self.record(registry="github", action="delete-asset", file=key[1] if key else None)
            self.send(204 if key else 404, b"")
            return
        self.send(404, {"message": "Not Found"})

    def github_enterprise(self, path):
        """The forge reads release-plz makes of a repository URL on this host.

        The pinned release tool derives a GitHub Enterprise API root, `<host>/api/v3`, from
        any repository URL that is not github.com's, so a journey points it here. It reads which pull
        requests a commit belongs to; every answer is empty, and any write is refused.
        """
        self.record(registry="github", action="forge-read" if self.command == "GET" else "forge-write")
        if self.command == "GET" and re.fullmatch(r"/repos/[^/]+/[^/]+/commits/[0-9a-f]+/pulls", path):
            self.send(200, [])
            return
        self.send(403 if self.command != "GET" else 404, {"message": "the double serves no other forge call"})

    def github_upload(self, path):
        match = re.fullmatch(r"/repos/[^/]+/[^/]+/releases/(\d+)/assets", path)
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
        if not match or self.command != "POST" or "name" not in query:
            self.send(404, {"message": "Not Found"})
            return
        name = query["name"][0]
        payload = self.body()
        tag = next((tag for tag in control().get("releases", []) if abs(hash(tag)) % 100000 == int(match.group(1))), None)
        status = refused("github", file=name)
        self.record(registry="github", action="upload", file=name, refused=bool(status))
        if status and tag is not None and partial(name):
            # An upload interrupted part of the way: GitHub keeps the asset, short and not
            # `uploaded`, under the name a re-run has to take back first.
            with lock:
                asset_id = next(asset_ids)
                github_assets[(tag, name)] = {"id": asset_id, "bytes": payload[: len(payload) // 2], "state": "starter"}
                github_ids[asset_id] = (tag, name)
        if status or tag is None:
            self.send(status or 404, {"message": "upload refused"})
            return
        with lock:
            if (tag, name) in github_assets:
                self.send(422, {"message": "Validation Failed", "errors": [{"code": "already_exists"}]})
                return
            asset_id = next(asset_ids)
            github_assets[(tag, name)] = {"id": asset_id, "bytes": payload}
            github_ids[asset_id] = (tag, name)
        self.send(201, {"id": asset_id, "name": name, "size": len(payload), "state": "uploaded"})

    def github_download(self, path):
        tag, _, name = urllib.parse.unquote(path).partition("/")
        rule = control().get("relabel")
        stored = name.replace(rule["to"], rule["from"]) if rule else name
        if rule and stored == name:
            stored = None
        with lock:
            asset = github_assets.get((tag, stored)) if stored and not hidden(stored) else None
        self.record(registry="github", action="download", file=name)
        if asset is None:
            self.send(404, b"Not Found", "text/plain")
            return
        self.send(200, served_bytes(stored, asset["bytes"]), "application/octet-stream")


    def pypi(self, path):
        if path in ("/legacy/", "/legacy") and self.command == "POST":
            message = email.message_from_bytes(b"Content-Type: " + self.headers["Content-Type"].encode() + b"\r\n\r\n" + self.body(), policy=email.policy.HTTP)
            fields, content, filename = {}, None, None
            for part in message.iter_parts():
                match part.get_param("name", header="content-disposition"):
                    case "content":
                        content, filename = part.get_payload(decode=True), part.get_filename()
                    case str() as name if name:
                        fields[name] = part.get_content().strip() if part.get_content_maintype() == "text" else part.get_payload(decode=True).decode()
            if content is None or not filename:
                self.send(400, "no file was uploaded", "text/plain")
                return
            status = refused("pypi", file=filename)
            self.record(registry="pypi", action="upload", file=filename, package=fields.get("name"), version=fields.get("version"), refused=bool(status))
            if status:
                self.send(status, "upload refused", "text/plain")
                return
            digest = hashlib.sha256(content).hexdigest()
            if fields.get("sha256_digest") not in (None, digest):
                self.send(400, "sha256_digest does not match the file", "text/plain")
                return
            with lock:
                if filename in pypi_files:
                    self.send(400, "File already exists", "text/plain")
                    return
                pypi_files[filename] = {"project": normalise(fields.get("name", "")), "bytes": content, "sha256": digest}
            self.send(200, "OK", "text/plain")
            return
        match = re.fullmatch(r"/simple/([^/]+)/?", path)
        if match:
            project = normalise(match.group(1))
            self.record(registry="pypi", action="read-index", package=project)
            with lock:
                files = sorted((name, entry["sha256"]) for name, entry in pypi_files.items() if entry["project"] == project and not hidden(name))
            if "application/vnd.pypi.simple.v1+json" in self.headers.get("Accept", ""):
                listing = {"meta": {"api-version": "1.0"}, "name": project, "files": [{"filename": relabel(name), "url": f"{self.base()}/pypi/files/{relabel(name)}", "hashes": {"sha256": digest}} for name, digest in files]}
                self.send(200, listing, "application/vnd.pypi.simple.v1+json")
                return
            links = "".join(f'<a href="{self.base()}/pypi/files/{html.escape(relabel(name))}#sha256={digest}">{html.escape(relabel(name))}</a>\n' for name, digest in files)
            self.send(200, f"<!DOCTYPE html><html><body>\n{links}</body></html>\n", "text/html")
            return
        match = re.fullmatch(r"/files/([^/]+)", path)
        if match:
            name = urllib.parse.unquote(match.group(1))
            rule = control().get("relabel")
            stored = name.replace(rule["to"], rule["from"]) if rule else name
            with lock:
                entry = pypi_files.get(stored)
            self.record(registry="pypi", action="download", file=name)
            if entry is None or hidden(stored):
                self.send(404, "Not Found", "text/plain")
                return
            self.send(200, served_bytes(stored, entry["bytes"]), "application/octet-stream")
            return
        self.send(404, "Not Found", "text/plain")


    def npm(self, path):
        decoded = urllib.parse.unquote(path.lstrip("/"))
        tarball = re.fullmatch(r"(.+)/-/([^/]+\.tgz)", decoded)
        if tarball and self.command == "GET":
            name, file = tarball.groups()
            with lock:
                found = next((record for record in npm_packages.get(name, {}).values() if record["file"] == file), None)
            self.record(registry="npm", action="download", package=name, file=file)
            if found is None:
                self.send(404, {"error": "Not found"})
                return
            self.send(200, served_bytes(file, found["bytes"]), "application/octet-stream")
            return
        name = decoded
        match self.command:
            case "PUT":
                self.npm_publish(name)
            case "GET":
                self.npm_packument(name)
            case _:
                self.send(405, {"error": "method not allowed"})

    def npm_publish(self, name):
        document = json.loads(self.body() or b"{}")
        versions = document.get("versions") or {}
        attachments = document.get("_attachments") or {}
        status = refused("npm", package=name)
        self.record(registry="npm", action="publish", package=name, versions=sorted(versions), files=sorted(attachments), refused=bool(status))
        if status:
            self.send(status, {"error": "publication refused"})
            return
        with lock:
            held = npm_packages.setdefault(name, {})
            for version, manifest in versions.items():
                if version in held:
                    self.send(403, {"error": f"You cannot publish over the previously published versions: {version}."})
                    return
                attachment = next(iter(attachments.values()))
                payload = base64.b64decode(attachment["data"])
                file = f"{name.split('/')[-1]}-{version}.tgz"
                held[version] = {"manifest": manifest, "file": file, "bytes": payload}
        self.send(201, {"ok": True})

    def npm_packument(self, name):
        self.record(registry="npm", action="read", package=name)
        with lock:
            held = {version: record for version, record in npm_packages.get(name, {}).items() if not hidden(f"{name}@{version}")}
        if not held:
            self.send(404, {"error": "Not found"})
            return
        versions = {}
        for version, record in held.items():
            payload = record["bytes"]
            served = relabel(version)
            manifest = {**record["manifest"], "version": served}
            manifest["dist"] = {
                "tarball": f"{self.base()}/npm/{name}/-/{record['file']}",
                "shasum": hashlib.sha1(payload).hexdigest(),
                "integrity": "sha512-" + base64.b64encode(hashlib.sha512(payload).digest()).decode(),
            }
            versions[served] = manifest
        latest = sorted(versions)[-1]
        self.send(200, {"name": name, "dist-tags": {"latest": latest}, "versions": versions})

    def crates(self, path):
        if path == "/index/config.json":
            self.send(200, {"dl": f"{self.base()}/crates/dl", "api": f"{self.base()}/crates"})
            return
        if path.startswith("/index/"):
            relative = path[len("/index/"):]
            name = relative.rsplit("/", 1)[-1]
            with lock:
                held = crates.get(name)
                lines = [json.dumps(record["entry"]) for _, record in sorted(held.items())] if held else None
            if relative == index_path(name) and (lines is not None or name.startswith("onetaskgraph")):
                # This repository's own crates are the double's alone: what crates.io holds
                # of them would answer a publication as already done.
                self.record(registry="crates", action="read-index", package=name)
                self.send(200, "\n".join(lines) + "\n", "text/plain") if lines else self.send(404, b"", "text/plain")
                return
            self.send(*self.proxy_index(relative))
            return
        match = re.fullmatch(r"/dl/([^/]+)/([^/]+)/download", path)
        if match:
            name, version = match.groups()
            with lock:
                record = crates.get(name, {}).get(version)
            if record is None:
                self.send(302, b"", headers={"Location": f"https://static.crates.io/crates/{name}/{name}-{version}.crate"})
                return
            self.send(200, record["bytes"], "application/x-tar")
            return
        if path == "/api/v1/crates/new" and self.command == "PUT":
            body = self.body()
            size = int.from_bytes(body[:4], "little")
            metadata = json.loads(body[4:4 + size])
            crate_size = int.from_bytes(body[4 + size:8 + size], "little")
            payload = body[8 + size:8 + size + crate_size]
            status = refused("crates", package=metadata["name"])
            self.record(registry="crates", action="publish", package=metadata["name"], version=metadata["vers"], refused=bool(status))
            if status:
                self.send(status, {"errors": [{"detail": "publication refused"}]})
                return
            features, features2 = {}, {}
            for feature, members in metadata.get("features", {}).items():
                (features2 if any(member.startswith("dep:") or "?/" in member for member in members) else features)[feature] = members
            entry = {
                "name": metadata["name"],
                "vers": metadata["vers"],
                "deps": [
                    {
                        "name": dependency.get("explicit_name_in_toml") or dependency["name"],
                        "req": dependency["version_req"],
                        "features": dependency.get("features", []),
                        "optional": dependency.get("optional", False),
                        "default_features": dependency.get("default_features", True),
                        "target": dependency.get("target"),
                        "kind": dependency.get("kind", "normal"),
                        **({"registry": dependency["registry"]} if dependency.get("registry") else {}),
                        **({"package": dependency["name"]} if dependency.get("explicit_name_in_toml") else {}),
                    }
                    for dependency in metadata.get("deps", [])
                ],
                "cksum": hashlib.sha256(payload).hexdigest(),
                "features": features,
                "yanked": False,
                **({"features2": features2, "v": 2} if features2 else {}),
                **({"links": metadata["links"]} if metadata.get("links") else {}),
                **({"rust_version": metadata["rust_version"]} if metadata.get("rust_version") else {}),
            }
            with lock:
                held = crates.setdefault(metadata["name"], {})
                if metadata["vers"] in held:
                    self.send(400, {"errors": [{"detail": f"crate version `{metadata['vers']}` is already uploaded"}]})
                    return
                held[metadata["vers"]] = {"entry": entry, "bytes": payload}
            self.send(200, {"warnings": {"invalid_categories": [], "invalid_badges": [], "other": []}})
            return
        match = re.fullmatch(r"/api/v1/crates/([^/]+)/([^/]+)", path)
        if match and self.command == "GET":
            name, version = match.groups()
            self.record(registry="crates", action="read", package=name, version=version)
            with lock:
                found = version in crates.get(name, {}) and not hidden(f"{name}@{version}")
            self.send(200 if found else 404, {"version": {"crate": name, "num": version}} if found else {"errors": [{"detail": "Not Found"}]})
            return
        self.send(404, {"errors": [{"detail": "Not Found"}]})

    def proxy_index(self, relative):
        with lock:
            if relative in proxied_index:
                return proxied_index[relative]
        request = urllib.request.Request(f"https://index.crates.io/{relative}", headers={"User-Agent": "onetaskgraph-release-journey"})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                answer = (200, response.read(), "text/plain")
        except urllib.error.HTTPError as error:
            answer = (error.code if error.code in (404, 410, 451) else 502, b"", "text/plain")
        except (urllib.error.URLError, TimeoutError, OSError):
            answer = (502, b"", "text/plain")
        with lock:
            proxied_index[relative] = answer
        return answer


class Loopback(ThreadingHTTPServer):
    """Bound without the reverse DNS lookup `HTTPServer.server_bind` does on its own account.

    scripts/check-loopback-registries.sh says why: on the macOS runner that lookup of
    127.0.0.1 outlasted the whole start-up window.
    """

    daemon_threads = True

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


servers = {name: Loopback(("127.0.0.1", 0), Handler) for name in ("main", "checksums")}
ports.update({name: server.server_address[1] for name, server in servers.items()})
threading.Thread(target=servers["checksums"].serve_forever, daemon=True).start()
(state / "ports.json.tmp").write_text(json.dumps(ports), encoding="utf-8")
(state / "ports.json.tmp").replace(state / "ports.json")
servers["main"].serve_forever()
