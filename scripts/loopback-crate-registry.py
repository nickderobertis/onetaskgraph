#!/usr/bin/env python3
"""The sparse crate index scripts/check-crate-sibling-resolution.sh serves on loopback.

Called as `loopback-crate-registry.py <port file> <index root>`: it binds a port the kernel
picks, writes that port to the file, and serves the index root until it is killed.

It is a file rather than a heredoc inside that check so that
scripts/check-loopback-registries.sh can run this very launcher against the bind `Loopback`
below states — a requirement nothing about the code that meets it makes visible.
"""

# llmlint: ignore-file[boundary_inputs_validated] The two arguments below are paths
# scripts/check-crate-sibling-resolution.sh creates and passes, and a wrong one raises into
# the log that check prints. Nothing else reaches this server: it serves a static index
# directory to one cargo over loopback.

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


class Unconditional(http.server.SimpleHTTPRequestHandler):
    """Every request answered with the file as it is now, never `304 Not Modified`.

    The stock handler sends `Last-Modified` and answers a matching `If-Modified-Since` with
    304 whenever the file's mtime, truncated to whole seconds, is not later than it. The
    check edits an index file cargo has already cached and resolves again, often within the
    same second as that cache, so cargo revalidated to 304, kept the release the edit
    removed, and the check's "sibling absent from the registry" case passed or failed by the
    clock. With no validator sent and none honoured, what cargo resolves against is always
    the index as the check last wrote it. scripts/check-crate-sibling-resolution-same-second.sh
    drives that check with every mtime pinned to one second, which is what holds this.
    """

    def send_head(self):
        del self.headers["If-Modified-Since"]
        return super().send_head()

    def send_header(self, keyword, value):
        if keyword.lower() != "last-modified":
            super().send_header(keyword, value)


handler = functools.partial(Unconditional, directory=root)
server = Loopback(("127.0.0.1", 0), handler)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
