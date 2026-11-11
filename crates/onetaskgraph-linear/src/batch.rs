//! Many reads and many writes of one kind sent to Linear as one aliased document.
//!
//! [`graphql::aliased`] composes the document this source sends for one item into one that
//! carries every item, each under its own alias. Linear answers each alias as it answers the
//! document alone, so a batch costs one request where the items would each have cost theirs.
//!
//! **Where Linear fails a batch as a whole.** Every root this source batches is non-null, so an
//! alias Linear refuses — an id naming nothing, an input it rejects — answers the whole
//! document with no `data`, the error's `path` naming that alias; Linear executes a mutation
//! document's fields in order, so every write aliased before the refused one was applied and
//! none after it. This source therefore never lets one item's refusal decide another's:
//!
//! - a **read** refused at one alias answers that item absent when the refusal says it names
//!   nothing — and otherwise fails as that item's own — and the rest are read again together;
//! - an **update** refused at one alias is sent again on its own, for its own answer, and the
//!   rest are sent together again: an update sets the values it names, so sending one Linear
//!   already applied again changes nothing;
//! - a **comment** — which is not safe to send twice — is created only on an issue a batched
//!   read just found, which removes the one refusal a caller's input predictably meets; one
//!   refused anyway is answered refused, each comment aliased before it is found again by a
//!   read of its issue's latest comments rather than written twice, and each after it is sent
//!   again together.
//!
//! A source the store's cache stands in front of reads with documents of their own, which this
//! does not batch: it answers each item as the single-item method does.

use onetaskgraph_plugin_api::{
    BatchAnswer, BatchComment, BatchUpdate, Comment, NativeId, Page, PageRequest, SourceError,
    Task, TaskDetailRead, TaskQuery, TaskUpdateOutcome, decided_on,
};
use serde_json::{Value, json};

use super::{
    Composed, LinearSource, MAX_PAGE_SIZE, MutationRoot, Read, Refusal, Under400, comment_page,
    connection, graphql, map_task, optional, optional_string, written_comment,
};

/// One issue as [`LinearSource::issue_held`] reads it: the task and its raw description.
type Held = (Task, Option<String>);

/// What Linear answered an aliased document with.
pub(crate) enum Aliased {
    /// Every alias's root value, in order.
    Answered(Vec<Value>),
    /// The alias at this index was refused, with this refusal; nothing was answered.
    Refused(usize, Refusal),
}

impl LinearSource {
    /// Send `single` once per entry of `each` — its variables — as one aliased document.
    ///
    /// # Errors
    ///
    /// Whatever the request failed with, and a refusal naming no alias, which is Linear failing
    /// the whole request rather than one item of it.
    pub(crate) async fn send_aliased(
        &self,
        single: &str,
        each: &[Value],
    ) -> Result<Aliased, SourceError> {
        self.send_composed_of(&vec![single; each.len()], each).await
    }

    /// Send each of `singles` with its variables in `each` as one [`graphql::composed`]
    /// document, on the terms of [`Self::send_aliased`].
    ///
    /// # Errors
    ///
    /// As [`Self::send_aliased`].
    pub(crate) async fn send_composed_of(
        &self,
        singles: &[&str],
        each: &[Value],
    ) -> Result<Aliased, SourceError> {
        let document = graphql::composed(singles);
        match self
            .answer_with(
                &document,
                graphql::aliased_variables(each),
                Under400::Unavailable,
            )
            .await?
        {
            Ok(data) => Ok(Aliased::Answered(
                (0..each.len())
                    .map(|at| data.get(format!("a{at}")).cloned().unwrap_or(Value::Null))
                    .collect(),
            )),
            Err(refusal) => match refusal.alias() {
                Some(at) if at < each.len() => Ok(Aliased::Refused(at, refusal)),
                _ => Err(refusal.into_error()),
            },
        }
    }

    /// Whether this source batches reads with `single` as it is — not when the store's cache
    /// stands in front of it and reads with documents of its own.
    fn batches(&self, single: &'static str) -> bool {
        self.read_document(single) == single
    }

    /// Each of `ids` read as [`Self::issue_held`] reads it — the issue and its raw description,
    /// `None` for one this source does not hold — in as few aliased reads as Linear answers.
    pub(crate) async fn issues_held(
        &self,
        ids: &[NativeId],
    ) -> Result<Vec<Option<Held>>, SourceError> {
        if ids.len() < 2 || !self.batches(graphql::ISSUE) {
            let mut read = Vec::with_capacity(ids.len());
            for id in ids {
                read.push(self.issue_held(id, Read::Load).await?);
            }
            return Ok(read);
        }
        let mut read: Vec<Option<Option<Held>>> = vec![None; ids.len()];
        let mut pending: Vec<usize> = (0..ids.len()).collect();
        while !pending.is_empty() {
            let each: Vec<Value> = pending.iter().map(|at| json!({"id": ids[*at].0})).collect();
            match self.send_aliased(graphql::ISSUE, &each).await? {
                Aliased::Answered(values) => {
                    for (at, value) in pending.iter().zip(values) {
                        let held = optional(&json!({"issue": value}), "issue", |v| {
                            Ok((
                                map_task(v, &self.name, &self.statuses, self.spelling())?,
                                optional_string(v, "description")?,
                            ))
                        })?
                        .filter(|(task, _)| self.in_scope(task.project.as_ref()));
                        read[*at] = Some(held);
                    }
                    pending.clear();
                }
                Aliased::Refused(refused, refusal) if refusal.entity_missing() => {
                    let at = pending.remove(refused);
                    read[at] = Some(None);
                }
                Aliased::Refused(_, refusal) => return Err(refusal.into_error()),
            }
        }
        Ok(read.into_iter().map(Option::flatten).collect())
    }

    /// [`TaskSource::update_tasks`](onetaskgraph_plugin_api::TaskSource::update_tasks): every
    /// issue read in one aliased read, each decided against its hold and its guard, and every
    /// update that changes something sent in one aliased `issueUpdate` document; see the module
    /// documentation for where an item is sent on its own.
    pub(crate) async fn update_batched(
        &self,
        updates: &[BatchUpdate],
    ) -> Vec<Result<BatchAnswer<TaskUpdateOutcome>, SourceError>> {
        let mut answers: Vec<Option<Result<BatchAnswer<TaskUpdateOutcome>, SourceError>>> =
            updates.iter().map(|_| None).collect();
        // Refused before anything is read, as `update_task` refuses it.
        for (at, each) in updates.iter().enumerate() {
            if let Err(error) = self.update_checked(&each.id, &each.update) {
                answers[at] = Some(Err(error));
            }
        }
        let wanted: Vec<usize> = (0..updates.len())
            .filter(|at| answers[*at].is_none())
            .collect();
        let ids: Vec<NativeId> = wanted.iter().map(|at| updates[*at].id.clone()).collect();
        let read = match self.issues_held(&ids).await {
            Ok(read) => read,
            Err(error) => {
                for at in &wanted {
                    answers[*at] = Some(Err(error.clone()));
                }
                return answers.into_iter().flatten().collect();
            }
        };
        let mut sending: Vec<(usize, Composed)> = Vec::new();
        for (at, held) in wanted.into_iter().zip(read) {
            let each = &updates[at];
            if let Some(answer) = decided_on(
                self,
                held.as_ref().map(|(task, _)| task),
                each.if_version.as_deref(),
                &each.when_priority,
            ) {
                answers[at] = Some(Ok(answer));
                continue;
            }
            let Some((before, description)) = held else {
                answers[at] = Some(Ok(BatchAnswer::Missing));
                continue;
            };
            match self.compose_update(&each.update, before, description).await {
                Ok(composed) if composed.relations.is_some() => {
                    // An update writing relations sends them after its issue's own write, so it
                    // is sent as `update_task` sends it.
                    answers[at] = Some(
                        self.send_composed(&each.id, &each.update, composed)
                            .await
                            .map(|outcome| {
                                outcome.map_or(BatchAnswer::Missing, BatchAnswer::Written)
                            }),
                    );
                }
                Ok(composed) if composed.input.is_empty() => {
                    let written = each.update.changed(&composed.before, &composed.before);
                    answers[at] = Some(Ok(BatchAnswer::Written(TaskUpdateOutcome {
                        delivers_before: composed.before.delivers.clone(),
                        task: composed.before,
                        written,
                    })));
                }
                Ok(composed) => sending.push((at, composed)),
                Err(error) => answers[at] = Some(Err(error)),
            }
        }
        while !sending.is_empty() {
            let each: Vec<Value> = sending
                .iter()
                .map(|(_, composed)| {
                    json!({"id": composed.before.id.0,
                           "input": Value::Object(composed.input.clone())})
                })
                .collect();
            let carries_state = sending
                .iter()
                .any(|(_, composed)| composed.input.contains_key("stateId"));
            let answered = if each.len() == 1 {
                // One update alone is sent as `update_task` sends it.
                self.send(graphql::ISSUE_UPDATE_READ, each[0].clone())
                    .await
                    .map(|data| Aliased::Answered(vec![data["issueUpdate"].clone()]))
            } else {
                self.send_aliased(graphql::ISSUE_UPDATE_READ, &each).await
            };
            match answered {
                Ok(Aliased::Answered(values)) => {
                    for ((at, composed), value) in sending.drain(..).zip(values) {
                        let id = composed.before.id.clone();
                        answers[at] = Some(
                            self.updated_issue(&json!({"issueUpdate": value}), &id)
                                .and_then(|task| {
                                    task.ok_or_else(|| SourceError::Malformed {
                                        message: format!(
                                            "task {id} was updated and then answered as no issue"
                                        ),
                                    })
                                })
                                .map(|task| {
                                    BatchAnswer::Written(TaskUpdateOutcome {
                                        written: updates[at]
                                            .update
                                            .changed(&composed.before, &task),
                                        delivers_before: composed.before.delivers.clone(),
                                        task,
                                    })
                                }),
                        );
                    }
                }
                Ok(Aliased::Refused(refused, _)) => {
                    // That one alone, for its own answer; the rest together again.
                    let (at, composed) = sending.remove(refused);
                    let id = updates[at].id.clone();
                    answers[at] = Some(
                        self.send_composed(&id, &updates[at].update, composed)
                            .await
                            .map(|outcome| {
                                outcome.map_or(BatchAnswer::Missing, BatchAnswer::Written)
                            }),
                    );
                }
                Err(error) => {
                    if carries_state {
                        self.forget_vocabulary();
                    }
                    for (at, _) in sending.drain(..) {
                        answers[at] = Some(Err(error.clone()));
                    }
                }
            }
        }
        answers
            .into_iter()
            .map(|answer| {
                answer.unwrap_or_else(|| {
                    Err(SourceError::Malformed {
                        message: "an update of the batch was given no answer".to_owned(),
                    })
                })
            })
            .collect()
    }

    /// [`TaskSource::add_comments`](onetaskgraph_plugin_api::TaskSource::add_comments): every
    /// issue read in one aliased read — which finds each, decides each hold, and is what keeps a
    /// comment off an issue Linear would refuse it on — and every comment then created in one
    /// aliased `commentCreate` document; see the module documentation.
    pub(crate) async fn comment_batched(
        &self,
        comments: &[BatchComment],
    ) -> Vec<Result<BatchAnswer<Comment>, SourceError>> {
        let mut answers: Vec<Option<Result<BatchAnswer<Comment>, SourceError>>> =
            comments.iter().map(|_| None).collect();
        for (at, each) in comments.iter().enumerate() {
            if let Some(author) = &each.comment.author {
                answers[at] = Some(Err(SourceError::Refused {
                    message: format!(
                        "source {} cannot post a comment as {author:?}, because Linear records \
                         the user whose API key makes the request as the author of every \
                         comment; leave the author out to post as that user",
                        self.name
                    ),
                }));
            }
        }
        let wanted: Vec<usize> = (0..comments.len())
            .filter(|at| answers[*at].is_none())
            .collect();
        let ids: Vec<NativeId> = wanted.iter().map(|at| comments[*at].task.clone()).collect();
        let read = match self.issues_held(&ids).await {
            Ok(read) => read,
            Err(error) => {
                for at in &wanted {
                    answers[*at] = Some(Err(error.clone()));
                }
                return answers.into_iter().flatten().collect();
            }
        };
        let mut sending: Vec<(usize, NativeId)> = Vec::new();
        for (at, held) in wanted.into_iter().zip(read) {
            let each = &comments[at];
            if let Some(answer) = decided_on(
                self,
                held.as_ref().map(|(task, _)| task),
                each.if_version.as_deref(),
                &[],
            ) {
                answers[at] = Some(Ok(answer));
                continue;
            }
            if let Some((task, _)) = held {
                sending.push((at, task.id));
            }
        }
        while !sending.is_empty() {
            let each: Vec<Value> = sending
                .iter()
                .map(|(at, issue)| {
                    json!({"input": {"issueId": issue.0,
                                     "body": comments[*at].comment.body.as_str()}})
                })
                .collect();
            let answered = if each.len() == 1 {
                self.send(graphql::COMMENT_CREATE, each[0].clone())
                    .await
                    .map(|data| Aliased::Answered(vec![data["commentCreate"].clone()]))
            } else {
                self.send_aliased(graphql::COMMENT_CREATE, &each).await
            };
            match answered {
                Ok(Aliased::Answered(values)) => {
                    for ((at, _), value) in sending.drain(..).zip(values) {
                        answers[at] = Some(
                            written_comment(
                                &json!({"commentCreate": value}),
                                MutationRoot::CommentCreate,
                            )
                            .map(BatchAnswer::Written),
                        );
                    }
                }
                Ok(Aliased::Refused(refused, refusal)) => {
                    // Every comment aliased before the refused one landed, unanswered: each is
                    // found by a read of its issue rather than written a second time.
                    let landed: Vec<(usize, NativeId)> = sending.drain(..refused).collect();
                    let (at, _) = sending.remove(0);
                    answers[at] = Some(Err(refusal.into_error()));
                    for (at, issue) in landed {
                        answers[at] = Some(
                            self.landed_comment(&issue, comments[at].comment.body.as_str())
                                .await
                                .map(BatchAnswer::Written),
                        );
                    }
                }
                Err(error) => {
                    for (at, _) in sending.drain(..) {
                        answers[at] = Some(Err(error.clone()));
                    }
                }
            }
        }
        answers
            .into_iter()
            .map(|answer| {
                answer.unwrap_or_else(|| {
                    Err(SourceError::Malformed {
                        message: "a comment of the batch was given no answer".to_owned(),
                    })
                })
            })
            .collect()
    }

    /// The newest comment on `issue` saying exactly `body` — one a batch created and whose
    /// answer Linear withheld when it refused a later alias.
    async fn landed_comment(&self, issue: &NativeId, body: &str) -> Result<Comment, SourceError> {
        let page = PageRequest {
            cursor: None,
            limit: MAX_PAGE_SIZE,
        };
        <Self as onetaskgraph_plugin_api::TaskSource>::task_comments(self, issue, &page)
            .await?
            .and_then(|page| {
                page.items
                    .into_iter()
                    .rev()
                    .find(|comment| comment.body == body)
            })
            .ok_or_else(|| SourceError::Unavailable {
                message: format!(
                    "a comment on {issue} was sent in a batch Linear refused part of, and a read \
                     of the issue finds it nowhere; read its comments before adding it again"
                ),
            })
    }

    /// [`TaskSource::get_task_details`](onetaskgraph_plugin_api::TaskSource::get_task_details)
    /// with a page of comments: every issue and its page in one aliased read.
    pub(crate) async fn details_batched(
        &self,
        ids: &[NativeId],
        page: &PageRequest,
    ) -> Option<Vec<Result<Option<TaskDetailRead>, SourceError>>> {
        if ids.len() < 2 || !self.batches(graphql::ISSUE_DETAIL) || page.limit == 0 {
            return None;
        }
        let mut read: Vec<Option<Result<Option<TaskDetailRead>, SourceError>>> =
            ids.iter().map(|_| None).collect();
        let mut pending: Vec<usize> = (0..ids.len()).collect();
        while !pending.is_empty() {
            let each: Vec<Value> = pending
                .iter()
                .map(|at| {
                    json!({"id": ids[*at].0, "last": page.limit.min(MAX_PAGE_SIZE),
                           "before": page.cursor.as_ref().map(|cursor| &cursor.0)})
                })
                .collect();
            match self.send_aliased(graphql::ISSUE_DETAIL, &each).await {
                Ok(Aliased::Answered(values)) => {
                    for (at, value) in pending.drain(..).zip(values) {
                        read[at] = Some(self.detail_of(&value, page));
                    }
                }
                Ok(Aliased::Refused(refused, refusal)) => {
                    let at = pending.remove(refused);
                    read[at] = Some(if refusal.entity_missing() {
                        Ok(None)
                    } else {
                        Err(refusal.into_error())
                    });
                }
                Err(error) => {
                    for at in pending.drain(..) {
                        read[at] = Some(Err(error.clone()));
                    }
                }
            }
        }
        Some(read.into_iter().flatten().collect())
    }

    /// One issue and its page of comments as an aliased detail read answered it.
    fn detail_of(
        &self,
        value: &Value,
        page: &PageRequest,
    ) -> Result<Option<TaskDetailRead>, SourceError> {
        Ok(optional(&json!({"issue": value}), "issue", |v| {
            let mut task = map_task(v, &self.name, &self.statuses, self.spelling())?;
            if self.attached() && page.cursor.is_some() {
                task.version = None;
            }
            Ok((task, comment_page(v)?))
        })?
        .filter(|(task, _)| self.in_scope(task.project.as_ref()))
        .map(|(task, comments)| TaskDetailRead {
            task,
            comments: Some(Ok(Some(comments))),
        }))
    }

    /// [`TaskSource::read_ahead_records_for_copy`](onetaskgraph_plugin_api::TaskSource::read_ahead_records_for_copy):
    /// every task and project the copy names read beside the first page of its forward
    /// relations, and every document, in one composed read, each held for the next read of it.
    /// An item Linear says is not there is left to its own read, which answers it absent, and
    /// the rest are read together again.
    pub(crate) async fn read_ahead_batched(
        &self,
        records: &[(onetaskgraph_plugin_api::MetadataRecord, NativeId)],
    ) -> Result<(), SourceError> {
        use onetaskgraph_plugin_api::MetadataRecord;
        if records.len() < 2
            || !self.batches(graphql::ISSUE)
            || !self.batches(graphql::PROJECT)
            || !self.batches(graphql::ISSUE_DEPENDS_ON)
        {
            return Ok(());
        }
        let mut pending: Vec<&(MetadataRecord, NativeId)> = records.iter().collect();
        while !pending.is_empty() {
            let mut singles = Vec::new();
            let mut each = Vec::new();
            let mut owners = Vec::new();
            for (at, (record, id)) in pending.iter().enumerate() {
                let reads: &[&'static str] = match record {
                    MetadataRecord::Task => &[graphql::ISSUE, graphql::ISSUE_DEPENDS_ON],
                    MetadataRecord::Project => &[graphql::PROJECT, graphql::PROJECT_DEPENDS_ON],
                    MetadataRecord::Document => &[graphql::DOCUMENT],
                };
                for (part, single) in reads.iter().enumerate() {
                    singles.push(*single);
                    each.push(if part == 0 {
                        json!({"id": id.0})
                    } else {
                        json!({"id": id.0, "first": MAX_PAGE_SIZE, "after": null})
                    });
                    owners.push((at, part));
                }
            }
            match self.send_composed_of(&singles, &each).await? {
                Aliased::Answered(values) => {
                    let mut ahead = self
                        .ahead
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut edges = self
                        .edges_ahead
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    for ((at, part), value) in owners.into_iter().zip(values) {
                        let (record, id) = pending[at];
                        let key = (*record, id.clone());
                        if part == 0 {
                            ahead.insert(key, value);
                        } else {
                            edges.insert(key, value);
                        }
                    }
                    pending.clear();
                }
                Aliased::Refused(refused, refusal)
                    if refusal.entity_missing()
                        || refusal.project_missing()
                        || refusal.document_missing() =>
                {
                    let (at, _) = owners[refused];
                    pending.remove(at);
                }
                Aliased::Refused(_, refusal) => return Err(refusal.into_error()),
            }
        }
        Ok(())
    }

    /// Create every relation held back for `finish_writes` in one aliased `issueRelationCreate`
    /// document. One Linear refuses fails the whole write — the copy then takes back what it
    /// made — since a relation create is not safe to send twice.
    pub(crate) async fn finish_relations(&self) -> Result<(), SourceError> {
        let held: Vec<(NativeId, Value)> = std::mem::take(
            &mut *self
                .deferred_relations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if held.is_empty() {
            return Ok(());
        }
        let each: Vec<Value> = held
            .iter()
            .map(|(_, variables)| variables.clone())
            .collect();
        let values = if each.len() == 1 {
            vec![
                self.send(graphql::ISSUE_RELATION_CREATE, each[0].clone())
                    .await?["issueRelationCreate"]
                    .clone(),
            ]
        } else {
            match self
                .send_aliased(graphql::ISSUE_RELATION_CREATE, &each)
                .await?
            {
                Aliased::Answered(values) => values,
                Aliased::Refused(_, refusal) => return Err(refusal.into_error()),
            }
        };
        for ((near, _), value) in held.iter().zip(values) {
            self.unknown_version(near);
            let answered = json!({"issueRelationCreate": value});
            let payload = super::mutation_payload(&answered, MutationRoot::IssueRelationCreate)?;
            let relation = payload
                .get("issueRelation")
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing issueRelationCreate relation".to_owned(),
                })?;
            super::backend_id(relation, "id")?;
        }
        Ok(())
    }

    /// [`TaskSource::query_tasks_batch`](onetaskgraph_plugin_api::TaskSource::query_tasks_batch):
    /// every query that narrows by no origin as one aliased `issues` read per round — a query by
    /// origin reads as `query_tasks` reads it, beside the copy's own lookups — each query's page
    /// confirmed as `query_tasks` confirms it, and a query whose page Linear refuses answered
    /// that refusal alone.
    pub(crate) async fn query_batched(
        &self,
        asks: &[(TaskQuery, PageRequest)],
    ) -> Vec<Result<Page<Task>, SourceError>> {
        let mut answers: Vec<Option<Result<Page<Task>, SourceError>>> =
            asks.iter().map(|_| None).collect();
        let mut pending = Vec::new();
        for (at, (query, page)) in asks.iter().enumerate() {
            if query.origin.is_some() || !self.batches(graphql::ISSUES) {
                answers[at] = Some(
                    <Self as onetaskgraph_plugin_api::TaskSource>::query_tasks(self, query, page)
                        .await,
                );
                continue;
            }
            match self.issue_filter(query) {
                Ok(filter) => pending.push((at, filter)),
                Err(error) => answers[at] = Some(Err(error)),
            }
        }
        while !pending.is_empty() {
            let each: Vec<Value> = pending
                .iter()
                .map(|(at, filter)| {
                    let page = &asks[*at].1;
                    json!({"first": page.limit.min(MAX_PAGE_SIZE),
                           "after": page.cursor.as_ref().map(|cursor| &cursor.0),
                           "filter": filter})
                })
                .collect();
            let answered = if each.len() == 1 {
                self.send(graphql::ISSUES, each[0].clone())
                    .await
                    .map(|data| Aliased::Answered(vec![data["issues"].clone()]))
            } else {
                self.send_aliased(graphql::ISSUES, &each).await
            };
            match answered {
                Ok(Aliased::Answered(values)) => {
                    for ((at, _), value) in pending.drain(..).zip(values) {
                        let query = &asks[at].0;
                        answers[at] = Some(
                            connection(&json!({"issues": value}), "issues", |v| {
                                map_task(v, &self.name, &self.statuses, self.spelling())
                            })
                            .map(|listed| Page {
                                items: listed
                                    .items
                                    .into_iter()
                                    .filter(|task| Self::confirms(query, task))
                                    .collect(),
                                next: listed.next,
                            }),
                        );
                    }
                }
                Ok(Aliased::Refused(refused, refusal)) => {
                    let (at, _) = pending.remove(refused);
                    answers[at] = Some(Err(refusal.into_error()));
                }
                Err(error) => {
                    for (at, _) in pending.drain(..) {
                        answers[at] = Some(Err(error.clone()));
                    }
                }
            }
        }
        answers
            .into_iter()
            .map(|answer| {
                answer.unwrap_or_else(|| {
                    Err(SourceError::Malformed {
                        message: "a query of the batch was given no answer".to_owned(),
                    })
                })
            })
            .collect()
    }
}
