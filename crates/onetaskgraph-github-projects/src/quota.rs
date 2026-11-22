//! What GitHub says is left of each rate-limit window this source's token is metered by.
//!
//! [`TaskSource::quota`](onetaskgraph_plugin_api::TaskSource::quota) is one `GET /rate_limit`
//! at the source's configured endpoint — GitHub's, or a loopback board's — reporting every
//! resource the answer names, in the order GitHub gives them. GitHub documents that endpoint
//! as not counting against the REST allowance, so the read is not counted in
//! [`metering`](onetaskgraph_plugin_api::TaskSource::metering)'s spend either.
//!
//! **Each response's own reading too**, for a caller's [`RequestGate`]: every request this
//! source sends carries `x-ratelimit-*` headers about the one window it drew on, which
//! [`accounting::RateLimit`] already reads, and [`header_readings`] turns into the reading the
//! gate is told. A token may be metered by two windows of one resource at once — measured on
//! the real board, interleaved, each with its own reset — so each reading is reported as its
//! own response gave it and never merged with `GET /rate_limit`'s.

use chrono::DateTime;
use onetaskgraph_plugin_api::{GatedRequest, QuotaReading, RequestGate, SourceError};
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::{GitHubProjectsSource, accounting};

/// The reading one response's `x-ratelimit-*` headers carried, or none when they did not name
/// the whole of one window — its resource, allowance, what was left and used, and its reset.
pub(crate) fn header_readings(limits: &accounting::RateLimit) -> Vec<QuotaReading> {
    match (
        limits.resource(),
        limits.limit(),
        limits.remaining(),
        limits.used_by_the_account(),
        limits.reset().and_then(|reset| i64::try_from(reset).ok()),
    ) {
        (Some(resource), Some(limit), Some(remaining), Some(used), Some(reset)) => {
            DateTime::from_timestamp(reset, 0)
                .map(|resets_at| QuotaReading {
                    resource: resource.to_owned(),
                    limit,
                    remaining,
                    used,
                    resets_at,
                })
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The reading `GET /rate_limit` gives one resource, `name`, or why it is not one.
fn resource_reading(name: &str, value: &Value) -> Result<QuotaReading, String> {
    let number = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("resource {name:?} has no whole-number {field}"))
    };
    let reset = number("reset")?;
    let resets_at = i64::try_from(reset)
        .ok()
        .and_then(|reset| DateTime::from_timestamp(reset, 0))
        .ok_or_else(|| format!("resource {name:?} resets at {reset}, which is no instant"))?;
    Ok(QuotaReading {
        resource: name.to_owned(),
        limit: number("limit")?,
        remaining: number("remaining")?,
        used: number("used")?,
        resets_at,
    })
}

/// Every resource a `GET /rate_limit` answer names, in its order, or why it does not read as
/// one.
pub(crate) fn rate_limit_readings(answer: &Value) -> Result<Vec<QuotaReading>, String> {
    let resources = answer
        .get("resources")
        .and_then(Value::as_object)
        .ok_or("the answer has no resources object")?;
    resources
        .iter()
        .map(|(name, value)| resource_reading(name, value))
        .collect()
}

impl GitHubProjectsSource {
    /// One `GET /rate_limit`, every resource it reports read as a [`QuotaReading`].
    pub(crate) async fn rate_limits(&self) -> Result<Vec<QuotaReading>, SourceError> {
        let mut url = self.endpoint.clone();
        url.set_path("/rate_limit");
        url.set_query(None);
        if !self.loopback() {
            url.set_host(Some("api.github.com"))
                .map_err(|error| SourceError::Config {
                    message: format!("cannot select GitHub API host: {error}"),
                })?;
        }
        let unreadable = |why: String| SourceError::Unavailable {
            message: format!(
                "source {} could not read its token's rate limits from GET /rate_limit: {why}",
                self.name
            ),
        };
        // Asked like every other request this source sends, at no cost: GitHub documents the
        // endpoint as not counting against the REST allowance.
        <dyn RequestGate>::before(&GatedRequest {
            resource: "core".to_owned(),
            content_creating: false,
            cost_hint: Some(0),
        })
        .await?;
        let response = self
            .client()?
            .get(url)
            .bearer_auth(self.token.expose_secret())
            .header("accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| unreadable(crate::transport_error(&error)))?;
        let status = response.status();
        let limits = accounting::RateLimit::read(|header| {
            response
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        <dyn RequestGate>::after(&header_readings(&limits));
        let body = response
            .bytes()
            .await
            .map_err(|error| unreadable(error.to_string()))?;
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SourceError::Auth {
                message: format!(
                    "GitHub rejected source {}'s credential reading GET /rate_limit with HTTP \
                     {status}",
                    self.name
                ),
            });
        }
        if !status.is_success() {
            return Err(unreadable(format!("GitHub answered HTTP {status}")));
        }
        let answer: Value =
            serde_json::from_slice(&body).map_err(|error| unreadable(error.to_string()))?;
        rate_limit_readings(&answer).map_err(|why| SourceError::Malformed {
            message: format!(
                "source {}'s GET /rate_limit answer does not read as rate limits: {why}",
                self.name
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_resource_of_the_answer_is_read_in_its_order() {
        let answer = json!({"resources": {
            "core": {"limit": 5000, "used": 12, "remaining": 4988, "reset": 1_795_306_927},
            "graphql": {"limit": 5000, "used": 411, "remaining": 4589, "reset": 1_795_309_716},
            "search": {"limit": 30, "used": 0, "remaining": 30, "reset": 1_795_306_000},
        }, "rate": {"limit": 5000, "used": 12, "remaining": 4988, "reset": 1_795_306_927}});
        let read = rate_limit_readings(&answer).unwrap();
        assert_eq!(
            read.iter()
                .map(|reading| reading.resource.as_str())
                .collect::<Vec<_>>(),
            ["core", "graphql", "search"]
        );
        assert_eq!(read[1].remaining, 4589);
        assert_eq!(read[1].used, 411);
        assert_eq!(read[1].resets_at.timestamp(), 1_795_309_716);
    }

    #[test]
    fn an_answer_missing_a_figure_is_refused_naming_it() {
        let answer = json!({"resources": {"core": {"limit": 5000, "used": 1, "reset": 1}}});
        assert_eq!(
            rate_limit_readings(&answer).unwrap_err(),
            "resource \"core\" has no whole-number remaining"
        );
        assert!(rate_limit_readings(&json!({})).is_err());
    }

    #[test]
    fn a_response_s_headers_read_as_one_window_only_when_whole() {
        let headers = |pairs: &[(&str, &str)]| {
            let pairs: Vec<(String, String)> = pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect();
            accounting::RateLimit::read(move |name| {
                pairs
                    .iter()
                    .find(|(held, _)| held == name)
                    .map(|(_, value)| value.clone())
            })
        };
        let whole = headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "4589"),
            ("x-ratelimit-used", "411"),
            ("x-ratelimit-reset", "1795306927"),
            ("x-ratelimit-resource", "graphql"),
        ]);
        assert_eq!(
            header_readings(&whole),
            [QuotaReading {
                resource: "graphql".into(),
                limit: 5000,
                remaining: 4589,
                used: 411,
                resets_at: DateTime::from_timestamp(1_795_306_927, 0).unwrap(),
            }]
        );
        let partial = headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-resource", "core"),
        ]);
        assert!(header_readings(&partial).is_empty());
    }
}
