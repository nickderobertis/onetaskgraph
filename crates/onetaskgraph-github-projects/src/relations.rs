//! Relations on a GitHub board: an issue's parent, which is its sub-issue link when the source
//! reads sub-issues as children (`sub_issues: parent`), and the `relates-to` relation a link type
//! is mirrored as.
//!
//! Every write here goes in aliased documents of at most [`RELATION_BATCH`] fields, each field
//! answered on its own: GitHub runs each alias by itself and answers a refusal — a cycle, a
//! depth or children limit — at that alias's path. Every relation mutation is paced like every
//! other mutation this source sends, each aliased field one content creation. A relation write
//! whose answer never arrived — a gateway timeout after GitHub ran part of it — is read back and
//! each relation answered from what GitHub holds, never sent again blindly.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    NativeId, NativeLink, ParentAnswer, ParentWrite, Relations, SourceError, version_at,
};
use serde_json::{Map, Value, json};

use super::batch::{alias_answer, errors_at};
use super::{
    BOARD_ITEMS_PAGE_SIZE, BoardKind, DETAIL_BATCH, GitHubProjectsSource, MAX_PAGE_SIZE,
    NESTED_PAGE_SIZE, RELATION_BATCH, Resolved, SubIssues, graphql, optional_str, required_str,
};
use onetaskgraph_plugin_api::ItemKind;

/// The one native word a link type may be mirrored as on a GitHub board.
pub(crate) const RELATES_TO: &str = "relates-to";

/// Why a `link_types` word this board could mean is never a native mirror here: blocking is
/// what a task's `depends_on` already writes as `blockedBy`, and a native duplicate closes the
/// issue it marks.
pub(crate) const REFUSED_WORDS: &[(&str, &str)] = &[
    (
        "blocks",
        "blocking is what a task's `depends_on` already means, written as GitHub's `blockedBy`, so no link type is mirrored as it",
    ),
    (
        "blocked-by",
        "blocking is what a task's `depends_on` already means, written as GitHub's `blockedBy`, so no link type is mirrored as it",
    ),
    (
        "duplicate",
        "GitHub marks a duplicate only by closing the issue, so a duplicate link stays in the store's metadata alone",
    ),
    (
        "similar",
        "GitHub has no similar relation; a duplicate or similar link stays in the store's metadata alone",
    ),
];

/// Which way a native `relates-to` write goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeWrite {
    /// `addRelatesTo`: afterwards the pair is related.
    Add,
    /// `removeRelatesTo`: afterwards the pair is not.
    Remove,
}

/// One parent write, as it goes out.
enum ParentOp {
    /// `addSubIssue` of `child` under `parent`, replacing the parent it has when `replace`.
    Add {
        parent: NativeId,
        child: NativeId,
        replace: bool,
    },
    /// `removeSubIssue` of `child` from `parent`.
    Remove { parent: NativeId, child: NativeId },
}

impl ParentOp {
    fn child(&self) -> &NativeId {
        match self {
            Self::Add { child, .. } | Self::Remove { child, .. } => child,
        }
    }

    /// The issue the write adds `child` under or removes it from.
    fn parent(&self) -> &NativeId {
        match self {
            Self::Add { parent, .. } | Self::Remove { parent, .. } => parent,
        }
    }
}

impl GitHubProjectsSource {
    /// Refuse a project write on a source reading sub-issues as children, naming the setting.
    pub(super) fn projectless(&self) -> Result<(), SourceError> {
        match self.sub_issues {
            SubIssues::Project => Ok(()),
            SubIssues::Parent => Err(SourceError::Refused {
                message: format!(
                    "source {} reads sub-issues as children (`sub_issues: parent`), so it holds \
                     no projects and refuses a project write; next: write the project to a source \
                     whose board keeps the default `sub_issues: project`",
                    self.name
                ),
            }),
        }
    }

    /// The refusal a parent write to a source reading sub-issues as projects answers.
    fn parentless(&self) -> SourceError {
        SourceError::Refused {
            message: format!(
                "source {} reads an issue's sub-issues as a project's tasks (`sub_issues: \
                 project`, the default), so it holds no parent relation; next: set \
                 `sources.{}.config.sub_issues: parent` on a source whose board is its own",
                self.name, self.name
            ),
        }
    }

    /// [`TaskSource::read_relations`](onetaskgraph_plugin_api::TaskSource::read_relations):
    /// every id read together — with each issue's parent and a page of its sub-issues on a
    /// source reading them as children, and as a batched read of each item alone on one reading
    /// them as projects, where no task has a parent.
    pub(super) async fn relations_of(
        &self,
        ids: &[NativeId],
    ) -> Result<Vec<Option<Relations>>, SourceError> {
        if self.sub_issues == SubIssues::Project {
            return self
                .items_by_ids(ids)
                .await?
                .into_iter()
                .map(|item| match item {
                    Some(item) if item.kind == BoardKind::Work(ItemKind::Task) => {
                        Ok(Some(Relations {
                            task: item.task()?,
                            parent: None,
                            children: Vec::new(),
                            children_complete: true,
                        }))
                    }
                    _ => Ok(None),
                })
                .collect();
        }
        let mut answers = Vec::with_capacity(ids.len());
        for batch in ids.chunks(DETAIL_BATCH) {
            let mut variables = Map::new();
            for slot in 0..DETAIL_BATCH {
                let id = batch.get(slot).or(batch.last()).map(|id| id.0.clone());
                variables.insert(format!("id{slot}"), json!(id));
            }
            variables.insert("first".to_owned(), json!(MAX_PAGE_SIZE));
            variables.insert("nestedFirst".to_owned(), json!(NESTED_PAGE_SIZE));
            variables.insert("boardItems".to_owned(), json!(BOARD_ITEMS_PAGE_SIZE));
            variables.insert("duplicates".to_owned(), json!(true));
            // Not one of the documents whose answer keeps its errors beside its data
            // (`send_once`): a response carrying any GraphQL error — one at an alias's path
            // included — is refused whole here, so no slot below is read from a part-answer.
            let data = self
                .graphql(graphql::ISSUE_RELATIONS, Value::Object(variables))
                .await?;
            for (slot, id) in batch.iter().enumerate() {
                let node = data
                    .get(format!("i{slot}"))
                    .ok_or_else(|| SourceError::Malformed {
                        message: format!(
                            "GitHub answered a relations read with no item for {}",
                            id.0
                        ),
                    })?;
                if node.is_null() || optional_str(node, "__typename")? != Some("Issue") {
                    answers.push(None);
                    continue;
                }
                if required_str(node, "id")? != id.0 {
                    return Err(SourceError::Malformed {
                        message: format!(
                            "GitHub answered the relations read of {} with issue {}",
                            id.0,
                            required_str(node, "id")?
                        ),
                    });
                }
                let Some(item) = self.resolve_issue(node).await? else {
                    answers.push(None);
                    continue;
                };
                if item.kind != BoardKind::Work(ItemKind::Task) {
                    answers.push(None);
                    continue;
                }
                let connection = node
                    .get("subIssues")
                    .ok_or_else(|| SourceError::Malformed {
                        message: format!("GitHub issue {} is missing subIssues", id.0),
                    })?;
                let children = connection
                    .get("nodes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SourceError::Malformed {
                        message: format!("GitHub issue {} subIssues.nodes is not an array", id.0),
                    })?
                    .iter()
                    .map(|child| required_str(child, "id").map(|id| NativeId(id.to_owned())))
                    .collect::<Result<Vec<_>, _>>()?;
                let more = connection
                    .pointer("/pageInfo/hasNextPage")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| SourceError::Malformed {
                        message: format!("GitHub issue {} subIssues has no pageInfo", id.0),
                    })?;
                answers.push(Some(Relations {
                    parent: item.issue_parent.clone(),
                    task: item.task()?,
                    children,
                    children_complete: !more,
                }));
            }
        }
        Ok(answers)
    }

    /// [`TaskSource::set_parents`](onetaskgraph_plugin_api::TaskSource::set_parents): every child
    /// read together, each write decided on that read, and the writes sent in
    /// [`graphql::SET_PARENTS`] documents of at most [`RELATION_BATCH`] fields.
    pub(super) async fn parents_set(
        &self,
        writes: &[ParentWrite],
    ) -> Vec<Result<ParentAnswer, SourceError>> {
        if self.sub_issues == SubIssues::Project {
            return writes.iter().map(|_| Err(self.parentless())).collect();
        }
        let mut answers: Vec<Option<Result<ParentAnswer, SourceError>>> =
            writes.iter().map(|_| None).collect();
        let children: Vec<NativeId> = writes.iter().fold(Vec::new(), |mut ids, write| {
            if !ids.contains(&write.child) {
                ids.push(write.child.clone());
            }
            ids
        });
        let read = match self.items_by_ids(&children).await {
            Ok(read) => read,
            Err(error) => return writes.iter().map(|_| Err(error.clone())).collect(),
        };
        let mut held: BTreeMap<NativeId, Resolved> = children
            .into_iter()
            .zip(read)
            .filter_map(|(id, item)| {
                item.filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
                    .map(|item| (id, item))
            })
            .collect();
        // The writes of one child go in batch order, one a round: each later one is decided on
        // what the one before it left — a refused edge leaving the parent it found, a written one
        // leaving its own and a version nothing read — and never sent beside it, because one
        // document runs its `addSubIssue` fields before its `removeSubIssue` ones.
        let mut rounds: Vec<Vec<usize>> = Vec::new();
        let mut seen: BTreeMap<&NativeId, usize> = BTreeMap::new();
        for (at, write) in writes.iter().enumerate() {
            let round = seen.entry(&write.child).or_insert(0);
            if rounds.len() <= *round {
                rounds.push(Vec::new());
            }
            rounds[*round].push(at);
            *round += 1;
        }
        // Each child's version as last known: as read, and unknown once this call wrote it.
        let mut versions: BTreeMap<NativeId, Option<String>> = held
            .iter()
            .map(|(id, item)| (id.clone(), item.version()))
            .collect();
        for round in rounds {
            let mut ops: Vec<(usize, ParentOp)> = Vec::new();
            for at in round {
                let write = &writes[at];
                let Some(child) = held.get(&write.child) else {
                    answers[at] = Some(Ok(ParentAnswer::Missing {
                        what: write.child.clone(),
                    }));
                    continue;
                };
                let version = versions.get(&write.child).cloned().flatten();
                if let Some(expected) = &write.if_version
                    && !version_at(self, version.as_deref(), expected)
                {
                    answers[at] = Some(Ok(ParentAnswer::VersionMoved { found: version }));
                    continue;
                }
                if child.issue_parent == write.parent {
                    answers[at] = Some(Ok(ParentAnswer::Unchanged { version }));
                    continue;
                }
                let op = match (&write.parent, &child.issue_parent) {
                    (Some(parent), current) => ParentOp::Add {
                        parent: parent.clone(),
                        child: write.child.clone(),
                        replace: current.is_some(),
                    },
                    (None, Some(current)) => ParentOp::Remove {
                        parent: current.clone(),
                        child: write.child.clone(),
                    },
                    (None, None) => unreachable!("an unchanged parent was answered above"),
                };
                ops.push((at, op));
            }
            for chunk in ops.chunks(RELATION_BATCH) {
                for (at, answer) in self.send_parents(chunk).await {
                    let write = &writes[at];
                    if matches!(answer, Ok(ParentAnswer::Written { .. }))
                        && let Some(child) = held.get_mut(&write.child)
                    {
                        child.issue_parent.clone_from(&write.parent);
                        versions.insert(write.child.clone(), None);
                    }
                    answers[at] = Some(answer);
                }
                for (_, op) in chunk {
                    let _ = self.forget(op.child());
                }
            }
        }
        answers
            .into_iter()
            .map(|answer| {
                answer.unwrap_or_else(|| {
                    Err(SourceError::Malformed {
                        message: "a parent write was left unanswered".into(),
                    })
                })
            })
            .collect()
    }

    /// One [`graphql::SET_PARENTS`] request of `chunk`, answering each write by its position.
    async fn send_parents(
        &self,
        chunk: &[(usize, ParentOp)],
    ) -> Vec<(usize, Result<ParentAnswer, SourceError>)> {
        let Some((_, first)) = chunk.first() else {
            return Vec::new();
        };
        let filler = match first {
            ParentOp::Add { parent, child, .. } | ParentOp::Remove { parent, child } => {
                json!({"issueId": parent.0, "subIssueId": child.0})
            }
        };
        let mut variables = Map::new();
        for slot in 0..RELATION_BATCH {
            for (prefix, send) in [("parent", "sendParent"), ("unparent", "sendUnparent")] {
                variables.insert(format!("{prefix}{slot}"), filler.clone());
                variables.insert(format!("{send}{slot}"), json!(false));
            }
        }
        let mut aliases = Vec::with_capacity(chunk.len());
        let (mut adds, mut removes) = (0, 0);
        for (at, op) in chunk {
            let alias = match op {
                ParentOp::Add {
                    parent,
                    child,
                    replace,
                } => {
                    let alias = format!("parent{adds}");
                    let mut input = json!({"issueId": parent.0, "subIssueId": child.0});
                    if *replace {
                        input["replaceParent"] = json!(true);
                    }
                    variables.insert(alias.clone(), input);
                    variables.insert(format!("sendParent{adds}"), json!(true));
                    adds += 1;
                    alias
                }
                ParentOp::Remove { parent, child } => {
                    let alias = format!("unparent{removes}");
                    variables.insert(
                        alias.clone(),
                        json!({"issueId": parent.0, "subIssueId": child.0}),
                    );
                    variables.insert(format!("sendUnparent{removes}"), json!(true));
                    removes += 1;
                    alias
                }
            };
            aliases.push((*at, alias, op.child().clone(), op.parent().clone()));
        }
        let doing = "writing several issues' parents";
        let data = match self
            .graphql(graphql::SET_PARENTS, Value::Object(variables))
            .await
        {
            Ok(data) => data,
            Err(error) => {
                return aliases
                    .into_iter()
                    .map(|(at, _, _, _)| (at, Err(error.clone())))
                    .collect();
            }
        };
        aliases
            .into_iter()
            .map(|(at, alias, child, parent)| {
                let answer = match alias_answer(&data, &alias, doing) {
                    Err(error) => Err(error),
                    // Written only when GitHub names both issues the write was about: the child,
                    // and the parent it was added under or removed from.
                    Ok(Some(answered))
                        if answered.pointer("/subIssue/id").and_then(Value::as_str)
                            == Some(child.0.as_str())
                            && answered.pointer("/issue/id").and_then(Value::as_str)
                                == Some(parent.0.as_str()) =>
                    {
                        Ok(ParentAnswer::Written { version: None })
                    }
                    Ok(Some(_)) => Err(SourceError::Malformed {
                        message: format!(
                            "GitHub answered {alias} while {doing} about another issue than \
                             {} under {}",
                            child.0, parent.0
                        ),
                    }),
                    Ok(None) => {
                        let said = errors_at(&data, &alias);
                        Ok(ParentAnswer::Refused {
                            reason: if said.is_empty() {
                                format!("GitHub answered nothing for the parent of {}", child.0)
                            } else {
                                format!(
                                    "GitHub refused the parent of {}: {}",
                                    child.0,
                                    said.join("; ")
                                )
                            },
                        })
                    }
                };
                (at, answer)
            })
            .collect()
    }

    /// Add or remove each native `relates-to` relation, in [`graphql::ADD_RELATES_TO`] or
    /// [`graphql::REMOVE_RELATES_TO`] documents of at most [`RELATION_BATCH`] fields; a field
    /// GitHub refused, or a document whose answer never arrived, read back and answered from
    /// what GitHub holds.
    pub(super) async fn native_links(
        &self,
        links: &[NativeLink],
        write: NativeWrite,
    ) -> Vec<Result<(), SourceError>> {
        let mut answers = Vec::with_capacity(links.len());
        for chunk in links.chunks(RELATION_BATCH) {
            answers.extend(self.send_native_links(chunk, write).await);
        }
        answers
    }

    async fn send_native_links(
        &self,
        chunk: &[NativeLink],
        write: NativeWrite,
    ) -> Vec<Result<(), SourceError>> {
        let Some(first) = chunk.first() else {
            return Vec::new();
        };
        if let Some(link) = chunk.iter().find(|link| link.native.as_str() != RELATES_TO) {
            let refused = SourceError::Refused {
                message: format!(
                    "source {} mirrors a link only as `{RELATES_TO}`, and was asked for `{}`",
                    self.name, link.native
                ),
            };
            return chunk.iter().map(|_| Err(refused.clone())).collect();
        }
        let (document, prefix, doing) = if write == NativeWrite::Add {
            (
                graphql::ADD_RELATES_TO,
                "relate",
                "adding several relates-to relations",
            )
        } else {
            (
                graphql::REMOVE_RELATES_TO,
                "unrelate",
                "removing several relates-to relations",
            )
        };
        let input =
            |link: &NativeLink| json!({"issueId": link.from.0, "relatedIssueId": link.to.0});
        let mut variables = Map::new();
        for slot in 0..RELATION_BATCH {
            let link = chunk.get(slot);
            variables.insert(format!("{prefix}{slot}"), input(link.unwrap_or(first)));
            variables.insert(format!("send{slot}"), json!(link.is_some()));
        }
        let sent = self.graphql(document, Value::Object(variables)).await;
        let mut answers: Vec<Result<(), SourceError>> = match &sent {
            Err(error) => chunk.iter().map(|_| Err(error.clone())).collect(),
            Ok(data) => (0..chunk.len())
                .map(|slot| {
                    let alias = format!("{prefix}{slot}");
                    let link = &chunk[slot];
                    match alias_answer(data, &alias, doing) {
                        // An answer about another pair says nothing of this one.
                        Ok(Some(answered))
                            if answered.pointer("/issue/id").and_then(Value::as_str)
                                == Some(link.from.0.as_str())
                                && answered.pointer("/relatedIssue/id").and_then(Value::as_str)
                                    == Some(link.to.0.as_str()) =>
                        {
                            Ok(())
                        }
                        Ok(Some(_)) => Err(SourceError::Malformed {
                            message: format!(
                                "GitHub answered {alias} while {doing} about another pair than \
                                 {} and {}",
                                link.from.0, link.to.0
                            ),
                        }),
                        Ok(None) => Err(SourceError::Refused {
                            message: format!(
                                "GitHub refused {alias} while {doing}: {}",
                                errors_at(data, &alias).join("; ")
                            ),
                        }),
                        Err(error) => Err(error),
                    }
                })
                .collect(),
        };
        if answers.iter().all(Result::is_ok) {
            return answers;
        }
        // What is still in doubt is answered from what GitHub holds now: a relation that is
        // there was added, and one that is not was removed, whatever the answer said — and only
        // where the read-back establishes it: an issue it could not read whole leaves its links
        // as they were answered.
        let doubted: Vec<usize> = (0..chunk.len())
            .filter(|at| answers[*at].is_err())
            .collect();
        let froms: Vec<NativeId> = doubted.iter().fold(Vec::new(), |mut ids, at| {
            if !ids.contains(&chunk[*at].from) {
                ids.push(chunk[*at].from.clone());
            }
            ids
        });
        let Ok(related) = self.relates_to(&froms).await else {
            return answers;
        };
        for at in doubted {
            let link = &chunk[at];
            match related.get(&link.from) {
                Some(Ok(others)) => {
                    if others.contains(&link.to) == (write == NativeWrite::Add) {
                        answers[at] = Ok(());
                    }
                }
                Some(Err(error)) => answers[at] = Err(error.clone()),
                None => {}
            }
        }
        answers
    }

    /// What each of `ids` relates to, read in [`graphql::RELATES_TO_READS`] documents — an issue
    /// whose connection is missing, malformed or longer than one page answered as an error, never
    /// as relating to nothing.
    async fn relates_to(
        &self,
        ids: &[NativeId],
    ) -> Result<BTreeMap<NativeId, Result<Vec<NativeId>, SourceError>>, SourceError> {
        let mut related = BTreeMap::new();
        for batch in ids.chunks(DETAIL_BATCH) {
            let mut variables = Map::new();
            for slot in 0..DETAIL_BATCH {
                let id = batch.get(slot).or(batch.last()).map(|id| id.0.clone());
                variables.insert(format!("id{slot}"), json!(id));
            }
            variables.insert("first".to_owned(), json!(MAX_PAGE_SIZE));
            // As the relations read: a response carrying any GraphQL error is refused whole, so
            // the read-back answers every link of it as unknown rather than as absent.
            let data = self
                .graphql(graphql::RELATES_TO_READS, Value::Object(variables))
                .await?;
            for (slot, id) in batch.iter().enumerate() {
                related.insert(id.clone(), related_ids(&data, slot, id));
            }
        }
        Ok(related)
    }
}

/// The ids alias `r<slot>` of a [`graphql::RELATES_TO_READS`] answer says `id` relates to, or why
/// that answer establishes nothing.
fn related_ids(data: &Value, slot: usize, id: &NativeId) -> Result<Vec<NativeId>, SourceError> {
    let malformed = |why: &str| SourceError::Malformed {
        message: format!(
            "GitHub's read of what {} relates to {why}, so whether the relation is there is not \
             known; next: run the call again",
            id.0
        ),
    };
    let node = data
        .get(format!("r{slot}"))
        .filter(|node| !node.is_null())
        .ok_or_else(|| malformed("answered no issue"))?;
    if node.get("id").and_then(Value::as_str) != Some(id.0.as_str()) {
        return Err(malformed("answered about another issue"));
    }
    let connection = node
        .get("relatesTo")
        .ok_or_else(|| malformed("has no relatesTo"))?;
    if connection.pointer("/pageInfo/hasNextPage") != Some(&Value::Bool(false)) {
        return Err(malformed(
            "is longer than one page, or says nothing of its end",
        ));
    }
    connection
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("has no list of nodes"))?
        .iter()
        .map(|node| {
            node.get("id")
                .and_then(Value::as_str)
                .map(|other| NativeId(other.to_owned()))
                .ok_or_else(|| malformed("holds a node with no id"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_relation_field_sent_is_one_creation() {
        let mut variables = Map::new();
        for slot in 0..RELATION_BATCH {
            variables.insert(
                format!("relate{slot}"),
                json!({"issueId": "A", "relatedIssueId": "B"}),
            );
            variables.insert(format!("send{slot}"), json!(slot < 30));
        }
        let charge =
            crate::pacing::Charge::graphql(graphql::ADD_RELATES_TO, &Value::Object(variables));
        assert_eq!(charge.creations, 30);
    }

    #[test]
    fn a_read_back_that_cannot_establish_what_an_issue_relates_to_is_an_error_never_absence() {
        let id = NativeId("A".into());
        let whole = json!({"r0": {"id": "A", "relatesTo": {"nodes": [{"id": "B"}],
                                                            "pageInfo": {"hasNextPage": false}}}});
        assert_eq!(related_ids(&whole, 0, &id).unwrap(), [NativeId("B".into())]);
        for (answer, why) in [
            (json!({}), "answered no issue"),
            (json!({"r0": null}), "answered no issue"),
            (
                json!({"r0": {"id": "Z", "relatesTo": {"nodes": [],
                "pageInfo": {"hasNextPage": false}}}}),
                "another issue",
            ),
            (json!({"r0": {"id": "A"}}), "has no relatesTo"),
            (
                json!({"r0": {"id": "A", "relatesTo": {"nodes": [],
                "pageInfo": {"hasNextPage": true}}}}),
                "longer than one page",
            ),
            (
                json!({"r0": {"id": "A", "relatesTo": {"nodes": []}}}),
                "longer than one page",
            ),
            (
                json!({"r0": {"id": "A", "relatesTo": {
                "pageInfo": {"hasNextPage": false}}}}),
                "no list of nodes",
            ),
            (
                json!({"r0": {"id": "A", "relatesTo": {"nodes": [{"name": "B"}],
                "pageInfo": {"hasNextPage": false}}}}),
                "a node with no id",
            ),
        ] {
            let Err(SourceError::Malformed { message }) = related_ids(&answer, 0, &id) else {
                panic!("{answer} was read as an answer");
            };
            assert!(message.contains(why), "{message}");
        }
    }
}
