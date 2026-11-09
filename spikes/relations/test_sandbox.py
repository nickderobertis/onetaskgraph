"""Every refusal the sandbox owes, offline: `python3 -m unittest spikes/relations/test_sandbox.py`."""

# llmlint: ignore-file[new_code_lands_in_a_project] Spike branch kept for the record, not merged; see harness.py.
# llmlint: ignore-file[modern_domain_modeling] Fixtures of the trackers' raw JSON inputs, which is exactly what the guard is handed; see harness.py.
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from sandbox import (
    GITHUB_ID_KEYS,
    LINEAR_ID_KEYS,  # noqa: E402 -- sys.path is set just above
    Enum,
    Mut,
    Refused,
    Sandbox,
    document,
    refuse_protected_board,
)

STAMP = "20261108T000000"


def box() -> Sandbox:
    return Sandbox(STAMP, "OWNER", {"R1", "R2"}, {"T2", "T1"}, {"I1", "I2", "B1", "L1", "P1"})


def github(root: str, **payload: object) -> str:
    muts = [Mut("a", root, {"input": payload})]
    box().check_github(muts)
    return document(muts)


def linear(root: str, args: dict) -> str:
    muts = [Mut("a", root, args)]
    box().check_linear(muts)
    return document(muts)


class Allowed(unittest.TestCase):
    def test_writes_inside_the_sandbox(self) -> None:
        self.assertIn(
            'repositoryId:"R2"', github("createIssue", repositoryId="R2", title=f"x {STAMP}")
        )
        self.assertIn(
            "replaceParent:true",
            github("addSubIssue", issueId="I1", subIssueId="I2", replaceParent=True),
        )
        self.assertIn(
            "addProjectV2ItemById", github("addProjectV2ItemById", projectId="B1", contentId="I1")
        )
        self.assertIn(
            "createProjectV2Field", github("createProjectV2Field", projectId="B1", name="n")
        )
        self.assertIn(
            "createProjectV2",
            github(
                "createProjectV2",
                ownerId="OWNER",
                title=f"onetaskgraph live board spike-relations-{STAMP}-a",
            ),
        )
        self.assertIn(
            'parentId:"L1"',
            linear(
                "issueCreate", {"input": {"teamId": "T2", "title": f"x {STAMP}", "parentId": "L1"}}
            ),
        )
        self.assertIn(
            "parentId:null", linear("issueUpdate", {"id": "L1", "input": {"parentId": None}})
        )

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
        self.assert_refused(github, "createProjectV2Field", projectId="BOARD1", name="x")

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

    def test_unstamped_titles_on_writes_to_owned_objects(self) -> None:
        self.assert_refused(github, "updateIssue", id="I1", title="unstamped")
        self.assert_refused(
            linear, "issueCreate", {"input": {"teamId": "T2", "title": "unstamped"}}
        )
        self.assert_refused(linear, "issueUpdate", {"id": "L1", "input": {"title": "unstamped"}})

    def test_linear_creation_and_moves_stay_inside(self) -> None:
        stamped = {"teamId": "T2", "title": f"x {STAMP}"}
        for key in ("parentId", "projectId"):
            with self.assertRaisesRegex(Refused, f"{key}='OTHER' was not created"):
                linear("issueCreate", {"input": {**stamped, key: "OTHER"}})
        self.assert_refused(linear, "issueUpdate", {"id": "L1", "input": {"teamId": "ENG"}})
        with self.assertRaisesRegex(Refused, "id='OTHER' was not created"):
            linear("issueUpdate", {"id": "OTHER", "input": {"title": f"x {STAMP}"}})
        with self.assertRaisesRegex(Refused, "id='OTHER' was not created"):
            linear("issueRelationUpdate", {"id": "OTHER", "input": {"type": "similar"}})
        self.assertIn(
            'title:"x ', linear("issueUpdate", {"id": "L1", "input": {"title": f"x {STAMP}"}})
        )

    def test_removed_grants_are_refused(self) -> None:
        self.assert_refused(linear, "issueRelationDelete", {"id": "P1"})
        self.assert_refused(github, "reprioritizeSubIssue", issueId="I1", subIssueId="I2")

    def test_every_github_reference_key_must_be_this_runs(self) -> None:
        for key in GITHUB_ID_KEYS:
            self.assert_refused(github, "updateIssue", **{key: "OTHER"})

    def test_every_linear_reference_key_must_be_this_runs(self) -> None:
        for key in LINEAR_ID_KEYS - {"id"}:
            self.assert_refused(linear, "issueUpdate", {"id": "L1", "input": {key: "OTHER"}})

    def test_linear_projects_and_relations(self) -> None:
        self.assertIn(
            "projectCreate", linear("projectCreate", {"input": {"teamIds": ["T2"], "name": STAMP}})
        )
        self.assert_refused(linear, "projectCreate", {"input": {"teamIds": [], "name": STAMP}})
        self.assert_refused(
            linear, "projectCreate", {"input": {"teamIds": ["T2", "ENG"], "name": STAMP}}
        )
        self.assert_refused(
            linear, "projectCreate", {"input": {"teamIds": ["T2"], "name": "unstamped"}}
        )
        self.assert_refused(
            linear, "issueRelationUpdate", {"id": "OTHER", "input": {"type": "similar"}}
        )
        self.assert_refused(linear, "projectDelete", {"id": "OTHER"})

    def test_document_spells_tuples(self) -> None:
        self.assertEqual(
            document([Mut("a", "x", {"input": {"t": ("p", 1)}}, "id")]),
            'mutation{a:x(input:{t:["p",1]}){id}}',
        )

    def test_a_title_that_is_not_text_is_refused(self) -> None:
        self.assert_refused(github, "createIssue", repositoryId="R1", title=42)
        self.assert_refused(linear, "issueCreate", {"input": {"teamId": "T2", "title": 42}})

    def test_one_forbidden_alias_refuses_the_whole_batch(self) -> None:
        allowed = Mut("a", "addSubIssue", {"input": {"issueId": "I1", "subIssueId": "I2"}})
        forbidden = Mut("b", "addSubIssue", {"input": {"issueId": "I1", "subIssueId": "OTHER"}})
        with self.assertRaises(Refused):
            box().check_github([allowed, forbidden])
        allowed = Mut("a", "issueUpdate", {"id": "L1", "input": {"parentId": None}})
        forbidden = Mut("b", "issueUpdate", {"id": "OTHER", "input": {"parentId": None}})
        with self.assertRaises(Refused):
            box().check_linear([allowed, forbidden])

    def test_a_board_this_run_may_use_passes(self) -> None:
        self.assertIsNone(refuse_protected_board(263))

    def test_protected_boards(self) -> None:
        for number in (1, 2, 3):
            self.assert_refused(refuse_protected_board, number)


if __name__ == "__main__":
    unittest.main()
