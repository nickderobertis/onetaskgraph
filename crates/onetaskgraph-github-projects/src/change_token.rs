//! A task's change token on a GitHub board: what [`Task::version`](onetaskgraph_plugin_api::Task::version)
//! holds here, and the one ordering of two of them.
//!
//! The token is built from five values every read of an issue on this board already selects,
//! so it costs no request of its own:
//!
//! - the issue's own `updatedAt`, which GitHub moves for a title or body edit — and so for a
//!   metadata change, which this source keeps in the body's slot, and for everything else that
//!   slot holds — a label change, a comment added or edited, and a state change;
//! - the board item's own `updatedAt` (`ProjectV2Item.updatedAt`), which a board field change
//!   moves — the `Status` and `Priority` options, the origin field and every other text field —
//!   where the issue's does not move;
//! - the issue's comment count (`comments { totalCount }`), which a deleted comment moves even
//!   where neither time does;
//! - how many relationship events the issue's timeline holds (`timelineItems(itemTypes:
//!   [BLOCKED_BY_ADDED_EVENT, BLOCKED_BY_REMOVED_EVENT, BLOCKING_ADDED_EVENT,
//!   BLOCKING_REMOVED_EVENT, PARENT_ISSUE_ADDED_EVENT, PARENT_ISSUE_REMOVED_EVENT])
//!   { totalCount }`), which every dependency and every project membership added or removed
//!   moves — a replaced one by two. **Neither time
//!   moves for a dependency**: the credentialed lane added one to a real issue and saw the
//!   issue's `updatedAt`, the board item's and the comment count all stay where they were, so a
//!   token of those three alone read an edge change as no change, and a copy that skipped on it
//!   kept edges that had moved. A project membership is the same kind of relationship — a
//!   parent issue — so it is counted the same way rather than trusted to a time. A timeline
//!   event is never removed, so this count only grows, and a later version always holds at
//!   least as many as an earlier one;
//! - the repository the issue is in (`repository { nameWithOwner }`), which an issue
//!   transferred to another repository changes. It is compared and never ordered: a token
//!   naming another repository is no version of this one read in order, so a transfer can never
//!   leave two tokens equal, whatever GitHub's times do.
//!
//! An item missing any of the five — a draft, which has no comments, or an item this process
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
//! falls in a later second than the one the token records, and moves it. The relationship
//! count and the repository need no such margin, because they move on every change whatever
//! second it falls in. An item changed more recently, and every item of a response carrying no
//! `Date`, gives no token and is read in full. `docs/plugin-protocol.md` §4.13d says the same,
//! and so does every reader that keys on this token — the board cache included.

use std::cmp::Ordering;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use onetaskgraph_plugin_api::Repository;

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
/// `gh2` since the relationship count and the repository joined the token: a `gh1` token,
/// which lacked both, is not one this plugin orders or ever gives, so a copy that recorded one
/// reads its task again once rather than trusting a version that could not see an edge, a
/// project membership or a transfer move.
pub const PREFIX: &str = "gh2|";

/// What one token of this plugin is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    /// When the issue last changed.
    pub issue: DateTime<Utc>,
    /// When its board item last changed.
    pub item: DateTime<Utc>,
    /// How many comments it holds.
    pub comments: u64,
    /// How many relationship events — dependencies and project memberships added or removed —
    /// its timeline holds.
    pub relationships: u64,
    /// The repository it is in. Spelled last, and read back as everything after the fourth
    /// count, so any origin a [`Repository`] admits comes back as it went.
    pub repository: Repository,
}

/// The token these parts spell.
#[must_use]
pub fn spell(parts: &Parts) -> String {
    format!(
        "{PREFIX}{}|{}|{}|{}|{}",
        parts.issue.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        parts.item.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        parts.comments,
        parts.relationships,
        parts.repository.as_str()
    )
}

/// The parts of a token this plugin gave, or `None` for any other value.
fn parts(token: &str) -> Option<Parts> {
    let mut parts = token.strip_prefix(PREFIX)?.splitn(5, '|');
    let issue = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let item = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let comments = parts.next()?.parse().ok()?;
    let relationships = parts.next()?.parse().ok()?;
    let repository = Repository::try_from(parts.next()?.to_owned()).ok()?;
    Some(Parts {
        issue: issue.with_timezone(&Utc),
        item: item.with_timezone(&Utc),
        comments,
        relationships,
        repository,
    })
}

/// How the version `a` of one issue stands to its version `b`: `Greater` when `a` is the later,
/// `Less` when it is the earlier, `Equal` when nothing the token covers changed between them —
/// and `None` when either is not a token of this plugin, or the two cannot both be versions of
/// one issue read in order.
///
/// Two versions naming different repositories are never ordered: a transfer moves the issue
/// and says nothing about which reading came first. Both times and the relationship count only
/// ever move forward, so a later version holds none of the three earlier or smaller; one
/// holding one of them ahead and another behind is no version read after the other. When all
/// three are equal and the comment counts are not, the version with fewer comments is the
/// later: the one change that moves the comment count and nothing else is a comment deleted,
/// because adding one moves the issue's time.
#[must_use]
pub fn order(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parts(a)?, parts(b)?);
    if a.repository != b.repository {
        return None;
    }
    let forward = [
        a.issue.cmp(&b.issue),
        a.item.cmp(&b.item),
        a.relationships.cmp(&b.relationships),
    ];
    if forward.iter().all(|part| part.is_eq()) {
        Some(b.comments.cmp(&a.comments))
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

    fn token(issue: u32, item: u32, comments: u64, relationships: u64) -> String {
        spell(&Parts {
            issue: at(issue),
            item: at(item),
            comments,
            relationships,
            repository: repository("github.com/acme/widgets"),
        })
    }

    fn repository(origin: &str) -> Repository {
        Repository::try_from(origin.to_owned()).expect("a repository origin")
    }

    #[test]
    fn every_change_kind_orders_the_later_version_after_the_earlier() {
        let before = token(1, 1, 2, 3);
        for after in [
            token(2, 1, 3, 3), // a comment added
            token(2, 1, 2, 3), // a comment edited, a body or title edit, a label
            token(1, 1, 1, 3), // a comment deleted
            token(1, 2, 2, 3), // a board field: status, priority, origin
            token(1, 1, 2, 4), // a dependency or a project membership added or removed
            token(1, 1, 2, 5), // either replaced: one removed and one added
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
    fn crossed_parts_another_repository_and_foreign_values_are_not_ordered() {
        assert_eq!(order(&token(2, 1, 0, 0), &token(1, 2, 0, 0)), None);
        assert_eq!(
            order(&token(2, 1, 0, 0), &token(1, 1, 0, 1)),
            None,
            "a later time beside fewer relationship events is no version read after the other"
        );
        let transferred = spell(&Parts {
            issue: at(1),
            item: at(1),
            comments: 0,
            relationships: 0,
            repository: repository("github.com/acme/gadgets"),
        });
        assert_eq!(
            order(&transferred, &token(1, 1, 0, 0)),
            None,
            "a token naming another repository is not equal, and not ordered"
        );
        assert_eq!(order("md1|abc", &token(1, 1, 0, 0)), None);
        assert_eq!(
            order(
                "gh2|not-a-time|x|1|0|github.com/a/b",
                "gh2|not-a-time|x|1|0|github.com/a/b"
            ),
            None
        );
        assert_eq!(
            order(
                "gh2|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|",
                "gh2|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|"
            ),
            None,
            "a token naming no repository"
        );
        assert_eq!(
            order(
                "gh2|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|a/b",
                "gh2|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|a/b"
            ),
            None,
            "a token naming something that is no repository origin"
        );
        assert_eq!(
            order(
                "gh1|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0",
                "gh1|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0"
            ),
            None,
            "a token from before the relationship count is not ordered, even against itself"
        );
        assert_eq!(
            order(&format!("{}|extra", token(1, 1, 0, 0)), &token(1, 1, 0, 0)),
            None,
            "anything after the repository is part of it, and names another"
        );
    }
}
