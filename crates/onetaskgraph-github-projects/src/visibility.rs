//! Who can read what this source writes: its board, and the repository an issue lives in.
//!
//! A GitHub board is readable by anybody when its Project is public, and an issue on it is
//! readable by anybody when the repository it was created in is public — so this source is
//! private for a write only when **both** are private. Each is one read, with this source's
//! own credential and never another: the Project's `public` field over GraphQL, and the
//! repository over REST. The two are sent together, so the time they add is one read's
//! rather than two, and neither is held for a later write: a board or a repository a person
//! makes public between two writes is read as public at the second.
//!
//! Reading a Project's `public` field needs the `read:project` scope. A credential without
//! it cannot answer, and the read says so, naming the scope, rather than answering from
//! anything else — an unread visibility is never guessed.

use onetaskgraph_plugin_api::{SourceError, Visibility, WriteTarget};
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use crate::{GitHubProjectsSource, RepositoryTarget, accounting, graphql};

impl GitHubProjectsSource {
    /// Who can read where `target` lands: private only when the board and the issue's
    /// repository both are.
    pub(crate) async fn write_visibility(
        &self,
        target: &WriteTarget<'_>,
    ) -> Result<Visibility, SourceError> {
        let repository = self.visibility_repository(target).await?;
        let (board, repository) = tokio::join!(self.board_visibility(), async {
            match &repository {
                Some(repository) => self.repository_visibility(repository).await,
                // A draft is filed in no repository, so the board alone decides who reads it.
                None => Ok(Visibility::Private),
            }
        });
        Ok(match (board?, repository?) {
            (Visibility::Private, Visibility::Private) => Visibility::Private,
            (Visibility::Unknown, _) | (_, Visibility::Unknown) => Visibility::Unknown,
            _ => Visibility::Public,
        })
    }

    /// The repository an issue `target` names lives in, or would be created in: an existing
    /// issue's own, and for a new one the rule creating it applies — its one named
    /// repository, else its project's, else the configured one. `None` for a draft.
    async fn visibility_repository(
        &self,
        target: &WriteTarget<'_>,
    ) -> Result<Option<RepositoryTarget>, SourceError> {
        let own = |item: Option<crate::Resolved>| {
            item.and_then(|item| item.own_repository)
                .map(|origin| origin_target(&origin))
                .transpose()
        };
        match target {
            WriteTarget::Existing(id) => own(self.bound_item(id).await?),
            WriteTarget::New {
                repositories,
                project,
            } => {
                if let [named] = repositories {
                    return origin_target(named).map(Some);
                }
                if let Some(project) = project
                    && let Some(repository) = own(self.bound_item(project).await?)?
                {
                    return Ok(Some(repository));
                }
                self.configured_repository().cloned().map(Some)
            }
        }
    }

    /// Whether the board's Project is public, read with this source's own credential.
    async fn board_visibility(&self) -> Result<Visibility, SourceError> {
        let answer = self
            .graphql(
                graphql::PROJECT_VISIBILITY,
                json!({"owner": self.owner, "number": self.project_number}),
            )
            .await
            .map_err(|error| match error {
                // Only GitHub's refusal of the scope is that scope's: a credential it rejects
                // outright, or one missing another grant, keeps its own diagnosis.
                SourceError::Auth { message } if message.contains("read:project") => {
                    SourceError::Auth {
                        message: format!(
                            "source {} cannot read whether its project is public: the credential \
                         in {} lacks the `read:project` scope that read needs; next: grant that \
                         scope to that credential",
                            self.name, self.credential_name
                        ),
                    }
                }
                other => other,
            })?;
        match answer.pointer("/visibility/projectV2/public") {
            Some(Value::Bool(true)) => Ok(Visibility::Public),
            Some(Value::Bool(false)) => Ok(Visibility::Private),
            _ => Err(SourceError::Malformed {
                message: format!(
                    "source {} read no `public` for project {} of {}",
                    self.name, self.project_number, self.owner
                ),
            }),
        }
    }

    /// Whether `repository` is public, read over REST with this source's own credential.
    ///
    /// GitHub answers `public`, `private` or `internal`; only `public` is public. A repository
    /// the credential cannot see is a failed read, never a guess.
    async fn repository_visibility(
        &self,
        repository: &RepositoryTarget,
    ) -> Result<Visibility, SourceError> {
        let mut url = self.endpoint.clone();
        url.set_path(&format!("/repos/{}/{}", repository.owner, repository.name));
        url.set_query(None);
        if !self.loopback() {
            url.set_host(Some("api.github.com"))
                .map_err(|error| SourceError::Config {
                    message: format!("cannot select GitHub API host: {error}"),
                })?;
        }
        let endpoint =
            accounting::Endpoint::parse(accounting::Method::Get, "/repos/{owner}/{repo}")
                .expect("a literal REST endpoint template");
        let unreadable = |why: String| SourceError::Unavailable {
            message: format!(
                "source {} could not read whether repository {} is public: {why}",
                self.name,
                repository.slug()
            ),
        };
        let response = self
            .client
            .get(url)
            .bearer_auth(self.token.expose_secret())
            .header("accept", "application/vnd.github+json")
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.ledger
                    .record(accounting::Request::rest(endpoint).finished(
                        accounting::Outcome::Refused,
                        accounting::RateLimit::default(),
                    ));
                return Err(unreadable(error.to_string()));
            }
        };
        let status = response.status();
        let limits = accounting::RateLimit::read(|header| {
            response
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        let body = response.bytes().await;
        self.ledger
            .record(accounting::Request::rest(endpoint).finished(
                if status.is_success() {
                    accounting::Outcome::Answered
                } else {
                    accounting::Outcome::Refused
                },
                limits,
            ));
        let body = body.map_err(|error| unreadable(error.to_string()))?;
        if !status.is_success() {
            return Err(unreadable(format!("GitHub answered HTTP {status}")));
        }
        let value: Value =
            serde_json::from_slice(&body).map_err(|error| unreadable(error.to_string()))?;
        // `visibility` is the answer; the older `private` flag is read only where it is absent.
        // A field present in any other shape is a malformed answer, never a fallback.
        match (value.get("visibility"), value.get("private")) {
            (Some(Value::String(named)), _) => match named.as_str() {
                "public" => Ok(Visibility::Public),
                "private" | "internal" => Ok(Visibility::Private),
                other => Err(unreadable(format!(
                    "its answer named the visibility {other:?}, which is none of public, private \
                     and internal"
                ))),
            },
            (None, Some(Value::Bool(false))) => Ok(Visibility::Public),
            (None, Some(Value::Bool(true))) => Ok(Visibility::Private),
            _ => Err(unreadable("its answer named no visibility".to_owned())),
        }
    }
}

/// The repository `origin` names, refused as this source refuses one it cannot create in.
fn origin_target(
    origin: &onetaskgraph_plugin_api::Repository,
) -> Result<RepositoryTarget, SourceError> {
    RepositoryTarget::from_origin(origin).map_err(|message| SourceError::Refused { message })
}
