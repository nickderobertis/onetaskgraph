//! A task's assignees on a GitHub board, for a source opted in to them (`config.assignees:
//! true`).
//!
//! **Read** on the reads this source already makes: [`graphql::with_assignees`] selects an
//! issue's `assignees` wherever a document reads one, so an opted-in read costs the requests it
//! did before, and a source that has not opted in sends every document as it was.
//!
//! **Written** as one person or none, by login, replacing whoever the issue holds: in the
//! `assigneeIds` of the `createIssue` or `updateIssue` the write already sends. GitHub takes node
//! ids there, so each login is looked up first — every login a call names that this process has
//! not resolved, in one request of [`graphql::USER_IDS`] sized to them ([`graphql::user_ids`]),
//! split only past GitHub's own node limit — and kept for the rest of the process. A write that
//! fails, or is answered with other people, has the people it named forgotten when its call ends:
//! the rest of the call does not look them up again, and the next call does. GitHub silently
//! leaves off a person who cannot
//! be assigned to the issue's repository, so the success of the call is never taken as the
//! answer: each write's own answer selects the people the issue holds afterwards, and an answer
//! that differs from what was asked refuses that item, naming the person.

use std::collections::BTreeMap;
use std::sync::MutexGuard;

use onetaskgraph_plugin_api::SourceError;
use serde_json::{Map, Value, json};

use super::{ANSWER_ERRORS, GitHubProjectsSource, PEOPLE_PER_LOOKUP, graphql, required_str};

/// Every login an issue's — or a draft's — `assignees` connection holds, in the order GitHub
/// answered them, read by a source opted in to assignees: every document such a source sends
/// selects the connection wherever it reads an issue or a draft
/// ([`graphql::with_assignees`]), so an answer without it is refused rather than read as
/// holding nobody.
pub(super) fn assignee_logins(content: &Value) -> Result<Vec<String>, SourceError> {
    let id = content
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("an issue");
    let Some(connection) = content.get("assignees") else {
        return Err(SourceError::Malformed {
            message: format!(
                "GitHub answered {id} without the assignees this source asked for beside it"
            ),
        });
    };
    let nodes = connection
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub issue assignees.nodes is not an array".into(),
        })?;
    nodes
        .iter()
        .map(|node| {
            node.get("login")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub issue assignee has no login".into(),
                })
        })
        .collect::<Result<Vec<_>, _>>()
}

/// Whether two lists of people name the same ones, whatever their order or the case of their
/// logins — GitHub compares logins without regard to case.
pub(super) fn same_people(left: &[String], right: &[String]) -> bool {
    let set = |people: &[String]| {
        people
            .iter()
            .map(|login| login.to_lowercase())
            .collect::<std::collections::BTreeSet<_>>()
    };
    set(left) == set(right)
}

/// Whether `data` carries a `NOT_FOUND` error about `alias`: GitHub's answer to a login it does
/// not know.
fn not_found_at(data: &Value, alias: &str) -> bool {
    data.get(ANSWER_ERRORS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|error| {
            error.get("type").and_then(Value::as_str) == Some("NOT_FOUND")
                && error
                    .get("path")
                    .and_then(Value::as_array)
                    .and_then(|path| path.first())
                    .and_then(Value::as_str)
                    == Some(alias)
        })
}

/// What a refusal calls the issue `issue` answers for: its address, else its number, else its
/// node id.
fn named(issue: Option<&Value>, fallback: &str) -> String {
    let field = |key: &str| issue.and_then(|issue| issue.get(key));
    if let Some(url) = field("url").and_then(Value::as_str) {
        return url.to_owned();
    }
    if let Some(number) = field("number").and_then(Value::as_u64) {
        return format!("issue #{number}");
    }
    match field("id").and_then(Value::as_str) {
        Some(id) => format!("issue {id}"),
        None => fallback.to_owned(),
    }
}

impl super::Resolved {
    /// What a refusal of an assignee write calls this item: its address, else its id.
    pub(super) fn written_name(&self) -> String {
        self.url
            .clone()
            .unwrap_or_else(|| format!("item {}", self.id.0))
    }
}

impl GitHubProjectsSource {
    /// The refusal of an assignee written to a board draft, which this source writes as an
    /// issue's alone.
    pub(super) fn assigns_a_draft(&self) -> SourceError {
        SourceError::Refused {
            message: format!(
                "source {} writes the assignees of an issue, and this task is a GitHub draft                  item; next: convert the draft to an issue, then assign it",
                self.name
            ),
        }
    }

    /// Put the node ids of the people `incoming` assigns a new issue to into `input`, its
    /// `createIssue` input — nothing at all for an issue created unassigned, which is how
    /// GitHub creates one.
    pub(super) fn assigning(
        &self,
        input: &mut Value,
        incoming: &super::Incoming<'_>,
    ) -> Result<(), SourceError> {
        if let Some(people) = incoming.assignees.filter(|people| !people.is_empty()) {
            input["assigneeIds"] =
                json!(self.people_ids(people, &format!("the new {}", incoming.title))?);
        }
        Ok(())
    }

    /// The people this process has looked up, by lowercased login.
    fn people(&self) -> MutexGuard<'_, BTreeMap<String, Option<String>>> {
        // A poisoned lock costs a lookup at most: what it holds is GitHub's answer, never a
        // decision of this process.
        self.people
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Look up every login of `logins` this process has not resolved yet in one
    /// [`graphql::USER_IDS`] request sized to them — split only past [`PEOPLE_PER_LOOKUP`],
    /// GitHub's own limit — and none at all when every one is resolved. A login GitHub does not
    /// know is kept as such, and refused by [`Self::people_ids`] for the write naming it alone.
    pub(super) async fn resolve_people<'a>(
        &self,
        logins: impl IntoIterator<Item = &'a String>,
    ) -> Result<(), SourceError> {
        let mut wanted: Vec<&String> = Vec::new();
        {
            let held = self.people();
            for login in logins {
                if !held.contains_key(&login.to_lowercase())
                    && !wanted.iter().any(|named| named.eq_ignore_ascii_case(login))
                {
                    wanted.push(login);
                }
            }
        }
        // llmlint: ignore[changed_behavior_has_e2e] A second lookup happens only for one call naming more than 500,000 unseen logins, GitHub's own node limit, which no journey can drive. The journeys drive this same loop for 22 logins in one request, and the schema test validates a lookup sized to any number of people.
        for chunk in wanted.chunks(PEOPLE_PER_LOOKUP) {
            let mut variables = Map::new();
            for (slot, login) in chunk.iter().enumerate() {
                variables.insert(format!("login{slot}"), json!(login));
            }
            let data = self
                .graphql(graphql::USER_IDS, Value::Object(variables))
                .await?;
            let mut found = Vec::with_capacity(chunk.len());
            for (slot, login) in chunk.iter().enumerate() {
                let alias = format!("u{slot}");
                match data.get(&alias).filter(|user| !user.is_null()) {
                    Some(user) => {
                        // The id is held for the person asked for only when the answer is that
                        // person: one naming somebody else would assign them in their place.
                        let answered = required_str(user, "login")?;
                        if !answered.eq_ignore_ascii_case(login) {
                            return Err(SourceError::Malformed {
                                message: format!(
                                    "GitHub answered the lookup of the login {login} with the \
                                     user {answered}"
                                ),
                            });
                        }
                        found.push((
                            login.to_lowercase(),
                            Some(required_str(user, "id")?.to_owned()),
                        ));
                        self.spellings
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(login.to_lowercase(), answered.to_owned());
                    }
                    None if not_found_at(&data, &alias) => found.push((login.to_lowercase(), None)),
                    None => {
                        return Err(SourceError::Malformed {
                            message: format!(
                                "GitHub answered nothing for the login {login} while looking up \
                                 the people an assignee write names"
                            ),
                        });
                    }
                }
            }
            self.people().extend(found);
        }
        Ok(())
    }

    /// The node ids of `logins`, each already looked up by [`Self::resolve_people`]: a login
    /// GitHub does not know refuses the write of `item`, naming the person.
    pub(super) fn people_ids(
        &self,
        logins: &[String],
        item: &str,
    ) -> Result<Vec<String>, SourceError> {
        let held = self.people();
        logins
            .iter()
            .map(|login| match held.get(&login.to_lowercase()) {
                Some(Some(id)) => Ok(id.clone()),
                Some(None) => Err(SourceError::Refused {
                    message: format!(
                        "GitHub knows no user {login}, so {item} cannot be assigned to them; \
                         next: check the login's spelling, or name someone GitHub knows"
                    ),
                }),
                None => Err(SourceError::Malformed {
                    message: format!(
                        "{login} was not looked up before the write of {item} named them"
                    ),
                }),
            })
            .collect()
    }

    /// The login of the person whose node id is `id`, as this process looked them up — or the id
    /// itself, for one it never did.
    /// `people` as GitHub spells each of them: its own login for every person this process
    /// looked up, and as given otherwise.
    pub(super) fn spelled(&self, people: &[String]) -> Vec<String> {
        let spellings = self
            .spellings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        people
            .iter()
            .map(|person| {
                spellings
                    .get(&person.to_lowercase())
                    .cloned()
                    .unwrap_or_else(|| person.clone())
            })
            .collect()
    }

    fn login_of(&self, id: &str) -> String {
        self.people()
            .iter()
            .find(|(_, held)| held.as_deref() == Some(id))
            .map_or_else(|| id.to_owned(), |(login, _)| login.clone())
    }

    /// The logins of every person `input`'s `assigneeIds` names.
    fn named_in(&self, input: &Value) -> Vec<String> {
        input
            .get("assigneeIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|id| self.login_of(id))
            .collect()
    }

    /// Mark every person `input`'s `assigneeIds` names to be forgotten when this call ends,
    /// after a write naming them failed or was answered with other people: the rest of this
    /// call does not look them up again, and the next call naming them does.
    pub(super) fn forget_people(&self, input: &Value) {
        let Some(ids) = input.get("assigneeIds").and_then(Value::as_array) else {
            return;
        };
        self.forgotten
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(ids.iter().filter_map(Value::as_str).map(str::to_owned));
    }

    /// Forget every person a failed or mismatched write of the call now ending named, so the
    /// next call naming them looks them up again.
    pub(super) fn forget_at_end_of_call(&self) {
        let ids = std::mem::take(
            &mut *self
                .forgotten
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if ids.is_empty() {
            return;
        }
        self.people().retain(|_, held| {
            !held
                .as_deref()
                .is_some_and(|held| ids.iter().any(|id| id == held))
        });
    }

    /// `error`, a refusal of a write sending `input`, with the people that write assigned named
    /// beside it — and those people forgotten — when it assigned any.
    pub(super) fn refusing_people(&self, input: &Value, error: SourceError) -> SourceError {
        let people = self.named_in(input);
        if input.get("assigneeIds").is_none() {
            return error;
        }
        self.forget_people(input);
        if people.is_empty() {
            return error;
        }
        super::noting(
            error,
            &format!(
                "; this write assigned {}, so it is not assigned to them",
                people.join(", ")
            ),
        )
    }

    /// Hold `issue`, the answer to a write sending `input`, to assigning exactly the people that
    /// input's `assigneeIds` names: GitHub leaves a person who cannot be assigned off without an
    /// error, so a call's success says nothing about whom it assigned. Nothing to hold when the
    /// write named no assignees.
    pub(super) fn assigned_as_asked(
        &self,
        input: &Value,
        issue: Option<&Value>,
    ) -> Result<(), SourceError> {
        let Some(asked) = input.get("assigneeIds").and_then(Value::as_array) else {
            return Ok(());
        };
        let asked: Vec<&str> = asked.iter().filter_map(Value::as_str).collect();
        let what = named(issue, "the issue written");
        let Some(holds) = issue
            .and_then(|issue| issue.pointer("/assignees/nodes"))
            .and_then(Value::as_array)
        else {
            self.forget_people(input);
            return Err(SourceError::Malformed {
                message: format!(
                    "GitHub answered the assignee write of {what} without the people it left \
                     the issue assigned to"
                ),
            });
        };
        // Every node is a person with an id and a login; one that is not is no answer to hold
        // the write to — a malformed connection must not pass for one holding nobody.
        let holds: Vec<(&str, &str)> = match holds
            .iter()
            .map(|node| Some((node.get("id")?.as_str()?, node.get("login")?.as_str()?)))
            .collect::<Option<Vec<_>>>()
        {
            Some(holds) => holds,
            None => {
                self.forget_people(input);
                return Err(SourceError::Malformed {
                    message: format!(
                        "GitHub answered the assignee write of {what} with an assignee that has \
                         no id or no login"
                    ),
                });
            }
        };
        let left_off: Vec<String> = asked
            .iter()
            .filter(|id| !holds.iter().any(|(held, _)| held == *id))
            .map(|id| self.login_of(id))
            .collect();
        if !left_off.is_empty() {
            self.forget_people(input);
            let people = left_off.join(", ");
            return Err(SourceError::Refused {
                message: format!(
                    "GitHub left {people} off {what}: it assigns only people who can be assigned \
                     to issues in that repository, and leaves anyone else off without an error; \
                     next: give {people} access to the repository, or name someone who has it"
                ),
            });
        }
        let kept: Vec<&str> = holds
            .iter()
            .filter(|(held, _)| !asked.contains(held))
            .map(|(_, login)| *login)
            .collect();
        if !kept.is_empty() {
            self.forget_people(input);
            return Err(SourceError::Refused {
                message: format!(
                    "GitHub kept {} on {what}, whom this write did not name, so the issue is not \
                     assigned as asked; next: run the write again",
                    kept.join(", ")
                ),
            });
        }
        Ok(())
    }
}
