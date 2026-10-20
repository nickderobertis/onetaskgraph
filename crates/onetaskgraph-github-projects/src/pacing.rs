//! How this source keeps under GitHub's documented secondary limits: over their windows, not
//! by a fixed spacing.
//!
//! GitHub documents three secondary limits a copy can reach, each over a window rather than as
//! a rate: at most [`CONTENT_CREATION_PER_MINUTE`] content-generating requests in any minute and
//! [`CONTENT_CREATION_PER_HOUR`] in any hour, and at most [`SECONDARY_POINTS_PER_MINUTE`]
//! secondary points in any minute, a GraphQL request with a mutation counting
//! [`SECONDARY_POINTS_PER_MUTATION`] and one without [`SECONDARY_POINTS_PER_QUERY`]. A request
//! that keeps every window it charges under its limit goes out at once; one that would not waits
//! until every such window has room for its whole charge, and not a moment longer.
//!
//! **Each aliased mutation counts one creation, and so does each upload.** Whether GitHub counts
//! an aliased request once or once per field it holds could not be established without
//! spending the shared token this host's every manager draws on, so this counts the larger, and
//! a request is never refused for a count this under-stated.
//!
//! **What is counted is when each request finished**, which is after it arrived: the limiter
//! counts arrivals, and a request leaves a window no later than this source thinks it does, so
//! a window this source thinks has room really does, whatever a request spent in transit.
//!
//! **Each source keeps its own windows, for its process's life, and nothing is shared or
//! stored.** Another process on the same token is invisible here — its share of the hour is
//! what `retry-after` and the backing-off schedule answer, as they always have.
//!
//! [`CONTENT_CREATION_PER_MINUTE`]: crate::CONTENT_CREATION_PER_MINUTE
//! [`CONTENT_CREATION_PER_HOUR`]: crate::CONTENT_CREATION_PER_HOUR
//! [`SECONDARY_POINTS_PER_MINUTE`]: crate::SECONDARY_POINTS_PER_MINUTE
//! [`SECONDARY_POINTS_PER_MUTATION`]: crate::SECONDARY_POINTS_PER_MUTATION
//! [`SECONDARY_POINTS_PER_QUERY`]: crate::SECONDARY_POINTS_PER_QUERY

use std::time::Duration;

use serde_json::Value;

use crate::{
    CONTENT_CREATION_PER_HOUR, CONTENT_CREATION_PER_MINUTE, SECONDARY_POINTS_PER_MINUTE,
    SECONDARY_POINTS_PER_MUTATION, SECONDARY_POINTS_PER_QUERY,
};

/// One minute, the window of the per-minute limits.
const MINUTE: Duration = Duration::from_secs(60);
/// One hour, the window of the per-hour limit.
const HOUR: Duration = Duration::from_secs(3_600);

/// Why one wait this source made was made — the word it names the wait by on its clock, so a
/// journey reading a simulated clock's record can tell a pacing wait from a backing off.
pub(crate) mod cause {
    /// Held until the minute's content creations had room.
    pub(crate) const CREATION_MINUTE: &str = "content-creation-minute";
    /// Held until the hour's content creations had room.
    pub(crate) const CREATION_HOUR: &str = "content-creation-hour";
    /// Held until the minute's secondary points had room.
    pub(crate) const POINTS_MINUTE: &str = "secondary-points-minute";
    /// Held for the configured `pacing.min_mutation_interval_ms`.
    pub(crate) const MIN_INTERVAL: &str = "min-mutation-interval";
    /// A refusal's `retry-after`, waited out as asked.
    pub(crate) const RETRY_AFTER: &str = "retry-after";
    /// An exhausted primary budget, waited out until the reset it named.
    pub(crate) const PRIMARY_RESET: &str = "primary-reset";
    /// A refusal naming no wait, answered by the doubling schedule.
    pub(crate) const BACKOFF: &str = "backoff";
}

/// What one request costs against the secondary windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Charge {
    /// Content creations: each mutation field it carries, or one for an upload.
    pub(crate) creations: u64,
    /// Secondary points: a GraphQL request's.
    pub(crate) points: u64,
}

impl Charge {
    /// What one GraphQL request of `document` with `variables` is charged: a mutation counts
    /// each mutation field it sends as a creation and [`SECONDARY_POINTS_PER_MUTATION`] points,
    /// and a query no creation and [`SECONDARY_POINTS_PER_QUERY`] point.
    pub(crate) fn graphql(document: &str, variables: &Value) -> Self {
        if crate::is_mutation(document) {
            Self {
                creations: included_fields(document, variables),
                points: SECONDARY_POINTS_PER_MUTATION,
            }
        } else {
            Self {
                creations: 0,
                points: SECONDARY_POINTS_PER_QUERY,
            }
        }
    }

    /// What one upload is charged: one creation, and no GraphQL points.
    pub(crate) const fn upload() -> Self {
        Self {
            creations: 1,
            points: 0,
        }
    }
}

/// How many fields the operation's own selection set sends: every top-level field, less
/// those an `@include(if: …)` evaluating false or a `@skip(if: …)` evaluating true leaves out,
/// with `$variables` read from `variables`. A field naming no directive is always sent.
///
/// Read by brace depth over the document's text, which is every document this source sends:
/// a nested selection, an argument list and a string are never counted.
pub(crate) fn included_fields(document: &str, variables: &Value) -> u64 {
    let bytes = document.as_bytes();
    let Some(open) = selection_start(bytes) else {
        return 0;
    };
    let mut fields: Vec<bool> = Vec::new();
    let mut depth = 1_usize;
    let mut at = open + 1;
    let mut after_name = false;
    while at < bytes.len() && depth > 0 {
        let byte = bytes[at];
        match byte {
            b'"' => {
                at = past_string(bytes, at);
                continue;
            }
            b'(' => {
                at = past_group(bytes, at, b'(', b')');
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'@' if depth == 1 => {
                let (name, next) = name_at(bytes, at + 1);
                let (argument, next) = match skip_space(bytes, next) {
                    Some(paren) if bytes[paren] == b'(' => {
                        let end = past_group(bytes, paren, b'(', b')');
                        (Some(&document[paren + 1..end - 1]), end)
                    }
                    _ => (None, next),
                };
                if let (Some(field), Some(argument)) = (fields.last_mut(), argument) {
                    let condition = condition(argument, variables);
                    match name {
                        "include" => *field &= condition,
                        "skip" => *field &= !condition,
                        _ => {}
                    }
                }
                at = next;
                after_name = false;
                continue;
            }
            byte if depth == 1 && (byte.is_ascii_alphabetic() || byte == b'_') => {
                let (_, next) = name_at(bytes, at);
                // An alias and its field are one field: `alias: field`.
                let alias = skip_space(bytes, next).is_some_and(|colon| bytes[colon] == b':');
                if !after_name {
                    fields.push(true);
                }
                after_name = alias;
                at = match skip_space(bytes, next) {
                    Some(colon) if alias => colon + 1,
                    _ => next,
                };
                continue;
            }
            _ => {}
        }
        at += 1;
    }
    fields.into_iter().filter(|sent| *sent).count() as u64
}

/// Where the operation's own selection set opens: the first `{` outside its variable
/// definitions.
fn selection_start(bytes: &[u8]) -> Option<usize> {
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => at = past_string(bytes, at),
            b'(' => at = past_group(bytes, at, b'(', b')'),
            b'{' => return Some(at),
            _ => at += 1,
        }
    }
    None
}

/// The index just past the group opening at `at`, strings inside it skipped.
fn past_group(bytes: &[u8], at: usize, open: u8, close: u8) -> usize {
    let mut depth = 0_usize;
    let mut index = at;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index = past_string(bytes, index);
                continue;
            }
            byte if byte == open => depth += 1,
            byte if byte == close => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    bytes.len()
}

/// The index just past the string opening at `at`.
fn past_string(bytes: &[u8], at: usize) -> usize {
    let mut index = at + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// The name starting at `at`, and the index past it.
fn name_at(bytes: &[u8], at: usize) -> (&str, usize) {
    let end = bytes[at..]
        .iter()
        .position(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_'))
        .map_or(bytes.len(), |length| at + length);
    (
        std::str::from_utf8(&bytes[at..end]).unwrap_or_default(),
        end,
    )
}

/// The first index at or after `at` that is not white space, or `None` at the end.
fn skip_space(bytes: &[u8], at: usize) -> Option<usize> {
    (at..bytes.len()).find(|index| !bytes[*index].is_ascii_whitespace() && bytes[*index] != b',')
}

/// What a directive's `if:` argument evaluates to: a literal, or a boolean variable — anything
/// else reads as true, so a field this cannot read is counted rather than left out.
fn condition(argument: &str, variables: &Value) -> bool {
    let value = argument
        .trim()
        .strip_prefix("if")
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix(':'))
        .map_or("", str::trim);
    match value {
        "false" => false,
        "true" => true,
        variable => variable
            .strip_prefix('$')
            .and_then(|name| variables.get(name))
            .and_then(Value::as_bool)
            .unwrap_or(true),
    }
}

/// When the secondary limits' windows, as this source has charged them, next have room.
#[derive(Debug, Default)]
pub(crate) struct Windows {
    /// One entry per content creation charged, at the time it was counted, by its ticket.
    creations: Vec<(Duration, u64)>,
    /// One entry per GraphQL request charged: the time it was counted, its points and its
    /// ticket.
    points: Vec<(Duration, u64, u64)>,
    /// The ticket the next charge is counted under.
    next: u64,
}

/// One request's charge as it stands in the windows, finished by [`Windows::finish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket(u64);

impl Windows {
    /// The earliest moment at or after `now` at which every window `charge` touches has room
    /// for the whole of it, and the cause of any wait until then — the window that holds it
    /// longest.
    pub(crate) fn room(&self, now: Duration, charge: Charge) -> (Duration, Option<&'static str>) {
        let creations: Vec<(Duration, u64)> =
            self.creations.iter().map(|(at, _)| (*at, 1)).collect();
        let points: Vec<(Duration, u64)> = self
            .points
            .iter()
            .map(|(at, points, _)| (*at, *points))
            .collect();
        let mut free = (now, None);
        for (entries, window, limit, wanted, why) in [
            (
                &creations,
                MINUTE,
                CONTENT_CREATION_PER_MINUTE,
                charge.creations,
                cause::CREATION_MINUTE,
            ),
            (
                &creations,
                HOUR,
                CONTENT_CREATION_PER_HOUR,
                charge.creations,
                cause::CREATION_HOUR,
            ),
            (
                &points,
                MINUTE,
                SECONDARY_POINTS_PER_MINUTE,
                charge.points,
                cause::POINTS_MINUTE,
            ),
        ] {
            if wanted == 0 {
                continue;
            }
            let at = earliest(entries, now, window, limit, wanted);
            if at > free.0 {
                free = (at, Some(why));
            }
        }
        free
    }

    /// Count `charge` at `at`, ahead of the request that carries it, so a request deciding
    /// after this one sees it.
    pub(crate) fn charge(&mut self, at: Duration, charge: Charge) -> Ticket {
        let ticket = self.next;
        self.next += 1;
        for _ in 0..charge.creations {
            self.creations.push((at, ticket));
        }
        if charge.points > 0 {
            self.points.push((at, charge.points, ticket));
        }
        Ticket(ticket)
    }

    /// The request `ticket` was charged for has finished at `now`: it is counted from here,
    /// which is no earlier than it arrived, and whatever has left every window is forgotten.
    pub(crate) fn finish(&mut self, ticket: Ticket, now: Duration) {
        for entry in &mut self.creations {
            if entry.1 == ticket.0 {
                entry.0 = entry.0.max(now);
            }
        }
        for entry in &mut self.points {
            if entry.2 == ticket.0 {
                entry.0 = entry.0.max(now);
            }
        }
        self.creations.retain(|(at, _)| *at + HOUR > now);
        self.points.retain(|(at, _, _)| *at + MINUTE > now);
    }
}

/// The earliest moment at or after `now` at which the entries still inside a window of
/// `window` leave room for `wanted` more under `limit`: now, when they already do, or the moment
/// enough of the oldest have left. A charge larger than the whole limit waits for the window to
/// empty, which is as much room as there ever is.
fn earliest(
    entries: &[(Duration, u64)],
    now: Duration,
    window: Duration,
    limit: u64,
    wanted: u64,
) -> Duration {
    let mut inside: Vec<(Duration, u64)> = entries
        .iter()
        .copied()
        .filter(|(at, _)| *at + window > now)
        .collect();
    let mut total: u64 = inside.iter().map(|(_, amount)| amount).sum();
    if total + wanted <= limit {
        return now;
    }
    inside.sort_by_key(|(at, _)| *at);
    for (at, amount) in &inside {
        total -= amount;
        if total + wanted <= limit {
            return (*at + window).max(now);
        }
    }
    inside.last().map_or(now, |(at, _)| (*at + window).max(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_mutation_counts_each_field_it_sends_and_none_its_directives_leave_out() {
        let document = "mutation($a:Input!,$b:Input!,$c:Input!,$sendB:Boolean!,$skipC:Boolean!){\
            first:createIssue(input:$a){issue{id}} second:addBlockedBy(input:$b) @include(if:$sendB){issue{id}}\
            third:addBlockedBy(input:$c) @skip(if:$skipC){issue{id}} fourth: updateIssue(input:{title:\"a { brace\"}){issue{id}}}";
        for (sent_b, skip_c, expected) in [
            (true, false, 4),
            (false, false, 3),
            (true, true, 3),
            (false, true, 2),
        ] {
            assert_eq!(
                included_fields(document, &json!({"sendB": sent_b, "skipC": skip_c})),
                expected,
                "sendB {sent_b} skipC {skip_c}"
            );
        }
        assert_eq!(
            included_fields(crate::graphql::CREATE_ISSUE, &json!({})),
            1,
            "a lone mutation"
        );
    }

    #[test]
    fn a_window_has_room_once_enough_of_its_oldest_have_left_and_not_before() {
        let mut windows = Windows::default();
        let tenth = Duration::from_millis(100);
        // 79 creations, one every tenth of a second.
        for at in 0..79 {
            let ticket = windows.charge(tenth * at, Charge::upload());
            windows.finish(ticket, tenth * at);
        }
        let now = tenth * 79;
        assert_eq!(
            windows.room(now, Charge::upload()),
            (now, None),
            "the 80th fits"
        );
        let eleven = Charge {
            creations: 11,
            points: SECONDARY_POINTS_PER_MUTATION,
        };
        // 79 + 11 is 90: ten have to leave, and the tenth oldest was counted at 0.9 s.
        assert_eq!(
            windows.room(now, eleven),
            (tenth * 9 + MINUTE, Some(cause::CREATION_MINUTE))
        );
        // A query charges no creation, so the full minute of them never holds one.
        assert_eq!(
            windows.room(now, Charge::graphql("query{viewer{id}}", &json!({}))),
            (now, None)
        );
    }
}
