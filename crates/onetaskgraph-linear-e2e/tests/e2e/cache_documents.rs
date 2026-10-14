//! Every query document the Linear plugin sends, with its source's cache off and on, held to
//! what 0.6.0 sent.
//!
//! `fixtures/documents-0.6.0.json` is every document of the plugin's `graphql` module at the tag
//! `v0.6.0`, extracted verbatim. With the cache off — `attach_cache` never having run — every
//! document the read and write verbs below send is one of those, byte for byte. With it on, the
//! only documents that differ are the seven the cache sends in place of `ISSUES`, `ISSUE`,
//! `ISSUE_DETAIL`, `ISSUE_RELATIONS`, `PROJECTS`, `PROJECT` and `PROJECT_RELATIONS`, each the one it stands in for with exactly the selections the cache's
//! token reads and nothing else — and a write's own read of its target sends the plain one,
//! since its answer fills no entry.

use std::collections::{BTreeMap, BTreeSet};

use onetaskgraph_linear::graphql;
use serde_json::Value;

use crate::cache::{Cached, dataset};

/// Every document 0.6.0's Linear plugin could send, by its constant's name.
fn released() -> BTreeMap<String, String> {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/documents-0.6.0.json")).expect("the fixture");
    serde_json::from_value(fixture["documents"].clone()).expect("documents by name")
}

/// The read and write verbs the Linear journeys drive, in an order each can follow the last.
fn verbs(cached: &Cached) -> Vec<Vec<String>> {
    let body = cached.sandbox.project().join("body.md");
    std::fs::write(&body, "A body written whole.").expect("a body");
    let body = body.display().to_string();
    let notes = cached.sandbox.project().join("notes");
    std::fs::write(
        notes.join("tasks/N-2.md"),
        "---\ntitle: Copied in\nstatus: todo\n---\nFrom the folder.\n",
    )
    .expect("a task to copy");
    std::fs::write(
        notes.join("documents/ND-1.md"),
        "---\ntitle: A note's design\n---\nFrom the folder.\n",
    )
    .expect("a document to copy");
    [
        "task list --source linear",
        "task show linear:L-1",
        "task show linear:L-1 --no-comments",
        "task show-many linear:L-1 linear:L-3",
        "task show-many linear:L-1 linear:L-3 --no-comments",
        "task comment list linear:L-1",
        "task deps linear:L-1",
        "task deps linear:L-2 --direction depended-on-by",
        "task status set linear:L-3 in-progress",
        "task priority set linear:L-3 low",
        "task metadata set linear:L-3 caller.note \"noted\"",
        "task update linear:L-3 --title Updated",
        "project list --source linear",
        "project show linear:P-1",
        "project deps linear:P-1",
        "project graph linear:P-1 --format json",
        "project metadata set linear:P-1 caller.note \"noted\"",
        "document list --source linear",
        "document show linear:D-1",
        "document metadata set linear:D-1 caller.note \"noted\"",
        "label list --source linear",
        "task copy notes:N-2 --to linear",
        "task copy notes:N-2 --to linear",
        "document copy notes:ND-1 --to linear",
        "task copy linear:L-1 --to notes --comments --no-link",
    ]
    .iter()
    .map(|line| line.split(' ').map(str::to_owned).collect::<Vec<_>>())
    .chain([
        vec![
            "task".to_owned(),
            "content".to_owned(),
            "set".to_owned(),
            "linear:L-4".to_owned(),
            "--file".to_owned(),
            body.clone(),
        ],
        vec![
            "task".to_owned(),
            "comment".to_owned(),
            "add".to_owned(),
            "linear:L-4".to_owned(),
            "--body-file".to_owned(),
            body,
        ],
    ])
    .collect()
}

/// Every distinct document Linear answered while `cached` ran each verb.
fn sent(cached: &Cached) -> BTreeSet<String> {
    for verb in verbs(cached) {
        let mut arguments: Vec<&str> = verb.iter().map(String::as_str).collect();
        arguments.push("--json");
        let output = cached.run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            crate::common::stderr(&output)
        );
    }
    cached
        .workspace
        .served()
        .into_iter()
        .map(|(document, _)| document)
        .collect()
}

#[test]
fn the_documents_0_6_0_sent_are_the_ones_this_build_sends_unchanged() {
    let released = released();
    let current = [
        ("ISSUE", graphql::ISSUE),
        ("ISSUES", graphql::ISSUES),
        ("ISSUE_DETAIL", graphql::ISSUE_DETAIL),
        ("ISSUE_RELATIONS", graphql::ISSUE_RELATIONS),
        ("ISSUE_COMMENTS", graphql::ISSUE_COMMENTS),
        ("PROJECT", graphql::PROJECT),
        ("PROJECTS", graphql::PROJECTS),
        ("PROJECT_RELATIONS", graphql::PROJECT_RELATIONS),
        ("DOCUMENT", graphql::DOCUMENT),
        ("DOCUMENTS", graphql::DOCUMENTS),
        ("LABELS", graphql::LABELS),
        ("VIEWER", graphql::VIEWER),
    ];
    for (name, document) in current {
        assert_eq!(
            released[name], document,
            "{name} is the document 0.6.0 sent"
        );
    }
}

#[test]
fn with_the_cache_off_every_document_is_one_0_6_0_sent_byte_for_byte() {
    let released: BTreeSet<String> = released().into_values().collect();
    let cached = Cached::with(dataset(), &serde_json::json!({"enabled": false}));
    let sent = sent(&cached);
    let unreleased: Vec<&String> = sent
        .iter()
        .filter(|sent| !released.contains(*sent))
        .collect();
    assert!(
        unreleased.is_empty(),
        "documents 0.6.0 never sent, with no cache attached: {unreleased:#?}"
    );
    for document in [
        graphql::ISSUES,
        graphql::ISSUE,
        graphql::ISSUE_DETAIL,
        graphql::ISSUE_RELATIONS,
    ] {
        assert!(sent.contains(document), "the verbs reach {document}");
    }
}

#[test]
fn with_the_cache_on_only_seven_documents_differ_each_by_exactly_what_the_cache_adds() {
    let released: BTreeSet<String> = released().into_values().collect();
    let cached = Cached::new(dataset());
    let sent = sent(&cached);
    // Each widened document, the plain one it stands in for, and every selection it adds — each
    // one a page or a part `change_token` spells a token from, as its own tests hold.
    let task_pages = [
        graphql::COMMENTS_PAGE,
        graphql::RELATIONS_PAGE,
        graphql::INVERSE_RELATIONS_PAGE,
    ];
    let widened: [(&str, &str, &[&str]); 7] = [
        (graphql::ISSUES_CACHED, graphql::ISSUES, &task_pages),
        (graphql::ISSUE_CACHED, graphql::ISSUE, &task_pages),
        (
            graphql::ISSUE_DETAIL_CACHED,
            graphql::ISSUE_DETAIL,
            &[graphql::RELATIONS_PAGE, graphql::INVERSE_RELATIONS_PAGE],
        ),
        (
            graphql::ISSUE_RELATIONS_CACHED,
            graphql::ISSUE_RELATIONS,
            &[graphql::TOKEN_PARTS],
        ),
        (
            graphql::PROJECTS_CACHED,
            graphql::PROJECTS,
            &[graphql::PROJECT_RELATIONS_PAGE],
        ),
        (
            graphql::PROJECT_CACHED,
            graphql::PROJECT,
            &[graphql::PROJECT_RELATIONS_PAGE],
        ),
        (
            graphql::PROJECT_RELATIONS_CACHED,
            graphql::PROJECT_RELATIONS,
            &[graphql::PROJECT_TOKEN_PARTS],
        ),
    ];
    for (cache, plain, added) in widened {
        let mut stripped = cache.to_owned();
        for added in added {
            assert_eq!(
                stripped.matches(&format!(" {added}")).count(),
                1,
                "{cache} carries `{added}` once"
            );
            stripped = stripped.replacen(&format!(" {added}"), "", 1);
        }
        assert_eq!(stripped, plain, "{cache} is {plain} with exactly {added:?}");
        assert!(sent.contains(cache), "the verbs send {cache}");
    }
    let other: Vec<&String> = sent
        .iter()
        .filter(|sent| {
            !released.contains(*sent) && !widened.iter().any(|(cache, _, _)| cache == sent)
        })
        .collect();
    assert!(
        other.is_empty(),
        "documents no read sent before: {other:#?}"
    );
}
