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
const FILE_UPLOAD: &str = "mutation AssetUpload($contentType:String!,$filename:String!,$size:Int!){fileUpload(contentType:$contentType,filename:$filename,size:$size){success uploadFile{uploadUrl assetUrl headers{key value}}}}";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadFile {
    upload_url: String,
    asset_url: String,
    headers: Vec<UploadHeader>,
}

#[derive(Deserialize)]
struct UploadHeader {
    key: String,
    value: String,
}

impl LinearSource {
    pub(super) async fn upload_assets(
        &self,
        assets: &AssetWrite,
    ) -> Result<AssetUploads, SourceError> {
        let mut uploads = AssetUploads::default();
        let mut waited = Duration::ZERO;
        for asset in &assets.assets {
            asset.checked()?;
            if let Some(url) = assets
                .recorded_assets
                .as_ref()
                .and_then(|held| held.reusable(&asset.name, &asset.sha256))
            {
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
                .ok_or_else(|| failure(asset, "mutation", "missing bytes for a new asset"))?;
            let upload = loop {
                let response = self.client.post(&self.endpoint.0)
                    .header("Authorization", self.key.expose_secret())
                    .json(&json!({"query":FILE_UPLOAD,"variables":{"contentType":asset.content_type.as_str(),"filename":asset.name.as_str(),"size":bytes.len()}}))
                    .send().await.map_err(|error| failure(asset, "mutation", &error.to_string()))?;
                let status = response.status();
                let hint = reset_wait(response.headers());
                if status.as_u16() == 429 {
                    self.wait_asset(asset, "mutation", hint, &mut waited)
                        .await?;
                    continue;
                }
                let body: serde_json::Value = response.json().await.map_err(|error| {
                    failure(asset, "mutation", &format!("HTTP {status}: {error}"))
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
                    self.wait_asset(asset, "mutation", hint, &mut waited)
                        .await?;
                    continue;
                }
                if !status.is_success()
                    || body.get("errors").is_some()
                    || body["data"]["fileUpload"]["success"] != true
                {
                    return Err(failure(
                        asset,
                        "mutation",
                        &format!("HTTP {status}: {body}"),
                    ));
                }
                break serde_json::from_value::<UploadFile>(
                    body["data"]["fileUpload"]["uploadFile"].clone(),
                )
                .map_err(|error| {
                    failure(
                        asset,
                        "mutation",
                        &format!("HTTP {status}: malformed upload: {error}"),
                    )
                })?;
            };
            // The upload URL is signed; never send the source credential to it.
            loop {
                let mut request = self
                    .client
                    .put(&upload.upload_url)
                    .header("Content-Type", asset.content_type.as_str())
                    .header("Cache-Control", "public, max-age=31536000");
                for header in &upload.headers {
                    request = request.header(&header.key, &header.value);
                }
                let response = request
                    .body(bytes.clone())
                    .send()
                    .await
                    .map_err(|error| failure(asset, "PUT", &error.to_string()))?;
                if response.status().as_u16() == 429 {
                    self.wait_asset(asset, "PUT", reset_wait(response.headers()), &mut waited)
                        .await?;
                    continue;
                }
                if !response.status().is_success() {
                    return Err(failure(
                        asset,
                        "PUT",
                        &format!("HTTP {}", response.status()),
                    ));
                }
                break;
            }
            loop {
                let response = self
                    .client
                    .get(&upload.asset_url)
                    .header("Authorization", self.key.expose_secret())
                    .send()
                    .await
                    .map_err(|error| {
                        failure(
                            asset,
                            "verifying read",
                            &format!("{}: {error}", upload.asset_url),
                        )
                    })?;
                if response.status().as_u16() == 429 {
                    self.wait_asset(
                        asset,
                        "verifying read",
                        reset_wait(response.headers()),
                        &mut waited,
                    )
                    .await?;
                    continue;
                }
                if !response.status().is_success() {
                    return Err(failure(
                        asset,
                        "verifying read",
                        &format!("{}: HTTP {}", upload.asset_url, response.status()),
                    ));
                }
                break;
            }
            uploads.0.insert(
                asset.name.clone(),
                AssetUpload {
                    sha256: asset.sha256.clone(),
                    url: upload.asset_url,
                },
            );
        }
        Ok(uploads)
    }

    async fn wait_asset(
        &self,
        asset: &AssetPayload,
        stage: &str,
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
                "RATELIMITED (HTTP 429): Linear limiter exceeds the 60 second total wait bound",
            ));
        }
        *waited += wait;
        self.clock.sleep(wait).await;
        Ok(())
    }
}

fn failure(asset: &AssetPayload, stage: &str, status: &str) -> SourceError {
    SourceError::Refused {
        message: format!("Linear asset {} {stage} failed: {status}", asset.name),
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
