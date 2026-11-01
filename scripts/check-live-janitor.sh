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

# The ci.yml steps that run the janitor: test-linux's cancellation cleanup, one for its GitHub
# artifacts and one for its Linear pool leases — test-linux being the one job that carries the
# credentials. They are credentialed and open no live session, so scripts/check-live-lane.sh and
# scripts/check-live-lane-selection.sh exempt exactly a step running one of these two commands,
# and this holds each to its whole shape: its job, its command, its condition, and exactly the
# credentials it is given.
CANCELLED_RUN = 'cargo run --locked -p onetaskgraph-live-janitor -- --cancelled-run'
RELEASE_LEASES = 'cargo run --locked -p onetaskgraph-live-janitor -- --linear --release-leases target/linear-leases'
CANCELLED_IF = "cancelled() && steps.release.outputs.tests == 'run' && !github.event.pull_request.head.repo.fork"
CANCELLED_ENV = {'GH_PROJECTS_TOKEN', 'GH_PROJECTS_OWNER', 'GH_PROJECTS_NUMBER', 'GH_PROJECTS_REPOSITORY'}
RELEASE_ENV = {'LINEAR_API_KEY', 'LINEAR_WRITE_TEAM'}
ci_steps = re.split(r'(?m)^      - (?=name:|uses:)', ci)
# A job's last step runs on to the next job's header; cut it there.
ci_steps = [re.split(r'(?m)^  (?=[a-z])', step, maxsplit=1)[0] for step in ci_steps]
cleanups = [step for step in ci_steps if re.search(r'onetaskgraph-live-janitor', step)]
test_linux = section(ci, '\n  test-linux:\n', '\n  test-windows:\n', 'test-linux job')
for command, expected_env, credential in [
    (CANCELLED_RUN, CANCELLED_ENV, ('GH_PROJECTS_TOKEN', '${{ secrets.GH_PROJECTS_TOKEN }}')),
    (RELEASE_LEASES, RELEASE_ENV, ('LINEAR_API_KEY', '${{ secrets.LINEAR_API_KEY }}')),
]:
    steps = [step for step in cleanups if values(step, 'run') == [command]]
    if len(steps) != 1:
        refuse(f'ci.yml must run {command!r} in exactly one cancellation step', '.github/workflows/ci.yml',
               'restore the cancellation cleanup step, then rerun')
        continue
    step = steps[0]
    if command not in test_linux:
        refuse(f'the cancellation step running {command!r} must be a step of test-linux, the job that '
               'carries the credentials', '.github/workflows/ci.yml', 'move it back into test-linux, then rerun')
    if values(step, 'if') != [CANCELLED_IF]:
        refuse(f'the cancellation step running {command!r} must run only when {CANCELLED_IF}',
               '.github/workflows/ci.yml', 'restore its if: condition, then rerun')
    environment = section(step, '\n        env:\n', '\n        run:', 'cancellation cleanup env')
    if set(re.findall(r'^          ([A-Z_]+):', environment, re.M)) != expected_env:
        refuse(f'a cancellation step running {command!r} must be given exactly {sorted(expected_env)}',
               '.github/workflows/ci.yml',
               'it reads no run, so it takes no GITHUB_TOKEN and no other credential; restore its env, then rerun')
    if values(step, credential[0]) != [credential[1]]:
        refuse(f'a cancellation step must use {credential[1]}', '.github/workflows/ci.yml',
               'restore its credential, then rerun')
if len(cleanups) != 2 or any(values(step, 'run')[:1] not in ([CANCELLED_RUN], [RELEASE_LEASES]) for step in cleanups):
    refuse('ci.yml runs the janitor only in the two cancellation steps', '.github/workflows/ci.yml',
           'remove the other janitor step, then rerun')
for key in ('GH_PROJECTS_OWNER', 'GH_PROJECTS_NUMBER', 'GH_PROJECTS_REPOSITORY'):
    nominated = values(source, key)
    if len(nominated) != 1 or values(ci, key) != nominated * 2:
        refuse(f'{key} differs from the credentialed ci.yml step and its cancellation cleanup')
if values(source, 'GH_PROJECTS_NUMBER') != ['1']:
    refuse('GH_PROJECTS_NUMBER must name board 1 only')
if values(source, 'GH_PROJECTS_REPOSITORY') != [constant('SCRATCH_REPOSITORY')]:
    refuse('GH_PROJECTS_REPOSITORY differs from SCRATCH_REPOSITORY')
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
if values(source, 'run') != ['cargo run --locked -p onetaskgraph-live-janitor',
                             'cargo run --locked -p onetaskgraph-live-janitor -- --linear']:
    refuse('run must invoke the GitHub and Linear janitor entry points')
linear_steps = [step for step in re.split(r'(?m)^      - (?=name:|uses:)', source)
                if values(step, 'run') == ['cargo run --locked -p onetaskgraph-live-janitor -- --linear']]
if len(linear_steps) != 1:
    refuse('exactly one step must sweep Linear documents')
else:
    linear_env = section(linear_steps[0], '\n        env:\n', '\n        run:', 'Linear cleanup env')
    if set(re.findall(r'^          ([A-Z_]+):', linear_env, re.M)) != {'LINEAR_API_KEY', 'LINEAR_WRITE_TEAM'}:
        refuse('Linear cleanup must receive only its own credential and test-team nomination')
    if values(linear_env, 'LINEAR_API_KEY') != ['${{ secrets.LINEAR_API_KEY }}']:
        refuse('Linear cleanup must use secrets.LINEAR_API_KEY')
    if values(linear_env, 'LINEAR_WRITE_TEAM') != ['${{ vars.LINEAR_WRITE_TEAM }}']:
        refuse('Linear cleanup must nominate vars.LINEAR_WRITE_TEAM')
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
if problems:
    for path, text, repair in problems:
        print(f'check-live-janitor: {path}: {text}', file=sys.stderr)
        print(f'check-live-janitor:   next: {repair}', file=sys.stderr)
    sys.exit(1)
print('check-live-janitor: scheduled cleanup targets and credential boundary agree')
PY
