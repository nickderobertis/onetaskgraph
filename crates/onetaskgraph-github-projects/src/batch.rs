//! The batched calls of [`TaskSource`] — many searches, many comments, many updates — answered
//! in as few requests as GitHub's GraphQL API allows: aliased fields of one document where the
//! single-item call sends one document each.
//!
//! Every item stays its own: one alias GitHub refuses fails that item's answer alone, and the
//! others are answered beside it, because GitHub runs each aliased field on its own and answers
//! a refusal at the refused field's path. What GitHub refuses as a whole — a batched node read
//! naming an id that resolves to nothing — is read again one item at a time, as
//! [`GitHubProjectsSource::items_by_ids`] already reads it.
//!
//! [`TaskSource`]: onetaskgraph_plugin_api::TaskSource

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    BatchAnswer, BatchComment, BatchUpdate, Comment, Cursor, ItemKind, NativeId, Page, PageRequest,
    ProjectFilter, SourceError, Task, TaskQuery, TaskSource, TaskUpdateOutcome, decided_on,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    ANSWER_ERRORS, BOARD_ITEMS_PAGE_SIZE, BoardKind, COMMENT_BATCH, ContentKind,
    GitHubProjectsSource, MAX_PAGE_SIZE, MUTATION_BATCH, NESTED_PAGE_SIZE, ORIGIN_FIELD, Resolved,
    SEARCH_BATCH, SEARCH_BATCH_FIRST_PAGE, SEARCH_BATCH_ORIGINS, SEARCH_CURSOR_VERSION,
    SEARCH_PAGE_SIZE, SearchConnection, SearchPosition, TASK_FIELD_BATCH, as_stored, comment_from,
    graphql, optional_nodes, quoted, refuse_unsearchable, required_bool, required_str,
    search_qualifiers, task_matches, validate_cursor_progress, validate_page,
};

/// The board field writes and issue updates a batch of targeted updates holds back while it
/// runs, each against the task it was made for, so they go out together once every update has
/// been composed: every field write first, then every issue update of a task whose field writes
/// all landed. Held only for the length of one [`GitHubProjectsSource::updates_batched`].
#[derive(Default)]
pub(super) struct TaskWrites {
    /// The task whose update is being composed.
    current: Option<NativeId>,
    /// Each task's board field writes: the board, the board item, each `(field, value)`
    /// written and each field cleared.
    fields: Vec<(NativeId, HeldFields)>,
    /// Each task's issue update, its `UpdateIssueInput` members other than `id`.
    issues: Vec<(NativeId, Value)>,
}

/// One task's board field writes on one board, held by [`TaskWrites`].
pub(super) struct HeldFields {
    board: String,
    item: String,
    writes: Vec<(String, Value)>,
    clears: Vec<String>,
}

/// Where a batched origin lookup stands between two pages of its answer: each of its two
/// connections, and the candidates its earlier pages already answered.
///
/// Its own shape, which [`SearchPosition`] cannot be read as, so a cursor of one never resumes
/// the other.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginPosition {
    version: u32,
    origin_items: SearchConnection,
    origin_search: SearchConnection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    answered: Vec<NativeId>,
}

/// One search of a batch, as one slot of [`graphql::SEARCH_BATCH`] asks it.
struct Slotted {
    /// The search, scoped to this board.
    search: String,
    /// Where its page resumes; `None` for its first page.
    after: Option<String>,
    /// The rows its page asks for.
    first: u32,
}

/// One task query of a batch that [`graphql::SEARCH_BATCH`] answers.
enum Batched {
    /// A narrowed issue search.
    Search(Slotted),
    /// An origin lookup: the board's field filter, and the issue search for the origin's
    /// mirror in the body — each `None` once walked to its end.
    Origin {
        filter: String,
        items: Option<Slotted>,
        search: Option<Slotted>,
        answered: Vec<NativeId>,
    },
}

impl Batched {
    fn search_slots(&self) -> usize {
        match self {
            Self::Search(_) => 1,
            Self::Origin { search, .. } => usize::from(search.is_some()),
        }
    }
    fn origin_slots(&self) -> usize {
        match self {
            Self::Search(_) => 0,
            Self::Origin { items, .. } => usize::from(items.is_some()),
        }
    }
}

/// The aliases one batched ask was given in a request, and the ask.
struct Placed<'a> {
    at: usize,
    query: &'a TaskQuery,
    batched: Batched,
    search_slot: Option<usize>,
    origin_slot: Option<usize>,
}

/// The messages of every GraphQL error a response carried beside its data whose path begins at
/// `alias`, or every error naming no alias at all — what refused the field `alias`.
fn errors_at(data: &Value, alias: &str) -> Vec<String> {
    data.get(ANSWER_ERRORS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|error| {
            error
                .get("path")
                .and_then(Value::as_array)
                .and_then(|path| path.first())
                .and_then(Value::as_str)
                .is_none_or(|first| first == alias)
        })
        .map(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("GitHub returned a GraphQL error")
                .to_owned()
        })
        .collect()
}

/// The refusal of the aliased field `alias` of a request `doing` something, in GitHub's words
/// when it gave any.
fn refused_alias(data: &Value, alias: &str, doing: &str) -> SourceError {
    let said = errors_at(data, alias);
    SourceError::Refused {
        message: if said.is_empty() {
            format!("GitHub answered nothing for {alias} while {doing}")
        } else {
            format!("GitHub refused {alias} while {doing}: {}", said.join("; "))
        },
    }
}

impl GitHubProjectsSource {
    /// Whether this process has written anything of this board, which a narrowed read
    /// completes its answer with — see `with_own_writes` — and a batched one cannot.
    fn wrote_any(&self) -> Result<bool, SourceError> {
        Ok(!self.created()?.is_empty() || !self.updated()?.is_empty())
    }

    /// How [`graphql::SEARCH_BATCH`] would answer `query` at `page`, or `None` for a query its
    /// own read answers — one validated or refused there, narrowed by a project or by comment
    /// activity, carrying no narrowing, or resumed from a cursor that read handed out.
    fn batched(query: &TaskQuery, page: &PageRequest, scope: &str) -> Option<Batched> {
        if validate_page(page).is_err()
            || refuse_unsearchable(query).is_err()
            || query.commented_since.is_some()
            || matches!(query.project, ProjectFilter::Is(_))
        {
            return None;
        }
        let limit = page.limit.min(MAX_PAGE_SIZE);
        let first = |after: &Option<String>| match after {
            None => SEARCH_BATCH_FIRST_PAGE.min(limit),
            Some(_) => SEARCH_PAGE_SIZE.min(limit),
        };
        let resumed = |connection: &SearchConnection| match connection {
            SearchConnection::Initial {} => Some(None),
            SearchConnection::Continuing { after } => Some(Some(after.0.clone())),
            SearchConnection::Exhausted {} => None,
        };
        if let Some(origin) = &query.origin {
            let (items, search, answered) = match &page.cursor {
                None => (Some(None), Some(None), Vec::new()),
                Some(cursor) => {
                    let position = serde_json::from_str::<OriginPosition>(&cursor.0)
                        .ok()
                        .filter(|position| position.version == SEARCH_CURSOR_VERSION)?;
                    (
                        resumed(&position.origin_items),
                        resumed(&position.origin_search),
                        position.answered,
                    )
                }
            };
            let search_for = format!("{scope} in:body {}", quoted(&as_stored(origin)));
            return Some(Batched::Origin {
                filter: format!("{ORIGIN_FIELD}:{}", quoted(origin)),
                items: items.map(|after| Slotted {
                    search: String::new(),
                    first: first(&after),
                    after,
                }),
                search: search.map(|after| Slotted {
                    search: search_for,
                    first: first(&after),
                    after,
                }),
                answered,
            });
        }
        let also = search_qualifiers(query)?;
        let after = match &page.cursor {
            None => None,
            Some(cursor) => {
                let position = serde_json::from_str::<SearchPosition>(&cursor.0).ok()?;
                match position.connection {
                    SearchConnection::Continuing { after }
                        if position.version == SEARCH_CURSOR_VERSION
                            && position.offset == 0
                            && position.seen.is_empty()
                            && position.own.is_empty()
                            && !after.0.is_empty() =>
                    {
                        Some(after.0)
                    }
                    _ => return None,
                }
            }
        };
        Some(Batched::Search(Slotted {
            search: format!("{scope} {also}"),
            first: first(&after),
            after,
        }))
    }

    /// [`TaskSource::query_tasks_batch`](onetaskgraph_plugin_api::TaskSource::query_tasks_batch):
    /// every narrowed search and origin lookup a page at a time in one
    /// [`graphql::SEARCH_BATCH`] request a round, and every other query by its own read.
    pub(super) async fn search_batched(
        &self,
        asks: &[(TaskQuery, PageRequest)],
    ) -> Vec<Result<Page<Task>, SourceError>> {
        let mut answers: Vec<Option<Result<Page<Task>, SourceError>>> =
            asks.iter().map(|_| None).collect();
        let scope = self.board_search(None);
        // What this process wrote completes a narrowed read's answer, which only that read's
        // own walk does; a batch is asked of a board this process has not written.
        let batchable = match self.wrote_any() {
            Ok(wrote) => !wrote,
            Err(error) => return asks.iter().map(|_| Err(error.clone())).collect(),
        };
        let mut batched: Vec<(usize, Batched)> = if batchable {
            asks.iter()
                .enumerate()
                .filter_map(|(at, (query, page))| {
                    Self::batched(query, page, &scope).map(|batched| (at, batched))
                })
                .collect()
        } else {
            Vec::new()
        };
        // One search alone is its own read, which asks a page of its own size at one point; an
        // origin resumed from a batch's cursor is answered only by another batch.
        let resumed_origin = batched.iter().any(|(at, batched)| {
            matches!(batched, Batched::Origin { .. }) && asks[*at].1.cursor.is_some()
        });
        if batched.len() < 2 && !resumed_origin {
            batched.clear();
        }
        while !batched.is_empty() {
            let mut placed: Vec<Placed<'_>> = Vec::new();
            let (mut searches, mut origins) = (0, 0);
            let mut rest = Vec::new();
            for (at, ask) in batched {
                if searches + ask.search_slots() <= SEARCH_BATCH
                    && origins + ask.origin_slots() <= SEARCH_BATCH_ORIGINS
                {
                    let search_slot = (ask.search_slots() == 1).then_some(searches);
                    let origin_slot = (ask.origin_slots() == 1).then_some(origins);
                    searches += ask.search_slots();
                    origins += ask.origin_slots();
                    placed.push(Placed {
                        at,
                        query: &asks[at].0,
                        batched: ask,
                        search_slot,
                        origin_slot,
                    });
                } else {
                    rest.push((at, ask));
                }
            }
            batched = rest;
            for (at, answer) in self.search_round(placed, &scope).await {
                answers[at] = Some(answer);
            }
        }
        let mut out = Vec::with_capacity(asks.len());
        for ((query, page), answer) in asks.iter().zip(answers) {
            out.push(match answer {
                Some(answer) => answer,
                None => TaskSource::query_tasks(self, query, page).await,
            });
        }
        out
    }

    /// One [`graphql::SEARCH_BATCH`] request answering every ask `placed` holds, each with its
    /// page or the refusal of its own slots.
    async fn search_round(
        &self,
        placed: Vec<Placed<'_>>,
        scope: &str,
    ) -> Vec<(usize, Result<Page<Task>, SourceError>)> {
        let mut variables = serde_json::Map::new();
        for (name, value) in [
            ("owner", json!(self.owner)),
            ("number", json!(self.project_number)),
            ("type", json!("ISSUE")),
            ("nestedFirst", json!(NESTED_PAGE_SIZE)),
            ("boardItems", json!(BOARD_ITEMS_PAGE_SIZE)),
            ("duplicates", json!(true)),
        ] {
            variables.insert(name.to_owned(), value);
        }
        // A slot no ask holds is left out by its `@include`, bound to a well-formed search of
        // one row, which GitHub requires of a variable it never runs.
        for slot in 0..SEARCH_BATCH {
            variables.insert(format!("search{slot}"), json!(scope));
            variables.insert(format!("after{slot}"), Value::Null);
            variables.insert(format!("first{slot}"), json!(1));
            variables.insert(format!("ask{slot}"), json!(false));
        }
        for slot in 0..SEARCH_BATCH_ORIGINS {
            variables.insert(format!("filter{slot}"), json!(""));
            variables.insert(format!("itemsAfter{slot}"), Value::Null);
            variables.insert(format!("itemsFirst{slot}"), json!(1));
            variables.insert(format!("origin{slot}"), json!(false));
        }
        for ask in &placed {
            let (search, items, filter) = match &ask.batched {
                Batched::Search(slotted) => (Some(slotted), None, None),
                Batched::Origin {
                    filter,
                    items,
                    search,
                    ..
                } => (search.as_ref(), items.as_ref(), Some(filter)),
            };
            if let (Some(slot), Some(search)) = (ask.search_slot, search) {
                variables.insert(format!("search{slot}"), json!(search.search));
                variables.insert(format!("after{slot}"), json!(search.after));
                variables.insert(format!("first{slot}"), json!(search.first));
                variables.insert(format!("ask{slot}"), json!(true));
            }
            if let (Some(slot), Some(items), Some(filter)) = (ask.origin_slot, items, filter) {
                variables.insert(format!("filter{slot}"), json!(filter));
                variables.insert(format!("itemsAfter{slot}"), json!(items.after));
                variables.insert(format!("itemsFirst{slot}"), json!(items.first));
                variables.insert(format!("origin{slot}"), json!(true));
            }
        }
        let data = match self
            .graphql(graphql::SEARCH_BATCH, Value::Object(variables))
            .await
        {
            Ok(data) => data,
            Err(error) => {
                return placed
                    .into_iter()
                    .map(|ask| (ask.at, Err(error.clone())))
                    .collect();
            }
        };
        let mut answered = Vec::with_capacity(placed.len());
        for ask in placed {
            let at = ask.at;
            answered.push((at, self.answer_slots(&data, ask).await));
        }
        answered
    }

    /// The page one ask's slots of a [`graphql::SEARCH_BATCH`] answer hold, or the refusal of
    /// one of them.
    async fn answer_slots(&self, data: &Value, ask: Placed<'_>) -> Result<Page<Task>, SourceError> {
        let doing = "searching this board's issues several ways at once";
        let connection = |alias: &str, pointer: &str| -> Result<Value, SourceError> {
            match data.get(alias).filter(|value| !value.is_null()) {
                Some(value) => value
                    .pointer(pointer)
                    .filter(|value| !value.is_null())
                    .cloned()
                    .ok_or_else(|| SourceError::Refused {
                        message: format!(
                            "GitHub project {}/{} was not found or is not visible to the token",
                            self.owner, self.project_number
                        ),
                    }),
                None => Err(refused_alias(data, alias, doing)),
            }
        };
        let next_of =
            |connection: &Value, after: Option<&str>| -> Result<Option<String>, SourceError> {
                let info = connection
                    .get("pageInfo")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub search connection has no pageInfo".into(),
                    })?;
                let next = required_bool(info, "hasNextPage")?
                    .then(|| required_str(info, "endCursor"))
                    .transpose()?
                    .map(str::to_owned);
                if let Some(next) = &next {
                    validate_cursor_progress(after, next)?;
                }
                Ok(next)
            };
        let mut found: Vec<Resolved> = Vec::new();
        let keep = |resolved: Resolved, found: &mut Vec<Resolved>| {
            if !found.iter().any(|held| held.id == resolved.id) {
                found.push(resolved);
            }
        };
        let page = match ask.batched {
            Batched::Search(slotted) => {
                let alias = format!("s{}", ask.search_slot.unwrap_or_default());
                let searched = connection(&alias, "")?;
                for node in optional_nodes(Some(&searched), "search")?
                    .into_iter()
                    .flatten()
                {
                    if let Some(resolved) = self.resolve_issue(node).await? {
                        keep(resolved, &mut found);
                    }
                }
                let next = next_of(&searched, slotted.after.as_deref())?;
                let tasks = self.matching(&found, ask.query)?;
                Page {
                    items: tasks,
                    next: next.map(|after| {
                        let position = SearchPosition {
                            connection: SearchConnection::Continuing {
                                after: Cursor(after),
                            },
                            ..SearchPosition::default()
                        };
                        Cursor(
                            serde_json::to_string(&position)
                                .expect("search position is serializable"),
                        )
                    }),
                }
            }
            Batched::Origin {
                items,
                search,
                mut answered,
                ..
            } => {
                let mut items_next = SearchConnection::Exhausted {};
                let mut search_next = SearchConnection::Exhausted {};
                if let (Some(slot), Some(items)) = (ask.origin_slot, &items) {
                    let read = connection(&format!("o{slot}"), "/projectV2/items")?;
                    for item in optional_nodes(Some(&read), "project items")?
                        .into_iter()
                        .flatten()
                    {
                        // The board's own items list its drafts too, and a draft is no issue:
                        // no narrowed read answers with one, whatever its origin field holds.
                        if let Some(resolved) = self.resolve(item)?
                            && resolved.content_kind == ContentKind::Issue
                        {
                            keep(resolved, &mut found);
                        }
                    }
                    if let Some(next) = next_of(&read, items.after.as_deref())? {
                        items_next = SearchConnection::Continuing {
                            after: Cursor(next),
                        };
                    }
                }
                if let (Some(slot), Some(search)) = (ask.search_slot, &search) {
                    let read = connection(&format!("s{slot}"), "")?;
                    for node in optional_nodes(Some(&read), "search")?.into_iter().flatten() {
                        if let Some(resolved) = self.resolve_issue(node).await? {
                            keep(resolved, &mut found);
                        }
                    }
                    if let Some(next) = next_of(&read, search.after.as_deref())? {
                        search_next = SearchConnection::Continuing {
                            after: Cursor(next),
                        };
                    }
                }
                // A candidate an earlier page answered is not answered again.
                found.retain(|item| !answered.contains(&item.id));
                answered.extend(found.iter().map(|item| item.id.clone()));
                let tasks = self.matching(&found, ask.query)?;
                let more = !items_next.exhausted() || !search_next.exhausted();
                Page {
                    items: tasks,
                    next: more.then(|| {
                        let position = OriginPosition {
                            version: SEARCH_CURSOR_VERSION,
                            origin_items: items_next,
                            origin_search: search_next,
                            answered,
                        };
                        Cursor(
                            serde_json::to_string(&position)
                                .expect("origin position is serializable"),
                        )
                    }),
                }
            }
        };
        Ok(page)
    }

    /// Every task among `found` that `query` keeps, in order — confirmed exactly as a read of
    /// one confirms its candidates.
    fn matching(&self, found: &[Resolved], query: &TaskQuery) -> Result<Vec<Task>, SourceError> {
        let mut tasks = Vec::new();
        for item in found
            .iter()
            .filter(|item| item.kind == BoardKind::Work(ItemKind::Task))
        {
            let task = item.task()?;
            if task_matches(&task, query, &query.project) {
                tasks.push(task);
            }
        }
        Ok(tasks)
    }

    /// Read the first page of each candidate's comments that a comment-activity read is about
    /// to confirm, together — [`DETAIL_BATCH`](super::DETAIL_BATCH) to a
    /// [`graphql::ISSUE_DETAILS`] request — and hold each for the confirmation and for the one
    /// comment listing of that task that follows in this command.
    ///
    /// So confirming a page of candidates and then listing their comments costs one request
    /// per batch rather than two per candidate. A candidate whose read fails is left to its own
    /// read, as it always was.
    pub(super) async fn hold_comment_pages(
        &self,
        candidates: &[NativeId],
    ) -> Result<(), SourceError> {
        let unheld: Vec<NativeId> = {
            let held = self.held_comments()?;
            candidates
                .iter()
                .filter(|id| !held.contains_key(id))
                .cloned()
                .collect()
        };
        if unheld.len() < 2 {
            return Ok(());
        }
        let page = PageRequest {
            cursor: None,
            limit: MAX_PAGE_SIZE,
        };
        let read = self.issue_details(&unheld, Some(&page)).await;
        let mut held = self.held_comments()?;
        for (id, read) in unheld.into_iter().zip(read) {
            if let Ok(Some(onetaskgraph_plugin_api::TaskDetailRead {
                comments: Some(Ok(Some(first))),
                ..
            })) = read
            {
                held.insert(id, first);
            }
        }
        Ok(())
    }

    /// The tasks among `items` whose comments a read of `query`'s comment activity confirms
    /// it by: each task issue it otherwise keeps that changed at or after the instant, or that
    /// this process commented on — none, for a query not narrowed by comment activity.
    pub(super) fn comment_candidates<'a>(
        &self,
        items: impl Iterator<Item = &'a Resolved>,
        query: &TaskQuery,
        project: &ProjectFilter,
    ) -> Result<Vec<NativeId>, SourceError> {
        let Some(since) = query.commented_since else {
            return Ok(Vec::new());
        };
        let commented = self.commented()?.clone();
        let mut candidates = Vec::new();
        for item in items {
            if item.kind != BoardKind::Work(ItemKind::Task)
                || item.content_kind != ContentKind::Issue
                || (item.updated_at.is_some_and(|updated| updated < since)
                    && !commented.contains(&item.id))
                || candidates.contains(&item.id)
            {
                continue;
            }
            if task_matches(&item.task()?, query, project) {
                candidates.push(item.id.clone());
            }
        }
        Ok(candidates)
    }

    pub(super) fn held_comments(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<NativeId, Page<Comment>>>, SourceError> {
        self.held_comments
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "comment pages held for this command were left inconsistent; run the \
                          command again"
                    .into(),
            })
    }

    /// [`TaskSource::add_comments`](onetaskgraph_plugin_api::TaskSource::add_comments): one
    /// batched read of every task commented on — deciding each comment's hold on it — and every
    /// comment that read lets through in one [`graphql::ADD_COMMENTS`] request per
    /// [`COMMENT_BATCH`].
    pub(super) async fn comments_batched(
        &self,
        comments: &[BatchComment],
    ) -> Vec<Result<BatchAnswer<Comment>, SourceError>> {
        if comments.len() < 2 || self.expected.journaling() {
            let mut answers = Vec::with_capacity(comments.len());
            for each in comments {
                answers.push(self.comment_alone(each).await);
            }
            return answers;
        }
        let mut answers: Vec<Option<Result<BatchAnswer<Comment>, SourceError>>> =
            comments.iter().map(|_| None).collect();
        // An author is refused before anything is read, as one comment's is.
        for (at, each) in comments.iter().enumerate() {
            if each.comment.author.is_some() {
                answers[at] = Some(
                    TaskSource::add_comment(self, &each.task, &each.comment)
                        .await
                        .map(|written| written.map_or(BatchAnswer::Missing, BatchAnswer::Written)),
                );
            }
        }
        let wanted: Vec<NativeId> = comments
            .iter()
            .zip(&answers)
            .filter(|(_, answer)| answer.is_none())
            .map(|(each, _)| each.task.clone())
            .fold(Vec::new(), |mut ids, id| {
                if !ids.contains(&id) {
                    ids.push(id);
                }
                ids
            });
        let read = match self.items_by_ids(&wanted).await {
            Ok(read) => read,
            Err(error) => {
                return answers
                    .into_iter()
                    .map(|answer| answer.unwrap_or_else(|| Err(error.clone())))
                    .collect();
            }
        };
        let item_of = |id: &NativeId| {
            wanted
                .iter()
                .position(|wanted| wanted == id)
                .and_then(|at| read[at].clone())
        };
        let mut sending: Vec<(usize, NativeId, String)> = Vec::new();
        for (at, each) in comments.iter().enumerate() {
            if answers[at].is_some() {
                continue;
            }
            let item =
                item_of(&each.task).filter(|item| item.kind == BoardKind::Work(ItemKind::Task));
            let decided = match &item {
                Some(item) if item.content_kind == ContentKind::DraftIssue => {
                    Some(Err(self.draft_has_no_comments(&each.task)))
                }
                Some(item) => match item.task() {
                    Ok(task) => {
                        decided_on(self, Some(&task), each.if_version.as_deref(), &[]).map(Ok)
                    }
                    Err(error) => Some(Err(error)),
                },
                None => Some(Ok(BatchAnswer::Missing)),
            };
            match (decided, item) {
                (Some(answer), _) => answers[at] = Some(answer),
                (None, Some(item)) => {
                    sending.push((at, item.id, each.comment.body.as_str().to_owned()));
                }
                (None, None) => answers[at] = Some(Ok(BatchAnswer::Missing)),
            }
        }
        for chunk in sending.chunks(COMMENT_BATCH) {
            for (at, answer) in self.comment_round(chunk).await {
                answers[at] = Some(answer);
            }
        }
        answers
            .into_iter()
            .map(|answer| answer.unwrap_or(Ok(BatchAnswer::Missing)))
            .collect()
    }

    /// One comment of a batch on its own, as the default of
    /// [`TaskSource::add_comments`](onetaskgraph_plugin_api::TaskSource::add_comments) adds it:
    /// one read where its hold needs one, then the comment.
    async fn comment_alone(
        &self,
        each: &BatchComment,
    ) -> Result<BatchAnswer<Comment>, SourceError> {
        if each.if_version.is_some() {
            let task = TaskSource::get_task(self, &each.task).await?;
            if let Some(answer) = decided_on(self, task.as_ref(), each.if_version.as_deref(), &[]) {
                return Ok(answer);
            }
        }
        Ok(TaskSource::add_comment(self, &each.task, &each.comment)
            .await?
            .map_or(BatchAnswer::Missing, BatchAnswer::Written))
    }

    /// One [`graphql::ADD_COMMENTS`] request adding every comment of `chunk` — each `(answer
    /// index, issue, body)` — answered one per comment.
    async fn comment_round(
        &self,
        chunk: &[(usize, NativeId, String)],
    ) -> Vec<(usize, Result<BatchAnswer<Comment>, SourceError>)> {
        let input = |(_, issue, body): &(usize, NativeId, String)| json!({"subjectId": issue.0, "body": body});
        let mut variables = serde_json::Map::new();
        for slot in 0..COMMENT_BATCH {
            // A slot no comment holds repeats the first one's input, well formed as GitHub
            // requires of a variable it never runs.
            variables.insert(
                format!("comment{slot}"),
                input(chunk.get(slot).unwrap_or(&chunk[0])),
            );
            variables.insert(format!("send{slot}"), json!(slot < chunk.len()));
        }
        if let Ok(mut held) = self.held_comments() {
            for (_, issue, _) in chunk {
                held.remove(issue);
            }
        }
        let data = match self
            .graphql(graphql::ADD_COMMENTS, Value::Object(variables))
            .await
        {
            Ok(data) => data,
            Err(error) => {
                return chunk
                    .iter()
                    .map(|(at, _, _)| (*at, Err(error.clone())))
                    .collect();
            }
        };
        let mut answered = Vec::with_capacity(chunk.len());
        for (slot, (at, issue, _)) in chunk.iter().enumerate() {
            let alias = format!("comment{slot}");
            let answer = match data.get(&alias).filter(|value| !value.is_null()) {
                None => Err(refused_alias(
                    &data,
                    &alias,
                    "adding several comments together",
                )),
                Some(added) => self.added_comment(added, issue),
            };
            answered.push((*at, answer));
        }
        answered
    }

    /// One `addComment` payload of `issue`, read as [`TaskSource::add_comment`] reads its own.
    ///
    /// [`TaskSource::add_comment`]: onetaskgraph_plugin_api::TaskSource::add_comment
    fn added_comment(
        &self,
        added: &Value,
        issue: &NativeId,
    ) -> Result<BatchAnswer<Comment>, SourceError> {
        let subject = added
            .get("subject")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub comment addition returned no subject".into(),
            })?;
        if required_str(subject, "id")? != issue.0 {
            return Err(SourceError::Malformed {
                message: "GitHub comment addition answered about another issue".into(),
            });
        }
        let node = added
            .pointer("/commentEdge/node")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub comment addition returned no comment".into(),
            })?;
        let comment = comment_from(node)?;
        self.remember_commented(issue)?;
        Ok(BatchAnswer::Written(comment))
    }

    fn task_writes(&self) -> Result<std::sync::MutexGuard<'_, Option<TaskWrites>>, SourceError> {
        self.task_writes
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "the writes a batch of updates held back were left inconsistent; run the \
                      command again"
                    .into(),
            })
    }

    /// Hold one task's board field writes on `board` for the batch of updates running, when one
    /// is: answers whether they were held.
    pub(super) fn hold_fields(
        &self,
        board: &str,
        item: &str,
        writes: &[(String, Value)],
        clears: &[&str],
    ) -> Result<bool, SourceError> {
        let mut held = self.task_writes()?;
        let Some(batch) = held.as_mut() else {
            return Ok(false);
        };
        let Some(task) = batch.current.clone() else {
            return Ok(false);
        };
        batch.fields.push((
            task,
            HeldFields {
                board: board.to_owned(),
                item: item.to_owned(),
                writes: writes.to_vec(),
                clears: clears.iter().map(|field| (*field).to_owned()).collect(),
            },
        ));
        Ok(true)
    }

    /// Hold one task's issue update for the batch of updates running, when one is: answers
    /// whether it was held.
    pub(super) fn hold_issue(&self, fields: &Value) -> Result<bool, SourceError> {
        let mut held = self.task_writes()?;
        let Some(batch) = held.as_mut() else {
            return Ok(false);
        };
        let Some(task) = batch.current.clone() else {
            return Ok(false);
        };
        batch.issues.push((task, fields.clone()));
        Ok(true)
    }

    /// One update of a batch on its own, as the default of
    /// [`TaskSource::update_tasks`](onetaskgraph_plugin_api::TaskSource::update_tasks) makes it:
    /// one read where its hold or its guard needs one, then the update.
    async fn update_alone(
        &self,
        each: &BatchUpdate,
    ) -> Result<BatchAnswer<TaskUpdateOutcome>, SourceError> {
        if each.if_version.is_some() || !each.when_priority.is_empty() {
            let task = TaskSource::get_task(self, &each.id).await?;
            if let Some(answer) = decided_on(
                self,
                task.as_ref(),
                each.if_version.as_deref(),
                &each.when_priority,
            ) {
                return Ok(answer);
            }
        }
        Ok(self
            .targeted_update(&each.id, &each.update)
            .await?
            .map_or(BatchAnswer::Missing, BatchAnswer::Written))
    }

    /// [`TaskSource::update_tasks`](onetaskgraph_plugin_api::TaskSource::update_tasks): one
    /// batched read of every task updated, deciding each update's hold and guard on it; then
    /// every update that read lets through composed exactly as one update is, with its board
    /// field writes and its issue update held back, and sent together in
    /// [`graphql::WRITE_TASKS`] requests — every field write first, then the issue update of
    /// every task whose field writes all landed.
    ///
    /// A batch of one is the update alone, as are a task that is a board draft, an update
    /// naming dependencies, and one of an item whose body this process has not read — each
    /// sends writes of its own the batch does not hold.
    pub(super) async fn updates_batched(
        &self,
        updates: &[BatchUpdate],
    ) -> Vec<Result<BatchAnswer<TaskUpdateOutcome>, SourceError>> {
        let alone = updates.len() < 2
            || self.expected.journaling()
            || self.deferring().unwrap_or(true)
            || self.task_writes().map_or(true, |held| held.is_some());
        if alone {
            let mut answers = Vec::with_capacity(updates.len());
            for each in updates {
                answers.push(self.update_alone(each).await);
            }
            return answers;
        }
        let mut answers: Vec<Option<Result<BatchAnswer<TaskUpdateOutcome>, SourceError>>> =
            updates.iter().map(|_| None).collect();
        let wanted: Vec<NativeId> =
            updates
                .iter()
                .map(|each| each.id.clone())
                .fold(Vec::new(), |mut ids, id| {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                    ids
                });
        let read = match self.items_by_ids(&wanted).await {
            Ok(read) => read,
            Err(error) => return updates.iter().map(|_| Err(error.clone())).collect(),
        };
        let mut writing: Vec<usize> = Vec::new();
        let mut separately: Vec<usize> = Vec::new();
        for (at, each) in updates.iter().enumerate() {
            let item = wanted
                .iter()
                .position(|id| *id == each.id)
                .and_then(|slot| read[slot].clone())
                .filter(|item| item.kind == BoardKind::Work(ItemKind::Task));
            let Some(item) = item else {
                answers[at] = Some(Ok(BatchAnswer::Missing));
                continue;
            };
            let task = match item.task() {
                Ok(task) => task,
                Err(error) => {
                    answers[at] = Some(Err(error));
                    continue;
                }
            };
            if let Some(answer) = decided_on(
                self,
                Some(&task),
                each.if_version.as_deref(),
                &each.when_priority,
            ) {
                answers[at] = Some(Ok(answer));
                continue;
            }
            if item.content_kind == ContentKind::DraftIssue
                || each.update.depends_on.is_some()
                || self.is_unread(&each.id)
            {
                separately.push(at);
            } else {
                writing.push(at);
            }
        }
        // Two updates of one task are each composed on what the one before left.
        let repeated: Vec<usize> = writing
            .iter()
            .copied()
            .filter(|at| {
                writing
                    .iter()
                    .filter(|other| updates[**other].id == updates[*at].id)
                    .count()
                    > 1
            })
            .collect();
        writing.retain(|at| !repeated.contains(at));
        separately.extend(repeated);
        for at in separately {
            answers[at] = Some(
                self.targeted_update(&updates[at].id, &updates[at].update)
                    .await
                    .map(|written| written.map_or(BatchAnswer::Missing, BatchAnswer::Written)),
            );
        }
        if let Err(error) = self
            .task_writes()
            .map(|mut held| *held = Some(TaskWrites::default()))
        {
            for at in writing {
                answers[at] = Some(Err(error.clone()));
            }
            return answers
                .into_iter()
                .map(|answer| answer.unwrap_or(Ok(BatchAnswer::Missing)))
                .collect();
        }
        let mut composed: Vec<(usize, TaskUpdateOutcome)> = Vec::new();
        for at in writing {
            let id = &updates[at].id;
            if let Ok(mut held) = self.task_writes()
                && let Some(batch) = held.as_mut()
            {
                batch.current = Some(id.clone());
            }
            let written = self.targeted_update(id, &updates[at].update).await;
            match written {
                Ok(Some(outcome)) => composed.push((at, outcome)),
                failed => {
                    // Nothing of an update that was not composed whole is sent.
                    if let Ok(mut held) = self.task_writes()
                        && let Some(batch) = held.as_mut()
                    {
                        batch.fields.retain(|(task, _)| task != id);
                        batch.issues.retain(|(task, _)| task != id);
                    }
                    answers[at] =
                        Some(failed.map(|written| {
                            written.map_or(BatchAnswer::Missing, BatchAnswer::Written)
                        }));
                }
            }
        }
        let held = self
            .task_writes()
            .map(|mut held| held.take())
            .ok()
            .flatten()
            .unwrap_or_default();
        let refused = self.send_task_writes(held).await;
        for (at, outcome) in composed {
            let id = &updates[at].id;
            answers[at] = Some(match refused.get(id) {
                None => Ok(BatchAnswer::Written(outcome)),
                Some(error) => {
                    // What this process recorded of the write is not what the board holds.
                    let _ = self.forget(id);
                    Err(error.clone())
                }
            });
        }
        answers
            .into_iter()
            .map(|answer| answer.unwrap_or(Ok(BatchAnswer::Missing)))
            .collect()
    }

    /// Send every write `held` holds, in [`graphql::WRITE_TASKS`] requests: every task's field
    /// writes and clears, then the issue update of every task none of whose field writes was
    /// refused — answering, by task, the refusal of each that was not written whole.
    async fn send_task_writes(&self, held: TaskWrites) -> BTreeMap<NativeId, SourceError> {
        let mut refused: BTreeMap<NativeId, SourceError> = BTreeMap::new();
        // Every write of one task goes in one request, so a task is never split across two.
        let mut requests: Vec<Vec<&(NativeId, HeldFields)>> = Vec::new();
        let (mut writes, mut clears) = (0, 0);
        for held in &held.fields {
            let (more_writes, more_clears) = (held.1.writes.len(), held.1.clears.len());
            let fits = writes + more_writes <= TASK_FIELD_BATCH
                && clears + more_clears <= MUTATION_BATCH
                && !requests.is_empty();
            if !fits {
                requests.push(Vec::new());
                (writes, clears) = (0, 0);
            }
            writes += more_writes;
            clears += more_clears;
            if let Some(request) = requests.last_mut() {
                request.push(held);
            }
        }
        for request in requests {
            for (task, error) in self.send_field_writes(&request).await {
                refused.entry(task).or_insert(error);
            }
        }
        let issues: Vec<&(NativeId, Value)> = held
            .issues
            .iter()
            .filter(|(task, _)| !refused.contains_key(task))
            .collect();
        for chunk in issues.chunks(MUTATION_BATCH) {
            for (task, error) in self.send_issue_updates(chunk).await {
                refused.entry(task).or_insert(error);
            }
        }
        refused
    }

    /// The variables of one [`graphql::WRITE_TASKS`] request sending nothing, every input well
    /// formed and naming `filler`, as GitHub requires of a variable it never runs.
    fn unsent_task_writes(filler: &str) -> serde_json::Map<String, Value> {
        let mut variables = serde_json::Map::new();
        for slot in 0..TASK_FIELD_BATCH {
            variables.insert(
                format!("field{slot}"),
                json!({"projectId":filler,"itemId":filler,"fieldId":filler,"value":{"text":""}}),
            );
            variables.insert(format!("sendField{slot}"), json!(false));
        }
        for slot in 0..MUTATION_BATCH {
            variables.insert(
                format!("clear{slot}"),
                json!({"projectId":filler,"itemId":filler,"fieldId":filler}),
            );
            variables.insert(format!("sendClear{slot}"), json!(false));
            variables.insert(format!("update{slot}"), json!({"id":filler}));
            variables.insert(format!("sendUpdate{slot}"), json!(false));
        }
        variables
    }

    /// One [`graphql::WRITE_TASKS`] request of `request`'s field writes and clears, answering
    /// each task with a write GitHub refused or answered about another item.
    async fn send_field_writes(
        &self,
        request: &[&(NativeId, HeldFields)],
    ) -> Vec<(NativeId, SourceError)> {
        let Some((_, first)) = request.first() else {
            return Vec::new();
        };
        let mut variables = Self::unsent_task_writes(&first.item);
        let mut sent: Vec<(String, &NativeId, &str)> = Vec::new();
        let (mut writes, mut clears) = (0, 0);
        for (task, held) in request {
            for (field, value) in &held.writes {
                let alias = format!("field{writes}");
                variables.insert(
                    alias.clone(),
                    json!({"projectId":held.board,"itemId":held.item,"fieldId":field,"value":value}),
                );
                variables.insert(format!("sendField{writes}"), json!(true));
                sent.push((alias, task, &held.item));
                writes += 1;
            }
            for field in &held.clears {
                let alias = format!("clear{clears}");
                variables.insert(
                    alias.clone(),
                    json!({"projectId":held.board,"itemId":held.item,"fieldId":field}),
                );
                variables.insert(format!("sendClear{clears}"), json!(true));
                sent.push((alias, task, &held.item));
                clears += 1;
            }
        }
        let doing = "writing several tasks' board fields together";
        let data = match self
            .graphql(graphql::WRITE_TASKS, Value::Object(variables))
            .await
        {
            Ok(data) => data,
            Err(error) => {
                return request
                    .iter()
                    .map(|(task, _)| (task.clone(), error.clone()))
                    .collect();
            }
        };
        let mut refused = Vec::new();
        for (alias, task, item) in sent {
            let answered = data
                .get(&alias)
                .filter(|value| !value.is_null())
                .and_then(|value| value.pointer("/projectV2Item/id"))
                .and_then(Value::as_str);
            match answered {
                Some(answered) if answered == item => {}
                Some(_) => refused.push((
                    task.clone(),
                    SourceError::Malformed {
                        message: format!(
                            "GitHub answered {alias} while {doing} with another project item"
                        ),
                    },
                )),
                None => refused.push((task.clone(), refused_alias(&data, &alias, doing))),
            }
        }
        refused
    }

    /// One [`graphql::WRITE_TASKS`] request of the issue updates of `chunk`, answering each task
    /// GitHub refused or answered about another issue.
    async fn send_issue_updates(
        &self,
        chunk: &[&(NativeId, Value)],
    ) -> Vec<(NativeId, SourceError)> {
        let Some((first, _)) = chunk.first() else {
            return Vec::new();
        };
        let mut variables = Self::unsent_task_writes(&first.0);
        for (slot, (task, fields)) in chunk.iter().enumerate() {
            let mut input = fields.clone();
            input["id"] = json!(task.0);
            variables.insert(format!("update{slot}"), input);
            variables.insert(format!("sendUpdate{slot}"), json!(true));
        }
        let doing = "updating several tasks together";
        let data = match self
            .graphql(graphql::WRITE_TASKS, Value::Object(variables))
            .await
        {
            Ok(data) => data,
            Err(error) => {
                return chunk
                    .iter()
                    .map(|(task, _)| (task.clone(), error.clone()))
                    .collect();
            }
        };
        let mut refused = Vec::new();
        for (slot, (task, _)) in chunk.iter().enumerate() {
            let alias = format!("update{slot}");
            let answered = data
                .get(&alias)
                .filter(|value| !value.is_null())
                .and_then(|value| value.pointer("/issue/id"))
                .and_then(Value::as_str);
            match answered {
                Some(answered) if answered == task.0 => {}
                Some(_) => refused.push((
                    task.clone(),
                    SourceError::Malformed {
                        message: format!(
                            "GitHub answered {alias} while {doing} with another issue"
                        ),
                    },
                )),
                None => refused.push((task.clone(), refused_alias(&data, &alias, doing))),
            }
        }
        refused
    }
}
