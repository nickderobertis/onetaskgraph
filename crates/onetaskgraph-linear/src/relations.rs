//! A Linear issue's parent and its native `related` relations, as the store's relation calls
//! read and write them.
//!
//! A parent is the issue's own `parent`, written through `issueUpdate(input: {parentId})` and
//! removed with `parentId: null`; a parent in another team of the same workspace is Linear's to
//! accept. The store states no limit of its own on depth or children — Linear accepted at least
//! ten levels and 105 children when spike `spike-relations` probed it on 2026-11-08 — and a
//! refusal Linear gives one edge, a cycle's `invalid parent` among them, is that edge's answer
//! alone.
//!
//! A link's native mirror is Linear's `related` issue relation, which is symmetric: created
//! twice in either direction Linear answers the same relation, so the type name and the
//! direction live in the store's metadata alone.

use std::collections::BTreeSet;

use onetaskgraph_plugin_api::{
    NativeId, NativeLink, ParentAnswer, ParentWrite, Relations, SourceError, TaskLink, version_at,
};
use serde_json::{Value, json};

use super::batch::AliasedEach;
use super::{LinearSource, MAX_PAGE_SIZE, graphql, map_task, metadata_description, optional_str};

/// Linear's word for the one issue relation a link is mirrored as.
pub const MIRRORED: &str = "related";

/// Why a `link_types` word Linear could mean is never a native mirror here: blocking is what a
/// task's `depends_on` already writes as Linear's `blocks` relation, and a native `duplicate`
/// moves the issue it marks to `Duplicate`.
pub(crate) const REFUSED_WORDS: &[(&str, &str)] = &[
    (
        "blocks",
        "blocking is what a task's `depends_on` already means, written as Linear's `blocks` relation, so no link type is mirrored as it",
    ),
    (
        "blocked-by",
        "blocking is what a task's `depends_on` already means, written as Linear's `blocks` relation, so no link type is mirrored as it",
    ),
    (
        "duplicate",
        "a native duplicate moves the issue to `Duplicate` on Linear, so a duplicate link stays in the store's metadata alone",
    ),
    (
        "similar",
        "Linear's `similar` is not offered as a mirror; a duplicate or similar link stays in the store's metadata alone",
    ),
];

/// One issue as a relation read answers it.
struct Read {
    relations: Relations,
}

impl LinearSource {
    /// The native ids of the issues this source holds that the links in `description`'s
    /// metadata slot name under a type this source mirrors as Linear's `related` relation:
    /// each a `related` relation that is a link's mirror rather than a `related` dependency.
    ///
    /// Empty for an issue holding no links, so nothing about its edges changes.
    pub(crate) fn mirrored_in(&self, description: Option<&str>) -> BTreeSet<String> {
        let mirrored = self.link_types.mirrored();
        if mirrored.is_empty() {
            return BTreeSet::new();
        }
        let Ok((_, metadata)) = metadata_description(description.map(str::to_owned)) else {
            return BTreeSet::new();
        };
        let prefix = format!("{}:", self.name);
        TaskLink::held(&metadata)
            .unwrap_or_default()
            .into_iter()
            .filter(|link| mirrored.contains_key(&link.link_type))
            .filter_map(|link| link.other.strip_prefix(&prefix).map(str::to_owned))
            .collect()
    }

    /// Every one of `ids` read as one aliased [`graphql::ISSUE_PARENT_READ`] — `None` for an
    /// issue Linear does not hold, has trashed, or that is outside this source's project.
    pub(crate) async fn relations_read(
        &self,
        ids: &[NativeId],
    ) -> Result<Vec<Option<Relations>>, SourceError> {
        let mut read: Vec<Option<Result<Option<Read>, SourceError>>> =
            ids.iter().map(|_| None).collect();
        let mut pending: Vec<usize> = (0..ids.len()).collect();
        while !pending.is_empty() {
            let each: Vec<Value> = pending.iter().map(|at| json!({"id": ids[*at].0})).collect();
            match self
                .send_aliased_each(graphql::ISSUE_PARENT_READ, &each)
                .await?
            {
                AliasedEach::Answered(values) => {
                    for (at, value) in pending.iter().zip(values) {
                        read[*at] = Some(value.and_then(|issue| {
                            answered_for(&issue, &ids[*at])?;
                            self.relations_of(&issue)
                        }));
                    }
                    pending.clear();
                }
                AliasedEach::Refused(refused, refusal) => {
                    let at = pending.remove(refused);
                    read[at] = Some(if refusal.entity_missing() {
                        Ok(None)
                    } else {
                        Err(refusal.into_error())
                    });
                }
            }
        }
        read.into_iter()
            .map(|read| {
                read.unwrap_or_else(|| {
                    Err(SourceError::Malformed {
                        message: "an issue of the relation read was given no answer".to_owned(),
                    })
                })
                .map(|read| read.map(|read| read.relations))
            })
            .collect()
    }

    /// One issue a relation read answered, as its relations — `None` for one trashed or out of
    /// this source's scope.
    fn relations_of(&self, issue: &Value) -> Result<Option<Read>, SourceError> {
        if issue.is_null() || issue.get("archivedAt").is_some_and(|at| !at.is_null()) {
            return Ok(None);
        }
        let task = map_task(issue, &self.name, &self.statuses, self.spelling())?;
        if !self.in_scope(task.project.as_ref()) {
            return Ok(None);
        }
        let parent = match issue.get("parent") {
            None => {
                return Err(SourceError::Malformed {
                    message: "a relation read answered an issue without its parent".into(),
                });
            }
            Some(Value::Null) => None,
            Some(parent) => Some(NativeId(
                optional_str(parent, "id")?
                    .ok_or_else(|| SourceError::Malformed {
                        message: "a relation read answered a parent without an id".into(),
                    })?
                    .to_owned(),
            )),
        };
        let children = issue
            .get("children")
            .ok_or_else(|| SourceError::Malformed {
                message: "a relation read answered an issue without its children".into(),
            })?;
        let nodes = children
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "missing children.nodes".into(),
            })?;
        let children_ids = nodes
            .iter()
            .map(|child| {
                child
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| NativeId(id.to_owned()))
                    .ok_or_else(|| SourceError::Malformed {
                        message: "a child without an id".into(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let more = children
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            .ok_or_else(|| SourceError::Malformed {
                message: "a relation read answered children without pageInfo.hasNextPage".into(),
            })?;
        Ok(Some(Read {
            relations: Relations {
                task,
                parent,
                children: children_ids,
                children_complete: !more,
            },
        }))
    }

    /// Every parent write: the children and every parent named read in one aliased read, each
    /// write decided on it, and every write that changes something sent in one aliased
    /// [`graphql::ISSUE_PARENT_UPDATE`] document — a refused alias answered on its own and the
    /// rest sent together again, because Linear applies an aliased document of mutations whole
    /// or not at all.
    pub(crate) async fn parents_written(
        &self,
        writes: &[ParentWrite],
    ) -> Vec<Result<ParentAnswer, SourceError>> {
        let mut wanted: Vec<NativeId> = Vec::new();
        for write in writes {
            for id in std::iter::once(&write.child).chain(write.parent.as_ref()) {
                if !wanted.contains(id) {
                    wanted.push(id.clone());
                }
            }
        }
        let read = match self.relations_read(&wanted).await {
            Ok(read) => read,
            Err(error) => return writes.iter().map(|_| Err(error.clone())).collect(),
        };
        let held = |id: &NativeId| {
            wanted
                .iter()
                .position(|asked| asked == id)
                .and_then(|at| read[at].as_ref())
        };
        let mut answers: Vec<Option<Result<ParentAnswer, SourceError>>> =
            writes.iter().map(|_| None).collect();
        let mut sending: Vec<usize> = Vec::new();
        // A write naming a child an earlier write of this call names is made after the batch,
        // alone and in order, decided on what the writes before it left.
        let mut later: Vec<usize> = Vec::new();
        for (at, write) in writes.iter().enumerate() {
            if writes[..at]
                .iter()
                .any(|earlier| earlier.child == write.child)
            {
                later.push(at);
                continue;
            }
            let Some(child) = held(&write.child) else {
                answers[at] = Some(Ok(ParentAnswer::Missing {
                    what: write.child.clone(),
                }));
                continue;
            };
            if let Some(expected) = &write.if_version
                && !version_at(self, child.task.version.as_deref(), expected)
            {
                answers[at] = Some(Ok(ParentAnswer::VersionMoved {
                    found: child.task.version.clone(),
                }));
                continue;
            }
            if let Some(parent) = &write.parent
                && held(parent).is_none()
            {
                answers[at] = Some(Ok(ParentAnswer::Missing {
                    what: parent.clone(),
                }));
                continue;
            }
            if child.parent == write.parent {
                answers[at] = Some(Ok(ParentAnswer::Unchanged {
                    version: child.task.version.clone(),
                }));
                continue;
            }
            sending.push(at);
        }
        while !sending.is_empty() {
            let each: Vec<Value> = sending
                .iter()
                .map(|at| {
                    let write = &writes[*at];
                    json!({"id": write.child.0,
                           "input": {"parentId": write.parent.as_ref().map(|parent| &parent.0)}})
                })
                .collect();
            match self
                .send_aliased_each(graphql::ISSUE_PARENT_UPDATE, &each)
                .await
            {
                Ok(AliasedEach::Answered(values)) => {
                    for (at, value) in sending.drain(..).zip(values) {
                        answers[at] =
                            Some(value.and_then(|payload| {
                                self.parent_answer(&payload, &writes[at].child)
                            }));
                    }
                }
                Ok(AliasedEach::Refused(refused, refusal)) => {
                    let at = sending.remove(refused);
                    answers[at] = Some(Ok(if refusal.entity_missing() {
                        ParentAnswer::Missing {
                            what: writes[at].child.clone(),
                        }
                    } else {
                        ParentAnswer::Refused {
                            reason: format!(
                                "Linear refused making {} the parent of {}: {}",
                                writes[at]
                                    .parent
                                    .as_ref()
                                    .map_or_else(|| "nothing".to_owned(), ToString::to_string),
                                writes[at].child,
                                refusal.0.said()
                            ),
                        }
                    }));
                }
                Err(error) => {
                    for at in sending.drain(..) {
                        answers[at] = Some(Err(error.clone()));
                    }
                }
            }
        }
        for at in later {
            let write = &writes[at];
            // What the writes of this child before it left: the parent and version the last of
            // them wrote, or the child as it was read when none wrote it.
            let mut state =
                held(&write.child).map(|child| (child.parent.clone(), child.task.version.clone()));
            for earlier in (0..at).filter(|earlier| writes[*earlier].child == write.child) {
                if let Some(Ok(ParentAnswer::Written { version })) = &answers[earlier] {
                    state = Some((writes[earlier].parent.clone(), version.clone()));
                }
            }
            answers[at] = Some(match state {
                None => Ok(ParentAnswer::Missing {
                    what: write.child.clone(),
                }),
                Some((_, version))
                    if write.if_version.as_ref().is_some_and(|expected| {
                        !version_at(self, version.as_deref(), expected)
                    }) =>
                {
                    Ok(ParentAnswer::VersionMoved { found: version })
                }
                Some(_)
                    if write
                        .parent
                        .as_ref()
                        .is_some_and(|parent| held(parent).is_none()) =>
                {
                    Ok(ParentAnswer::Missing {
                        what: write.parent.clone().unwrap_or_else(|| write.child.clone()),
                    })
                }
                Some((parent, version)) if parent == write.parent => {
                    Ok(ParentAnswer::Unchanged { version })
                }
                Some(_) => self.parent_alone(write).await,
            });
        }
        answers
            .into_iter()
            .map(|answer| {
                answer.unwrap_or_else(|| {
                    Err(SourceError::Malformed {
                        message: "a parent write of the batch was given no answer".to_owned(),
                    })
                })
            })
            .collect()
    }

    /// One parent write on its own, after the batch an earlier write of the same child went in.
    async fn parent_alone(&self, write: &ParentWrite) -> Result<ParentAnswer, SourceError> {
        let each = [json!({"id": write.child.0,
                           "input": {"parentId": write.parent.as_ref().map(|parent| &parent.0)}})];
        match self
            .send_aliased_each(graphql::ISSUE_PARENT_UPDATE, &each)
            .await?
        {
            AliasedEach::Answered(mut values) => {
                let value = values.pop().ok_or_else(|| SourceError::Malformed {
                    message: "a parent write was given no answer".to_owned(),
                })??;
                self.parent_answer(&value, &write.child)
            }
            AliasedEach::Refused(_, refusal) if refusal.entity_missing() => {
                Ok(ParentAnswer::Missing {
                    what: write.child.clone(),
                })
            }
            AliasedEach::Refused(_, refusal) => Ok(ParentAnswer::Refused {
                reason: format!(
                    "Linear refused making {} the parent of {}: {}",
                    write
                        .parent
                        .as_ref()
                        .map_or_else(|| "nothing".to_owned(), ToString::to_string),
                    write.child,
                    refusal.0.said()
                ),
            }),
        }
    }

    /// What one `issueUpdate` of a parent answered, as the child's version after it.
    fn parent_answer(
        &self,
        payload: &Value,
        child: &NativeId,
    ) -> Result<ParentAnswer, SourceError> {
        let task = self.updated_issue(&json!({"issueUpdate": payload}), child)?;
        Ok(match task {
            Some(task) => ParentAnswer::Written {
                version: task.version,
            },
            None => ParentAnswer::Missing {
                what: child.clone(),
            },
        })
    }

    /// Every `related` relation of `links` created in one aliased
    /// [`graphql::ISSUE_RELATION_CREATE`] document — one Linear refuses answered on its own and
    /// the rest sent together again.
    pub(crate) async fn native_links_added(
        &self,
        links: &[NativeLink],
    ) -> Vec<Result<(), SourceError>> {
        let mut answers: Vec<Option<Result<(), SourceError>>> =
            links.iter().map(|_| None).collect();
        let mut sending: Vec<usize> = (0..links.len()).collect();
        while !sending.is_empty() {
            let each: Vec<Value> = sending
                .iter()
                .map(|at| {
                    json!({"input": {"issueId": links[*at].from.0,
                                     "relatedIssueId": links[*at].to.0,
                                     "type": MIRRORED}})
                })
                .collect();
            match self
                .send_aliased_each(graphql::ISSUE_RELATION_CREATE, &each)
                .await
            {
                Ok(AliasedEach::Answered(values)) => {
                    for (at, value) in sending.drain(..).zip(values) {
                        answers[at] = Some(value.and_then(|payload| succeeded(&payload)));
                        self.unknown_version(&links[at].from);
                        self.unknown_version(&links[at].to);
                    }
                }
                Ok(AliasedEach::Refused(refused, refusal)) => {
                    let at = sending.remove(refused);
                    answers[at] = Some(Err(refusal.into_error()));
                }
                Err(error) => {
                    for at in sending.drain(..) {
                        answers[at] = Some(Err(error.clone()));
                    }
                }
            }
        }
        answers
            .into_iter()
            .map(|answer| answer.unwrap_or(Ok(())))
            .collect()
    }

    /// Every `related` relation between each pair of `links` deleted: the relations of each
    /// pair's `from` read in one aliased [`graphql::ISSUE_RELATIONS`] read of its largest page —
    /// both directions, because Linear holds the one relation with whichever end created it
    /// first — and every one found deleted in one aliased [`graphql::ISSUE_RELATION_DELETE`]
    /// document. A pair with no such relation is no change; one whose `from` holds more
    /// relations than that page is refused, because what it did not read could hold the
    /// relation.
    pub(crate) async fn native_links_removed(
        &self,
        links: &[NativeLink],
    ) -> Vec<Result<(), SourceError>> {
        let each: Vec<Value> = links
            .iter()
            .map(|link| json!({"id": link.from.0, "first": MAX_PAGE_SIZE, "after": null}))
            .collect();
        let read = match self
            .send_aliased_each(graphql::ISSUE_RELATIONS, &each)
            .await
        {
            Ok(AliasedEach::Answered(values)) => values,
            Ok(AliasedEach::Refused(_, refusal)) => {
                let error = refusal.into_error();
                return links.iter().map(|_| Err(error.clone())).collect();
            }
            Err(error) => return links.iter().map(|_| Err(error.clone())).collect(),
        };
        let mut answers: Vec<Option<Result<(), SourceError>>> =
            links.iter().map(|_| None).collect();
        let mut deleting: Vec<(usize, String)> = Vec::new();
        for (at, (link, value)) in links.iter().zip(read).enumerate() {
            // Positional: the relations read selects no id to check an alias's answer against.
            match value.and_then(|issue| related_between(&issue, &link.from, &link.to)) {
                Ok(found) if found.is_empty() => answers[at] = Some(Ok(())),
                Ok(found) => deleting.extend(found.into_iter().map(|id| (at, id))),
                Err(error) => answers[at] = Some(Err(error)),
            }
        }
        while !deleting.is_empty() {
            let each: Vec<Value> = deleting.iter().map(|(_, id)| json!({"id": id})).collect();
            match self
                .send_aliased_each(graphql::ISSUE_RELATION_DELETE, &each)
                .await
            {
                Ok(AliasedEach::Answered(values)) => {
                    for ((at, _), value) in deleting.drain(..).zip(values) {
                        let done = value.and_then(|payload| succeeded(&payload));
                        let earlier = answers[at].take().unwrap_or(Ok(()));
                        answers[at] = Some(earlier.and(done));
                        self.unknown_version(&links[at].from);
                        self.unknown_version(&links[at].to);
                    }
                }
                Ok(AliasedEach::Refused(refused, refusal)) => {
                    let (at, _) = deleting.remove(refused);
                    answers[at] = Some(Err(refusal.into_error()));
                }
                Err(error) => {
                    for (at, _) in deleting.drain(..) {
                        answers[at] = Some(Err(error.clone()));
                    }
                }
            }
        }
        answers
            .into_iter()
            .map(|answer| answer.unwrap_or(Ok(())))
            .collect()
    }
}

/// Refuse an alias's answer naming another issue than the one it asked for.
fn answered_for(issue: &Value, asked: &NativeId) -> Result<(), SourceError> {
    match issue.get("id").and_then(Value::as_str) {
        Some(id) if id == asked.0 => Ok(()),
        _ if issue.is_null() => Ok(()),
        answered => Err(SourceError::Malformed {
            message: format!(
                "Linear answered the alias asking for {asked} with {}",
                answered.map_or_else(|| "an issue without an id".to_owned(), str::to_owned)
            ),
        }),
    }
}

/// The ids of every `related` relation `issue` holds with `other`, either way round — every
/// one, because a connection with another page is refused rather than read in part, and a row
/// that does not say its id, its type or its other end is refused rather than passed over.
fn related_between(
    issue: &Value,
    near: &NativeId,
    other: &NativeId,
) -> Result<Vec<String>, SourceError> {
    if issue.is_null() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for (connection, end) in [("relations", "relatedIssue"), ("inverseRelations", "issue")] {
        let held = issue
            .get(connection)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {connection}"),
            })?;
        match held
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
        {
            Some(false) => {}
            Some(true) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "{near} holds more {connection} than one page of {MAX_PAGE_SIZE}, so \
                         whether one mirrors this link cannot be told; nothing was removed"
                    ),
                });
            }
            None => {
                return Err(SourceError::Malformed {
                    message: format!("missing {connection}.pageInfo.hasNextPage"),
                });
            }
        }
        for relation in
            held.get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: format!("missing {connection}.nodes"),
                })?
        {
            let malformed = |what: &str| SourceError::Malformed {
                message: format!("a relation of {connection} without {what}"),
            };
            let id = relation
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("an id"))?;
            let kind = relation
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("a type"))?;
            let far = relation
                .pointer(&format!("/{end}/id"))
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("its other end"))?;
            if far == other.0 && kind == MIRRORED {
                found.push(id.to_owned());
            }
        }
    }
    Ok(found)
}

/// A mutation payload's `success`, as its write's answer.
fn succeeded(payload: &Value) -> Result<(), SourceError> {
    if payload.get("success").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(SourceError::Malformed {
            message: format!("Linear answered a relation write without success: {payload}"),
        })
    }
}
