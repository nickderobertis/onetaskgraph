//! A store an earlier release wrote, read and written by this one.
//!
//! A local Markdown store is a long-lived directory on a user's disk, often under version
//! control. `tests/fixtures/plan-store` holds copies of real records of this project's own
//! plan store with every word of their text, keys and file names replaced by a placeholder
//! word of the same length and case and every other byte kept, so each record keeps the
//! shape YAML writers disagree on while carrying none of its text: strings that need quotes,
//! a timestamp-shaped string, blank lines inside literal blocks, a block opening with a blank
//! line (`|2-`), a multi-line string holding a whitespace-only line, lines past eighty
//! columns, a `|+` block, stored answers blocks holding those, edges, `delivers`, nested
//! metadata, projects and documents. No record of that store carries a label, so
//! `placeholder-project` holds the labelled ones: a project and two tasks written by the
//! release before the YAML crate moved, through this source, with labels that need quoting
//! (`needs: quoting`, `#triage`, `1.0`, `007`) and labels that do not (`2026-Q4`, `yes`), and
//! a task whose labels are written by hand in the short forms the documentation shows.
//! `plan-store.expected.json` is what that release observed of every record — each read,
//! rewritten whole, given a status, and its title and status spliced back — so a difference
//! is a record this release reads with another meaning or writes with other bytes.
mod capture;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// What the earlier release observed, under `part`.
fn expected(part: &str) -> BTreeMap<String, Value> {
    let text = std::fs::read_to_string(fixture("plan-store.expected.json"))
        .expect("the expected observations");
    let mut all: BTreeMap<String, BTreeMap<String, Value>> =
        serde_json::from_str(&text).expect("the expected observations are JSON");
    all.remove(part).expect("the part is recorded")
}

/// Every record whose observation differs, each with the first line where it does.
fn differences(
    observed: &BTreeMap<String, Value>,
    expected: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut found = Vec::new();
    for (record, wanted) in expected {
        let Some(got) = observed.get(record) else {
            found.push(format!("{record}: not observed"));
            continue;
        };
        if got == wanted {
            continue;
        }
        let shown = |value: &Value| serde_json::to_string_pretty(value).expect("renders");
        let (got, wanted) = (shown(got), shown(wanted));
        let line = got
            .lines()
            .zip(wanted.lines())
            .enumerate()
            .find(|(_, (got, wanted))| got != wanted)
            .map_or_else(
                || "one observation ends early".to_owned(),
                |(at, (got, wanted))| format!("line {}: now {got}\n    was {wanted}", at + 1),
            );
        found.push(format!("{record}: {line}"));
    }
    found.extend(
        observed
            .keys()
            .filter(|record| !expected.contains_key(*record))
            .map(|record| format!("{record}: not expected")),
    );
    found
}

#[tokio::test]
async fn every_record_an_earlier_release_wrote_reads_with_the_same_meaning() {
    let observed = capture::reads(&fixture("plan-store")).await;
    let expected = expected("reads");
    assert_eq!(observed.len(), 15, "every record of the store is read");
    let found = differences(&observed, &expected);
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[tokio::test]
async fn rewriting_and_splicing_real_records_keeps_the_bytes_the_earlier_release_wrote() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let observed = capture::writes(&fixture("plan-store"), scratch.path()).await;
    let expected = expected("writes");
    let found = differences(&observed, &expected);
    assert!(found.is_empty(), "{}", found.join("\n"));
}
