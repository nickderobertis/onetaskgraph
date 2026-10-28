//! A re-copy into a cached Linear source whose listing of the project's members misbehaves still
//! lands, driven through the binary against the loopback Linear.
//!
//! Before a re-copy of a project reads each counterpart, the engine walks the destination
//! project's members once to pin the change tokens that listing carries in the cache standing
//! in front of it. That walk only pins: a page it refuses — one overrunning the page it asked
//! for, one answering with the cursor it was sent, or two cursors answering each other — ends
//! it, pinning nothing more, and every counterpart is read as if there were no cache. Linear is
//! the cached source whose listing hands the engine its own cursors, so it is where the walk's
//! guards are reachable from the command line.

use serde_json::{Value, json};

use crate::cache::{Cached, dataset};

/// How the loopback Linear bends its answers to the pinning walk.
#[derive(Clone, Copy)]
enum Fault {
    /// One more member than the page asked for, saying more follow.
    OverrunsThePage,
    /// The first page says more follow at a cursor, and the page at that cursor answers with it.
    RepeatsTheCursor,
    /// Two cursors answering each other, so each page advances and the walk never ends.
    CyclesItsCursors,
}

impl Fault {
    /// How many of the walk's pages the fault bends — every page the walk asks for before its
    /// guards end it.
    fn pages(self) -> usize {
        match self {
            Self::OverrunsThePage => 1,
            Self::RepeatsTheCursor => 2,
            Self::CyclesItsCursors => 3,
        }
    }
}

/// One outcome of a copy: its source, its destination and its action.
type Outcome = (String, String, String);

/// One request Linear answered: its document and its variables.
type Sent = (String, Value);

/// Whether a request is a listing of the issues filed under the project `project`.
fn lists_members_of(project: &str, operation: &str, variables: &Value) -> bool {
    operation.contains("issues(first:$first")
        && variables["filter"]
            .to_string()
            .contains(&format!("\"{project}\""))
}

/// A project of two tasks, one depending on the other, in the `notes` folder.
fn plan(cached: &Cached) {
    let notes = cached.sandbox.project().join("notes");
    for (path, text) in [
        (
            "projects/NP.md",
            "---\ntitle: A plan\nstatus: todo\n---\nThe plan.\n",
        ),
        (
            "tasks/NT-1.md",
            "---\ntitle: First\nstatus: todo\nproject: NP\n---\nFirst.\n",
        ),
        (
            "tasks/NT-2.md",
            "---\ntitle: Second\nstatus: todo\nproject: NP\ndepends_on: [NT-1]\n---\nSecond.\n",
        ),
    ] {
        std::fs::write(notes.join(path), text).expect("a plan file");
    }
}

/// Each outcome a copy reported, as its source, its destination and its action.
fn outcomes(report: &Value) -> Vec<Outcome> {
    report["items"]
        .as_array()
        .expect("a copy report")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().unwrap_or_default().to_owned(),
                item["destination"].as_str().unwrap_or_default().to_owned(),
                item["action"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The plan copied into Linear, then copied again with the pinning walk bent by `fault`: what
/// the re-copy reported, and every request it sent, with its variables.
fn recopied(fault: Option<Fault>) -> (Value, Vec<Sent>, Vec<Outcome>) {
    let cached = Cached::new(dataset());
    plan(&cached);
    let copy = ["project", "copy", "notes:NP", "--to", "linear", "--json"];
    let (first, _) = cached.ok(&copy);
    let landed = outcomes(&first);
    let project = landed[0]
        .1
        .strip_prefix("linear:")
        .expect("the project's counterpart")
        .to_owned();
    if let Some(fault) = fault {
        let member_of = project.clone();
        let mut answered = 0;
        cached.workspace.bend_answers(
            move |operation, variables| lists_members_of(&member_of, operation, variables),
            fault.pages(),
            move |variables, data| {
                answered += 1;
                let issues = &mut data["issues"];
                match fault {
                    Fault::OverrunsThePage => {
                        let asked = usize::try_from(variables["first"].as_u64().expect("first"))
                            .expect("first fits usize");
                        let nodes = issues["nodes"].as_array_mut().expect("nodes");
                        let one = nodes.first().cloned().expect("a member to repeat");
                        nodes.resize(asked + 1, one);
                        // Saying more follow, so a walk trusting the page would ask for the next.
                        issues["pageInfo"] = json!({"hasNextPage": true, "endCursor": "1"});
                    }
                    Fault::RepeatsTheCursor => {
                        issues["nodes"] = json!([]);
                        issues["pageInfo"] = json!({"hasNextPage": true, "endCursor": "1"});
                    }
                    Fault::CyclesItsCursors => {
                        issues["nodes"] = json!([]);
                        let next = if answered % 2 == 1 { "1" } else { "2" };
                        issues["pageInfo"] = json!({"hasNextPage": true, "endCursor": next});
                    }
                }
            },
        );
    }
    let before = cached.workspace.served().len();
    let (again, _) = cached.ok(&copy);
    let sent = cached.workspace.served()[before..].to_vec();
    (again, sent, landed)
}

/// The listings of the project's members a re-copy sent, and the issues it read one by one.
fn members_listed_and_read(sent: &[Sent], landed: &[Outcome]) -> (usize, Vec<String>) {
    let project = landed[0].1.trim_start_matches("linear:");
    let listed = sent
        .iter()
        .filter(|(operation, variables)| lists_members_of(project, operation, variables))
        .count();
    let read = landed[1..]
        .iter()
        .map(|(_, destination, _)| destination.trim_start_matches("linear:").to_owned())
        .filter(|id| {
            sent.iter().any(|(operation, variables)| {
                operation.contains("issue(id:$id)") && variables["id"] == json!(id)
            })
        })
        .collect();
    (listed, read)
}

/// The re-copy bent by `fault` lands what an unbent one does, its walk stopped after the pages
/// the fault bent, and every task is read.
fn lands_as_if_unbent(fault: Fault) {
    let (unbent, unbent_sent, landed) = recopied(None);
    let (bent, bent_sent, bent_landed) = recopied(Some(fault));
    assert_eq!(
        outcomes(&bent),
        outcomes(&unbent),
        "the bent re-copy lands what an unbent one does: {bent:#}"
    );
    assert!(
        outcomes(&bent)
            .iter()
            .all(|(_, _, action)| action != "failed"),
        "{bent:#}"
    );
    let (unbent_listed, _) = members_listed_and_read(&unbent_sent, &landed);
    let (listed, read) = members_listed_and_read(&bent_sent, &bent_landed);
    assert_eq!(
        listed,
        unbent_listed - 1 + fault.pages(),
        "the walk stops at the page its guard refuses, then the orphan check lists the members \
         once as it always does"
    );
    let expected: Vec<String> = bent_landed[1..]
        .iter()
        .map(|(_, destination, _)| destination.trim_start_matches("linear:").to_owned())
        .collect();
    assert_eq!(read, expected, "every task is read before it is written");
}

#[test]
fn a_member_listing_overrunning_its_page_ends_the_pinning_and_the_recopy_lands() {
    lands_as_if_unbent(Fault::OverrunsThePage);
}

#[test]
fn a_member_listing_repeating_its_cursor_ends_the_pinning_and_the_recopy_lands() {
    lands_as_if_unbent(Fault::RepeatsTheCursor);
}

#[test]
fn a_member_listing_whose_cursors_cycle_ends_the_pinning_and_the_recopy_lands() {
    lands_as_if_unbent(Fault::CyclesItsCursors);
}
