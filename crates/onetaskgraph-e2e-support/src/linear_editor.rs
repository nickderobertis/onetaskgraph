//! The editor state the loopback Linear workspace answers each record with, in the shape real
//! Linear stores it.
//!
//! Linear keeps every issue's description and every project's and document's content as a Yjs
//! document beside its Markdown — `descriptionState` and `contentState`, base64 of a Yjs
//! update — whose `prosemirror` fragment holds one `heading` node per heading, carrying a
//! `level` and an `id` UUID Linear assigns when the record is written. Observed against team
//! `TES2` on 2026-11-02: every create's and update's own answer carries it, and an update keeps
//! the id of every heading whose text it leaves alone, a heading it adds getting a fresh one.
//! [`restated`] does exactly that here, so a journey reads ids that behave as Linear's do, and a
//! journey that needs Linear's own bytes seeds a record with a captured sample instead
//! ([`CAPTURED`]).

use base64::Engine as _;
use yrs::types::xml::{XmlElementPrelim, XmlTextPrelim};
use yrs::{Doc, ReadTxn, Transact, Xml, XmlFragment};

/// Editor states captured from real `TES2` records, each with the record it came from.
pub const CAPTURED: &[(&str, &str)] = &[
    (
        "TES2-773",
        include_str!("../../onetaskgraph-linear/tests/fixtures/editor-state/tes2-773.json"),
    ),
    (
        "section-links-spike-plain-document",
        include_str!(
            "../../onetaskgraph-linear/tests/fixtures/editor-state/spike-plain-document.json"
        ),
    ),
];

/// The captured sample named `name`: its Markdown, its base64 state, and each heading's text,
/// id, expected fragment and that fragment's source, as the fixture states them.
#[must_use]
pub fn captured(name: &str) -> serde_json::Value {
    let (_, raw) = CAPTURED
        .iter()
        .find(|(held, _)| *held == name)
        .unwrap_or_else(|| panic!("no captured editor state {name}"));
    serde_json::from_str(raw).expect("a captured editor state fixture is JSON")
}

/// Every ATX heading of `content`, level and text, outside fenced code — as Linear's editor
/// reads the Markdown it is written.
fn headings(content: &str) -> Vec<(u32, String)> {
    let mut found = Vec::new();
    let mut fence: Option<char> = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = trimmed.chars().next().unwrap_or('`');
            fence = match fence {
                None => Some(marker),
                Some(open) if open == marker => None,
                held => held,
            };
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let marks = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&marks) && trimmed[marks..].starts_with(' ') {
            let text = trimmed[marks..]
                .trim()
                .trim_end_matches('#')
                .trim()
                .to_owned();
            found.push((u32::try_from(marks).unwrap_or(1), text));
        }
    }
    found
}

/// One heading id, shaped as Linear's are, made from `seed` so a run is reproducible.
fn fresh_id(seed: &str) -> String {
    // Two FNV-1a passes over the seed, read as 32 hex digits.
    let mut hex = String::new();
    for salt in [0x811c_9dc5_u64, 0xcbf2_9ce4_8422_2325] {
        let mut hash = salt;
        for byte in seed.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hex.push_str(&format!("{hash:016x}"));
    }
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

/// The headings `state` holds, text and id, in order.
#[must_use]
pub fn stored(state: &str) -> Vec<(String, String)> {
    onetaskgraph_linear::headings::decoded(state)
        .unwrap_or_default()
        .into_iter()
        .map(|heading| (heading.text, heading.id))
        .collect()
}

/// The state a record holding `content` is stored with once written over `before` — its
/// previous state, when it had one: every heading keeps the id the heading of the same text at
/// the same position among those of its text had, and every other heading gets a fresh one
/// made from `seed`.
#[must_use]
pub fn restated(before: Option<&str>, content: &str, seed: &str) -> String {
    let held = before.map(stored).unwrap_or_default();
    let mut seen: Vec<String> = Vec::new();
    let mut ids = Vec::new();
    for (index, (level, text)) in headings(content).into_iter().enumerate() {
        let ordinal = seen.iter().filter(|earlier| **earlier == text).count();
        seen.push(text.clone());
        let id = held
            .iter()
            .filter(|(held_text, _)| *held_text == text)
            .nth(ordinal)
            .map_or_else(
                || fresh_id(&format!("{seed}/{index}/{text}")),
                |(_, id)| id.clone(),
            );
        ids.push((level, text, id));
    }
    encoded(&ids)
}

/// A Yjs update holding one `heading` node per entry, as Linear's editor state holds them.
fn encoded(headings: &[(u32, String, String)]) -> String {
    let doc = Doc::new();
    let fragment = doc.get_or_insert_xml_fragment("prosemirror");
    {
        let mut txn = doc.transact_mut();
        for (index, (level, text, id)) in headings.iter().enumerate() {
            let heading = fragment.insert(
                &mut txn,
                u32::try_from(index).unwrap_or(u32::MAX),
                XmlElementPrelim::empty("heading"),
            );
            heading.insert_attribute(&mut txn, "level", f64::from(*level));
            heading.insert_attribute(&mut txn, "id", id.as_str());
            heading.insert(&mut txn, 0, XmlTextPrelim::new(text.as_str()));
        }
    }
    let update = doc
        .transact()
        .encode_state_as_update_v1(&yrs::StateVector::default());
    base64::engine::general_purpose::STANDARD.encode(update)
}

/// Whether `state` decodes to the headings `content` holds, in order — a captured sample
/// served for exactly the Markdown it was captured with.
#[must_use]
pub fn holds(state: &str, content: &str) -> bool {
    let texts: Vec<String> = headings(content)
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    let held: Vec<String> = stored(state).into_iter().map(|(text, _)| text).collect();
    texts == held
}
