# llmlint: ignore-file[async_typed_clients_at_boundaries] This CLI compares local Git blobs and invokes a local parser, with no hosted API or shared service client; sequential subprocess execution keeps one conservative classification and matches the repository guard convention.
"""Compare release-owned values; all other parsed fields must remain identical."""
import json
from dataclasses import dataclass
import subprocess
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from product_versions import RECONCILED_VERSION_FILES, JsonVersionFile, RegexVersionFile, SEMANTIC_VERSION_RE, SemanticVersion

@dataclass(frozen=True)
class BunComparison:
    normalized: dict[str, object]
    versions: tuple[SemanticVersion, ...]

    @classmethod
    def parse(cls, text: str) -> 'BunComparison':
        value = json.loads(text)
        if not isinstance(value, dict) or set(value) != {'normalized', 'versions'} or not isinstance(value['normalized'], dict) or not isinstance(value['versions'], list):
            raise ValueError('invalid local Bun parser response')
        return cls(value['normalized'], tuple(validated_version(version) for version in value['versions']))

def validated_version(value: object, prefix: str = '') -> SemanticVersion:
    if not isinstance(value, str) or not value.startswith(prefix) or not SEMANTIC_VERSION_RE.fullmatch(value[len(prefix):]):
        raise ValueError('invalid product version; restore the field written by scripts/set-version.sh')
    return SemanticVersion(value[len(prefix):])


def version(value: object, prefix: str = '') -> str:
    validated_version(value, prefix)
    return prefix + '<version>'

def blob(ref, path):
    return subprocess.check_output(['git', 'show', f'{ref}:{path}'], text=True)

def normalize_lock(text, path, products):
    obj = tomllib.loads(text)
    packages = obj.get('package')
    if not isinstance(packages, list) or not packages or any(not isinstance(package, dict) for package in packages):
        raise ValueError('lockfile must contain a nonempty package array')
    for package in packages:
        name = package.get('name', '')
        local = name in products and ('source' not in package if path == 'Cargo.lock' else isinstance(package.get('source'), dict) and ('editable' in package['source'] or 'virtual' in package['source']))
        if local:
            package['version'] = version(package['version'])
        dependencies = package.get('dependencies', [])
        metadata = package.get('metadata', {})
        if not isinstance(dependencies, list) or not isinstance(metadata, dict):
            raise ValueError('invalid lockfile dependencies or metadata')
        requires = metadata.get('requires-dist', [])
        if not isinstance(requires, list) or any(not isinstance(dep, dict) for dep in requires):
            raise ValueError('invalid lockfile requires-dist records')
        if path == 'Cargo.lock' and any(not isinstance(dep, str) for dep in dependencies):
            raise ValueError('invalid Cargo dependency records')
        if path != 'Cargo.lock' and any(not isinstance(dep, dict) for dep in dependencies):
            raise ValueError('invalid uv dependency records')
        for dep in dependencies:
            if isinstance(dep, dict) and dep.get('name', '') in products and 'version' in dep:
                dep['version'] = version(dep['version'])
        for dep in requires:
            if dep.get('name', '') in products and 'specifier' in dep:
                dep['specifier'] = version(dep['specifier'], '==')
    return obj

def normalize(text, entry):
    if isinstance(entry, JsonVersionFile):
        obj = json.loads(text)
        for field in entry.fields:
            node = obj
            for key in field[:-1]:
                node = node[key]
            node[field[-1]] = version(node[field[-1]])
        return obj
    # Only the registered generated constants/manifest fields can be substituted.
    # Parsed TOML equality also permits formatting; source constants retain every other byte.
    for pattern in entry.patterns:
        text, count = pattern.subn(lambda m: m[1] + version(m[2]) + m[3], text)
        if not count:
            raise ValueError('missing registered version')
    if entry.path.suffix == '.toml':
        return tomllib.loads(text)
    return text

def main():
    base, head = [subprocess.check_output(['git', 'rev-parse', '--verify', '--end-of-options', ref + '^{commit}'], text=True).strip() for ref in sys.argv[1:]]
    entries = {str(entry.path): entry for entry in RECONCILED_VERSION_FILES}
    root = tomllib.loads(blob(base, 'Cargo.toml'))
    members = root['workspace']['members']
    paths = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', base], text=True).splitlines()
    import fnmatch
    if not isinstance(members, list) or any(not isinstance(member, str) or '**' in member for member in members):
        raise ValueError('unresolved workspace member patterns')
    def workspace_member(directory: str) -> bool:
        parts = directory.split('/')
        return any(len(parts) == len(member.split('/')) and all(fnmatch.fnmatchcase(part, pattern) for part, pattern in zip(parts, member.split('/'))) for member in members)
    crates = {p.removesuffix('/Cargo.toml') for p in paths if p.endswith('/Cargo.toml') and workspace_member(p.removesuffix('/Cargo.toml'))}
    products = {tomllib.loads(blob(base, p + '/Cargo.toml'))['package']['name'] for p in crates} | {tomllib.loads(blob(base, path))['project']['name'] for path in ('pyproject.toml', 'sdks/python/pyproject.toml')}
    changelogs = {'CHANGELOG.md', *(p + '/CHANGELOG.md' for p in crates)}
    changed = subprocess.check_output(['git', 'diff', '--name-only', '--no-renames', base, head], text=True).splitlines()
    if not changed:
        return False
    for path in changed:
        if path in changelogs:
            # Deletion is not a changelog update; a new member changelog is permitted.
            blob(head, path)
            continue
        before, after = blob(base, path), blob(head, path)
        match path:
            case 'Cargo.lock' | 'uv.lock' | 'sdks/python/uv.lock':
                left, right = normalize_lock(before, path, products), normalize_lock(after, path, products)
            case 'bun.lock':
                helper = str(Path(__file__).with_name('semantic-json.mjs'))
                comparisons = [BunComparison.parse(subprocess.check_output(['node', helper], input=text, text=True)) for text in (before, after)]
                # Drift gate: the helper may mask exactly the values the production
                # updater registers, and no field it does not own.
                for text, comparison in zip((before, after), comparisons):
                    masked = text
                    for pattern in entries[path].patterns:
                        masked = pattern.sub(lambda match: match[1] + '<version>' + match[3], masked)
                    registered = json.loads(subprocess.check_output(['node', helper, '--parse-only'], input=masked, text=True))
                    if registered != comparison.normalized:
                        raise ValueError('Bun masked fields disagree with product_versions.py; reconcile the release inventory and classifier')
                left, right = [comparison.normalized for comparison in comparisons]
            case registered if registered in entries:
                left, right = normalize(before, entries[path]), normalize(after, entries[path])
            case _:
                return False
        if left != right:
            return False
    return True

if __name__ == '__main__':
    try:
        print(json.dumps(main()))
    except (ValueError, KeyError, subprocess.CalledProcessError, TypeError) as error:
        print(f'unreadable release diff: {error}; next: restore the base/head refs and release-owned version fields', file=sys.stderr)
        sys.exit(1)
