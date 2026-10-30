# llmlint: ignore-file[async_typed_clients_at_boundaries] This subprocess journey deliberately executes the production release phases sequentially and doubles only transport; an async service client would replace the actual CLI boundaries the task requires it to prove.
"""Drive the release's packaging and installation over a real host-native wheel."""
import argparse
import base64
import csv
import hashlib
import platform
import io
import json
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import zipfile
from release_products import PLATFORM_TARGETS, PlatformTarget, RustTarget, VerificationReport, validate_tag, validate_target
from release_cd import plan
from product_versions import SEMANTIC_VERSION_RE, SemanticVersion

PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())
# In Actions each refusal and each packaging run says what it took, because the hosted
# runners are where this journey is slow and its own output says nothing while it works.
TIMED = os.environ.get('GITHUB_ACTIONS') == 'true'


def timed(label: str, started: float) -> None:
    if TIMED:
        print(f'release products timing: {time.monotonic() - started:6.1f}s {label}', file=sys.stderr)
# Tampered fixtures are judged by their payload, never by compression level.
FIXTURE_COMPRESSION = 1


def target_row(target: RustTarget) -> PlatformTarget:
    candidates = [row for row in PLATFORM_TARGETS if row.target == target]
    if len(candidates) != 1:
        raise argparse.ArgumentError(None, 'unsupported --target; choose one PLATFORM_TARGETS in scripts/release_products.py declares')
    return candidates[0]


def staging_tree(root: Path, tree: Path) -> None:
    """The files the production packaging phase reads."""
    shutil.copytree(root / 'scripts', tree / 'scripts')
    shutil.copytree(root / 'npm/platforms', tree / 'npm/platforms')
    shutil.copy2(root / 'justfile', tree / 'justfile')


def stage_only(binary: Path, target: RustTarget) -> None:
    """Drive the real packaging phase with the relative-interpreter PATH, without rebuilding.

    Only the wheel input is constructed here; release_cd.py performs the same extraction,
    archive/checksum creation and carrier packaging the full distribution journey drives.
    """
    root = Path(__file__).resolve().parents[1]
    selected = target_row(target)
    manifest = json.loads((root / 'npm/platforms' / selected.npm_platform / 'package.json').read_text())
    value = manifest.get('version') if isinstance(manifest, dict) else None
    if not isinstance(value, str) or not SEMANTIC_VERSION_RE.fullmatch(value):
        raise ValueError('invalid carrier version; restore the version written by scripts/set-version.sh')
    version = SemanticVersion(value)
    tag = validate_tag(f'v{version}')
    with tempfile.TemporaryDirectory(prefix='release staging-') as scratch:
        tree = Path(scratch)
        staging_tree(root, tree)
        probe = os.environ.get('ONETASKGRAPH_RELATIVE_INTERPRETER_PROBE')
        if probe:
            shutil.copytree(root / probe, tree / probe, ignore=shutil.ignore_patterns('call-sites.*', 'stdin.*'))
        wheel_name, = [artifact.file for artifact in plan(selected.platform, tag)
                       if artifact.target == target and artifact.kind == 'wheel']
        wheels = tree / 'dist/wheels' / target
        wheels.mkdir(parents=True)
        name = 'onetaskgraph.exe' if selected.extension == 'zip' else 'onetaskgraph'
        with zipfile.ZipFile(wheels / wheel_name, 'w', zipfile.ZIP_DEFLATED, compresslevel=FIXTURE_COMPRESSION) as wheel:
            wheel.write(binary, f'onetaskgraph_cli-{version}.data/scripts/{name}')
        subprocess.run(['just', f'release-{selected.platform}',
                        'package', '--tag', tag, '--dist', shlex.quote(str(tree / 'dist'))],
                       cwd=tree, check=True, stdout=subprocess.DEVNULL)
        asset = tree / 'dist/assets' / f'onetaskgraph-{tag}-{target}.{selected.extension}'
        if not asset.is_file() or not Path(f'{asset}.sha256').is_file():
            raise ValueError(f'the packaging phase staged no {asset.name} and checksum')


def run(wheel_dir: Path, target: RustTarget) -> None:
    root = Path(__file__).resolve().parents[1]
    selected = target_row(target)
    manifest = tomllib.loads((root / 'pyproject.toml').read_text())
    project = manifest.get('project')
    if not isinstance(project, dict):
        raise ValueError('invalid manifest project table; restore pyproject.toml project metadata')
    version = project.get('version')
    if not isinstance(version, str) or not SEMANTIC_VERSION_RE.fullmatch(version):
        raise ValueError('invalid manifest version; restore the version written by scripts/set-version.sh')
    version = SemanticVersion(version)
    with tempfile.TemporaryDirectory(prefix='release-manifest-refusal-') as malformed:
        malformed_root = Path(malformed)
        shutil.copytree(root / 'scripts', malformed_root / 'scripts')
        (malformed_root / 'pyproject.toml').write_text('project = "invalid table"\n')
        rejected = subprocess.run([PYTHON_EXECUTABLE, 'scripts/test-release-products.py', '--wheel-dir', str(wheel_dir.resolve()), '--target', target], cwd=malformed_root, capture_output=True, text=True)
        if rejected.returncode != 1 or 'invalid manifest project table' not in rejected.stderr:
            raise ValueError('malformed project table did not produce its boundary diagnostic')
    binary = 'onetaskgraph.exe' if selected.extension == 'zip' else 'onetaskgraph'
    with tempfile.TemporaryDirectory(prefix='release products-') as scratch:
        tree = Path(scratch)
        staging_tree(root, tree)
        wheels = tree / 'dist/wheels' / target
        shutil.copytree(wheel_dir.resolve(), wheels)
        canonical = tree / f'target/{target}/release/{binary}'
        extraction = [PYTHON_EXECUTABLE, 'scripts/release-wheel-binary.py', '--wheels', str(wheels), '--binary', str(canonical)]
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
            with zipfile.ZipFile(wheel_path, 'w', compresslevel=FIXTURE_COMPRESSION) as malformed:
                for entry in entries:
                    malformed.writestr(entry, b'invalid')
            rejected = subprocess.run(extraction, cwd=tree, capture_output=True, text=True)
            if rejected.returncode == 0 or 'exactly one CLI' not in rejected.stderr or canonical.exists():
                raise ValueError('malformed wheel scripts were accepted by extraction')
        wheel_path.write_bytes(original_wheel)
        subprocess.run([PYTHON_EXECUTABLE, 'scripts/release-wheel-binary.py', '--wheels', str(wheels), '--binary', str(canonical)], cwd=tree, check=True, stdout=subprocess.DEVNULL)
        package = ['just', f'release-{selected.platform}', 'package', '--tag', f'v{version}', '--dist', shlex.quote(str(tree / 'dist'))]
        started = time.monotonic()
        subprocess.run(package, cwd=tree, check=True, stdout=subprocess.DEVNULL)
        timed('packaging', started)
        # Every refusal below corrupts the products this one packaging produced and is judged
        # against them; they are kept here once and put back at the end, rather than packaged
        # again, for the final verification.
        pristine = tree / 'pristine'
        shutil.copytree(tree / 'dist', pristine / 'dist')
        shutil.copy2(canonical, pristine / binary)
        command = [PYTHON_EXECUTABLE, 'scripts/verify-release-products.py', '--target', target, '--npm', selected.npm_platform, '--tag', f'v{version}', '--ext', selected.extension, '--dist', str(tree / 'dist')]
        started = time.monotonic()
        installed = subprocess.run(command + ['--install'], cwd=tree, capture_output=True, text=True)
        timed('first install', started)
        if installed.returncode:
            print(installed.stderr, file=sys.stderr)
            raise subprocess.CalledProcessError(installed.returncode, command, installed.stdout, installed.stderr)
        report = VerificationReport.parse(installed.stdout)
        expected_hash = hashlib.sha256(canonical.read_bytes()).hexdigest()
        if report.target != target or report.tag != f'v{version}' or set(report.payload_hashes.digests()) != {expected_hash}:
            raise ValueError('installer did not emit the documented single JSON verification result')

        def refuses(reason: str) -> None:
            started = time.monotonic()
            result = subprocess.run(command + ['--install'], cwd=tree, capture_output=True, text=True)
            timed(f'refusal: {reason}', started)
            if result.returncode == 0 or reason not in result.stderr:
                raise ValueError(f'tampered staged product did not fail for {reason}: status={result.returncode}, stderr={result.stderr}; repair the production verifier before publishing')

        for container in [wheel_path, next((tree / f'dist/carriers/{selected.npm_platform}').glob('*.tgz'))]:
            original = container.read_bytes()
            container.write_bytes(b'corrupt container')
            refuses('verification failed')
            container.write_bytes(original)
        checksum = tree / 'dist/assets' / f'onetaskgraph-v{version}-{target}.{selected.extension}.sha256'
        original_checksum = checksum.read_bytes()
        checksum.write_text('0' * 64 + '  incorrect-archive\n')
        refuses('checksum mismatch')
        checksum.write_bytes(original_checksum)
        checksum.unlink()
        refuses(f'missing archive checksum {checksum.name}')
        checksum.write_bytes(original_checksum)
        original_binary = canonical.read_bytes()
        canonical.write_bytes(original_binary + b'corrupt')
        refuses('release binary drift')
        canonical.write_bytes(original_binary)
        wheel_path, = wheels.glob('*.whl')
        original_wheel = wheel_path.read_bytes()

        def alter_wheel(suffix: str, replacement: bytes) -> None:
            with zipfile.ZipFile(wheel_path) as wheel:
                members = [(info, wheel.read(info.filename)) for info in wheel.infolist()]
            with zipfile.ZipFile(wheel_path, 'w', compresslevel=FIXTURE_COMPRESSION) as wheel:
                for info, contents in members:
                    wheel.writestr(info, replacement if info.filename.endswith(suffix) else contents)

        # Each packaged input is independently corrupted while the canonical binary
        # stays intact, so deleting any one comparison makes a journey fail.
        archive = tree / 'dist/assets' / f'onetaskgraph-v{version}-{target}.{selected.extension}'
        original_archive = archive.read_bytes()
        if selected.extension == 'zip':
            with zipfile.ZipFile(archive, 'w', compresslevel=FIXTURE_COMPRESSION) as packed:
                packed.writestr(binary, original_binary + b'corrupt')
        else:
            with tarfile.open(archive, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
                info = tarfile.TarInfo(binary)
                info.size = len(original_binary) + len(b'corrupt')
                packed.addfile(info, io.BytesIO(original_binary + b'corrupt'))
        refuses('release binary drift')
        archive.write_bytes(original_archive)
        if selected.extension == 'tar.gz':
            archive.write_bytes(b'not a tar archive')
            refuses('verification failed')
            for names in [[], [binary, binary], [binary, 'extra']]:
                with tarfile.open(archive, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
                    for name in names:
                        info = tarfile.TarInfo(name)
                        info.size = len(original_binary)
                        packed.addfile(info, io.BytesIO(original_binary))
                refuses('expected exactly one archive CLI entry')
            with tarfile.open(archive, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
                info = tarfile.TarInfo('nested/' + binary)
                info.size = len(original_binary)
                packed.addfile(info, io.BytesIO(original_binary))
            refuses('archive CLI must be at the root')
            with tarfile.open(archive, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
                info = tarfile.TarInfo(binary)
                info.type = tarfile.SYMTYPE
                info.linkname = 'other'
                packed.addfile(info)
            refuses('archive CLI entry must be a regular file')
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
        with tarfile.open(carrier, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
            for info, contents in members:
                if info.name == f'package/bin/{binary}':
                    contents += b'corrupt'
                    info.size = len(contents)
                packed.addfile(info, io.BytesIO(contents) if contents is not None else None)
        refuses('release binary drift')
        carrier.write_bytes(original_carrier)
        with tarfile.open(carrier) as packed:
            contract_members = [(info, packed.extractfile(info).read() if info.isfile() else None) for info in packed.getmembers()]
        for path, kind in [(f'package/bin/{binary}', 'carrier CLI entry'), ('package/package.json', 'carrier manifest entry')]:
            for duplicate in [False, True]:
                with tarfile.open(carrier, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
                    for info, data in contract_members:
                        if info.name != path or duplicate:
                            packed.addfile(info, io.BytesIO(data) if data is not None else None)
                        if info.name == path and duplicate:
                            packed.addfile(info, io.BytesIO(data) if data is not None else None)
                refuses('expected exactly one ' + kind)
            with tarfile.open(carrier, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
                for info, data in contract_members:
                    if info.name == path:
                        bad = tarfile.TarInfo(path)
                        bad.type = tarfile.SYMTYPE
                        bad.linkname = 'other'
                        packed.addfile(bad)
                    else:
                        packed.addfile(info, io.BytesIO(data) if data is not None else None)
            refuses(kind + ' must be a regular file')
            carrier.write_bytes(original_carrier)
        # llmlint: ignore[changed_behavior_has_e2e] This fixture explicitly opens in write mode ('w:gz'); the real native product journey and distribution test execute this malformed-manifest refusal and its recovery.
        with tarfile.open(carrier, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
            for info, data in contract_members:
                if info.name == 'package/package.json':
                    info = tarfile.TarInfo(info.name)
                    data = b'{malformed JSON'
                    info.size = len(data)
                packed.addfile(info, io.BytesIO(data) if data is not None else None)
        refuses('Expecting property name')
        carrier.write_bytes(original_carrier)
        alter_wheel('/METADATA', f'Name: onetaskgraph-cli\nName: foreign-cli\nVersion: {version}\n'.encode())
        refuses('wheel identity or version mismatch')
        wheel_path.write_bytes(original_wheel)
        with zipfile.ZipFile(wheel_path, 'a') as packed:
            packed.writestr(f'foreign-package/{binary}', original_binary)
        refuses('wheel install must expose one CLI executable')
        wheel_path.write_bytes(original_wheel)
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
        foreign_name = wheel_path.with_name('-'.join(wheel_path.stem.rsplit('-', 3)[:3]) + '-foreign_arch.whl')
        wheel_path.rename(foreign_name)
        refuses('wheel filename tags mismatch')
        foreign_name.rename(wheel_path)
        invalid_tags = wheel_path.with_name('-'.join(wheel_path.stem.rsplit('-', 3)[:3]) + '-bad!tag.whl')
        wheel_path.rename(invalid_tags)
        refuses('invalid wheel filename tag grammar')
        invalid_tags.rename(wheel_path)
        installer = tree / 'scripts/install.sh'
        installer_body = installer.read_bytes()
        installer.write_text('exit 37\n')
        refuses('returned non-zero exit status 37')
        installer.write_bytes(installer_body)


        carrier, = (tree / f'dist/carriers/{selected.npm_platform}').glob('*.tgz')
        original_carrier = carrier.read_bytes()
        with tarfile.open(carrier) as packed:
            members = [(info, packed.extractfile(info).read() if info.isfile() else None) for info in packed.getmembers()]
        with tarfile.open(carrier, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
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
            with tarfile.open(carrier, 'w:gz', compresslevel=FIXTURE_COMPRESSION) as packed:
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
        failures = [(version.encode(), b'9' * len(version), 'installed CLI version mismatch'), (b'Usage:', b'Ubage:', 'installed CLI did not render help')]
        if platform.system() == 'Linux':
            failures.append((b'\x7fELF', b'BAD!', 'Exec format error'))
        for old, new, reason in failures:
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
            with zipfile.ZipFile(wheel_path, 'w', compresslevel=FIXTURE_COMPRESSION) as wheel:
                for info, data in contents.values():
                    wheel.writestr(info, data)
            started = time.monotonic()
            subprocess.run(package, cwd=tree, check=True, stdout=subprocess.DEVNULL)
            timed('packaging', started)
            refuses(reason)
            canonical.write_bytes(original_binary)
            wheel_path.write_bytes(original_wheel)
        # Each corrupted case replaces its predecessor; restore the pristine products once,
        # before the final successful verification, rather than between corruptions.
        shutil.rmtree(tree / 'dist')
        shutil.copytree(pristine / 'dist', tree / 'dist')
        shutil.copy2(pristine / binary, canonical)
        subprocess.run(command, cwd=tree, check=True, stdout=subprocess.DEVNULL)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, epilog='Exit codes: 0 journey passed; 1 packaging/install/proof failure; 2 invalid invocation. Repair the named packaging phase or staged product before retrying.')
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument('--wheel-dir', type=Path)
    inputs.add_argument('--stage-only', type=Path, metavar='BINARY', help='run packaging alone over this built binary')
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    try:
        validate_target(args.target)
    except ValueError as error:
        parser.error(f'{error}; next: choose a Rust target scripts/release_products.py declares')
    try:
        if args.stage_only is not None:
            stage_only(args.stage_only, RustTarget(args.target))
        else:
            run(args.wheel_dir, RustTarget(args.target))
    except argparse.ArgumentError as error:
        parser.error(f'{error}; next: choose a Rust target scripts/release_products.py declares')
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError, zipfile.BadZipFile, tarfile.TarError) as error:
        print(f'release product journey failed: {error}; next: repair the packaging in scripts/release_cd.py or its named product verifier, then rerun scripts/test-distribution.sh', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
