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
    LinearSource, MAX_PAGE_SIZE, ORIGIN_BATCH, connection, graphql, map_project, map_task,
    slot_phrase, str_at,
};

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
}

/// The task query a copy asks for the items copied from `origin`, and nothing else.
fn origin_query(origin: &str) -> TaskQuery {
    TaskQuery {
        origin: Some(origin.to_owned()),
        ..TaskQuery::default()
    }
}

/// The cursor prefix a by-origin project listing hands out: the same one the trait's default
/// hands out, so a walk resumed from either reads the same.
const MATCHED_PAGE_CURSOR: &str = "origin:";

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
            if all {
                held.organization = None;
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
    pub(crate) async fn read_origins_ahead(&self, origins: &[String]) -> Result<(), SourceError> {
        // With the store's cache in front, a listing carries the token parts each row is pinned
        // by and prices each row higher; its lookups stay as they were.
        if self.attached() {
            return Ok(());
        }
        let unasked: Vec<&String> = {
            let held = self.origins_ahead()?;
            let mut unasked: Vec<&String> = Vec::new();
            for origin in origins {
                if slot_phrase(origin).is_some()
                    && !held.tasks.contains_key(origin)
                    && !held.projects.contains_key(origin)
                    && !unasked.contains(&origin)
                {
                    unasked.push(origin);
                }
            }
            unasked
        };
        for chunk in unasked.chunks(ORIGIN_BATCH) {
            self.read_origin_chunk(chunk).await?;
        }
        Ok(())
    }

    /// One request of [`graphql::ORIGIN_LOOKUPS`] for at most [`ORIGIN_BATCH`] origins.
    async fn read_origin_chunk(&self, chunk: &[&String]) -> Result<(), SourceError> {
        let mut issues = self.issue_scope();
        issues.push(json!({"or": chunk
            .iter()
            .filter_map(|origin| slot_phrase(origin))
            .collect::<Vec<Value>>()}));
        // The issues page is sized to the origins it answers; the projects page, which no
        // origin narrows, to the rest of one page of `MAX_PAGE_SIZE` rows, so the request is
        // priced as one listing page is.
        let issues_first = u32::try_from(chunk.len())
            .unwrap_or(MAX_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let projects_first = (MAX_PAGE_SIZE - issues_first)
            .min(self.projects_page())
            .max(1);
        let data = self
            .send(
                graphql::ORIGIN_LOOKUPS,
                json!({
                    "issuesFirst": issues_first,
                    "issuesFilter": Self::narrowed(issues),
                    "projectsFirst": projects_first,
                    "projectsFilter": self.project_filter(&LabelFilter::default(), &[]),
                }),
            )
            .await?;
        let tasks = connection(&data, "originIssues", |v| {
            map_task(v, &self.name, &self.statuses, self.spelling())
        })?;
        let projects = connection(&data, "originProjects", |v| {
            map_project(v, &self.statuses, self.spelling())
        })?;
        let organization = str_at(
            data.get("organization")
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing organization".into(),
                })?,
            "id",
        )?
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
        if page.cursor.is_none() {
            let held = self.origins_ahead()?.projects.remove(origin);
            if let Some(projects) = held
                && fits(projects.len(), page)
            {
                return Ok(Page {
                    items: projects,
                    next: None,
                });
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
        page_of_matches(matched, page)
    }
}

/// Whether `rows` fit one page of `page`.
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
