"""Drive the scheduled Linear executable through its real HTTP and process boundaries."""
from dataclasses import asdict, dataclass, field, replace
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


class ProjectFilter(TypedDict):
    name: TitleFilter


class ListingVariables(TypedDict):
    after: Cursor | None
    filter: DocumentFilter


class ProjectListingVariables(TypedDict):
    after: Cursor | None
    filter: ProjectFilter


class DeleteVariables(TypedDict):
    id: DocumentId


class Request(TypedDict):
    query: str
    variables: ListingVariables | ProjectListingVariables | DeleteVariables


class Operation(StrEnum):
    """Which of the janitor's four Linear operations a request is, read once off its query."""
    DOCUMENTS = auto()
    PROJECTS = auto()
    DELETE_DOCUMENT = auto()
    DELETE_PROJECT = auto()

    @classmethod
    def of(cls, query: str) -> 'Operation':
        for root, operation in [('documentDelete(', cls.DELETE_DOCUMENT), ('projectDelete(', cls.DELETE_PROJECT),
                                ('documents(', cls.DOCUMENTS), ('projects(', cls.PROJECTS)]:
            if root in query:
                return operation
        raise AssertionError(f'not an operation the janitor sends: {query}')


def read_request(value: object) -> Request:
    assert isinstance(value, dict) and isinstance(value.get('query'), str)
    variables = value.get('variables')
    assert isinstance(variables, dict)
    match Operation.of(value['query']):
        case Operation.DELETE_DOCUMENT | Operation.DELETE_PROJECT:
            assert set(variables) == {'id'}
            assert isinstance(variables['id'], str) and variables['id'].strip()
        case listing:
            assert set(variables) == {'after', 'filter'}
            assert variables['after'] is None or (isinstance(variables['after'], str) and variables['after'].strip())
            field = 'name' if listing is Operation.PROJECTS else 'title'
            assert variables['filter'] == {field: {'startsWith': 'onetaskgraph live cleanup '}}
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
    updatedAt: str
    team: Team | None
    project: Project | None

    @classmethod
    def recorded(cls, record: dict[str, Any]) -> 'Document':
        # Any is confined to nested JSON fixture decoding; schema.rs validates this recorded shape.
        project = record['project']
        return cls(DocumentId(record['id']), record['title'], record['updatedAt'],
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


@dataclass
class ListedProject:
    id: DocumentId
    name: str
    updatedAt: str
    teams: Teams

    @classmethod
    def recorded(cls, record: dict[str, Any]) -> 'ListedProject':
        # Any is confined to nested JSON fixture decoding; schema.rs validates this recorded shape.
        teams = record['teams']
        return cls(DocumentId(record['id']), record['name'], record['updatedAt'],
                   Teams([Team(**node) for node in teams['nodes']], TeamPageInfo(**teams['pageInfo'])))


@dataclass
class ProjectPage:
    nodes: list[ListedProject]
    pageInfo: PageInfo


@dataclass
class ProjectData:
    projects: ProjectPage


@dataclass
class ProjectResponse:
    data: ProjectData


def recorded_projects() -> list[ListedProject]:
    record = json.loads((Path(__file__).parent / 'fixtures/linear-projects.json').read_text())
    return [ListedProject.recorded(node) for node in record['data']['projects']['nodes']]


def updated(seconds_ago: int) -> str:
    """An `updatedAt` as Linear spells one, this long before now."""
    return time.strftime('%Y-%m-%dT%H:%M:%S.000Z', time.gmtime(time.time() - seconds_ago))


class FailureMode(StrEnum):
    PROJECT_LISTING = auto()
    PROJECT_CURSOR = auto()
    PROJECT_READ_LIMIT = auto()
    MALFORMED_PROJECT_DELETE = auto()
    PROJECT_DELETE = auto()
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


class ProjectMalformation(StrEnum):
    NO_ID = auto()
    NO_NAME = auto()
    NO_UPDATED_AT = auto()
    NO_TEAMS = auto()
    NO_TEAM_PAGE_INFO = auto()
    NO_CURSOR = auto()
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
        # No project unless a journey seeds one: `linear-projects.json` is what one seeds.
        self.projects: list[ListedProject] = []
        self.project_page_size = 100
        self.project_malformed: ProjectMalformation | None = None
        self.deleted: list[DocumentId] = []
        self.requests: list[Request] = []
        self.failure = failure
        self.malformed = malformed

    def answer(self, request: Request) -> dict[str, Any]:
        # Any admits deliberately malformed wire payloads as well as schema-checked success records.
        self.requests.append(request)
        match Operation.of(request['query']):
            case Operation.DELETE_PROJECT:
                return self.delete_project(cast(DeleteVariables, request['variables'])['id'])
            case Operation.PROJECTS:
                assert 'teams(first:2){nodes{key} pageInfo{hasNextPage}}' in request['query']
                return self.project_page(cast(ProjectListingVariables, request['variables'])['after'])
            case Operation.DELETE_DOCUMENT:
                # read_request validated delete variables before this operation discriminator.
                return self.delete_document(cast(DeleteVariables, request['variables'])['id'])
            case Operation.DOCUMENTS:
                return self.document_page(request)

    def delete_project(self, identifier: DocumentId) -> dict[str, Any]:
        # Any admits deliberately malformed wire payloads as well as schema-checked success records.
        match self.failure:
            case FailureMode.PROJECT_DELETE:
                return {'data': {'projectDelete': {'success': False}}}
            case FailureMode.MALFORMED_PROJECT_DELETE:
                return {'data': {'projectDelete': {}}}
        self.deleted.append(identifier)
        return json.loads((Path(__file__).parent / 'fixtures/linear-project-delete.json').read_text())

    def project_page(self, after: Cursor | None) -> dict[str, Any]:
        # Any admits deliberately malformed wire payloads as well as schema-checked success records.
        start = int(after.removeprefix('project-')) if after else 0
        match self.failure:
            case FailureMode.PROJECT_LISTING:
                return {'errors': [{'message': 'recorded listing refusal'}]}
            case FailureMode.PROJECT_CURSOR:
                return asdict(ProjectResponse(ProjectData(ProjectPage([], PageInfo(True, Cursor('project-0'))))))
            case FailureMode.PROJECT_READ_LIMIT:
                return asdict(ProjectResponse(ProjectData(ProjectPage(
                    [], PageInfo(True, Cursor(f'project-{start + 1}'))))))
        # A deleted record is gone from the next listing, as it is from Linear's.
        live = [project for project in self.projects if project.id not in self.deleted]
        end = start + self.project_page_size
        more = end < len(live)
        answer = asdict(ProjectResponse(ProjectData(ProjectPage(
            live[start:end], PageInfo(more, Cursor(f'project-{end}') if more else None)))))
        # Deliberately corrupt wire records, never construct invalid domain models.
        page = answer['data']['projects']
        match self.project_malformed:
            case ProjectMalformation.NO_ID:
                page['nodes'][0].pop('id')
            case ProjectMalformation.NO_NAME:
                page['nodes'][0].pop('name')
            case ProjectMalformation.NO_UPDATED_AT:
                page['nodes'][0].pop('updatedAt')
            case ProjectMalformation.NO_TEAMS:
                page['nodes'][0].pop('teams')
            case ProjectMalformation.NO_TEAM_PAGE_INFO:
                page['nodes'][0]['teams'].pop('pageInfo')
            case ProjectMalformation.NO_CURSOR:
                page['pageInfo'] = {'hasNextPage': True}
            case ProjectMalformation.NO_PAGE_INFO:
                page.pop('pageInfo')
        return answer

    def delete_document(self, identifier: DocumentId) -> dict[str, Any]:
        # Any admits deliberately malformed wire payloads as well as schema-checked success records.
        match self.failure:
            case FailureMode.DELETE:
                return {'data': {'documentDelete': {'success': False}}}
            case FailureMode.SECOND_DELETE if len(self.deleted) == 1:
                return {'data': {'documentDelete': {'success': False}}}
            case FailureMode.MALFORMED_DELETE:
                return {'data': {'documentDelete': {}}}
        self.deleted.append(identifier)
        return json.loads((Path(__file__).parent / 'fixtures/linear-document-delete.json').read_text())

    def document_page(self, request: Request) -> dict[str, Any]:
        # Any admits deliberately malformed wire payloads as well as schema-checked success records.
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
            return self.listed(self.pages[1])
        # Deliberately corrupt wire records, never construct invalid domain models.
        answer = self.listed(self.pages[0])
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

    def listed(self, page: Response) -> dict[str, Any]:
        # Any admits the recorded JSON tree. A deleted record is gone from the next listing.
        answer = asdict(page)
        nodes = answer['data']['documents']['nodes']
        answer['data']['documents']['nodes'] = [node for node in nodes if node['id'] not in self.deleted]
        return answer


RETAINED = 'onetaskgraph live cleanup retained-v1 '
SUPERSEDED = [RETAINED + 'ci TEST ' + slot for slot in ['filed', 'loose', 'follow-up', 'assets']]
# Every standing identity the janitor never touches, however old: a current pool member's
# slots, a local invocation's set, another team's, and titles no current scheme parses.
KEPT = [RETAINED + 'ci TEST ' + slot + ' pool-3' for slot in ['filed', 'loose', 'follow-up', 'assets']]
KEPT += [RETAINED + 'local TEST ' + slot + ' 1-2-3' for slot in ['filed', 'loose', 'follow-up']]
KEPT += [RETAINED + 'ci OTHER filed', RETAINED + 'ci TEST unknown', RETAINED + 'ci TEST parent',
         RETAINED + 'ci TEST filed pool-x', RETAINED + 'ci TEST filed pool-17',
         'onetaskgraph live cleanup retained-v2 ci TEST filed',
         'onetaskgraph live cleanup retained-v1 ci TEST filed extra',
         'onetaskgraph live cleanup lease-v1 TEST plugin pool-3 gen-2 7-42-1700000000000000']



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
        # Two document pages and the one project page, then the three deletes.
        self.assertEqual(len(workspace.requests), 6)
        self.assertIn('0 REST reads, 3 GraphQL queries, 3 content writes', result.stdout)
        self.assertIn('[0, 1000000, 2000000]', result.stdout)

    def standing(self, workspace: Workspace, recent: set[str] | None = None) -> None:
        """Seeds every standing document, a month untouched unless `recent` names its title."""
        sample = workspace.pages[0].data.documents.nodes[0]
        workspace.pages[0].data.documents.nodes = [
            replace(sample, id=DocumentId(title), title=title,
                    updatedAt=updated(60) if title in (recent or set()) else sample.updatedAt)
            for title in SUPERSEDED + KEPT]
        workspace.pages[1].data.documents.nodes = []
        workspace.projects = recorded_projects()

    def test_the_superseded_pre_pool_set_is_retired_and_every_other_standing_identity_kept(self) -> None:
        workspace = Workspace()
        self.standing(workspace)
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        # Its four documents, then the parent project they were filed under; never a parent
        # shared with another team, nor any project of a pool member, a local set or another team.
        self.assertEqual(workspace.deleted, [*SUPERSEDED, 'superseded-parent'])
        self.assertIn('3 GraphQL queries, 5 content writes', result.stdout)
        # The next scheduled pass finds no such set left, and writes nothing.
        again = self.run_janitor(workspace)
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertEqual(workspace.deleted, [*SUPERSEDED, 'superseded-parent'])
        self.assertIn('0 content writes', again.stdout)

    def test_a_superseded_set_written_within_a_day_is_kept_with_its_parent(self) -> None:
        # A run on a branch from before the pool still writes the set each pass.
        workspace = Workspace()
        self.standing(workspace, recent={SUPERSEDED[1]})
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(workspace.deleted, [SUPERSEDED[0], *SUPERSEDED[2:]])
        self.assertNotIn('superseded-parent', workspace.deleted)
        # Nor is a parent written within a day, or one whose time does not read.
        for name, stamp in [('recent', updated(60)), ('unreadable', 'yesterday')]:
            with self.subTest(parent=name):
                workspace = Workspace()
                self.standing(workspace)
                workspace.projects = [replace(project, updatedAt=stamp) for project in workspace.projects]
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(workspace.deleted, SUPERSEDED)

    def test_a_superseded_document_expires_a_day_after_linear_last_changed_it(self) -> None:
        day = 24 * 3600
        for name, stamp, retired in [('just-past', updated(day + 60), True),
                                     ('just-short', updated(day - 60), False),
                                     ('unreadable', 'yesterday', False)]:
            with self.subTest(updated=name):
                workspace = Workspace()
                self.standing(workspace)
                nodes = workspace.pages[0].data.documents.nodes
                nodes[0] = replace(nodes[0], updatedAt=stamp)
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 0, result.stderr)
                # One the janitor keeps holds its parent back with it.
                self.assertEqual(workspace.deleted,
                                 [*SUPERSEDED, 'superseded-parent'] if retired else SUPERSEDED[1:])

    def test_a_project_listing_that_does_not_complete_writes_nothing(self) -> None:
        for failure in [FailureMode.PROJECT_LISTING, FailureMode.PROJECT_CURSOR]:
            with self.subTest(failure=failure):
                workspace = Workspace(failure)
                self.standing(workspace)
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(workspace.deleted, [])

    def test_a_malformed_project_listing_writes_nothing(self) -> None:
        for malformed in ProjectMalformation:
            with self.subTest(malformed=malformed):
                workspace = Workspace()
                self.standing(workspace)
                workspace.project_malformed = malformed
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(workspace.deleted, [])

    def test_an_unconfirmed_or_malformed_project_delete_fails_after_its_documents(self) -> None:
        for failure, said in [(FailureMode.PROJECT_DELETE, 'did not confirm project deletion'),
                              (FailureMode.MALFORMED_PROJECT_DELETE, 'invalid or malformed JSON')]:
            with self.subTest(failure=failure):
                workspace = Workspace(failure)
                self.standing(workspace)
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(said, result.stderr)
                self.assertEqual(workspace.deleted, SUPERSEDED)

    def test_a_superseded_document_it_cannot_vouch_for_is_kept_with_its_parent(self) -> None:
        sample_project = Project(Teams([Team('TEST')], TeamPageInfo(False)))
        for name, change in [
            ('other-team', {'team': Team('OTHER')}),
            ('shared-project', {'team': None, 'project': Project(Teams([Team('TEST'), Team('OTHER')], TeamPageInfo(False)))}),
            ('incomplete-project', {'team': None, 'project': Project(Teams([Team('TEST')], TeamPageInfo(True)))}),
            ('filed', {'team': None, 'project': sample_project}),
        ]:
            with self.subTest(document=name):
                workspace = Workspace()
                self.standing(workspace)
                nodes = workspace.pages[0].data.documents.nodes
                nodes[0] = replace(nodes[0], **change)
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 0, result.stderr)
                if name == 'filed':
                    # Filed under a project of the nomination alone, it is vouched for.
                    self.assertEqual(workspace.deleted, [*SUPERSEDED, 'superseded-parent'])
                else:
                    self.assertEqual(workspace.deleted, SUPERSEDED[1:])

    def test_a_parent_whose_teams_do_not_read_as_the_nomination_alone_is_kept(self) -> None:
        for name, teams in [('no-team', Teams([], TeamPageInfo(False))),
                            ('incomplete', Teams([Team('TEST')], TeamPageInfo(True)))]:
            with self.subTest(teams=name):
                workspace = Workspace()
                self.standing(workspace)
                workspace.projects = [replace(project, teams=teams) if project.id == 'superseded-parent' else project
                                      for project in workspace.projects]
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(workspace.deleted, SUPERSEDED)

    def test_the_parent_is_found_on_a_later_project_page(self) -> None:
        workspace = Workspace()
        self.standing(workspace)
        workspace.projects = [*workspace.projects[1:], workspace.projects[0]]
        workspace.project_page_size = 2
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(workspace.deleted, [*SUPERSEDED, 'superseded-parent'])
        # Two document pages and three project pages.
        self.assertIn('5 GraphQL queries, 5 content writes', result.stdout)

    def test_a_repeated_project_id_writes_nothing(self) -> None:
        for name in ['repeated-project', 'project-named-as-a-document']:
            with self.subTest(case=name):
                workspace = Workspace()
                self.standing(workspace)
                parent = workspace.projects[0]
                twin = parent if name == 'repeated-project' else replace(parent, id=DocumentId(SUPERSEDED[0]))
                workspace.projects = [*workspace.projects, twin]
                result = self.run_janitor(workspace)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn('repeated an id', result.stderr)
                self.assertEqual(workspace.deleted, [])

    def test_the_project_listing_shares_the_read_cap_and_writes_nothing_past_it(self) -> None:
        workspace = Workspace(FailureMode.PROJECT_READ_LIMIT)
        self.standing(workspace)
        result = self.run_janitor(workspace)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn('project listing exceeded its read limit', result.stderr)
        self.assertEqual(len(workspace.requests), 50)
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
        self.assertEqual(len(workspace.requests), 153)

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
                self.assertEqual(len(workspace.requests), 4)
                self.assertEqual(workspace.deleted, [])



LeaseId = NewType('LeaseId', str)
LeaseTitle = NewType('LeaseTitle', str)


class LeaseIssue(TypedDict):
    id: LeaseId
    title: LeaseTitle


@dataclass(frozen=True)
class LeaseRead:
    """`issue(id:)`, what `--release-leases` asks of each recorded lease."""
    id: LeaseId


@dataclass(frozen=True)
class LeaseUpdate:
    """`issueUpdate`, the one write `--release-leases` sends: the lease and its new title."""
    id: LeaseId
    title: LeaseTitle


def read_lease_request(value: object) -> LeaseRead | LeaseUpdate:
    """The janitor's request, its whole shape checked rather than assumed."""
    assert isinstance(value, dict), value
    query, variables = value.get('query'), value.get('variables')
    assert isinstance(query, str) and isinstance(variables, dict), value
    identifier = variables.get('id')
    assert isinstance(identifier, str) and identifier, variables
    if 'issueUpdate(' in query:
        assert set(variables) == {'id', 'input'}, variables
        update = variables['input']
        assert isinstance(update, dict) and set(update) == {'title'}, update
        title = update['title']
        assert isinstance(title, str) and title, update
        return LeaseUpdate(LeaseId(identifier), LeaseTitle(title))
    assert 'issue(id:$id)' in query and set(variables) == {'id'}, value
    return LeaseRead(LeaseId(identifier))


class LeaseFault(StrEnum):
    UNIDENTIFIED_READ = auto()
    NO_ISSUE = auto()
    UNCONFIRMED_RELEASE = auto()


class RecordsPath(StrEnum):
    """What stands at the path `--release-leases` is given."""
    DIRECTORY = auto()
    ABSENT = auto()
    FILE = auto()


@dataclass
class LeaseTeam:
    """What the stand-in Linear holds of the pool's lease issues, and what it was asked."""
    issues: dict[LeaseId, LeaseTitle]
    fault: LeaseFault | None = None
    reads: list[LeaseId] = field(default_factory=list)
    updates: list[LeaseUpdate] = field(default_factory=list)

    def answer(self, request: LeaseRead | LeaseUpdate) -> dict[str, object]:
        match request:
            case LeaseUpdate():
                self.updates.append(request)
                if self.fault is LeaseFault.UNCONFIRMED_RELEASE:
                    return {'data': {'issueUpdate': {'success': False}}}
                self.issues[request.id] = request.title
                return {'data': {'issueUpdate': {'success': True}}}
            case LeaseRead():
                self.reads.append(request.id)
                title = self.issues.get(request.id)
                match self.fault:
                    case LeaseFault.UNIDENTIFIED_READ:
                        return {'data': {'issue': {'title': title}}}
                    case LeaseFault.NO_ISSUE:
                        return {'data': {}}
                    case _:
                        issue: LeaseIssue | None = None if title is None else LeaseIssue(id=request.id, title=title)
                        return {'data': {'issue': issue}}


class LeaseJourneys(unittest.TestCase):
    """`--linear --release-leases DIR`: a cancelled job hands back what it recorded, only that."""

    PREFIX = 'onetaskgraph live cleanup lease-v1 TEST plugin '

    def release(self, team: LeaseTeam, records: dict[str, str],
                path: RecordsPath = RecordsPath.DIRECTORY) -> tuple[subprocess.CompletedProcess[str], Path]:
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args: object) -> None:
                pass

            def do_POST(self) -> None:
                assert self.headers['Authorization'] == 'offline-linear-key'
                request = read_lease_request(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                body = json.dumps(team.answer(request)).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        directory = Path(os.environ.get('TMPDIR', '/tmp')) / f'lease-records-{os.getpid()}-{time.monotonic_ns()}'
        match path:
            case RecordsPath.DIRECTORY:
                directory.mkdir(parents=True)
                for name, content in records.items():
                    (directory / f'{name}.json').write_text(content)
            case RecordsPath.FILE:
                directory.write_text('not a directory of lease records')
            case RecordsPath.ABSENT:
                pass
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
        return result, directory

    @staticmethod
    def record(identifier: str, title: str) -> str:
        return json.dumps({'id': identifier, 'title': title})

    def test_a_cancelled_runs_recorded_lease_is_released_and_nothing_else(self) -> None:
        held = LeaseTitle(self.PREFIX + 'pool-3 gen-2 7-42-1700000000000000')
        superseded = LeaseTitle(self.PREFIX + 'pool-4 gen-1 7-42-1700000000000000')
        team = LeaseTeam({
            LeaseId('held'): held,
            # Recorded by this run, since superseded by another claimant's generation.
            LeaseId('superseded'): LeaseTitle(self.PREFIX + 'pool-4 gen-1 8-1-1700000000000000'),
            # Another run's lease, never recorded here.
            LeaseId('other'): LeaseTitle(self.PREFIX + 'pool-5 gen-1 8-1-1700000000000000'),
        })
        result, directory = self.release(team, {
            'held': self.record('held', held),
            'superseded': self.record('superseded', superseded),
            # Recorded by this run and gone from Linear: nothing to hand back.
            'gone': self.record('gone', LeaseTitle(self.PREFIX + 'pool-6 gen-1 7-42-1700000000000000')),
        })
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(team.updates, [LeaseUpdate(LeaseId('held'), LeaseTitle(self.PREFIX + 'pool-3 gen-2 released'))])
        self.assertNotIn(LeaseId('other'), team.reads)
        self.assertEqual(list(directory.iterdir()), [], 'every settled record is removed')

    def test_no_records_send_nothing(self) -> None:
        team = LeaseTeam({})
        result, _ = self.release(team, {})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((team.reads, team.updates), ([], []))

    def test_an_absent_records_directory_is_a_run_that_held_no_lease(self) -> None:
        team = LeaseTeam({LeaseId('held'): LeaseTitle(self.PREFIX + 'pool-3 gen-2 7-42-1700000000000000')})
        result, directory = self.release(team, {}, RecordsPath.ABSENT)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((team.reads, team.updates), ([], []))
        self.assertFalse(directory.exists(), 'nothing is created where no lease was recorded')

    def test_a_records_path_that_is_not_a_directory_fails_sending_nothing(self) -> None:
        team = LeaseTeam({LeaseId('held'): LeaseTitle(self.PREFIX + 'pool-3 gen-2 7-42-1700000000000000')})
        result, directory = self.release(team, {}, RecordsPath.FILE)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn('the lease records could not be read', result.stderr)
        self.assertEqual((team.reads, team.updates), ([], []))
        self.assertTrue(directory.is_file(), 'what stood there is left as it was')

    def test_a_record_that_does_not_read_soundly_is_refused_writing_nothing(self) -> None:
        held = self.PREFIX + 'pool-3 gen-2 7-42-1700000000000000'
        for name, content, said in [
            ('invalid-json', '{', 'not a lease record'),
            ('no-title', json.dumps({'id': 'held'}), 'names no issue id and title'),
            ('released', self.record('held', self.PREFIX + 'pool-3 gen-2 released'), 'names no held lease'),
            ('malformed', self.record('held', self.PREFIX + 'pool-3 gen-02 7-42-1'), 'names no held lease'),
            ('empty-id', self.record('', held), 'unusable issue id'),
            ('foreign-team', self.record('held', held.replace(' TEST ', ' OTHER ')), 'records no held lease of test'),
        ]:
            with self.subTest(name=name):
                team = LeaseTeam({LeaseId('held'): LeaseTitle(held)})
                result, directory = self.release(team, {name: content})
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(said, result.stderr)
                self.assertEqual((team.reads, team.updates), ([], []))
                self.assertEqual(len(list(directory.iterdir())), 1, 'a refused record is kept')

    def test_an_unidentified_or_missing_read_or_an_unconfirmed_release_keeps_the_record(self) -> None:
        held = LeaseTitle(self.PREFIX + 'pool-3 gen-2 7-42-1700000000000000')
        for fault, said in [
            (LeaseFault.UNIDENTIFIED_READ, 'does not identify'),
            (LeaseFault.NO_ISSUE, 'with no issue at all'),
            (LeaseFault.UNCONFIRMED_RELEASE, 'did not confirm releasing'),
        ]:
            with self.subTest(fault=fault):
                team = LeaseTeam({LeaseId('held'): held}, fault)
                result, directory = self.release(team, {'held': self.record('held', held)})
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(said, result.stderr)
                if fault is not LeaseFault.UNCONFIRMED_RELEASE:
                    self.assertEqual(team.updates, [], 'a read that settles nothing writes nothing')
                self.assertEqual(len(list(directory.iterdir())), 1, 'the record is kept for a re-run')

if __name__ == '__main__':
    unittest.main()
