"""Validated identities for the repository's staged release products."""
from dataclasses import dataclass
from typing import Literal, NewType
import re
from pathlib import Path
from product_versions import SEMANTIC_VERSION_RE, SemanticVersion


RustTarget = NewType("RustTarget", str)
NpmPlatform = NewType("NpmPlatform", str)
ReleaseTag = NewType("ReleaseTag", str)
RunnerLabel = NewType("RunnerLabel", str)
ArchiveFormat = Literal["tar.gz", "zip"]


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
        if not self.tag.startswith('v') or not SEMANTIC_VERSION_RE.fullmatch(self.tag[1:]):
            raise ValueError('invalid semantic release tag')
        if self.extension not in ('tar.gz', 'zip'):
            raise ValueError('invalid release archive extension')
        workflow = (Path(__file__).resolve().parents[1] / '.github/workflows/release.yml').read_text()
        mappings = {target: (npm, extension) for target, extension, npm in re.findall(r'target: ([a-z0-9_-]+), ext: ([a-z.]+), npm: ([a-z0-9-]+)', workflow)}
        mac_pairs = re.search(r'^      TARGETS: (.*)$', workflow, re.M)
        if mac_pairs:
            mappings.update({target: (npm, 'tar.gz') for target, npm in (pair.split('=') for pair in mac_pairs[1].split())})
        if mappings.get(self.target) != (self.npm_platform, self.extension):
            raise ValueError('unsupported or inconsistent target/npm/archive combination; use a mapping declared in release.yml')


    @property
    def version(self) -> SemanticVersion:
        return SemanticVersion(self.tag[1:])

    @property
    def binary_name(self) -> str:
        return 'onetaskgraph.exe' if self.extension == 'zip' else 'onetaskgraph'

    @property
    def os(self) -> str:
        return 'Darwin' if self.target.endswith('apple-darwin') else 'Windows' if self.extension == 'zip' else 'Linux'

    @property
    def wheel_arch(self) -> str:
        if self.target.startswith('aarch64'):
            return 'arm64' if self.os == 'Darwin' else 'aarch64'
        return 'amd64' if self.os == 'Windows' else 'x86_64'
