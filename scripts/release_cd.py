# llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] This repository's release machinery lives in the `scripts` project: release_products.py, release-probe.sh, publish-npm.sh, prepare-release-pr.sh and their checks were there before this change, AGENTS.md names scripts/release_cd.py as the one implementation every CD job's recipe runs, and github-workflows' default input lists each of these files so a change here selects the release journeys and nothing else.
# llmlint: ignore-file[async_typed_clients_at_boundaries] One release job runs these registry calls strictly in order — a publication must land before the verification that reads it back — so there is no concurrent work for an async client to overlap, and the build tools it drives (maturin, npm, uv, cargo) are subprocesses, not services.
"""Build, package, test-install, publish and verify one release platform's artifacts.

This is the whole body of the four CD jobs of .github/workflows/release.yml. Each job calls
it through one justfile recipe — `release-linux`, `release-macos`, `release-windows`, and
`release-publish` for the platform-independent `publish` job — one phase per workflow step,
so that a step is one decision the workflow test can read and a publication cannot run
before the test-install that guards it:

  plan       list, without building, every file, package, wheel tag and target a platform
             releases (the contract scripts and .github/tests hold to v0.7.1's artifacts)
  build      build each target once, package its archive, wheel and npm platform package
             from that one binary, prove the three carry the same bytes, and install and
             run each before anything is published
  publish    send one registry exactly the files the build produced, skipping any already
             there — so a re-run after a failed upload publishes only what is missing
  verify     install the release version back out of one registry on this OS and run it
  available  (`publish` job) confirm a registry resolves the version for every platform
  run        build, then publish to every registry in dry-run mode: the whole job, locally

Every registry address has an override so the journeys can stand a double in for it;
unset, each is the real registry. Exit codes: 0 done; 1 a release step failed (stderr
names it and what to do next); 2 an invalid invocation.
"""

from __future__ import annotations

import argparse
import email.parser
import hashlib
import http.client
import io
import json
import os
import platform as host_platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path
import math
import re
from typing import Iterable, Literal, NewType, NoReturn

from product_versions import read_reconciled_versions
from release_products import (
    PLATFORM_TARGETS,
    WHEEL_INTERPRETER_TAGS,
    PlatformTarget,
    ReleaseTag,
    bash_executable,
    platform_targets,
    validate_tag,
)

ROOT = Path(__file__).resolve().parents[1]
PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())

Registry = Literal["github", "pypi", "npm", "crates"]
ArtifactKind = Literal["archive", "checksum", "wheel", "sdist", "npm-platform", "npm", "crate"]
PackageName = NewType("PackageName", str)
# A release-targets.toml identifier, `<registry>:<name>`.
ReleaseTargetId = NewType("ReleaseTargetId", str)
RepositoryName = NewType("RepositoryName", str)
AssetId = NewType("AssetId", int)
Token = NewType("Token", str)
PlanPlatform = Literal["linux", "macos", "windows", "independent"]
BUILD_PLATFORMS = ("linux", "macos", "windows")
PLATFORM_REGISTRIES: tuple[Registry, ...] = ("github", "pypi", "npm")
INDEPENDENT_REGISTRIES: tuple[Registry, ...] = ("crates", "pypi", "npm")
JOB_OF = {"linux": "build-linux", "macos": "build-macos", "windows": "build-windows", "independent": "publish"}

CLI_DISTRIBUTION = "onetaskgraph-cli"
SDK_DISTRIBUTION = "onetaskgraph-sdk"
# The order each crate reaches crates.io in: a crate is published only after every sibling
# it requires, which scripts/crate-sibling-resolution.sh proves against the registry first.
CRATE_PUBLICATION_ORDER = (
    "onetaskgraph-plugin-api",
    "onetaskgraph-in-memory",
    "onetaskgraph-local-md",
    "onetaskgraph-linear",
    "onetaskgraph-github-projects",
    "onetaskgraph-status-options",
    "onetaskgraph-core",
    "onetaskgraph",
)


@dataclass(frozen=True)
class RootNpmPackage:
    """A platform-independent npm package and the stem `npm pack` names its tarball with."""

    name: PackageName
    tarball_stem: str


ROOT_NPM_PACKAGES = (
    RootNpmPackage(PackageName("@onetaskgraph/cli"), "onetaskgraph-cli"),
    RootNpmPackage(PackageName("@onetaskgraph/sdk"), "onetaskgraph-sdk"),
)

DEFAULT_GITHUB_API = "https://api.github.com"
DEFAULT_PYPI_UPLOAD = "https://upload.pypi.org/legacy/"
DEFAULT_PYPI_SIMPLE = "https://pypi.org/simple/"
DEFAULT_NPM_REGISTRY = "https://registry.npmjs.org/"
RELEASE_DOWNLOADS = "https://github.com/nickderobertis/onetaskgraph/releases/download"
USER_AGENT = "onetaskgraph-release (https://github.com/nickderobertis/onetaskgraph)"


class ReleaseStepFailed(Exception):
    """A release step that cannot go on; carries what failed and the next action."""

    def __init__(self, problem: str, next_action: str) -> None:
        super().__init__(problem)
        self.problem = problem
        self.next_action = next_action


def fail(problem: str, next_action: str) -> NoReturn:
    raise ReleaseStepFailed(problem, next_action)


# urlsplit parses rather than validates, so a host is held to name characters and a port, and
# the whole value to what http.client sends: a space or control character there would escape
# this refusal as an uncaught InvalidURL at the first request.
HOST_AND_PORT = re.compile(r"[A-Za-z0-9.-]+(?::[0-9]+)?")
URL_DISALLOWED = re.compile(r"[\x00-\x20\x7f]")


def endpoint(variable: str, default: str) -> str:
    """A registry address from the environment, held to an http(s) URL naming a host and nothing else.

    Each override exists so the journeys can stand a double in for a registry; whatever it
    names is handed a token, so a value with credentials, a query or no host is refused.
    """
    value = os.environ.get(variable) or default
    try:
        parts = urllib.parse.urlsplit(value)
        usable = parts.scheme in ("http", "https") and bool(HOST_AND_PORT.fullmatch(parts.netloc)) and not URL_DISALLOWED.search(value) and not parts.query and not parts.fragment
        parts.port  # noqa: B018 - raises ValueError on a port that is not 0-65535
    except ValueError:
        usable = False
    if not usable:
        fail(f"{variable} is {value!r}, not an http(s) URL naming a host", f"unset {variable} to use {default}")
    return value


REPOSITORY_NAME = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")


def say(message: str) -> None:
    print(f"release-cd: {message}", file=sys.stderr, flush=True)


@dataclass(frozen=True)
class Artifact:
    """One file a release publishes, and where."""

    platform: PlanPlatform
    registry: Registry
    kind: ArtifactKind
    file: str
    package: PackageName | None = None
    # The release-targets.toml identifier this file is published under: a target's own id,
    # or one its `covers` names. GitHub Release archives belong to no registry, so none.
    release_target: ReleaseTargetId | None = None
    target: str | None = None
    wheel_tags: tuple[str, ...] = ()

    def record(self) -> dict[str, object]:
        value: dict[str, object] = {"job": JOB_OF[self.platform], "platform": self.platform, "registry": self.registry, "kind": self.kind, "file": self.file}
        for key in ("package", "release_target", "target"):
            if getattr(self, key) is not None:
                value[key] = getattr(self, key)
        if self.wheel_tags:
            value["wheel_tags"] = list(self.wheel_tags)
        return value

    @property
    def staged(self) -> bool:
        """Whether the build stages this file; cargo and npm pack crates and root packages at publication."""
        return self.registry != "crates" and not (self.registry == "npm" and self.platform == "independent")


def version_of(tag: ReleaseTag) -> str:
    return tag[1:]


def archive_name(tag: ReleaseTag, row: PlatformTarget) -> str:
    return f"onetaskgraph-{tag}-{row.target}.{row.extension}"


def plan(platform: PlanPlatform, tag: ReleaseTag) -> list[Artifact]:
    version = version_of(tag)
    if platform == "independent":
        artifacts = [Artifact("independent", "crates", "crate", f"{crate}-{version}.crate", PackageName(crate), ReleaseTargetId(f"crate:{crate}")) for crate in CRATE_PUBLICATION_ORDER]
        sdk = PackageName(SDK_DISTRIBUTION)
        artifacts.append(Artifact("independent", "pypi", "sdist", f"onetaskgraph_sdk-{version}.tar.gz", sdk, ReleaseTargetId(f"pypi:{sdk}")))
        artifacts.append(Artifact("independent", "pypi", "wheel", f"onetaskgraph_sdk-{version}-py3-none-any.whl", sdk, ReleaseTargetId(f"pypi:{sdk}"), wheel_tags=("py3-none-any",)))
        artifacts.extend(Artifact("independent", "npm", "npm", f"{package.tarball_stem}-{version}.tgz", package.name, ReleaseTargetId(f"npm:{package.name}")) for package in ROOT_NPM_PACKAGES)
        return artifacts
    artifacts = []
    for row in platform_targets(platform):
        archive = archive_name(tag, row)
        wheel_tag = f"{WHEEL_INTERPRETER_TAGS}-{row.wheel_platform_tag}"
        artifacts.append(Artifact(platform, "github", "archive", archive, target=row.target))
        artifacts.append(Artifact(platform, "github", "checksum", f"{archive}.sha256", target=row.target))
        artifacts.append(Artifact(platform, "pypi", "wheel", f"onetaskgraph_cli-{version}-{wheel_tag}.whl", PackageName(CLI_DISTRIBUTION), ReleaseTargetId(f"pypi:{CLI_DISTRIBUTION}"), row.target, (wheel_tag,)))
        package = PackageName(f"@onetaskgraph/cli-{row.npm_platform}")
        artifacts.append(Artifact(platform, "npm", "npm-platform", f"onetaskgraph-cli-{row.npm_platform}-{version}.tgz", package, ReleaseTargetId(f"npm:{package}"), row.target))
    return artifacts


def staged_path(dist: Path, artifact: Artifact) -> Path:
    """Where the build stages one artifact, and where its publication reads it from."""
    match artifact.kind:
        case "archive" | "checksum":
            return dist / "assets" / artifact.file
        case "wheel" if artifact.platform != "independent":
            return dist / "wheels" / str(artifact.target) / artifact.file
        case "npm-platform":
            npm_platform = next(row.npm_platform for row in PLATFORM_TARGETS if row.target == artifact.target)
            return dist / "carriers" / npm_platform / artifact.file
        case "wheel" | "sdist":
            return dist / "python-sdk" / artifact.file
        case _:
            return dist / "independent" / artifact.file


def source_digest(artifact: Artifact) -> str:
    """The digest of what cargo or npm packs an unstaged artifact from at publication.

    A crate is packed from its crate directory and the workspace's manifest and lock; a root
    npm package from its package directory, the TypeScript SDK's from what the build made.
    Recording it at build time is what lets a publication refuse a tree that moved since.
    """
    match artifact.registry, artifact.package:
        case "crates", crate:
            # README.md is the one file outside a crate's directory a crate packs (`readme`).
            roots = [ROOT / "crates" / str(crate), ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "README.md"]
        case "npm", "@onetaskgraph/cli":
            roots = [ROOT / "npm" / "cli"]
        case "npm", _:
            sdk = ROOT / "sdks" / "typescript"
            roots = [sdk / "package.json", sdk / "README.md", sdk / "dist"]
        case _:
            raise ValueError(f"{artifact.file} is staged by the build, not packed at publication")
    digest = hashlib.sha256()
    for root in roots:
        for path in sorted([root] if root.is_file() else (p for p in root.rglob("*") if p.is_file() and "target" not in p.relative_to(root).parts)):
            digest.update(path.relative_to(ROOT).as_posix().encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def artifact_digest(dist: Path, artifact: Artifact) -> str | None:
    if artifact.staged:
        path = staged_path(dist, artifact)
        return sha256(path) if path.is_file() else None
    return source_digest(artifact)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(command: list[str], what: str, next_action: str, env: dict[str, str] | None = None, cwd: Path = ROOT, capture: bool = False) -> str:
    """Run one release command; its own output goes to stderr unless it is captured."""
    try:
        completed = subprocess.run(command, cwd=cwd, env=env, check=False, text=True, stdout=subprocess.PIPE if capture else sys.stderr, stderr=subprocess.PIPE if capture else None)
    except OSError as error:
        fail(f"{what}: could not start {command[0]}: {error}", next_action)
    if completed.returncode != 0:
        if capture:
            sys.stderr.write(completed.stdout or "")
            sys.stderr.write(completed.stderr or "")
        fail(f"{what} (exit {completed.returncode})", next_action)
    return completed.stdout or ""


def tool(name: str) -> str:
    """An absolute path to a tool on PATH, so Windows resolves npm.cmd and uv.exe as well."""
    found = shutil.which(name)
    if found is None:
        fail(f"{name} is not on PATH, and this release step needs it", f"install {name} (the release workflow's setup steps do), then rerun")
    return found


def require_tree_version(tag: ReleaseTag) -> None:
    versions = read_reconciled_versions()
    wrong = sorted(path for path, value in versions.items() if value != version_of(tag))
    if wrong:
        fail(f"the tree is not at the release version {version_of(tag)}: {', '.join(wrong)} disagree", f"check out the commit {tag} was cut from, or run scripts/set-version.sh {version_of(tag)} on a release branch")



def maturin_requirement() -> str:
    """The maturin the release builds with: pyproject.toml's own build requirement."""
    import tomllib

    system = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8")).get("build-system")
    requirements = system.get("requires") if isinstance(system, dict) else None
    if not isinstance(requirements, list) or not all(isinstance(requirement, str) for requirement in requirements):
        fail("pyproject.toml's [build-system] has no list of requirements", "restore `requires = [\"maturin>=…\"]` under [build-system]")
    selected = [requirement for requirement in requirements if requirement.startswith("maturin")]
    if len(selected) != 1:
        fail("pyproject.toml's build-system names no single maturin requirement", "restore `requires = [\"maturin>=…\"]` under [build-system]")
    return selected[0]


def build_target(row: PlatformTarget, tag: ReleaseTag, dist: Path) -> None:
    wheels = dist / "wheels" / row.target
    shutil.rmtree(wheels, ignore_errors=True)
    wheels.mkdir(parents=True)
    run(
        [tool("uv"), "tool", "run", "--from", maturin_requirement(), "maturin", "build", "--release", "--locked", "--target", row.target, "--out", str(wheels)],
        f"maturin could not build the {row.target} wheel",
        "fix the build error above; the wheel's binary is the one every other artifact carries",
        env={**os.environ, "RUSTFLAGS": "-D warnings"},
    )
    package_target(row, tag, dist)
    run(
        [PYTHON_EXECUTABLE, "scripts/verify-release-products.py", "--target", row.target, "--npm", row.npm_platform, "--tag", tag, "--ext", row.extension, "--dist", str(dist), "--install"],
        f"the {row.target} artifacts did not install and run",
        "nothing was published; fix the artifact the verification named above and rerun the build",
    )


def package_target(row: PlatformTarget, tag: ReleaseTag, dist: Path) -> None:
    """Package the archive, its checksum and the npm platform package from the wheel's binary.

    The half of a build after maturin, and the half that is the same on every host: the
    guards drive it over stand-in wheels for targets this machine cannot build.
    """
    version = version_of(tag)
    expected = {artifact.kind: artifact for artifact in plan(row.platform, tag) if artifact.target == row.target}
    wheels = dist / "wheels" / row.target
    built = sorted(path.name for path in wheels.glob("*.whl"))
    if built != [expected["wheel"].file]:
        fail(f"maturin built {built or 'no wheel'} for {row.target}, where the release publishes {expected['wheel'].file}", "a wheel tag or glibc floor moved: restore the runner the platform table in scripts/release_products.py names, or change that table deliberately")
    carriers = dist / "carriers" / row.npm_platform
    shutil.rmtree(carriers, ignore_errors=True)
    carriers.mkdir(parents=True)
    (dist / "assets").mkdir(parents=True, exist_ok=True)
    binary = "onetaskgraph.exe" if row.extension == "zip" else "onetaskgraph"
    canonical = ROOT / "target" / row.target / "release" / binary
    run([PYTHON_EXECUTABLE, "scripts/release-wheel-binary.py", "--wheels", str(wheels), "--binary", str(canonical.relative_to(ROOT))], f"could not take the {row.target} binary out of its wheel", "rebuild the wheel and rerun")
    archive = dist / "assets" / expected["archive"].file
    if row.extension == "zip":
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as packed:
            packed.write(canonical, binary)
    else:
        # gzip's own default level, which is what `tar -czf` packed releases with before.
        with tarfile.open(archive, "w:gz", compresslevel=6) as packed:
            packed.add(canonical, binary, filter=_plain_member)
    (dist / "assets" / expected["checksum"].file).write_text(f"{sha256(archive)}  {archive.name}\n", encoding="utf-8", newline="\n")
    with tempfile.TemporaryDirectory(prefix="release-carrier-") as scratch:
        package = Path(scratch) / row.npm_platform
        (package / "bin").mkdir(parents=True)
        shutil.copy2(ROOT / "npm" / "platforms" / row.npm_platform / "package.json", package / "package.json")
        shutil.copy2(canonical, package / "bin" / binary)
        run([tool("npm"), "pack", str(package), "--pack-destination", str(carriers)], f"npm pack refused the {row.npm_platform} platform package", "fix what npm reported above and rerun", capture=True)
    packed_files = sorted(path.name for path in carriers.glob("*.tgz"))
    if packed_files != [expected["npm-platform"].file]:
        fail(f"npm packed {packed_files} for {row.npm_platform}, where the release publishes {expected['npm-platform'].file}", f"restore npm/platforms/{row.npm_platform}/package.json's name and version {version}")


def _plain_member(info: tarfile.TarInfo) -> tarfile.TarInfo:
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mode = 0o755
    return info


def manifest_path(dist: Path, platform: PlanPlatform) -> Path:
    return dist / f"release-{platform}.json"


@dataclass(frozen=True)
class StagedFile:
    file: str
    sha256: str | None


@dataclass(frozen=True)
class BuildManifest:
    """What one build staged, written by the build and read by every publication after it."""

    tag: ReleaseTag
    platform: PlanPlatform
    files: tuple[StagedFile, ...]

    @classmethod
    def parse(cls, value: object) -> "BuildManifest":
        if not isinstance(value, dict) or not isinstance(value.get("tag"), str) or value.get("platform") not in (*BUILD_PLATFORMS, "independent") or not isinstance(value.get("artifacts"), list):
            raise ValueError("not a build manifest: it needs a tag, a platform and a list of artifacts")
        files = []
        for entry in value["artifacts"]:
            digest = entry.get("sha256") if isinstance(entry, dict) else None
            if not isinstance(entry, dict) or not isinstance(entry.get("file"), str) or not (digest is None or (isinstance(digest, str) and len(digest) == 64 and all(c in "0123456789abcdef" for c in digest))):
                raise ValueError(f"an artifact entry is not a file and an optional SHA-256: {entry!r:.200}")
            files.append(StagedFile(entry["file"], digest))
        return cls(validate_tag(value["tag"]), value["platform"], tuple(files))


def write_manifest(dist: Path, platform: PlanPlatform, tag: ReleaseTag) -> None:
    entries = []
    for artifact in plan(platform, tag):
        digest = artifact_digest(dist, artifact)
        if digest is None:
            fail(f"the build left no {staged_path(dist, artifact).relative_to(dist)}", "rerun the build; the release publishes exactly the files it stages")
        entries.append({**artifact.record(), "sha256": digest})
    manifest_path(dist, platform).write_text(json.dumps({"tag": tag, "platform": platform, "artifacts": entries}, indent=2) + "\n", encoding="utf-8")


def build(platform: PlanPlatform, tag: ReleaseTag, dist: Path, announce: bool = True) -> None:
    require_tree_version(tag)
    dist.mkdir(parents=True, exist_ok=True)
    manifest_path(dist, platform).unlink(missing_ok=True)
    if platform == "independent":
        build_independent(tag, dist)
    else:
        for row in platform_targets(platform):
            build_target(row, tag, dist)
    write_manifest(dist, platform, tag)
    if announce:
        say(f"built {platform} for {tag}: {', '.join(artifact.file for artifact in plan(platform, tag))}")


def build_independent(tag: ReleaseTag, dist: Path) -> None:
    sdk = dist / "python-sdk"
    shutil.rmtree(sdk, ignore_errors=True)
    run([tool("uv"), "build", "sdks/python", "--out-dir", str(sdk)], "uv could not build the Python SDK", "fix the build error above and rerun")
    # The TypeScript SDK publishes what it built — its manifest names dist/ — so it is built
    # here, in the job that publishes it, through its own Nx target. One Nx command is all a
    # release runs, so no daemon is started for it: one left by another checkout on the same
    # host has stalled this step indefinitely.
    run(
        [bash_executable(host_platform.system()), "scripts/nx.sh", "run", "sdk-typescript:build"],
        "the TypeScript SDK did not build",
        "fix the build error above and rerun",
        env={**os.environ, "NX_DAEMON": "false"},
    )



def load_manifest(dist: Path, platform: PlanPlatform, tag: ReleaseTag) -> list[Artifact]:
    path = manifest_path(dist, platform)
    rebuild = f"run `just release-{platform if platform != 'independent' else 'publish'} build --tag {tag}`; a publication sends only what a build produced and recorded"
    try:
        recorded = BuildManifest.parse(json.loads(path.read_text(encoding="utf-8")))
    except (OSError, ValueError) as error:
        fail(f"no usable build of {platform} to publish ({path}: {error})", rebuild)
    if recorded.tag != tag or recorded.platform != platform:
        fail(f"{path} records a {recorded.platform} build of {recorded.tag}, not a {platform} build of {tag}", rebuild)
    artifacts = plan(platform, tag)
    listed = [entry.file for entry in recorded.files]
    if sorted(listed) != sorted(artifact.file for artifact in artifacts):
        fail(f"{path} does not list exactly the files {platform} releases, each once", rebuild)
    by_file = {entry.file: entry for entry in recorded.files}
    for artifact in artifacts:
        digest = by_file[artifact.file].sha256
        if digest is None or artifact_digest(dist, artifact) != digest:
            what = staged_path(dist, artifact) if artifact.staged else f"the source {artifact.file} is packed from"
            fail(f"{what} is not what the build produced", rebuild)
    return artifacts


def publishable(platform: PlanPlatform, tag: ReleaseTag, dist: Path, registry: Registry) -> list[Artifact]:
    """What this build hands `registry`, each file held to the manifest the build wrote."""
    artifacts = [artifact for artifact in load_manifest(dist, platform, tag) if artifact.registry == registry]
    if not artifacts:
        fail(f"{platform} publishes nothing to {registry}", f"choose one of {', '.join(sorted({a.registry for a in plan(platform, tag)}))}")
    return artifacts


def would_publish(artifacts: list[Artifact], registry: Registry) -> str:
    return f"would publish {', '.join(artifact.file for artifact in artifacts)} to {registry}"


def publish(platform: PlanPlatform, tag: ReleaseTag, dist: Path, registry: Registry, dry_run: bool) -> None:
    artifacts = publishable(platform, tag, dist, registry)
    if dry_run:
        say(f"dry run: {would_publish(artifacts, registry)}")
        return
    match registry:
        case "github":
            publish_github(tag, [staged_path(dist, artifact) for artifact in artifacts])
        case "pypi":
            publish_pypi([staged_path(dist, artifact) for artifact in artifacts])
        case "npm":
            publish_npm(platform, artifacts, dist)
        case "crates":
            publish_crates(tag)


class SameOriginRedirects(urllib.request.HTTPRedirectHandler):
    """Follow a redirect only within the origin a token was sent to.

    urllib copies every header but the body's onto the redirected request, Authorization
    included, so a redirect to another host would hand it the token. Such a redirect is
    refused instead, as the HTTP error it arrived as.
    """

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        before, after = urllib.parse.urlsplit(req.full_url), urllib.parse.urlsplit(newurl)
        if (before.scheme, before.netloc) != (after.scheme, after.netloc):
            raise urllib.error.HTTPError(req.full_url, code, f"refusing to follow a redirect to {after.scheme}://{after.netloc} with credentials", headers, fp)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


AUTHENTICATED = urllib.request.build_opener(SameOriginRedirects)


def is_count(value: object, minimum: int) -> bool:
    """An integer GitHub sends as an id or a size, at least `minimum`; JSON's booleans are not one."""
    return isinstance(value, int) and not isinstance(value, bool) and value >= minimum


@dataclass(frozen=True)
class ReleaseAsset:
    id: AssetId
    name: str
    size: int
    uploaded: bool

    @classmethod
    def parse(cls, value: object) -> "ReleaseAsset":
        if not isinstance(value, dict) or not is_count(value.get("id"), minimum=1) or not isinstance(value.get("name"), str) or not is_count(value.get("size"), minimum=0):
            raise ValueError(f"not a release asset: {value!r:.200}")
        return cls(AssetId(value["id"]), value["name"], value["size"], value.get("state") == "uploaded")


@dataclass(frozen=True)
class Release:
    upload_url: str
    assets: tuple[ReleaseAsset, ...]

    @classmethod
    def parse(cls, value: object, tag: ReleaseTag) -> "Release":
        """GitHub's release-by-tag answer, as config/github-interfaces.toml pins it."""
        if not isinstance(value, dict) or not isinstance(value.get("upload_url"), str) or not isinstance(value.get("assets"), list):
            raise ValueError("it has no upload_url and assets list")
        if value.get("tag_name") != tag:
            raise ValueError(f"it is the release of {value.get('tag_name')!r}")
        return cls(value["upload_url"].split("{", 1)[0], tuple(ReleaseAsset.parse(asset) for asset in value["assets"]))


@dataclass(frozen=True)
class GitHub:
    api: str
    repository: RepositoryName
    token: Token

    @classmethod
    def from_environment(cls) -> "GitHub":
        token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN") or ""
        repository = os.environ.get("GITHUB_REPOSITORY", "")
        if not token:
            fail("no GitHub token: GH_TOKEN and GITHUB_TOKEN are both empty", "set GH_TOKEN from secrets.GITHUB_TOKEN, as the release workflow does")
        if not REPOSITORY_NAME.fullmatch(repository):
            fail(f"GITHUB_REPOSITORY is {repository!r}, not owner/name", "run this in GitHub Actions, or set GITHUB_REPOSITORY")
        return cls(endpoint("GITHUB_API_URL", DEFAULT_GITHUB_API).rstrip("/"), RepositoryName(repository), Token(token))

    def trusts(self, url: str) -> bool:
        """Whether a URL GitHub handed back may be sent this token: the API's own host, or its upload host.

        It is held to what `endpoint` holds an address to as well, because a space or control
        character would escape as an uncaught InvalidURL, and a query of its own would break
        the `?name=` an upload appends.
        """
        api, given = urllib.parse.urlsplit(self.api), urllib.parse.urlsplit(url)
        hosts = {api.netloc, api.netloc.replace("api.", "uploads.", 1)}
        return given.scheme == api.scheme and given.netloc in hosts and not URL_DISALLOWED.search(url) and not given.query and not given.fragment

    def request(self, method: str, url: str, body: bytes | None = None, content_type: str | None = None) -> object:
        headers = {"Authorization": f"Bearer {self.token}", "Accept": "application/vnd.github+json", "User-Agent": USER_AGENT, "X-GitHub-Api-Version": "2022-11-28"}
        if content_type:
            headers["Content-Type"] = content_type
        request = urllib.request.Request(url, data=body, method=method, headers=headers)
        try:
            with AUTHENTICATED.open(request, timeout=300) as response:
                payload = response.read()
        except urllib.error.HTTPError as error:
            fail(f"GitHub answered {method} {url} with HTTP {error.code}: {error.msg}: {(error.read() if error.fp else b'')[:500].decode('utf-8', 'replace')}", "rerun the step once GitHub answers; assets already uploaded are skipped")
        except (urllib.error.URLError, http.client.InvalidURL, TimeoutError, OSError) as error:
            fail(f"could not reach GitHub for {method} {url}: {error}", "rerun the step once GitHub is reachable; assets already uploaded are skipped")
        try:
            return json.loads(payload) if payload else None
        except json.JSONDecodeError as error:
            fail(f"GitHub answered {method} {url} with something that is not JSON: {error}", "rerun the step once GitHub answers")

    def release(self, tag: ReleaseTag) -> Release:
        found = self.request("GET", f"{self.api}/repos/{self.repository}/releases/tags/{urllib.parse.quote(tag)}")
        try:
            release = Release.parse(found, tag)
        except ValueError as error:
            fail(f"GitHub's answer for the {tag} release is not a release: {error}", "check that the release-plz job cut the GitHub Release for this tag")
        if not self.trusts(release.upload_url):
            fail(f"the {tag} release names {release.upload_url!r} to upload to, which is not a plain URL on GitHub's API or upload host", "check GITHUB_API_URL; the token is sent nowhere else")
        return release


def publish_github(tag: ReleaseTag, files: list[Path]) -> None:
    github = GitHub.from_environment()
    release = github.release(tag)
    assets = {asset.name: asset for asset in release.assets}
    sent, kept = [], []
    for path in files:
        existing = assets.get(path.name)
        if existing is not None and existing.uploaded and existing.size == path.stat().st_size:
            kept.append(path.name)
            continue
        if existing is not None:
            # A partial asset from an interrupted upload holds the name; take it away first.
            github.request("DELETE", f"{github.api}/repos/{github.repository}/releases/assets/{existing.id}")
        answer = github.request("POST", f"{release.upload_url}?name={urllib.parse.quote(path.name)}", path.read_bytes(), "application/octet-stream")
        try:
            landed = ReleaseAsset.parse(answer)
        except ValueError as error:
            fail(f"GitHub answered the upload of {path.name} with {error}", "re-run this step: a re-run takes back whatever asset the upload left and sends it again")
        if landed.name != path.name or landed.size != path.stat().st_size or not landed.uploaded:
            fail(f"GitHub answered the upload of {path.name} ({path.stat().st_size} bytes) with {landed.name} ({landed.size} bytes, {'uploaded' if landed.uploaded else 'not uploaded'})", "re-run this step: a re-run takes back whatever asset the upload left and sends it again")
        sent.append(path.name)
    say(f"the {tag} release holds {len(files)} file(s) of this build: uploaded {', '.join(sent) or 'none'}; already there {', '.join(kept) or 'none'}")


def pypi_simple_base() -> str:
    return endpoint("ONETASKGRAPH_PYPI_SIMPLE_URL", DEFAULT_PYPI_SIMPLE).rstrip("/") + "/"


def publish_pypi(files: list[Path]) -> None:
    if not os.environ.get("UV_PUBLISH_TOKEN"):
        fail("PYPI_TOKEN is required: UV_PUBLISH_TOKEN is empty", "set UV_PUBLISH_TOKEN from the PYPI_TOKEN repository secret — gh-secrets.json declares it")
    upload = endpoint("ONETASKGRAPH_PYPI_UPLOAD_URL", DEFAULT_PYPI_UPLOAD)
    # --check-url skips a file the index already holds with these bytes, so a re-run after a
    # failed upload sends only what is missing. It names the index root: uv appends the
    # project, so a project page here is a page nobody serves and nothing is ever skipped.
    run(
        [tool("uv"), "publish", "--publish-url", upload, "--check-url", pypi_simple_base(), *map(str, files)],
        f"uv could not publish {', '.join(path.name for path in files)}",
        "fix what PyPI reported above and rerun; files already there are skipped",
    )


def npm_registry() -> str:
    return endpoint("NPM_REGISTRY", DEFAULT_NPM_REGISTRY)


def publish_npm(platform: PlanPlatform, artifacts: list[Artifact], dist: Path) -> None:
    bash = bash_executable(host_platform.system())
    if platform == "independent":
        run([bash, "scripts/publish-npm.sh"], "the npm launcher and TypeScript SDK were not published", "fix what scripts/publish-npm.sh reported above and rerun", env={**os.environ, "NPM_PUBLISH_SET": "root"})
        return
    for artifact in artifacts:
        directory = staged_path(dist, artifact).parent
        run(
            [bash, "scripts/publish-npm.sh"],
            f"{artifact.package} was not published",
            "fix what scripts/publish-npm.sh reported above and rerun; a package already at this version is left alone",
            env={**os.environ, "NPM_PUBLISH_SET": "carriers", "NPM_PLATFORMS": directory.name, "NPM_CARRIERS": str(directory)},
        )


def publish_crates(tag: ReleaseTag) -> None:
    alternate = os.environ.get("ONETASKGRAPH_CARGO_REGISTRY", "")
    if not alternate and not os.environ.get("CARGO_REGISTRY_TOKEN"):
        fail("CARGO_REGISTRY_TOKEN is required", "set it from the CARGO_REGISTRY_TOKEN repository secret")
    bash = bash_executable(host_platform.system())
    kept = []
    for crate in CRATE_PUBLICATION_ORDER:
        status = run([bash, "scripts/crate-publication-status.sh", crate, version_of(tag)], f"could not learn whether {crate} {version_of(tag)} is on the registry", "rerun once the registry answers", capture=True).strip()
        if status == "published":
            kept.append(crate)
            continue
        if status != "absent":
            fail(f"unusable registry answer for {crate} {version_of(tag)}: {status}", "rerun once the registry answers 200 or 404")
        run([bash, "scripts/crate-sibling-resolution.sh", crate], f"{crate}'s siblings do not resolve on the registry", "publish the siblings it names first, then rerun")
        # Packaging verification builds every crate again under these RUSTFLAGS, a different
        # unit from the one onetaskgraph:build links, so in the clone's shared target it would
        # relink the debug binary while the e2e suites are spawning it. A directory of
        # its own, still inside that target, keeps the verification and leaves that file alone.
        command = [tool("cargo"), "publish", "-p", crate, "--locked", "--target-dir", str(ROOT / "target" / "crate-publication")]
        if alternate:
            command += ["--registry", alternate]
        run(command, f"cargo could not publish {crate}", "fix what cargo reported above and rerun; published crates are skipped", env={**os.environ, "RUSTFLAGS": "-D warnings"})
    if kept:
        say(f"already on the registry, left alone: {', '.join(kept)}")



def retrying(what: str, wait: float, attempt) -> None:
    """Retry one registry read until it succeeds or `wait` seconds pass.

    A registry serves a version a little after accepting it — PyPI's index and npm's
    packuments sit behind caches — so a read straight after publication is retried rather
    than read as the package being absent. With `wait` 0 it is asked exactly once.
    """
    deadline = time.monotonic() + wait
    while True:
        try:
            attempt()
            return
        except ReleaseStepFailed as error:
            if time.monotonic() >= deadline:
                raise
            say(f"{what}: {error.problem}; asking again in 20 s")
            time.sleep(min(20.0, max(0.0, deadline - time.monotonic())))


def run_installed(executable: Path, version: str, where: str) -> None:
    try:
        output = subprocess.run([str(executable), "--version"], capture_output=True, text=True, timeout=60, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        fail(f"the onetaskgraph installed from {where} did not run: {error}", "the published package is unusable; publish a fixed release")
    if output.returncode != 0 or output.stdout.strip() != f"onetaskgraph {version}":
        fail(f"the onetaskgraph installed from {where} answered {output.stdout.strip() or output.stderr.strip()!r}, not 'onetaskgraph {version}'", "the registry is not serving this release's binary; check what it holds for this version")


def host_target() -> PlatformTarget:
    system = host_platform.system()
    machine = host_platform.machine().lower()
    arch = "arm64" if machine in ("arm64", "aarch64") else "x64" if machine in ("x86_64", "amd64") else machine
    npm_platform = {"Linux": "linux", "Darwin": "darwin", "Windows": "win32"}.get(system, system.lower()) + "-" + arch
    for row in PLATFORM_TARGETS:
        if row.npm_platform == npm_platform:
            return row
    fail(f"this host ({system} {machine}) is not a release platform", "verify a platform's release on that platform's runner")


def verify(platform: PlanPlatform, tag: ReleaseTag, registry: Registry, wait: float) -> None:
    version = version_of(tag)
    if platform == "independent":
        retrying(f"{registry} for the platform-independent packages", wait, lambda: resolve_independent(tag, registry))
        return
    row = host_target()
    if row.platform != platform:
        fail(f"the {platform} release cannot be installed on this {row.platform} host", f"run this in the build-{platform} job")
    with tempfile.TemporaryDirectory(prefix=f"release-verify-{registry}-") as scratch:
        directory = Path(scratch)
        match registry:
            case "github":
                retrying("the GitHub Release", wait, lambda: install_from_github(row, tag, directory))
            case "pypi":
                retrying("PyPI", wait, lambda: install_from_pypi(version, directory))
            case "npm":
                retrying("npm", wait, lambda: install_from_npm(row, version, directory))
            case _:
                fail(f"{platform} publishes nothing to {registry}", "choose github, pypi or npm")
    say(f"installed {tag} for {row.target} from {registry} and ran it")


def install_from_github(row: PlatformTarget, tag: ReleaseTag, directory: Path) -> None:
    target = directory / f"github-{time.monotonic_ns()}"
    binary = "onetaskgraph.exe" if row.extension == "zip" else "onetaskgraph"
    completed = subprocess.run([bash_executable(host_platform.system()), "scripts/install.sh", "--version", tag, "--to", str(target)], cwd=ROOT, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        fail(f"scripts/install.sh could not install {tag} from the GitHub Release: {completed.stderr.strip()}", "check the release's assets for this platform; the build job uploads them")
    run_installed(target / binary, version_of(tag), "the GitHub Release")


def install_from_pypi(version: str, directory: Path) -> None:
    environment = directory / f"venv-{time.monotonic_ns()}"
    run([PYTHON_EXECUTABLE, "-m", "venv", str(environment)], "could not create a virtual environment to install the wheel into", "check this runner's Python")
    python = environment / ("Scripts/python.exe" if host_platform.system() == "Windows" else "bin/python")
    completed = subprocess.run(
        [str(python), "-m", "pip", "install", "--no-deps", "--no-cache-dir", "--disable-pip-version-check", "--index-url", pypi_simple_base(), f"{CLI_DISTRIBUTION}=={version}"],
        capture_output=True, text=True, check=False,
    )
    if completed.returncode != 0:
        fail(f"pip could not install {CLI_DISTRIBUTION}=={version} for this platform: {completed.stderr.strip()[-800:]}", "check what the index serves for this version and platform; the build job publishes the wheel")
    run_installed(environment / ("Scripts/onetaskgraph.exe" if host_platform.system() == "Windows" else "bin/onetaskgraph"), version, "PyPI")


def npm_command(*arguments: str) -> list[str]:
    return [tool("npm"), *arguments, "--registry", npm_registry(), "--no-audit", "--no-fund"]


def install_from_npm(row: PlatformTarget, version: str, directory: Path) -> None:
    prefix = directory / f"npm-{time.monotonic_ns()}"
    package = f"@onetaskgraph/cli-{row.npm_platform}"
    completed = subprocess.run(npm_command("install", "--prefix", str(prefix), "--ignore-scripts", "--prefer-online", f"{package}@{version}"), capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        fail(f"npm could not install {package}@{version}: {completed.stderr.strip()[-800:]}", "check what the registry serves for this version; the build job publishes the package")
    installed = prefix / "node_modules" / package
    try:
        manifest = json.loads((installed / "package.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"npm installed {package} without a readable package.json: {error}", "check what the registry serves for this version")
    if not isinstance(manifest, dict) or manifest.get("version") != version:
        fail(f"npm installed {package} as {manifest!r:.200} for {version}", "check what the registry serves for this version")
    run_installed(installed / "bin" / ("onetaskgraph.exe" if row.extension == "zip" else "onetaskgraph"), version, "npm")


def fetch(url: str, what: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=300) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        fail(f"{what} is not available: HTTP {error.code} for {url}", "publish it from the build job that owns it, then rerun")
    except (urllib.error.URLError, http.client.InvalidURL, TimeoutError, OSError) as error:
        fail(f"could not read {what} from {url}: {error}", "rerun once the registry is reachable")


def available(tag: ReleaseTag, registry: Registry, wait: float) -> None:
    """Confirm one registry resolves the version to an installable artifact for every platform."""
    retrying(f"every platform on {registry}", wait, lambda: resolve_every_platform(tag, registry))
    say(f"{registry} serves {tag} for every platform")


def resolve_every_platform(tag: ReleaseTag, registry: Registry) -> None:
    version = version_of(tag)
    rows = [row for platform in BUILD_PLATFORMS for row in platform_targets(platform)]
    with tempfile.TemporaryDirectory(prefix="release-available-") as scratch:
        directory = Path(scratch)
        for row in rows:
            artifacts = {artifact.kind: artifact for artifact in plan(row.platform, tag) if artifact.target == row.target}
            match registry:
                case "github":
                    base = endpoint("ONETASKGRAPH_RELEASE_BASE_URL", RELEASE_DOWNLOADS).rstrip("/")
                    checksums = endpoint("ONETASKGRAPH_CHECKSUM_BASE_URL", RELEASE_DOWNLOADS).rstrip("/")
                    archive = fetch(f"{base}/{tag}/{artifacts['archive'].file}", f"the {row.target} archive")
                    recorded = fetch(f"{checksums}/{tag}/{artifacts['checksum'].file}", f"the {row.target} checksum").decode("utf-8", "replace").split()
                    if not recorded or recorded[0] != hashlib.sha256(archive).hexdigest():
                        fail(f"the {row.target} archive on the release does not match its checksum", "re-run that platform's build job, which re-uploads both")
                    _archive_holds_binary(archive, row)
                case "pypi":
                    wheel = artifacts["wheel"]
                    target = directory / row.target
                    completed = subprocess.run(
                        [PYTHON_EXECUTABLE, "-m", "pip", "download", "--no-deps", "--no-cache-dir", "--disable-pip-version-check", "--only-binary=:all:", "--platform", row.wheel_platform_tag, "--python-version", "3.12", "--index-url", pypi_simple_base(), "--dest", str(target), f"{CLI_DISTRIBUTION}=={version}"],
                        capture_output=True, text=True, check=False,
                    )
                    got = sorted(path.name for path in target.glob("*.whl")) if target.is_dir() else []
                    if completed.returncode != 0 or got != [wheel.file]:
                        fail(f"PyPI does not resolve {CLI_DISTRIBUTION}=={version} to {wheel.file} for {row.target}: {completed.stderr.strip()[-400:] or got}", f"re-run build-{row.platform}, which publishes that wheel")
                    _wheel_identity(target / wheel.file, version, row)
                case "npm":
                    resolve_npm_package(artifacts["npm-platform"].package or "", version, directory, row)
                case _:
                    fail(f"platforms publish nothing to {registry}", "choose github, pypi or npm")


def binary_name(row: PlatformTarget) -> str:
    return "onetaskgraph.exe" if row.extension == "zip" else "onetaskgraph"


def _archive_holds_binary(archive: bytes, row: PlatformTarget) -> None:
    """The archive as served is what scripts/install.sh extracts: one regular file, the binary, at its root."""
    try:
        if row.extension == "zip":
            with zipfile.ZipFile(io.BytesIO(archive)) as packed:
                members = [(entry.filename, not entry.is_dir() and ((entry.external_attr >> 16) & 0o170000) in (0, 0o100000)) for entry in packed.infolist()]
        else:
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as packed:
                members = [(entry.name, entry.isfile()) for entry in packed.getmembers()]
    except (zipfile.BadZipFile, tarfile.TarError, OSError) as error:
        fail(f"the {row.target} archive on the release is not a readable archive: {error}", f"re-run build-{row.platform}, which re-uploads it")
    if members != [(binary_name(row), True)]:
        fail(f"the {row.target} archive on the release holds {members}, not the one regular file {binary_name(row)}", f"re-run build-{row.platform}, which re-uploads it")


def _wheel_identity(path: Path, version: str, row: PlatformTarget) -> None:
    """The wheel as served: one identity, this release's, tagged for this platform, carrying its binary."""
    planned = next(artifact for artifact in plan(row.platform, ReleaseTag(f"v{version}")) if artifact.kind == "wheel" and artifact.target == row.target)
    try:
        with zipfile.ZipFile(path) as packed:
            entries = packed.infolist()
            read = lambda suffix: [packed.read(entry).decode("utf-8", "replace") for entry in entries if entry.filename.endswith(suffix)]
            metadata, wheel = read(".dist-info/METADATA"), read(".dist-info/WHEEL")
    except (zipfile.BadZipFile, OSError) as error:
        fail(f"{path.name} as served is not a readable wheel: {error}", "re-run the build job that owns it")
    if len(metadata) != 1 or len(wheel) != 1:
        fail(f"{path.name} as served has {len(metadata)} METADATA and {len(wheel)} WHEEL files, not one each", f"re-run build-{row.platform}, which publishes it")
    fields = email.parser.Parser().parsestr(metadata[0], headersonly=True)
    tags = [line.removeprefix("Tag: ") for line in wheel[0].splitlines() if line.startswith("Tag: ")]
    if fields.get_all("Name") != [CLI_DISTRIBUTION] or fields.get_all("Version") != [version]:
        fail(f"{path.name} as served is {fields.get_all('Name')} {fields.get_all('Version')}, not {CLI_DISTRIBUTION} {version}", "re-run the build job that owns it")
    if tags != list(planned.wheel_tags):
        fail(f"{path.name} as served is tagged {tags}, not {list(planned.wheel_tags)}", f"re-run build-{row.platform}, which publishes it")
    binaries = [entry for entry in entries if entry.filename.endswith(f".data/scripts/{binary_name(row)}")]
    if len(binaries) != 1 or binaries[0].is_dir() or ((binaries[0].external_attr >> 16) & 0o170000) not in (0, 0o100000):
        fail(f"{path.name} as served carries no {binary_name(row)} it could install", f"re-run build-{row.platform}, which publishes it")


def resolve_npm_package(package: str, version: str, directory: Path, row: PlatformTarget | None = None) -> None:
    destination = directory / f"pack-{time.monotonic_ns()}"
    destination.mkdir()
    completed = subprocess.run(npm_command("pack", f"{package}@{version}", "--pack-destination", str(destination)), capture_output=True, text=True, check=False, cwd=destination)
    tarballs = list(destination.glob("*.tgz"))
    if completed.returncode != 0 or len(tarballs) != 1:
        fail(f"npm does not resolve {package}@{version}: {completed.stderr.strip()[-400:]}", "re-run the job that publishes it")
    try:
        with tarfile.open(tarballs[0]) as packed:
            member = packed.extractfile("package/package.json")
            manifest = json.loads(member.read()) if member else {}
            binaries = [entry for entry in packed.getmembers() if row is not None and entry.name == f"package/bin/{binary_name(row)}"]
    except (tarfile.TarError, KeyError, json.JSONDecodeError) as error:
        fail(f"{package}@{version} as served is not a readable package: {error}", "re-run the job that publishes it")
    if not isinstance(manifest, dict) or manifest.get("name") != package or manifest.get("version") != version:
        fail(f"npm serves {manifest!r:.200} for {package}@{version}", "re-run the job that publishes it")
    if row is not None and (len(binaries) != 1 or not binaries[0].isfile()):
        fail(f"{package}@{version} as served carries no binary", f"re-run build-{row.platform}, which publishes it")


def sdk_minimum_python() -> str:
    manifest = Path("sdks/python/pyproject.toml")
    try:
        declared = re.search(r'^requires-python = ">=([0-9]+\.[0-9]+)"$', manifest.read_text(encoding="utf-8"), re.MULTILINE)
    except OSError as error:
        fail(f"could not read {manifest.as_posix()}: {error}", "run the release from a checkout of this repository")
    if declared is None:
        fail(f"{manifest.as_posix()} declares no requires-python of the form '>=X.Y'", "restore that floor, which the SDK's PyPI check resolves for")
    return declared.group(1)


def resolve_independent(tag: ReleaseTag, registry: Registry) -> None:
    version = version_of(tag)
    bash = bash_executable(host_platform.system())
    match registry:
        case "crates":
            for crate in CRATE_PUBLICATION_ORDER:
                status = run([bash, "scripts/crate-publication-status.sh", crate, version], f"could not ask the registry for {crate} {version}", "rerun once the registry answers", capture=True).strip()
                if status != "published":
                    fail(f"{crate} {version} is not on the registry ({status})", "rerun the publish job's crate step")
        case "pypi":
            with tempfile.TemporaryDirectory(prefix="release-sdk-") as scratch:
                # Resolved for the oldest python the SDK declares rather than for this runner's
                # own, which the SDK may not support: ubuntu-latest's python3 is older than it.
                completed = subprocess.run([PYTHON_EXECUTABLE, "-m", "pip", "download", "--no-deps", "--no-cache-dir", "--disable-pip-version-check", "--only-binary=:all:", "--python-version", sdk_minimum_python(), "--index-url", pypi_simple_base(), "--dest", scratch, f"{SDK_DISTRIBUTION}=={version}"], capture_output=True, text=True, check=False)
                if completed.returncode != 0:
                    fail(f"PyPI does not resolve {SDK_DISTRIBUTION}=={version}: {completed.stderr.strip()[-400:]}", "rerun the publish job's PyPI step")
        case "npm":
            with tempfile.TemporaryDirectory(prefix="release-root-npm-") as scratch:
                for package in ROOT_NPM_PACKAGES:
                    resolve_npm_package(package.name, version, Path(scratch))
        case _:
            fail(f"the publish job publishes nothing to {registry}", "choose crates, pypi or npm")



def seconds(value: str) -> float:
    try:
        parsed = float(value)
    except ValueError:
        parsed = math.nan
    if not math.isfinite(parsed) or parsed < 0:
        raise argparse.ArgumentTypeError(f"{value!r} is not a finite, non-negative number of seconds")
    return parsed


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--platform", required=True, choices=[*BUILD_PLATFORMS, "independent"], help="the CD job's platform; `independent` is the publish job")
    commands = parser.add_subparsers(dest="command", required=True)

    def command(name: str, help_text: str) -> argparse.ArgumentParser:
        sub = commands.add_parser(name, help=help_text)
        sub.add_argument("--tag", required=True, help="the release tag, vX.Y.Z")
        sub.add_argument("--dist", type=Path, default=ROOT / "dist", help="the staging directory (default: dist/)")
        return sub

    command("plan", "print, as JSON, every artifact this platform releases, without building")
    command("build", "build, package and test-install this platform's artifacts")
    command("package", "package the wheels already in <dist>/wheels, without building or installing: the guards' entry to the packaging every build runs")
    publish_parser = command("publish", "publish the built artifacts to one registry")
    publish_parser.add_argument("--to", required=True, choices=["github", "pypi", "npm", "crates"])
    publish_parser.add_argument("--dry-run", action="store_true", help="name what would be published and send nothing")
    for name, help_text in (("verify", "install the release from one registry on this OS and run it"), ("available", "confirm one registry serves the release for every platform")):
        sub = command(name, help_text)
        sub.add_argument("--from", dest="registry", required=True, choices=["github", "pypi", "npm", "crates"])
        sub.add_argument("--wait", type=seconds, default=600.0, help="seconds to keep asking a registry that has not caught up (default 600)")
    command("run", "build, test-install, then publish to every registry in dry-run mode").add_argument("--dry-run", action="store_true", required=True)
    args = parser.parse_args(None if argv is None else list(argv))
    try:
        tag = validate_tag(args.tag)
    except ValueError as error:
        parser.error(str(error))
    dist = args.dist.resolve()
    try:
        match args.command:
            case "plan":
                print(json.dumps([artifact.record() for artifact in plan(args.platform, tag)], indent=2))
            case "build":
                build(args.platform, tag, dist)
            case "package":
                if args.platform == "independent":
                    parser.error("`package` packages a platform's wheels: --platform linux, macos or windows")
                for row in platform_targets(args.platform):
                    package_target(row, tag, dist)
            case "publish":
                registries = INDEPENDENT_REGISTRIES if args.platform == "independent" else PLATFORM_REGISTRIES
                if args.to not in registries:
                    parser.error(f"--platform {args.platform} publishes to {', '.join(registries)}, not {args.to}")
                publish(args.platform, tag, dist, args.to, args.dry_run)
            case "verify":
                verify(args.platform, tag, args.registry, args.wait)
            case "available":
                if args.platform != "independent" or args.registry == "crates":
                    parser.error("`available` is the publish job's check: --platform independent --from github|pypi|npm")
                available(tag, args.registry, args.wait)
            case "run":
                build(args.platform, tag, dist, announce=False)
                registries = INDEPENDENT_REGISTRIES if args.platform == "independent" else PLATFORM_REGISTRIES
                planned = "; ".join(would_publish(publishable(args.platform, tag, dist, registry), registry) for registry in registries)
                say(f"built {args.platform} for {tag}; dry run: {planned}")
    except ReleaseStepFailed as error:
        say(error.problem)
        say(f"next: {error.next_action}")
        return 1
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError, zipfile.BadZipFile) as error:
        say(f"{args.command} for {args.platform} stopped on an input it could not read: {type(error).__name__}: {error}")
        say(f"next: rerun the `build` phase, which rewrites everything under {dist}, then this one; if it recurs, the input named above is what to repair")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
