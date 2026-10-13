"""Validated identities for the repository's staged release products."""
from dataclasses import dataclass
import json
from typing import Literal, NewType
import re
from pathlib import Path
from product_versions import SemanticVersion


RustTarget = NewType("RustTarget", str)
NpmPlatform = NewType("NpmPlatform", str)
ReleaseTag = NewType("ReleaseTag", str)
RunnerLabel = NewType("RunnerLabel", str)
ArchiveFormat = Literal["tar.gz", "zip"]
ReleaseOS = Literal["Darwin", "Windows", "Linux"]
WheelArchitecture = Literal["arm64", "aarch64", "amd64", "x86_64"]


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
        if not re.fullmatch(r'[a-z0-9_]+(?:-[a-z0-9_]+)+', self.target):
            raise ValueError('invalid Rust target identifier')
        if not re.fullmatch(r'[a-z0-9]+-[a-z0-9]+', self.npm_platform):
            raise ValueError('invalid npm platform identifier')

        if self.extension not in ('tar.gz', 'zip'):
            raise ValueError('invalid release archive extension')
        workflow = (Path(__file__).resolve().parents[1] / '.github/workflows/release.yml').read_text()
        grammars = set(re.findall(r"grep -Eq '([^']+)'", workflow))
        if len(grammars) != 1 or re.fullmatch(next(iter(grammars)), self.tag) is None:
            raise ValueError('invalid semantic release tag; use the release workflow tag grammar')

        mappings = {RustTarget(target): target_mapping(npm, extension) for target, extension, npm in re.findall(r'target: ([a-z0-9_-]+), ext: ([a-z.]+), npm: ([a-z0-9-]+)', workflow)}
        mac_pairs = re.search(r'^      TARGETS: (.*)$', workflow, re.M)
        if mac_pairs:
            mappings.update({RustTarget(target): target_mapping(npm, 'tar.gz') for target, npm in (pair.split('=') for pair in mac_pairs[1].split())})
        if mappings.get(self.target) != TargetMapping(self.npm_platform, self.extension):
            raise ValueError('unsupported or inconsistent target/npm/archive combination; use a mapping declared in release.yml')


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
        return cls(RustTarget(value['target']), ReleaseTag(value['tag']), PayloadHashes.parse(value.get('payloadHashes')))

    def record(self) -> dict[str, object]:
        return {'target': self.target, 'tag': self.tag, 'payloadHashes': self.payload_hashes.record()}
