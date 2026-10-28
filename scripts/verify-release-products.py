# llmlint: ignore-file[async_typed_clients_at_boundaries] Staged installers and CLI subprocesses must run sequentially before publication; these are local executable boundaries, not hosted services with a reusable typed SDK client.
"""Verify identical staged payloads and install/run the host's release products."""
import argparse
from email.parser import Parser
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
from typing import Iterable, TypeVar
from release_products import bash_executable, ReleaseProduct, RustTarget, NpmPlatform, ReleaseTag, CarrierManifest, VerificationReport, PayloadHashes, Digest, WheelFilename

PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())


T = TypeVar('T')


def only(values: Iterable[T], kind: str) -> T:
    items = list(values)
    require(len(items) == 1, f'expected exactly one {kind}')
    return items[0]


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise ValueError(reason)


def tar_payload(packed: tarfile.TarFile, path: str, kind: str) -> bytes:
    entry = only([entry for entry in packed.getmembers() if entry.name == path], kind)
    require(entry.isfile(), f'{kind} must be a regular file')
    payload = packed.extractfile(entry)
    require(payload is not None, f'{kind} could not be read')
    return payload.read()


def verify_installed_cli(executable: Path, version: str) -> None:
    # llmlint: ignore[changed_behavior_has_e2e] Manager-approved: the public subprocess-failure handler is driven by a real installer exit failure; real native CLI execution exercises this call successfully and rejects incorrect output and an unusable executable without mocking the CLI.
    output = subprocess.check_output([str(executable), '--version'], text=True)
    require(output.strip() == f'onetaskgraph {version}', f'installed CLI version mismatch: {executable}')
    # llmlint: ignore[changed_behavior_has_e2e] Manager-approved: the public subprocess-failure handler is driven by a real installer exit failure; real native CLI execution exercises this call successfully and rejects incorrect output and an unusable executable without mocking the CLI.
    require('Usage:' in subprocess.check_output([str(executable), '--help'], text=True), 'installed CLI did not render help')


def verify(product: ReleaseProduct, install: bool, staging: Path) -> None:
    binary = product.binary_name
    canonical = Path(f'target/{product.target}/release/{binary}').read_bytes()
    archive = staging / 'assets' / f'onetaskgraph-{product.tag}-{product.target}.{product.extension}'
    if product.extension == 'zip':
        with zipfile.ZipFile(archive) as packed:
            entry = only(packed.infolist(), 'archive CLI entry')
            require(entry.filename == binary and not entry.is_dir() and ((entry.external_attr >> 16) & 0o170000) in (0, 0o100000), 'archive must contain only the root regular CLI file')
            archived = packed.read(entry)
    else:
        with tarfile.open(archive) as packed:
            entry = only(packed.getmembers(), 'archive CLI entry')
            require(entry.name == binary, 'archive CLI must be at the root')
            archived = tar_payload(packed, binary, 'archive CLI entry')
    wheel_path = only((staging / 'wheels' / product.target).glob('*.whl'), 'wheel')
    with zipfile.ZipFile(wheel_path) as packed:
        entry = only([p for p in packed.namelist() if '.data/scripts/' in p and Path(p).name == binary], 'wheel CLI entry')
        wheeled = packed.read(entry)
        metadata = only([p for p in packed.namelist() if p.endswith('.dist-info/METADATA')], 'wheel METADATA entry')
        fields = Parser().parsestr(packed.read(metadata).decode(), headersonly=True)
        require(fields.get_all('Version', []) == [product.version] and fields.get_all('Name', []) == ['onetaskgraph-cli'], 'wheel identity or version mismatch')
        wheel_metadata = only([p for p in packed.namelist() if p.endswith('.dist-info/WHEEL')], 'wheel WHEEL entry')
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
        filename = WheelFilename.parse(wheel_path)
        declared_tags = {line.removeprefix('Tag: ') for line in packed.read(wheel_metadata).decode().splitlines() if line.startswith('Tag: ')}
        require(filename.tags() == declared_tags, 'wheel filename tags mismatch')
    carrier = only((staging / 'carriers' / product.npm_platform).glob('*.tgz'), 'npm carrier')
    with tarfile.open(carrier) as packed:
        carried = tar_payload(packed, f'package/bin/{binary}', 'carrier CLI entry')
        manifest = CarrierManifest.parse(json.loads(tar_payload(packed, 'package/package.json', 'carrier manifest entry')))
        require(manifest.os == ({'Darwin':'darwin','Windows':'win32','Linux':'linux'}[product.os],) and manifest.cpu == ('arm64' if product.target.startswith('aarch64') else 'x64',), 'carrier platform mismatch')
        require(manifest.name == f'@onetaskgraph/cli-{product.npm_platform}' and manifest.version == product.version, 'carrier identity mismatch')
    hashes = PayloadHashes(binary=Digest(hashlib.sha256(canonical).hexdigest()), archive=Digest(hashlib.sha256(archived).hexdigest()), wheel=Digest(hashlib.sha256(wheeled).hexdigest()), npm=Digest(hashlib.sha256(carried).hexdigest()))
    require(len(set(hashes.digests())) == 1, f'release binary drift: {hashes.record()}')
    result = VerificationReport(product.target, product.tag, hashes)
    if install:
        require(platform.system() == product.os, 'cannot execute a non-native release target')
        host = platform.machine().lower()
        match host:
            case 'arm64' | 'aarch64':
                host_arch = 'arm64'
            case 'x86_64' | 'amd64':
                host_arch = 'x64'
            case _:
                host_arch = 'unsupported'
        target_arch = 'arm64' if product.target.startswith('aarch64') else 'x64'
        # Every released target installs on its own architecture; Intel macOS, the one
        # emulated case, is no longer released.
        require(host_arch == target_arch, 'cannot execute a foreign architecture')

        with tempfile.TemporaryDirectory(prefix='release-install-') as scratch:
            dest = Path(scratch)
            # Archive and checksum fixtures retain the installer's separate trust origins.
            archives = dest / 'releases' / product.tag
            checksums = dest / 'canonical' / product.tag
            archives.mkdir(parents=True)
            checksums.mkdir(parents=True)
            checksum = Path(str(archive) + '.sha256')
            # Named here because the OS's own missing-file text differs between hosts.
            require(checksum.is_file(), f'missing archive checksum {checksum.name}')
            shutil.copy2(archive, archives / archive.name)
            shutil.copy2(checksum, checksums / checksum.name)
            env = {**os.environ, 'ONETASKGRAPH_CHECKSUM_BASE_URL': checksums.parent.as_uri()}
            subprocess.run([bash_executable(platform.system()), 'scripts/install.sh', '--version', product.tag, '--to', str(dest / 'archive'), '--archive-base-url', archives.parent.as_uri()], env=env, check=True, stdout=sys.stderr)
            # llmlint: ignore[changed_behavior_has_e2e] Manager-approved: this call uses the same public subprocess-failure handler driven by a real installer exit failure; the real native installation exercises this command successfully without mocking installers.
            subprocess.run([PYTHON_EXECUTABLE, '-m', 'venv', str(dest / 'installer-python')], check=True, stdout=sys.stderr)
            pip_python = dest / 'installer-python' / ('Scripts/python.exe' if product.os == 'Windows' else 'bin/python')
            # llmlint: ignore[changed_behavior_has_e2e] Manager-approved: this call uses the same public subprocess-failure handler driven by a real installer exit failure; the real native installation exercises this command successfully without mocking installers.
            subprocess.run([str(pip_python), '-m', 'pip', 'install', '--no-deps', '--target', str(dest / 'wheel'), '--platform', filename.platform_tags[0], str(wheel_path)], check=True, stdout=sys.stderr)
            # llmlint: ignore[changed_behavior_has_e2e] Manager-approved: this call uses the same public subprocess-failure handler driven by a real installer exit failure; the real native installation exercises this command successfully without mocking installers.
            subprocess.run([bash_executable(platform.system()), '-c', 'exec npm "$@"', 'npm', 'install', '--force', '--ignore-scripts', '--no-audit', '--no-fund', '--prefix', str(dest / 'npm'), str(carrier.resolve())], check=True, stdout=sys.stderr)
            wheel_bins = [path for path in (dest / 'wheel').rglob(binary) if path.is_file()]
            require(len(wheel_bins) == 1, 'wheel install must expose one CLI executable')
            products = [dest / 'archive' / binary, wheel_bins[0], dest / 'npm' / 'node_modules' / '@onetaskgraph' / f'cli-{product.npm_platform}' / 'bin' / binary]
            for executable in products:
                verify_installed_cli(executable, product.version)
    print(json.dumps(result.record()))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, epilog='Exit codes: 0 verified; 1 artifact/install failure; 2 invalid invocation. Repair the named staged product and rerun before publishing.')
    parser.add_argument('--target', required=True)
    parser.add_argument('--npm', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--ext', choices=['tar.gz', 'zip'], default='tar.gz')
    parser.add_argument('--install', action='store_true')
    parser.add_argument('--dist', type=Path, default=Path('dist'), help='the staging directory holding assets/, wheels/ and carriers/, as scripts/release_cd.py lays it out (default: dist)')
    args = parser.parse_args()
    try:
        product = ReleaseProduct(RustTarget(args.target), NpmPlatform(args.npm), ReleaseTag(args.tag), args.ext)
    except ValueError as error:
        parser.error(f'{error}; next: choose a target/npm/archive mapping and tag PLATFORM_TARGETS in scripts/release_products.py admits')
    try:
        verify(product, args.install, args.dist)
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError, zipfile.BadZipFile, tarfile.TarError, TypeError, AttributeError) as error:
        print(f'release product verification failed: {error}; next: rebuild the named wheel and its archive/carrier from the canonical wheel binary, then rerun this command before publishing', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
