//! The ids Linear's editor gave a record's headings, read out of the editor state an answer
//! carries.
//!
//! Linear stores each issue's description and each project's and document's content as a
//! Yjs document as well as Markdown: `descriptionState` on an issue, `contentState` on a
//! project or a document, base64 of a Yjs update. Every heading is a `heading` node of its
//! `prosemirror` fragment carrying an `id` attribute, a UUID Linear assigns when the record is
//! written and keeps across an update that leaves the heading's text alone — which is what
//! Linear's own heading link, `#heading-<slug>-<first 8 hex of the id>`, names. Nothing here
//! decides that link: the engine's `section_links` does, from what [`decoded`] reports.

use base64::Engine as _;
use onetaskgraph_plugin_api::{MetadataRecord, NativeId, StoredHeading};
use serde_json::Value;
use yrs::types::xml::XmlOut;
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, Out, Text, Transact, Update, Xml, XmlFragment};

/// The Yjs fragment Linear's editor keeps a record's content in.
const FRAGMENT: &str = "prosemirror";

/// Every heading of one editor state, in order, with the id Linear stored it under — `None`
/// for a state that does not decode, which says nothing about the record's headings.
#[must_use]
pub fn decoded(state: &str) -> Option<Vec<StoredHeading>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(state)
        .ok()?;
    let update = Update::decode_v1(&bytes).ok()?;
    let doc = Doc::new();
    let fragment = doc.get_or_insert_xml_fragment(FRAGMENT);
    doc.transact_mut().apply_update(update).ok()?;
    let txn = doc.transact();
    let mut headings = Vec::new();
    for node in fragment.successors(&txn) {
        let XmlOut::Element(element) = node else {
            continue;
        };
        if element.tag().as_ref() != "heading" {
            continue;
        }
        let Some(Out::Any(Any::String(id))) = element.get_attribute(&txn, "id") else {
            continue;
        };
        let mut text = String::new();
        for inner in element.successors(&txn) {
            if let XmlOut::Text(run) = inner {
                for chunk in run.diff(&txn, |_| ()) {
                    if let Out::Any(Any::String(piece)) = chunk.insert {
                        text.push_str(&piece);
                    }
                }
            }
        }
        headings.push(StoredHeading {
            text: text.trim().to_owned(),
            id: id.to_string(),
        });
    }
    Some(headings)
}

/// Every record an answer carries an editor state for, with its headings: an issue by its
/// `descriptionState`, a project — the node naming a `name` — and a document — a `title` —
/// by their `contentState`, wherever in the answer each sits.
pub(crate) fn carried(data: &Value) -> Vec<((MetadataRecord, NativeId), Vec<StoredHeading>)> {
    let mut found = Vec::new();
    walk(data, &mut found);
    found
}

fn walk(value: &Value, found: &mut Vec<((MetadataRecord, NativeId), Vec<StoredHeading>)>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(id)) = object.get("id") {
                let held = match (object.get("descriptionState"), object.get("contentState")) {
                    (Some(Value::String(state)), _) => Some((MetadataRecord::Task, state)),
                    (_, Some(Value::String(state))) if object.contains_key("name") => {
                        Some((MetadataRecord::Project, state))
                    }
                    (_, Some(Value::String(state))) => Some((MetadataRecord::Document, state)),
                    _ => None,
                };
                if let Some((record, state)) = held
                    && let Some(headings) = decoded(state)
                {
                    found.push(((record, NativeId(id.clone())), headings));
                }
            }
            for inner in object.values() {
                walk(inner, found);
            }
        }
        Value::Array(items) => {
            for inner in items {
                walk(inner, found);
            }
        }
        _ => {}
    }
}
