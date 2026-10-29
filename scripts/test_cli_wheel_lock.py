"""Drive both lock entry points in separate processes against a real shared file."""
from pathlib import Path
import shutil
import os
import errno
import time
import subprocess
import sys
import tempfile
import zipfile
import unittest


PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())


class CliWheelLockTest(unittest.TestCase):
    def test_concurrent_real_cli_wheels(self):
        from cli_wheel_lock import cli_wheel_lock
        from release_cd import maturin_requirement
        from release_products import PLATFORM_TARGETS
        from product_versions import read_reconciled_versions

        root = Path(__file__).resolve().parents[1]
        host = next(line.removeprefix('host: ') for line in subprocess.check_output(
            ['rustc', '-vV'], text=True, encoding='utf-8').splitlines()
            if line.startswith('host: '))
        row, = [row for row in PLATFORM_TARGETS if row.target == host]
        tag = 'v' + str(next(iter(read_reconciled_versions().values())))
        with tempfile.TemporaryDirectory(prefix='cli-wheel-builds-') as scratch:
            directory = Path(scratch)
            release = [PYTHON_EXECUTABLE, str(Path(__file__).resolve()),
                       '--observe-release', '--platform', row.platform]
            dist = directory / 'release'
            command = ['uv', 'tool', 'run', '--from', maturin_requirement(),
                       'maturin', 'build', '--release', '--locked', '--target', host]
            commands = [release + ['build', '--tag', tag, '--dist', str(dist)],
                        [PYTHON_EXECUTABLE, str(root / 'scripts/cli_wheel_lock.py'),
                         *command, '--out', str(directory / 'distribution')]]
            children = []
            try:
                for index, command in enumerate(commands):
                    log = (directory / f'{index}.log').open('w+', encoding='utf-8')
                    child = subprocess.Popen(command, cwd=root, stdout=log,
                                             stderr=subprocess.STDOUT)
                    children.append((child, log))
                for child, log in children:
                    child.wait()
                    log.seek(0)
                    self.assertEqual(child.returncode, 0, log.read())
                for output in (dist, directory / 'distribution'):
                    wheel, = output.rglob('onetaskgraph_cli-*.whl')
                    with zipfile.ZipFile(wheel) as packed:
                        binary, = [name for name in packed.namelist()
                                   if '.data/scripts/' in name
                                   and Path(name).name in ('onetaskgraph', 'onetaskgraph.exe')]
                        self.assertGreater(len(packed.read(binary)), 0)
            finally:
                for child, log in children:
                    if child.poll() is None:
                        child.kill()
                    child.wait()
                    log.close()
            # The real package entry point must wait on the same lock, even when its
            # missing wheel will make it fail. That failure must let the next one in.
            with cli_wheel_lock():
                failed = subprocess.Popen(
                    release + ['package', '--tag', tag, '--dist', str(directory / 'missing')],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8')
                try:
                    time.sleep(1)
                    self.assertIsNone(failed.poll(), 'package bypassed the shared lock')
                except BaseException:
                    failed.kill()
                    failed.communicate()
                    raise
            stdout, stderr = failed.communicate(timeout=15)
            self.assertEqual(failed.returncode, 1, stdout + stderr)
            self.assertIn('no wheel', stderr)
            recovered = subprocess.run(
                release + ['package', '--tag', tag, '--dist', str(dist)],
                capture_output=True, text=True, encoding='utf-8', timeout=60)
            self.assertEqual(recovered.returncode, 0, recovered.stdout + recovered.stderr)

    def test_concurrent_consumers_and_failure_release(self):
        with tempfile.TemporaryDirectory(prefix='cli-wheel-lock-') as scratch:
            root = Path(scratch)
            scripts = root / 'scripts'
            scripts.mkdir()
            shutil.copy2(Path(__file__).with_name('cli_wheel_lock.py'), scripts)
            build = scripts / 'build.py'
            build.write_text('''import pathlib, sys, time
root = pathlib.Path(__file__).resolve().parents[1]
active = root / 'active'
active.mkdir()
try:
    print('build entered', flush=True)
    time.sleep(0.2)
    print('build finished', flush=True)
    sys.exit(int(sys.argv[1]))
finally:
    active.rmdir()
''', encoding='utf-8')
            imported = scripts / 'release.py'
            imported.write_text('''from cli_wheel_lock import cli_wheel_lock
import pathlib, subprocess, sys
PYTHON_EXECUTABLE = str(pathlib.Path(sys.executable).resolve())
with cli_wheel_lock():
    sys.exit(subprocess.call([PYTHON_EXECUTABLE, str(pathlib.Path(__file__).with_name('build.py')), sys.argv[1]]))
''', encoding='utf-8')
            command = [PYTHON_EXECUTABLE, str(scripts / 'cli_wheel_lock.py'), PYTHON_EXECUTABLE, str(build)]
            first = subprocess.Popen([PYTHON_EXECUTABLE, str(imported), '0'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            second = subprocess.Popen(command + ['0'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                results = [(child, *child.communicate(timeout=15)) for child in (first, second)]
                for child, stdout, stderr in results:
                    self.assertEqual(child.returncode, 0, stderr)
                    self.assertEqual(stdout.splitlines(), ['build entered', 'build finished'])
            finally:
                for child in (first, second):
                    if child.poll() is None:
                        child.kill()
                    child.communicate()
            refused = subprocess.run(command + ['37'], capture_output=True, text=True, timeout=15)
            self.assertEqual(refused.returncode, 37, refused.stderr)
            recovered = subprocess.run([PYTHON_EXECUTABLE, str(imported), '0'], capture_output=True, text=True, timeout=15)
            self.assertEqual(recovered.returncode, 0, recovered.stderr)
            self.assertIn('build finished', recovered.stdout)


def observe_release():
    """Observe the real CLI's subprocesses without replacing a tool or release function."""
    from release_cd import ROOT, main

    phases = set()

    def audit(event, arguments):
        if event != 'subprocess.Popen':
            return
        raw = arguments[1]
        command = ' '.join(raw) if isinstance(raw, (list, tuple)) else raw
        phase = ('maturin' if 'maturin' in command else
                 'verify' if 'verify-release-products.py' in command else
                 'package' if 'pack' in command else None)
        if phase is None:
            return
        with (ROOT / 'target/.cli-wheel-build.lock').open('r+b') as handle:
            try:
                if os.name == 'nt':
                    import msvcrt
                    msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
                    msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    fcntl.flock(handle, fcntl.LOCK_UN)
            except OSError as error:
                if error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                    raise
            else:
                raise AssertionError(f'release {phase} ran without the shared lock')
        phases.add(phase)

    sys.addaudithook(audit)
    result = main(sys.argv[2:])
    if result == 0:
        required = {'maturin', 'package', 'verify'} if 'build' in sys.argv else {'package'}
        if not required <= phases:
            raise AssertionError(f'release phases were not observed: {required - phases}')
    return result


if __name__ == '__main__':
    if sys.argv[1:2] == ['--observe-release']:
        sys.exit(observe_release())
    unittest.main()
