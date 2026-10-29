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


class Upstream(BaseHTTPRequestHandler):
    counts = {}

    def do_GET(self):
        count = self.counts.get(self.path, 0) + 1
        self.counts[self.path] = count
        if count == 1 and self.path.endswith('/transport'):
            self.connection.close()
            return
        status = int(self.path.rsplit('/', 1)[-1]) if count == 1 else 200
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
                        for failure in (503, 404, 410, 451, 'transport'):
                            path = f'/pr/ed/{failure}'
                            url = f'http://127.0.0.1:{port}/crates/index{path}'
                            status = failure if failure in (404, 410, 451) else 502
                            expect(read(url), Answer(status, b''), 'first request exposes upstream failure')
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
    main()
