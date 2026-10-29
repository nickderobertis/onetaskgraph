#!/usr/bin/env python3
"""Drive the release registry's HTTP proxy through an upstream failure and recovery."""

import json
import subprocess
import sys
import tempfile
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
        self.send_response(503 if count == 1 else 200)
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
def request(url, *args, **kwargs):
    return original(url.replace('https://index.crates.io/', upstream + '/'), *args, **kwargs)
urllib.request.Request = request
launcher = sys.argv.pop(1)
runpy.run_path(launcher, run_name='__main__')
"""


def read(url):
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        with error:
            return error.code, error.read()


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
                                raise AssertionError(process.communicate())
                            if time.monotonic() >= deadline:
                                raise AssertionError('release registry did not report its ports')
                            time.sleep(0.05)
                        port = json.loads(port_file.read_text())['main']
                        path = '/pr/ed/predicates'
                        url = f'http://127.0.0.1:{port}/crates/index{path}'
                        assert read(url) == (502, b''), 'first request must expose upstream failure'
                        assert Upstream.counts == {path: 1}
                        assert read(url) == (200, b'recovered index entry\n'), 'second request must retry upstream'
                        assert Upstream.counts == {path: 2}
                        assert read(url) == (200, b'recovered index entry\n'), 'third request must reuse successful answer'
                        assert Upstream.counts == {path: 2}, 'cache hit must not reach upstream'
                    finally:
                        process.terminate()
                        process.communicate(timeout=5)
        finally:
            upstream.shutdown()
            thread.join()
    print('check-release-index-cache: upstream failure retries and recovery is cached')


if __name__ == '__main__':
    main()
