//! GitHub user attachments, prepared and verified before a record's body is written.

use super::*;
use onetaskgraph_plugin_api::{AssetName, AssetUpload, AssetUploads, AssetWrite};

enum AssetOperation {
    Repository,
    Upload,
    Verification,
    Read,
}

impl AssetOperation {
    fn stage(&self, url: &Url) -> String {
        match self {
            Self::Repository => "reading repository id".into(),
            Self::Upload => "upload".into(),
            Self::Verification => format!("verification of {url}"),
            Self::Read => "reading attachment".into(),
        }
    }
}

impl GitHubProjectsSource {
    /// The uploads host follows the API host for a loopback journey only. Production
    /// attachments always land on GitHub's uploads host, without another source setting.
    fn upload_endpoint(&self) -> Result<Url, SourceError> {
        if self.loopback() {
            self.endpoint.join("/user-attachments/assets")
        } else {
            Url::parse("https://uploads.github.com/user-attachments/assets")
        }
        .map_err(|error| SourceError::Config {
            message: error.to_string(),
        })
    }

    fn loopback(&self) -> bool {
        self.endpoint
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"))
    }

    fn attachment_url(&self, name: &AssetName, value: &str) -> Result<Url, SourceError> {
        let url = Url::parse(value).map_err(|error| SourceError::Refused {
            message: format!("GitHub asset {name} returned invalid URL {value:?}: {error}"),
        })?;
        let production = url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.port_or_known_default() == Some(443)
            && url.path().starts_with("/user-attachments/assets/")
            && !url.path().trim_end_matches('/').ends_with("assets");
        let fixture = self.loopback() && url.origin() == self.endpoint.origin();
        if (!production && !fixture) || !url.username().is_empty() || url.password().is_some() {
            return Err(SourceError::Refused {
                message: format!(
                    "GitHub asset {name} returned unexpected attachment URL {value:?}"
                ),
            });
        }
        Ok(url)
    }

    async fn numeric_repository(
        &self,
        repository: &RepositoryTarget,
        name: &AssetName,
    ) -> Result<std::num::NonZeroU64, SourceError> {
        let mut repositories = self.numeric_repositories.lock().await;
        if let Some(id) = repositories.get(repository).copied() {
            return Ok(id);
        }
        let mut base = self.endpoint.clone();
        base.set_path(&format!("/repos/{}/{}", repository.owner, repository.name));
        if !self.loopback() {
            base.set_host(Some("api.github.com"))
                .map_err(|error| SourceError::Config {
                    message: format!("cannot select GitHub API host: {error}"),
                })?;
        }
        base.set_query(None);
        let bytes = self
            .asset_request(self.client.get(base), name, AssetOperation::Repository)
            .await?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| SourceError::Refused {
                message: format!(
                    "GitHub repository for asset {name} returned invalid JSON: {error}"
                ),
            })?;
        let id = value["id"]
            .as_u64()
            .and_then(std::num::NonZeroU64::new)
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub repository {} for asset {name} returned no numeric id",
                    repository.slug()
                ),
            })?;
        repositories.insert(repository.clone(), id);
        Ok(id)
    }

    pub(super) async fn upload_assets(
        &self,
        repository: Option<&Repository>,
        assets: &AssetWrite,
    ) -> Result<AssetUploads, SourceError> {
        for asset in &assets.assets {
            asset.checked()?;
        }
        let mut uploads = AssetUploads::default();
        for asset in &assets.assets {
            let reused = assets
                .recorded_assets
                .as_ref()
                .and_then(|recorded| recorded.reusable(&asset.name, &asset.sha256));
            let url = if let Some(url) = reused {
                self.attachment_url(&asset.name, url)?;
                url.to_owned()
            } else {
                let bytes = asset.bytes.as_ref().ok_or_else(|| SourceError::Refused {
                    message: format!(
                        "asset {} has neither bytes nor a matching destination sha256",
                        asset.name
                    ),
                })?;
                let repository = repository.ok_or_else(|| SourceError::Refused {
                    message: format!("asset {} belongs to no issue repository; a draft cannot hold user attachments", asset.name),
                })?;
                let target = RepositoryTarget::from_origin(repository)
                    .map_err(|message| SourceError::Refused { message })?;
                let id = self.numeric_repository(&target, &asset.name).await?;
                let request = self
                    .client
                    .post(self.upload_endpoint()?)
                    .query(&[
                        ("name", asset.name.as_str().to_owned()),
                        ("content_type", asset.content_type.as_str().to_owned()),
                        ("repository_id", id.to_string()),
                    ])
                    .header(reqwest::header::CONTENT_TYPE, asset.content_type.as_str())
                    .body(bytes.clone());
                let answer = self
                    .asset_request(request, &asset.name, AssetOperation::Upload)
                    .await?;
                let value: Value =
                    serde_json::from_slice(&answer).map_err(|error| SourceError::Refused {
                        message: format!(
                            "GitHub upload of asset {} returned invalid JSON: {error}",
                            asset.name
                        ),
                    })?;
                let url = value["url"].as_str().ok_or_else(|| SourceError::Refused {
                    message: format!("GitHub upload of asset {} returned no url", asset.name),
                })?;
                let verified = self.attachment_url(&asset.name, url)?;
                self.asset_request(
                    self.client.get(verified),
                    &asset.name,
                    AssetOperation::Verification,
                )
                .await?;
                url.to_owned()
            };
            uploads.0.insert(
                asset.name.clone(),
                AssetUpload {
                    sha256: asset.sha256.clone(),
                    url,
                },
            );
        }
        Ok(uploads)
    }

    pub(super) async fn held_assets(
        &self,
        id: &NativeId,
        kind: BoardKind,
    ) -> Result<Vec<onetaskgraph_plugin_api::Asset>, SourceError> {
        let Some(item) = self.bound_item(id).await?.filter(|item| item.kind == kind) else {
            return Ok(Vec::new());
        };
        let uploads = AssetUploads::read(&item.slot)
            .map_err(|message| SourceError::Refused { message })?
            .unwrap_or_default();
        Ok(uploads
            .0
            .into_iter()
            .map(|(name, upload)| onetaskgraph_plugin_api::Asset {
                content_type: name.content_type(),
                name,
                sha256: upload.sha256,
                path: None,
            })
            .collect())
    }

    pub(super) async fn held_asset(
        &self,
        id: &NativeId,
        kind: BoardKind,
        name: &AssetName,
    ) -> Result<Option<Vec<u8>>, SourceError> {
        let Some(item) = self.bound_item(id).await?.filter(|item| item.kind == kind) else {
            return Ok(None);
        };
        let uploads = AssetUploads::read(&item.slot)
            .map_err(|message| SourceError::Refused { message })?
            .unwrap_or_default();
        let Some(upload) = uploads.0.get(name) else {
            return Ok(None);
        };
        let url = self.attachment_url(name, &upload.url)?;
        self.asset_request(self.client.get(url), name, AssetOperation::Read)
            .await
            .map(Some)
    }

    /// Both attachment stages use the GraphQL path's limiter classification, pacing and
    /// bounded backoff. Reads never take a content-creating slot.
    async fn asset_request(
        &self,
        request: reqwest::RequestBuilder,
        name: &AssetName,
        operation: AssetOperation,
    ) -> Result<Vec<u8>, SourceError> {
        let mutation = matches!(operation, AssetOperation::Upload);
        let request = request
            .bearer_auth(self.token.expose_secret())
            .build()
            .map_err(|error| SourceError::Refused {
                message: format!("GitHub asset {name} request is invalid: {error}"),
            })?;
        let stage = operation.stage(request.url());
        let (method, path) = match operation {
            AssetOperation::Repository => (accounting::Method::Get, "/repos/{owner}/{repo}"),
            AssetOperation::Upload => (accounting::Method::Post, "/user-attachments/assets"),
            AssetOperation::Verification | AssetOperation::Read => {
                (accounting::Method::Get, "/user-attachments/assets/{asset}")
            }
        };
        let endpoint =
            accounting::Endpoint::parse(method, path).expect("a literal REST endpoint template");
        let mut waited = Duration::ZERO;
        let mut waits = 0;
        let mut backoff = self.pacing.retry_backoff;
        loop {
            if mutation {
                self.clock.sleep(self.reserve_mutation_slot()).await;
            }
            let response = self
                .client
                .execute(request.try_clone().ok_or_else(|| SourceError::Refused {
                    message: format!("GitHub asset {name} {stage} cannot be retried"),
                })?)
                .await;
            if mutation {
                self.finish_mutation();
            }
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    self.ledger
                        .record(accounting::Request::rest(endpoint.clone()).finished(
                            accounting::Outcome::Refused,
                            accounting::RateLimit::default(),
                        ));
                    return Err(SourceError::Unavailable {
                        message: format!("GitHub asset {name} {stage} request failed: {error}"),
                    });
                }
            };
            let status = response.status();
            if !matches!(operation, AssetOperation::Repository) {
                self.ledger
                    .record_attachment(accounting::AttachmentResponse {
                        name: name.clone(),
                        url: request.url().clone(),
                        status,
                        operation: if mutation {
                            accounting::AttachmentOperation::Upload
                        } else {
                            accounting::AttachmentOperation::Read
                        },
                    });
            }
            let limits = accounting::RateLimit::read(|header| {
                response
                    .headers()
                    .get(header)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            });
            let exhausted = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                == Some("0");
            let hint = whole_seconds(response.headers().get("retry-after")).or_else(|| {
                exhausted
                    .then(|| whole_seconds(response.headers().get("x-ratelimit-reset")))
                    .flatten()
                    .map(|reset| reset.saturating_sub(Utc::now().timestamp().max(0).unsigned_abs()))
            });
            let bytes = match response.bytes().await {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.ledger.record(
                        accounting::Request::rest(endpoint.clone())
                            .finished(accounting::Outcome::Refused, limits),
                    );
                    return Err(SourceError::Unavailable {
                        message: format!(
                            "GitHub asset {name} {stage} response could not be read: {error}"
                        ),
                    });
                }
            };
            let body = String::from_utf8_lossy(&bytes);
            let limiter = Limiter::classify(status, exhausted, &body);
            let outcome = if limiter.is_some() {
                accounting::Outcome::RateLimited
            } else if status.is_success() {
                accounting::Outcome::Answered
            } else {
                accounting::Outcome::Refused
            };
            self.ledger
                .record(accounting::Request::rest(endpoint.clone()).finished(outcome, limits));
            if let Some(limiter) = limiter {
                let limited = Limited { limiter, hint };
                let wait = hint.map_or(Duration::from_secs(1), |hint| Duration::from_secs(hint).max(backoff));
                if wait.is_zero() || wait > self.pacing.retry_budget.saturating_sub(waited) {
                    return Err(limited.exhausted(
                        &format!("asset {name} {stage}"),
                        waits,
                        waited,
                        wait,
                        self.pacing.retry_budget,
                    ));
                }
                self.clock.sleep(wait).await;
                waited += wait;
                waits += 1;
                backoff = backoff.saturating_mul(2);
                continue;
            }
            if !status.is_success() {
                let token = if mutation {
                    "; the token type may not be accepted by the upload endpoint"
                } else {
                    ""
                };
                return Err(SourceError::Refused {
                    message: format!("GitHub asset {name} {stage} returned HTTP {status}{token}"),
                });
            }
            return Ok(bytes.to_vec());
        }
    }
}
