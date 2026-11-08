"""The `budgets` job of ci.yml, driven at GitHub's boundary with the action release it pins.

Each journey runs what the job's `uses:` pins with its `with:` inputs: action.yml's own steps, the
released driver and plugin archive, the published core wheel, the real `gh` and the real
`actions/upload-artifact` code, by the harness the action ships at that same tag. Only GitHub's
services, the release host and the package index are doubled. The repository under the job is a
copy of this tree, so every row of the real catalog is proven. Reaching the release, PyPI and
codeload once is why a cold run needs a network; the job runs on Linux, so its proof does too.

The figures of `onetaskgraph-ci-check-time` and `onetaskgraph-ci-job-count` are REPLAYED TELEMETRY,
never live CI. On a pull request the real figures script reads its own run through the real `gh` from
the double, which serves `recorded_jobs()`; on a push or a tag the real onevcs reads this repository's
recordings of v0.13.0 through ONEVCS_GH, with each pull request's jobs edited as `replay()` states.

`SavedResults` drives the whole path of a saved figure: this repository's real `budgets` targets under
real Nx, a replayed cache hit included, ci.yml's real upload steps, the action's real download, the
comment and the base push's history. `recorded_figures()` stands in for the workloads alone, replacing
each command with one that reports a recorded figure.
"""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from dataclasses import asdict, dataclass
from typing import Literal, NamedTuple, NewType
from types import ModuleType
import unittest
import urllib.request

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
TELEMETRY = HERE / '.telemetry'
STATE = ROOT / 'target' / 'report-job'
ACTION = 'nickderobertis/onebudgetspec-github-action'
RECORDINGS = ROOT / '.github/policy/tests/fixtures/release-tail/recordings-v0.13.0'
BudgetId = NewType('BudgetId', str)
MetricName = NewType('MetricName', str)
Unit = Literal['count', 'seconds', 'percent', 'requests', 'bytes', 'jobs']
Direction = Literal['max', 'min']
Measure = Literal['reported', 'elapsed']
Verdict = Literal['within', 'over', 'error']
RELEASE_TAG = re.compile(r'v\d+\.\d+\.\d+')
CHECK_TIME = BudgetId('onetaskgraph-ci-check-time')
JOB_COUNT = BudgetId('onetaskgraph-ci-job-count')
CD_JOB_COUNT = BudgetId('onetaskgraph-cd-job-count')
OWN = BudgetId('onetaskgraph-budget-report-command-seconds')
REST_METRIC = MetricName('report-github-api-requests-per-run')
TIME_METRIC = MetricName('report-command-seconds')
RUNNER_TARGETS = {'x86_64': 'x86_64-unknown-linux-gnu', 'aarch64': 'aarch64-unknown-linux-gnu'}
# The workload .github/reporting/budgets.yaml states for the job's own budget, and the command reads
# the 20-request design total allows for.
ROWS = 60
COMMITS = 20
SAVED_ARTIFACT_BYTES = 50_000
UNJUDGED = 1_000_000
# Below this many samples the action's report shows a row as having no baseline yet.
MINIMUM_SAMPLES = 5
# Exactly the budgets a pull request cannot measure or move, which the job leaves out.
OMIT_LABEL = 'not-measured-on-pr'
OMITTED = frozenset(BudgetId(identifier) for identifier in (
    'onetaskgraph-cd-job-time', 'onetaskgraph-release-cycle-time', 'onetaskgraph-cd-job-count',
    'onetaskgraph-release-read-requests', 'onetaskgraph-pr-read-requests', 'onetaskgraph-nx-cache-use',
    'onetaskgraph-gate-time', 'sdk-import-cpu-seconds', 'github-plan-copy-real-seconds-unchanged',
    'github-plan-copy-real-seconds-changed', 'github-plan-copy-real-seconds-first'))
OMITTED_LINE = (f'{len(OMITTED)} budgets are left out because a pull request cannot measure them '
                f'(label {OMIT_LABEL}).')
LEFT_OUT = 'left out because a pull request cannot measure them'
NO_SAVED_RESULT = 'no saved result was found on this run'
# The core's comment layout, which the pinned release posts: where each part opens, each closing at
# its next `</details>`, and the headings and sentences it writes inside them.
Section = Literal['attention', 'within', 'not-measured']
SECTIONS = {'attention': '<details open><summary><b>Needs attention (', 'within': '<details><summary><b>Within budget (',
            'details': '<details><summary>Details</summary>'}
ATTENTION_HEADER = '| Budget | Now · vs base | Reason |'
WITHIN_HEADER = '| Budget | Now · vs base |'
NOTHING_NEEDS_ATTENTION = 'Nothing failed or was flagged.'
NOT_MEASURED = 'Not measured on this run'
MISSING_FIGURES = 'Why some figures are missing'
# Off a pull request's own run the CI-time pair reads a release, and this job holds no onevcs.
FIGURES_NEED_ONEVCS = 'onevcs is not on PATH'
LIVE_SESSION = BudgetId('github-live-session-points')
# The one row beyond the CI-time pair whose command runs in the job: it needs no release and no saved
# result, and what it reads is GitHub's cache listing.
HEADROOM = BudgetId('onetaskgraph-cache-headroom')
CACHE_LISTING_REFUSED = 'actions/cache/storage-limit answered HTTP'
# ci.yml's budgets targets test-linux runs, by the file each saves. Nx caches the three whose names
# hold no test of its own to re-run; the other two always execute.
CACHED_SAVING_TARGETS = {'onetaskgraph-live-janitor:budgets': 'onetaskgraph-live-janitor-budget-results.json',
                         'onetaskgraph-linear-e2e:budgets': 'onetaskgraph-linear-e2e-budget-results.json',
                         'onetaskgraph-github-projects-e2e:budgets': 'onetaskgraph-github-projects-e2e-budget-results.json'}
UNCACHED_SAVING_TARGETS = {'github-workflow-policy:budgets': 'github-workflow-policy-budget-results.json',
                           'report-job:budgets': 'report-job-budget-results.json'}
SAVING_BUDGET_FILES = ('crates/onetaskgraph-live-janitor/budgets.yaml', 'crates/onetaskgraph-linear-e2e/budgets.yaml',
                       'crates/onetaskgraph-github-projects-e2e/budgets.yaml', 'pool-maintenance/budgets.yaml',
                       '.github/policy/budgets.yaml', '.github/reporting/budgets.yaml')
LONGEST_JOB_SECONDS = 1234
RECORDED_OWN_SECONDS = 77.5
TRANSFER_SERVICES = ('artifact-service', 'blob-storage', 'object-storage', 'release-host', 'package-index')


@dataclass(frozen=True)
class CatalogBudget:
    """One production budget, as its budgets file registers it."""

    id: BudgetId
    command: tuple[str, ...]


@dataclass(frozen=True)
class FixtureBudget:
    """A budget a journey adds to its copy of the tree."""

    id: BudgetId
    command: tuple[str, ...]
    unit: Unit = 'count'
    direction: Direction = 'max'
    threshold: float = 100
    measure: Measure = 'reported'
    labels: tuple[str, ...] = ()

    def harness_entry(self):
        """The (id, command, options) entry the released harness's `budgets_yaml` takes."""
        options = {'unit': self.unit, 'direction': self.direction, 'threshold': self.threshold,
                   'measure': self.measure}
        return (self.id, list(self.command), options | ({'labels': list(self.labels)} if self.labels else {}))


@dataclass(frozen=True)
class Refusal:
    """A status the doubled host answers a path with."""

    path: str
    status: int
    times: int
    retry_after: int | None = None

    def harness_rule(self):
        """The rule the released harness's `Host.refuse` takes."""
        rule = {'path': self.path, 'status': self.status, 'times': self.times}
        return rule if self.retry_after is None else rule | {'retry_after': self.retry_after}


@dataclass(frozen=True)
class ReplayedFigure:
    """What onebudgetspec answered for one registered command over replayed telemetry."""

    actual: float
    threshold: float
    verdict: Verdict


@dataclass(frozen=True)
class ReplayedFigures:
    """The recording of those answers, per the replay's longest job in seconds."""

    provenance: str
    figures: dict[int, dict[BudgetId, ReplayedFigure]]


class RecordedFigures(NamedTuple):
    """What `recorded_figures` installed in a copy of the tree."""

    figures: dict[BudgetId, float]
    log: Path


class SavingTree(NamedTuple):
    """A journey over a copy of this tree whose saving targets judge recorded figures."""

    journey: 'Journey'
    figures: dict[BudgetId, float]
    log: Path


class SavedFigure(NamedTuple):
    """One budget's result in a saved check report."""

    actual: float
    verdict: Verdict


class CommandOutcome(NamedTuple):
    """How one check's real entry point ended."""

    status: int
    output: str


class EnforcingCheck(NamedTuple):
    """A real check of the left-out budgets, and how it treats one over its line."""

    name: str
    budgets_file: str
    command: list[str]
    status: int
    warns: bool
    budgets: frozenset[BudgetId]


class Released(NamedTuple):
    """The action at ci.yml's pin."""

    tag: str
    tree: Path
    harness: ModuleType


def workflow_job():
    return yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())['jobs']['budgets']


def action_step():
    steps = [step for step in workflow_job()['steps'] if step.get('uses', '').startswith(f'{ACTION}@')]
    assert len(steps) == 1, steps
    return steps[0]


def job_inputs(**override):
    """The action inputs ci.yml really names, the token stood in for."""
    given = dict(action_step()['with'])
    assert given.pop('token') == '${{ github.token }}'
    return {'token': 'job-token', **given, **override}


def workflow_name():
    return yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())['name']


def fetch(url):
    # llmlint: ignore[async_typed_clients_at_boundaries] A test's one-shot download of a pinned release asset, the way the released harness fetches its own; nothing here is concurrent or a service client.
    with urllib.request.urlopen(url, timeout=120) as answer:
        return answer.read()


def released_action():
    """The action at ci.yml's pin: its tree, its harness and its archive, fetched once."""
    tag = action_step()['uses'].split('@', 1)[1]
    assert RELEASE_TAG.fullmatch(tag), f'ci.yml pins the action at {tag}, which is no release tag'
    tree = STATE / tag
    version = tag.removeprefix('v')
    name = f'onebudgetspec-github-action-{version}-{RUNNER_TARGETS[platform.machine()]}.tar.gz'
    archive = tree / 'dist' / name
    if not archive.is_file():
        shutil.rmtree(tree, ignore_errors=True)
        tree.mkdir(parents=True)
        source = fetch(f'https://codeload.github.com/{ACTION}/tar.gz/refs/tags/{tag}')
        with tarfile.open(fileobj=io.BytesIO(source)) as bundle:
            for member in bundle.getmembers():
                member.name = member.name.split('/', 1)[-1]
                bundle.extract(member, tree, filter='data')
        releases = f'https://github.com/{ACTION}/releases/download/{tag}'
        (tree / 'release-action.yml').write_bytes(fetch(f'{releases}/action.yml'))
        payload = fetch(f'{releases}/{name}')
        listed = f'{hashlib.sha256(payload).hexdigest()}  {name}'
        assert listed in fetch(f'{releases}/SHA256SUMS').decode().splitlines(), name
        archive.parent.mkdir()
        archive.write_bytes(payload)
    spec = importlib.util.spec_from_file_location('released_action_harness', tree / 'tests/action-harness/harness.py')
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    harness._cached['archive'] = archive
    # The harness knows the artifact actions the action itself pins. ci.yml's upload steps pin their
    # own, and it is that release of the action this file proves.
    pinned_by_the_action = harness.artifact_action

    # llmlint: ignore[protocol_based_seams] The released harness is one fixed module with no seam to implement; this wraps its one resolver for one more action, and only this file replaces it.
    def artifact_action(uses):
        name, reference = uses.split('@', 1)
        if name == 'actions/upload-artifact' and uses in ci_upload_actions():
            return harness._action_entry('upload-artifact', (reference, reference), 'dist/upload/index.js')
        return pinned_by_the_action(uses)

    harness.artifact_action = artifact_action
    return Released(tag, tree, harness)


TAG, TREE, harness = (None, None, None)


def ci_upload_step(job_name):
    """The one step of ci.yml's job uploading saved budget results."""
    job = yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())['jobs'][job_name]
    steps = [step for step in job['steps'] if str(step.get('with', {}).get('name', '')).startswith('budget-check-results-')]
    assert len(steps) == 1, (job_name, steps)
    return steps[0]


def ci_upload_actions():
    return {ci_upload_step(job_name)['uses'] for job_name in ('test-linux', 'lint')}


def recorded_jobs(longest_seconds):
    """A pull request's jobs as GitHub really listed them, with this journey's `budgets` job running.

    The first job lasts `longest_seconds`, the others less, and `budgets` is added in progress: the job
    reading this listing, which has no time yet and counts toward the job count alone.
    """
    recording = next(path for path in sorted(RECORDINGS.glob('jobs-of-pr-*.http')))
    text = recording.read_text()
    split = text.index('\r\n\r\n') + 4 if '\r\n\r\n' in text else text.index('\n\n') + 2
    # llmlint: ignore-block[modern_domain_modeling] This edits GitHub's recorded response JSON in place and serves it back whole; a model would drop every field the edit does not name.
    jobs = json.loads(text[split:])['jobs']
    day = jobs[0]['started_at'][:11]
    minutes, seconds = divmod(longest_seconds, 60)
    hours, minutes = divmod(minutes, 60)
    finished = [dict(job, started_at=f'{day}01:00:00Z', completed_at=f'{day}01:00:30Z', steps=[]) for job in jobs]
    finished[0]['completed_at'] = f'{day}{1 + hours:02}:{minutes:02}:{seconds:02}Z'
    running = dict(finished[-1], id=finished[-1]['id'] + 1, name='budgets', status='in_progress', conclusion=None,
                   completed_at=None)
    # llmlint: ignore-end[modern_domain_modeling]
    return [*finished, running]


def real_onevcs():
    """The onevcs release.yml's tail installs, the one the replay's recordings were driven through."""
    pin = re.search(r'onevcs-cli@(\S+)', (ROOT / '.github/workflows/release.yml').read_text())[1]
    assert RELEASE_TAG.fullmatch(f'v{pin}'), f'release.yml pins onevcs at {pin}, which is no release version'
    prefix = STATE / f'onevcs-{pin}'
    binary = prefix / 'node_modules/.bin/onevcs'
    if not binary.exists():
        subprocess.run(['npm', 'install', '--no-save', '--no-package-lock', '--no-audit', '--no-fund', '--prefix',
                        str(prefix), f'onevcs-cli@{pin}'], check=True, capture_output=True)
    return binary


def runner_bin(directory, record, bus_requests):
    """What a hosted runner's PATH holds for a budget command: node and cargo, no uv, bun, just or onevcs.

    `cargo` records that it was started: the runner really has one, and a budget naming saved
    telemetry must not start a build there. `dbus-launch` records each session bus the real `gh` asks
    for and starts none, so a job that loses the variable below fails rather than leaves a daemon.
    """
    directory.mkdir(parents=True, exist_ok=True)
    (directory / 'node').symlink_to(shutil.which('node'))
    cargo = directory / 'cargo'
    cargo.write_text(f'#!/bin/sh\necho "cargo $*" >> "{record}"\nexit 1\n')
    cargo.chmod(0o755)
    bus = directory / 'dbus-launch'
    bus.write_text(f'#!/bin/sh\necho "dbus-launch $*" >> "{bus_requests}"\nexit 1\n')
    bus.chmod(0o755)
    return f'{directory}{os.pathsep}/usr/bin{os.pathsep}/bin'


def job_environment(path, run_id, overrides):
    """What every `harness.Job` adds to the harness's hermetic environment.

    The action's own `gh` calls hold its token, which applies at the double's host and asks for no
    session bus. A `gh` a step starts without one falls back to the keyring, and that autolaunches one.
    """
    return {'PATH': path,
            'DBUS_SESSION_BUS_ADDRESS': 'disabled:',
            'GITHUB_REPOSITORY': harness.REPOSITORY,
            'GITHUB_RUN_ID': str(run_id),
            'RUNNER_NAME': 'hosted-runner',
            **overrides}


def replay(directory, longest_seconds):
    """A double of `gh`, serving v0.13.0's recordings through ONEVCS_GH.

    Each pull request's run gains the `budgets` job, so it counts 5 jobs, and its first job is set
    to last `longest_seconds`. The release commit gains the `ci` run of a push to main, whose jobs
    are recorded nowhere: a CD figure reading them would fail on the missing recording.
    """
    responses = directory / 'responses'
    responses.mkdir(parents=True)
    routes = []
    for index, path in enumerate(sorted(RECORDINGS.glob('*.http'))):
        text = path.read_text()
        source = json.loads(Path(f'{path}.source.json').read_text())
        split = text.index('\r\n\r\n') + 4 if '\r\n\r\n' in text else text.index('\n\n') + 2
        head, body = text[:split], json.loads(text[split:])
        # llmlint: ignore-block[modern_domain_modeling] This edits GitHub's recorded response JSON in place and writes it back whole; a model would drop every field the edit does not name.
        if path.name.startswith('jobs-of-pr-'):
            first = body['jobs'][0]
            started = first['started_at']
            minutes, rest = divmod(longest_seconds, 60)
            hours, minutes = divmod(minutes, 60)
            day = started[:11]
            first['started_at'] = f'{day}01:00:00Z'
            first['completed_at'] = f'{day}{1 + hours:02}:{minutes:02}:{rest:02}Z'
            body['jobs'] = [first] + [dict(job, started_at=first['started_at'], completed_at=f'{day}01:00:30Z',
                                           steps=[]) for job in body['jobs'][1:]]
            body['jobs'].append(dict(body['jobs'][-1], id=body['jobs'][-1]['id'] + 1, name='budgets'))
            body['total_count'] = len(body['jobs'])
        if path.name == 'runs-at-release-commit.http':
            ci = dict(body['workflow_runs'][0], id=999000001, name=workflow_name(), event='push', conclusion='success')
            body['workflow_runs'].append(ci)
            body['total_count'] = len(body['workflow_runs'])
        # llmlint: ignore-end[modern_domain_modeling]
        (responses / str(index)).write_text(head + json.dumps(body))
        routes.append(f'{source["path"]}\t{index}\n')
    (directory / 'routes.tsv').write_text(''.join(routes))
    program = directory / 'gh'
    program.write_text(
        '#!/bin/sh\n'
        f'dir="{directory}"\n'
        'for arg in "$@"; do last="$arg"; done\n'
        'request="${last#https://api.github.com/}"\n'
        'n=$(awk -F "\\t" -v r="$request" \'$1 == r { print $2; exit }\' "$dir/routes.tsv")\n'
        'if [ -z "$n" ]; then echo "replay: no recording for $request" >&2; exit 1; fi\n'
        'cat "$dir/responses/$n"\n')
    program.chmod(0o755)
    # Named to the figures script alone through ONEVCS: the action's own proxy forwards through
    # ONEVCS_GH, so the job's environment must not carry this double under that name.
    onevcs = directory / 'onevcs'
    onevcs.write_text(f'#!/bin/sh\nONEVCS_GH="{program}" exec "{real_onevcs()}" "$@"\n')
    onevcs.chmod(0o755)
    return onevcs


class Journey:
    """A copy of this tree and one host double, driven through ci.yml's job."""

    def __init__(self, test):
        self.test = test
        self.scratch = tempfile.TemporaryDirectory()
        test.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.host = harness.Host()
        test.addCleanup(self.host.close)
        self.repository = harness.Repository()
        test.addCleanup(self.repository.close)
        listed = subprocess.run(['git', 'ls-files', '-z', '-co', '--exclude-standard'], cwd=ROOT,
                                capture_output=True, check=True).stdout.decode().split('\0')
        for name in filter(None, listed):
            source = ROOT / name
            if source.is_file():
                target = self.repository.path / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, target)
        self.cargo_started = self.root / 'cargo-started'
        self.bus_requests = self.root / 'session-bus-requests'
        test.addCleanup(lambda: test.assertFalse(self.bus_requests.exists(), 'a real gh asked for a session bus'))
        self.path = runner_bin(self.root / 'runner-bin', self.cargo_started, self.bus_requests)
        self.next_run = 1000
        self.environment = {}
        self.release_environment = {}
        self.longest_seconds = LONGEST_JOB_SECONDS
        self.serve_own_run()
        # This test records what the job cost; report-job:budgets alone judges it.
        own = self.repository.path / '.github/reporting/budgets.yaml'
        own.write_text(own.read_text().replace('threshold: 120', f'threshold: {UNJUDGED}'))
        self.base = self.repository.commit('the tree')

    def serve_own_run(self):
        """GitHub's job listing of a run, which the harness the action ships has no need of."""
        answer_otherwise = self.host.read

        # llmlint: ignore[protocol_based_seams] The released harness's host is one fixed class with no seam to implement; this adds the one GitHub route it has no need of, and only this file replaces it.
        def read(handler, path):
            if re.fullmatch(r'/repos/owner/repo/actions/runs/\d+/jobs', path.split('?')[0]) is None:
                return answer_otherwise(handler, path)
            jobs = recorded_jobs(self.longest_seconds)
            # llmlint: ignore[modern_domain_modeling] This is GitHub's job-listing response on the wire, and the figures script validates its shape.
            return self.host.reply(handler, 200, json.dumps({'total_count': len(jobs), 'jobs': jobs}).encode())

        self.host.read = read

    def add_budgets(self, relative, budgets):
        self.repository.write(relative, harness.budgets_yaml([budget.harness_entry() for budget in budgets]))

    def refuse(self, refusal):
        self.host.refuse.append(refusal.harness_rule())

    def run(self, event_name, sha, inputs=None, **options):
        self.next_run += 1
        # What a runner sets for every step: a pull request's figures read its own run, and only a
        # push or a tag sees the release the recordings serve.
        overrides = {**self.environment, 'GITHUB_EVENT_NAME': event_name,
                     **(self.release_environment if event_name != 'pull_request' else {})}
        job = harness.Job(self.host, self.repository, event_name, sha, inputs=inputs or job_inputs(),
                          run_id=self.next_run, action_path=TREE,
                          environment=job_environment(self.path, self.next_run, overrides), **options)
        self.test.addCleanup(job.close)
        return job.run()

    def push(self, inputs=None):
        self.repository.git('checkout', '--quiet', 'main')
        self.base = self.repository.commit('merged')
        return self.run('push', self.base, inputs=inputs)

    def pull_request(self, inputs=None, branch='change', **event):
        self.repository.git('checkout', '--quiet', '-B', branch, self.base)
        head = self.repository.commit('change')
        self.host.head = head
        job = self.run('pull_request', head, inputs=inputs, base_ref='main', register=False,
                       event=harness.event_pull_request(1, head, self.base, **event))
        # Every comment a journey posts is held to the core's layout over its own run's saved rows.
        if any(f'head: {head} ' in comment['body'] for comment in self.host.comments):
            assert_layout(self.test, job)
        return job

    def replaying(self, longest_seconds):
        """A base push sees a release and the real onevcs, over the recordings; a pull request reads its run."""
        self.longest_seconds = longest_seconds
        self.release_environment = {
            'ONEVCS': str(replay(Path(tempfile.mkdtemp(dir=self.root)), longest_seconds)),
            'ONEVCS_HOME': str(self.root / 'onevcs-home'),
            'GITHUB_REPOSITORY': 'nickderobertis/onetaskgraph',
            'GITHUB_REF_TYPE': 'tag',
            'GITHUB_REF_NAME': 'v0.13.0',
            'GH_TOKEN': 'replay-token',
        }


class Placement(NamedTuple):
    """Where the core's comment layout shows one budget, and what it shows there."""

    section: Section
    # A table row's cells after Budget, or a Not measured line's text after the budget's name.
    cells: tuple[str, ...]


def section(markdown, name):
    """One part of the core's comment layout, from where it opens to its first `</details>`."""
    start = markdown.find(SECTIONS[name])
    assert start >= 0, (name, markdown)
    return markdown[start:markdown.index('</details>', start)]


def table_rows(text):
    """A section's table rows, as each budget's shown name and its other cells."""
    rows = []
    for line in text.splitlines():
        if line.startswith('| **'):
            name, *cells = [cell.strip() for cell in line.strip().strip('|').split('|')]
            rows.append((name.removeprefix('**').removesuffix('**'), tuple(cells)))
    return rows


def listed(text, heading):
    """The `- names: text` bullets under one bold heading of the Details section, as (name, text)."""
    if f'\n**{heading}' not in text:
        return []
    block = text.split(f'\n**{heading}', 1)[1].split('\n\n', 2)[1]
    return [(name, said) for line in block.splitlines() if line.startswith('- ')
            for names, said in [line[2:].split(': ', 1)] for name in names.split(', ')]


def shown_names(report):
    """The name the core's layout shows each saved row under: its id less the repository's own prefix,
    with its file beside it when another file registers the same id."""
    prefix = report['current']['repository'].rsplit('/', 1)[-1] + '-'
    names = {}
    for row in report['rows']:
        short = row['id'].removeprefix(prefix) or row['id']
        shared = any(other['id'] == row['id'] and other['file'] != row['file'] for other in report['rows'])
        names[f'{short} ({row["file"]})' if shared else short] = BudgetId(row['id'])
    return names


def comment_body(job):
    """The comment `job` posted or updated for its own head, or its report when it posted none."""
    posted = [comment['body'] for comment in job.host.comments if f'head: {job.sha} ' in comment['body']]
    return posted[-1] if posted else job.markdown()


def placements(job):
    """Every place the report part of `job`'s comment shows a budget, by budget id, each appearing once."""
    markdown = comment_body(job)
    names = shown_names(job.saved('comparison-report.json'))
    found = [*((name, Placement('attention', cells)) for name, cells in table_rows(section(markdown, 'attention'))),
             *((name, Placement('within', cells)) for name, cells in table_rows(section(markdown, 'within'))),
             *((name, Placement('not-measured', (said,)))
               for name, said in listed(section(markdown, 'details'), NOT_MEASURED))]
    shown = [name for name, _ in found]
    assert len(shown) == len(set(shown)), f'a budget shown twice: {sorted(name for name in shown if shown.count(name) > 1)}'
    assert set(shown) <= set(names), f'a budget no saved row has: {sorted(set(shown) - set(names))}'
    return {names[name]: placement for name, placement in found}


def missing_reasons(job):
    """Each budget's reason under `Why some figures are missing`, by budget id."""
    names = shown_names(job.saved('comparison-report.json'))
    return {names[name]: said for name, said in listed(section(comment_body(job), 'details'), MISSING_FIGURES)}


def signed(value):
    """A change as the core's layout writes it: signed, to one decimal, with no negative zero."""
    rounded = f'{value:+.1f}'
    return '+0.0' if rounded == '-0.0' else rounded


def report_only_ids(report):
    excluded = set(report['current']['enforcement_exclude_labels'])
    return {(budget['file'], budget['id']) for budget in report['current']['catalog'] if excluded & set(budget['labels'])}


def expected_placement(row, report_only):
    """The section the core contract puts a saved row in, with the Reason it gives one needing attention."""
    verdict = row.get('verdict')
    if verdict == 'over' or (verdict == 'error' and not report_only) or row['flag']:
        if verdict == 'over':
            phrase = 'Over limit · flagged' if row['flag'] else 'Over limit'
        elif verdict == 'error':
            phrase = 'Error'
        elif row.get('sd') == 0:
            phrase = 'Flagged (was constant)'
        else:
            phrase = f'Flagged {signed(row.get("sigma_change") or 0)} SD'
        return 'attention', phrase + (' (report-only)' if report_only else '')
    return ('within', None) if verdict == 'within' else ('not-measured', None)


def assert_layout(test, job):
    """The comment `job` posted, in the core's layout, holding exactly its run's saved rows, each once."""
    report = job.saved('comparison-report.json')
    markdown = comment_body(job)
    shown = placements(job)
    rows = {BudgetId(row['id']): row for row in report['rows']}
    test.assertEqual(set(shown), set(rows), 'the three sections hold exactly the saved results')
    test.assertEqual(len(rows), len(report['rows']), 'one saved row per budget id')
    report_only = report_only_ids(report)
    counts = {'attention': 0, 'within': 0, 'not-measured': 0}
    for identifier, row in rows.items():
        place, reason = expected_placement(row, (row['file'], row['id']) in report_only)
        counts[place] += 1
        test.assertEqual(shown[identifier].section, place, (identifier, row))
        if place == 'attention':
            test.assertEqual(len(shown[identifier].cells), 2, identifier)
            test.assertEqual(shown[identifier].cells[1], reason, (identifier, row))
        elif place == 'within':
            test.assertEqual(len(shown[identifier].cells), 1, f'{identifier}: Budget and Now · vs base alone')
    attention = section(markdown, 'attention')
    test.assertTrue(attention.startswith(f'{SECTIONS["attention"]}{counts["attention"]})</b></summary>'), attention)
    if counts['attention']:
        test.assertIn(f'\n{ATTENTION_HEADER}\n', attention)
    else:
        test.assertEqual(attention.split('</summary>\n\n', 1)[1], f'{NOTHING_NEEDS_ATTENTION}\n\n')
    within = section(markdown, 'within')
    test.assertTrue(within.startswith(f'{SECTIONS["within"]}{counts["within"]})</b></summary>'), within)
    if counts['within']:
        test.assertIn(f'\n{WITHIN_HEADER}\n', within)
    for mark in ('✅', '❌', '⚠️'):
        test.assertEqual([line for line in markdown.splitlines() if line.startswith('|') and mark in line], [],
                         f'no table cell holds {mark}')
    summary = re.search(r'^\*\*(?:⚠️ (\d+) need attention|✅ Nothing needs attention)\*\* · (\d+) within budget'
                        r'(?: · (\d+) not measured)? · head ', markdown, re.MULTILINE)
    test.assertIsNotNone(summary, markdown)
    test.assertEqual(tuple(int(count or 0) for count in summary.groups()),
                     (counts['attention'], counts['within'], counts['not-measured']), summary[0])
    left_out = len([budget for budget in job.saved('budget-run.json')['catalog'] if OMIT_LABEL in budget['labels']])
    lines = [line for line in markdown.splitlines() if LEFT_OUT in line]
    test.assertEqual(len(lines), 1 if left_out else 0, 'the left-out line is shown once, when any budget is left out')
    test.assertEqual([line for line in section(markdown, 'details').splitlines() if LEFT_OUT in line], lines,
                     'the left-out line is inside Details')
    if left_out == len(OMITTED):
        test.assertEqual(section(markdown, 'details').count(OMITTED_LINE), 1, 'the left-out line is inside Details')


def history_figures(job, sha):
    """Each budget's figure in the rolling history entry of `sha`, which `job` saved."""
    history = json.loads(job.host.artifact_files(job.run_id)['rolling-history.json'])
    [entry] = [saved_run for saved_run in history['runs'] if saved_run['head_sha'] == sha]
    return {result['id']: result['actual'] for result in entry['results']['results']}


def measured(saved_run):
    """The budgets one saved base run has a figure for."""
    return {result['id'] for result in saved_run['results']['results']}


def catalog():
    """Every production budget, from the same discovery rule the action applies."""
    found = {}
    listed = subprocess.run(['git', 'ls-files', '-co', '--exclude-standard'], cwd=ROOT, capture_output=True,
                            text=True, check=True).stdout.split()
    for name in listed:
        if Path(name).name == 'budgets.yaml' and not {'fixture', 'fixtures'} & set(Path(name).parts):
            for budget in yaml.safe_load((ROOT / name).read_text())['budgets']:
                identifier = BudgetId(budget['id'])
                found[identifier] = CatalogBudget(identifier, tuple(budget['command']))
    return found


def shown():
    """The production budgets a pull request's report has a row for."""
    return catalog().keys() - OMITTED


def assert_left_out_and_counted(test, job):
    """The budgets a pull request cannot measure: no row, no command run, and one line counting them."""
    rows = job.rows()
    run = job.saved('budget-run.json')
    outcomes = {result['id'] for result in run['results']['results']} | {entry['id'] for entry in run['unavailable']}
    markdown = job.markdown()
    for identifier in OMITTED:
        test.assertNotIn(identifier, rows)
        test.assertNotIn(identifier, outcomes, 'an omitted budget is neither run nor reported')
        test.assertNotIn(identifier, placements(job))
        test.assertNotIn(f'**{identifier}**', markdown)
    test.assertEqual(sum(OMITTED_LINE in line for line in markdown.splitlines()), 1, markdown)
    test.assertEqual(len([budget for budget in run['catalog'] if OMIT_LABEL in budget['labels']]), len(OMITTED),
                     'budget-run.json keeps the catalog whole')


class SessionBus(unittest.TestCase):
    def test_the_real_gh_under_a_job_asks_for_no_session_bus_and_does_once_the_variable_is_gone(self):
        if shutil.which('gh') is None:
            self.skipTest('gh is not installed')
        with tempfile.TemporaryDirectory() as scratch:
            requests = Path(scratch) / 'session-bus-requests'
            path = runner_bin(Path(scratch) / 'runner-bin', Path(scratch) / 'cargo-started', requests)
            # This holds no step's token, and nothing listens at the proxy: gh asks for a token first.
            environment = {**harness.real_gh_environment('http://127.0.0.1:1', scratch), 'HOME': scratch,
                           **job_environment(path, 1, {})}

            def gh(env):
                subprocess.run(['gh', 'api', 'repos/owner/repo/commits/0'], env=env, capture_output=True,
                               check=False, timeout=30)
                return len(requests.read_text().splitlines()) if requests.exists() else 0

            control = {name: value for name, value in environment.items() if name != 'DBUS_SESSION_BUS_ADDRESS'}
            if gh(control) == 0:
                self.skipTest('this gh asks for no session bus even without the variable, so none can leak')
            self.assertEqual(gh(environment), 1, "the real gh asked for a session bus under a job's environment")


class ProductionCatalog(unittest.TestCase):
    def test_every_budget_a_pull_request_can_read_has_one_row_the_job_stays_green_and_nothing_builds(self):
        journey = Journey(self)
        job = journey.pull_request()
        self.assertEqual(job.exit_code, 0, job.stdout)
        rows = job.rows()
        self.assertEqual(set(rows), shown())
        self.assertEqual(len(rows) + len(OMITTED), len(catalog()))
        run = job.saved('budget-run.json')
        verdicts = {result['id']: result['verdict'] for result in run['results']['results']}
        self.assertEqual(verdicts, {OWN: 'within', CHECK_TIME: 'within', JOB_COUNT: 'within'},
                         'the commands that can run here')
        self.assertFalse(journey.cargo_started.exists(), 'a budget started a build on the runner')
        reasons = {entry['id']: entry['reason'] for entry in run['unavailable']}
        markdown = job.markdown()
        self.assertIn(job.sha, markdown)
        places = placements(job)
        missing = missing_reasons(job)
        for identifier in shown() - set(verdicts) - {HEADROOM}:
            self.assertEqual(reasons[identifier], NO_SAVED_RESULT, identifier)
            self.assertEqual(places[identifier], Placement('not-measured', ('no result this run · baseline 0/5',)))
            self.assertEqual(missing[identifier], NO_SAVED_RESULT, identifier)
        # Its command does run here; this job's double holds no cache listing, and the read says so.
        self.assertIn(CACHE_LISTING_REFUSED, reasons[HEADROOM])
        self.assertIn(CACHE_LISTING_REFUSED, missing[HEADROOM])
        self.assertEqual(places[HEADROOM], Placement('not-measured', ('no result this run (report-only) · baseline 0/5',)))
        self.assertEqual({places[identifier].section for identifier in verdicts}, {'within'})
        assert_left_out_and_counted(self, job)

    def test_a_pull_request_reports_its_own_runs_jobs_with_one_read_per_figure(self):
        journey = Journey(self)
        journey.host.clear_requests()
        job = journey.pull_request()
        self.assertEqual(job.exit_code, 0, job.stdout)
        rows = job.rows()
        self.assertEqual(rows[CHECK_TIME]['current'], LONGEST_JOB_SECONDS, 'the job still running has no time yet')
        self.assertEqual(rows[JOB_COUNT]['current'], len(recorded_jobs(LONGEST_JOB_SECONDS)), 'and still counts')
        self.assertEqual(rows[JOB_COUNT]['current'], 5)
        self.assertEqual(journey.host.count(fr'/runs/{job.run_id}/jobs'), 2, 'one read per figure')
        self.assertEqual(journey.host.count(r'/releases/tags|/compare'), 0, 'no release is read')

    def test_a_base_push_with_no_pull_request_says_why_the_ci_time_rows_have_no_figure(self):
        journey = Journey(self)
        base = journey.push()
        self.assertEqual(base.exit_code, 0, base.stdout)
        reasons = {entry['id']: entry['reason'] for entry in base.saved('budget-run.json')['unavailable']}
        for identifier in (CHECK_TIME, JOB_COUNT):
            self.assertIn(FIGURES_NEED_ONEVCS, reasons[identifier])
        self.assertEqual(journey.host.count(r'/runs/\d+/jobs'), 0)

    def test_a_second_head_updates_the_one_comment_and_a_base_push_saves_the_same_results(self):
        journey = Journey(self)
        base = journey.push()
        self.assertEqual(base.exit_code, 0, base.stdout)
        self.assertEqual(journey.host.comments, [], 'a base push posts nothing')
        saved = journey.host.artifact_files(base.run_id)
        self.assertIn('budget-run.json', saved)

        first = journey.pull_request()
        self.assertEqual(first.exit_code, 0, first.stdout)
        self.assertEqual(len(journey.host.comments), 1)
        self.assertIn(first.sha, journey.host.comments[0]['body'])
        self.assertTrue((first.out / 'budget-run.json').is_file(), 'the same result artifact')

        second = journey.pull_request()
        self.assertEqual(second.exit_code, 0, second.stdout)
        self.assertNotEqual(first.sha, second.sha)
        self.assertEqual(len(journey.host.comments), 1, 'one standing comment')
        body = journey.host.comments[0]['body']
        self.assertIn(f'head: {second.sha}', body)
        self.assertNotIn(f'head: {first.sha}', body)
        self.assertIn(first.sha, body, 'the earlier head stays as informational history')

    def test_base_pushes_save_the_jobs_own_budget_as_history_a_pull_request_reads_as_its_baseline(self):
        # On this catalog the job's own budget is the only measurement, and it joins only the final
        # upload: this is the case the pinned release has to save history for.
        journey = Journey(self)
        first = journey.push()
        self.assertEqual(first.exit_code, 0, first.stdout)
        history = json.loads(journey.host.artifact_files(first.run_id)['rolling-history.json'])
        self.assertEqual([saved_run['head_sha'] for saved_run in history['runs']], [first.sha])
        self.assertEqual(measured(history['runs'][0]), {OWN}, 'the only measurement on this catalog')

        few = journey.pull_request()
        self.assertEqual(few.exit_code, 0, few.stdout)
        baseline = few.saved('comparison-input.json')['baseline']
        self.assertEqual([sample['head_sha'] for sample in baseline], [first.sha], 'base runs alone')
        self.assertEqual(measured(baseline[0]), {OWN})
        self.assertEqual(few.rows()[OWN]['baseline_n'], 1)
        self.assertTrue(placements(few)[OWN].cells[0].endswith(f'<br>baseline 1/{MINIMUM_SAMPLES}'),
                        'a sample below the minimum is shown as one')

        journey.repository.git('checkout', '--quiet', 'main')
        pushes = [first, *(journey.push() for _ in range(MINIMUM_SAMPLES - 1))]
        history = json.loads(journey.host.artifact_files(pushes[-1].run_id)['rolling-history.json'])
        heads = [push.sha for push in pushes]
        self.assertEqual([saved_run['head_sha'] for saved_run in history['runs']], heads)

        enough = journey.pull_request()
        self.assertEqual(enough.exit_code, 0, enough.stdout)
        baseline = enough.saved('comparison-input.json')['baseline']
        self.assertEqual([sample['head_sha'] for sample in baseline], heads, 'no pull request run enters')
        rows = enough.rows()
        self.assertEqual(rows[OWN]['baseline_n'], MINIMUM_SAMPLES)
        self.assertNotIn('baseline unavailable', rows[OWN].get('unavailable', ''), rows[OWN])
        self.assertNotRegex(placements(enough)[OWN].cells[0], fr'baseline \d+/{MINIMUM_SAMPLES}')
        unmeasured = shown() - {OWN}
        self.assertEqual({rows[identifier]['baseline_n'] for identifier in unmeasured}, {0})

    def test_a_fork_gets_a_summary_and_artifact_with_no_comment(self):
        journey = Journey(self)
        journey.push()
        job = journey.pull_request(head_repository='someone/fork')
        self.assertEqual(job.exit_code, 0, job.stdout)
        self.assertEqual(journey.host.comments, [])
        self.assertIn('someone/fork', job.summary)
        self.assertTrue((job.out / 'report.md').is_file())

    def test_a_refused_comment_and_no_baseline_are_diagnostics_and_never_fail_the_job(self):
        journey = Journey(self)
        journey.refuse(Refusal(r'issues/1/comments', 403, 99))
        journey.refuse(Refusal(r'artifacts\?', 403, 99))
        job = journey.pull_request()
        self.assertEqual(job.exit_code, 0, job.stdout)
        self.assertEqual(journey.host.comments, [])
        self.assertTrue(job.summary, 'the report still reaches the summary')


class Replayed(unittest.TestCase):
    """The release figures, over replayed telemetry: never a claim about a live pull request."""

    def enforced_over(self, journey):
        command = ('bash', '-c', 'echo "{\\"value\\": 5}" > "$ONEBUDGETSPEC_RESULT"')
        journey.add_budgets('demo/budgets.yaml', [FixtureBudget(BudgetId('demo-enforced'), command, threshold=1)])

    def test_check_time_over_its_line_is_report_only_with_its_flag_and_an_enforced_overrun_is_not(self):
        journey = Journey(self)
        for longest in (1000, 1100, 1200, 1300, 1400):
            journey.replaying(longest)
            self.assertEqual(journey.push().exit_code, 0)
        saved = json.loads(journey.host.artifact_files(journey.next_run)['rolling-history.json'])
        self.assertEqual(len(saved['runs']), 5, 'one rolling file, read and extended by each base push')
        journey.replaying(2400)
        over = journey.pull_request()
        self.assertEqual(over.exit_code, 0, over.stdout)
        row = over.rows()[CHECK_TIME]
        self.assertEqual((row['verdict'], row['current'], row['flag']), ('over', 2400, True), row)
        shown = placements(over)[CHECK_TIME]
        self.assertEqual(shown.section, 'attention')
        self.assertEqual(shown.cells[1], 'Over limit · flagged (report-only)')
        self.assertIn('2400', shown.cells[0].replace(',', ''))
        self.assertEqual(len(over.saved('comparison-input.json')['baseline']), 5)

        journey.repository.git('checkout', '--quiet', 'main')
        self.enforced_over(journey)
        journey.base = journey.repository.commit('an enforced budget')
        red = journey.pull_request()
        self.assertEqual(red.exit_code, 1, red.stdout)
        self.assertEqual(red.rows()['demo-enforced']['verdict'], 'over')
        self.assertEqual(red.rows()[CHECK_TIME]['verdict'], 'over')

    def test_a_within_figure_is_not_flagged_and_an_improvement_keeps_its_number_without_one(self):
        journey = Journey(self)
        for longest in (1000, 1100, 1200, 1300, 1400):
            journey.replaying(longest)
            journey.push()
        journey.replaying(400)
        row = journey.pull_request().rows()[CHECK_TIME]
        self.assertEqual((row['verdict'], row['current'], row['flag']), ('within', 400, False), row)

    def test_removing_the_label_enforces_the_overrun_through_the_generic_input(self):
        journey = Journey(self)
        journey.replaying(2400)
        # While the job also excludes `release-tail`, that label keeps this release figure report-only.
        path = journey.repository.path / 'budgets.yaml'
        path.write_text(path.read_text().replace('labels: [release-tail, report-only]', 'labels: [release-tail]'))
        journey.base = journey.repository.commit('the label removed')
        kept = journey.pull_request()
        self.assertEqual(kept.exit_code, 0, kept.stdout)
        self.assertEqual(kept.rows()[CHECK_TIME]['verdict'], 'over')
        self.assertEqual(placements(kept)[CHECK_TIME].section, 'attention')
        self.assertTrue(placements(kept)[CHECK_TIME].cells[1].endswith(' (report-only)'))

        # Excluding only `report-only`, the label alone decides. Only this budget stays, because
        # its release-tail siblings would then be enforced too, and error over these recordings.
        document = yaml.safe_load(path.read_text())
        document['budgets'] = [budget for budget in document['budgets'] if budget['id'] == CHECK_TIME]
        path.write_text(yaml.safe_dump(document))
        journey.base = journey.repository.commit('only this budget in the root file')
        inputs = job_inputs(**{'enforcement-exclude-labels': 'report-only'})
        enforced = journey.pull_request(inputs=inputs)
        self.assertEqual(enforced.exit_code, 1, enforced.stdout)
        self.assertEqual(enforced.rows()[CHECK_TIME]['verdict'], 'over')
        document['budgets'][0]['labels'].append('report-only')
        path.write_text(yaml.safe_dump(document))
        journey.base = journey.repository.commit('the label put back')
        reported = journey.pull_request(inputs=inputs)
        self.assertEqual(reported.exit_code, 0, reported.stdout)
        self.assertEqual(reported.rows()[CHECK_TIME]['verdict'], 'over')

    def test_the_registered_commands_measure_the_release_with_their_ordinary_verdicts(self):
        journey = Journey(self)
        journey.replaying(2400)
        tools = STATE / 'tools'
        if not (tools / 'onebudgetspec').exists():
            job = journey.run('push', journey.base)
            shutil.copytree(job.temp / 'onebudgetspec-action/tools', tools, dirs_exist_ok=True)
        environment = {'PATH': os.pathsep.join([str(tools), journey.path]), 'HOME': str(journey.root),
                       **journey.release_environment}
        figures = {}
        for longest, verdict in ((2400, 'over'), (1200, 'within')):
            journey.replaying(longest)
            environment['ONEVCS'] = journey.release_environment['ONEVCS']
            checked = subprocess.run(
                ['onebudgetspec', 'check', '--json', 'budgets.yaml', '--id', CHECK_TIME, '--id', JOB_COUNT,
                 '--id', CD_JOB_COUNT],
                cwd=journey.repository.path, env=environment, capture_output=True, text=True, check=False)
            results = {result['id']: result for result in json.loads(checked.stdout)['results']}
            self.assertEqual(results[CHECK_TIME]['verdict'], verdict, checked.stdout)
            self.assertEqual(results[CHECK_TIME]['actual'], longest)
            self.assertEqual(results[JOB_COUNT]['actual'], 5, checked.stdout)
            self.assertEqual(results[JOB_COUNT]['verdict'], 'within')
            self.assertEqual(results[CD_JOB_COUNT]['actual'], 6, 'the ci push run is left out')
            figures[longest] = {BudgetId(identifier): ReplayedFigure(result['actual'], result['threshold'], result['verdict'])
                                for identifier, result in results.items()}
        TELEMETRY.mkdir(exist_ok=True)
        recording = ReplayedFigures('replayed telemetry over the v0.13.0 recordings, never live CI', figures)
        (TELEMETRY / 'replayed-budget-figures.json').write_text(json.dumps(asdict(recording), indent=2))


class WholeJob(unittest.TestCase):
    """The workload the job's budget states: its figures are recorded, never asserted against a line."""

    def catalog(self):
        """60 rows as the real catalog divides them: the job's own, its run's pair, the left out, the saved."""
        budgets = [FixtureBudget(OWN, ('onebudgetspec-github-action', 'telemetry', '--metric', TIME_METRIC,
                                       '--input', 'report-job.json'),
                                 unit='seconds', labels=('ci-report',), threshold=UNJUDGED)]
        counted = 'echo "$ONEBUDGETSPEC_BUDGET_ID" >> "$JOURNEY_COUNTS"'
        read = 'gh api "repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/jobs" > /dev/null || exit 1'
        write = 'echo "{\\"value\\": 1}" > "$ONEBUDGETSPEC_RESULT"'
        for identifier in (CHECK_TIME, JOB_COUNT):
            budgets.append(FixtureBudget(identifier, ('bash', '-c', f'{counted}; {read}; {write}'), labels=('release-tail',)))
        for index in range(len(OMITTED)):
            budgets.append(FixtureBudget(BudgetId(f'left-out-{index}'), ('bash', '-c', f'{counted}; {write}'),
                                         labels=(OMIT_LABEL,)))
        while len(budgets) < ROWS:
            budgets.append(FixtureBudget(BudgetId(f'saved-{len(budgets)}'), ('tool-this-runner-lacks', 'telemetry.json')))
        return budgets

    def saved_results(self, identifiers):
        """A check's results of the stated size, each with a breakdown that does not compress away."""
        digest = hashlib.sha256()
        results = []
        for identifier in identifiers:
            pieces = []
            for _ in range(SAVED_ARTIFACT_BYTES // (32 * len(identifiers))):
                digest.update(b'.')
                pieces.append(digest.hexdigest())
            results.append(harness.check_result(identifier, 1, detail=''.join(pieces)))
        return harness.check_report(*results)

    def test_record_one_complete_job(self):
        journey = Journey(self)
        for path in journey.repository.git('ls-files', '*budgets.yaml').split():
            (journey.repository.path / path).unlink()
        budgets = self.catalog()
        identifiers = [budget.id for budget in budgets]
        saved = [identifier for identifier in identifiers if identifier.startswith('saved-')]
        journey.add_budgets('budgets.yaml', budgets)
        counts = journey.root / 'counts'
        journey.environment = {'JOURNEY_COUNTS': str(counts)}
        heads = [journey.repository.commit(f'base {index}') for index in range(COMMITS)]
        first = journey.run('push', heads[-1])
        self.assertEqual(first.exit_code, 0, first.stdout)
        journey.host.artifacts.clear()
        journey.host.runs.clear()
        # llmlint: ignore-block[modern_domain_modeling] The rolling file is the action's own format, seeded from a run it saved, so its next run reads what the action wrote.
        history = {'schema_version': 1, 'repository': harness.REPOSITORY, 'base_ref': 'main', 'runs': [
            {**first.saved('budget-run.json'), 'head_sha': head, 'run_id': str(500 + index), 'attempt': 1}
            for index, head in enumerate(heads)]}
        # llmlint: ignore-end[modern_domain_modeling]
        journey.host.add_run(519, heads[-1], minute=519)
        journey.host.attach(519, 'budget-results', zip_of('rolling-history.json', json.dumps(history)))

        journey.base = heads[-1]
        artifact = harness.zipped({'report.json': self.saved_results(saved)})
        journey.host.attach(journey.next_run + 1, 'budget-check-results-test-linux', artifact)
        counts.unlink(missing_ok=True)
        journey.host.clear_requests()
        journey.refuse(Refusal(r'artifacts\?', 429, 1, retry_after=1))
        pull_request = journey.pull_request()
        figures = [self.assert_whole(journey, pull_request, identifiers, saved, COMMITS)]

        journey.repository.git('checkout', '--quiet', '--force', 'main')
        merge = journey.repository.commit('the squash')
        journey.host.add_run(pull_request.run_id, pull_request.sha, event='pull_request', branch='change', minute=600)
        journey.host.merge_pull_request(merge, pull_request.sha)
        counts.unlink(missing_ok=True)
        journey.host.clear_requests()
        journey.refuse(Refusal(r'artifacts\?', 429, 1, retry_after=1))
        push = journey.run('push', merge)
        figures.append(self.assert_whole(journey, push, identifiers, saved, COMMITS))
        self.assertGreater(journey.host.count(fr'/runs/{pull_request.run_id}/artifacts'), 0, 'the merged run is read')
        self.assertFalse(counts.exists(), 'a figure the merged run holds runs no command')
        TELEMETRY.mkdir(exist_ok=True)
        # llmlint: ignore-block[modern_domain_modeling] This is the telemetry file the released reader reads, restated field by field in the action's own shape.
        worst = max(figures, key=lambda record: record['metrics'][TIME_METRIC]['value'])
        worst['metrics'][REST_METRIC] = max((record['metrics'][REST_METRIC] for record in figures),
                                            key=lambda metric: metric['value'])
        # llmlint: ignore-end[modern_domain_modeling]
        worst['metrics'][TIME_METRIC]['detail'] += f'; workload: {ROWS} rows, {COMMITS} commits, {len(artifact)} zipped saved-results bytes'
        (TELEMETRY / 'report-job.json').write_text(json.dumps(worst, indent=2))
        tools = STATE / 'tools'
        shutil.copytree(push.temp / 'onebudgetspec-action/tools', tools, dirs_exist_ok=True)

    def assert_whole(self, journey, job, identifiers, saved, commits):
        """What every budgets job of this workload shows, and its telemetry with the requests restated."""
        self.assertEqual(job.exit_code, 0, job.stdout)
        self.assertEqual(job.outcomes['final-upload'], 'success', job.stdout)
        rows = job.rows()
        self.assertEqual(set(rows), {identifier for identifier in identifiers if not identifier.startswith('left-out-')})
        self.assertEqual(len(rows) + len(OMITTED), ROWS)
        self.assertEqual({rows[identifier]['current'] for identifier in saved}, {1})
        self.assertLessEqual(len(job.saved('comparison-input.json')['baseline']), commits)
        figures = boundary_record(journey.host, job.saved('report-job.json'))
        self.assertEqual(figures['metrics'][REST_METRIC]['value'], journey.host.count(''))
        return figures


def boundary_record(host, telemetry):
    """The job's telemetry, with its request figure restated from what the double received."""
    # llmlint: ignore-block[modern_domain_modeling] This restates a metric inside the telemetry file the released reader reads, in the action's own shape.
    record = json.loads(json.dumps(telemetry))
    transfers = {service: host.transferred(service) for service in TRANSFER_SERVICES}
    metric = record['metrics'][REST_METRIC]
    metric['value'] = host.count('')
    metric['detail'] = f'{metric.get("detail", "")}; measured at the boundary, file transfers {transfers}'
    # llmlint: ignore-end[modern_domain_modeling]
    return record


def zip_of(name, content):
    import zipfile
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        archive.writestr(name, content)
    return buffer.getvalue()


def recorded_figures(tree, identifiers, over=()):
    """Replace each named budget's command in a copy of the tree with one reporting a recorded figure.

    Only the workload is stood in for: the id, labels, unit, direction and threshold are the production
    budget's, and each is what the target then judges. The figure sits on the threshold, or just past it
    for one named in `over`. Returns the figures, and the file the commands log each run to.
    """
    program = tree.parent / 'recorded-figure'
    log = tree.parent / 'recorded-figure-runs'
    figures_file = tree.parent / 'recorded-figures.json'
    program.write_text('#!/bin/sh\n'
                       f'echo "$ONEBUDGETSPEC_BUDGET_ID" >> "{log}"\n'
                       f'jq -c --arg id "$ONEBUDGETSPEC_BUDGET_ID" \'{{value: .[$id]}}\' "{figures_file}" '
                       '> "$ONEBUDGETSPEC_RESULT"\n')
    program.chmod(0o755)
    figures = {}
    for name in identifiers:
        path = tree / name
        # llmlint: ignore-block[modern_domain_modeling] This edits one field of a budgets file in place and writes it back whole; a model would drop every field the edit does not name.
        document = yaml.safe_load(path.read_text())
        for budget in document['budgets']:
            # The live lane saves nothing without a credential, and the action knows the job's own
            # budget by its command, whose telemetry the caller records.
            if budget['id'] in (LIVE_SESSION, OWN) or budget['id'] not in identifiers[name]:
                continue
            step = 1 if budget['direction'] == 'max' else -1
            figures[budget['id']] = budget['threshold'] + (step if budget['id'] in over else 0)
            budget['command'] = [str(program)]
        # llmlint: ignore-end[modern_domain_modeling]
        path.write_text(yaml.safe_dump(document))
    figures_file.write_text(json.dumps(figures))
    return RecordedFigures(figures, log)


def budget_ids(tree, name):
    return {budget['id'] for budget in yaml.safe_load((tree / name).read_text())['budgets']}


def host_environment(tree):
    """What a job's own step holds beyond its tools. Nothing of an outer Nx run reaches a nested one."""
    return {'PATH': os.environ['PATH'], 'HOME': str(tree.parent), 'NX_DAEMON': 'false', 'NX_TUI': 'false',
            'NX_NO_CLOUD': 'true', 'CI': '1', 'DBUS_SESSION_BUS_ADDRESS': 'disabled:',
            'UV_CACHE_DIR': os.environ.get('UV_CACHE_DIR', str(Path.home() / '.cache/uv'))}


def run_in(tree, command, **options):
    return subprocess.run(command, cwd=tree, env=host_environment(tree), capture_output=True, text=True, check=False,
                          timeout=900, **options)


def nx_budgets(tree, target):
    """One `budgets` target, as ci-job.sh runs it after its `test`, which already recorded the figure."""
    return run_in(tree, [str(ROOT / 'node_modules/.bin/nx'), 'run', target, '--excludeTaskDependencies', '--outputStyle=static'])


class SavedResults(unittest.TestCase):
    """A figure's whole path: target, Nx's outputs, ci.yml's upload, the action's download, the comment."""

    def saving_tree(self, over=()):
        """A copy of this tree whose Nx can run, and the budgets its saving targets judge recorded figures for."""
        journey = Journey(self)
        tree = journey.repository.path
        for link in ('node_modules', '.github/node_modules'):
            (tree / link).symlink_to(ROOT / link)
        identifiers = {name: budget_ids(tree, name) for name in SAVING_BUDGET_FILES}
        recorded = recorded_figures(tree, identifiers, over=over)
        journey.base = journey.repository.commit('recorded figures')
        # report-job:budgets reads with the release's own reader, already fetched for this run, over
        # the recording report-job:test made.
        shutil.copytree(TREE, tree / 'target/report-job' / TAG)
        recording = tree / '.github/reporting/.telemetry/report-job.json'
        recording.parent.mkdir()
        # llmlint: ignore[modern_domain_modeling] The telemetry file the released reader reads, in the action's own shape.
        recording.write_text(json.dumps({'schema_version': 1, 'labels': ['ci-report'], 'metrics': {
            TIME_METRIC: {'value': RECORDED_OWN_SECONDS, 'detail': 'recorded', 'unavailable': None}}}))
        return SavingTree(journey, recorded.figures, recorded.log)

    def upload(self, journey, job_name, run_id):
        """ci.yml's real upload step of `job_name`, by the real action, to the doubled results service."""
        step = ci_upload_step(job_name)
        self.assertEqual(step['with']['path'], 'target/budget-results/')
        uploader = harness.Job(journey.host, journey.repository, 'pull_request', 'head', run_id=run_id, action_path=TREE)
        self.addCleanup(uploader.close)
        # llmlint: ignore[modern_domain_modeling] This is the expression context the released harness's Expression reads, in its own shape.
        context = {'inputs': {}, 'github': uploader.github, 'steps': {}, 'runner': {'temp': str(uploader.temp)}}
        output = uploader.temp / 'upload-output'
        outcome, _ = uploader._artifact_action(step, harness.Expression(context), output)
        self.assertEqual(outcome, 'success', uploader.stdout)
        return journey.host.artifact_files(run_id, step['with']['name'])

    def artifact_figures(self, files):
        """Each budget's figure and verdict across an artifact's check reports."""
        results = [result for content in files.values() for result in json.loads(content)['results']]
        return {result['id']: SavedFigure(result['actual'], result['verdict']) for result in results}

    def another_jobs_checkout(self, journey):
        """`budgets` checks out afresh: neither a result nor what measured it reaches it."""
        shutil.rmtree(journey.repository.path / 'target')
        shutil.rmtree(journey.repository.path / '.github/reporting/.telemetry')

    def test_a_saved_figure_travels_from_its_target_to_the_row_and_on_into_the_base_history(self):
        journey, figures, log = self.saving_tree()
        tree = journey.repository.path

        # Each target saves, and a replayed hit restores the same bytes without measuring again.
        saved_files = {}
        for target, name in (CACHED_SAVING_TARGETS | UNCACHED_SAVING_TARGETS).items():
            ran = nx_budgets(tree, target)
            self.assertEqual(ran.returncode, 0, ran.stdout + ran.stderr)
            self.assertNotIn('local cache', ran.stdout, target)
            saved_files[name] = (tree / 'target/budget-results' / name).read_bytes()
        log.unlink()
        for target, name in CACHED_SAVING_TARGETS.items():
            (tree / 'target/budget-results' / name).unlink()
            replayed = nx_budgets(tree, target)
            self.assertEqual(replayed.returncode, 0, replayed.stdout + replayed.stderr)
            self.assertIn('local cache', replayed.stdout, target)
            self.assertEqual((tree / 'target/budget-results' / name).read_bytes(), saved_files[name], target)
        self.assertFalse(log.exists(), 'a cache hit measures nothing')

        # The release pull request's lint step, through its real recipe.
        pool = run_in(tree, ['just', 'pool-maintenance-budget'])
        self.assertEqual(pool.returncode, 0, pool.stdout + pool.stderr)
        pool_results = (tree / 'target/budget-results/pool-maintenance-budget-results.json').read_bytes()
        (tree / 'target/budget-results/pool-maintenance-budget-results.json').unlink()

        # test-linux's upload, of the restored files alone, and then lint's.
        run_id = journey.next_run + 1
        uploaded = self.upload(journey, 'test-linux', run_id)
        self.assertEqual({name: uploaded[name] for name in saved_files}, saved_files)
        self.assertEqual(set(uploaded), set(saved_files), 'what the targets saved and nothing else')
        shutil.rmtree(tree / 'target/budget-results')
        (tree / 'target/budget-results').mkdir()
        (tree / 'target/budget-results/pool-maintenance-budget-results.json').write_bytes(pool_results)
        uploaded_by_lint = self.upload(journey, 'lint', run_id)
        self.assertEqual(set(uploaded_by_lint), {'pool-maintenance-budget-results.json'})
        expected = self.artifact_figures({**uploaded, **uploaded_by_lint})
        self.assertEqual({identifier: saved.actual for identifier, saved in expected.items() if identifier != OWN},
                         {identifier: figure for identifier, figure in figures.items() if identifier not in OMITTED})
        self.assertIn(OWN, expected, 'report-job:budgets saved a figure, which the row must not take')
        self.assertNotIn(LIVE_SESSION, expected, 'no credential, so the live lane saved nothing')
        self.another_jobs_checkout(journey)

        log.unlink(missing_ok=True)
        journey.path = f'{log.parent}{os.pathsep}{journey.path}'
        pull_request = journey.pull_request()
        self.assertEqual(pull_request.exit_code, 0, pull_request.stdout)
        self.assertEqual(pull_request.outcomes['saved-results'], 'success', pull_request.stdout)
        rows = pull_request.rows()
        places = placements(pull_request)
        for identifier, saved in expected.items():
            if identifier in OMITTED or identifier == OWN:
                continue
            self.assertEqual((rows[identifier]['current'], rows[identifier]['verdict']), (saved.actual, saved.verdict), identifier)
            self.assertIn(f'{saved.actual:g}'.split('.')[0], places[identifier].cells[0].replace(',', ''), identifier)
        own_figure = pull_request.saved('report-job.json')['metrics'][TIME_METRIC]['value']
        self.assertEqual(rows[OWN]['current'], own_figure, "the job's own figure is its own")
        self.assertEqual(expected[OWN].actual, RECORDED_OWN_SECONDS)
        self.assertNotEqual(rows[OWN]['current'], RECORDED_OWN_SECONDS)
        self.assertFalse(log.exists(), 'a budget with a saved result runs no command here')

        self.assertNotIn('current', rows[LIVE_SESSION], rows[LIVE_SESSION])
        self.assertEqual(rows[LIVE_SESSION]['unavailable'], NO_SAVED_RESULT)
        self.assertEqual(places[LIVE_SESSION], Placement('not-measured', ('no result this run · baseline 0/5',)),
                         'listed with no figure')
        self.assertEqual(missing_reasons(pull_request)[LIVE_SESSION], NO_SAVED_RESULT)
        self.assertEqual(rows[CHECK_TIME]['current'], LONGEST_JOB_SECONDS)
        assert_left_out_and_counted(self, pull_request)
        self.assertIn(OMITTED_LINE, journey.host.comments[0]['body'])
        self.assertTrue(journey.host.artifact_files(pull_request.run_id))

        # The merge, whose push runs no test job: every figure in its history entry is that run's.
        journey.repository.git('checkout', '--quiet', 'main')
        merge = journey.repository.commit('the squash')
        journey.host.add_run(pull_request.run_id, pull_request.sha, event='pull_request', branch='change', minute=600)
        journey.host.merge_pull_request(merge, pull_request.sha)
        journey.host.clear_requests()
        push = journey.run('push', merge)
        self.assertEqual(push.exit_code, 0, push.stdout)
        self.assertEqual(journey.host.count(r'/runs/\d+/jobs'), 0, "the push's own command read no job")
        self.assertEqual(journey.host.comments[0]['body'].count(OMITTED_LINE), 1, 'it posts nothing')
        self.assertFalse(log.exists(), "those budgets' commands do not run in this job")
        taken = history_figures(push, merge)
        merged_figures = {result['id']: result['actual'] for result in pull_request.saved('budget-run.json')['results']['results']}
        for identifier, saved in expected.items():
            if identifier not in OMITTED and identifier != OWN:
                self.assertEqual(taken[identifier], saved.actual, identifier)
        self.assertEqual(taken[CHECK_TIME], merged_figures[CHECK_TIME])
        self.assertEqual(taken[JOB_COUNT], merged_figures[JOB_COUNT])
        self.assertFalse(set(taken) & OMITTED)

    def test_an_over_figure_is_saved_and_fails_the_report_while_an_unreadable_file_has_no_number(self):
        janitor = BudgetId('live-janitor-requests-per-run')
        journey, figures, _ = self.saving_tree(over={janitor})
        tree = journey.repository.path
        failed = nx_budgets(tree, 'onetaskgraph-live-janitor:budgets')
        self.assertEqual(failed.returncode, 1, failed.stdout + failed.stderr)
        run_id = journey.next_run + 1
        (tree / 'target/budget-results/broken.json').write_text('{ not a report')
        uploaded = self.upload(journey, 'test-linux', run_id)
        self.assertEqual(self.artifact_figures({'r': uploaded['onetaskgraph-live-janitor-budget-results.json']}),
                         {janitor: SavedFigure(figures[janitor], 'over')})
        self.another_jobs_checkout(journey)
        job = journey.pull_request()
        self.assertEqual(job.exit_code, 1, 'an enforced saved verdict fails the job')
        row = job.rows()[janitor]
        self.assertEqual((row['verdict'], row['current']), ('over', figures[janitor]))
        self.assertIn('saved results file', job.markdown() + job.stdout)
        self.assertIn('broken.json', job.markdown() + job.stdout)

    def test_no_saved_results_artifact_leaves_each_row_saying_so_and_fails_nothing(self):
        journey = Journey(self)
        job = journey.pull_request()
        self.assertEqual(job.exit_code, 0, job.stdout)
        self.assertEqual(job.outcomes['saved-results'], 'success', job.stdout)
        self.assertEqual(job.rows()[BudgetId('live-janitor-requests-per-run')]['unavailable'], NO_SAVED_RESULT)


class Enforcement(unittest.TestCase):
    """The budgets the report leaves out, each put to the real check that enforces it, over a recorded figure.

    Every entry runs twice: with the labels this tree gives the budget, and with the omit label removed, as
    the check saw it before. Selection and verdict have to be the same.
    """

    TAIL = 'budgets.yaml'
    SDK = 'sdks/python/budgets.yaml'
    PLAN = 'crates/onetaskgraph-github-projects-e2e/budgets.yaml'

    def checks(self):
        project = lambda name: json.loads((ROOT / name).read_text())
        release = yaml.safe_load((ROOT / '.github/workflows/release.yml').read_text())
        tail = release['jobs']['publish']['steps'][-1]['run'].strip()
        return (
            # publish's last step: red on an overrun, and nothing reads its result.
            EnforcingCheck('the release tail', self.TAIL, ['bash', '-c', tail], 1, False, OMITTED & {
                'onetaskgraph-cd-job-time', 'onetaskgraph-release-cycle-time', 'onetaskgraph-cd-job-count',
                'onetaskgraph-release-read-requests', 'onetaskgraph-pr-read-requests', 'onetaskgraph-nx-cache-use'}),
            # `just gate`'s check: a warning, because a development machine's timing varies too much.
            EnforcingCheck('the gate', self.TAIL, ['bash', 'scripts/gate-time.sh', '--check'], 0, True,
                           OMITTED & {'onetaskgraph-gate-time'}),
            # sdks/python:budgets-host: a warning, for a host-variable reading alone.
            EnforcingCheck('the SDK', self.SDK,
                           ['bash', '-c', project('sdks/python/project.json')['targets']['budgets-host']['options']['command']],
                           0, True, OMITTED & {'sdk-import-cpu-seconds'}),
            # The plan copy's budgets-host: a failure, a host budget not being host-variable.
            EnforcingCheck('the plan copy', self.PLAN,
                           ['bash', '-c', project('crates/onetaskgraph-github-projects-e2e/project.json')['targets']['budgets-host']['options']['command']],
                           1, False, OMITTED & {'github-plan-copy-real-seconds-unchanged', 'github-plan-copy-real-seconds-changed',
                                                'github-plan-copy-real-seconds-first'}),
        )

    def outcome(self, journey, check, *, with_label):
        tree = journey.repository.path
        path = tree / check.budgets_file
        original = path.read_text()
        self.addCleanup(lambda text=original: path.write_text(text))
        if not with_label:
            path.write_text(original.replace(f', {OMIT_LABEL}]', ']'))
        recorded_figures(tree, {check.budgets_file: budget_ids(tree, check.budgets_file)}, over=check.budgets)
        ran = run_in(tree, check.command)
        path.write_text(original)
        return CommandOutcome(ran.returncode, ran.stdout + ran.stderr)

    def test_each_left_out_budget_stays_selected_with_the_verdict_its_check_gave_before(self):
        journey = Journey(self)
        tree = journey.repository.path
        for link in ('node_modules', '.github/node_modules'):
            (tree / link).symlink_to(ROOT / link)
        covered = set()
        for check in self.checks():
            self.assertTrue(check.budgets, check.name)
            covered |= check.budgets
            before = self.outcome(journey, check, with_label=False)
            after = self.outcome(journey, check, with_label=True)
            self.assertEqual((after.status, before.status), (check.status, check.status),
                             check.name + after.output + before.output)
            for identifier in check.budgets:
                for outcome in (before, after):
                    line = next((line for line in outcome.output.splitlines() if line.startswith(f'budget {identifier}:')), '')
                    self.assertIn('— over', line, f'{check.name} {identifier}: {outcome.output}')
            if check.warns:
                self.assertIn('warning', after.output, check.name)
                self.assertIn('warning', before.output, check.name)
        self.assertEqual(covered, OMITTED)


class Contract(unittest.TestCase):
    def test_the_vendored_definition_is_the_release_itself(self):
        self.assertEqual((TREE / 'release-action.yml').read_bytes(), (HERE / f'action-{TAG}.yml').read_bytes())
        self.assertEqual((TREE / 'action.yml').read_bytes(), (HERE / f'action-{TAG}.yml').read_bytes())

    def test_the_real_host_comparison_reproduces_from_its_saved_input(self):
        saved = HERE / 'fixtures/demo-comparison-input.json'
        reader = STATE / 'reader'
        reader.mkdir(parents=True, exist_ok=True)
        with tarfile.open(next((TREE / 'dist').glob('*.tar.gz'))) as archive:
            archive.extractall(reader, filter='data')
        tools = STATE / 'tools'
        if not (tools / 'onebudgetspec').exists():
            journey = Journey(self)
            job = journey.run('push', journey.base)
            shutil.copytree(job.temp / 'onebudgetspec-action/tools', tools, dirs_exist_ok=True)
        path = os.pathsep.join([str(reader), str(tools), os.environ['PATH']])
        reproduced = subprocess.run(['onebudgetspec-github-action', 'reproduce', '--input', str(saved)],
                                    env={**os.environ, 'PATH': path}, capture_output=True, text=True, check=False)
        self.assertEqual(reproduced.returncode, 0, reproduced.stderr)
        self.assertIn('head: bf9fd2313e5118e0f0d6493651457bfd4d47457e', reproduced.stdout)
        rows = [*table_rows(section(reproduced.stdout, 'attention')), *table_rows(section(reproduced.stdout, 'within')),
                *listed(section(reproduced.stdout, 'details'), NOT_MEASURED)]
        self.assertEqual(len(rows), 60)
        self.assertEqual(len({name for name, _ in rows}), 60, 'each row once')
        # The core shows each id less this repository's own name as its prefix.
        self.assertIn(CHECK_TIME.removeprefix('onetaskgraph-'), {name for name, _ in rows})

    def test_the_job_names_only_inputs_the_action_declares_and_every_one_it_requires(self):
        declared = yaml.safe_load((TREE / 'action.yml').read_text())['inputs']
        given = set(action_step()['with'])
        self.assertEqual(given - set(declared), set())
        self.assertEqual({name for name, spec in declared.items() if spec.get('required')} - given, set())

    def test_exactly_the_stated_budgets_carry_the_label_and_the_job_names_both_inputs(self):
        labelled = set()
        listed = subprocess.run(['git', 'ls-files', '-co', '--exclude-standard'], cwd=ROOT, capture_output=True,
                                text=True, check=True).stdout.split()
        for name in listed:
            if Path(name).name == 'budgets.yaml' and not {'fixture', 'fixtures'} & set(Path(name).parts):
                labelled |= {budget['id'] for budget in yaml.safe_load((ROOT / name).read_text())['budgets']
                             if OMIT_LABEL in budget.get('labels', [])}
        self.assertEqual(labelled, OMITTED)
        self.assertEqual(job_inputs()['omit-labels'], OMIT_LABEL)
        self.assertEqual(job_inputs()['saved-results-artifacts'], 'budget-check-results-*')

    def test_the_budget_reads_a_metric_the_released_reader_serves(self):
        budget = yaml.safe_load((HERE / 'budgets.yaml').read_text())['budgets'][0]
        self.assertEqual(budget['command'][:2], ['onebudgetspec-github-action', 'telemetry'])
        self.assertEqual(budget['id'], OWN)
        self.assertEqual(budget['threshold'], 120)
        self.assertEqual(budget['labels'], ['ci-report'])
        metric = budget['command'][budget['command'].index('--metric') + 1]
        self.assertEqual(metric, TIME_METRIC)
        source = (TREE / 'crates/onebudgetspec-github-action/src/finish.rs').read_text()
        self.assertIn(f'"{metric}"', source + (TREE / 'crates/onebudgetspec-github-action/src/telemetry.rs').read_text())


if __name__ == '__main__':
    if platform.system() != 'Linux':
        print(f'report-job test: skipped on {platform.system()}: the job runs on ubuntu-latest alone')
        sys.exit(0)
    TAG, TREE, harness = released_action()
    unittest.main(verbosity=1)
