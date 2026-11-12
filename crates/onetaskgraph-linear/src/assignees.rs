//! A task's assignee, for a source whose configuration opted in with `assignees: true`.
//!
//! **Read.** Every issue selection a task is read from — a listing, a read by id, a batched
//! read, a detail read, the cache's widened reads and every write's own answer — selects
//! `assignee{displayName}` beside the rest ([`graphql::with_assignees`], applied where every
//! request is sent), so each task carries `assignees`: the one person Linear holds as the
//! issue's assignee, by display name, or none. A source that has not opted in sends every
//! document byte for byte as before and carries no `assignees` at all.
//!
//! **Write.** One person or none, replacing: the issue's `assigneeId` in the very input the
//! write already sends — `issueCreate`, `issueUpdate` — so naming an assignee adds no write
//! request, and `null` unassigns. A create naming nobody leaves the member out, so Linear
//! applies its own template or triage default as it does for any create. A Linear issue holds
//! at most one assignee, so a write naming more is refused before anything is sent.
//!
//! **Who a person is.** A display name is resolved to the user's id by one lookup request per
//! call for every name the call needs that this process has not resolved — [`graphql::USERS`]
//! for up to 250 names, and that document aliased once a 250 beyond, split into a second
//! request only past [`PEOPLE_PER_LOOKUP`], where one would exceed Linear's documented query
//! complexity limit. What it finds is held for the process with the team's other resolved ids,
//! shared by every instance over the same tracker. A write naming an assignee that fails, or is
//! answered with another person, has all of it dropped when the call ends: the rest of the
//! call goes on with what its one lookup found, and the next call resolves afresh. A name
//! Linear holds no user under is refused for that item alone, naming it, and remembered by
//! this instance until the command ends. Unassigning resolves nothing.
//!
//! **What landed.** Each write's own answer selects the assignee, and an answer naming another
//! person than the write asked for — or anybody, where it unassigned — is refused for that
//! item, naming both, and has what this process resolved dropped when the call ends.
//!
//! **The change token.** Linear moves an issue's `updatedAt` when its assignee changes, and the
//! `lin1` token is spelled from it, so an assignee-only change moves the task's version.

use onetaskgraph_plugin_api::{NativeId, SourceError, Task, unwritable_field};
use serde_json::{Value, json};

use super::{KIND, LinearSource, graphql, shared, str_at};

/// The most users one `users` connection asks for: Linear's connection maximum.
const USERS_PAGE: usize = 250;

/// The most `users` connections one lookup request carries, each aliased with its own
/// [`USERS_PAGE`] of names: under Linear's documented maximum of 10,000 complexity points a
/// query. Linear prices a property at 0.1, an object at 1, and multiplies a connection's children
/// by its page size, so one connection of 250 users, each an object with an `id` and a
/// `displayName`, prices at most 1 + 1 + 250 × 1.2 = 302 points — and 33 of them at 9,966.
const USERS_CONNECTIONS: usize = 33;

/// The most people one lookup request names: [`USERS_CONNECTIONS`] connections of
/// [`USERS_PAGE`] names. A call naming more people than this — and only such a call — is
/// resolved in more than one request.
pub const PEOPLE_PER_LOOKUP: usize = USERS_PAGE * USERS_CONNECTIONS;

/// The people `v` — an issue as an opted-in source's selection answers it — is assigned to:
/// its assignee's display name, or none when Linear answers `null`.
///
/// # Errors
///
/// [`SourceError::Malformed`] for an issue answered without the member, or with an assignee
/// carrying no display name.
pub(crate) fn read(v: &Value) -> Result<Vec<String>, SourceError> {
    match v.get("assignee") {
        None => Err(SourceError::Malformed {
            message: "an issue of a source reading assignees was answered without its assignee"
                .to_owned(),
        }),
        Some(Value::Null) => Ok(Vec::new()),
        Some(user) => Ok(vec![str_at(user, "displayName")?.to_owned()]),
    }
}

/// Whether `variables` — one request's — name an assignee: a write whose failure lets go of
/// every person resolved.
pub(crate) fn carried(variables: &Value) -> bool {
    variables.as_object().is_some_and(|variables| {
        variables
            .iter()
            .any(|(name, value)| name.starts_with("input") && value.get("assigneeId").is_some())
    })
}

/// `input` — an issue write's, carrying `assigneeId: null` for a task naming nobody — as a
/// create sends it: with that member left out, so Linear applies its own template or triage
/// default to a new issue as it does for any create. Kept in the input until it is sent, so a
/// later write of the issue saying the same thing reads as what the create wrote.
pub(crate) fn create_input(mut input: Value) -> Value {
    if let Some(members) = input.as_object_mut()
        && members.get("assigneeId").is_some_and(Value::is_null)
    {
        members.remove("assigneeId");
    }
    input
}

/// What a write's answer is held to of `people`: nothing for a create naming nobody, which
/// left Linear's own default in place, and otherwise exactly the people named.
pub(crate) fn asked(people: Option<&[String]>, create: bool) -> Option<&[String]> {
    people.filter(|people| !(create && people.is_empty()))
}

/// The people `writes` name, each once.
pub(crate) fn named<'a>(people: impl IntoIterator<Item = &'a [String]>) -> Vec<&'a str> {
    let mut named: Vec<&str> = Vec::new();
    for person in people.into_iter().flatten() {
        if !named.contains(&person.as_str()) {
            named.push(person);
        }
    }
    named
}

impl LinearSource {
    /// Refuse, before anything is sent, a write naming `people` as a task's assignees: on a
    /// source that has not opted in, and for more than the one person a Linear issue holds.
    pub(crate) fn assignees_writable(&self, people: Option<&[String]>) -> Result<(), SourceError> {
        let Some(people) = people else {
            return Ok(());
        };
        if !self.assignees {
            return Err(unwritable_field(KIND, "assignees"));
        }
        if people.len() > 1 {
            return Err(SourceError::Refused {
                message: format!(
                    "a Linear issue holds one assignee, and the write to source {} names {}; \
                     next: name one person, or none",
                    self.name,
                    people.len()
                ),
            });
        }
        Ok(())
    }

    /// The `assigneeId` a write sets the people `wanted` with: the one person's user id, or
    /// `null` for none.
    pub(crate) async fn assignee_id(&self, wanted: &[String]) -> Result<Value, SourceError> {
        self.assignees_writable(Some(wanted))?;
        match wanted.first() {
            None => Ok(Value::Null),
            Some(person) => Ok(json!(self.person_id(person).await?)),
        }
    }

    /// Resolve every one of `names` this process holds no id for, and this instance has not
    /// already found unknown, in one request — split only past [`PEOPLE_PER_LOOKUP`].
    ///
    /// # Errors
    ///
    /// What the request failed with, or a malformed answer — neither of which holds anything.
    pub(crate) async fn resolve_people(&self, names: &[&str]) -> Result<(), SourceError> {
        let tracker = self.tracker();
        let unresolved: Vec<String> = {
            let held = shared::people();
            let unknown = self.unknown();
            names
                .iter()
                .filter(|name| {
                    !held.contains_key(&(tracker.clone(), (**name).to_owned()))
                        && !unknown.contains(**name)
                })
                .map(|name| (*name).to_owned())
                .collect()
        };
        // llmlint: ignore[changed_behavior_has_e2e] The split past 8,250 names is reachable only by one call naming 8,251 unseen people. Measured on the loopback: an update-many of that many items over its five issues sent 16,498 requests and took 24 s, and one over 8,251 issues of their own is refused HTTP 413 by its batched read. The journeys drive this same loop with one connection and with two (252 names), and the plugin's pinned-schema test validates the 33-connection document.
        for request in unresolved.chunks(PEOPLE_PER_LOOKUP) {
            // One connection answers one page of names: one page is sent as `USERS` itself, and
            // more as that document aliased once a page.
            let pages: Vec<&[String]> = request.chunks(USERS_PAGE).collect();
            let answered: Vec<(Option<Value>, &[String])> = if let [asked] = pages.as_slice() {
                let data = self
                    .send(
                        graphql::USERS,
                        json!({"names": asked, "first": asked.len()}),
                    )
                    .await?;
                vec![(data.get("users").cloned(), *asked)]
            } else {
                let mut variables = serde_json::Map::new();
                for (at, asked) in pages.iter().enumerate() {
                    variables.insert(format!("names_{at}"), json!(asked));
                    variables.insert(format!("first_{at}"), json!(asked.len()));
                }
                let data = self
                    .send(
                        &graphql::aliased(graphql::USERS, pages.len()),
                        Value::Object(variables),
                    )
                    .await?;
                pages
                    .iter()
                    .enumerate()
                    .map(|(at, asked)| (data.get(format!("a{at}")).cloned(), *asked))
                    .collect()
            };
            for (users, asked) in answered {
                self.hold_people(users.as_ref(), asked)?;
            }
        }
        Ok(())
    }

    /// Hold what one `users` connection answered for the names `asked`: each one's id, or that
    /// Linear holds no user under it.
    fn hold_people(&self, users: Option<&Value>, asked: &[String]) -> Result<(), SourceError> {
        let tracker = self.tracker();
        let nodes = users
            .and_then(|users| users.get("nodes"))
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "missing users.nodes".to_owned(),
            })?;
        let mut found: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for node in nodes {
            let name = str_at(node, "displayName")?;
            let id = str_at(node, "id")?;
            if !asked.iter().any(|asked| asked == name) {
                continue;
            }
            if found.insert(name.to_owned(), id.to_owned()).is_some() {
                return Err(SourceError::Malformed {
                    message: format!(
                        "Linear answered two users displayed as {name:?}, a name its schema says \
                         is unique within the workspace"
                    ),
                });
            }
        }
        let mut unknown = self.unknown();
        let mut held = shared::people();
        for name in asked {
            match found.remove(name) {
                Some(id) => {
                    held.insert((tracker.clone(), name.clone()), id);
                }
                None => {
                    unknown.insert(name.clone());
                }
            }
        }
        Ok(())
    }

    /// The user id of the person displayed as `name`, resolving it when this process has not.
    ///
    /// # Errors
    ///
    /// [`SourceError::Refused`] naming `name` when Linear holds no user under it, and what the
    /// resolution failed with.
    pub(crate) async fn person_id(&self, name: &str) -> Result<String, SourceError> {
        self.resolve_people(&[name]).await?;
        if let Some(id) = shared::people().get(&(self.tracker(), name.to_owned())) {
            return Ok(id.clone());
        }
        Err(SourceError::Refused {
            message: format!(
                "Linear holds no user displayed as {name:?} in the workspace source {} reads; \
                 next: name the person by their Linear display name, exactly as Linear spells \
                 it, or leave the task unassigned",
                self.name
            ),
        })
    }

    /// Let go of every person this tracker resolved, and every name this instance found
    /// unknown, once this call ends: a write naming one failed, or answered another person. The
    /// rest of the call goes on with what its one lookup found; the next call resolves afresh.
    pub(crate) fn forget_people(&self) {
        *self
            .forget_at_end
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    }

    /// Let go of what [`Self::forget_people`] asked to, now that the call has ended.
    pub(crate) fn forget_people_at_end_of_call(&self) {
        let due = std::mem::take(
            &mut *self
                .forget_at_end
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if due {
            let tracker = self.tracker();
            shared::people().retain(|(held, _), _| *held != tracker);
            self.unknown().clear();
        }
    }

    /// Let go of the names this instance found unknown, which a person may since have added.
    pub(crate) fn forget_unknown_people(&self) {
        self.unknown().clear();
    }

    fn unknown(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<String>> {
        self.unknown_people
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Refuse a write of `id` whose own answer, `task`, does not hold exactly the people
    /// `asked` named — letting go of every person resolved, since the id sent may be stale.
    pub(crate) fn assigned_as(
        &self,
        id: &NativeId,
        asked: Option<&[String]>,
        task: &Task,
    ) -> Result<(), SourceError> {
        self.answered_as(id, asked, task.assignees.as_deref().unwrap_or_default())
    }

    /// [`Self::assigned_as`], over the people a write's answer named.
    pub(crate) fn answered_as(
        &self,
        id: &NativeId,
        asked: Option<&[String]>,
        answered: &[String],
    ) -> Result<(), SourceError> {
        let Some(asked) = asked else {
            return Ok(());
        };
        if answered == asked {
            return Ok(());
        }
        self.forget_people();
        let said = |people: &[String]| match people {
            [] => "nobody".to_owned(),
            people => people
                .iter()
                .map(|person| format!("{person:?}"))
                .collect::<Vec<_>>()
                .join(", "),
        };
        Err(SourceError::Refused {
            message: format!(
                "Linear answered the write of task {id} on source {} as assigned to {} where it \
                 was asked to assign {}; next: read the task, and set its assignee again",
                self.name,
                said(answered),
                said(asked)
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PEOPLE_PER_LOOKUP, USERS_CONNECTIONS, USERS_PAGE};

    /// The README's row for how many names one Linear lookup carries is this plugin's own
    /// limit, worked out as the README works it out.
    #[test]
    fn the_readme_states_the_lookup_limit_this_plugin_splits_at() {
        let readme = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"));
        // One connection of a page of users, each an object of two properties, priced as
        // Linear documents it: under its 10,000-point query limit at this many connections.
        let per_connection = 1 + 1 + USERS_PAGE * 12 / 10;
        assert!(per_connection * USERS_CONNECTIONS <= 10_000);
        assert!(per_connection * (USERS_CONNECTIONS + 1) > 10_000);
        let row = format!(
            "| `linear` | {},{:03} display names | {USERS_CONNECTIONS} `users` connections of \
             {USERS_PAGE} (Linear's connection maximum), each at most {per_connection} of the \
             10,000 complexity points Linear allows a query |",
            PEOPLE_PER_LOOKUP / 1000,
            PEOPLE_PER_LOOKUP % 1000
        );
        assert!(readme.contains(&row), "the README states {row:?}");
    }
}
