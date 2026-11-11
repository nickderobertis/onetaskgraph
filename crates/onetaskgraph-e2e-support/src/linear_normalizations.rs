//! The recorded Linear normalizations: what real Linear stored for each Markdown spike
//! `spike-expandable-blocks` wrote it, committed unchanged under
//! `fixtures/expandable_blocks/` of this crate from the spike's `normalizations.json` and
//! `fixtures/` (that directory's `README.md` names the commit).
//!
//! It is the one set three things read, and none of them keeps a list of its own:
//!
//! - the loopback Linear's [`stored`](crate::linear_markdown::stored), which this module's tests
//!   hold to storing every recorded fixture's sent Markdown exactly as Linear stored it;
//! - a journey that accounts for every line a chain of copies through Linear changed, by
//!   [`unaccounted`], which finds the one recorded entry each changed line is an instance of;
//! - a journey that sends a shape the authored form does not recognise, which reads the
//!   fixture holding that shape, by [`fixture`].

use std::collections::BTreeSet;

use serde_json::Value;

/// One recorded fixture: a record the spike wrote through Linear's API, and what Linear stored.
#[derive(Debug, Clone)]
pub struct Recorded {
    /// Its file's name, without `.json`: `issue-authored`, `document-linear-plus`, ….
    pub name: &'static str,
    /// The Markdown the spike sent.
    pub sent: String,
    /// The Markdown Linear stored.
    pub stored: String,
    /// The editor state's node types Linear stored it as, as the spike decoded them.
    pub node_types: Value,
}

/// Each recorded fixture's file, by name.
const FIXTURES: [(&str, &str); 17] = [
    (
        "document-all-forms",
        include_str!("../fixtures/expandable_blocks/fixtures/document-all-forms.json"),
    ),
    (
        "document-authored",
        include_str!("../fixtures/expandable_blocks/fixtures/document-authored.json"),
    ),
    (
        "document-details-open",
        include_str!("../fixtures/expandable_blocks/fixtures/document-details-open.json"),
    ),
    (
        "document-in-fenced-code",
        include_str!("../fixtures/expandable_blocks/fixtures/document-in-fenced-code.json"),
    ),
    (
        "document-in-list-item",
        include_str!("../fixtures/expandable_blocks/fixtures/document-in-list-item.json"),
    ),
    (
        "document-linear-arrow",
        include_str!("../fixtures/expandable_blocks/fixtures/document-linear-arrow.json"),
    ),
    (
        "document-linear-plus",
        include_str!("../fixtures/expandable_blocks/fixtures/document-linear-plus.json"),
    ),
    (
        "document-no-blank-after-summary",
        include_str!("../fixtures/expandable_blocks/fixtures/document-no-blank-after-summary.json"),
    ),
    (
        "document-sample-after",
        include_str!("../fixtures/expandable_blocks/fixtures/document-sample-after.json"),
    ),
    (
        "issue-all-forms",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-all-forms.json"),
    ),
    (
        "issue-authored",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-authored.json"),
    ),
    (
        "issue-details-open",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-details-open.json"),
    ),
    (
        "issue-in-fenced-code",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-in-fenced-code.json"),
    ),
    (
        "issue-in-list-item",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-in-list-item.json"),
    ),
    (
        "issue-linear-arrow",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-linear-arrow.json"),
    ),
    (
        "issue-linear-plus",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-linear-plus.json"),
    ),
    (
        "issue-no-blank-after-summary",
        include_str!("../fixtures/expandable_blocks/fixtures/issue-no-blank-after-summary.json"),
    ),
];

/// The spike's `normalizations.json`.
const NORMALIZATIONS: &str = include_str!("../fixtures/expandable_blocks/normalizations.json");

/// Every recorded fixture, in name order.
///
/// # Panics
///
/// When a committed fixture is not the JSON the spike wrote, which no run can recover from.
#[must_use]
pub fn recorded() -> Vec<Recorded> {
    FIXTURES
        .iter()
        .map(|(name, text)| {
            let fixture: Value = serde_json::from_str(text)
                .unwrap_or_else(|error| panic!("fixture {name} is JSON: {error}"));
            let field = |key: &str| {
                fixture[key]
                    .as_str()
                    .unwrap_or_else(|| panic!("fixture {name} records {key}"))
                    .to_owned()
            };
            Recorded {
                name,
                sent: field("sent_markdown"),
                stored: field("stored_markdown"),
                node_types: fixture["node_types"].clone(),
            }
        })
        .collect()
}

/// The recorded fixture named `name`.
///
/// # Panics
///
/// When no fixture has that name.
#[must_use]
pub fn fixture(name: &str) -> Recorded {
    recorded()
        .into_iter()
        .find(|recorded| recorded.name == name)
        .unwrap_or_else(|| panic!("no recorded fixture {name}"))
}

/// One entry of `normalizations.json`: lines sent, and the lines Linear stored for them.
#[derive(Debug, Clone)]
pub struct Normalization {
    /// The fixture it was recorded from, as the file names it.
    pub fixture: String,
    /// The lines sent.
    pub sent: Vec<String>,
    /// The lines stored.
    pub stored: Vec<String>,
}

/// Every recorded normalization.
///
/// # Panics
///
/// When the committed file is not the JSON the spike wrote.
#[must_use]
pub fn normalizations() -> Vec<Normalization> {
    let file: Value = serde_json::from_str(NORMALIZATIONS).expect("normalizations.json is JSON");
    let lines = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .expect("an entry's lines")
            .iter()
            .map(|line| line.as_str().expect("a line").to_owned())
            .collect()
    };
    file["normalizations"]
        .as_array()
        .expect("the normalizations")
        .iter()
        .map(|entry| Normalization {
            fixture: entry["fixture"].as_str().expect("its fixture").to_owned(),
            sent: lines(&entry["sent"]),
            stored: lines(&entry["stored"]),
        })
        .collect()
}

/// What one change of a run of lines does, with the lines' own text left out: the edits each
/// line holding something underwent, and how the run's blank lines moved. Two changes of this
/// shape are one normalization applied to different text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shape {
    /// Every edit — text taken out, text put in — of a line holding something.
    edits: BTreeSet<(String, String)>,
    /// Whether the run lost blank lines (`Less`), gained some (`Greater`), or kept their count.
    blanks: std::cmp::Ordering,
    /// Whether a line of whitespace alone became empty.
    emptied: bool,
}

/// The shape of the change from `sent` to `stored`, when its lines holding something pair up
/// one for one.
fn shape(sent: &[String], stored: &[String]) -> Option<Shape> {
    let holding = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .cloned()
            .collect()
    };
    let (before, after) = (holding(sent), holding(stored));
    if before.len() != after.len() {
        return None;
    }
    let edits = before
        .iter()
        .zip(&after)
        .flat_map(|(a, b)| edits(a, b))
        .collect();
    let blank = |lines: &[String]| lines.iter().filter(|line| line.trim().is_empty()).count();
    Some(Shape {
        edits,
        blanks: blank(stored).cmp(&blank(sent)),
        emptied: sent
            .iter()
            .any(|line| !line.is_empty() && line.trim().is_empty()),
    })
}

/// The edits taking `a` to `b`: each maximal run of characters taken out and put in between two
/// characters the two lines share, by their longest common subsequence.
fn edits(a: &str, b: &str) -> Vec<(String, String)> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut table = vec![vec![0_usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut found = Vec::new();
    let mut pending = (String::new(), String::new());
    let flush = |pending: &mut (String, String), found: &mut Vec<(String, String)>| {
        if !pending.0.is_empty() || !pending.1.is_empty() {
            found.push(std::mem::take(pending));
        }
    };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            flush(&mut pending, &mut found);
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || table[i][j + 1] >= table[i + 1][j]) {
            pending.1.push(b[j]);
            j += 1;
        } else {
            pending.0.push(a[i]);
            i += 1;
        }
    }
    flush(&mut pending, &mut found);
    found
}

/// One run of lines a line diff found changed: the lines taken out, and the lines put in.
pub type Hunk = (Vec<String>, Vec<String>);

/// Every run of lines in which `stored` differs from `sent`, found as the spike found the entries
/// of `normalizations.json`: Python's `difflib.SequenceMatcher` over the lines, with no junk
/// heuristic — each matching run the longest one left, the earliest of equal length first.
#[must_use]
pub fn hunks(sent: &str, stored: &str) -> Vec<Hunk> {
    // Split on every line break, as the spike split: a content ending in one has an empty last
    // line, which a store dropping that break leaves out.
    let a: Vec<&str> = sent.split('\n').collect();
    let b: Vec<&str> = stored.split('\n').collect();
    let mut at: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (index, line) in b.iter().enumerate() {
        at.entry(line).or_default().push(index);
    }
    let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| {
        let (mut best_i, mut best_j, mut best) = (alo, blo, 0_usize);
        let mut runs: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for (i, line) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut next = std::collections::HashMap::new();
            for &j in at.get(line).map_or(&[][..], Vec::as_slice) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let run = j
                    .checked_sub(1)
                    .and_then(|before| runs.get(&before))
                    .map_or(0, |run| *run)
                    + 1;
                next.insert(j, run);
                if run > best {
                    (best_i, best_j, best) = (i + 1 - run, j + 1 - run, run);
                }
            }
            runs = next;
        }
        (best_i, best_j, best)
    };
    let mut matching = Vec::new();
    let mut queue = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, size) = longest(alo, ahi, blo, bhi);
        if size > 0 {
            matching.push((i, j, size));
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + size < ahi && j + size < bhi {
                queue.push((i + size, ahi, j + size, bhi));
            }
        }
    }
    matching.sort_unstable();
    matching.push((a.len(), b.len(), 0));
    let (mut i, mut j) = (0, 0);
    let mut found = Vec::new();
    for (ai, bj, size) in matching {
        if i < ai || j < bj {
            found.push((
                a[i..ai].iter().map(|line| (*line).to_owned()).collect(),
                b[j..bj].iter().map(|line| (*line).to_owned()).collect(),
            ));
        }
        (i, j) = (ai + size, bj + size);
    }
    found
}

/// The recorded entry `hunk` is an instance of: one whose change has its shape — each of its
/// edits one the entry made, its blank lines moving as the entry's did or not at all, and a line
/// of whitespace emptied only where the entry emptied one.
#[must_use]
pub fn accounting_entry(hunk: &Hunk) -> Option<Normalization> {
    static SHAPED: std::sync::OnceLock<Vec<(Normalization, Option<Shape>)>> =
        std::sync::OnceLock::new();
    let shaped = SHAPED.get_or_init(|| {
        normalizations()
            .into_iter()
            .map(|entry| {
                let shaped = shape(&entry.sent, &entry.stored);
                (entry, shaped)
            })
            .collect()
    });
    // A change whose lines do not pair up one for one is accounted for only by an entry recording
    // those very lines.
    let Some(wanted) = shape(&hunk.0, &hunk.1) else {
        return shaped
            .iter()
            .find(|(entry, _)| entry.sent == hunk.0 && entry.stored == hunk.1)
            .map(|(entry, _)| entry.clone());
    };
    shaped
        .iter()
        .find(|(_, recorded)| {
            recorded.as_ref().is_some_and(|recorded| {
                wanted.edits.is_subset(&recorded.edits)
                    && (wanted.blanks == std::cmp::Ordering::Equal
                        || wanted.blanks == recorded.blanks)
                    && (!wanted.emptied || recorded.emptied)
            })
        })
        .map(|(entry, _)| entry.clone())
}

/// Every run of lines in which `stored` differs from `sent` that no recorded entry accounts for.
#[must_use]
pub fn unaccounted(sent: &str, stored: &str) -> Vec<Hunk> {
    hunks(sent, stored)
        .into_iter()
        .filter(|hunk| accounting_entry(hunk).is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{fixture, normalizations, recorded, unaccounted};
    use crate::linear_markdown::stored;

    /// The loopback Linear stores every recorded fixture's sent Markdown exactly as Linear
    /// stored it, and storing that again changes nothing, as the spike recorded of Linear.
    #[test]
    fn the_loopback_stores_every_recorded_fixture_as_linear_did() {
        let all = recorded();
        assert_eq!(all.len(), 17);
        for fixture in all {
            assert_eq!(
                stored(&fixture.sent),
                fixture.stored,
                "{}: the loopback stores what Linear stored",
                fixture.name
            );
            assert_eq!(
                super::hunks(&fixture.stored, &stored(&fixture.stored)),
                Vec::new(),
                "{}: and storing it again changes nothing",
                fixture.name
            );
        }
    }

    /// Every recorded fixture's own change is accounted for by the recorded entries, and a
    /// change none of them made — a word dropped, a line moved, a summary changed — is not.
    #[test]
    fn every_recorded_change_is_accounted_for_and_a_change_of_meaning_is_not() {
        assert_eq!(normalizations().len(), 542);
        for fixture in recorded() {
            assert_eq!(
                unaccounted(&fixture.sent, &fixture.stored),
                Vec::new(),
                "{}",
                fixture.name
            );
        }
        let authored = fixture("issue-authored").sent;
        for (what, changed) in [
            (
                "a word dropped",
                authored.replacen("End of probe A07.", "End of A07.", 1),
            ),
            (
                "a summary changed",
                authored.replacen("Plain summary text", "Plain summary", 1),
            ),
            (
                "a line moved out of its block",
                authored.replacen(
                    "End of probe A01.\n\n</details>",
                    "</details>\n\nEnd of probe A01.",
                    1,
                ),
            ),
            (
                "a block opened expanded",
                authored.replacen("<details>", "<details open>", 1),
            ),
        ] {
            assert_ne!(changed, authored, "{what}");
            assert!(
                !unaccounted(&authored, &changed).is_empty(),
                "{what} is not a recorded normalization"
            );
        }
    }
}
