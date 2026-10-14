//! A Linear item's change token, `lin1`: what [`Task::version`](onetaskgraph_plugin_api::Task::version),
//! [`Project::version`](onetaskgraph_plugin_api::Project::version) and
//! [`Document::version`](onetaskgraph_plugin_api::Document::version) hold here, and the one
//! ordering of two of them.
//!
//! `docs/plugin-protocol.md` §4.13d is the specification. Every part is spelled from the response
//! the source already receives: the item's `updatedAt`, and the digest of its *facets* — what a
//! person can change about it without moving that time, which `spike-linear-coverage` measured
//! `updatedAt` alone missing: a label renamed or recoloured, a workflow state renamed, a
//! document's content and slot edited. With the store's cache attached a task's token carries
//! three parts more — the digests of its first page of comments, of its first page of relations
//! and of its first page of inverse relations — and a project's one, the digest of its first page
//! of relations, because a comment, a relation and an inverse relation added or removed moved no
//! time the spike could see on the item that holds the token.
//!
//! An item whose labels came back as 50 nodes or more — Linear's default page, which the
//! documents here select without a `first` or a `pageInfo` — gives no token, and so does an item
//! whose comments, relations or inverse relations page reports another page or holds more than
//! [`PAGE`] rows: a part read from part of a connection vouches for nothing.

use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::asset_sha256;
use serde::Serialize;
use serde_json::Value;

/// What every token this plugin gives starts with.
pub const PREFIX: &str = "lin1|";

/// The rows a comments, relations or inverse relations page a token is spelled from holds at
/// most: the `first` of the pages the cache's documents select.
pub const PAGE: usize = 10;

/// The labels Linear answers a `labels` connection with when it names no `first`: a response
/// holding this many may be a first page of more, so it gives no token.
pub const DEFAULT_LABEL_PAGE: usize = 50;

/// One of the parts the cache's widened reads add to a token, each the digest of one first page.
///
/// Spelled in this order, which is the only order a token holds them in, so the part a later
/// page of one walk is held to — everything before [`Part::Relations`] — is a prefix of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// A task's comments, as `[id, updatedAt]` rows.
    Comments,
    /// A task's or a project's relations, as `[id, type, far end]` rows.
    Relations,
    /// A task's inverse relations, as `[id, type, near end]` rows.
    Inverse,
}

impl Part {
    const ALL: [Self; 3] = [Self::Comments, Self::Relations, Self::Inverse];

    fn tag(self) -> &'static str {
        match self {
            Self::Comments => "c:",
            Self::Relations => "r:",
            Self::Inverse => "i:",
        }
    }
}

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
/// first page — made only by hashing, or read back out of a token and refused when it is not
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

/// An instant, read and spelled back the one way this module spells it.
fn instant(value: &Value) -> Option<String> {
    let at: DateTime<Utc> = value.as_str()?.parse().ok()?;
    Some(at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
}

/// `updatedAt`, read as an instant and spelled back the one way this module spells it.
fn updated(item: &Value) -> Option<String> {
    instant(item.get("updatedAt")?)
}

/// The nodes of the connection `key` of `item` when they are the whole of it — a page reporting
/// no further page either way it walks, of at most [`PAGE`] rows — or `None`.
///
/// A page walked forwards says so with `hasNextPage`, one walked backwards with
/// `hasPreviousPage`; the caller of a page that resumed from a cursor knows it is not the whole
/// and asks for none.
fn whole_page<'a>(item: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    let page = item.get(key)?;
    let info = page.get("pageInfo")?;
    let more = info
        .get("hasNextPage")
        .or_else(|| info.get("hasPreviousPage"))?;
    if more != &Value::Bool(false) {
        return None;
    }
    let nodes = page.get("nodes")?.as_array()?;
    (nodes.len() <= PAGE).then_some(nodes)
}

/// The digest of `rows`, sorted.
fn rows_digest<T: Ord + Serialize>(mut rows: Vec<T>) -> Option<Sha256Hex> {
    rows.sort_unstable();
    Some(Sha256Hex::of(serde_json::to_string(&rows).ok()?.as_bytes()))
}

/// The digest of the relations page `key` of `item`, each row `[id, type, <far>.id]`.
fn relation_digest(item: &Value, key: &str, far: &str) -> Option<Sha256Hex> {
    rows_digest(
        whole_page(item, key)?
            .iter()
            .map(|node| {
                Some((
                    node.get("id")?.as_str()?,
                    node.get("type")?.as_str()?,
                    node.get(far)?.get("id")?.as_str()?,
                ))
            })
            .collect::<Option<Vec<_>>>()?,
    )
}

/// The digest of an issue's comments page, each row `[id, updatedAt]`.
fn comment_digest(issue: &Value) -> Option<Sha256Hex> {
    rows_digest(
        whole_page(issue, "comments")?
            .iter()
            .map(|node| Some((node.get("id")?.as_str()?, instant(node.get("updatedAt")?)?)))
            .collect::<Option<Vec<_>>>()?,
    )
}

/// The digest one part of a task's token is spelled from, read from `issue`.
fn task_part(issue: &Value, part: Part) -> Option<Sha256Hex> {
    match part {
        Part::Comments => comment_digest(issue),
        Part::Relations => relation_digest(issue, "relations", "relatedIssue"),
        Part::Inverse => relation_digest(issue, "inverseRelations", "issue"),
    }
}

/// `lin1|<updatedAt>|<facets>` of the issue `issue`.
fn task_base(issue: &Value) -> Option<String> {
    let facets = TaskFacets {
        key: text(issue, "identifier")?,
        labels: labels(issue)?,
        project: project(issue)?,
        state: named_type(issue.get("state")?)?,
        url: nullable_text(issue, "url")?,
    };
    Some(format!("{PREFIX}{}|{}", updated(issue)?, digest(&facets)?))
}

/// `base` with each of `parts` of `item` appended, or `None` when any is not whole.
fn with_parts(
    base: String,
    parts: &[Part],
    read: impl Fn(Part) -> Option<Sha256Hex>,
) -> Option<String> {
    let mut token = base;
    for part in parts {
        token.push('|');
        token.push_str(part.tag());
        token.push_str(&read(*part)?.0);
    }
    Some(token)
}

/// The token of the issue `issue`, as one response answered it — with the digests of its first
/// page of comments, relations and inverse relations when `attached`, and none at all when any of
/// those pages is not whole — or `None` when the response does not carry every part whole.
#[must_use]
pub fn task(issue: &Value, attached: bool) -> Option<String> {
    let base = task_base(issue)?;
    if attached {
        with_parts(base, &Part::ALL, |part| task_part(issue, part))
    } else {
        Some(base)
    }
}

/// The part of the attached token of the issue `issue` that a page of a forward-edge walk past
/// its first is held to: the token up to its relations part, which a page answering only part of
/// the relations cannot spell.
#[must_use]
pub fn task_before_relations(issue: &Value) -> Option<String> {
    with_parts(task_base(issue)?, &[Part::Comments], |part| {
        task_part(issue, part)
    })
}

/// The token of the project `project`, as one response answered it — with the digest of its
/// first page of relations when `attached`, and none at all when that page is not whole.
#[must_use]
pub fn project_token(project: &Value, attached: bool) -> Option<String> {
    let base = project_before_relations(project)?;
    if attached {
        with_parts(base, &[Part::Relations], |_| {
            relation_digest(project, "relations", "relatedProject")
        })
    } else {
        Some(base)
    }
}

/// The part of the attached token of the project `project` that a page of a forward-edge walk
/// past its first is held to: the token without its relations part.
#[must_use]
pub fn project_before_relations(project: &Value) -> Option<String> {
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

/// The token `token` with its relations part and every part after it taken off: what a later
/// page of one walk, read with no whole relations page of its own, is held to.
#[must_use]
pub fn without_relations(token: &str) -> &str {
    token
        .split_once(&format!("|{}", Part::Relations.tag()))
        .map_or(token, |(base, _)| base)
}

/// What one token of this plugin is made of.
struct Parts {
    updated: DateTime<Utc>,
    facets: Sha256Hex,
    /// Each part beside those two, in the one order a token spells them.
    extra: Vec<(Part, Sha256Hex)>,
}

fn parts(token: &str) -> Option<Parts> {
    let mut parts = token.strip_prefix(PREFIX)?.split('|');
    let updated = DateTime::parse_from_rfc3339(parts.next()?)
        .ok()?
        .with_timezone(&Utc);
    let facets = Sha256Hex::parse(parts.next()?)?;
    let mut extra: Vec<(Part, Sha256Hex)> = Vec::new();
    for text in parts {
        let part = Part::ALL
            .into_iter()
            .find(|part| text.starts_with(part.tag()))?;
        // In the one order, each at most once.
        if extra.last().is_some_and(|(last, _)| {
            Part::ALL.iter().position(|known| known == last)
                >= Part::ALL.iter().position(|known| *known == part)
        }) {
            return None;
        }
        extra.push((part, Sha256Hex::parse(&text[part.tag().len()..])?));
    }
    Some(Parts {
        updated,
        facets,
        extra,
    })
}

/// How the version `a` of one item stands to its version `b`: by `updatedAt` as an instant when
/// their facets and every other part — which parts are present, and each one's value — are
/// equal, `Equal` only when every part is, and `None` when either is not a token of this plugin
/// or any of those differ, which says nothing about which reading came first.
///
/// So a token of the spelling an earlier release gave a task or a project with the cache
/// attached — a relations part alone — is never ordered beside one of this spelling, and an entry
/// that release wrote is read again.
#[must_use]
pub fn order(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parts(a)?, parts(b)?);
    (a.facets == b.facets && a.extra == b.extra).then(|| a.updated.cmp(&b.updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An issue as the cache's widened reads answer it: no comments, no inverse relations, and
    /// `relations` as given.
    fn issue(updated: &str, label: &str, relations: Value) -> Value {
        json!({
            "id": "i1", "identifier": "ENG-1", "url": "https://linear.app/x/issue/ENG-1",
            "updatedAt": updated, "state": {"name": "Todo", "type": "unstarted"},
            "project": {"id": "p1"},
            "labels": {"nodes": [{"id": "l1", "name": label, "color": "#ff0000"}]},
            "comments": {"nodes": [], "pageInfo": {"hasNextPage": false}},
            "relations": relations,
            "inverseRelations": inverse(&[], false),
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

    fn inverse(rows: &[(&str, &str)], more: bool) -> Value {
        json!({
            "nodes": rows.iter().map(|(id, near)| json!({"id": id, "type": "blocks",
                                                         "issue": {"id": near}}))
                .collect::<Vec<_>>(),
            "pageInfo": {"hasNextPage": more},
        })
    }

    fn comments(rows: &[(&str, &str)], more: bool) -> Value {
        json!({
            "nodes": rows.iter().map(|(id, updated)| json!({"id": id, "updatedAt": updated}))
                .collect::<Vec<_>>(),
            "pageInfo": {"hasNextPage": more},
        })
    }

    #[test]
    fn a_later_time_with_the_same_facets_and_parts_orders_after() {
        let none = page(&[], false);
        let before = task(&issue("2026-10-01T00:00:00Z", "bug", none.clone()), true).unwrap();
        let after = task(&issue("2026-10-01T00:00:01Z", "bug", none), true).unwrap();
        assert_eq!(order(&after, &before), Some(Ordering::Greater));
        assert_eq!(order(&before, &after), Some(Ordering::Less));
        assert_eq!(order(&before, &before), Some(Ordering::Equal));
    }

    #[test]
    fn another_facet_or_part_is_never_ordered() {
        let none = page(&[], false);
        let held = issue("2026-10-01T00:00:00Z", "bug", none.clone());
        let before = task(&held, true).unwrap();
        let renamed = task(&issue("2026-10-01T00:00:05Z", "defect", none.clone()), true).unwrap();
        assert_eq!(order(&renamed, &before), None, "a label renamed");
        let related = task(
            &issue("2026-10-01T00:00:05Z", "bug", page(&[("r1", "i2")], false)),
            true,
        )
        .unwrap();
        assert_eq!(order(&related, &before), None, "a relation added");
        let mut depended = held.clone();
        depended["inverseRelations"] = inverse(&[("r2", "i3")], false);
        assert_eq!(
            order(&task(&depended, true).unwrap(), &before),
            None,
            "an inverse relation added"
        );
        let mut commented = held.clone();
        commented["comments"] = comments(&[("c1", "2026-10-01T00:00:00Z")], false);
        let commented_token = task(&commented, true).unwrap();
        assert_eq!(order(&commented_token, &before), None, "a comment added");
        let mut edited = commented.clone();
        edited["comments"] = comments(&[("c1", "2026-10-01T00:00:09Z")], false);
        assert_eq!(
            order(&task(&edited, true).unwrap(), &commented_token),
            None,
            "a comment edited"
        );
        assert_eq!(order(&before, &commented_token), None, "a comment deleted");
        let unattached = task(&issue("2026-10-01T00:00:00Z", "bug", none), false).unwrap();
        assert!(!unattached.contains("|r:") && !unattached.contains("|c:"));
        assert_eq!(
            order(&unattached, &before),
            None,
            "a token with the cache's parts and one without"
        );
        assert_eq!(order("gh3|x", &before), None);
        assert_eq!(order(&format!("{before}|extra"), &before), None);
    }

    #[test]
    fn the_earlier_spelling_of_an_attached_token_is_never_ordered_beside_this_one() {
        let held = issue("2026-10-01T00:00:00Z", "bug", page(&[], false));
        let now = task(&held, true).unwrap();
        // What the release before this one spelled for the same issue: its relations part alone.
        let relations = relation_digest(&held, "relations", "relatedIssue").unwrap();
        let earlier = format!("{}|r:{relations}", task(&held, false).unwrap());
        assert!(parts(&earlier).is_some(), "it still reads as a token");
        assert_eq!(order(&earlier, &now), None);
        assert_eq!(order(&now, &earlier), None);
        // A part out of its one order, or twice, is no token of this plugin.
        let (base, rest) = now.split_once("|c:").unwrap();
        let comments_part = rest.split('|').next().unwrap();
        assert_eq!(
            order(&format!("{base}|r:{relations}|c:{comments_part}"), &now),
            None
        );
        assert!(parts(&format!("{base}|r:{relations}|c:{comments_part}")).is_none());
        assert!(parts(&format!("{base}|c:{comments_part}|c:{comments_part}")).is_none());
    }

    #[test]
    fn a_part_read_from_part_of_a_connection_gives_no_token() {
        let more = page(&[("r1", "i2")], true);
        assert_eq!(
            task(&issue("2026-10-01T00:00:00Z", "bug", more), true),
            None
        );
        let rows: Vec<(String, String)> = (0..=PAGE)
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
        let whole = issue("2026-10-01T00:00:00Z", "bug", page(&[], false));
        for (connection, value) in [
            ("inverseRelations", inverse(&[("r1", "i2")], true)),
            ("inverseRelations", inverse(&eleven, false)),
            (
                "comments",
                comments(&[("c1", "2026-10-01T00:00:00Z")], true),
            ),
            (
                "comments",
                json!({"nodes": [], "pageInfo": {"hasPreviousPage": true}}),
            ),
        ] {
            let mut partial = whole.clone();
            partial[connection] = value.clone();
            assert_eq!(task(&partial, true), None, "{connection}: {value}");
            assert!(
                task(&partial, false).is_some(),
                "{connection}: a token without the cache's parts reads none of them"
            );
        }
        let stamps: Vec<String> = (0..=PAGE).map(|n| format!("c{n}")).collect();
        let crowd: Vec<(&str, &str)> = stamps
            .iter()
            .map(|id| (id.as_str(), "2026-10-01T00:00:00Z"))
            .collect();
        let mut crowded = whole.clone();
        crowded["comments"] = comments(&crowd, false);
        assert_eq!(task(&crowded, true), None, "eleven comments");
        let mut crowded = whole;
        crowded["labels"]["nodes"] = Value::Array(
            (0..DEFAULT_LABEL_PAGE)
                .map(|n| json!({"id": format!("l{n}"), "name": "x", "color": null}))
                .collect(),
        );
        assert_eq!(task(&crowded, true), None);
        assert_eq!(task(&crowded, false), None);
    }

    #[test]
    fn a_comments_page_read_backwards_or_forwards_spells_one_part() {
        let mut forwards = issue("2026-10-01T00:00:00Z", "bug", page(&[], false));
        forwards["comments"] = comments(
            &[
                ("c1", "2026-10-01T00:00:00Z"),
                ("c2", "2026-10-01T00:00:01.000Z"),
            ],
            false,
        );
        let mut backwards = forwards.clone();
        backwards["comments"] = json!({
            "nodes": [
                {"id": "c2", "body": "two", "updatedAt": "2026-10-01T00:00:01Z",
                 "user": {"displayName": "a person"}},
                {"id": "c1", "body": "one", "updatedAt": "2026-10-01T00:00:00Z", "user": null},
            ],
            "pageInfo": {"hasPreviousPage": false, "startCursor": "c1"},
        });
        assert_eq!(task(&forwards, true), task(&backwards, true));
        assert!(task(&forwards, true).is_some());
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
        let mut broken = whole;
        broken["comments"] = comments(&[("c1", "yesterday")], false);
        assert_eq!(
            task(&broken, true),
            None,
            "a comment time that is no instant"
        );
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
    fn the_pages_a_token_is_read_from_are_the_ones_the_cache_selects() {
        for selected in [
            crate::graphql::RELATIONS_PAGE,
            crate::graphql::INVERSE_RELATIONS_PAGE,
            crate::graphql::COMMENTS_PAGE,
            crate::graphql::PROJECT_RELATIONS_PAGE,
        ] {
            assert!(
                selected.contains(&format!("(first:{PAGE})")),
                "{selected} selects a page of {PAGE}"
            );
        }
    }

    #[test]
    fn a_projects_token_carries_its_relations_only_attached() {
        let project = |relations: Value| {
            json!({"updatedAt": "2026-10-01T00:00:00Z", "status": {"name": "Doing", "type": "started"},
                   "labels": {"nodes": []}, "relations": relations})
        };
        let related = json!({"nodes": [{"id": "pr1", "type": "dependency",
                                        "relatedProject": {"id": "p2"}}],
                             "pageInfo": {"hasNextPage": false}});
        let none = json!({"nodes": [], "pageInfo": {"hasNextPage": false}});
        let plain = project_token(&project(none.clone()), false).unwrap();
        assert!(!plain.contains("|r:"));
        let attached = project_token(&project(none), true).unwrap();
        let added = project_token(&project(related), true).unwrap();
        assert_eq!(order(&attached, &added), None, "a project relation added");
        assert_eq!(order(&plain, &attached), None);
        assert_eq!(without_relations(&added), plain);
        let more = json!({"nodes": [], "pageInfo": {"hasNextPage": true}});
        assert_eq!(project_token(&project(more), true), None);
    }

    #[test]
    fn a_documents_token_moves_with_its_content_and_the_relations_parts_come_off() {
        let document = |content: &str| {
            json!({"title": "Design", "url": "https://linear.app/x/document/design", "project": null,
                   "updatedAt": "2026-10-01T00:00:00Z", "content": content})
        };
        let one = document_token(&document("a")).unwrap();
        let other = document_token(&document("a, edited")).unwrap();
        assert_eq!(order(&one, &other), None);
        let held = issue("2026-10-01T00:00:00Z", "bug", page(&[], false));
        let token = task(&held, true).unwrap();
        assert_eq!(
            without_relations(&token),
            task_before_relations(&held).unwrap()
        );
        assert!(without_relations(&token).contains("|c:"));
        assert!(!without_relations(&token).contains("|i:"));
    }
}
