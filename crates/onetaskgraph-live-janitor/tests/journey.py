"""Real janitor subprocess journeys against a fully paginated loopback GitHub."""
import copy
import datetime
import http.server
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import threading
import unittest
from enum import StrEnum
from typing import NamedTuple, NewType, TypedDict, cast

from urllib.parse import parse_qs, urlparse, urlencode

Repository = NewType('Repository', str)
IssueId = NewType('IssueId', str)
ItemId = NewType('ItemId', str)
LabelName = NewType('LabelName', str)
RunId = NewType('RunId', int)
Token = NewType('Token', str)

JANITOR = (Path(__file__).resolve().parents[1] / 'src/lib.rs').read_text()
# The janitor's one declaration of GitHub's run statuses, read rather than restated.
Status = StrEnum('Status', [(re.sub(r'(?<!^)(?=[A-Z])', '_', variant).upper(), spelled) for variant, spelled in re.findall(r'Self::(\w+) => "([a-z_]+)",', JANITOR.split('fn spelling(self)', 1)[1].split('fn read(', 1)[0])])
assert {s.value for s in Status} == {'completed', 'queued', 'in_progress', 'waiting', 'requested', 'pending'}, 'RunStatus::spelling changed; update the journeys to the new vocabulary'

class Issue(TypedDict):
    node_id: IssueId
    title: str

class Label(TypedDict):
    name: LabelName

class RepositoryRecord(TypedDict):
    nameWithOwner: Repository

class Content(TypedDict, total=False):
    __typename: str
    title: str
    repository: RepositoryRecord

class Item(TypedDict):
    id: ItemId
    content: Content

class Run(TypedDict, total=False):
    id: RunId
    created_at: str

class Allowance(TypedDict):
    limit: int
    remaining: int
    reset: int

class Request(NamedTuple):
    method: str
    target: str

class Authentication(NamedTuple):
    target: str
    token: Token

class Board(NamedTuple):
    owner: object
    number: object

class Deletion(StrEnum):
    ISSUE = 'issue'
    ITEM = 'item'
    LABEL = 'label'

class IssueDelete(NamedTuple):
    id: IssueId

class ItemDelete(NamedTuple):
    board: str
    id: ItemId

class LabelDelete(NamedTuple):
    path: str

class Write(NamedTuple):
    method: str
    target: IssueDelete | ItemDelete | LabelDelete

class OriginField(TypedDict):
    id: str
    name: str

# The board's own field, which no cleanup may touch.
ORIGIN_FIELD: OriginField = {'id': 'origin', 'name': 'onetaskgraph.origin'}

class FailedPage(NamedTuple):
    endpoint: str
    page: int

ROOT = Path(__file__).resolve().parents[3]
BINARY = os.environ.get('JANITOR_BINARY', str(ROOT / 'target/debug/onetaskgraph-live-janitor'))
# Read the shared declarations, rather than introducing another nomination or prefix.
SOURCE = (ROOT / 'crates/onetaskgraph-github-live/src/lib.rs').read_text()

def declaration(name):
    return re.search(rf'pub const {name}: &str = "([^"]+)"', SOURCE)[1]

BOARD_OWNER = declaration('BOARD_OWNER')
# The plugin's document prefix, read from its one declaration like the names above.
DESIGN = re.search(r'pub const DESIGN_TITLE_PREFIX: &str = "([^"]+)"', (ROOT / 'crates/onetaskgraph-github-projects/src/lib.rs').read_text())[1]
BOARD_NUMBER = int(re.search(r'pub const BOARD_NUMBER: u32 = (\d+);', SOURCE)[1])
# The entry point's exit statuses, as its module documentation states them.
FAILED, REFUSED, DECLINED = 1, 2, 3

CORE = Repository(declaration('CORE_REPOSITORY'))
SCRATCH = Repository(declaration('SCRATCH_REPOSITORY'))
TITLE = declaration('ARTIFACT_PREFIX')
LABEL = declaration('LABEL_PREFIX')
CUTOVER = int(re.search(r'pub const CUTOVER_MICROS: u64 = ([\d_]+)', (ROOT / 'crates/onetaskgraph-live-janitor/src/lib.rs').read_text())[1].replace('_', ''))
DAY = 86_400_000_000
RestShape = dict[str, 'RestShape | None']

class RestOperation(TypedDict):
    operation_id: str
    method: str
    path: str
    query: dict[str, list[str] | None]
    status: int
    response: RestShape | None

# GitHub's REST contract for every operation the janitor sends, reduced from GitHub's
# published description; fixtures/rest-operations.json says from which commit and when.
REST: list[RestOperation] = cast(list[RestOperation], json.loads((Path(__file__).resolve().parent / 'fixtures/rest-operations.json').read_text())['operations'])

def rest_operation(method: str, path: str) -> RestOperation | None:
    for operation in REST:
        if operation['method'] == method and re.fullmatch(re.sub(r'\{[^/]+\}', '[^/]+', operation['path']), path):
            return operation
    return None

def unpinned_request(method, target):
    """Why GitHub's description does not admit this request, or None when it does."""
    parsed = urlparse(target)
    operation = rest_operation(method, parsed.path)
    if operation is None:
        return f'{method} {parsed.path} is no operation GitHub describes'
    for name, values in parse_qs(parsed.query).items():
        if name not in operation['query']:
            return f'{method} {parsed.path} sends {name}, which GitHub does not describe'
        allowed = operation['query'][name]
        if allowed is not None and any(value not in allowed for value in values):
            return f'{method} {parsed.path} sends {name}={values}, outside {allowed}'
    return None

def unpinned_fields(shape: RestShape | None, value: object, where: str = 'response') -> str | None:
    """Why GitHub's description has no such field in this body, or None when it has every one."""
    match value:
        case list():
            if shape is None or '[]' not in shape:
                return f'{where} is an array GitHub does not describe'
            if shape['[]'] is not None:
                return next(filter(None, (unpinned_fields(shape['[]'], member, f'{where}[{n}]') for n, member in enumerate(value))), None)
        case dict():
            if shape is None or '[]' in shape:
                return f'{where} is an object GitHub does not describe'
            for name, member in value.items():
                if name not in shape:
                    return f'{where}.{name} is no field GitHub describes'
                if shape[name] is not None:
                    found = unpinned_fields(shape[name], member, f'{where}.{name}')
                    if found:
                        return found
    return None

NOW = CUTOVER + 2 * DAY

class Github:
    def __init__(self):
        self.issues: dict[Repository, list[Issue]] = {CORE: [], SCRATCH: []}
        self.labels: dict[Repository, list[Label]] = {CORE: [], SCRATCH: []}
        self.items: list[Item] = []
        # Malformed API values are intentional fixtures alongside valid statuses.
        self.status: dict[RunId, Status | str | int | None] = {}
        self.owners: dict[Status, list[Run]] = {s: [] for s in Status if s != Status.COMPLETED}
        self.requests: list[Request] = []
        self.writes: list[Write] = []
        self.fail = None
        self.repeat = None
        self.status_reads = {}
        self.change_after = None
        self.inject = None
        self.allowance = 5000
        self.owner_reads = 0
        self.owner_change_after = None
        self.owner_fail_after = None
        self.undecodable = None
        self.page_size = 100
        self.write_inject = None
        self.invalid_json = None
        self.authentication: list[Authentication] = []
        self.fail_page = None
        self.board_invalid_json = False
        self.board_fail_after = None
        self.board_reads = 0
        self.owner_inject = None
        self.link_override = None
        self.total_count = None
        self.omit_total_count = False
        self.board_transform = None
        self.delete_response: dict[Deletion, object] = {}
        self.boards: set[Board] = set()
        self.label_delete_status: int | None = None
        self.delete_failure: Deletion | None = None
        self.allowance_record = None
        self.origin_field: OriginField = copy.deepcopy(ORIGIN_FIELD)
        self.origin_values: dict[ItemId, str] = {}
        # Every way a request or a served body departed from GitHub's description.
        self.unpinned: list[str] = []

    def pin(self, method, target):
        found = unpinned_request(method, target)
        if found:
            self.unpinned.append(found)
        return rest_operation(method, urlparse(target).path)

    def artifact(self, repository: Repository, stamp: str, design: bool = False, run: int | None = None) -> IssueId:
        identity = IssueId(f'{repository}:{len(self.issues[repository])}')
        title = (DESIGN if design else '') + TITLE + stamp
        issue = {'node_id': identity, 'title': title}
        self.issues[repository].append(issue)
        self.items.append({'id': ItemId('item:' + identity), 'content': {'__typename': 'Issue', 'title': title, 'repository': {'nameWithOwner': repository}}})
        if not any(v['name'] == LABEL + stamp for v in self.labels[repository]):
            self.labels[repository].append({'name': LabelName(LABEL + stamp)})
        if run is not None:
            self.status[RunId(run)] = Status.COMPLETED
        return identity

    def serve(self):
        state = self
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            # Set once a request reaches the stand-in's ordinary answer, so a fault a
            # journey injects on purpose is not held to GitHub's description.
            operation = None

            def reply(self, value, status=200, link=None):
                if self.operation and 200 <= status < 300:
                    if self.operation['response'] is None:
                        if value is not None:
                            state.unpinned.append(f"{self.operation['path']} answers no body")
                    else:
                        found = unpinned_fields(self.operation['response'], value)
                        if found:
                            state.unpinned.append(f"{self.operation['path']}: {found}")
                body = json.dumps(value).encode() if value is not None else b''
                self.send_response(status)
                if link:
                    self.send_header('Link', link)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                parsed = urlparse(self.path)
                query = parse_qs(parsed.query)
                path = parsed.path
                state.requests.append(Request('GET', self.path))
                operation = state.pin('GET', self.path)
                state.authentication.append(Authentication(self.path, Token(self.headers.get('Authorization', ''))))
                page = int(query.get('page', ['1'])[0])
                if state.fail_page and state.fail_page[0] in self.path and page >= state.fail_page[1]:
                    self.reply({}, 500)
                    return
                if state.invalid_json and state.invalid_json in self.path:
                    self.send_response(200)
                    self.send_header('Content-Length', '1')
                    self.end_headers()
                    self.wfile.write(b'{')
                    return
                if state.undecodable and state.undecodable in self.path:
                    self.reply({'unexpected': True})
                    return
                if state.fail and state.fail in self.path:
                    self.reply({}, 500)
                    return
                # A malformed allowance is a fault the journey injects, like those above.
                self.operation = operation if state.allowance_record is None else None
                match path.split('/'):
                    case ['', 'rate_limit']:
                        record: Allowance = {'limit': 5000, 'remaining': state.allowance, 'reset': 2000000000}
                        self.reply({'resources': {r: state.allowance_record if state.allowance_record is not None else record for r in ('core', 'graphql')}})
                        return
                    case ['', 'repos', _, _, 'actions', 'runs', spelled]:
                        run = RunId(int(spelled))
                        state.status_reads[run] = state.status_reads.get(run, 0) + 1
                        status = state.status.get(run)
                        if state.change_after and state.status_reads[run] > state.change_after:
                            status = Status.IN_PROGRESS
                        if state.inject:
                            callback, state.inject = state.inject, None
                            callback()
                        self.reply({'status': status} if status is not None else {}, 200 if status is not None else 404)
                        return
                    case ['', 'repos', _, _, 'actions', 'workflows', _, 'runs']:
                        state.owner_reads += 1
                        if state.owner_inject and state.owner_reads == 6:
                            callback, state.owner_inject = state.owner_inject, None
                            callback()
                        if state.owner_fail_after and state.owner_reads > state.owner_fail_after:
                            self.reply({}, 500)
                            return
                        if state.owner_change_after and state.owner_reads > state.owner_change_after:
                            state.owners[Status.QUEUED] = [{'created_at': '2020-01-01T00:00:00Z'}]
                        nodes = state.owners[Status(query['status'][0])]
                        field = 'workflow_runs'
                    case ['', 'repos', owner, name, 'issues' | 'labels' as kind]:
                        repository = Repository(f'{owner}/{name}')
                        field = None
                        nodes = state.issues[repository] if kind == 'issues' else state.labels[repository]
                    case _:
                        self.reply({'message': 'Not Found'}, 404)
                        return
                if state.repeat and state.repeat in path:
                    page = 1
                count = len(nodes)
                nodes = copy.deepcopy(nodes[(page - 1) * state.page_size:page * state.page_size])
                query['page'] = [str(page + 1)]
                next_url = path + '?' + urlencode(query, doseq=True)
                link = f'<http://127.0.0.1:{self.server.server_port}{next_url}>; rel="next"' if page * state.page_size < count else None
                if state.link_override is not None:
                    filters = urlencode({k: v for k, v in query.items() if k != 'page'}, doseq=True)
                    link = state.link_override.format(origin=f'http://127.0.0.1:{self.server.server_port}', path=path, filters=filters)
                value = {field: nodes} if field else nodes
                # GitHub states a run listing's whole size on every page.
                if field and not state.omit_total_count:
                    value['total_count'] = state.total_count if state.total_count is not None else count
                self.reply(value, link=link)

            def do_POST(self):
                payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                query, variables = payload['query'], payload['variables']
                state.requests.append(Request('POST', query))
                state.authentication.append(Authentication('/graphql', Token(self.headers.get('Authorization', ''))))
                match query:
                    case str() if query.startswith('query'):
                        board = Board(variables.get('owner'), variables.get('number'))
                        state.boards.add(board)
                        # The stand-in serves the nominated board alone.
                        if board != Board(BOARD_OWNER, BOARD_NUMBER):
                            self.reply({'errors': [{'message': 'Could not resolve to a ProjectV2'}]})
                            return
                        page = int(variables['after'] or 0)
                        state.board_reads += 1
                        if state.board_invalid_json:
                            self.send_response(200)
                            self.send_header('Content-Length', '1')
                            self.end_headers()
                            self.wfile.write(b'{')
                            return
                        if state.fail == 'board' or (state.board_fail_after and state.board_reads > state.board_fail_after):
                            self.reply({})
                            return
                        nodes = copy.deepcopy(state.items[page:page + 100])
                        end = str(page + 100) if state.repeat != 'board' else '100'
                        value = {'data': {'board': {'projectV2': {'id': 'board1', 'items': {'nodes': nodes, 'pageInfo': {'hasNextPage': page + 100 < len(state.items), 'endCursor': end}}}}}}
                        if state.board_transform:
                            state.board_transform(value)
                        self.reply(value)
                        return
                    case str() if 'deleteProjectV2Field' in query:
                        state.origin_field.clear()
                        state.origin_values.clear()
                        self.reply({'data': {'deleteProjectV2Field': {'deletedFieldId': 'origin'}}})
                        return
                    case str() if 'updateProjectV2ItemFieldValue' in query or 'clearProjectV2ItemFieldValue' in query:
                        item = ItemId(variables['input']['itemId'])
                        field_operation = 'clearProjectV2ItemFieldValue' if 'clearProjectV2ItemFieldValue' in query else 'updateProjectV2ItemFieldValue'
                        if field_operation == 'clearProjectV2ItemFieldValue':
                            state.origin_values.pop(item, None)
                        else:
                            state.origin_values[item] = variables['input']['value']['text']
                        self.reply({'data': {field_operation: {'projectV2Item': {'id': item}}}})
                        return
                    case _:
                        operation = Deletion.ISSUE if 'deleteIssue' in query else Deletion.ITEM
                        deletion = variables['input']
                        identity = deletion['issueId'] if operation is Deletion.ISSUE else deletion['itemId']
                        state.writes.append(Write('POST', IssueDelete(IssueId(identity)) if operation is Deletion.ISSUE else ItemDelete(deletion['projectId'], ItemId(identity))))
                        if state.write_inject:
                            callback, state.write_inject = state.write_inject, None
                            callback()
                        if state.delete_failure == operation:
                            self.reply({}, 500)
                            return
                        if operation in state.delete_response:
                            self.reply(state.delete_response[operation])
                            return
                        if operation is Deletion.ISSUE:
                            owner = next(repository for repository, issues in state.issues.items() if any(v['node_id'] == identity for v in issues))
                            state.issues[owner] = [v for v in state.issues[owner] if v['node_id'] != identity]
                            state.items = [v for v in state.items if v['id'] != 'item:' + identity]
                            state.origin_values.pop(ItemId('item:' + identity), None)
                            self.reply({'data': {'deleteIssue': {'repository': {'nameWithOwner': owner}}}})
                        else:
                            state.items = [v for v in state.items if v['id'] != identity]
                            state.origin_values.pop(ItemId(identity), None)
                            self.reply({'data': {'deleteProjectV2Item': {'deletedItemId': identity}}})

            def do_DELETE(self):
                state.requests.append(Request('DELETE', self.path))
                operation = state.pin('DELETE', self.path)
                state.authentication.append(Authentication(self.path, Token(self.headers.get('Authorization', ''))))
                state.writes.append(Write('DELETE', LabelDelete(self.path)))
                if state.label_delete_status is not None:
                    self.reply(None, state.label_delete_status)
                    return
                if state.delete_failure is Deletion.LABEL:
                    self.reply({}, 500)
                    return
                parts = self.path.split('/')
                repository = '/'.join(parts[2:4])
                state.labels[repository] = [v for v in state.labels[repository] if v['name'] != parts[-1]]
                self.operation = operation
                self.reply(None, operation['status'] if operation else 204)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        return server, thread

    def run(self, now=NOW, overrides=None, arguments=None):
        server, thread = self.serve()
        environment = dict(os.environ, GH_PROJECTS_OWNER='nickderobertis', GH_PROJECTS_NUMBER='1', GH_PROJECTS_REPOSITORY=SCRATCH, GH_PROJECTS_LEGACY_REPOSITORY=CORE, GH_PROJECTS_TOKEN='offline-write', GITHUB_TOKEN='offline-actions')
        environment.update(overrides or {})
        for name, value in list(environment.items()):
            if value is None:
                del environment[name]
        command = [BINARY] + (arguments if arguments is not None else [ '--loopback', f'http://127.0.0.1:{server.server_port}', str(now), '--virtual-clock'])
        command = [argument.replace('{origin}', f'http://127.0.0.1:{server.server_port}').replace('{host}', f'127.0.0.1:{server.server_port}') for argument in command]
        try:
            result = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=30)
        finally:
            server.shutdown()
            thread.join()
            server.server_close()
        # Every journey holds the janitor and this stand-in to GitHub's description.
        if self.unpinned:
            raise AssertionError('departs from fixtures/rest-operations.json: ' + '; '.join(sorted(set(self.unpinned))))
        return result


def workload(scale=1):
    state = Github()
    for n in range(194 * scale):
        state.artifact(CORE, f'1-{n % (48 * scale) + 1}-{CUTOVER - DAY - n}', design=n % 7 == 0)
    # One label per leaked run, independent of issue stamp times.
    state.labels[CORE] = [{'name': LABEL + f'1-{n + 1}-{CUTOVER - DAY}'} for n in range(48 * scale)]
    for n in range((356 if scale == 1 else 360)):
        state.issues[CORE].append({'node_id': f'ordinary:{n}', 'title': f'Feature {n}'})
    for n in range(6 * scale):
        state.artifact(SCRATCH, f'ci-{n // 6 + 1}-1-{CUTOVER - DAY - n}', run=n // 6 + 1)
    # A cancelled run leaves one label shared by its six issues.
    state.labels[SCRATCH] = [{'name': LabelName(LABEL + f'ci-{n + 1}-1-{CUTOVER - DAY - 6 * n}')} for n in range(scale)]
    state.artifact(SCRATCH, f'ci-999-1-{CUTOVER - DAY}', run=999)
    state.status[RunId(999)] = Status.IN_PROGRESS
    state.artifact(SCRATCH, f'1-1-{CUTOVER - DAY}')
    state.issues[SCRATCH].append({'node_id': 'scratch-ordinary', 'title': 'Ordinary scratch feature'})
    state.labels[SCRATCH].append({'name': 'ordinary'})
    state.labels[CORE].append({'name': 'ordinary'})
    state.items.append({'id': 'draft', 'content': {'__typename': 'DraftIssue'}})
    return state

class Journeys(unittest.TestCase):
    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)

    def assert_failed(self, result):
        self.assertEqual(result.returncode, FAILED, result.stderr)
        self.assertIn('janitor failed:', result.stderr)

    def test_entry_refuses_invalid_configuration_without_requests(self):
        for name in ('GH_PROJECTS_OWNER', 'GH_PROJECTS_NUMBER', 'GH_PROJECTS_REPOSITORY', 'GH_PROJECTS_LEGACY_REPOSITORY', 'GH_PROJECTS_TOKEN', 'GITHUB_TOKEN'):
            state = Github()
            result = state.run(overrides={name: None})
            self.assertEqual(result.returncode, REFUSED, result.stderr)
            self.assertIn(name, result.stderr)
            self.assertFalse(state.requests)
        for name in ('GH_PROJECTS_OWNER', 'GH_PROJECTS_NUMBER', 'GH_PROJECTS_REPOSITORY', 'GH_PROJECTS_LEGACY_REPOSITORY'):
            state = Github()
            result = state.run(overrides={name: 'other'})
            self.assertEqual(result.returncode, REFUSED, result.stderr)
            self.assertIn(name, result.stderr)
            self.assertFalse(state.requests)
        # Only a bare HTTP origin on the IPv4 loopback may stand in for GitHub, so no offline
        # argument can hand either token to another host or smuggle anything into the URL.
        for origin in ('https://example.com', 'http://example.com/', 'https://{host}/',
                       'http://user:secret@{host}/', 'http://{host}/?q=1',
                       'http://{host}/#fragment', 'http://{host}/api/', 'not a url'):
            state = Github()
            result = state.run(arguments=['--loopback', origin, str(NOW)])
            self.assertEqual(result.returncode, REFUSED, (origin, result.stderr))
            self.assertFalse(state.requests, origin)
        for arguments in (['--wrong'], ['--loopback', '{origin}', 'bad-clock'], ['--loopback', '{origin}', str(NOW), '--other']):
            state = Github()
            self.assertEqual(state.run(arguments=arguments).returncode, REFUSED)
            self.assertFalse(state.requests)

    def test_real_clock_paces_writes(self):
        state = Github()
        state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY}', run=123)
        result = state.run(arguments=['--loopback', '{origin}', str(NOW)])
        self.assert_success(result)
        times = json.loads(result.stdout.split('write times (monotonic micros): ')[1].splitlines()[0])
        self.assertEqual(len(times), 3)
        self.assertTrue(all(b - a >= 1_000_000 for a, b in zip(times, times[1:])))

    def test_completed_plain_and_design_artifacts_age_and_machine_ownership(self):
        for design in (False, True):
            state = Github()
            stamp = f'ci-123-2-{NOW - DAY}'
            state.artifact(SCRATCH, stamp, design, run=123)
            state.artifact(SCRATCH, f'1-2-{NOW - 3 * DAY}')
            self.assert_success(state.run(NOW - 1))
            self.assertEqual(len(state.writes), 0)
            self.assert_success(state.run())
            self.assertEqual(len(state.writes), 3)
            self.assertEqual(len(state.issues[SCRATCH]), 1)
            self.assertEqual(state.status_reads, {123: 1})

    def test_status_and_read_failures_protect_artifacts(self):
        for status in (*(s for s in Status if s is not Status.COMPLETED), None, 42, 'cancelled', 'Completed', ''):
            state = Github()
            state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY}', run=123)
            state.status[123] = status
            result = state.run()
            if isinstance(status, Status):
                self.assert_success(result)
            else:
                # Absent (the stand-in answers 404), of another type, or a word GitHub does
                # not document; the last two are undecodable and named, which is how a status
                # GitHub adds is noticed.
                self.assert_failed(result)
                if status is not None:
                    self.assertIn(f'undecodable status ({json.dumps(status)})', result.stderr)
            self.assertFalse(state.writes, status)
        state.fail = '/actions/runs/'
        self.assertNotEqual(state.run().returncode, 0)
        self.assertFalse(state.writes)

    def test_each_batch_refreshes_and_new_attempt_survives(self):
        state = Github()
        for n in range(30):
            state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY - n}', run=123)
        state.write_inject = lambda: state.artifact(SCRATCH, f'ci-123-3-{NOW}', run=123)
        state.change_after = 1
        state.inject = lambda: state.artifact(SCRATCH, f'ci-123-2-{NOW}', run=123)
        self.assert_success(state.run())
        self.assertEqual(len(state.writes), 25)
        self.assertEqual(state.status_reads[123], 2)
        for attempt in (2, 3):
            self.assertTrue(any(f'ci-123-{attempt}-' in v['title'] for v in state.issues[SCRATCH]))
            self.assertTrue(any(f'ci-123-{attempt}-' in v['name'] for v in state.labels[SCRATCH]))
            self.assertTrue(any(f'ci-123-{attempt}-' in v.get('content', {}).get('title', '') for v in state.items))
        # The next batch starts with a fresh ownership read, after the 25 writes.
        indices = [i for i, (_, path) in enumerate(state.requests) if '/actions/runs/123' in path]
        between = state.requests[indices[0] + 1:indices[1]]
        self.assertEqual(sum(method == 'DELETE' or (method == 'POST' and path.startswith('mutation')) for method, path in between), 25)
        state.change_after = None
        state.status[RunId(123)] = Status.IN_PROGRESS
        writes = len(state.writes)
        self.assert_success(state.run())
        self.assertEqual(len(state.writes), writes)

    def test_legacy_cutover_and_old_running_attempt(self):
        state = Github()
        state.artifact(CORE, f'1-1-{CUTOVER - 1}', design=True)
        state.artifact(CORE, f'1-2-{CUTOVER}')
        self.assert_success(state.run(CUTOVER + DAY - 1))
        self.assertFalse(state.writes)
        state.owners[Status.IN_PROGRESS] = [{'created_at': '2020-01-01T00:00:00Z'}]
        self.assert_success(state.run())
        self.assertFalse(state.writes)
        state.owners[Status.IN_PROGRESS] = [{'created_at': '2030-01-01T00:00:00Z'}]
        self.assert_success(state.run())
        self.assertEqual(len(state.writes), 3)
        self.assertEqual(len(state.issues[CORE]), 1)

    def test_legacy_skew_boundary_is_strict_for_plain_and_design(self):
        margin = int(re.search(r'CLOCK_SKEW_MARGIN: Duration = Duration::from_secs\((\d+) \* (\d+)\)', (ROOT / 'crates/onetaskgraph-live-janitor/src/lib.rs').read_text())[1]) * 60 * 1_000_000
        stamp_time = (CUTOVER - DAY) // 1_000_000 * 1_000_000
        for design in (False, True):
            state = Github()
            state.artifact(CORE, f'1-1-{stamp_time}', design=design)
            boundary = datetime.datetime.fromtimestamp((stamp_time + margin) / 1_000_000, datetime.timezone.utc)
            state.owners[Status.WAITING] = [{'created_at': boundary.isoformat()}]
            self.assert_success(state.run())
            self.assertFalse(state.writes)
            state.owners[Status.WAITING] = [{'created_at': (boundary + datetime.timedelta(seconds=1)).isoformat()}]
            self.assert_success(state.run())
            self.assertEqual(len(state.writes), 3)

    def test_incomplete_enumeration_and_unaffordable_are_write_free(self):
        for fail in ('/issues', '/labels', 'board', '/actions/workflows/'):
            state = workload()
            state.fail = fail
            result = state.run()
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse(state.writes)
        for repeat in ('/issues', '/labels', 'board', '/actions/workflows/'):
            state = workload()
            match repeat:
                case '/labels':
                    state.labels[CORE].extend({'name': f'ordinary-{n}'} for n in range(100))
                case '/actions/workflows/':
                    state.owners[Status.QUEUED] = [{'id': RunId(n), 'created_at': '2030-01-01T00:00:00Z'} for n in range(100)]
            state.repeat = repeat
            result = state.run()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('advance', result.stderr)
            self.assertFalse(state.writes)
        state = workload()
        state.allowance = 0
        result = state.run()
        self.assertEqual(result.returncode, DECLINED, result.stderr)
        self.assertIn('janitor declined', result.stderr)
        self.assertFalse(state.writes)

    def test_legacy_refresh_is_paginated_and_late_failure_stops_writes(self):
        for failure in (False, True):
            state = Github()
            for n in range(30):
                state.artifact(CORE, f'1-{n + 1}-{CUTOVER - DAY}')
            # Two full pages and a final partial page, across each ownership refresh.
            state.owners[Status.PENDING] = [{'id': n, 'created_at': '2030-01-01T00:00:00Z'} for n in range(201)]
            # Initial evidence + first batch refresh = 14 calls, then refuse.
            if failure:
                state.owner_fail_after = 14
            else:
                state.owner_change_after = 14
            result = state.run()
            self.assertEqual(result.returncode == 0, not failure, result.stderr)
            self.assertEqual(len(state.writes), 25)
            self.assertGreater(state.owner_reads, 14)

    def test_legacy_batch_only_deletes_previously_listed_artifacts(self):
        state = Github()
        for n in range(30):
            state.artifact(CORE, f'1-{n + 1}-{CUTOVER - DAY}')
        # Even an old-looking artifact inserted by a rerun after enumeration is not
        # owned by this batch. A second one arrives during its first write.
        before = f'1-999-{CUTOVER - DAY - 1}'
        during = f'1-998-{CUTOVER - DAY - 2}'
        state.write_inject = lambda: state.artifact(CORE, during)
        state.owner_inject = lambda: state.artifact(CORE, before)
        self.assert_success(state.run())
        for stamp in (before, during):
            self.assertTrue(any(v['title'].endswith(stamp) for v in state.issues[CORE]))
            self.assertTrue(any(v['name'].endswith(stamp) for v in state.labels[CORE]))
            self.assertTrue(any(v.get('content', {}).get('title', '').endswith(stamp) for v in state.items))

    def test_scratch_later_status_failure_stops_writes(self):
        state = Github()
        for n in range(30):
            state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY - n}', run=123)
        state.artifact(SCRATCH, f'ci-124-1-{NOW - DAY}', run=124)
        state.artifact(CORE, f'1-124-{CUTOVER - DAY}')
        state.write_inject = lambda: setattr(state, 'invalid_json', '/actions/runs/123')
        self.assertNotEqual(state.run().returncode, 0)
        self.assertEqual(len(state.writes), 25)

    def test_noncompleted_status_protects_only_its_run(self):
        state = Github()
        state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY}', run=123)
        state.status[RunId(123)] = Status.IN_PROGRESS
        state.artifact(SCRATCH, f'ci-124-1-{NOW - DAY}', run=124)
        state.artifact(CORE, f'1-124-{CUTOVER - DAY}')
        self.assert_success(state.run())
        self.assertEqual(len(state.writes), 6)
        self.assertEqual(len(state.issues[SCRATCH]), 1)
        self.assertFalse(state.issues[CORE])

    def test_later_list_failures_are_write_free(self):
        for endpoint in ('/issues', '/labels', '/actions/workflows/', 'board'):
            state = workload()
            match endpoint:
                case '/labels':
                    state.labels[CORE].extend({'name': f'ordinary-{n}'} for n in range(201))
                case '/actions/workflows/':
                    state.owners[Status.PENDING] = [{'id': RunId(n), 'created_at': '2030-01-01T00:00:00Z'} for n in range(201)]
                case 'board':
                    state.board_fail_after = 1
            if endpoint != 'board':
                state.fail_page = FailedPage(endpoint, 2)
            result = state.run()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(state.writes)
        state = workload()
        state.board_invalid_json = True
        self.assertNotEqual(state.run().returncode, 0)
        self.assertFalse(state.writes)

    def test_short_pages_with_links_are_walked(self):
        state = Github()
        state.page_size = 10
        for n in range(25):
            state.issues[CORE].append({'node_id': f'o:{n}', 'title': 'Ordinary'})
        state.artifact(CORE, f'1-1-{CUTOVER - DAY}')
        self.assert_success(state.run())
        self.assertEqual(len(state.writes), 3)
        self.assertTrue(any('page=3' in path for _, path in state.requests))

    def test_undecodable_lists_fail_closed(self):
        for path in ('/issues', '/labels', '/actions/workflows/'):
            state = workload()
            state.undecodable = path
            result = state.run()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(state.writes)
        for path in ('/issues', '/labels', '/actions/workflows/'):
            state = workload()
            state.invalid_json = path
            self.assertNotEqual(state.run().returncode, 0)
            self.assertFalse(state.writes)
        state = workload()
        state.undecodable = '/actions/runs/'
        result = state.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(isinstance(target, IssueDelete) and target.id.startswith(SCRATCH) for _, target in state.writes))

    def test_malformed_pagination_links_fail_closed(self):
        for link in ('broken', '<{origin}{path}?page=2&state=open>; rel="next"',
                     '<https://example.com{path}?page=2>; rel="next"',
                     '<{origin}/repos/other/repository/issues?page=2>; rel="next"',
                     '<{origin}{path}?page=2#fragment>; rel="next"',
                     '<http://user:secret@127.0.0.1:80{path}?page=2>; rel="next"',
                     '<{origin}{path}?page=2>; rel="next", <{origin}{path}?page=3>; rel="next"'):
            with self.subTest(link=link):
                state = workload()
                state.link_override = link
                self.assertNotEqual(state.run().returncode, 0)
                self.assertFalse(state.writes)

    def test_each_pagination_refusal_is_reached_with_the_real_filters(self):
        # Every link keeps the request's own filters, so only the named defect differs.
        for link, refusal in (
                ('<{origin}{path}?{filters}&page=2&page=2>; rel="next"', 'duplicate pagination query parameter'),
                ('<{origin}{path}?{filters}>; rel="next"', 'pagination page missing'),
                ('<{origin}{path}?{filters}&page=02>; rel="next"', 'invalid pagination page'),
                ('<{origin}{path}?{filters}&page=two>; rel="next"', 'invalid pagination page'),
                ('<{origin}{path}?{filters}&page=99999999999999999999999>; rel="next"', 'invalid pagination page'),
                ('<{origin}{path}?{filters}&page=3>; rel="next"', 'did not advance'),
                ('<{origin}{path}?{filters}&page=2>; rel="next", <{origin}{path}?{filters}&page=2>; rel="next"',
                 'multiple next pagination Links')):
            with self.subTest(link=link):
                state = workload()
                state.link_override = link
                result = state.run()
                self.assert_failed(result)
                self.assertIn(refusal, result.stderr)
                self.assertFalse(state.writes)

    def test_rest_pin_is_exactly_what_the_janitor_sends(self):
        state = workload()
        self.assert_success(state.run())
        sent = {(o['method'], o['path']) for o in (rest_operation(r.method, urlparse(r.target).path) for r in state.requests if r.method != 'POST') if o}
        # The pin names nothing the janitor no longer sends, and run() refused anything it does not name.
        self.assertEqual(sent, {(o['method'], o['path']) for o in REST})

    def test_rest_pin_refuses_what_github_does_not_describe(self):
        issues = rest_operation('GET', '/repos/a/b/issues')
        for method, target in (('GET', '/repos/a/b/pulls'), ('GET', '/repos/a/b/issues?filter=all'),
                               ('GET', '/repos/a/b/issues?state=everything'), ('PATCH', '/repos/a/b/labels/x'),
                               ('GET', '/repos/a/b/actions/workflows/ci.yml/runs?status=running')):
            with self.subTest(target=target):
                self.assertIsNotNone(unpinned_request(method, target))
        self.assertIsNone(unpinned_request('GET', '/repos/a/b/issues?state=all&sort=created&direction=asc&per_page=100&page=2'))
        self.assertIsNotNone(unpinned_fields(issues['response'], [{'title': 't', 'node_identifier': 'x'}]))
        self.assertIsNotNone(unpinned_fields(issues['response'], {'title': 't'}))
        self.assertIsNone(unpinned_fields(issues['response'], [{'title': 't', 'node_id': 'x', 'pull_request': {}}]))
        runs = rest_operation('GET', '/repos/a/b/actions/workflows/ci.yml/runs')
        self.assertIsNotNone(unpinned_fields(runs['response'], {'total_count': 1, 'workflow_runs': [{'created': 'x'}]}))
        state = Github()
        state.unpinned.append('a departure')
        with self.assertRaisesRegex(AssertionError, 'a departure'):
            state.run()

    def test_incomplete_run_total_count_fails_closed(self):
        state = workload()
        state.total_count = 1
        self.assert_failed(state.run())
        self.assertFalse(state.writes)
        # A listing that does not say how large it is cannot be shown complete.
        state = workload()
        state.omit_total_count = True
        result = state.run()
        self.assert_failed(result)
        self.assertIn('incomplete enumeration', result.stderr)
        self.assertFalse(state.writes)

    def test_invalid_records_fail_closed(self):
        for collection, record in (('issues', {'node_id': 42, 'title': 'ordinary'}),
                                   ('issues', {'node_id': '', 'title': 'ordinary'}),
                                   ('issues', {'node_id': 'id\nnext', 'title': 'ordinary'}),
                                   ('issues', {'node_id': 'id', 'title': None}),
                                   ('labels', {'name': 42})):
            state = workload()
            getattr(state, collection)[CORE].append(record)
            self.assertNotEqual(state.run().returncode, 0, repr(record))
            self.assertFalse(state.writes)
        for field, value in (('id', 42), ('items', {'nodes': None}),
                             ('items', {'nodes': [{'id': 42, 'content': None}]}),
                             ('items', {'nodes': [{'id': 'item'}], 'pageInfo': {'hasNextPage': False}}),
                             ('items', {'nodes': [{'id': 'item', 'content': {}}], 'pageInfo': {'hasNextPage': False}}),
                             ('items', {'nodes': [{'id': 'item', 'content': {'__typename': 42}}], 'pageInfo': {'hasNextPage': False}}),
                             ('items', {'nodes': [{'id': 'item', 'content': {'__typename': 'Issue', 'title': 'ordinary', 'repository': None}}]}),
                             ('items', {'nodes': [], 'pageInfo': {'hasNextPage': 'yes', 'endCursor': 'a'}}),
                             ('items', {'nodes': [], 'pageInfo': {'hasNextPage': True, 'endCursor': None}})):
            state = workload()
            state.board_transform = lambda body, f=field, v=value: body['data']['board']['projectV2'].__setitem__(f, v)
            self.assertNotEqual(state.run().returncode, 0, repr((field, value)))
            self.assertFalse(state.writes)
        for created in ('not-a-date', '1969-12-31T23:59:59Z', None):
            state = workload()
            state.owners[Status.QUEUED] = [{'created_at': created}]
            self.assert_failed(state.run())
            self.assertFalse(state.writes, created)

    def test_an_unreadable_allowance_sends_nothing_further(self):
        # The allowance is the first request; without it no list is read and nothing written.
        for failure in ('fail', 'undecodable'):
            state = workload()
            setattr(state, failure, '/rate_limit')
            result = state.run()
            self.assert_failed(result)
            self.assertIn('rate_limit', result.stderr)
            self.assertEqual(state.requests, [Request('GET', '/rate_limit')], failure)
            self.assertFalse(state.writes)

    def test_failed_deletes_stop_invocation(self):
        for operation in Deletion:
            state = Github()
            state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY}', run=123)
            state.delete_failure = operation
            self.assert_failed(state.run())
            self.assertLessEqual(len(state.writes), 3)
        issue_done = {'data': {'deleteIssue': {'repository': {'nameWithOwner': CORE}}}}
        item_done = {'data': {'deleteProjectV2Item': {'deletedItemId': 'item:another'}}}
        refusals = {
            # Board items are deleted first, so the first write is the one refused.
            Deletion.ITEM: ({'errors': [{'message': 'refused'}]}, {'data': None}, {'data': {}},
                     {'data': {'deleteProjectV2Item': {}}}, item_done, issue_done),
            # Every board item lands; the first issue answer is the one refused.
            Deletion.ISSUE: ({'data': {'deleteIssue': None}}, {'data': {'deleteIssue': {}}},
                      {'data': {'deleteIssue': {'repository': None}}},
                      # Deleted, GitHub says, from a repository the janitor did not name.
                      {'data': {'deleteIssue': {'repository': {'nameWithOwner': 'other/repository'}}}}, item_done),
        }
        for operation, responses in refusals.items():
            for response in responses:
                state = workload()
                state.delete_response[operation] = response
                self.assert_failed(state.run())
                issue_writes = [w for w in state.writes if isinstance(w.target, IssueDelete)]
                self.assertEqual(len(issue_writes), operation is Deletion.ISSUE, response)
                self.assertEqual(state.writes[-1].method, 'POST', response)
                self.assertEqual(isinstance(state.writes[-1].target, IssueDelete), operation is Deletion.ISSUE, response)

    def test_label_delete_redirect_is_a_failure(self):
        for status in (302, 200):
            state = Github()
            state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY}', run=123)
            labels = copy.deepcopy(state.labels[SCRATCH])
            state.label_delete_status = status
            result = state.run()
            self.assert_failed(result)
            self.assertIn('delete label', result.stderr)
            self.assertEqual(state.labels[SCRATCH], labels)
            self.assertIsInstance(state.writes[-1].target, LabelDelete)

    def test_invalid_allowances_are_write_free(self):
        for record in ({}, {'limit': '5000', 'remaining': 5000, 'reset': 2000000000},
                       {'limit': 5000, 'remaining': -1, 'reset': 2000000000}):
            state = workload()
            state.allowance_record = record
            self.assert_failed(state.run())
            self.assertFalse(state.writes)

    def test_inconsistent_allowance_stops_before_enumeration(self):
        state = workload()
        state.allowance_record = {'limit': 5000, 'remaining': 5001, 'reset': 2000000000}
        result = state.run()
        self.assert_failed(result)
        self.assertIn('rate_limit allowance invalid', result.stderr)
        self.assertEqual(state.requests, [Request('GET', '/rate_limit')])
        self.assertFalse(state.writes)

    def test_rest_limit_during_later_refresh_stops_all_writes(self):
        state = Github()
        for n in range(50):
            state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY - n}', run=123)
        state.artifact(SCRATCH, f'ci-124-1-{NOW - DAY}', run=124)
        state.artifact(CORE, f'1-1-{CUTOVER - DAY}')
        state.issues[CORE].extend({'node_id': IssueId(f'ordinary:{n}'), 'title': 'Ordinary'} for n in range(23_600))
        result = state.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('REST read limit', result.stderr)
        self.assertEqual(len(state.writes), 100)
        self.assertEqual(state.status_reads, {123: 4})
        self.assertTrue(any(issue['title'].endswith(f'ci-124-1-{NOW - DAY}') for issue in state.issues[SCRATCH]))
        self.assertTrue(any(issue['title'].endswith(f'1-1-{CUTOVER - DAY}') for issue in state.issues[CORE]))

    def test_read_limits_fail_closed_even_after_eligible_pages(self):
        for resource in ('REST', 'GraphQL'):
            state = Github()
            state.artifact(SCRATCH, f'ci-123-1-{NOW - DAY}', run=123)
            match resource:
                case 'REST':
                    state.issues[CORE].extend({'node_id': IssueId(f'o:{n}'), 'title': 'Ordinary'} for n in range(25_000))
                case 'GraphQL':
                    state.items.extend({'id': ItemId(f'd:{n}'), 'content': {'__typename': 'DraftIssue'}} for n in range(5_000))
            result = state.run()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(resource + ' read limit', result.stderr)
            self.assertFalse(state.writes)

    def test_exact_grammar_and_other_repository_survive(self):
        state = Github()
        for title in ('Feature request', TITLE, TITLE + 'ci-01-1-1', 'copy of ' + DESIGN + TITLE + '1-1-1'):
            state.issues[CORE].append({'node_id': title, 'title': title})
        state.labels[CORE].append({'name': LABEL + 'malformed'})
        state.items.append({'id': 'foreign', 'content': {'__typename': 'Issue', 'title': TITLE + '1-1-1', 'repository': {'nameWithOwner': 'other/repository'}}})
        # An item whose content this token cannot see is answered as null, and is nobody's residue.
        state.items.append({'id': 'hidden', 'content': None})
        # The issues endpoint lists pull requests too; one titled like residue is not an issue.
        stamp = f'ci-123-1-{NOW - DAY}'
        state.status[RunId(123)] = Status.COMPLETED
        state.issues[SCRATCH].append({'node_id': 'pull', 'title': TITLE + stamp, 'pull_request': {'url': 'https://example.invalid/pull/1'}})
        self.assert_success(state.run())
        self.assertFalse(state.writes)
        self.assertFalse(state.status_reads)
        self.assertIn('pull', [issue['node_id'] for issue in state.issues[SCRATCH]])

    def test_future_stamps_and_legacy_ci_stamps_survive(self):
        state = Github()
        state.status[RunId(123)] = Status.COMPLETED
        for design in (False, True):
            state.artifact(SCRATCH, f'ci-123-1-{NOW + DAY}', design=design)
            state.artifact(CORE, f'ci-123-1-{CUTOVER - DAY}', design=design)
        issues = copy.deepcopy(state.issues)
        labels = copy.deepcopy(state.labels)
        items = copy.deepcopy(state.items)
        self.assert_success(state.run())
        self.assertFalse(state.writes)
        self.assertFalse(state.status_reads)
        self.assertEqual(state.issues, issues)
        self.assertEqual(state.labels, labels)
        self.assertEqual(state.items, items)

    def test_both_passes_preserve_origin_field_and_retained_item_values(self):
        state = workload()
        # Populate values on every board item, including the running CI run, machine
        # artifacts and drafts which both passes must leave. Values belong to items,
        # so values on items actually deleted are not part of the preservation claim.
        state.origin_values = {item['id']: f"source:{item['id']}" for item in state.items}
        before_field = copy.deepcopy(state.origin_field)
        before_values = copy.deepcopy(state.origin_values)
        self.assert_success(state.run())
        self.assertTrue(any(isinstance(write.target, IssueDelete) and write.target.id.startswith(SCRATCH) for write in state.writes))
        self.assertTrue(any(isinstance(write.target, IssueDelete) and write.target.id.startswith(CORE) for write in state.writes))
        self.assertEqual(state.origin_field, before_field)
        retained = {item['id'] for item in state.items}
        self.assertTrue(retained)
        self.assertEqual({item: state.origin_values[item] for item in retained},
                         {item: before_values[item] for item in retained})

    def test_realistic_budget_and_tenfold_drain(self):
        state = workload()
        result = state.run()
        self.assert_success(result)
        self.assertEqual(len(state.writes), 150)
        times = json.loads(result.stdout.split('write times (monotonic micros): ')[1].splitlines()[0])
        self.assertTrue(all(b - a >= 1_000_000 for a, b in zip(times, times[1:])))
        self.assertEqual(state.requests[0], ('GET', '/rate_limit'))
        for path, token in state.authentication:
            self.assertEqual(token, 'Bearer offline-actions' if '/actions/' in path else 'Bearer offline-write')
            if path.startswith('/repos/'):
                self.assertIn('/'.join(path.split('/')[2:4]), (CORE, SCRATCH))
        self.assertEqual(state.boards, {Board(BOARD_OWNER, 1)})
        self.assertTrue(all('deleteIssue' in path or 'deleteProjectV2Item' in path for method, path in state.requests if method == 'POST' and path.startswith('mutation')))
        self.assertEqual(state.origin_field, ORIGIN_FIELD)
        telemetry = os.environ.get('JANITOR_BUDGET_TELEMETRY')
        if telemetry:
            Path(telemetry).parent.mkdir(parents=True, exist_ok=True)
            Path(telemetry).write_text(json.dumps({
                'value': len(state.requests),
                'detail': 'Every REST and GraphQL request under both tokens at the realistic workload',
            }))
        state = workload(10)
        protected = copy.deepcopy(state.issues[SCRATCH][-3:])
        ordinary = copy.deepcopy([v for v in state.issues[CORE] if v['title'].startswith('Feature')])
        protected_items = copy.deepcopy([v for v in state.items if v['id'] == 'draft' or any(v['id'] == 'item:' + issue['node_id'] for issue in protected)])
        protected_labels = {repo: copy.deepcopy([v for v in nodes if v['name'] == 'ordinary' or (repo == SCRATCH and ('ci-999-' in v['name'] or v['name'].startswith(LABEL + '1-')))]) for repo, nodes in state.labels.items()}
        for _ in range(40):
            before = len(state.requests)
            writes = len(state.writes)
            listing_sizes = {(repo, kind): len(nodes) for repo in (CORE, SCRATCH) for kind, nodes in (('issues', state.issues[repo]), ('labels', state.labels[repo]))}
            board_pages = max(1, (len(state.items) + 99) // 100)
            result = state.run()
            self.assert_success(result)
            times = json.loads(result.stdout.split('write times (monotonic micros): ')[1].splitlines()[0])
            self.assertTrue(all(b - a >= 1_000_000 for a, b in zip(times, times[1:])))
            for issue in protected:
                self.assertIn(issue, state.issues[SCRATCH])
            for issue in ordinary:
                self.assertIn(issue, state.issues[CORE])
            for item in protected_items:
                self.assertIn(item, state.items)
            for repo, nodes in protected_labels.items():
                for label in nodes:
                    self.assertIn(label, state.labels[repo])
            self.assertEqual(state.origin_field, ORIGIN_FIELD)
            requests = state.requests[before:]
            for (repo, kind), size in listing_sizes.items():
                pages = [path for method, path in requests if method == 'GET' and urlparse(path).path == f'/repos/{repo}/{kind}']
                self.assertEqual(len(pages), size // 100 + 1)
                self.assertEqual([int(parse_qs(urlparse(path).query)['page'][0]) for path in pages], list(range(1, len(pages) + 1)))
            self.assertEqual(sum(method == 'POST' and path.startswith('query') for method, path in requests), board_pages)
            self.assertLessEqual(sum(method == 'GET' for method, _ in requests), 250)
            self.assertLessEqual(sum(method == 'POST' and path.startswith('query') for method, path in requests), 50)
            self.assertLessEqual(len(state.writes) - writes, 150)
            eligible = any(v['title'].startswith(TITLE) or v['title'].startswith(DESIGN + TITLE) for v in state.issues[CORE]) or any('ci-999-' not in v['title'] and 'ci-' in v['title'] for v in state.issues[SCRATCH]) or any(v['name'].startswith(LABEL) for v in state.labels[CORE])
            if not eligible:
                break
            self.assertGreater(len(state.writes), writes)
        else:
            self.fail('backlog did not drain')
        self.assertEqual(state.issues[SCRATCH], protected)
        self.assertEqual(len(state.issues[CORE]), 360)
        self.assertTrue(any(v['id'] == 'draft' for v in state.items))

if __name__ == '__main__':
    unittest.main()
