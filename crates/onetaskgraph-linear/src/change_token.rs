//! A Linear item's change token, `lin1`: what [`Task::version`](onetaskgraph_plugin_api::Task::version),
//! [`Project::version`](onetaskgraph_plugin_api::Project::version) and
//! [`Document::version`](onetaskgraph_plugin_api::Document::version) hold here, and the one
//! ordering of two of them.
//!
//! `docs/plugin-protocol.md` §4.13d is the specification. Every part is spelled from the response
//! the source already receives: the item's `updatedAt`, and the digest of its *facets* — what a
//! person can change about it without moving that time, which `spike-linear-coverage` measured
//! `updatedAt` alone missing: a label renamed or recoloured, a workflow state renamed, a
//! document's content and slot edited. With the store's cache attached a task's token carries a
//! third part, the digest of its first page of relations, because a relation added or removed
//! moved no time the spike could see.
//!
//! An item whose labels came back as 50 nodes or more — Linear's default page, which the
//! documents here select without a `first` or a `pageInfo` — gives no token, and so does a task
//! whose relations page reports another page or holds more than [`RELATIONS_PAGE`] rows: a part
//! read from part of a connection vouches for nothing.

use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::asset_sha256;
use serde::Serialize;
use serde_json::Value;

/// What every token this plugin gives starts with.
pub const PREFIX: &str = "lin1|";

/// What the relations part of a task's token starts with.
const RELATIONS: &str = "r:";

/// The rows the relations page a task's token is spelled from holds at most: the `first` of the
/// page the cache's documents select.
pub const RELATIONS_PAGE: usize = 10;

/// The labels Linear answers a `labels` connection with when it names no `first`: a response
/// holding this many may be a first page of more, so it gives no token.
pub const DEFAULT_LABEL_PAGE: usize = 50;

/// One label as a facet spells it: its id, its name and its colour, which Linear may leave null.
type LabelFacet<'a> = (&'a str, &'a str, Option<&'a str>);

/// A workflow state's or a project status's name and type.
type NamedType<'a> = (&'a str, &'a str);

/// A task's facets, as its token's digest spells them.
#[derive(Serialize)]
struct TaskFacets<'a> {
    key: &'a str,
    labels: Vec<LabelFacet<'a>>,
    project: Option<&'a str>,
    state: NamedType<'a>,
    url: Option<&'a str>,
}

/// A project's facets.
#[derive(Serialize)]
struct ProjectFacets<'a> {
    labels: Vec<LabelFacet<'a>>,
    status: NamedType<'a>,
}

/// A lowercase hex SHA-256 — of a document's content, of a facets part's canonical JSON, of a
/// relations page — made only by hashing, or read back out of a token and refused when it is not
/// one.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Sha256Hex(String);

impl Sha256Hex {
    fn of(bytes: &[u8]) -> Self {
        Self(asset_sha256(bytes))
    }

    fn parse(text: &str) -> Option<Self> {
        (text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| Self(text.to_owned()))
    }
}

impl std::fmt::Display for Sha256Hex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A document's facets.
#[derive(Serialize)]
struct DocumentFacets<'a> {
    content: Sha256Hex,
    project: Option<&'a str>,
    title: &'a str,
    url: &'a str,
}

/// The lowercase hex SHA-256 of `facets`' canonical JSON.
fn digest(facets: &impl Serialize) -> Option<Sha256Hex> {
    Some(Sha256Hex::of(
        serde_json::to_string(facets).ok()?.as_bytes(),
    ))
}

/// `key` of `item` when it is a string — and `None`, so no token, when it is missing or anything
/// else: a part read as something Linear's schema does not give vouches for nothing.
fn text<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key)?.as_str()
}

/// `key` of `item` when it is a string or null, `None` within when null — and `None` when it is
/// missing or anything else.
fn nullable_text<'a>(item: &'a Value, key: &str) -> Option<Option<&'a str>> {
    match item.get(key)? {
        Value::Null => Some(None),
        Value::String(text) => Some(Some(text)),
        _ => None,
    }
}

/// Every label of `item`, as `[id, name, color]` sorted by id — or `None` when the connection is
/// missing, may be only a first page, or holds a label without a string id and name and a
/// string or null colour.
fn labels(item: &Value) -> Option<Vec<LabelFacet<'_>>> {
    let nodes = item.pointer("/labels/nodes")?.as_array()?;
    if nodes.len() >= DEFAULT_LABEL_PAGE {
        return None;
    }
    let mut labels = nodes
        .iter()
        .map(|label| {
            Some((
                text(label, "id")?,
                text(label, "name")?,
                nullable_text(label, "color")?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    labels.sort_unstable();
    Some(labels)
}

/// The project `item` is filed under, as its facets name it: its id, or `Some(None)` when it is
/// in none — and `None` when the member is missing or names a project without a string id.
fn project(item: &Value) -> Option<Option<&str>> {
    match item.get("project")? {
        Value::Null => Some(None),
        project => Some(Some(text(project, "id")?)),
    }
}

/// A `{name, type}` status or workflow state, as a facet names it, when both are strings.
fn named_type(status: &Value) -> Option<NamedType<'_>> {
    Some((text(status, "name")?, text(status, "type")?))
}

/// `updatedAt`, read as an instant and spelled back the one way this module spells it.
fn updated(item: &Value) -> Option<String> {
    let at: DateTime<Utc> = item.get("updatedAt")?.as_str()?.parse().ok()?;
    Some(at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
}

/// The digest of a task's first page of relations, or `None` when the page reports another or
/// holds more than [`RELATIONS_PAGE`] rows, or is missing.
fn relations(issue: &Value) -> Option<Sha256Hex> {
    let page = issue.get("relations")?;
    if page.pointer("/pageInfo/hasNextPage") != Some(&Value::Bool(false)) {
        return None;
    }
    let nodes = page.get("nodes")?.as_array()?;
    if nodes.len() > RELATIONS_PAGE {
        return None;
    }
    let mut rows = nodes
        .iter()
        .map(|node| {
            Some((
                node.get("id")?.as_str()?,
                node.get("type")?.as_str()?,
                node.pointer("/relatedIssue/id")?.as_str()?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    rows.sort_unstable();
    Some(Sha256Hex::of(serde_json::to_string(&rows).ok()?.as_bytes()))
}

/// The token of the issue `issue`, as one response answered it — with the digest of its first
/// page of relations when `attached`, and none at all when that page is not whole — or `None`
/// when the response does not carry every part whole.
#[must_use]
pub fn task(issue: &Value, attached: bool) -> Option<String> {
    let facets = TaskFacets {
        key: text(issue, "identifier")?,
        labels: labels(issue)?,
        project: project(issue)?,
        state: named_type(issue.get("state")?)?,
        url: nullable_text(issue, "url")?,
    };
    let base = format!("{PREFIX}{}|{}", updated(issue)?, digest(&facets)?);
    if attached {
        Some(format!("{base}|{RELATIONS}{}", relations(issue)?))
    } else {
        Some(base)
    }
}

/// The token of the project `project`, as one response answered it.
#[must_use]
pub fn project_token(project: &Value) -> Option<String> {
    let facets = ProjectFacets {
        labels: labels(project)?,
        status: named_type(project.get("status")?)?,
    };
    Some(format!(
        "{PREFIX}{}|{}",
        updated(project)?,
        digest(&facets)?
    ))
}

/// The token of the document `document`, as one response answered it: its content's digest is
/// of the content as Linear holds it, metadata slot and all.
#[must_use]
pub fn document_token(document: &Value) -> Option<String> {
    let facets = DocumentFacets {
        content: Sha256Hex::of(
            nullable_text(document, "content")?
                .unwrap_or_default()
                .as_bytes(),
        ),
        project: project(document)?,
        title: text(document, "title")?,
        url: text(document, "url")?,
    };
    Some(format!(
        "{PREFIX}{}|{}",
        updated(document)?,
        digest(&facets)?
    ))
}

/// The token `token` with its relations part taken off: what a later page of one task's walk,
/// read with no relations page of its own, is held to.
#[must_use]
pub fn without_relations(token: &str) -> &str {
    token
        .split_once(&format!("|{RELATIONS}"))
        .map_or(token, |(base, _)| base)
}

/// What one token of this plugin is made of.
struct Parts {
    updated: DateTime<Utc>,
    facets: Sha256Hex,
    relations: Option<Sha256Hex>,
}

fn parts(token: &str) -> Option<Parts> {
    let mut parts = token.strip_prefix(PREFIX)?.split('|');
    let updated = DateTime::parse_from_rfc3339(parts.next()?)
        .ok()?
        .with_timezone(&Utc);
    let facets = Sha256Hex::parse(parts.next()?)?;
    let relations = match parts.next() {
        None => None,
        Some(part) => Some(Sha256Hex::parse(part.strip_prefix(RELATIONS)?)?),
    };
    parts.next().is_none().then_some(Parts {
        updated,
        facets,
        relations,
    })
}

/// How the version `a` of one item stands to its version `b`: by `updatedAt` as an instant when
/// their facets and their relations parts — present or absent, and their value — are equal,
/// `Equal` only when every part is, and `None` when either is not a token of this plugin or any
/// of those differ, which says nothing about which reading came first.
#[must_use]
pub fn order(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parts(a)?, parts(b)?);
    (a.facets == b.facets && a.relations == b.relations).then(|| a.updated.cmp(&b.updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn issue(updated: &str, label: &str, relations: Value) -> Value {
        json!({
            "id": "i1", "identifier": "ENG-1", "url": "https://linear.app/x/issue/ENG-1",
            "updatedAt": updated, "state": {"name": "Todo", "type": "unstarted"},
            "project": {"id": "p1"},
            "labels": {"nodes": [{"id": "l1", "name": label, "color": "#ff0000"}]},
            "relations": relations,
        })
    }

    fn page(rows: &[(&str, &str)], more: bool) -> Value {
        json!({
            "nodes": rows.iter().map(|(id, far)| json!({"id": id, "type": "blocks",
                                                        "relatedIssue": {"id": far}}))
                .collect::<Vec<_>>(),
            "pageInfo": {"hasNextPage": more},
        })
    }

    #[test]
    fn a_later_time_with_the_same_facets_and_relations_orders_after() {
        let none = page(&[], false);
        let before = task(&issue("2026-10-01T00:00:00Z", "bug", none.clone()), true).unwrap();
        let after = task(&issue("2026-10-01T00:00:01Z", "bug", none), true).unwrap();
        assert_eq!(order(&after, &before), Some(Ordering::Greater));
        assert_eq!(order(&before, &after), Some(Ordering::Less));
        assert_eq!(order(&before, &before), Some(Ordering::Equal));
    }

    #[test]
    fn another_facet_or_relations_part_is_never_ordered() {
        let none = page(&[], false);
        let before = task(&issue("2026-10-01T00:00:00Z", "bug", none.clone()), true).unwrap();
        let renamed = task(&issue("2026-10-01T00:00:05Z", "defect", none.clone()), true).unwrap();
        assert_eq!(order(&renamed, &before), None, "a label renamed");
        let related = task(
            &issue("2026-10-01T00:00:05Z", "bug", page(&[("r1", "i2")], false)),
            true,
        )
        .unwrap();
        assert_eq!(order(&related, &before), None, "a relation added");
        let unattached = task(&issue("2026-10-01T00:00:00Z", "bug", none), false).unwrap();
        assert!(!unattached.contains("|r:"));
        assert_eq!(
            order(&unattached, &before),
            None,
            "a token with relations and one without"
        );
        assert_eq!(order("gh3|x", &before), None);
        assert_eq!(order(&format!("{before}|extra"), &before), None);
    }

    #[test]
    fn a_part_read_from_part_of_a_connection_gives_no_token() {
        let more = page(&[("r1", "i2")], true);
        assert_eq!(
            task(&issue("2026-10-01T00:00:00Z", "bug", more), true),
            None
        );
        let rows: Vec<(String, String)> = (0..=RELATIONS_PAGE)
            .map(|n| (format!("r{n}"), format!("i{n}")))
            .collect();
        let eleven: Vec<(&str, &str)> = rows
            .iter()
            .map(|(id, far)| (id.as_str(), far.as_str()))
            .collect();
        assert_eq!(
            task(
                &issue("2026-10-01T00:00:00Z", "bug", page(&eleven, false)),
                true
            ),
            None
        );
        let mut crowded = issue("2026-10-01T00:00:00Z", "bug", page(&[], false));
        crowded["labels"]["nodes"] = Value::Array(
            (0..DEFAULT_LABEL_PAGE)
                .map(|n| json!({"id": format!("l{n}"), "name": "x", "color": null}))
                .collect(),
        );
        assert_eq!(task(&crowded, true), None);
        assert_eq!(task(&crowded, false), None);
    }

    #[test]
    fn a_part_of_another_json_type_than_linears_schema_gives_no_token() {
        let whole = issue("2026-10-01T00:00:00Z", "bug", page(&[], false));
        assert!(task(&whole, true).is_some());
        for (part, value) in [
            ("/identifier", json!(7)),
            ("/url", json!(["a"])),
            ("/project", json!({"id": 7})),
            ("/project", json!("p1")),
            ("/state/type", json!(null)),
            ("/state/name", json!(1)),
            ("/labels/nodes/0/name", json!(null)),
            ("/labels/nodes/0/id", json!(1)),
            ("/labels/nodes/0/color", json!(0)),
        ] {
            let mut broken = whole.clone();
            *broken.pointer_mut(part).expect("the part") = value.clone();
            assert_eq!(task(&broken, true), None, "{part} as {value}");
        }
        let document = json!({"title": "Design", "url": "https://linear.app/x/document/design",
                              "project": null, "updatedAt": "2026-10-01T00:00:00Z",
                              "content": null});
        assert!(
            document_token(&document).is_some(),
            "Linear's content may be null"
        );
        for (part, value) in [
            ("/title", json!(null)),
            ("/url", json!(null)),
            ("/content", json!(1)),
        ] {
            let mut broken = document.clone();
            *broken.pointer_mut(part).expect("the part") = value.clone();
            assert_eq!(document_token(&broken), None, "{part} as {value}");
        }
    }

    #[test]
    fn the_relations_page_a_token_is_read_from_is_the_one_the_cache_selects() {
        assert!(
            crate::graphql::RELATIONS_PAGE.contains(&format!("relations(first:{RELATIONS_PAGE})")),
            "{} selects a page of {RELATIONS_PAGE}",
            crate::graphql::RELATIONS_PAGE
        );
    }

    #[test]
    fn a_documents_token_moves_with_its_content_and_the_relations_part_comes_off() {
        let document = |content: &str| {
            json!({"title": "Design", "url": "https://linear.app/x/document/design", "project": null,
                   "updatedAt": "2026-10-01T00:00:00Z", "content": content})
        };
        let one = document_token(&document("a")).unwrap();
        let other = document_token(&document("a, edited")).unwrap();
        assert_eq!(order(&one, &other), None);
        let token = task(
            &issue("2026-10-01T00:00:00Z", "bug", page(&[], false)),
            true,
        )
        .unwrap();
        assert_eq!(
            without_relations(&token),
            task(
                &issue("2026-10-01T00:00:00Z", "bug", page(&[], false)),
                false
            )
            .unwrap()
        );
    }
}
