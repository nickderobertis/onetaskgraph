# llmlint: ignore-file[async_typed_clients_at_boundaries] This subprocess journey deliberately executes the production workflow bodies sequentially and doubles only transport; an async service client would replace the actual CLI boundaries the task requires it to prove.
"""Drive the release workflow's packaging and installation over a real host-native wheel."""
import argparse
import base64
import csv
import hashlib
import platform
from dataclasses import dataclass
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
from release_products import RustTarget, NpmPlatform, RunnerLabel, ArchiveFormat

PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())


@dataclass(frozen=True)
class ReleaseTarget:
    os: RunnerLabel
    target: RustTarget
    extension: ArchiveFormat
    npm_platform: NpmPlatform


def run(wheel_dir: Path, target: RustTarget) -> None:
    root = Path(__file__).resolve().parents[1]
    workflow = (root / '.github/workflows/release.yml').read_text()
    native = re.search(r'(?ms)^  native-assets-and-carriers:\n(.*?)(?=^  [a-z-]+:)', workflow)
    mac = re.search(r'(?ms)^  macos-assets-carriers-and-wheels:\n(.*?)(?=^  [a-z-]+:)', workflow)
    if native is None or mac is None:
        raise ValueError('release build jobs missing; restore their declared packaging paths')
    rows = [ReleaseTarget(*row) for row in re.findall(r'- \{ os: ([^,]+), target: ([^,]+), ext: ([^,]+), npm: ([^ }]+) \}', native.group(1))]
    mac_pairs = re.search(r'^      TARGETS: (.*)$', mac.group(1), re.M)
    if mac_pairs is None:
        raise ValueError('Darwin TARGETS missing; restore the Rust/npm platform mapping')
    rows.extend(ReleaseTarget('macos-latest', pair.split('=')[0], 'tar.gz', pair.split('=')[1]) for pair in mac_pairs[1].split())
    selected, = [row for row in rows if row.target == target]
    job = mac.group(1) if selected.os == 'macos-latest' else native.group(1)
    step = re.search(r'(?ms)^      - name: Archive, checksum, and stage\n.*?^        run: \|\n(.*?)(?=^      - )', job)
    if step is None:
        raise ValueError('release packaging body missing; retain the drivable run block')
    body = '\n'.join(line[10:] for line in step[1].splitlines())
    version = tomllib.loads((root / 'pyproject.toml').read_text())['project']['version']
    binary = 'onetaskgraph.exe' if selected.extension == 'zip' else 'onetaskgraph'
    with tempfile.TemporaryDirectory(prefix='release-products-') as scratch:
        tree = Path(scratch)
        shutil.copytree(root / 'scripts', tree / 'scripts')
        (tree / '.github/workflows').mkdir(parents=True)
        shutil.copy2(root / '.github/workflows/release.yml', tree / '.github/workflows/release.yml')
        shutil.copytree(root / 'npm/platforms', tree / 'npm/platforms')
        wheels = tree / 'dist/wheels' / target
        shutil.copytree(wheel_dir.resolve(), wheels)
        extraction = [PYTHON_EXECUTABLE, 'scripts/release-wheel-binary.py', '--wheels', str(wheels), '--binary', str(tree / 'rejected-binary')]
        wheel_path, = wheels.glob('*.whl')
        original_wheel = wheel_path.read_bytes()
        wheel_path.unlink()
        rejected = subprocess.run(extraction, cwd=tree, capture_output=True, text=True)
        if rejected.returncode == 0 or 'expected one wheel' not in rejected.stderr:
            raise ValueError('zero wheels were accepted by extraction')
        wheel_path.write_bytes(original_wheel)
        duplicate = wheels / 'duplicate.whl'
        duplicate.write_bytes(original_wheel)
        rejected = subprocess.run(extraction, cwd=tree, capture_output=True, text=True)
        if rejected.returncode == 0 or 'expected one wheel' not in rejected.stderr:
            raise ValueError('multiple wheels were accepted by extraction')
        duplicate.unlink()
        for entries in [[], [f'cli.data/scripts/{binary}', f'other.data/scripts/{binary}']]:
            with zipfile.ZipFile(wheel_path, 'w') as malformed:
                for entry in entries:
                    malformed.writestr(entry, b'invalid')
            rejected = subprocess.run(extraction, cwd=tree, capture_output=True, text=True)
            if rejected.returncode == 0 or 'exactly one CLI' not in rejected.stderr or (tree / 'rejected-binary').exists():
                raise ValueError('malformed wheel scripts were accepted by extraction')
        wheel_path.write_bytes(original_wheel)
        canonical = tree / f'target/{target}/release/{binary}'
        subprocess.run([PYTHON_EXECUTABLE, 'scripts/release-wheel-binary.py', '--wheels', str(wheels), '--binary', str(canonical)], cwd=tree, check=True, stdout=subprocess.DEVNULL)
        env = {**os.environ, 'TARGETS': f'{target}={selected.npm_platform}', 'EXT': selected.extension, 'TAG': f'v{version}'}
        subprocess.run(['bash', '-c', body], cwd=tree, env=env, check=True)
        command = [PYTHON_EXECUTABLE, 'scripts/verify-release-products.py', '--target', target, '--npm', selected.npm_platform, '--tag', f'v{version}', '--ext', selected.extension]
        subprocess.run(command + ['--install'], cwd=tree, check=True, stdout=subprocess.DEVNULL)

        def refuses(reason: str) -> None:
            result = subprocess.run(command + ['--install'], cwd=tree, capture_output=True, text=True)
            if result.returncode == 0 or reason not in result.stderr:
                raise ValueError(f'tampered staged product did not fail for {reason}; repair the production verifier before publishing')

        original_binary = canonical.read_bytes()
        canonical.write_bytes(original_binary + b'corrupt')
        refuses('release binary drift')
        canonical.write_bytes(original_binary)
        wheel_path, = wheels.glob('*.whl')
        original_wheel = wheel_path.read_bytes()

        def alter_wheel(suffix: str, replacement: bytes) -> None:
            with zipfile.ZipFile(wheel_path) as wheel:
                members = [(info, wheel.read(info.filename)) for info in wheel.infolist()]
            with zipfile.ZipFile(wheel_path, 'w') as wheel:
                for info, contents in members:
                    wheel.writestr(info, replacement if info.filename.endswith(suffix) else contents)

        # Each packaged input is independently corrupted while the canonical binary
        # stays intact, so deleting any one comparison makes a journey fail.
        archive = tree / f'onetaskgraph-v{version}-{target}.{selected.extension}'
        original_archive = archive.read_bytes()
        if selected.extension == 'zip':
            with zipfile.ZipFile(archive, 'w') as packed:
                packed.writestr(binary, original_binary + b'corrupt')
        else:
            with tarfile.open(archive, 'w:gz') as packed:
                info = tarfile.TarInfo(binary)
                info.size = len(original_binary) + len(b'corrupt')
                packed.addfile(info, io.BytesIO(original_binary + b'corrupt'))
        refuses('release binary drift')
        archive.write_bytes(original_archive)
        with zipfile.ZipFile(wheel_path) as wheel:
            entry, = [name for name in wheel.namelist() if '.data/scripts/' in name and Path(name).name == binary]
        alter_wheel(entry, original_binary + b'corrupt')
        refuses('release binary drift')
        wheel_path.write_bytes(original_wheel)
        carrier, = (tree / f'dist/carriers/{selected.npm_platform}').glob('*.tgz')
        original_carrier = carrier.read_bytes()
        with tarfile.open(carrier) as packed:
            members = [(info, packed.extractfile(info).read() if info.isfile() else None) for info in packed.getmembers()]
        with tarfile.open(carrier, 'w:gz') as packed:
            for info, contents in members:
                if info.name == f'package/bin/{binary}':
                    contents += b'corrupt'
                    info.size = len(contents)
                packed.addfile(info, io.BytesIO(contents) if contents is not None else None)
        refuses('release binary drift')
        carrier.write_bytes(original_carrier)
        alter_wheel('/METADATA', b'Name: onetaskgraph-cli\nVersion: 999.0.0\n')
        refuses('wheel identity or version mismatch')
        wheel_path.write_bytes(original_wheel)
        alter_wheel('/WHEEL', b'Tag: py3-none-foreign_unknown\n')
        refuses('wheel architecture or OS mismatch')
        wheel_path.write_bytes(original_wheel)
        renamed = wheel_path.with_name(wheel_path.name.replace(version, '999.0.0', 1))
        wheel_path.rename(renamed)
        refuses('wheel filename version mismatch')
        renamed.rename(wheel_path)
        carrier, = (tree / f'dist/carriers/{selected.npm_platform}').glob('*.tgz')
        original_carrier = carrier.read_bytes()
        with tarfile.open(carrier) as packed:
            members = [(info, packed.extractfile(info).read() if info.isfile() else None) for info in packed.getmembers()]
        with tarfile.open(carrier, 'w:gz') as packed:
            for info, contents in members:
                if info.name == 'package/package.json':
                    manifest = json.loads(contents)
                    manifest['name'] = '@forged/cli'
                    contents = json.dumps(manifest).encode()
                    info.size = len(contents)
                packed.addfile(info, io.BytesIO(contents) if contents is not None else None)
        refuses('carrier identity mismatch')
        carrier.write_bytes(original_carrier)
        for field, invalid, reason in [('version', '999.0.0', 'carrier identity mismatch'), ('os', ['foreign'], 'carrier platform mismatch'), ('cpu', ['foreign'], 'carrier platform mismatch')]:
            with tarfile.open(carrier) as packed:
                members = [(info, packed.extractfile(info).read() if info.isfile() else None) for info in packed.getmembers()]
            with tarfile.open(carrier, 'w:gz') as packed:
                for info, contents in members:
                    if info.name == 'package/package.json':
                        manifest = json.loads(contents)
                        manifest[field] = invalid
                        contents = json.dumps(manifest).encode()
                        info.size = len(contents)
                    packed.addfile(info, io.BytesIO(contents) if contents is not None else None)
            refuses(reason)
            carrier.write_bytes(original_carrier)
        # Keep all forms hash-identical while corrupting the real compiled executable's
        # response. This proves that equality alone cannot make an unusable CLI pass.
        for old, new, reason in [(version.encode(), b'9' * len(version), 'installed CLI version mismatch'), (b'Usage:', b'Ubage:', 'installed CLI did not render help')]:
            if old not in original_binary:
                raise ValueError('native CLI response marker was absent; update this corruption journey for the compiled product')
            payload = original_binary.replace(old, new)
            canonical.write_bytes(payload)
            if platform.system() == 'Darwin':
                # Changing Mach-O data invalidates its ad-hoc signature; signing this
                # deliberately corrupted test artifact lets the real process answer.
                subprocess.run(['codesign', '--force', '--sign', '-', str(canonical)], check=True, stdout=subprocess.DEVNULL)
                payload = canonical.read_bytes()
            with zipfile.ZipFile(wheel_path) as wheel:
                contents = {info.filename: (info, wheel.read(info.filename)) for info in wheel.infolist()}
            for name, (info, data) in list(contents.items()):
                if '.data/scripts/' in name and Path(name).name == binary:
                    contents[name] = (info, payload)
            record, = [name for name in contents if name.endswith('.dist-info/RECORD')]
            rows = [[name, 'sha256=' + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b'=').decode(), str(len(data))] for name, (_, data) in contents.items() if name != record]
            rows.append([record, '', ''])
            text = io.StringIO()
            csv.writer(text, lineterminator='\n').writerows(rows)
            contents[record] = (contents[record][0], text.getvalue().encode())
            with zipfile.ZipFile(wheel_path, 'w') as wheel:
                for info, data in contents.values():
                    wheel.writestr(info, data)
            subprocess.run(['bash', '-c', body], cwd=tree, env=env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            refuses(reason)
            canonical.write_bytes(original_binary)
            wheel_path.write_bytes(original_wheel)
            subprocess.run(['bash', '-c', body], cwd=tree, env=env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(command, cwd=tree, check=True, stdout=subprocess.DEVNULL)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, epilog='Exit codes: 0 journey passed; 1 packaging/install/proof failure; 2 invalid invocation. Repair the named workflow body or staged product before retrying.')
    parser.add_argument('--wheel-dir', type=Path, required=True)
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    try:
        run(args.wheel_dir, RustTarget(args.target))
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError, zipfile.BadZipFile, tarfile.TarError) as error:
        print(f'release product journey failed: {error}; next: repair the release workflow packaging or its named product verifier, then rerun scripts/test-distribution.sh', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
