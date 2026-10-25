//! The loopback Linear stores each committed sample as spike `spike-linear-stored-content`
//! measured real Linear storing it.
//!
//! Every sample is one probe the spike wrote to real Linear, under
//! `tests/fixtures/linear_rewrites/`, whose README names the report each comes from; the line each
//! case names is where that report says Linear's stored form first parted from what was sent,
//! and the text is what Linear stored there. The journeys of this suite compose their plan bodies
//! from these samples, so what they prove holds of a destination rewriting them as Linear does.

use onetaskgraph_e2e_support::linear_markdown::stored;

/// A committed sample, by its file name.
pub(crate) fn sample(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/linear_rewrites/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .trim_end_matches('\n')
        .to_owned()
}

/// Every sample but the images', as the spike sent it — the bodies a plan is composed from.
pub(crate) const SAMPLES: [&str; 8] = [
    "pipe-table.md",
    "bullet-list.md",
    "indented-code.md",
    "code-span-wrap.md",
    "bold-wrap.md",
    "lazy-ordered-item.md",
    "list-after-heading.md",
    "list-after-paragraph.md",
];

/// The first line, counted from 1, at which `stored` parts from `sent`, and its text there.
fn first_difference(sent: &str, stored: &str) -> Option<(usize, String)> {
    let sent: Vec<&str> = sent.split('\n').collect();
    let stored: Vec<&str> = stored.split('\n').collect();
    (0..sent.len().max(stored.len()))
        .find(|&at| sent.get(at) != stored.get(at))
        .map(|at| {
            (
                at + 1,
                stored.get(at).copied().unwrap_or("<absent>").to_owned(),
            )
        })
}

#[test]
fn each_sample_is_stored_where_and_as_the_spike_measured_real_linear_storing_it() {
    for (name, line, text) in [
        ("pipe-table.md", 2, "| -- | -- |"),
        ("bullet-list.md", 1, "* first item"),
        ("indented-code.md", 3, "```"),
        (
            "code-span-wrap.md",
            1,
            "See `crates/onetaskgraph-linear/src/lib.rs` and a `code span that wraps across a \
             line break` in prose.",
        ),
        (
            "bold-wrap.md",
            1,
            "This sentence has **bold text that spans**",
        ),
        (
            "lazy-ordered-item.md",
            1,
            "* a list item whose text wraps onto",
        ),
        ("list-after-heading.md", 2, ""),
        ("list-after-paragraph.md", 2, ""),
        ("adjacent-images.md", 2, ""),
    ] {
        let sent = sample(name);
        let kept = stored(&sent);
        assert_eq!(
            first_difference(&sent, &kept),
            Some((line, text.to_owned())),
            "{name} is stored as the spike measured:\n{kept}"
        );
        assert_eq!(
            stored(&kept),
            kept,
            "{name}: a follow-up write of what was stored is stored as it was sent"
        );
    }
    // The two rewrites the report names beyond the first difference of their probe.
    assert!(
        stored(&sample("pipe-table.md")).ends_with("| three | four |\n"),
        "a table ending the content keeps its last row's line break"
    );
    assert!(
        stored(&sample("lazy-ordered-item.md"))
            .contains("\n  3\\. a lazy continuation line that opens like an ordered item\n"),
        "the lazy continuation's ordered marker is escaped under its item"
    );
}
