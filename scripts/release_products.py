"""Validated identities for the repository's staged release products."""

# llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] These identities are what the release's own scripts in this project share — release_cd.py, verify-release-products.py and release-wheel-binary.py read them — and this file was the release products' home in the `scripts` project before this change; AGENTS.md names `PLATFORM_TARGETS` here as where the released platforms are declared.
from dataclasses import dataclass
import json
from typing import Literal, NewType
import re
import shutil
import subprocess
from pathlib import Path
from product_versions import SemanticVersion


RustTarget = NewType("RustTarget", str)
NpmPlatform = NewType("NpmPlatform", str)
ReleaseTag = NewType("ReleaseTag", str)
RunnerLabel = NewType("RunnerLabel", str)
ArchiveFormat = Literal["tar.gz", "zip"]
ReleaseOS = Literal["Darwin", "Windows", "Linux"]
WheelArchitecture = Literal["arm64", "aarch64", "amd64", "x86_64"]
# The three release platforms, each one CD job of .github/workflows/release.yml
# (`build-<platform>`) and one recipe of the justfile (`release-<platform>`).
ReleasePlatform = Literal["linux", "macos", "windows"]

# The one grammar a release tag is held to: the GitHub Release the release-plz job cuts is
# named by it, scripts/install.sh repeats it, and scripts/check-distribution-contract.sh
# holds that copy to this one.
RELEASE_TAG_PATTERN = r'^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'


@dataclass(frozen=True)
class PlatformTarget:
    """One Rust target a release builds, and every name its artifacts are published under.

    The wheel tag is part of the contract rather than an observation: the build refuses a
    wheel maturin tagged otherwise, so a runner image whose glibc moved changes nothing
    published without this table — and the published artifacts of v0.7.1 — saying so.
    """

    platform: ReleasePlatform
    runner: RunnerLabel
    target: RustTarget
    npm_platform: NpmPlatform
    extension: ArchiveFormat
    wheel_platform_tag: str


# Every target a release builds, in the order each platform's job builds them. Linux arm64
# and Intel macOS are no longer released: scripts/install.sh and the npm launcher refuse both
# by name, naming cargo.
PLATFORM_TARGETS: tuple[PlatformTarget, ...] = (
    PlatformTarget("linux", RunnerLabel("ubuntu-latest"), RustTarget("x86_64-unknown-linux-gnu"), NpmPlatform("linux-x64"), "tar.gz", "manylinux_2_39_x86_64"),
    PlatformTarget("macos", RunnerLabel("macos-latest"), RustTarget("aarch64-apple-darwin"), NpmPlatform("darwin-arm64"), "tar.gz", "macosx_11_0_arm64"),
    PlatformTarget("windows", RunnerLabel("windows-latest"), RustTarget("x86_64-pc-windows-msvc"), NpmPlatform("win32-x64"), "zip", "win_amd64"),
)
# maturin's `bin` bindings carry no Python ABI, so every CLI wheel is py3-none.
WHEEL_INTERPRETER_TAGS = "py3-none"


def platform_targets(platform: str) -> tuple[PlatformTarget, ...]:
    selected = tuple(row for row in PLATFORM_TARGETS if row.platform == platform)
    if not selected:
        raise ValueError(f'unknown release platform {platform!r}; choose one of linux, macos, windows')
    return selected


def validate_tag(value: str) -> ReleaseTag:
    if re.fullmatch(RELEASE_TAG_PATTERN, value) is None:
        raise ValueError(f'invalid semantic release tag {value!r}; use vX.Y.Z')
    return ReleaseTag(value)


def bash_executable(system: str) -> str:
    """Select Git Bash on Windows, where PATH may prefer the WSL launcher."""
    if system == 'Windows':
        git = shutil.which('git')
        if git is not None:
            # The git on PATH may be a shim outside the installation, so git's
            # own exec path (mingw64/libexec/git-core) is asked as well.
            starts = [Path(git).resolve().parent]
            try:
                exec_path = subprocess.run([git, '--exec-path'], capture_output=True, text=True, check=True).stdout.strip()
            except (OSError, subprocess.CalledProcessError):
                exec_path = ''
            if exec_path and Path(exec_path).is_absolute():
                starts.append(Path(exec_path).resolve())
            for start in starts:
                # Git Bash exposes git from usr/bin or mingw*/bin; native PATH
                # from cmd or bin; the exec path sits three levels down.
                for directory in [start, *start.parents][:4]:
                    for relative in ('bin/bash.exe', 'usr/bin/bash.exe'):
                        candidate = directory / relative
                        if candidate.is_file():
                            return str(candidate)
        raise ValueError('Git Bash could not be found beside git; install Git for Windows before packaging')
    bash = shutil.which('bash')
    if bash is None:
        raise ValueError('bash could not be found; install bash before packaging')
    return str(Path(bash).resolve())


def validate_target(value: str) -> None:
    if not re.fullmatch(r'[a-z0-9_]+(?:-[a-z0-9_]+)+', value):
        raise ValueError('invalid Rust target identifier')


def validate_npm_platform(value: str) -> None:
    if not re.fullmatch(r'[a-z0-9]+-[a-z0-9]+', value):
        raise ValueError('invalid npm platform identifier')


@dataclass(frozen=True)
class TargetMapping:
    npm_platform: NpmPlatform
    extension: ArchiveFormat


def target_mapping(npm: str, extension: str) -> TargetMapping:
    match extension:
        case 'tar.gz':
            return TargetMapping(NpmPlatform(npm), 'tar.gz')
        case 'zip':
            return TargetMapping(NpmPlatform(npm), 'zip')
        case _:
            raise ValueError('unsupported archive format in the release workflow')



@dataclass(frozen=True)
class ReleaseProduct:
    target: RustTarget
    npm_platform: NpmPlatform
    tag: ReleaseTag
    extension: ArchiveFormat

    def __post_init__(self) -> None:
        validate_target(self.target)
        validate_npm_platform(self.npm_platform)

        if self.extension not in ('tar.gz', 'zip'):
            raise ValueError('invalid release archive extension')
        validate_tag(self.tag)
        mappings = {row.target: target_mapping(row.npm_platform, row.extension) for row in PLATFORM_TARGETS}
        if mappings.get(self.target) != TargetMapping(self.npm_platform, self.extension):
            raise ValueError('unsupported or inconsistent target/npm/archive combination; use a mapping PLATFORM_TARGETS in scripts/release_products.py declares')


    @property
    def version(self) -> SemanticVersion:
        return SemanticVersion(self.tag[1:])

    @property
    def binary_name(self) -> str:
        return 'onetaskgraph.exe' if self.extension == 'zip' else 'onetaskgraph'

    @property
    def os(self) -> ReleaseOS:
        match self.target, self.extension:
            case target, _ if target.endswith('apple-darwin'):
                return 'Darwin'
            case _, 'zip':
                return 'Windows'
            case _:
                return 'Linux'

    @property
    def wheel_arch(self) -> WheelArchitecture:
        if self.target.startswith('aarch64'):
            return 'arm64' if self.os == 'Darwin' else 'aarch64'
        return 'amd64' if self.os == 'Windows' else 'x86_64'


@dataclass(frozen=True)
class CarrierManifest:
    name: str
    version: str
    os: tuple[str, ...]
    cpu: tuple[str, ...]

    @classmethod
    def parse(cls, value: object) -> 'CarrierManifest':
        if not isinstance(value, dict) or not isinstance(value.get('name'), str) or not isinstance(value.get('version'), str):
            raise ValueError('invalid carrier identity fields')
        for key in ('os', 'cpu'):
            if not isinstance(value.get(key), list) or any(not isinstance(item, str) for item in value[key]):
                raise ValueError('invalid carrier platform fields')
        return cls(value['name'], value['version'], tuple(value['os']), tuple(value['cpu']))


Digest = NewType('Digest', str)


@dataclass(frozen=True)
class PayloadHashes:
    binary: Digest
    archive: Digest
    wheel: Digest
    npm: Digest

    @classmethod
    def parse(cls, value: object) -> 'PayloadHashes':
        if not isinstance(value, dict) or set(value) != {'binary', 'archive', 'wheel', 'npm'} or any(not isinstance(digest, str) or re.fullmatch(r'[a-f0-9]{64}', digest) is None for digest in value.values()):
            raise ValueError('invalid verification payload hashes')
        return cls(Digest(value['binary']), Digest(value['archive']), Digest(value['wheel']), Digest(value['npm']))

    def digests(self) -> tuple[Digest, ...]:
        return self.binary, self.archive, self.wheel, self.npm

    def record(self) -> dict[str, str]:
        return {'binary': self.binary, 'archive': self.archive, 'wheel': self.wheel, 'npm': self.npm}


@dataclass(frozen=True)
class WheelFilename:
    distribution_version: str
    python_tags: tuple[str, ...]
    abi_tags: tuple[str, ...]
    platform_tags: tuple[str, ...]

    @classmethod
    def parse(cls, path: Path) -> 'WheelFilename':
        parts = path.stem.rsplit('-', 3)
        if len(parts) != 4:
            raise ValueError('invalid wheel filename tags')
        identity, python, abi, platform = parts
        if any(not re.fullmatch(r'[A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)*', value) for value in (python, abi, platform)):
            raise ValueError('invalid wheel filename tag grammar')
        return cls(identity, tuple(python.split('.')), tuple(abi.split('.')), tuple(platform.split('.')))

    def tags(self) -> set[str]:
        return {f'{python}-{abi}-{platform}' for python in self.python_tags for abi in self.abi_tags for platform in self.platform_tags}


@dataclass(frozen=True)
class VerificationReport:
    target: RustTarget
    tag: ReleaseTag
    payload_hashes: PayloadHashes

    @classmethod
    def parse(cls, text: str) -> 'VerificationReport':
        value = json.loads(text)
        if not isinstance(value, dict) or not isinstance(value.get('target'), str) or not isinstance(value.get('tag'), str):
            raise ValueError('invalid verification report')
        validate_target(value['target'])
        if re.fullmatch(RELEASE_TAG_PATTERN, value['tag']) is None:
            raise ValueError('invalid verification report release tag')
        return cls(RustTarget(value['target']), ReleaseTag(value['tag']), PayloadHashes.parse(value.get('payloadHashes')))

    def record(self) -> dict[str, object]:
        return {'target': self.target, 'tag': self.tag, 'payloadHashes': self.payload_hashes.record()}
