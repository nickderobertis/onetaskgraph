#!/usr/bin/env bash
# Bound the scheduled cleanup's credentials and targets separately from the test lane.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || {
  echo "check-live-janitor: cannot enter the repository root from ${BASH_SOURCE[0]}" >&2
  echo "check-live-janitor: next: run it as 'bash scripts/check-live-janitor.sh' from a checkout" >&2
  exit 1
}
python3 - <<'PY'
import json
import re
import sys
import subprocess
from pathlib import Path

janitor = Path('.github/workflows/live-janitor.yml').as_posix()
TARGETS = f'restore the explicit targets and credential boundary in {janitor}, then rerun'
problems = []

def refuse(text, path=janitor, repair=TARGETS):
    problems.append((path, text, repair))

def unreadable(path, cause, repair):
    print(f'check-live-janitor: {path}: {cause}', file=sys.stderr)
    print(f'check-live-janitor: next: {repair}', file=sys.stderr)
    sys.exit(1)

def read(path):
    try:
        return Path(path).read_text(encoding='utf-8')
    except (OSError, UnicodeDecodeError) as error:
        unreadable(path, f'cannot be read ({error})', 'restore it from git, then rerun')

def git(*arguments):
    try:
        return subprocess.run(['git', *arguments], capture_output=True, text=True, encoding='utf-8')
    except OSError as error:
        unreadable('git', f'cannot be run ({error})', 'put git on PATH, then rerun')

def section(text, start, end, name):
    if start not in text:
        refuse(f'{name} block is missing')
        return ''
    return text.split(start, 1)[1].split(end, 1)[0]

source = read(janitor)
ci = read('.github/workflows/ci.yml')
declarations = read('crates/onetaskgraph-github-live/src/lib.rs')

def values(text, key):
    return [value.strip().strip('"\'') for value in re.findall(rf'^\s*{key}:\s*(.+)$', text, re.M)]

def constant(name, spelled=r'&str = "([^"]+)"'):
    found = re.search(rf'pub const {name}: {spelled};', declarations)
    return found.group(1) if found else None

for key in ('GH_PROJECTS_OWNER', 'GH_PROJECTS_NUMBER', 'GH_PROJECTS_REPOSITORY'):
    nominated = values(source, key)
    if len(nominated) != 1 or values(ci, key) != nominated * 2:
        refuse(f'{key} differs from the two credentialed ci.yml steps')
if values(source, 'GH_PROJECTS_NUMBER') != ['1']:
    refuse('GH_PROJECTS_NUMBER must name board 1 only')
if values(source, 'GH_PROJECTS_REPOSITORY') != [constant('SCRATCH_REPOSITORY')]:
    refuse('GH_PROJECTS_REPOSITORY differs from SCRATCH_REPOSITORY')
if values(source, 'GH_PROJECTS_LEGACY_REPOSITORY') != [constant('CORE_REPOSITORY')]:
    refuse('GH_PROJECTS_LEGACY_REPOSITORY differs from CORE_REPOSITORY')
if values(source, 'GH_PROJECTS_OWNER') != [constant('BOARD_OWNER')]:
    refuse('GH_PROJECTS_OWNER differs from BOARD_OWNER')
if values(source, 'GH_PROJECTS_NUMBER') != [constant('BOARD_NUMBER', r'u32 = ([0-9]+)')]:
    refuse('GH_PROJECTS_NUMBER differs from BOARD_NUMBER')
on_block = section(source, '\non:\n', '\npermissions:', 'on')
if re.findall(r'^  ([a-z_]+):', on_block, re.M) != ['schedule', 'workflow_dispatch']:
    refuse('only schedule and workflow_dispatch may trigger cleanup')
cron = values(on_block, '- cron')
if len(cron) != 1 or not re.fullmatch(r'(?:[0-9]|[1-5][0-9]) \* \* \* \*', cron[0]):
    refuse('schedule must fire exactly once every hour')
permissions = section(source, '\npermissions:\n', '\nconcurrency:', 'permissions')
if permissions.strip().splitlines() != ['actions: read', '  contents: read']:
    refuse('permissions must be actions: read and contents: read only')
for key, secret in [('GH_PROJECTS_TOKEN', 'GH_PROJECTS_TOKEN'), ('GITHUB_TOKEN', 'GITHUB_TOKEN')]:
    if values(source, key) != ['${{ secrets.' + secret + ' }}']:
        refuse(f'{key} must use secrets.{secret}')
if values(source, 'run') != ['cargo run --locked -p onetaskgraph-live-janitor']:
    refuse('run must invoke the real janitor entry point')
# No required check opens a janitor session. Offline tests invoke a loopback binary instead.
for path in [*Path('crates').glob('*/project.json'), *Path('sdks').glob('*/project.json'), Path('workspace/project.json'), Path('scripts/project.json')]:
    try:
        project = json.loads(read(path))
    except json.JSONDecodeError as error:
        unreadable(path, f'is not JSON ({error})', 'repair the project file, then rerun')
    targets = project.get('targets', {}) if isinstance(project, dict) else None
    if not isinstance(targets, dict):
        unreadable(path, 'has no targets object', 'repair the project file, then rerun')
    for name in ('test', 'coverage'):
        commands = json.dumps(targets.get(name, {}))
        if re.search(r'cargo\s+run\b.*onetaskgraph-live-janitor', commands):
            refuse(f'{name} opens a janitor session', path,
                   'remove that cargo run of onetaskgraph-live-janitor; the janitor runs only from its workflow')
# Check committed provenance once the finished constant is in HEAD. An uncommitted
# introduction has no author timestamp yet; committing it makes this check applicable.
cutover_path = 'crates/onetaskgraph-live-janitor/src/lib.rs'
listed = git('ls-tree', '--name-only', 'HEAD', '--', cutover_path)
if listed.returncode != 0:
    unreadable(cutover_path, f'HEAD cannot be read ({listed.stderr.strip()})', 'run this from a git checkout with at least one commit, then rerun')
committed_text = ''
if listed.stdout.strip():
    committed = git('show', f'HEAD:{cutover_path}')
    if committed.returncode != 0:
        unreadable(cutover_path, f'HEAD lists it but it cannot be read ({committed.stderr.strip()})', 'repair the clone, then rerun')
    committed_text = committed.stdout
current = read(cutover_path)
pattern = r'pub const CUTOVER_MICROS: u64 = ([0-9_]+);'
match = re.search(pattern, current)
# A HEAD without the declaration has not introduced it yet, exactly like a HEAD without the file.
in_head = re.search(pattern, committed_text)
if not match:
    refuse('janitor must declare CUTOVER_MICROS', cutover_path,
           'declare `pub const CUTOVER_MICROS: u64 = <author second>_000_000;` and commit it at that second')
elif in_head and in_head.group(1) != match.group(1):
    refuse('CUTOVER_MICROS changed after its introducing commit', cutover_path,
           f'restore it to {in_head.group(1)}, the value that commit introduced; it names that commit')
elif in_head:
    log = git('log', '--reverse', '--format=%at', '-S', 'pub const CUTOVER_MICROS:', '--', cutover_path)
    if log.returncode != 0:
        unreadable(cutover_path, f'history cannot be read ({log.stderr.strip()})', 'run this from a full clone, then rerun')
    introduced = log.stdout.splitlines()
    if not introduced or int(match.group(1).replace('_', '')) != int(introduced[0]) * 1_000_000:
        refuse('CUTOVER_MICROS must equal its introducing commit author second in microseconds', cutover_path,
               'set it to the author second of the commit that introduced it, times 1,000,000, '
               f'which git reports as {introduced[0] if introduced else "no commit"}')
if problems:
    for path, text, repair in problems:
        print(f'check-live-janitor: {path}: {text}', file=sys.stderr)
        print(f'check-live-janitor:   next: {repair}', file=sys.stderr)
    sys.exit(1)
print('check-live-janitor: scheduled cleanup targets and credential boundary agree')
PY
