"""Every refusal the sandbox owes, offline: `python3 -m unittest spikes/relations/test_sandbox.py`."""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from sandbox import Enum, Mut, Refused, Sandbox, document, refuse_protected_board  # noqa: E402

STAMP = "20261108T000000"


def box() -> Sandbox:
    return Sandbox(STAMP, "OWNER", {"R1", "R2"}, {"T2", "T1"}, {"I1", "I2", "B1", "L1", "P1"})


def github(root: str, **payload: object) -> None:
    box().check_github([Mut("a", root, {"input": payload})])


def linear(root: str, args: dict) -> None:
    box().check_linear([Mut("a", root, args)])


class Allowed(unittest.TestCase):
    def test_writes_inside_the_sandbox(self) -> None:
        github("createIssue", repositoryId="R2", title=f"x {STAMP}")
        github("addSubIssue", issueId="I1", subIssueId="I2", replaceParent=True)
        github("addProjectV2ItemById", projectId="B1", contentId="I1")
        github(
            "createProjectV2",
            ownerId="OWNER",
            title=f"onetaskgraph live board spike-relations-{STAMP}-a",
        )
        linear("issueCreate", {"input": {"teamId": "T2", "title": f"x {STAMP}", "parentId": "L1"}})
        linear("issueUpdate", {"id": "L1", "input": {"parentId": None}})

    def test_document_spells_enums_and_nulls(self) -> None:
        text = document([Mut("a", "x", {"input": {"t": Enum("BLOCKS"), "p": None}}, "id")])
        self.assertEqual(text, "mutation{a:x(input:{t:BLOCKS,p:null}){id}}")


class Refusals(unittest.TestCase):
    def assert_refused(self, call, *args, **kwargs) -> None:
        with self.assertRaises(Refused):
            call(*args, **kwargs)

    def test_unknown_mutation(self) -> None:
        self.assert_refused(github, "deleteRepository", repositoryId="R1")
        self.assert_refused(linear, "teamDelete", {"id": "T1"})

    def test_other_repository_or_unstamped_title(self) -> None:
        self.assert_refused(github, "createIssue", repositoryId="R9", title=f"x {STAMP}")
        self.assert_refused(github, "createIssue", repositoryId="R1", title="unstamped")

    def test_issue_or_board_not_created_by_this_run(self) -> None:
        self.assert_refused(github, "addSubIssue", issueId="I1", subIssueId="OTHER")
        self.assert_refused(github, "deleteIssue", issueId="OTHER")
        self.assert_refused(github, "addProjectV2ItemById", projectId="BOARD3", contentId="I1")
        self.assert_refused(github, "updateProjectV2", projectId="BOARD1", public=True)

    def test_board_title_and_owner(self) -> None:
        self.assert_refused(github, "createProjectV2", ownerId="OWNER", title="plans")
        self.assert_refused(
            github,
            "createProjectV2",
            ownerId="OTHER",
            title=f"onetaskgraph live board spike-relations-{STAMP}",
        )

    def test_linear_other_team_or_foreign_ids(self) -> None:
        self.assert_refused(linear, "issueCreate", {"input": {"teamId": "ENG", "title": STAMP}})
        self.assert_refused(linear, "projectCreate", {"input": {"teamIds": ["ENG"], "name": STAMP}})
        self.assert_refused(linear, "issueDelete", {"id": "lease-issue"})
        self.assert_refused(linear, "issueUpdate", {"id": "L1", "input": {"parentId": "OTHER"}})
        self.assert_refused(linear, "issueRelationCreate", {"input": {"issueId": "OTHER"}})

    def test_protected_boards(self) -> None:
        for number in (1, 2, 3):
            self.assert_refused(refuse_protected_board, number)


if __name__ == "__main__":
    unittest.main()
