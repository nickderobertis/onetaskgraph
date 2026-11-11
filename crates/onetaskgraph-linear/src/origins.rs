//! What a copy asks this source together before it looks for the items copied from each origin
//! — [`TaskSource::read_ahead_origins_for_copy`] — and how each later lookup is answered from it.
//!
//! See the crate documentation's ruling on the requests a copy sends.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    Cursor, LabelFilter, Page, PageRequest, Project, ProjectQuery, SourceError, Task, TaskQuery,
    TaskSource,
};
use serde_json::{Value, json};

use crate::{
    CreateId, LinearSource, MAX_PAGE_SIZE, ORIGIN_BATCH, RESOLUTION_ROWS, Vocabulary, connection,
    create_key_of, graphql, linear_ids_of_key, map_project, map_task, slot_phrase,
};
use onetaskgraph_plugin_api::{Document, MetadataRecord, NativeId};

/// The namespace every create key is a version-5 UUID under — the store's, which its README
/// publishes so another implementation computes the same keys, and which this source computes
/// [`LinearSource::origin_key`] under for exactly that reason.
pub const CREATE_KEY_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x7a1c_3f0e_5b2d_4c8a_9e61_0f4d_2b7c_5a39);

/// What [`TaskSource::read_ahead_origins_for_copy`] read, held for the command.
#[derive(Default)]
pub(crate) struct OriginsAhead {
    /// The issues each origin's own task query would have answered, by origin — held only for
    /// an origin the batched issues page answered whole.
    tasks: BTreeMap<String, Vec<Task>>,
    /// The projects each origin's own [`TaskSource::query_projects_by_origin`] would have
    /// answered, by origin — held only when the batched projects page held every project.
    projects: BTreeMap<String, Vec<Project>>,
    /// The workspace's organization id that same request read, which
    /// [`TaskSource::designator`] digests.
    organization: Option<String>,
    /// The origins a copy asked about and no lookup has needed yet: asked of Linear together,
    /// in one request per [`ORIGIN_BATCH`], at the first lookup by origin — so a copy that never
    /// looks an issue or a project up by its origin, a document copy among them, sends none.
    pending: Vec<String>,
    /// The origins a copy asked about together — two or more in one read ahead — for the whole
    /// command: a copy creating several issues, whose lookup of each that lists none is not
    /// followed by a read of the id its key derives; see [`LinearSource::tasks_with_copied_by_key`].
    together: std::collections::BTreeSet<String>,
}

/// The one task query a copy asks by origin — no other member set — which is the only query
/// [`LinearSource::held_origin_tasks`] answers from what the batched read held.
fn origin_query(origin: &str) -> TaskQuery {
    TaskQuery {
        origin: Some(origin.to_owned()),
        ..TaskQuery::default()
    }
}

/// The cursor prefix this source's by-origin project listing hands out. Opaque to a caller and
/// read back only by [`page_of_matches`], which every page of that listing goes through — a page
/// answered from the batched read included — so no other implementation's cursor reaches it.
const MATCHED_PAGE_CURSOR: &str = "linear-origin:";

impl LinearSource {
    /// This source's record of what the batched origin read held, or the refusal a poisoned
    /// lock is.
    fn origins_ahead(&self) -> Result<std::sync::MutexGuard<'_, OriginsAhead>, SourceError> {
        self.origins_ahead
            .lock()
            .map_err(|_| SourceError::Unavailable {
                message: "this source's record of the items copied from each origin was left \
                          inconsistent by an earlier failure; next: run the command again"
                    .into(),
            })
    }

    /// Drop every origin's held answer, as a write does: what it read may no longer be what
    /// Linear holds. The organization id is kept unless `all` — no write moves which workspace
    /// this is — and `all` is the end of the command.
    pub(crate) fn forget_origins_ahead(&self, all: bool) {
        if let Ok(mut held) = self.origins_ahead.lock() {
            held.tasks.clear();
            held.projects.clear();
            held.pending.clear();
            if all {
                held.organization = None;
                held.together.clear();
            }
        }
    }

    /// The organization id the batched origin read carried, when one did this command.
    pub(crate) fn held_organization(&self) -> Result<Option<String>, SourceError> {
        Ok(self.origins_ahead()?.organization.clone())
    }

    /// The answer to `query` at `page` held for its origin, taken once — or `None` when the
    /// query is not exactly a copy's origin lookup, asks for a later page, or holds more rows
    /// than the page may, each of which is sent as it always was.
    pub(crate) fn held_origin_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Option<Page<Task>>, SourceError> {
        let Some(origin) = &query.origin else {
            return Ok(None);
        };
        if page.cursor.is_some() || *query != origin_query(origin) {
            return Ok(None);
        }
        let Some(tasks) = self.origins_ahead()?.tasks.remove(origin) else {
            return Ok(None);
        };
        Ok(fits(tasks.len(), page).then_some(Page {
            items: tasks,
            next: None,
        }))
    }

    /// Ask, together, which issues and projects were copied from each of `origins`, and hold
    /// each answer this command can trust; see [`TaskSource::read_ahead_origins_for_copy`].
    ///
    /// An origin whose slot phrase cannot be sent ([`slot_phrase`]) is not asked: its own
    /// lookup reads the whole team, which no `or` of phrases could stand in for.
    ///
    /// Nothing is sent yet: the origins are held until the first lookup by origin needs one,
    /// which [`Self::send_origins_ahead`] answers. A source with no `team` holds none, since the
    /// read carries the resolution a write needs and only a team names it.
    pub(crate) async fn read_origins_ahead(&self, origins: &[String]) -> Result<(), SourceError> {
        if self.team.is_none() {
            return Ok(());
        }
        let mut held = self.origins_ahead()?;
        if origins.len() > 1 {
            held.together.extend(origins.iter().cloned());
        }
        for origin in origins {
            if slot_phrase(origin).is_some()
                && !held.tasks.contains_key(origin)
                && !held.projects.contains_key(origin)
                && !held.pending.contains(origin)
            {
                held.pending.push(origin.clone());
            }
        }
        Ok(())
    }

    /// Ask Linear about every origin [`Self::read_origins_ahead`] held, in one request per
    /// [`ORIGIN_BATCH`]; nothing when none is held.
    pub(crate) async fn send_origins_ahead(&self) -> Result<(), SourceError> {
        let pending = std::mem::take(&mut self.origins_ahead()?.pending);
        let pending: Vec<&String> = pending.iter().collect();
        for chunk in pending.chunks(ORIGIN_BATCH) {
            self.read_origin_chunk(chunk).await?;
        }
        Ok(())
    }

    /// One request of [`graphql::COUNTERPART_LOOKUPS`] for at most [`ORIGIN_BATCH`] origins, carrying
    /// the team's workflow states and the workspace's project statuses beside them — the
    /// resolution a copy's first write reads otherwise, since a copy finding nothing creates —
    /// held as [`LinearSource::vocabulary`] holds a resolution read on its own, only when it
    /// read completely.
    async fn read_origin_chunk(&self, chunk: &[&String]) -> Result<(), SourceError> {
        let Some(team) = self.team.as_ref() else {
            return Ok(());
        };
        let mut issues = self.issue_scope();
        issues.push(json!({"or": chunk
            .iter()
            .filter_map(|origin| slot_phrase(origin))
            .collect::<Vec<Value>>()}));
        // The issues page is sized to the origins it answers; the projects page, which no
        // origin narrows, to the rest of one projects listing page, so the request asks for no
        // more rows than that page does — with the cache in front, each row carrying the token
        // parts a listing of it would.
        let issues_first = u32::try_from(chunk.len())
            .unwrap_or(MAX_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let projects_first = self
            .projects_page()
            .saturating_sub(issues_first + RESOLUTION_ROWS)
            .max(1);
        let data = self
            .send(
                self.read_document(graphql::COUNTERPART_LOOKUPS),
                json!({
                    "issuesFirst": issues_first,
                    "issuesFilter": Self::narrowed(issues),
                    "projectsFirst": projects_first,
                    "projectsFilter": self.project_filter(&LabelFilter::default(), &[]),
                    "key": team.0,
                }),
            )
            .await?;
        // Held only when it read whole, exactly as a resolution read on its own is; otherwise the
        // first write reads it as it always did.
        if let Ok(read) = Vocabulary::read(&data, &self.name, &team.0) {
            *self
                .vocabulary
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(std::sync::Arc::new(read));
        }
        let tasks = connection(&data, "originIssues", |v| {
            map_task(v, &self.name, &self.statuses, self.spelling())
        })?;
        let projects = connection(&data, "originProjects", |v| {
            map_project(v, &self.statuses, self.spelling())
        })?;
        // A connection answering more rows than its page asked for is refused, as a walk's page
        // is: a backend that oversteps its page cannot be trusted to have answered it.
        for (name, rows, first) in [
            ("issues", tasks.items.len(), issues_first),
            ("projects", projects.items.len(), projects_first),
        ] {
            if rows as u64 > u64::from(first) {
                return Err(SourceError::Malformed {
                    message: format!(
                        "the source returned {rows} {name} rows for a page of at most {first}; a \
                         source may return fewer than it was asked for and never more"
                    ),
                });
            }
        }
        // Read as the resolution reads it: an answer naming no workspace, or none by a string id,
        // holds none here, and the resolution read on its own refuses it where it is needed.
        let organization = data
            .get("organization")
            .and_then(|organization| organization.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        // An origin is never taken as having no counterpart from a page with more behind it:
        // a match on the next page is one this page cannot see.
        let mut held = self.origins_ahead()?;
        for origin in chunk {
            let query = origin_query(origin);
            if tasks.next.is_none() {
                let found = tasks
                    .items
                    .iter()
                    .filter(|task| Self::confirms(&query, task))
                    .cloned()
                    .collect();
                held.tasks.insert((*origin).clone(), found);
            }
            if projects.next.is_none() {
                let found = projects
                    .items
                    .iter()
                    .filter(|project| query.origin_matches(&project.metadata))
                    .cloned()
                    .collect();
                held.projects.insert((*origin).clone(), found);
            }
        }
        // An empty id names no workspace; the designator's own read refuses it.
        if !organization.trim().is_empty() {
            held.organization = Some(organization);
        }
        Ok(())
    }

    /// [`TaskSource::query_projects_by_origin`]: the answer held for `origin`, taken once, at
    /// the first page — and otherwise exactly the trait's default, the whole scope walked and
    /// narrowed here, because `ProjectFilter` has no member a slot phrase could be sent in.
    pub(crate) async fn projects_by_origin(
        &self,
        origin: &str,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.send_origins_ahead().await?;
        if page.cursor.is_none() {
            let held = self.origins_ahead()?.projects.remove(origin);
            if let Some(projects) = held
                && fits(projects.len(), page)
            {
                return self
                    .with_copied_by_key(
                        MetadataRecord::Project,
                        origin,
                        page,
                        Page {
                            items: projects,
                            next: None,
                        },
                        Copied::into_project,
                    )
                    .await;
            }
        }
        let wanted = origin_query(origin);
        let mut matched = Vec::new();
        let mut cursor: Option<Cursor> = None;
        let mut visited: Vec<String> = Vec::new();
        loop {
            let listed = self
                .query_projects(
                    &ProjectQuery::default(),
                    &PageRequest {
                        cursor: cursor.clone(),
                        limit: page.limit,
                    },
                )
                .await?;
            if listed.items.len() as u64 > u64::from(page.limit) {
                return Err(SourceError::Malformed {
                    message: format!(
                        "the source returned {} rows for a page of at most {}; a source may \
                         return fewer than it was asked for and never more",
                        listed.items.len(),
                        page.limit
                    ),
                });
            }
            matched.extend(
                listed
                    .items
                    .into_iter()
                    .filter(|project| wanted.origin_matches(&project.metadata)),
            );
            cursor = match listed.next {
                None => break,
                Some(next) if visited.contains(&next.0) => {
                    return Err(SourceError::Malformed {
                        message: "the source returned a cursor it had already handed out while \
                                  its projects were walked for one origin, so the walk would \
                                  never end"
                            .into(),
                    });
                }
                Some(next) => {
                    visited.push(next.0.clone());
                    Some(next)
                }
            };
        }
        if page.cursor.is_none() && matched.is_empty() {
            return self
                .with_copied_by_key(
                    MetadataRecord::Project,
                    origin,
                    page,
                    Page::last(Vec::new()),
                    Copied::into_project,
                )
                .await;
        }
        page_of_matches(matched, page)
    }
}

impl LinearSource {
    /// The create key the store derives for a create of `record` copied from `origin` into this
    /// source that replaces no link: the version-5 UUID, under [`CREATE_KEY_NAMESPACE`], of
    /// `<origin>|<this source's name>|`, with `|project` or `|document` appended for a project or
    /// a document — the README's rule, so it is the key a copy recording an intent gives that
    /// create too, and the ids it derives are the ones that create is sent under.
    pub(crate) fn origin_key(&self, record: MetadataRecord, origin: &str) -> uuid::Uuid {
        let mut name = format!("{origin}|{}|", self.name);
        match record {
            MetadataRecord::Task => {}
            MetadataRecord::Project => name.push_str("|project"),
            MetadataRecord::Document => name.push_str("|document"),
        }
        uuid::Uuid::new_v5(&CREATE_KEY_NAMESPACE, name.as_bytes())
    }

    /// The ids a create of `record` carrying `metadata` is sent under: its own create key's,
    /// when it carries one; otherwise, when it records an origin, the ids of that origin's key —
    /// so a copy that recorded no intent, and no link, still sends each create under ids a later
    /// copy can read it back by — and otherwise none. An update chooses none.
    ///
    /// # Errors
    ///
    /// [`SourceError::Malformed`] for a create key that is not a hyphenated RFC 4122 UUID, as
    /// [`linear_ids_of_key`] refuses one.
    pub(crate) fn create_id(
        &self,
        record: MetadataRecord,
        metadata: &std::collections::BTreeMap<String, Value>,
        target: Option<&NativeId>,
    ) -> Result<CreateId, SourceError> {
        if target.is_some() {
            return Ok(CreateId::Unchosen);
        }
        if let Some(key) = create_key_of(metadata, target) {
            linear_ids_of_key(&key)?;
            return uuid::Uuid::parse_str(&key)
                .map(CreateId::Keyed)
                .map_err(|_| SourceError::Malformed {
                    message: format!("create key {key:?} is not a hyphenated UUID"),
                });
        }
        Ok(
            match metadata.get(TaskQuery::ORIGIN_KEY).and_then(Value::as_str) {
                Some(origin) => CreateId::Origin {
                    key: self.origin_key(record, origin),
                    origin: origin.to_owned(),
                },
                None => CreateId::Unchosen,
            },
        )
    }

    /// The `record` at `id` when it records `origin`, read by that id alone — `None` when this
    /// source holds none there, holds a trashed one, or holds one recording anything else.
    pub(crate) async fn copied_at(
        &self,
        record: MetadataRecord,
        id: &NativeId,
        origin: &str,
    ) -> Result<Option<NativeId>, SourceError> {
        Ok(self
            .read_copied(record, id)
            .await?
            .filter(|held| origin_query(origin).origin_matches(held.metadata()))
            .map(|held| held.id().clone()))
    }

    async fn read_copied(
        &self,
        record: MetadataRecord,
        id: &NativeId,
    ) -> Result<Option<Copied>, SourceError> {
        Ok(match record {
            MetadataRecord::Task => self.get_task(id).await?.map(Copied::Task),
            MetadataRecord::Project => self.get_project(id).await?.map(Copied::Project),
            MetadataRecord::Document => self.get_document(id).await?.map(Copied::Document),
        })
    }

    /// The `record` an earlier copy of `origin` created here, read by the ids [`Self::origin_key`]
    /// derives — the ids that create was sent under — rather than through any filtered or
    /// searched read, so it is there however soon after that create it is asked for.
    ///
    /// The generations are read in turn, as a create tries them: one holding a record recording
    /// another origin is read past, and the first reading as no record — none there, or a trashed
    /// one, which a read answers as none — ends the read, since a create of `origin` tries that id
    /// before any later one. A counterpart behind a trashed record is therefore not reached here;
    /// its create is refused naming it instead ([`LinearSource::copied_at`]). Asked only once the
    /// lookup itself has listed nothing, so a copy whose lookup finds its counterpart sends
    /// nothing more.
    async fn copied_by_origin_key(
        &self,
        record: MetadataRecord,
        origin: &str,
    ) -> Result<Option<Copied>, SourceError> {
        for id in linear_ids_of_key(&self.origin_key(record, origin).hyphenated().to_string())? {
            match self.read_copied(record, &NativeId(id)).await? {
                None => return Ok(None),
                Some(held) if origin_query(origin).origin_matches(held.metadata()) => {
                    return Ok(Some(held));
                }
                Some(_) => {}
            }
        }
        Ok(None)
    }

    /// `listed`, the first page of a lookup of the `record`s copied from `origin`, with the one
    /// [`Self::copied_by_origin_key`] reads in its place when it listed none and has no page
    /// behind it.
    pub(crate) async fn with_copied_by_key<T>(
        &self,
        record: MetadataRecord,
        origin: &str,
        page: &PageRequest,
        listed: Page<T>,
        take: impl Fn(Copied) -> Option<T>,
    ) -> Result<Page<T>, SourceError> {
        if page.cursor.is_some() || !listed.items.is_empty() || listed.next.is_some() {
            return Ok(listed);
        }
        Ok(Page {
            items: self
                .copied_by_origin_key(record, origin)
                .await?
                .and_then(take)
                .into_iter()
                .collect(),
            next: None,
        })
    }

    /// `listed`, a page of `query`'s issues, with the one [`Self::copied_by_origin_key`] reads in
    /// its place when `query` is exactly a copy's lookup by origin and its first page listed none
    /// — but for an origin a copy asked about together with others. A copy creating several
    /// issues sends no read of each first: one an earlier copy made under that origin's key, which
    /// a lagging lookup did not list, is found by the keyed create's own conflict, Linear refusing
    /// a create under an id a record holds — which no index can lag — and is brought in step
    /// there by a compared update.
    pub(crate) async fn tasks_with_copied_by_key(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
        listed: Page<Task>,
    ) -> Result<Page<Task>, SourceError> {
        match &query.origin {
            Some(origin)
                if *query == origin_query(origin)
                    && !self.origins_ahead()?.together.contains(origin) =>
            {
                self.with_copied_by_key(
                    MetadataRecord::Task,
                    origin,
                    page,
                    listed,
                    Copied::into_task,
                )
                .await
            }
            _ => Ok(listed),
        }
    }

    /// [`TaskSource::query_documents_by_origin`]: the scope's documents walked and narrowed to
    /// `origin`, as the trait's default walks them, and then the one an earlier copy created
    /// under that origin's key, read by id, when the walk found none.
    pub(crate) async fn documents_by_origin(
        &self,
        origin: &str,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        let wanted = origin_query(origin);
        let mut matched = Vec::new();
        let mut cursor: Option<Cursor> = None;
        let mut visited: Vec<String> = Vec::new();
        loop {
            let listed = self
                .query_documents(
                    &onetaskgraph_plugin_api::DocumentQuery::default(),
                    &PageRequest {
                        cursor: cursor.clone(),
                        limit: page.limit,
                    },
                )
                .await?;
            if !fits(listed.items.len(), page) {
                return Err(SourceError::Malformed {
                    message: format!(
                        "the source returned {} rows for a page of at most {}; a source may \
                         return fewer than it was asked for and never more",
                        listed.items.len(),
                        page.limit
                    ),
                });
            }
            matched.extend(
                listed
                    .items
                    .into_iter()
                    .filter(|document| wanted.origin_matches(&document.metadata)),
            );
            cursor = match listed.next {
                None => break,
                Some(next) if visited.contains(&next.0) => {
                    return Err(SourceError::Malformed {
                        message: "the source returned a cursor it had already handed out while \
                                  its documents were walked for one origin, so the walk would \
                                  never end"
                            .into(),
                    });
                }
                Some(next) => {
                    visited.push(next.0.clone());
                    Some(next)
                }
            };
        }
        if page.cursor.is_none() && matched.is_empty() {
            return self
                .with_copied_by_key(
                    MetadataRecord::Document,
                    origin,
                    page,
                    Page::last(Vec::new()),
                    Copied::into_document,
                )
                .await;
        }
        page_of_matches(matched, page)
    }
}

/// A record read by id for an origin's key.
pub(crate) enum Copied {
    Task(Task),
    Project(Project),
    Document(Document),
}

impl Copied {
    fn id(&self) -> &NativeId {
        match self {
            Self::Task(task) => &task.id,
            Self::Project(project) => &project.id,
            Self::Document(document) => &document.id,
        }
    }

    fn metadata(&self) -> &std::collections::BTreeMap<String, Value> {
        match self {
            Self::Task(task) => &task.metadata,
            Self::Project(project) => &project.metadata,
            Self::Document(document) => &document.metadata,
        }
    }

    pub(crate) fn into_task(self) -> Option<Task> {
        match self {
            Self::Task(task) => Some(task),
            _ => None,
        }
    }

    pub(crate) fn into_project(self) -> Option<Project> {
        match self {
            Self::Project(project) => Some(project),
            _ => None,
        }
    }

    pub(crate) fn into_document(self) -> Option<Document> {
        match self {
            Self::Document(document) => Some(document),
            _ => None,
        }
    }
}

/// Whether `rows` fit one page of `page`: a held answer larger than the page asked for is never
/// handed over, since a source may answer fewer rows than it was asked for and never more.
fn fits(rows: usize, page: &PageRequest) -> bool {
    rows as u64 <= u64::from(page.limit)
}

/// One page of `matched`, positioned by a cursor this listing handed out — the trait default's
/// own paging, so a walk reads the same whichever answered it.
fn page_of_matches<T>(matched: Vec<T>, page: &PageRequest) -> Result<Page<T>, SourceError> {
    let start = match &page.cursor {
        None => 0,
        Some(cursor) => cursor
            .0
            .strip_prefix(MATCHED_PAGE_CURSOR)
            .and_then(|position| position.parse::<usize>().ok())
            .filter(|position| *position <= matched.len())
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "cursor {:?} was not handed out by this by-origin listing; start the walk \
                     again without one",
                    cursor.0
                ),
            })?,
    };
    let end = start.saturating_add(page.limit as usize).min(matched.len());
    let next = (end < matched.len()).then(|| Cursor(format!("{MATCHED_PAGE_CURSOR}{end}")));
    Ok(Page {
        items: matched.into_iter().skip(start).take(end - start).collect(),
        next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_by_origin_page_walks_its_matches_by_the_cursor_it_hands_out() {
        let page = |cursor: Option<&str>| PageRequest {
            cursor: cursor.map(|c| Cursor(c.into())),
            limit: 2,
        };
        let first = page_of_matches(vec![1, 2, 3], &page(None)).unwrap();
        assert_eq!(first.items, vec![1, 2]);
        let next = first.next.expect("a next page");
        let second = page_of_matches(vec![1, 2, 3], &page(Some(&next.0))).unwrap();
        assert_eq!(second.items, vec![3]);
        assert!(second.next.is_none());
        assert!(page_of_matches(vec![1], &page(Some("elsewhere"))).is_err());
    }

    #[test]
    fn only_a_copys_own_origin_query_is_answered_from_what_was_held() {
        let mut query = origin_query("plan:T-1");
        assert_eq!(query, origin_query("plan:T-1"));
        query.project = onetaskgraph_plugin_api::ProjectFilter::Orphans;
        assert_ne!(query, origin_query("plan:T-1"));
    }
}
