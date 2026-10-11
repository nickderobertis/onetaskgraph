//! A task's change token on a GitHub board: what [`Task::version`](onetaskgraph_plugin_api::Task::version)
//! holds here, and the one ordering of two of them.
//!
//! The token is built from four values every read of an issue on this board already selects,
//! so it costs no request of its own:
//!
//! - the issue's own `updatedAt`, which GitHub moves for a title or body edit — and so for a
//!   metadata change, which this source keeps in the body's slot — a label change, a comment
//!   added or edited, and a state change;
//! - the board item's own `updatedAt` (`ProjectV2Item.updatedAt`), which a board field change
//!   moves — the `Status` and `Priority` options, the origin field — where the issue's does not
//!   move;
//! - the issue's comment count (`comments { totalCount }`), which a deleted comment moves even
//!   where neither time does;
//! - how many dependency events the issue's timeline holds (`timelineItems(itemTypes:
//!   [BLOCKED_BY_ADDED_EVENT, BLOCKED_BY_REMOVED_EVENT, BLOCKING_ADDED_EVENT,
//!   BLOCKING_REMOVED_EVENT]) { totalCount }`), which every dependency added or removed moves —
//!   a replaced one by two. **Neither time moves for a dependency**: the credentialed lane
//!   added one to a real issue and saw the issue's `updatedAt`, the board item's and the
//!   comment count all stay where they were, so a token of the first three alone read an edge
//!   change as no change, and a copy that skipped on it kept edges that had moved. A timeline
//!   event is never removed, so this count only grows, and a later version always holds at
//!   least as many as an earlier one.
//!
//! An item missing any of the four — a draft, which has no comments, or an item this process
//! wrote and has not read back — gives no token, which a reader takes as *always read it*.
//! `docs/plugin-protocol.md` §4.13d states the change kinds covered, and the credentialed
//! journey in `tests/journey` re-takes each one against GitHub itself.
//!
//! **A token is given only once the item has settled.** GitHub reports both times to the
//! second, so a change made in the same second as the one a read saw would leave both times
//! where they were and, with the comment count unmoved, read as the same version — and a copy
//! that recorded that version would skip the item and keep content that changed. So an item is
//! given a token only when its later time is more than [`SETTLED`] before the server's own
//! clock on the response that read it, its `Date` header: any change after that read then
//! falls in a later second than the one the token records, and moves it. The dependency count
//! needs no such margin, because it moves on every event whatever second it falls in. An item changed more
//! recently, and every item of a response carrying no `Date`, gives no token and is read in
//! full. `docs/plugin-protocol.md` §4.13d says the same, and so does every reader that keys on
//! this token — the board cache included.

use std::cmp::Ordering;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

/// How long before the response that read it an item must last have changed for its token to
/// be given.
///
/// GitHub aborts a GraphQL query that runs past ten seconds, so the state a response reports
/// was read at most about ten seconds before the `Date` it is stamped with. Sixty seconds is
/// that bound with a wide margin: an item whose later time is further back than this was last
/// changed in an earlier second than the read, so no change after the read can share its
/// second.
pub const SETTLED: TimeDelta = TimeDelta::seconds(60);

/// Whether an issue last changed at `issue`, whose board item last changed at `item`, had
/// settled by `served` — the `Date` of the response that read it — so its token may be given.
#[must_use]
pub fn settled(issue: DateTime<Utc>, item: DateTime<Utc>, served: DateTime<Utc>) -> bool {
    served - issue.max(item) > SETTLED
}

/// What every token this plugin gives starts with, so a reader can tell it from another
/// plugin's.
///
/// `gh2` since the dependency count joined the token: a `gh1` token, which lacked it, is not one
/// this plugin orders, so a copy that recorded one reads its task again once rather than
/// trusting a version that could not see an edge move.
pub const PREFIX: &str = "gh2|";

/// The token for an issue last changed at `issue`, whose board item last changed at `item`,
/// holding `comments` comments and `edges` dependency events.
#[must_use]
pub fn spell(issue: DateTime<Utc>, item: DateTime<Utc>, comments: u64, edges: u64) -> String {
    format!(
        "{PREFIX}{}|{}|{comments}|{edges}",
        issue.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        item.to_rfc3339_opts(SecondsFormat::AutoSi, true)
    )
}

/// The four parts of a token this plugin gave, or `None` for any other value.
fn parts(token: &str) -> Option<(DateTime<Utc>, DateTime<Utc>, u64, u64)> {
    let mut parts = token.strip_prefix(PREFIX)?.split('|');
    let issue = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let item = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let comments = parts.next()?.parse().ok()?;
    let edges = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((
        issue.with_timezone(&Utc),
        item.with_timezone(&Utc),
        comments,
        edges,
    ))
}

/// How the version `a` of one issue stands to its version `b`: `Greater` when `a` is the later,
/// `Less` when it is the earlier, `Equal` when nothing the token covers changed between them —
/// and `None` when either is not a token of this plugin, or the two cannot both be versions of
/// one issue read in order.
///
/// Both times and the dependency count only ever move forward, so a later version holds none of
/// the three earlier or smaller; one holding one of them ahead and another behind is no version
/// read after the other. When all three are equal and the comment counts are not, the version
/// with fewer comments is the later: the one change that moves the comment count and nothing
/// else is a comment deleted, because adding one moves the issue's time.
#[must_use]
pub fn order(a: &str, b: &str) -> Option<Ordering> {
    let (a_issue, a_item, a_count, a_edges) = parts(a)?;
    let (b_issue, b_item, b_count, b_edges) = parts(b)?;
    let forward = [
        a_issue.cmp(&b_issue),
        a_item.cmp(&b_item),
        a_edges.cmp(&b_edges),
    ];
    if forward.iter().all(|part| part.is_eq()) {
        Some(b_count.cmp(&a_count))
    } else if forward.iter().all(|part| part.is_le()) {
        Some(Ordering::Less)
    } else if forward.iter().all(|part| part.is_ge()) {
        Some(Ordering::Greater)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, second).unwrap()
    }

    #[test]
    fn every_change_kind_orders_the_later_version_after_the_earlier() {
        let before = spell(at(1), at(1), 2, 3);
        for after in [
            spell(at(2), at(1), 3, 3), // a comment added
            spell(at(2), at(1), 2, 3), // a comment edited, a body or title edit, a label
            spell(at(1), at(1), 1, 3), // a comment deleted
            spell(at(1), at(2), 2, 3), // a board field: status, priority, origin
            spell(at(1), at(1), 2, 4), // a dependency added or removed, neither time moving
            spell(at(1), at(1), 2, 5), // a dependency replaced: one removed and one added
        ] {
            assert_eq!(order(&after, &before), Some(Ordering::Greater), "{after}");
            assert_eq!(order(&before, &after), Some(Ordering::Less), "{after}");
        }
        assert_eq!(order(&before, &before), Some(Ordering::Equal));
    }

    #[test]
    fn a_token_is_given_only_once_the_later_time_is_more_than_the_margin_behind_the_read() {
        let served = at(0) + SETTLED + TimeDelta::seconds(1);
        assert!(settled(at(0), at(0), served));
        assert!(
            !settled(at(0), at(1), served),
            "the board item's later time counts"
        );
        assert!(!settled(at(1), at(0), served), "and so does the issue's");
        assert!(
            !settled(at(0), at(0), at(0) + SETTLED),
            "exactly the margin is not past it"
        );
        assert!(
            !settled(at(5), at(5), at(5)),
            "read in the second of the change"
        );
    }

    #[test]
    fn crossed_times_and_foreign_values_are_not_ordered() {
        assert_eq!(
            order(&spell(at(2), at(1), 0, 0), &spell(at(1), at(2), 0, 0)),
            None
        );
        assert_eq!(
            order(&spell(at(2), at(1), 0, 0), &spell(at(1), at(1), 0, 1)),
            None,
            "a later time beside fewer dependency events is no version read after the other"
        );
        assert_eq!(order("md1|abc", &spell(at(1), at(1), 0, 0)), None);
        assert_eq!(order("gh2|not-a-time|x|1|0", "gh2|not-a-time|x|1|0"), None);
        assert_eq!(
            order(
                "gh1|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0",
                "gh1|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0"
            ),
            None,
            "a token from before the dependency count is not ordered, even against itself"
        );
        assert_eq!(
            order(
                &format!("{}|extra", spell(at(1), at(1), 0, 0)),
                &spell(at(1), at(1), 0, 0)
            ),
            None
        );
    }
}
