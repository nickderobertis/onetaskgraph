# llmlint: ignore-file[async_typed_clients_at_boundaries] Staged installers and CLI subprocesses must run sequentially before publication; these are local executable boundaries, not hosted services with a reusable typed SDK client.
"""Verify identical staged payloads and install/run the host's release products."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from typing import TypedDict
from release_products import ReleaseProduct, RustTarget, NpmPlatform, ReleaseTag

PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())


class VerificationReport(TypedDict):
    target: RustTarget
    tag: ReleaseTag
    payloadHashes: dict[str, str]


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise ValueError(reason)


def verify(product: ReleaseProduct, install: bool) -> None:
    binary = product.binary_name
    canonical = Path(f'target/{product.target}/release/{binary}').read_bytes()
    archive = Path(f'onetaskgraph-{product.tag}-{product.target}.{product.extension}')
    if product.extension == 'zip':
        with zipfile.ZipFile(archive) as packed:
            matches = [p for p in packed.namelist() if Path(p).name == binary]
            require(len(matches) == 1, 'archive must carry one CLI binary')
            archived = packed.read(matches[0])
    else:
        with tarfile.open(archive) as packed:
            archived = packed.extractfile(binary).read()
    wheel_path, = Path(f'dist/wheels/{product.target}').glob('*.whl')
    with zipfile.ZipFile(wheel_path) as packed:
        entry, = [p for p in packed.namelist() if '.data/scripts/' in p and Path(p).name == binary]
        wheeled = packed.read(entry)
        metadata, = [p for p in packed.namelist() if p.endswith('.dist-info/METADATA')]
        fields = packed.read(metadata).decode().splitlines()
        require(f'Version: {product.version}' in fields and 'Name: onetaskgraph-cli' in fields, 'wheel identity or version mismatch')
        wheel_metadata, = [p for p in packed.namelist() if p.endswith('.dist-info/WHEEL')]
        platforms = [line.removeprefix('Tag: ').split('-')[-1] for line in packed.read(wheel_metadata).decode().splitlines() if line.startswith('Tag: ')]
        match product.os:
            case 'Darwin':
                os_prefix = 'macosx_'
            case 'Windows':
                os_prefix = 'win_'
            case 'Linux':
                os_prefix = ('manylinux_', 'linux_', 'musllinux_')
        require(bool(platforms) and all(all(part.startswith(os_prefix) and part.endswith('_' + product.wheel_arch) for part in value.split('.')) for value in platforms), 'wheel architecture or OS mismatch')
        require(wheel_path.name.startswith(f'onetaskgraph_cli-{product.version}-'), 'wheel filename version mismatch')
        filename = wheel_path.stem.rsplit('-', 3)
        require(len(filename) == 4, 'invalid wheel filename tags')
        declared_tags = {line.removeprefix('Tag: ') for line in packed.read(wheel_metadata).decode().splitlines() if line.startswith('Tag: ')}
        filename_tags = {f'{python}-{abi}-{platform}' for python in filename[1].split('.') for abi in filename[2].split('.') for platform in filename[3].split('.')}
        require(filename_tags == declared_tags, 'wheel filename tags mismatch')

    carrier, = Path(f'dist/carriers/{product.npm_platform}').glob('*.tgz')
    with tarfile.open(carrier) as packed:
        carried = packed.extractfile(f'package/bin/{binary}').read()
        manifest = json.load(packed.extractfile('package/package.json'))
        require(manifest.get('os') == [{'Darwin':'darwin','Windows':'win32','Linux':'linux'}[product.os]] and manifest.get('cpu') == ['arm64' if product.target.startswith('aarch64') else 'x64'], 'carrier platform mismatch')
        require(manifest['name'] == f'@onetaskgraph/cli-{product.npm_platform}' and manifest['version'] == product.version, 'carrier identity mismatch')
    hashes = {name: hashlib.sha256(payload).hexdigest() for name, payload in [('binary', canonical), ('archive', archived), ('wheel', wheeled), ('npm', carried)]}
    require(len(set(hashes.values())) == 1, f'release binary drift: {hashes}')
    result: VerificationReport = {'target': product.target, 'tag': product.tag, 'payloadHashes': hashes}
    if install:
        require(platform.system() == product.os, 'cannot execute a non-native release target')
        with tempfile.TemporaryDirectory(prefix='release-install-') as scratch:
            dest = Path(scratch)
            # Archive and checksum fixtures retain the installer's separate trust origins.
            archives = dest / 'releases' / product.tag
            checksums = dest / 'canonical' / product.tag
            archives.mkdir(parents=True)
            checksums.mkdir(parents=True)
            shutil.copy2(archive, archives / archive.name)
            shutil.copy2(str(archive) + '.sha256', checksums / (archive.name + '.sha256'))
            # llmlint: ignore[changed_behavior_has_e2e] Darwin/Rosetta execution needs an Apple Silicon host unavailable in this dispatch; local native installation and foreign-tag/hash contracts are driven, and this exact command runs in the single release Mac job before Linux publication.
            prefix = ['arch', '-x86_64'] if product.os == 'Darwin' and product.target.startswith('x86_64') and platform.machine() == 'arm64' else []
            env = {**os.environ, 'ONETASKGRAPH_CHECKSUM_BASE_URL': checksums.parent.as_uri()}
            subprocess.run(prefix + ['bash', 'scripts/install.sh', '--version', product.tag, '--to', str(dest / 'archive'), '--archive-base-url', archives.parent.as_uri()], env=env, check=True, stdout=sys.stderr)
            subprocess.run([PYTHON_EXECUTABLE, '-m', 'venv', str(dest / 'installer-python')], check=True, stdout=sys.stderr)
            pip_python = dest / 'installer-python' / ('Scripts/python.exe' if product.os == 'Windows' else 'bin/python')
            subprocess.run([str(pip_python), '-m', 'pip', 'install', '--no-deps', '--target', str(dest / 'wheel'), '--platform', filename[3].split('.')[0], str(wheel_path)], check=True, stdout=sys.stderr)
            subprocess.run(['npm', 'install', '--force', '--ignore-scripts', '--no-audit', '--no-fund', '--prefix', str(dest / 'npm'), str(carrier.resolve())], check=True, stdout=sys.stderr)
            wheel_bins = list((dest / 'wheel').rglob(binary))
            require(len(wheel_bins) == 1, 'wheel install must expose one CLI executable')
            products = [dest / 'archive' / binary, wheel_bins[0], dest / 'npm' / 'node_modules' / '@onetaskgraph' / f'cli-{product.npm_platform}' / 'bin' / binary]
            for executable in products:
                output = subprocess.check_output(prefix + [str(executable), '--version'], text=True)
                require(output.strip() == f'onetaskgraph {product.version}', f'installed CLI version mismatch: {executable}')
                require('Usage:' in subprocess.check_output(prefix + [str(executable), '--help'], text=True), 'installed CLI did not render help')
    print(json.dumps(result))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, epilog='Exit codes: 0 verified; 1 artifact/install failure; 2 invalid invocation. Repair the named staged product and rerun before publishing.')
    parser.add_argument('--target', required=True)
    parser.add_argument('--npm', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--ext', choices=['tar.gz', 'zip'], default='tar.gz')
    parser.add_argument('--install', action='store_true')
    args = parser.parse_args()
    try:
        product = ReleaseProduct(RustTarget(args.target), NpmPlatform(args.npm), ReleaseTag(args.tag), args.ext)
        verify(product, args.install)
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError, zipfile.BadZipFile, tarfile.TarError, TypeError, AttributeError) as error:
        print(f'release product verification failed: {error}; next: rebuild the named wheel and its archive/carrier from the canonical wheel binary, then rerun this command before publishing', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
