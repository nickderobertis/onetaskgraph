# llmlint: ignore-file[modern_domain_modeling] This journey speaks the released harness's own shapes (budget tuples, refusal dictionaries) and edits GitHub's recorded wire JSON; a model of either would restate a contract the action owns, beside the one it drives.
"""The `budgets` job of ci.yml, driven at GitHub's boundary with the action release it pins.

Each journey runs what the job's `uses:` pins with its `with:` inputs: action.yml's own steps, the
released driver and plugin archive, the published core wheel, the real `gh` and the real
`actions/upload-artifact` code, by the harness the action ships at that same tag. Only GitHub's
services, the release host and the package index are doubled. The repository under the job is a
copy of this tree, so every row of the real catalog is proven. Reaching the release, PyPI and
codeload once is why a cold run needs a network; the job runs on Linux, so its proof does too.

The figures of `onetaskgraph-ci-check-time` and `onetaskgraph-ci-job-count` are REPLAYED TELEMETRY,
never live CI: the real onevcs reads this repository's recordings of v0.13.0 through ONEVCS_GH, with
each pull request's jobs edited as `replay()` states.
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
from typing import NamedTuple
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
CHECK_TIME = 'onetaskgraph-ci-check-time'
JOB_COUNT = 'onetaskgraph-ci-job-count'
OWN = 'onetaskgraph-budget-report-command-seconds'
REST_METRIC = 'report-github-api-requests-per-run'
TIME_METRIC = 'report-command-seconds'
RUNNER_TARGETS = {'x86_64': 'x86_64-unknown-linux-gnu', 'aarch64': 'aarch64-unknown-linux-gnu'}
# The workload `## Budgets` states for the job's own budget, and the design's command reads.
ROWS = 100
COMMITS = 20
COMMANDS_READING_REST = 12
TRANSFER_SERVICES = ('artifact-service', 'blob-storage', 'object-storage', 'release-host', 'package-index')


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


def fetch(url):
    # llmlint: ignore[async_typed_clients_at_boundaries] A test's one-shot download of a pinned release asset, the way the released harness fetches its own; nothing here is concurrent or a service client.
    with urllib.request.urlopen(url, timeout=120) as answer:
        return answer.read()


def released_action():
    """The action at ci.yml's pin: its tree, its harness and its archive, fetched once."""
    tag = action_step()['uses'].split('@', 1)[1]
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
    return Released(tag, tree, harness)


TAG, TREE, harness = (None, None, None)


def real_onevcs():
    """The onevcs release.yml's tail installs, the one the replay's recordings were driven through."""
    pin = re.search(r'onevcs-cli@(\S+)', (ROOT / '.github/workflows/release.yml').read_text())[1]
    prefix = STATE / f'onevcs-{pin}'
    binary = prefix / 'node_modules/.bin/onevcs'
    if not binary.exists():
        subprocess.run(['npm', 'install', '--no-save', '--no-package-lock', '--no-audit', '--no-fund', '--prefix',
                        str(prefix), f'onevcs-cli@{pin}'], check=True, capture_output=True)
    return binary


def runner_bin(directory, record):
    """What a hosted runner's PATH holds for a budget command: node and cargo, no uv, bun, just or onevcs.

    `cargo` records that it was started: the runner really has one, and a budget naming saved
    telemetry must not start a build there.
    """
    directory.mkdir(parents=True, exist_ok=True)
    (directory / 'node').symlink_to(shutil.which('node'))
    cargo = directory / 'cargo'
    cargo.write_text(f'#!/bin/sh\necho "cargo $*" >> "{record}"\nexit 1\n')
    cargo.chmod(0o755)
    return f'{directory}{os.pathsep}/usr/bin{os.pathsep}/bin'


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
            ci = dict(body['workflow_runs'][0], id=999000001, name='ci', event='push', conclusion='success')
            body['workflow_runs'].append(ci)
            body['total_count'] = len(body['workflow_runs'])
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
        self.path = runner_bin(self.root / 'runner-bin', self.cargo_started)
        self.next_run = 1000
        self.environment = {}
        self.base = self.repository.commit('the tree')

    def add_budgets(self, relative, entries):
        self.repository.write(relative, harness.budgets_yaml(entries))

    def run(self, event_name, sha, inputs=None, **options):
        self.next_run += 1
        job = harness.Job(self.host, self.repository, event_name, sha, inputs=inputs or job_inputs(),
                          run_id=self.next_run, action_path=TREE, environment={
                              'PATH': self.path,
                              'GITHUB_REPOSITORY': harness.REPOSITORY,
                              'GITHUB_RUN_ID': str(self.next_run),
                              **self.environment}, **options)
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
        return self.run('pull_request', head, inputs=inputs, base_ref='main', register=False,
                        event=harness.event_pull_request(1, head, self.base, **event))

    def replaying(self, longest_seconds):
        """A budget command in this job sees a release and the real onevcs, over the recordings."""
        self.environment = {
            'ONEVCS': str(replay(Path(tempfile.mkdtemp(dir=self.root)), longest_seconds)),
            'ONEVCS_HOME': str(self.root / 'onevcs-home'),
            'GITHUB_REPOSITORY': 'nickderobertis/onetaskgraph',
            'GITHUB_REF_TYPE': 'tag',
            'GITHUB_REF_NAME': 'v0.13.0',
            'GH_TOKEN': 'replay-token',
        }


def table_row(markdown, budget):
    rows = [line for line in markdown.splitlines() if line.startswith('|') and f'/ {budget} ' in line]
    assert len(rows) == 1, (budget, rows)
    return rows[0]


def catalog():
    """Every production budget, from the same discovery rule the action applies."""
    found = {}
    listed = subprocess.run(['git', 'ls-files', '-co', '--exclude-standard'], cwd=ROOT, capture_output=True,
                            text=True, check=True).stdout.split()
    for name in listed:
        if Path(name).name == 'budgets.yaml' and not {'fixture', 'fixtures'} & set(Path(name).parts):
            for budget in yaml.safe_load((ROOT / name).read_text())['budgets']:
                found[budget['id']] = {**budget, 'file': name}
    return found


def expected_reason(budget):
    """Why this production budget cannot measure in the job, in the words of its row."""
    match budget['command']:
        case ['bash', 'scripts/actions-budget-figures.sh', 'cache-use', *_]:
            return 'no release: pass --release TAG'
        case ['bash', 'scripts/actions-budget-figures.sh', *_]:
            return 'onevcs is not on PATH'
        case ['uv', *_]:
            return 'needs uv'
        case [*_, named]:
            return f'reads {named}'


class ProductionCatalog(unittest.TestCase):
    def test_every_budget_has_one_row_the_job_stays_green_and_nothing_builds(self):
        journey = Journey(self)
        job = journey.pull_request()
        self.assertEqual(job.exit_code, 0, job.stdout)
        budgets = catalog()
        rows = job.rows()
        self.assertEqual(set(rows), set(budgets))
        run = job.saved('budget-run.json')
        verdicts = {result['id']: result['verdict'] for result in run['results']['results']}
        self.assertEqual(verdicts, {OWN: 'within'}, 'the one command that can run here')
        self.assertFalse(journey.cargo_started.exists(), 'a budget started a build on the runner')
        reasons = {entry['id']: entry['reason'] for entry in run['unavailable']}
        markdown = job.markdown()
        self.assertIn(job.sha, markdown)
        for identifier, budget in budgets.items():
            if identifier == OWN:
                continue
            self.assertIn(expected_reason(budget), reasons[identifier], identifier)
            row = table_row(markdown, identifier)
            self.assertIn('unavailable', row, row)
            self.assertIn(expected_reason(budget), row, row)
        self.assertIn('report-only', table_row(markdown, CHECK_TIME))

    def test_a_second_head_updates_the_one_comment_and_a_base_push_saves_the_same_results(self):
        journey = Journey(self)
        base = journey.push()
        self.assertEqual(base.exit_code, 0, base.stdout)
        self.assertEqual(journey.host.comments, [], 'a base push posts nothing')
        saved = journey.host.artifact_files(base.run_id)
        self.assertIn('budget-run.json', saved)
        # On this catalog only the job's own budget measures, after the main phase, and action
        # v0.1.1 starts a rolling file only from a main phase that measured something. So no
        # baseline accumulates, and the report says so rather than inventing one.
        self.assertNotIn('rolling-history.json', saved)
        self.assertIn('no rolling file saved', base.markdown() + base.summary + base.stdout)

        first = journey.pull_request()
        self.assertEqual(first.exit_code, 0, first.stdout)
        self.assertEqual(len(journey.host.comments), 1)
        self.assertIn(first.sha, journey.host.comments[0]['body'])
        self.assertEqual(first.saved('comparison-input.json')['baseline'], [], 'no PR-run substitute')
        self.assertTrue((first.out / 'budget-run.json').is_file(), 'the same result artifact')

        second = journey.pull_request()
        self.assertEqual(second.exit_code, 0, second.stdout)
        self.assertNotEqual(first.sha, second.sha)
        self.assertEqual(len(journey.host.comments), 1, 'one standing comment')
        body = journey.host.comments[0]['body']
        self.assertIn(f'head: {second.sha}', body)
        self.assertNotIn(f'head: {first.sha}', body)
        self.assertIn(first.sha, body, 'the earlier head stays as informational history')

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
        journey.host.refuse.append({'path': r'issues/1/comments', 'status': 403, 'times': 99})
        journey.host.refuse.append({'path': r'artifacts\?', 'status': 403, 'times': 99})
        job = journey.pull_request()
        self.assertEqual(job.exit_code, 0, job.stdout)
        self.assertEqual(journey.host.comments, [])
        self.assertTrue(job.summary, 'the report still reaches the summary')


class Replayed(unittest.TestCase):
    """The release figures, over replayed telemetry: never a claim about a live pull request."""

    def enforced_over(self, journey):
        journey.add_budgets('demo/budgets.yaml', [
            ('demo-enforced', ['bash', '-c', 'echo "{\\"value\\": 5}" > "$ONEBUDGETSPEC_RESULT"'], {'threshold': 1})])

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
        shown = table_row(over.markdown(), CHECK_TIME)
        self.assertIn('report-only', shown)
        self.assertIn('2400', shown)
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
        self.assertIn('report-only', table_row(kept.markdown(), CHECK_TIME))

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
                       **journey.environment}
        figures = {}
        for longest, verdict in ((2400, 'over'), (1200, 'within')):
            journey.replaying(longest)
            environment['ONEVCS'] = journey.environment['ONEVCS']
            checked = subprocess.run(
                ['onebudgetspec', 'check', '--json', 'budgets.yaml', '--id', CHECK_TIME, '--id', JOB_COUNT,
                 '--id', 'onetaskgraph-cd-job-count'],
                cwd=journey.repository.path, env=environment, capture_output=True, text=True, check=False)
            results = {result['id']: result for result in json.loads(checked.stdout)['results']}
            self.assertEqual(results[CHECK_TIME]['verdict'], verdict, checked.stdout)
            self.assertEqual(results[CHECK_TIME]['actual'], longest)
            self.assertEqual(results[JOB_COUNT]['actual'], 5, checked.stdout)
            self.assertEqual(results[JOB_COUNT]['verdict'], 'within')
            self.assertEqual(results['onetaskgraph-cd-job-count']['actual'], 6, 'the ci push run is left out')
            figures[str(longest)] = {identifier: {'actual': result['actual'], 'threshold': result['threshold'],
                                                  'verdict': result['verdict']}
                                     for identifier, result in results.items()}
        TELEMETRY.mkdir(exist_ok=True)
        (TELEMETRY / 'replayed-budget-figures.json').write_text(json.dumps(
            {'provenance': 'replayed telemetry over the v0.13.0 recordings, never live CI', 'figures': figures},
            indent=2))


class WholeJob(unittest.TestCase):
    """The workload the job's budget states: its figures are recorded, never asserted against a line."""

    def catalog(self):
        write = 'echo "{\\"value\\": 1}" > "$ONEBUDGETSPEC_RESULT"'
        counted = 'echo "$ONEBUDGETSPEC_BUDGET_ID" >> "$JOURNEY_COUNTS"'
        entries = [
            (OWN, ['onebudgetspec-github-action', 'telemetry', '--metric', TIME_METRIC, '--input', 'report-job.json'],
             {'unit': 'seconds', 'labels': ['ci-report'], 'threshold': 120}),
            ('not-on-this-runner', ['tool-this-runner-lacks'], {}),
            ('saved-telemetry-absent', ['bash', '-c', counted, 'target/absent/record.json'], {}),
            ('a-labelled-one', ['bash', '-c', f'{counted}; {write}'], {'labels': ['ci-report'], 'unit': 'percent',
                                                                       'direction': 'min', 'threshold': 0}),
            ('an-elapsed-one', ['bash', '-c', counted], {'measure': 'elapsed', 'unit': 'seconds'}),
        ]
        for index in range(COMMANDS_READING_REST):
            entries.append((f'reads-rest-{index}', ['bash', '-c', f'{counted}; gh api repos/owner/repo/commits/{index}'
                                                    f' > /dev/null || exit 1; {write}'], {'unit': 'requests'}))
        while len(entries) < ROWS:
            entries.append((f'local-{len(entries)}', ['bash', '-c', f'{counted}; {write}'], {'unit': 'bytes'}))
        return harness.budgets_yaml(entries), [entry[0] for entry in entries]

    def test_record_one_complete_job(self):
        journey = Journey(self)
        for path in journey.repository.git('ls-files', '*budgets.yaml').split():
            (journey.repository.path / path).unlink()
        text, identifiers = self.catalog()
        journey.repository.write('budgets.yaml', text)
        counts = journey.root / 'counts'
        journey.environment = {'JOURNEY_COUNTS': str(counts)}
        heads = [journey.repository.commit(f'base {index}') for index in range(COMMITS)]
        first = journey.run('push', heads[-1])
        self.assertEqual(first.exit_code, 0, first.stdout)
        journey.host.artifacts.clear()
        journey.host.runs.clear()
        history = {'schema_version': 1, 'repository': harness.REPOSITORY, 'base_ref': 'main', 'runs': [
            {**first.saved('budget-run.json'), 'head_sha': head, 'run_id': str(500 + index), 'attempt': 1}
            for index, head in enumerate(heads)]}
        journey.host.add_run(519, heads[-1], minute=519)
        journey.host.attach(519, 'budget-results', zip_of('rolling-history.json', json.dumps(history)))

        journey.base = heads[-1]
        figures = []
        for label, start in (('pull_request', journey.pull_request), ('push', lambda: journey.run(
                'push', heads[-1], attempt=2, register=False))):
            counts.unlink(missing_ok=True)
            journey.host.clear_requests()
            journey.host.refuse.append({'path': r'artifacts\?', 'status': 429, 'times': 1, 'retry_after': 1})
            job = start()
            self.assertEqual(job.exit_code, 0, job.stdout)
            self.assertEqual(job.outcomes['final-upload'], 'success', job.stdout)
            self.assertEqual(len(job.rows()), ROWS)
            self.assertEqual(len(job.saved('comparison-input.json')['baseline']), COMMITS - (label == 'push'))
            ran = counts.read_text().split()
            self.assertEqual(sorted(ran), sorted(set(ran)), 'each command once')
            self.assertEqual(set(ran), set(identifiers) - {OWN, 'not-on-this-runner', 'saved-telemetry-absent'})
            reasons = {entry['id']: entry['reason'] for entry in job.saved('budget-run.json')['unavailable']}
            self.assertIn('tool-this-runner-lacks', reasons['not-on-this-runner'])
            self.assertIn('target/absent/record.json', reasons['saved-telemetry-absent'])
            self.assertEqual(set(reasons), {'not-on-this-runner', 'saved-telemetry-absent'})
            figures.append(boundary_record(journey.host, job.saved('report-job.json')))
            self.assertEqual(figures[-1]['metrics'][REST_METRIC]['value'], journey.host.count(''))
        TELEMETRY.mkdir(exist_ok=True)
        worst = max(figures, key=lambda record: record['metrics'][TIME_METRIC]['value'])
        worst['metrics'][REST_METRIC] = max((record['metrics'][REST_METRIC] for record in figures),
                                            key=lambda metric: metric['value'])
        (TELEMETRY / 'report-job.json').write_text(json.dumps(worst, indent=2))
        tools = STATE / 'tools'
        shutil.copytree(job.temp / 'onebudgetspec-action/tools', tools, dirs_exist_ok=True)


def boundary_record(host, telemetry):
    """The job's telemetry, with its request figure restated from what the double received."""
    record = json.loads(json.dumps(telemetry))
    transfers = {service: host.transferred(service) for service in TRANSFER_SERVICES}
    metric = record['metrics'][REST_METRIC]
    metric['value'] = host.count('')
    metric['detail'] = f'{metric.get("detail", "")}; measured at the boundary, file transfers {transfers}'
    return record


def zip_of(name, content):
    import zipfile
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        archive.writestr(name, content)
    return buffer.getvalue()


class Contract(unittest.TestCase):
    def test_the_vendored_definition_is_the_release_itself(self):
        self.assertEqual((TREE / 'release-action.yml').read_bytes(), (HERE / f'action-{TAG}.yml').read_bytes())
        self.assertEqual((TREE / 'action.yml').read_bytes(), (HERE / f'action-{TAG}.yml').read_bytes())

    def test_the_real_host_comparison_reproduces_from_its_saved_input(self):
        # Saved from run 42169097213 of throwaway pull request #3904: 60 rows at head bf9fd231.
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
        rows = [line for line in reproduced.stdout.splitlines() if line.startswith('| ') and ' / ' in line]
        self.assertEqual(len(rows), 60)
        self.assertIn('onetaskgraph-ci-check-time', reproduced.stdout)

    def test_the_job_names_only_inputs_the_action_declares_and_every_one_it_requires(self):
        declared = yaml.safe_load((TREE / 'action.yml').read_text())['inputs']
        given = set(action_step()['with'])
        self.assertEqual(given - set(declared), set())
        self.assertEqual({name for name, spec in declared.items() if spec.get('required')} - given, set())

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
