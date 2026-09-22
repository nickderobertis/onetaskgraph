#!/usr/bin/env python3
"""The sparse crate index scripts/check-crate-sibling-resolution.sh serves on loopback.

Called as `loopback-crate-registry.py <port file> <index root>`: it binds a port the kernel
picks, writes that port to the file, and serves the index root until it is killed.

It is a server of this repository's own rather than `python3 -m http.server`. That one
binds through `http.server.HTTPServer.server_bind`, which names itself with
`socket.getfqdn(host)` BEFORE printing the banner the check used to read the port out of —
and on the macOS runner that reverse lookup of 127.0.0.1 outlasted the whole wait, so a
registry that had in fact bound was reported as one that never reported a port, with an
empty log where the reason belonged. So this binds without the lookup, as
scripts/loopback-npm-registry.py does for the same reason, and writes its port to a file of
its own the moment it has one: the same report on every platform, read from nothing a
platform's resolver can delay.

It is a file rather than a heredoc inside that check because that requirement is invisible
in the code that meets it — a cleanup back to the stock bind passes every Linux check and
hangs the macOS release lane. scripts/check-loopback-registries.sh runs this very launcher
with `socket.getfqdn` replaced by a function that blocks, which is what holds it.
"""

import functools
import http.server
import socketserver
import sys

port_file, root = sys.argv[1:]


class Loopback(http.server.ThreadingHTTPServer):
    """Bound without the reverse DNS lookup `HTTPServer.server_bind` does on its own account."""

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=root)
server = Loopback(("127.0.0.1", 0), handler)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
