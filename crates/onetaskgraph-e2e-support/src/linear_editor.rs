//! The editor state the loopback Linear workspace answers each record with, in the shape real
//! Linear stores it.
//!
//! Linear keeps every issue's description and every project's and document's content as a Yjs
//! document beside its Markdown — `descriptionState` and `contentState`, base64 of a Yjs
//! update — whose `prosemirror` fragment holds the ProseMirror tree Linear's editor shows: one
//! `heading` node per heading, carrying a `level` and an `id` UUID Linear assigns when the
//! record is written, and a node per paragraph, table, list and collapsible section. Observed
//! against team `TES2` on 2026-11-02: every create's and update's own answer carries it, and an
//! update keeps the id of every heading whose text it leaves alone, a heading it adds getting a
//! fresh one. [`restated`] does exactly that here, so a journey reads ids that behave as
//! Linear's do, and a journey that needs Linear's own bytes seeds a record with a captured
//! sample instead ([`CAPTURED`]).
//!
//! **What a person sees.** [`node_tree`] is the tree a stored Markdown form becomes, by the node
//! shapes spike `spike-linear-collapsed-slot` captured from `TES2` (`fixtures/collapsed_slot/`,
//! which this module's tests hold it to): an HTML comment is a visible paragraph of its literal
//! text — `<!-- onetaskgraph.metadata ` unmarked, the payload marked `code`, ` -->` unmarked —
//! and a `+++ <summary>` … `+++` section is a top-level `collapsible_section` (attributes `id`
//! and, only sometimes, `wasCopiedWhileOpen`) holding a `collapsible_heading > paragraph` with
//! the summary and then its body's blocks. Linear stores no open state, and its web client opens
//! such a section collapsed for any reader who has not opened it in that browser. So
//! [`closed_text`] is the text a person sees with every collapsed node closed, and it is what the
//! journeys read to hold that no store-written metadata or HTML comment is in sight.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Map, Value, json};
use yrs::types::xml::{XmlElementPrelim, XmlOut, XmlTextPrelim};
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, Out, ReadTxn, Text, Transact, Update, Xml, XmlFragment};

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
/// previous state, when it had one: the whole [`node_tree`] of `content`, in which every heading
/// keeps the id the heading of the same text at the same position among those of its text had,
/// and every other heading gets a fresh one made from `seed`.
///
/// A content whose tree does not hold exactly the headings Linear's heading ids are read for —
/// one indented, or under a fence another way — is stored as its headings alone, as every state
/// was before the tree was modelled, so no heading id a journey reads moves.
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
    let mut tree = node_tree(content, seed);
    let mut found = Vec::new();
    tree_headings(&tree, &mut found);
    if found
        != ids
            .iter()
            .map(|(level, text, _)| (*level, text.clone()))
            .collect::<Vec<_>>()
    {
        return encoded(&ids);
    }
    let mut next = ids.iter().map(|(_, _, id)| id.clone());
    give_heading_ids(&mut tree, &mut next);
    encode_tree(&tree)
}

/// Every heading of `node`, level and text, in document order.
fn tree_headings(node: &Value, found: &mut Vec<(u32, String)>) {
    if node["type"] == "heading" {
        let level = node["attrs"]["level"].as_f64().unwrap_or(1.0);
        found.push((level as u32, run_text(node)));
    }
    for child in node["content"].as_array().into_iter().flatten() {
        tree_headings(child, found);
    }
}

/// `node` with each heading, in document order, given the next of `ids`.
fn give_heading_ids(node: &mut Value, ids: &mut impl Iterator<Item = String>) {
    if node["type"] == "heading"
        && let Some(id) = ids.next()
    {
        node["attrs"]["id"] = json!(id);
    }
    if let Some(children) = node["content"].as_array_mut() {
        for child in children {
            give_heading_ids(child, ids);
        }
    }
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

/// The ProseMirror tree `markdown` — a long form as Linear stores it — becomes in Linear's editor,
/// in the node shapes spike `spike-linear-collapsed-slot` captured (`{"type", "attrs",
/// "content"}`, text as `{"type": "#text", "runs": [{"marks", "text"}]}`), every node id made from
/// `seed`. A collapsible section carries `wasCopiedWhileOpen` as `null`, as Linear writes it on an
/// issue's and a project's; it leaves it out on a document's sometimes, which
/// [`same_tree`] accepts.
#[must_use]
pub fn node_tree(markdown: &str, seed: &str) -> Value {
    let lines: Vec<&str> = markdown.split('\n').collect();
    json!({"type": "doc", "content": blocks(&lines, seed)})
}

/// The text a person sees of `tree` with every collapsed node closed: each top-level block's text
/// on a line of its own, a collapsible section's summary standing for the whole of it.
#[must_use]
pub fn closed_text(tree: &Value) -> String {
    tree["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|node| {
            if node["type"] == "collapsible_section" {
                node["content"]
                    .as_array()
                    .and_then(|content| content.first())
                    .map(run_text)
                    .unwrap_or_default()
            } else {
                run_text(node)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The ProseMirror tree a base64 Yjs editor state holds, in [`node_tree`]'s shapes — `None` for a
/// state that does not decode. Linear's `attribution` formatting, and a mark it set to `null`,
/// are not markup, and are dropped, the runs they alone separated merged.
#[must_use]
pub fn decoded_tree(state: &str) -> Option<Value> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(state)
        .ok()?;
    let update = Update::decode_v1(&bytes).ok()?;
    let doc = Doc::new();
    let fragment = doc.get_or_insert_xml_fragment("prosemirror");
    doc.transact_mut().apply_update(update).ok()?;
    let txn = doc.transact();
    let content: Vec<Value> = (0..fragment.len(&txn))
        .filter_map(|index| fragment.get(&txn, index))
        .map(|node| decoded_node(&txn, node))
        .collect();
    Some(json!({"type": "doc", "content": content}))
}

/// Whether two trees are the same tree as Linear stores it: equal but for each node's `id`, which
/// Linear assigns when it stores the record, and a collapsible section's `wasCopiedWhileOpen`
/// held as `null` or left out.
#[must_use]
pub fn same_tree(left: &Value, right: &Value) -> bool {
    comparable(left) == comparable(right)
}

/// `node` with every `id` attribute blanked and every `null` `wasCopiedWhileOpen` left out.
fn comparable(node: &Value) -> Value {
    match node {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let value = if key == "attrs" {
                        let mut attrs = value.as_object().cloned().unwrap_or_default();
                        if attrs.contains_key("id") {
                            attrs.insert("id".into(), json!("<id>"));
                        }
                        if attrs.get("wasCopiedWhileOpen") == Some(&Value::Null) {
                            attrs.remove("wasCopiedWhileOpen");
                        }
                        Value::Object(attrs)
                    } else {
                        comparable(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(comparable).collect()),
        other => other.clone(),
    }
}

/// Every run of text under `node`, joined: a paragraph's, a heading's, and a table's or a list's
/// cells and items each on a line of its own.
fn run_text(node: &Value) -> String {
    if node["type"] == "#text" {
        return node["runs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|run| run["text"].as_str())
            .collect();
    }
    node["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(run_text)
        .collect::<Vec<_>>()
        .join(
            if node["type"] == "paragraph" || node["type"] == "heading" {
                ""
            } else {
                "\n"
            },
        )
}

/// A line's inline runs: code spans, `**strong**`, and text, adjacent runs of one mark merged.
fn inline(text: &str) -> Value {
    let mut runs: Vec<(Value, String)> = Vec::new();
    let mut push = |marks: Value, piece: &str| {
        if piece.is_empty() {
            return;
        }
        match runs.last_mut() {
            Some((held, text)) if *held == marks => text.push_str(piece),
            _ => runs.push((marks, piece.to_owned())),
        }
    };
    let mut rest = text;
    while !rest.is_empty() {
        let tick = rest.find('`');
        let strong = rest.find("**");
        match (tick, strong) {
            (Some(at), strong) if strong.is_none_or(|strong| at < strong) => {
                let Some(close) = rest[at + 1..].find('`') else {
                    push(json!({}), rest);
                    break;
                };
                push(json!({}), &rest[..at]);
                push(json!({"code": {}}), &rest[at + 1..at + 1 + close]);
                rest = &rest[at + 2 + close..];
            }
            (_, Some(at)) => {
                let Some(close) = rest[at + 2..].find("**") else {
                    push(json!({}), rest);
                    break;
                };
                push(json!({}), &rest[..at]);
                push(json!({"strong": {}}), &rest[at + 2..at + 2 + close]);
                rest = &rest[at + 4 + close..];
            }
            _ => {
                push(json!({}), rest);
                break;
            }
        }
    }
    let runs: Vec<Value> = runs
        .into_iter()
        .map(|(marks, text)| json!({"marks": marks, "text": text}))
        .collect();
    json!({"type": "#text", "runs": runs})
}

fn paragraph(text: &str) -> Value {
    json!({"type": "paragraph", "attrs": {}, "content": [inline(text)]})
}

/// Whether `line` is a bullet or a numbered list item.
fn list_item(line: &str) -> bool {
    line.starts_with("- ")
        || line.starts_with("* ")
        || line.split_once(". ").is_some_and(|(number, _)| {
            !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
        })
}

/// The top-level blocks `lines` hold, as Linear's editor stores them.
fn blocks(lines: &[&str], seed: &str) -> Vec<Value> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let seed = format!("{seed}/{index}");
        if line.trim().is_empty() {
            index += 1;
        } else if let Some(summary) = line.strip_prefix("+++ ") {
            let (mut depth, mut close, mut fence) = (1, index + 1, false);
            while close < lines.len() {
                if lines[close].starts_with("```") {
                    fence = !fence;
                } else if !fence && lines[close].starts_with("+++ ") {
                    depth += 1;
                } else if !fence && lines[close] == "+++" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                close += 1;
            }
            let mut content = vec![json!({
                "type": "collapsible_heading", "attrs": {}, "content": [paragraph(summary)]
            })];
            content.extend(blocks(&lines[index + 1..close.min(lines.len())], &seed));
            found.push(json!({
                "type": "collapsible_section",
                "attrs": {"id": fresh_id(&seed), "wasCopiedWhileOpen": null},
                "content": content
            }));
            index = close + 1;
        } else if let Some(language) = line.strip_prefix("```") {
            let close = lines[index + 1..]
                .iter()
                .position(|held| *held == "```")
                .map_or(lines.len(), |at| index + 1 + at);
            let code = lines[index + 1..close.min(lines.len())].join("\n");
            found.push(json!({
                "type": "code_block", "attrs": {"id": fresh_id(&seed), "language": language},
                "content": [{"type": "#text", "runs": [{"marks": {}, "text": code}]}]
            }));
            index = close + 1;
        } else if line.starts_with('#') && line.trim_start_matches('#').starts_with(' ') {
            let level = line.len() - line.trim_start_matches('#').len();
            // The heading's text as `restated` keys its id by: its Markdown, marks and all.
            let text = line[level..].trim().trim_end_matches('#').trim();
            found.push(json!({
                "type": "heading", "attrs": {"id": fresh_id(&seed), "level": level as f64},
                "content": [{"type": "#text", "runs": [{"marks": {}, "text": text}]}]
            }));
            index += 1;
        } else if line.starts_with('|') {
            let mut rows = Vec::new();
            let mut first = true;
            while index < lines.len() && lines[index].starts_with('|') {
                let cells: Vec<&str> = lines[index]
                    .trim()
                    .trim_matches('|')
                    .split(" | ")
                    .map(str::trim)
                    .collect();
                if cells
                    .iter()
                    .all(|cell| !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':'))
                {
                    index += 1;
                    continue;
                }
                let cell = if first { "table_header" } else { "table_cell" };
                first = false;
                rows.push(json!({
                    "type": "table_row", "attrs": {},
                    "content": cells.iter().map(|text| json!({
                        "type": cell, "attrs": {"colspan": 1.0, "rowspan": 1.0},
                        "content": [paragraph(text)]
                    })).collect::<Vec<_>>()
                }));
                index += 1;
            }
            found.push(json!({"type": "table", "attrs": {"sizing": "fit"}, "content": rows}));
        } else if list_item(line) {
            let ordered = !(line.starts_with("- ") || line.starts_with("* "));
            let mut items = Vec::new();
            while index < lines.len() && list_item(lines[index]) {
                let text = lines[index].split_once(' ').map_or("", |(_, text)| text);
                items.push(json!({"type": "list_item", "attrs": {}, "content": [paragraph(text)]}));
                index += 1;
            }
            found.push(if ordered {
                json!({"type": "ordered_list", "attrs": {"order": 1.0}, "content": items})
            } else {
                json!({"type": "bullet_list", "attrs": {}, "content": items})
            });
        } else {
            // A paragraph runs to a blank line or a section's opening or closing line, and always
            // takes its own first line — a `+++` no section opened among them — so the walk moves on.
            let start = index;
            index += 1;
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && !lines[index].starts_with("+++ ")
                && lines[index] != "+++"
            {
                index += 1;
            }
            found.push(paragraph(&lines[start..index].join("\n")));
        }
    }
    found
}

/// A base64 Yjs update holding `tree` in a `prosemirror` fragment, as Linear's editor keeps it.
fn encode_tree(tree: &Value) -> String {
    let doc = Doc::new();
    let fragment = doc.get_or_insert_xml_fragment("prosemirror");
    {
        let mut txn = doc.transact_mut();
        for (index, node) in tree["content"].as_array().into_iter().flatten().enumerate() {
            encode_node(&mut txn, Parent::Fragment(&fragment), index, node);
        }
    }
    let update = doc
        .transact()
        .encode_state_as_update_v1(&yrs::StateVector::default());
    base64::engine::general_purpose::STANDARD.encode(update)
}

/// Where an encoded node is inserted: the document's fragment, or an element of it.
#[derive(Clone, Copy)]
enum Parent<'a> {
    Fragment(&'a yrs::XmlFragmentRef),
    Element(&'a yrs::XmlElementRef),
}

fn encode_node(txn: &mut yrs::TransactionMut, parent: Parent<'_>, index: usize, node: &Value) {
    let index = u32::try_from(index).unwrap_or(u32::MAX);
    if node["type"] == "#text" {
        let prelim = XmlTextPrelim::new("");
        let held = match parent {
            Parent::Fragment(fragment) => fragment.insert(txn, index, prelim),
            Parent::Element(element) => element.insert(txn, index, prelim),
        };
        for run in node["runs"].as_array().into_iter().flatten() {
            let at = held.len(txn);
            let marks: yrs::types::Attrs = run["marks"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(key, value)| (Arc::from(key.as_str()), json_any(value)))
                .collect();
            held.insert_with_attributes(txn, at, run["text"].as_str().unwrap_or_default(), marks);
        }
        return;
    }
    let prelim = XmlElementPrelim::empty(node["type"].as_str().unwrap_or("paragraph"));
    let held = match parent {
        Parent::Fragment(fragment) => fragment.insert(txn, index, prelim),
        Parent::Element(element) => element.insert(txn, index, prelim),
    };
    for (key, value) in node["attrs"].as_object().into_iter().flatten() {
        held.insert_attribute(txn, key.as_str(), json_any(value));
    }
    for (at, child) in node["content"].as_array().into_iter().flatten().enumerate() {
        encode_node(txn, Parent::Element(&held), at, child);
    }
}

fn decoded_node<T: ReadTxn>(txn: &T, node: XmlOut) -> Value {
    match node {
        XmlOut::Element(element) => {
            let attrs: Map<String, Value> = element
                .attributes(txn)
                .map(|(key, value)| (key.to_owned(), out_json(&value)))
                .collect();
            let content: Vec<Value> = (0..element.len(txn))
                .filter_map(|index| element.get(txn, index))
                .map(|child| decoded_node(txn, child))
                .collect();
            json!({"type": element.tag().as_ref(), "attrs": sorted(attrs), "content": content})
        }
        XmlOut::Text(text) => {
            let mut runs: Vec<Value> = Vec::new();
            for chunk in text.diff(txn, |_| ()) {
                let marks: Map<String, Value> = chunk
                    .attributes
                    .map(|attrs| {
                        attrs
                            .iter()
                            .filter(|(key, value)| {
                                key.as_ref() != "attribution" && !matches!(value, Any::Null)
                            })
                            .map(|(key, value)| (key.to_string(), any_json(value)))
                            .collect()
                    })
                    .unwrap_or_default();
                let piece = match chunk.insert {
                    Out::Any(Any::String(piece)) => piece.to_string(),
                    other => format!("{other:?}"),
                };
                let marks = sorted(marks);
                match runs.last_mut() {
                    Some(last) if last["marks"] == marks => {
                        let joined =
                            format!("{}{piece}", last["text"].as_str().unwrap_or_default());
                        last["text"] = json!(joined);
                    }
                    _ => runs.push(json!({"marks": marks, "text": piece})),
                }
            }
            json!({"type": "#text", "runs": runs})
        }
        XmlOut::Fragment(_) => json!({"type": "#fragment"}),
    }
}

fn any_json(any: &Any) -> Value {
    match any {
        Any::Null | Any::Undefined => Value::Null,
        Any::Bool(held) => json!(held),
        Any::Number(number) => json!(number.as_f64()),
        Any::String(text) => json!(text.as_ref()),
        Any::Buffer(bytes) => json!(base64::engine::general_purpose::STANDARD.encode(bytes)),
        Any::Array(items) => Value::Array(items.iter().map(any_json).collect()),
        Any::Map(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), any_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
    }
}

fn json_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(held) => Any::Bool(*held),
        Value::Number(number) => {
            Any::Number(yrs::any::Number::Float(number.as_f64().unwrap_or(0.0)))
        }
        Value::String(text) => Any::String(Arc::from(text.as_str())),
        Value::Array(items) => Any::Array(items.iter().map(json_any).collect()),
        Value::Object(map) => Any::Map(Arc::new(
            map.iter()
                .map(|(key, value)| (key.clone(), json_any(value)))
                .collect::<HashMap<_, _>>(),
        )),
    }
}

fn out_json(out: &Out) -> Value {
    match out {
        Out::Any(any) => any_json(any),
        other => json!(format!("{other:?}")),
    }
}

fn sorted(map: Map<String, Value>) -> Value {
    Value::Object(
        map.into_iter()
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{closed_text, decoded_tree, node_tree, same_tree};

    /// The six editor states spike `spike-linear-collapsed-slot` captured from `TES2`: each
    /// kind's long form ending in the legacy comment slot, and in the `Fields` section followed by
    /// the block opening with its sentinel.
    const CAPTURED: [(&str, &str); 6] = [
        (
            "issue-legacy",
            include_str!("../fixtures/collapsed_slot/collapsed-slot-issue-legacy.json"),
        ),
        (
            "project-legacy",
            include_str!("../fixtures/collapsed_slot/collapsed-slot-project-legacy.json"),
        ),
        (
            "document-legacy",
            include_str!("../fixtures/collapsed_slot/collapsed-slot-document-legacy.json"),
        ),
        (
            "issue-fields-sentinel",
            include_str!("../fixtures/collapsed_slot/collapsed-slot-issue-fields-sentinel.json"),
        ),
        (
            "project-fields-sentinel",
            include_str!("../fixtures/collapsed_slot/collapsed-slot-project-fields-sentinel.json"),
        ),
        (
            "document-fields-sentinel",
            include_str!("../fixtures/collapsed_slot/collapsed-slot-document-fields-sentinel.json"),
        ),
    ];

    fn captured() -> impl Iterator<Item = (&'static str, Value)> {
        CAPTURED
            .into_iter()
            .map(|(name, raw)| (name, serde_json::from_str(raw).expect("a captured fixture")))
    }

    #[test]
    fn each_captured_markdown_maps_to_its_recorded_node_tree() {
        for (name, fixture) in captured() {
            let markdown = fixture["markdown"].as_str().expect("markdown");
            let derived = node_tree(markdown, name);
            assert!(
                same_tree(&derived, &fixture["node_tree"]),
                "{name}: derived {derived:#}\n\nrecorded {:#}",
                fixture["node_tree"]
            );
            // And Linear's own state decodes to the tree the fixture recorded of it.
            let decoded = decoded_tree(fixture["state"].as_str().expect("state")).expect("decodes");
            assert!(
                same_tree(&decoded, &fixture["node_tree"]),
                "{name}: decoded"
            );
        }
    }

    #[test]
    fn a_stray_section_line_is_a_paragraph_and_the_walk_ends() {
        for stray in ["+++", "+++x", "Text\n+++\nMore", "+++ unclosed\n\nbody"] {
            let tree = node_tree(stray, "stray");
            assert!(
                !tree["content"].as_array().expect("content").is_empty(),
                "{stray:?}"
            );
        }
        assert_eq!(closed_text(&node_tree("+++", "stray")), "+++");
    }

    #[test]
    fn a_collapsible_section_is_one_node_with_or_without_was_copied_while_open() {
        let mut sections = 0;
        for (name, fixture) in captured().filter(|(name, _)| name.ends_with("fields-sentinel")) {
            for node in fixture["node_tree"]["content"].as_array().expect("content") {
                if node["type"] == "collapsible_section" {
                    sections += 1;
                    let attrs = node["attrs"].as_object().expect("attrs");
                    assert!(attrs.contains_key("id"), "{name}");
                    assert!(
                        attrs
                            .keys()
                            .all(|key| key == "id" || key == "wasCopiedWhileOpen"),
                        "{name}: {attrs:?}"
                    );
                }
            }
        }
        // Two per kind — the `Fields` section and the metadata block — and Linear left
        // `wasCopiedWhileOpen` off the document's, which compares as the same tree.
        assert_eq!(sections, 6);
    }

    #[test]
    fn the_closed_view_shows_a_legacy_slot_and_hides_the_collapsed_block() {
        for (name, fixture) in captured() {
            let markdown = fixture["markdown"].as_str().expect("markdown");
            let seen = closed_text(&node_tree(markdown, name));
            assert!(
                seen.contains("Run onetaskgraph task show"),
                "{name}: {seen}"
            );
            if name.ends_with("legacy") {
                // The comment Linear renders as text, its JSON with it.
                assert!(
                    seen.contains("<!-- onetaskgraph.metadata {\"onepipeline"),
                    "{name}"
                );
                assert!(seen.contains("\"onetaskgraph.origin\""), "{name}");
            } else {
                assert!(!seen.contains("<!--"), "{name}: {seen}");
                assert!(!seen.contains("onetaskgraph.origin"), "{name}: {seen}");
                assert!(
                    !seen.contains("Owner"),
                    "{name}: the Fields section is collapsed"
                );
                assert!(
                    seen.ends_with("Fields\nonetaskgraph metadata"),
                    "{name}: {seen}"
                );
            }
        }
    }
}
