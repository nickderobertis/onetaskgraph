"""Drive both lock entry points in separate processes against a real shared file."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile
import unittest


PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())


class CliWheelLockTest(unittest.TestCase):
    def test_concurrent_real_cli_wheels(self):
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory(prefix='cli-wheel-builds-') as scratch:
            directory = Path(scratch)
            imported = directory / 'release.py'
            imported.write_text(
                'import sys\n'
                f'sys.path.insert(0, {str(root / "scripts")!r})\n'
                'from cli_wheel_lock import cli_wheel_lock\n'
                'with cli_wheel_lock():\n'
                '    import subprocess\n'
                '    sys.exit(subprocess.call(sys.argv[1:]))\n',
                encoding='utf-8',
            )
            host = next(line.removeprefix('host: ') for line in subprocess.check_output(
                ['rustc', '-vV'], text=True, encoding='utf-8').splitlines()
                if line.startswith('host: '))
            command = ['uv', 'tool', 'run', '--from', 'maturin>=1.9,<2',
                       'maturin', 'build', '--release', '--locked', '--target', host]
            entries = [imported, root / 'scripts/cli_wheel_lock.py']
            children = []
            try:
                for index, entry in enumerate(entries):
                    output = directory / str(index)
                    log = (directory / f'{index}.log').open('w+', encoding='utf-8')
                    child = subprocess.Popen(
                        [PYTHON_EXECUTABLE, str(entry), *command, '--out', str(output)],
                        cwd=root, stdout=log, stderr=subprocess.STDOUT,
                    )
                    children.append((child, log, output))
                for child, log, output in children:
                    child.wait()
                    log.seek(0)
                    self.assertEqual(child.returncode, 0, log.read())
                    wheel, = output.glob('onetaskgraph_cli-*.whl')
                    with zipfile.ZipFile(wheel) as packed:
                        binary, = [name for name in packed.namelist()
                                   if '.data/scripts/' in name
                                   and Path(name).name in ('onetaskgraph', 'onetaskgraph.exe')]
                        self.assertGreater(len(packed.read(binary)), 0)
            finally:
                for child, log, _ in children:
                    if child.poll() is None:
                        child.kill()
                    child.wait()
                    log.close()

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


if __name__ == '__main__':
    unittest.main()
