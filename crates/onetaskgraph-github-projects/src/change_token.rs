//! An item's change token on a GitHub board: what [`Task::version`](onetaskgraph_plugin_api::Task::version),
//! [`Project::version`](onetaskgraph_plugin_api::Project::version) and
//! [`Document::version`](onetaskgraph_plugin_api::Document::version) hold here, and the one
//! ordering of two of them.
//!
//! `docs/plugin-protocol.md` §4.13d is the specification: the parts the token is built from, the
//! change kinds each covers member by member, and why a token waits for its item to settle. What
//! is here is how this module holds to it. Each part is selected by every list, search and
//! detail read already, so the token costs no request; an item missing any part — a draft, an
//! item this process wrote and has not read back, or one whose labels or board field values
//! came back as only a first page — gives none. [`order`] says how two tokens of one issue
//! stand to each other, and an item whose later time is within [`SETTLED`] of the response's
//! `Date`, or read from a response without one, gives no token.

use std::cmp::Ordering;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use onetaskgraph_plugin_api::{Label, Repository, asset_sha256};
use serde::Serialize;

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
/// `gh3` since the item's facets — its own kind, its board field values and its labels — joined
/// the token. A `gh2` token, which lacked them, could stay equal across a label renamed or
/// recoloured and a field option renamed, and order a token read after a far end's deletion
/// before the one held; a `gh1` token lacked the relationship count and the repository too.
/// Neither is a token this plugin orders or ever gives, so an entry or a copy that recorded one
/// reads its item again once rather than trusting a version that could not see those.
pub const PREFIX: &str = "gh3|";

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
    /// The digest of the item's [`Facets`].
    pub facets: FacetsDigest,
    /// The repository it is in. Spelled last, and read back as everything after the facets, so
    /// any origin a [`Repository`] admits comes back as it went.
    pub repository: Repository,
}

/// What an item is and holds that neither of its times moves for: its own kind, the value of
/// every board field it holds one of, and every label it carries, each named by what a person
/// can rename — a field option's name, a label's name and colour.
///
/// Its canonical JSON is
/// `{"kind":<"task"|"project"|"document">,"fields":[[<field>,<option or text>],...],"labels":[[<id>,<name>,<color>],...]}`,
/// each list sorted by its first element; the token carries its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Facets {
    /// The item's own kind, as this plugin derives it from the response — the `DESIGN: ` title
    /// prefix, its sub-issues — rather than the raw sub-issue total, so a project gaining a task
    /// does not move it.
    kind: FacetKind,
    /// Each board field this item holds a value of, by name, with the option's name or the text.
    fields: Vec<(String, String)>,
    /// Each label, by id, with its name and colour.
    labels: Vec<(String, String, Option<String>)>,
}

/// The kind of item a token's facets name: the one closed set the canonical JSON's `kind` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FacetKind {
    /// A task.
    Task,
    /// A project.
    Project,
    /// A document.
    Document,
}

/// The lowercase hex SHA-256 of an item's [`Facets`]: the token's facets part, held to that
/// spelling so a token spelled from it is always one [`order`] can read back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FacetsDigest(String);

/// Spelled as the string it serializes to, inline, exactly as the `String` it replaced was.
impl schemars::JsonSchema for FacetsDigest {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        String::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(generator)
    }
}

impl TryFrom<String> for FacetsDigest {
    type Error = String;

    fn try_from(digest: String) -> Result<Self, Self::Error> {
        Self::parse(&digest).ok_or_else(|| format!("{digest:?} is not a lowercase hex SHA-256"))
    }
}

impl From<FacetsDigest> for String {
    fn from(digest: FacetsDigest) -> Self {
        digest.0
    }
}

impl FacetsDigest {
    /// `digest`, when it is spelled as a lowercase hex SHA-256.
    #[must_use]
    pub fn parse(digest: &str) -> Option<Self> {
        is_digest(digest).then(|| Self(digest.to_owned()))
    }

    /// The digest as the token spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Facets {
    /// The facets of an item of `kind` holding `fields` and carrying `labels`, in any order.
    #[must_use]
    pub fn new(kind: FacetKind, mut fields: Vec<(String, String)>, labels: &[Label]) -> Self {
        fields.sort();
        let mut labels: Vec<(String, String, Option<String>)> = labels
            .iter()
            .map(|label| (label.id.0.clone(), label.name.clone(), label.color.clone()))
            .collect();
        labels.sort();
        Self {
            kind,
            fields,
            labels,
        }
    }

    /// The lowercase hex SHA-256 of their canonical JSON: the token's facets part.
    #[must_use]
    pub fn digest(&self) -> FacetsDigest {
        // A struct of strings and lists of them always serializes, and a SHA-256 is always
        // spelled as one.
        FacetsDigest(asset_sha256(
            serde_json::to_string(self).unwrap_or_default().as_bytes(),
        ))
    }
}

/// The token these parts spell.
#[must_use]
pub fn spell(parts: &Parts) -> String {
    format!(
        "{PREFIX}{}|{}|{}|{}|{}|{}",
        parts.issue.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        parts.item.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        parts.comments,
        parts.relationships,
        parts.facets.as_str(),
        parts.repository.as_str()
    )
}

/// Whether `facets` is spelled as a digest is.
fn is_digest(facets: &str) -> bool {
    facets.len() == 64
        && facets
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The parts of a token this plugin gave, or `None` for any other value.
fn parts(token: &str) -> Option<Parts> {
    let mut parts = token.strip_prefix(PREFIX)?.splitn(6, '|');
    let issue = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let item = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let comments = parts.next()?.parse().ok()?;
    let relationships = parts.next()?.parse().ok()?;
    let facets = FacetsDigest::parse(parts.next()?)?;
    let repository = Repository::try_from(parts.next()?.to_owned()).ok()?;
    Some(Parts {
        issue: issue.with_timezone(&Utc),
        item: item.with_timezone(&Utc),
        comments,
        relationships,
        facets,
        repository,
    })
}

/// How the version `a` of one issue stands to its version `b`: `Greater` when `a` is the later,
/// `Less` when it is the earlier, `Equal` only when every part is equal — and `None` when either
/// is not a token of this plugin, or the two cannot be put in order.
///
/// Two versions naming different repositories, different facets or different relationship
/// counts are never ordered: a transfer, a label renamed, a field option renamed, a kind
/// changed or a far end deleted says nothing about which reading came first, and a count that
/// can fall — a deleted far end takes its events with it — orders nothing. Otherwise both times
/// only ever move forward, so a later version holds neither earlier; one holding one of them
/// ahead and the other behind is no version read after the other. When both times are equal and
/// the comment counts are not, the version with fewer comments is the later: the one change that
/// moves the comment count and nothing else is a comment deleted, because adding one moves the
/// issue's time.
#[must_use]
pub fn order(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parts(a)?, parts(b)?);
    if a.repository != b.repository || a.facets != b.facets || a.relationships != b.relationships {
        return None;
    }
    let forward = [a.issue.cmp(&b.issue), a.item.cmp(&b.item)];
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
    use onetaskgraph_plugin_api::NativeId;

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, second).unwrap()
    }

    fn label(id: &str, name: &str, color: &str) -> Label {
        Label {
            id: NativeId::from(id),
            name: name.to_owned(),
            color: Some(color.to_owned()),
        }
    }

    /// The facets of a task in `Todo` carrying the label `bug`.
    fn facets() -> FacetsDigest {
        Facets::new(
            FacetKind::Task,
            vec![("Status".to_owned(), "Todo".to_owned())],
            &[label("L_1", "bug", "d73a4a")],
        )
        .digest()
    }

    fn token(issue: u32, item: u32, comments: u64, relationships: u64) -> String {
        token_with(issue, item, comments, relationships, &facets())
    }

    fn token_with(
        issue: u32,
        item: u32,
        comments: u64,
        relationships: u64,
        facets: &FacetsDigest,
    ) -> String {
        spell(&Parts {
            issue: at(issue),
            item: at(item),
            comments,
            relationships,
            facets: facets.clone(),
            repository: repository("github.com/acme/widgets"),
        })
    }

    fn repository(origin: &str) -> Repository {
        Repository::try_from(origin.to_owned()).expect("a repository origin")
    }

    #[test]
    fn the_facets_are_the_digest_of_their_canonical_json_whatever_order_they_arrive_in() {
        let one = Facets::new(
            FacetKind::Project,
            vec![
                ("Status".to_owned(), "Todo".to_owned()),
                ("Priority".to_owned(), "P1".to_owned()),
            ],
            &[label("L_2", "ui", "ffffff"), label("L_1", "bug", "d73a4a")],
        );
        let other = Facets::new(
            FacetKind::Project,
            vec![
                ("Priority".to_owned(), "P1".to_owned()),
                ("Status".to_owned(), "Todo".to_owned()),
            ],
            &[label("L_1", "bug", "d73a4a"), label("L_2", "ui", "ffffff")],
        );
        assert_eq!(one, other);
        assert_eq!(
            serde_json::to_string(&one).unwrap(),
            r#"{"kind":"project","fields":[["Priority","P1"],["Status","Todo"]],"labels":[["L_1","bug","d73a4a"],["L_2","ui","ffffff"]]}"#
        );
        assert_eq!(
            one.digest().as_str(),
            asset_sha256(serde_json::to_string(&one).unwrap().as_bytes())
        );
    }

    #[test]
    fn every_change_kind_the_times_carry_orders_the_later_version_after_the_earlier() {
        let before = token(1, 1, 2, 3);
        for after in [
            token(2, 1, 3, 3), // a comment added
            token(2, 1, 2, 3), // a comment edited, a body or title edit
            token(1, 1, 1, 3), // a comment deleted
            token(1, 2, 2, 3), // a board field write that leaves its value's name as it was
        ] {
            assert_eq!(order(&after, &before), Some(Ordering::Greater), "{after}");
            assert_eq!(order(&before, &after), Some(Ordering::Less), "{after}");
        }
        assert_eq!(order(&before, &before), Some(Ordering::Equal));
    }

    #[test]
    fn another_facet_another_relationship_count_or_another_repository_is_never_ordered() {
        let before = token(1, 1, 2, 3);
        let renamed = Facets::new(
            FacetKind::Task,
            vec![("Status".to_owned(), "Todo".to_owned())],
            &[label("L_1", "defect", "d73a4a")],
        )
        .digest();
        let recoloured = Facets::new(
            FacetKind::Task,
            vec![("Status".to_owned(), "Todo".to_owned())],
            &[label("L_1", "bug", "000000")],
        )
        .digest();
        let option_renamed = Facets::new(
            FacetKind::Task,
            vec![("Status".to_owned(), "To do".to_owned())],
            &[label("L_1", "bug", "d73a4a")],
        )
        .digest();
        let became_a_project = Facets::new(
            FacetKind::Project,
            vec![("Status".to_owned(), "Todo".to_owned())],
            &[label("L_1", "bug", "d73a4a")],
        )
        .digest();
        for (change, facets) in [
            ("a label renamed", &renamed),
            ("a label recoloured", &recoloured),
            ("a field option renamed", &option_renamed),
            ("its own kind changed", &became_a_project),
        ] {
            // Not even a later time puts it in order: nothing says which reading came first.
            for after in [
                token_with(1, 1, 2, 3, facets),
                token_with(2, 2, 2, 3, facets),
            ] {
                assert_eq!(order(&after, &before), None, "{change}");
                assert_eq!(order(&before, &after), None, "{change}");
            }
        }
        for relationships in [2, 4, 5] {
            assert_eq!(
                order(&token(2, 2, 2, relationships), &before),
                None,
                "a relationship count of {relationships} against 3: a far end deleted lowers it"
            );
        }
        let transferred = spell(&Parts {
            issue: at(1),
            item: at(1),
            comments: 2,
            relationships: 3,
            facets: facets(),
            repository: repository("github.com/acme/gadgets"),
        });
        assert_eq!(order(&transferred, &before), None);
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
    fn crossed_times_and_foreign_or_older_values_are_not_ordered() {
        assert_eq!(order(&token(2, 1, 0, 0), &token(1, 2, 0, 0)), None);
        assert_eq!(order("md1|abc", &token(1, 1, 0, 0)), None);
        let digest = facets().as_str().to_owned();
        for spelled in ["not-a-digest", &digest.to_uppercase(), &digest[1..]] {
            assert_eq!(FacetsDigest::parse(spelled), None, "{spelled}");
            assert!(
                serde_json::from_value::<FacetsDigest>(serde_json::json!(spelled)).is_err(),
                "a context record holding {spelled} is refused"
            );
        }
        assert_eq!(
            serde_json::from_value::<FacetsDigest>(serde_json::json!(digest)).ok(),
            Some(facets())
        );
        for malformed in [
            format!("gh3|not-a-time|x|1|0|{digest}|github.com/a/b"),
            format!("gh3|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|{digest}|"),
            format!("gh3|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|{digest}|a/b"),
            "gh3|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|not-a-digest|github.com/a/b"
                .to_owned(),
        ] {
            assert_eq!(order(&malformed, &malformed), None, "{malformed}");
        }
        for older in [
            "gh2|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0|0|github.com/a/b",
            "gh1|2026-10-01T00:00:01Z|2026-10-01T00:00:01Z|0",
        ] {
            assert_eq!(
                order(older, older),
                None,
                "a token from before the facets is not ordered, even against itself"
            );
        }
        assert_eq!(
            order(&format!("{}|extra", token(1, 1, 0, 0)), &token(1, 1, 0, 0)),
            None,
            "anything after the repository is part of it, and names another"
        );
    }
}
