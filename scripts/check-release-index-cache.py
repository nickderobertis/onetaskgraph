#!/usr/bin/env python3
"""Drive the release registry's HTTP proxy through an upstream failure and recovery."""

import json
import subprocess
import sys
import tempfile
from typing import NamedTuple
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class Failure(NamedTuple):
    route: str
    upstream_status: int | None
    expected_status: int


FAILURES = (
    Failure('/pr/ed/503', 503, 502),
    Failure('/pr/ed/404', 404, 404),
    Failure('/pr/ed/410', 410, 410),
    Failure('/pr/ed/451', 451, 451),
    Failure('/pr/ed/transport', None, 502),
)


class Upstream(BaseHTTPRequestHandler):
    counts = {}

    def do_GET(self):
        case = next((case for case in FAILURES if case.route == self.path), None)
        if case is None:
            self.send_error(404)
            return
        count = self.counts.get(self.path, 0) + 1
        self.counts[self.path] = count
        if count == 1 and case.upstream_status is None:
            self.connection.close()
            return
        status = case.upstream_status if count == 1 else 200
        self.send_response(status)
        self.end_headers()
        if count > 1:
            self.wfile.write(b'recovered index entry\n')

    def log_message(self, *_args):
        pass


# Redirect only the upstream origin in the child interpreter. Both the upstream double
# and the registry under test still read and answer real HTTP requests.
BOOTSTRAP = """
import runpy
import sys
import urllib.request
original = urllib.request.Request
upstream = sys.argv.pop(1)
from urllib.parse import urlsplit
from pathlib import Path
origin = urlsplit(upstream)
if origin.scheme != 'http' or origin.hostname != '127.0.0.1' or not origin.port:
    raise ValueError('expected loopback upstream origin')
def request(url, *args, **kwargs):
    return original(url.replace('https://index.crates.io/', upstream + '/'), *args, **kwargs)
urllib.request.Request = request
launcher = sys.argv.pop(1)
if not Path(launcher).is_file():
    raise ValueError('expected existing registry launcher')
runpy.run_path(launcher, run_name='__main__')
"""


class Answer(NamedTuple):
    status: int
    body: bytes


# llmlint: ignore-block[async_typed_clients_at_boundaries] This synchronous HTTP test uses
# only the standard library, like the registry fixture it drives; no application client
# exists for this release-only subprocess boundary.
def read(url) -> Answer:
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            return Answer(response.status, response.read())
    except urllib.error.HTTPError as error:
        with error:
            return Answer(error.code, error.read())

# llmlint: ignore-end[async_typed_clients_at_boundaries]


def expect(actual, expected, detail):
    if actual != expected:
        raise AssertionError(f'{detail}: expected {expected!r}, got {actual!r}; next: repair the release registry proxy cache and rerun scripts/check-loopback-registries.sh')


def main():
    launcher = Path(__file__).with_name('loopback-release-registries.py')
    with ThreadingHTTPServer(('127.0.0.1', 0), Upstream) as upstream:
        thread = threading.Thread(target=upstream.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                with subprocess.Popen([
                    sys.executable, '-c', BOOTSTRAP,
                    f'http://127.0.0.1:{upstream.server_port}', str(launcher), directory,
                ], stdout=subprocess.PIPE, stderr=subprocess.PIPE) as process:
                    try:
                        port_file = Path(directory) / 'ports.json'
                        deadline = time.monotonic() + 30
                        while not port_file.exists():
                            if process.poll() is not None:
                                raise AssertionError(f'release registry exited: {process.communicate()!r}; next: repair the launcher startup error and rerun scripts/check-loopback-registries.sh')
                            if time.monotonic() >= deadline:
                                process.terminate()
                                raise AssertionError(f'release registry did not report its ports: {process.communicate()!r}; next: inspect the launcher startup and rerun scripts/check-loopback-registries.sh')
                            time.sleep(0.05)
                        port = json.loads(port_file.read_text())['main']
                        if type(port) is not int or not 1 <= port <= 65535:
                            raise AssertionError(f'invalid registry port {port!r}; next: repair the launcher port announcement and rerun scripts/check-loopback-registries.sh')
                        for case in FAILURES:
                            path = case.route
                            url = f'http://127.0.0.1:{port}/crates/index{path}'
                            expect(read(url), Answer(case.expected_status, b''), 'first request exposes upstream failure')
                            expect(Upstream.counts[path], 1, 'first request reaches upstream')
                            expect(read(url), Answer(200, b'recovered index entry\n'), 'second request retries upstream')
                            expect(Upstream.counts[path], 2, 'recovery reaches upstream again')
                            expect(read(url), Answer(200, b'recovered index entry\n'), 'third request reuses success')
                            expect(Upstream.counts[path], 2, 'cache hit avoids upstream')
                    finally:
                        process.terminate()
                        process.communicate(timeout=5)
        finally:
            upstream.shutdown()
            thread.join()
    print('check-release-index-cache: upstream failure retries and recovery is cached')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(f'check-release-index-cache: {type(error).__name__}: {error}', file=sys.stderr)
        print('check-release-index-cache: next: inspect loopback socket availability, temporary-directory permissions and the registry launcher diagnostics above; repair the named failure and rerun scripts/check-loopback-registries.sh', file=sys.stderr)
        sys.exit(1)
