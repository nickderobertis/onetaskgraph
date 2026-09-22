#!/usr/bin/env python3
"""The sparse crate index scripts/check-crate-sibling-resolution.sh serves on loopback.

Called as `loopback-crate-registry.py <port file> <index root>`: it binds a port the kernel
picks, writes that port to the file, and serves the index root until it is killed.

It is a file rather than a heredoc inside that check so that
scripts/check-loopback-registries.sh can run this very launcher against the bind `Loopback`
below states — a requirement nothing about the code that meets it makes visible.
"""

import functools
import http.server
import socketserver
import sys

port_file, root = sys.argv[1:]


class Loopback(http.server.ThreadingHTTPServer):
    """Bound without the reverse DNS lookup `HTTPServer.server_bind` does on its own account.

    That lookup of 127.0.0.1 outlasted the whole start-up window on the macOS release
    runner, so a registry that had in fact bound was reported as one that never reported a
    port — which is why the port below goes to a file rather than to a banner.
    """

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=root)
server = Loopback(("127.0.0.1", 0), handler)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
