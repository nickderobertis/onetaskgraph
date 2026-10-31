"""Drive the scheduled Linear executable through its real HTTP and process boundaries."""
from dataclasses import asdict, dataclass, replace
from enum import StrEnum, auto
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import threading
import time
from typing import Any, NewType, TypedDict, cast
import unittest

DocumentId = NewType('DocumentId', str)
Cursor = NewType('Cursor', str)


class TitleFilter(TypedDict):
    startsWith: str


class DocumentFilter(TypedDict):
    title: TitleFilter


class ListingVariables(TypedDict):
    after: Cursor | None
    filter: DocumentFilter


class DeleteVariables(TypedDict):
    id: DocumentId


class Request(TypedDict):
    query: str
    variables: ListingVariables | DeleteVariables


def read_request(value: object) -> Request:
    assert isinstance(value, dict) and isinstance(value.get('query'), str)
    variables = value.get('variables')
    assert isinstance(variables, dict)
    if 'documentDelete(' in value['query']:
        assert set(variables) == {'id'}
        assert isinstance(variables['id'], str) and variables['id'].strip()
    else:
        assert set(variables) == {'after', 'filter'}
        assert variables['after'] is None or (isinstance(variables['after'], str) and variables['after'].strip())
        assert variables['filter'] == {'title': {'startsWith': 'onetaskgraph live cleanup '}}
    # Runtime checks above establish the union shape; Python cannot infer a TypedDict from dictionary checks.
    return cast(Request, value)


@dataclass
class Team:
    key: str


@dataclass
class PageInfo:
    hasNextPage: bool
    endCursor: Cursor | None


@dataclass
class TeamPageInfo:
    hasNextPage: bool


@dataclass
class Teams:
    nodes: list[Team]
    pageInfo: TeamPageInfo


@dataclass
class Project:
    teams: Teams


@dataclass
class Document:
    id: DocumentId
    title: str
    team: Team | None
    project: Project | None

    @classmethod
    def recorded(cls, record: dict[str, Any]) -> 'Document':
        # Any is confined to nested JSON fixture decoding; schema.rs validates this recorded shape.
        project = record['project']
        return cls(DocumentId(record['id']), record['title'],
                   Team(**record['team']) if record['team'] else None,
                   Project(Teams([Team(**node) for node in project['teams']['nodes']],
                                 TeamPageInfo(**project['teams']['pageInfo']))) if project else None)


@dataclass
class DocumentPage:
    nodes: list[Document]
    pageInfo: PageInfo


@dataclass
class DocumentData:
    documents: DocumentPage


@dataclass
class Response:
    data: DocumentData

    @classmethod
    def recorded(cls, record: dict[str, Any]) -> 'Response':
        # Any accommodates the heterogeneous JSON tree validated upstream by schema.rs.
        page = record['data']['documents']
        return cls(DocumentData(DocumentPage([Document.recorded(node) for node in page['nodes']],
                                             PageInfo(**page['pageInfo']))))


class FailureMode(StrEnum):
    DELETE = auto()
    SECOND_DELETE = auto()
    LISTING = auto()
    CURSOR = auto()
    READ_LIMIT = auto()
    HTTP = auto()
    INVALID_JSON = auto()
    NO_DATA = auto()
    MALFORMED_DELETE = auto()
    CONNECTION = auto()


class Malformation(StrEnum):
    NO_TEAM = auto()
    NO_PROJECT = auto()
    NO_ID = auto()
    EMPTY_ID = auto()
    NO_TITLE = auto()
    DUPLICATE_ID = auto()
    NO_CURSOR = auto()
    EMPTY_CURSOR = auto()
    NO_PAGE_INFO = auto()


@dataclass(frozen=True)
class ConfigCase:
    variables: dict[str, str | None]
    arguments: list[str] | None = None


class Workspace:
    def __init__(self, failure: FailureMode | None = None,
                 malformed: Malformation | None = None):
        record = json.loads((Path(__file__).parent / 'fixtures/linear-documents.json').read_text())
        response = Response.recorded(record)
        nodes = response.data.documents.nodes
        self.pages = [Response(DocumentData(DocumentPage(nodes[:5], PageInfo(True, Cursor('page-two'))))),
                      Response(DocumentData(DocumentPage(nodes[5:], PageInfo(False, None))))]
        self.deleted: list[DocumentId] = []
        self.requests: list[Request] = []
        self.failure = failure
        self.malformed = malformed

    def answer(self, request: Request) -> dict[str, Any]:
        # Any admits deliberately malformed wire payloads as well as schema-checked success records.
        self.requests.append(request)
        if 'documentDelete(' in request['query']:
            # read_request validated delete variables before this operation discriminator.
            identifier = cast(DeleteVariables, request['variables'])['id']
            match self.failure:
                case FailureMode.DELETE:
                    return {'data': {'documentDelete': {'success': False}}}
                case FailureMode.SECOND_DELETE if len(self.deleted) == 1:
                    return {'data': {'documentDelete': {'success': False}}}
                case FailureMode.MALFORMED_DELETE:
                    return {'data': {'documentDelete': {}}}
            self.deleted.append(identifier)
            return json.loads((Path(__file__).parent / 'fixtures/linear-document-delete.json').read_text())
        assert 'team{key}' in request['query']
        assert 'project{teams(first:2)' in request['query']
        # read_request validated the listing shape; Python cannot narrow a TypedDict union by query text.
        variables = cast(ListingVariables, request['variables'])
        assert variables['filter']['title']['startsWith'] == 'onetaskgraph live cleanup '
        after = variables['after']
        match self.failure:
            case FailureMode.READ_LIMIT:
                page = int(after) + 1 if after else 1
                return asdict(Response(DocumentData(DocumentPage([], PageInfo(True, Cursor(str(page)))))))
            case FailureMode.NO_DATA:
                return {}
        if after is not None:
            assert after == 'page-two'
            match self.failure:
                case FailureMode.LISTING:
                    return {'errors': [{'message': 'recorded listing refusal'}]}
                case FailureMode.CURSOR:
                    return asdict(Response(DocumentData(DocumentPage([], PageInfo(True, Cursor('page-two'))))))
            return asdict(self.pages[1])
        # Deliberately corrupt wire records, never construct invalid domain models.
        answer = asdict(self.pages[0])
        page = answer['data']['documents']
        match self.malformed:
            case Malformation.NO_TEAM:
                page['nodes'][0].pop('team')
            case Malformation.NO_PROJECT:
                page['nodes'][0].pop('project')
            case Malformation.NO_ID:
                page['nodes'][0].pop('id')
            case Malformation.EMPTY_ID:
                page['nodes'][0]['id'] = ''
            case Malformation.NO_TITLE:
                page['nodes'][0].pop('title')
            case Malformation.DUPLICATE_ID:
                page['nodes'].append(page['nodes'][0])
            case Malformation.NO_CURSOR:
                page['pageInfo'].pop('endCursor')
            case Malformation.EMPTY_CURSOR:
                page['pageInfo']['endCursor'] = ''
            case Malformation.NO_PAGE_INFO:
                page.pop('pageInfo')
        return answer


class LinearJourneys(unittest.TestCase):
    def run_janitor(self, workspace: Workspace, *, arguments: list[str] | None = None,
                    variables: dict[str, str | None] | None = None,
                    virtual_clock: bool = True) -> subprocess.CompletedProcess[str]:
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args: object) -> None:
                pass

            def do_POST(self) -> None:
                assert self.path == '/graphql'
                assert self.headers['Authorization'] == 'offline-linear-key'
                request = read_request(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                match workspace.failure:
                    case FailureMode.CONNECTION:
                        workspace.requests.append(request)
                        self.connection.shutdown(socket.SHUT_RDWR)
                        return
                    case FailureMode.HTTP:
                        workspace.requests.append(request)
                        self.send_response(503)
                        self.end_headers()
                        return
                    case FailureMode.INVALID_JSON:
                        workspace.requests.append(request)
                        body = b'not JSON'
                    case _:
                        body = json.dumps(workspace.answer(request)).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith(('LINEAR_', 'GH_', 'GITHUB_'))}
            environment.update(LINEAR_API_KEY='offline-linear-key', LINEAR_WRITE_TEAM='test')
            for name, value in (variables or {}).items():
                if value is None:
                    environment.pop(name, None)
                else:
                    environment[name] = value
            args = arguments if arguments is not None else [
                '--loopback', f'http://127.0.0.1:{server.server_port}']
            if arguments is None and virtual_clock:
                args.append('--virtual-clock')
            return subprocess.run([os.environ['JANITOR_BINARY'], '--linear', *args],
                                  env=environment, capture_output=True, text=True, timeout=30)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_real_clock_runs_cleanup_and_reports_listing_failure(self) -> None:
        for failure in [None, FailureMode.LISTING]:
            with self.subTest(failure=failure):
                workspace = Workspace(failure)
                sample = workspace.pages[0].data.documents.nodes[0]
                workspace.pages[0].data.documents.nodes = [sample]
                workspace.pages[1].data.documents.nodes = []
                result = self.run_janitor(workspace, virtual_clock=False)
                self.assertEqual(result.returncode, 0 if failure is None else 1, result.stderr)
                self.assertEqual(workspace.deleted, ['stale-machine'] if failure is None else [])

    def test_configuration_refusals_send_no_requests(self) -> None:
        cases = [
            ConfigCase({'LINEAR_API_KEY': None}), ConfigCase({'LINEAR_API_KEY': ' '}),
            ConfigCase({'LINEAR_WRITE_TEAM': None}), ConfigCase({'LINEAR_WRITE_TEAM': ' '}),
            ConfigCase({'LINEAR_WRITE_TEAM': 'TEST\n'}),
            ConfigCase({}, ['--unexpected']), ConfigCase({}, ['--loopback']),
            *[ConfigCase({}, ['--loopback', origin]) for origin in [
                'http://example.com', 'http://127.0.0.1/other', 'http://user@127.0.0.1/',
                'http://127.0.0.1/?query', 'http://127.0.0.1/#fragment',
                'https://127.0.0.1/', 'not a URL']],
        ]
        for case in cases:
            with self.subTest(case=case):
                workspace = Workspace()
                result = self.run_janitor(workspace, variables=case.variables, arguments=case.arguments)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn('sending nothing', result.stderr)
                self.assertEqual(workspace.requests, [])

    def test_stale_test_documents_removed_and_other_documents_preserved(self) -> None:
        workspace = Workspace()
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(workspace.deleted, ['stale-machine', 'stale-ci', 'stale-project'])
        all_ids = {node.id for page in workspace.pages for node in page.data.documents.nodes}
        self.assertEqual(all_ids - set(workspace.deleted), {
            'fresh', 'unstamped', 'legacy-asset', 'other-team', 'shared-project',
            'unknown-team', 'malformed-stamp', 'unvouched', 'incomplete-project'})
        self.assertEqual(len(workspace.requests), 5)
        self.assertIn('0 REST reads, 2 GraphQL queries, 3 content writes', result.stdout)
        self.assertIn('[0, 1000000, 2000000]', result.stdout)

    def test_retained_sets_never_expire_by_age(self) -> None:
        workspace = Workspace()
        sample = workspace.pages[0].data.documents.nodes[0]
        prefix = 'onetaskgraph live cleanup retained-v1 '
        titles = [prefix + 'ci TEST ' + slot for slot in ['filed', 'loose', 'follow-up', 'assets']]
        titles += [prefix + 'local TEST filed 1-2-3',
                   prefix + 'ci TEST unknown', prefix + 'ci OTHER filed']
        workspace.pages[0].data.documents.nodes = [
            replace(sample, id=DocumentId(str(index)), title=title)
            for index, title in enumerate(titles)]
        workspace.pages[1].data.documents.nodes = []
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(workspace.deleted, [])

    def test_direct_team_precedes_project_and_unvouched_documents_survive(self) -> None:
        workspace = Workspace()
        sample = workspace.pages[0].data.documents.nodes[0]
        project = Project(Teams([Team('TEST')], TeamPageInfo(False)))
        workspace.pages[0].data.documents.nodes = [
            replace(sample, id=DocumentId('direct-other'), team=Team('OTHER'), project=project),
            replace(sample, id=DocumentId('direct-test'), project=Project(Teams([Team('OTHER')], TeamPageInfo(False)))),
            replace(sample, id=DocumentId('empty-project'), team=None, project=Project(Teams([], TeamPageInfo(False)))),
            replace(sample, id=DocumentId('zero-stamp'), title='onetaskgraph live cleanup 1-2-0'),
        ]
        workspace.pages[1].data.documents.nodes = []
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(workspace.deleted, ['direct-test'])

    def test_expiry_is_twenty_four_hours(self) -> None:
        workspace = Workspace()
        now = int(time.time() * 1_000_000)
        sample = workspace.pages[0].data.documents.nodes[0]
        expired = replace(sample, id=DocumentId('expired'),
                          title=f'onetaskgraph live cleanup 1-2-{now - (24 * 3600 + 60) * 1_000_000}')
        protected = replace(sample, id=DocumentId('protected'),
                            title=f'onetaskgraph live cleanup 1-2-{now - (24 * 3600 - 60) * 1_000_000}')
        workspace.pages[0].data.documents.nodes = [expired, protected]
        workspace.pages[1].data.documents.nodes = []
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(workspace.deleted, ['expired'])

    def test_incomplete_or_refused_listing_sends_no_deletes(self) -> None:
        for failure in [FailureMode.LISTING, FailureMode.CURSOR, FailureMode.CONNECTION,
                        FailureMode.HTTP, FailureMode.INVALID_JSON, FailureMode.NO_DATA]:
            with self.subTest(failure=failure):
                workspace = Workspace(failure)
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(workspace.deleted, [])
        for malformed in Malformation:
            with self.subTest(malformed=malformed):
                workspace = Workspace(malformed=malformed)
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(workspace.deleted, [])

    def test_read_cap_refuses_an_unfinished_listing_without_writing(self) -> None:
        workspace = Workspace(FailureMode.READ_LIMIT)
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn('exceeded its read limit', result.stderr)
        self.assertEqual(len(workspace.requests), 50)
        self.assertEqual(workspace.deleted, [])

    def test_write_cap_leaves_the_rest_for_the_next_scheduled_pass(self) -> None:
        workspace = Workspace()
        sample = workspace.pages[0].data.documents.nodes[0]
        nodes = [replace(sample, id=DocumentId(f'residue-{index}')) for index in range(155)]
        workspace.pages[0].data.documents.nodes = nodes[:100]
        workspace.pages[1].data.documents.nodes = nodes[100:]
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(workspace.deleted, [f'residue-{index}' for index in range(150)])
        self.assertEqual(len(workspace.requests), 152)

    def test_later_delete_failure_keeps_prior_removal_and_stops(self) -> None:
        workspace = Workspace(FailureMode.SECOND_DELETE)
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(workspace.deleted, ['stale-machine'])
        # The filter selects delete operations whose variables read_request already validated.
        deletes = [cast(DeleteVariables, request['variables'])['id'] for request in workspace.requests
                   if 'documentDelete(' in request['query']]
        self.assertEqual(deletes, ['stale-machine', 'stale-ci'])
        self.assertIn('did not confirm document deletion', result.stderr)

    def test_refused_or_malformed_delete_fails_and_stops_the_batch(self) -> None:
        for failure in [FailureMode.DELETE, FailureMode.MALFORMED_DELETE]:
            with self.subTest(failure=failure):
                workspace = Workspace(failure)
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(len(workspace.requests), 3)
                self.assertEqual(workspace.deleted, [])



class LeaseJourneys(unittest.TestCase):
    """`--linear --release-leases DIR`: a cancelled job hands back what it recorded, only that."""

    PREFIX = 'onetaskgraph live cleanup lease-v1 TEST plugin '

    def release(self, issues: dict[str, str], records: dict[str, str]) -> tuple[subprocess.CompletedProcess[str], list[dict[str, Any]], Path]:
        requests: list[dict[str, Any]] = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args: object) -> None:
                pass

            def do_POST(self) -> None:
                assert self.headers['Authorization'] == 'offline-linear-key'
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                requests.append(request)
                identifier = request['variables']['id']
                if 'issueUpdate(' in request['query']:
                    issues[identifier] = request['variables']['input']['title']
                    answer: dict[str, Any] = {'data': {'issueUpdate': {'success': True}}}
                else:
                    assert 'issue(id:$id)' in request['query'], request['query']
                    title = issues.get(identifier)
                    answer = {'data': {'issue': None if title is None else {'id': identifier, 'title': title}}}
                body = json.dumps(answer).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        directory = Path(os.environ.get('TMPDIR', '/tmp')) / f'lease-records-{os.getpid()}-{time.monotonic_ns()}'
        directory.mkdir(parents=True)
        for identifier, title in records.items():
            (directory / f'{identifier}.json').write_text(json.dumps({'id': identifier, 'title': title}))
        try:
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith(('LINEAR_', 'GH_', 'GITHUB_'))}
            environment.update(LINEAR_API_KEY='offline-linear-key', LINEAR_WRITE_TEAM='test')
            result = subprocess.run(
                [os.environ['JANITOR_BINARY'], '--linear', '--release-leases', str(directory),
                 '--loopback', f'http://127.0.0.1:{server.server_port}'],
                env=environment, capture_output=True, text=True, timeout=30)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        return result, requests, directory

    def test_a_cancelled_runs_recorded_lease_is_released_and_nothing_else(self) -> None:
        held = self.PREFIX + 'pool-3 gen-2 7-42-1700000000000000'
        superseded = self.PREFIX + 'pool-4 gen-1 7-42-1700000000000000'
        issues = {
            'held': held,
            # Recorded by this run, since superseded by another claimant's generation.
            'superseded': self.PREFIX + 'pool-4 gen-1 8-1-1700000000000000',
            # Another run's lease, never recorded here.
            'other': self.PREFIX + 'pool-5 gen-1 8-1-1700000000000000',
        }
        result, requests, directory = self.release(dict(issues), {'held': held, 'superseded': superseded})
        self.assertEqual(result.returncode, 0, result.stderr)
        updates = [r for r in requests if 'issueUpdate(' in r['query']]
        self.assertEqual([r['variables']['id'] for r in updates], ['held'])
        self.assertEqual(updates[0]['variables']['input']['title'], self.PREFIX + 'pool-3 gen-2 released')
        self.assertFalse(any(r['variables']['id'] == 'other' for r in requests))
        self.assertEqual(list(directory.iterdir()), [], 'every settled record is removed')

    def test_no_records_send_nothing_and_a_foreign_record_is_refused(self) -> None:
        result, requests, _ = self.release({}, {})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(requests, [])
        foreign = 'onetaskgraph live cleanup lease-v1 OTHER plugin pool-3 gen-2 7-42-1'
        result, requests, directory = self.release({'x': foreign}, {'x': foreign})
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn('records no held lease of test', result.stderr)
        self.assertEqual(requests, [])
        self.assertEqual(len(list(directory.iterdir())), 1, 'a refused record is kept')

if __name__ == '__main__':
    unittest.main()
