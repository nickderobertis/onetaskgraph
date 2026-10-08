//! Linear's private file upload flow, completed before a record is written.
//!
//! Issue descriptions and document content, including records filed into a routed member
//! project, use the same flow: [`fileUpload`](https://linear.app/developers/how-to-upload-a-file-to-linear)
//! receives the content type, filename and size and returns `uploadFile { uploadUrl, assetUrl,
//! headers { key, value } }`. A PUT sends all bytes to the signed URL with those headers,
//! Content-Type and Cache-Control. Before writing content, an authenticated GET of `assetUrl`
//! must answer 2xx; any refusal names the asset, stage and status, and a verifying refusal
//! also names the URL. Content is written once, with image targets rewritten and the trailing
//! metadata slot recording `onetaskgraph.assets` entries of `{sha256, url}` by asset name.
//!
//! A matching recorded SHA-256 reuses its URL without upload or verification. Records without
//! assets retain the existing write path and spend no asset request. Images are visible to
//! authenticated members of the workspace, as described in
//! [file storage authentication](https://linear.app/developers/file-storage-authentication).
//!
//! Rate-limited mutations (`RATELIMITED`, including HTTP 400) and HTTP 429 PUTs and GETs retry
//! after the reset hint, using the source's injected Clock. The total wait per record is
//! bounded to 60 seconds across all images and stages; a refusal beyond it names the limiter
//! and asset. The reset headers are UTC epoch milliseconds as
//! [Linear's rate-limit documentation](https://linear.app/developers/rate-limiting) specifies.

use std::time::Duration;

use onetaskgraph_plugin_api::{AssetPayload, AssetUpload, AssetUploads, AssetWrite, SourceError};
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::json;

use super::LinearSource;

/// Total rate-limit wait allowed for one record's assets, across all stages and images.
const MAX_WAIT: Duration = Duration::from_secs(60);
use super::graphql::FILE_UPLOAD;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadFile {
    upload_url: UploadUrl,
    asset_url: AssetUrl,
    headers: Vec<UploadHeader>,
}

#[derive(Deserialize)]
struct UploadHeader {
    #[serde(deserialize_with = "header_name")]
    key: reqwest::header::HeaderName,
    #[serde(deserialize_with = "header_value")]
    value: reqwest::header::HeaderValue,
}

#[derive(Clone, Copy)]
enum Stage {
    Mutation,
    Put,
    VerifyingRead,
}

impl std::fmt::Display for Stage {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::Mutation => "mutation",
            Self::Put => "PUT",
            Self::VerifyingRead => "verifying read",
        })
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
struct UploadUrl(reqwest::Url);

#[derive(Deserialize)]
#[serde(try_from = "String")]
struct AssetUrl(reqwest::Url);

fn loopback(url: &reqwest::Url) -> bool {
    url.host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback())
}

fn http_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|error| error.to_string())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback(&url)))
    {
        return Err(format!(
            "inappropriate upload URL {value:?}; use HTTPS, or an explicitly configured loopback"
        ));
    }
    Ok(url)
}

impl TryFrom<String> for UploadUrl {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        http_url(&value).map(Self)
    }
}

impl TryFrom<String> for AssetUrl {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let url = http_url(&value)?;
        if !(url.scheme() == "https"
            && url.host_str() == Some("uploads.linear.app")
            && url.port_or_known_default() == Some(443))
            && !loopback(&url)
        {
            return Err(format!(
                "untrusted Linear asset URL {value:?}; authenticated files are served by https://uploads.linear.app"
            ));
        }
        Ok(Self(url))
    }
}

impl std::fmt::Display for AssetUrl {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(out)
    }
}

fn header_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<reqwest::header::HeaderName, D::Error> {
    let value = String::deserialize(deserializer)?;
    reqwest::header::HeaderName::from_bytes(value.as_bytes()).map_err(serde::de::Error::custom)
}

fn header_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<reqwest::header::HeaderValue, D::Error> {
    let value = String::deserialize(deserializer)?;
    reqwest::header::HeaderValue::from_str(&value).map_err(serde::de::Error::custom)
}

impl LinearSource {
    pub(super) async fn upload_assets(
        &self,
        assets: &AssetWrite,
    ) -> Result<AssetUploads, SourceError> {
        let mut uploads = AssetUploads::default();
        let Some(first) = assets.assets.first() else {
            return Ok(uploads);
        };
        // A redirect is not the requested URL's 2xx answer and must never forward a key.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| failure(first, Stage::Mutation, &format!("client setup: {error}")))?;
        let mut waited = Duration::ZERO;
        for asset in &assets.assets {
            asset.checked()?;
            if let Some(url) = assets
                .recorded_assets
                .as_ref()
                .and_then(|held| held.reusable(&asset.name, &asset.sha256))
            {
                let parsed = AssetUrl::try_from(url.to_owned())
                    .map_err(|error| failure(asset, Stage::Mutation, &error))?;
                if loopback(&parsed.0) {
                    let endpoint = reqwest::Url::parse(&self.endpoint.0)
                        .map_err(|error| failure(asset, Stage::Mutation, &error.to_string()))?;
                    if !loopback(&endpoint) || endpoint.origin() != parsed.0.origin() {
                        return Err(failure(
                            asset,
                            Stage::Mutation,
                            "recorded asset URL is outside the configured loopback",
                        ));
                    }
                }
                uploads.0.insert(
                    asset.name.clone(),
                    AssetUpload {
                        sha256: asset.sha256.clone(),
                        url: url.to_owned(),
                    },
                );
                continue;
            }
            let bytes = asset
                .bytes
                .as_ref()
                .ok_or_else(|| failure(asset, Stage::Mutation, "missing bytes for a new asset"))?;
            let upload = loop {
                let response = client.post(&self.endpoint.0)
                    .header("Authorization", self.key.expose_secret())
                    .json(&json!({"query":FILE_UPLOAD,"variables":{"contentType":asset.content_type.as_str(),"filename":asset.name.as_str(),"size":bytes.len()}}))
                    .send().await.map_err(|error| failure(asset, Stage::Mutation, &error.to_string()))?;
                let status = response.status();
                let hint = reset_wait(response.headers());
                if status.as_u16() == 429 {
                    self.wait_asset(asset, Stage::Mutation, status, hint, &mut waited)
                        .await?;
                    continue;
                }
                let body: serde_json::Value = response.json().await.map_err(|error| {
                    failure(asset, Stage::Mutation, &format!("HTTP {status}: {error}"))
                })?;
                if status.as_u16() == 429
                    || body["errors"].as_array().is_some_and(|errors| {
                        errors
                            .iter()
                            .any(|error| error["extensions"]["code"] == "RATELIMITED")
                    })
                {
                    let hint = hint.or_else(|| {
                        body["errors"][0]["extensions"]["retryAfter"]
                            .as_u64()
                            .map(Duration::from_secs)
                    });
                    self.wait_asset(asset, Stage::Mutation, status, hint, &mut waited)
                        .await?;
                    continue;
                }
                if !status.is_success()
                    || body.get("errors").is_some()
                    || body["data"]["fileUpload"]["success"] != true
                {
                    return Err(failure(
                        asset,
                        Stage::Mutation,
                        &format!("HTTP {status}: {body}"),
                    ));
                }
                break serde_json::from_value::<UploadFile>(
                    body["data"]["fileUpload"]["uploadFile"].clone(),
                )
                .map_err(|error| {
                    failure(
                        asset,
                        Stage::Mutation,
                        &format!("HTTP {status}: malformed upload: {error}"),
                    )
                })?;
            };
            if loopback(&upload.asset_url.0) {
                let endpoint = reqwest::Url::parse(&self.endpoint.0)
                    .map_err(|error| failure(asset, Stage::Mutation, &error.to_string()))?;
                if !loopback(&endpoint) || endpoint.origin() != upload.asset_url.0.origin() {
                    return Err(failure(
                        asset,
                        Stage::Mutation,
                        &format!(
                            "untrusted asset URL {} is outside the configured loopback",
                            upload.asset_url
                        ),
                    ));
                }
            }
            // The upload URL is signed; never send the source credential to it.
            loop {
                let mut request = client
                    .put(upload.upload_url.0.clone())
                    .header("Content-Type", asset.content_type.as_str())
                    .header("Cache-Control", "public, max-age=31536000");
                for header in &upload.headers {
                    request = request.header(header.key.clone(), header.value.clone());
                }
                let response = request
                    .body(bytes.clone())
                    .send()
                    .await
                    .map_err(|error| failure(asset, Stage::Put, &error.to_string()))?;
                if response.status().as_u16() == 429 {
                    self.wait_asset(
                        asset,
                        Stage::Put,
                        response.status(),
                        reset_wait(response.headers()),
                        &mut waited,
                    )
                    .await?;
                    continue;
                }
                if !response.status().is_success() {
                    return Err(failure(
                        asset,
                        Stage::Put,
                        &format!("HTTP {}", response.status()),
                    ));
                }
                break;
            }
            loop {
                let response = client
                    .get(upload.asset_url.0.clone())
                    .header("Authorization", self.key.expose_secret())
                    .send()
                    .await
                    .map_err(|error| {
                        failure(
                            asset,
                            Stage::VerifyingRead,
                            &format!("{}: {error}", upload.asset_url),
                        )
                    })?;
                if response.status().as_u16() == 429 {
                    self.wait_asset(
                        asset,
                        Stage::VerifyingRead,
                        response.status(),
                        reset_wait(response.headers()),
                        &mut waited,
                    )
                    .await
                    .map_err(|error| {
                        failure(
                            asset,
                            Stage::VerifyingRead,
                            &format!("{}: {error}", upload.asset_url),
                        )
                    })?;
                    continue;
                }
                if !response.status().is_success() {
                    return Err(failure(
                        asset,
                        Stage::VerifyingRead,
                        &format!("{}: HTTP {}", upload.asset_url, response.status()),
                    ));
                }
                break;
            }
            uploads.0.insert(
                asset.name.clone(),
                AssetUpload {
                    sha256: asset.sha256.clone(),
                    url: upload.asset_url.0.to_string(),
                },
            );
        }
        Ok(uploads)
    }

    async fn wait_asset(
        &self,
        asset: &AssetPayload,
        stage: Stage,
        status: reqwest::StatusCode,
        hint: Option<Duration>,
        waited: &mut Duration,
    ) -> Result<(), SourceError> {
        let wait = hint
            .unwrap_or(Duration::from_secs(1))
            .max(Duration::from_millis(1));
        if wait > MAX_WAIT.saturating_sub(*waited) {
            return Err(failure(
                asset,
                stage,
                &format!(
                    "RATELIMITED: Linear limiter returned HTTP {status} and exceeds the 60 second total wait bound"
                ),
            ));
        }
        *waited += wait;
        self.clock.sleep(wait).await;
        Ok(())
    }
}

fn failure(asset: &AssetPayload, stage: Stage, detail: &str) -> SourceError {
    SourceError::Refused {
        message: format!("Linear asset {} {stage} failed: {detail}", asset.name),
    }
}

fn reset_wait(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let reset = [
        "x-ratelimit-endpoint-requests-reset",
        "x-ratelimit-requests-reset",
        "x-ratelimit-complexity-reset",
    ]
    .iter()
    .filter_map(|name| headers.get(*name)?.to_str().ok()?.parse::<u64>().ok())
    .max();
    reset
        .map(|reset| {
            Duration::from_millis(reset.saturating_sub(
                u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default(),
            ))
        })
        .or_else(|| {
            headers
                .get("retry-after")?
                .to_str()
                .ok()?
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
        })
}
