//! A task's change token on a GitHub board: what [`Task::version`](onetaskgraph_plugin_api::Task::version)
//! holds here, and the one ordering of two of them.
//!
//! The token is built from three values every read of an issue on this board already selects,
//! so it costs no request of its own:
//!
//! - the issue's own `updatedAt`, which GitHub moves for a title or body edit — and so for a
//!   metadata change, which this source keeps in the body's slot — a label change, a comment
//!   added or edited, a state change, and a dependency added or removed;
//! - the board item's own `updatedAt` (`ProjectV2Item.updatedAt`), which a board field change
//!   moves — the `Status` and `Priority` options, the origin field — where the issue's does not
//!   move;
//! - the issue's comment count (`comments { totalCount }`), which a deleted comment moves even
//!   where neither time does.
//!
//! An item missing any of the three — a draft, which has no comments, or an item this process
//! wrote and has not read back — gives no token, which a reader takes as *always read it*.
//! `docs/plugin-protocol.md` §4.13c states the change kinds covered, and the credentialed
//! journey in `tests/journey` re-takes each one against GitHub itself.
//!
//! **One bound, stated rather than discovered.** GitHub reports both times to the second, so
//! two changes inside one second that leave the comment count where it was read as one
//! version. A reader asking again a second later sees the later one.

use std::cmp::Ordering;

use chrono::{DateTime, SecondsFormat, Utc};

/// What every token this plugin gives starts with, so a reader can tell it from another
/// plugin's.
pub const PREFIX: &str = "gh1|";

/// The token for an issue last changed at `issue`, whose board item last changed at `item`,
/// holding `comments` comments.
#[must_use]
pub fn spell(issue: DateTime<Utc>, item: DateTime<Utc>, comments: u64) -> String {
    format!(
        "{PREFIX}{}|{}|{comments}",
        issue.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        item.to_rfc3339_opts(SecondsFormat::AutoSi, true)
    )
}

/// The three parts of a token this plugin gave, or `None` for any other value.
fn parts(token: &str) -> Option<(DateTime<Utc>, DateTime<Utc>, u64)> {
    let mut parts = token.strip_prefix(PREFIX)?.split('|');
    let issue = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let item = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let comments = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((
        issue.with_timezone(&Utc),
        item.with_timezone(&Utc),
        comments,
    ))
}

/// How the version `a` of one issue stands to its version `b`: `Greater` when `a` is the later,
/// `Less` when it is the earlier, `Equal` when nothing the token covers changed between them —
/// and `None` when either is not a token of this plugin, or the two cannot both be versions of
/// one issue read in order.
///
/// Both times only ever move forward, so a later version holds neither earlier; one holding
/// one time later and the other earlier is no version read after the other. When both times
/// are equal and the counts are not, the version with fewer comments is the later: the one
/// change that moves the count and neither time is a comment deleted, because adding one moves
/// the issue's time.
#[must_use]
pub fn order(a: &str, b: &str) -> Option<Ordering> {
    let (a_issue, a_item, a_count) = parts(a)?;
    let (b_issue, b_item, b_count) = parts(b)?;
    let times = (a_issue.cmp(&b_issue), a_item.cmp(&b_item));
    match times {
        (Ordering::Equal, Ordering::Equal) => Some(b_count.cmp(&a_count)),
        (Ordering::Less | Ordering::Equal, Ordering::Less | Ordering::Equal) => {
            Some(Ordering::Less)
        }
        (Ordering::Greater | Ordering::Equal, Ordering::Greater | Ordering::Equal) => {
            Some(Ordering::Greater)
        }
        _ => None,
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
        let before = spell(at(1), at(1), 2);
        for after in [
            spell(at(2), at(1), 3), // a comment added
            spell(at(2), at(1), 2), // a comment edited, a body or title edit, a label, an edge
            spell(at(1), at(1), 1), // a comment deleted
            spell(at(1), at(2), 2), // a board field: status, priority, origin
        ] {
            assert_eq!(order(&after, &before), Some(Ordering::Greater), "{after}");
            assert_eq!(order(&before, &after), Some(Ordering::Less), "{after}");
        }
        assert_eq!(order(&before, &before), Some(Ordering::Equal));
    }

    #[test]
    fn crossed_times_and_foreign_values_are_not_ordered() {
        assert_eq!(
            order(&spell(at(2), at(1), 0), &spell(at(1), at(2), 0)),
            None
        );
        assert_eq!(order("md1|abc", &spell(at(1), at(1), 0)), None);
        assert_eq!(order("gh1|not-a-time|x|1", "gh1|not-a-time|x|1"), None);
        assert_eq!(
            order(
                &format!("{}|extra", spell(at(1), at(1), 0)),
                &spell(at(1), at(1), 0)
            ),
            None
        );
    }
}
