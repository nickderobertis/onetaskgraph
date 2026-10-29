"""Keep maturin's shared native output stable until its consumer has finished.

The release builder also packages and verifies that output while holding the lock. The
installer journey takes the same lock for its wheel build, then works in its own tree.

The command wrapper exits 2 for a missing command, 1 for an OS error, and otherwise
forwards the build command's exit status.
"""
from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import errno
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


@contextmanager
def cli_wheel_lock() -> Iterator[None]:
    path = ROOT / 'target' / '.cli-wheel-build.lock'
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('a+b') as handle:
        if os.name == 'nt':
            import msvcrt
            # Windows byte-range locks require a byte to lock. Appending one concurrently
            # is harmless: every holder locks byte zero, never the appended byte.
            handle.write(b'\0')
            handle.flush()
            while True:
                handle.seek(0)
                try:
                    msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
                    break
                except OSError as error:
                    if error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                        raise
                    time.sleep(0.05)
            try:
                yield
            finally:
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            fcntl.flock(handle, fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(handle, fcntl.LOCK_UN)


def main() -> int:
    if len(sys.argv) < 2:
        print('cli-wheel-lock: a build command is required; next: run the release recipe or distribution journey', file=sys.stderr)
        return 2
    try:
        with cli_wheel_lock():
            return subprocess.call(sys.argv[1:], cwd=ROOT)
    except OSError as error:
        print(f'cli-wheel-lock: {error}; next: check the build command and target directory permissions, then rerun', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
