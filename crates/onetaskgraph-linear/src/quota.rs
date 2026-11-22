//! What Linear's last response said is left of each rate-limit window this source's key is
//! metered by.
//!
//! Linear carries its allowance on every response, in the `x-ratelimit-*` headers its
//! rate-limiting documentation names — one set for requests, one for complexity, and one for
//! an endpoint metered on its own — each a limit, what remains of it, and when it resets in
//! epoch milliseconds. It sends no figure for what was used, which is the limit less what
//! remains. [`TaskSource::quota`](onetaskgraph_plugin_api::TaskSource::quota) answers the
//! readings of the last response this source received, and nothing before its first: it
//! makes no request of its own, so asking costs Linear nothing.

use chrono::DateTime;
use onetaskgraph_plugin_api::QuotaReading;

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] https://linear.app/developers/rate-limiting documents these headers and epoch milliseconds, but Linear publishes no machine-readable definition of them; `reset_wait` beside this reads the same three resets, and the loopback quota journey in `onetaskgraph-linear-e2e` holds what a response carrying them answers.
/// Each window Linear names on a response, by the resource it is reported under and the
/// prefix of its three headers.
const WINDOWS: [(&str, &str); 3] = [
    ("requests", "x-ratelimit-requests"),
    ("complexity", "x-ratelimit-complexity"),
    ("endpoint-requests", "x-ratelimit-endpoint-requests"),
];
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

/// Every window one response's headers name whole — a limit, what remains and a reset — in
/// [`WINDOWS`]' order; a window missing any of the three, or naming more remaining than its
/// limit, is not reported.
pub(crate) fn header_readings(headers: &reqwest::header::HeaderMap) -> Vec<QuotaReading> {
    let number = |name: String| {
        headers
            .get(name.as_str())?
            .to_str()
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
    };
    WINDOWS
        .iter()
        .filter_map(|(resource, prefix)| {
            let limit = number(format!("{prefix}-limit"))?;
            let remaining = number(format!("{prefix}-remaining"))?;
            let reset = i64::try_from(number(format!("{prefix}-reset"))?).ok()?;
            if remaining > limit {
                return None;
            }
            Some(QuotaReading {
                resource: (*resource).to_owned(),
                limit,
                remaining,
                used: limit - remaining,
                resets_at: DateTime::from_timestamp_millis(reset)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn each_window_a_response_names_whole_is_read_with_what_was_used() {
        let read = header_readings(&headers(&[
            ("x-ratelimit-requests-limit", "1500"),
            ("x-ratelimit-requests-remaining", "1490"),
            ("x-ratelimit-requests-reset", "1795306927000"),
            ("x-ratelimit-complexity-limit", "250000"),
            ("x-ratelimit-complexity-remaining", "249000"),
            ("x-ratelimit-complexity-reset", "1795306927500"),
        ]));
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].resource, "requests");
        assert_eq!(read[0].used, 10);
        assert_eq!(read[0].resets_at.timestamp(), 1_795_306_927);
        assert_eq!(read[1].resource, "complexity");
        assert_eq!(read[1].used, 1000);
    }

    #[test]
    fn a_window_missing_a_figure_or_over_its_limit_is_not_reported() {
        assert!(
            header_readings(&headers(&[
                ("x-ratelimit-requests-limit", "1500"),
                ("x-ratelimit-requests-reset", "1795306927000"),
            ]))
            .is_empty()
        );
        assert!(
            header_readings(&headers(&[
                ("x-ratelimit-requests-limit", "10"),
                ("x-ratelimit-requests-remaining", "11"),
                ("x-ratelimit-requests-reset", "1795306927000"),
            ]))
            .is_empty()
        );
    }
}
