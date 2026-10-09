//! A source's `routes`: where an item written to it goes instead, chosen by the item's
//! repositories.
//!
//! Engine-owned rather than plugin-owned: `routes` sits beside `plugin` and `config` in a
//! source's entry, and a plugin's own block never sees it. Every writer reaches the store,
//! so the rule is held here once rather than restated, and drifting, in each of them.
//!
//! An entry is one of two kinds. A **repository** entry, `{repositories, to}`, matches an
//! item when **every** one of the item's repositories matches at least one of its patterns;
//! an item with no repositories matches none. A **classification** entry,
//! `{classification, to}`, matches every item of that classification.
//!
//! **Safety is decided before matching.** An item that may only be written somewhere
//! private is never placed in a source not declared private: for it, only entries sending
//! to a private source are tried — classification entries before repository entries, each
//! in the order written — and then the source itself when it is private. When none is, it
//! stays in the source itself, and the write is refused there rather than sent anywhere
//! less safe. Every other item tries its classification entries, then its repository
//! entries, each in order, and no match leaves it in the source itself.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{Classification, Repository, SourceName};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ConfigError, SourceVisibility};

/// One pattern over a normalized repository origin, `host/owner/name`, where a `*` segment
/// matches exactly one whole segment of the origin.
///
/// Kept as it was written, because that is how `config show` and every refusal spell it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryPattern(String);

impl RepositoryPattern {
    /// One pattern, once it is established it is one.
    ///
    /// # Errors
    ///
    /// Returns why when the pattern is not itself spelled as a normalized origin — the
    /// shape [`Repository`] holds every origin to, so a pattern no origin could match is
    /// refused by the one rule that defines an origin rather than by a restatement of it —
    /// or when a segment holds a `*` beside other characters.
    pub fn new(pattern: impl Into<String>) -> Result<Self, String> {
        let pattern = pattern.into();
        let valid = Repository::try_from(pattern.clone()).is_ok()
            && pattern
                .split('/')
                .all(|segment| segment == "*" || !segment.contains('*'));
        valid.then_some(Self(pattern.clone())).ok_or_else(|| {
            format!(
                "{pattern:?} is not a repository pattern: a pattern is host/owner/name, each \
                 segment either a literal or a lone `*` matching one whole segment, with no \
                 scheme and no .git suffix"
            )
        })
    }

    /// Whether `origin` is one this pattern names.
    #[must_use]
    pub fn matches(&self, origin: &Repository) -> bool {
        let pattern: Vec<&str> = self.0.split('/').collect();
        let origin: Vec<&str> = origin.as_str().split('/').collect();
        pattern.len() == origin.len()
            && pattern
                .iter()
                .zip(&origin)
                .all(|(pattern, origin)| *pattern == "*" || pattern == origin)
    }

    /// The pattern as it was written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One entry of a source's `routes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    matcher: Matcher,
    to: SourceName,
}

/// What one entry matches on: exactly one of the two, so an entry naming both or neither
/// cannot be held.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Matcher {
    /// Items every one of whose repositories one of these patterns matches. Never empty.
    Repositories(Vec<RepositoryPattern>),
    /// Every item of this classification.
    Classification(Classification),
}

impl Route {
    /// The patterns, in the order written — empty for a classification entry.
    #[must_use]
    pub fn repositories(&self) -> &[RepositoryPattern] {
        match &self.matcher {
            Matcher::Repositories(patterns) => patterns,
            Matcher::Classification(_) => &[],
        }
    }

    /// The classification a classification entry matches, or `None` for a repository entry.
    #[must_use]
    pub fn classification(&self) -> Option<Classification> {
        match &self.matcher {
            Matcher::Repositories(_) => None,
            Matcher::Classification(classification) => Some(*classification),
        }
    }

    /// The configured source an item this entry matches goes to.
    #[must_use]
    pub fn to(&self) -> &SourceName {
        &self.to
    }

    /// Whether every one of `repositories` matches one of this entry's patterns — `false`
    /// for an item naming none, and for a classification entry.
    #[must_use]
    pub fn matches(&self, repositories: &[Repository]) -> bool {
        let Matcher::Repositories(patterns) = &self.matcher else {
            return false;
        };
        !repositories.is_empty()
            && repositories
                .iter()
                .all(|origin| patterns.iter().any(|pattern| pattern.matches(origin)))
    }
}

/// Where one item written to a source lands, and which of its routes put it there.
///
/// `route` is always written, `null` included: a reader of a routed copy's report is told
/// that no entry matched rather than left to infer it from an absent key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Placement {
    /// The source the item lands in: the routed one, or the source itself.
    pub destination: SourceName,
    /// The index of the route entry that matched, or `None` when none did and the item
    /// stays in the source itself.
    pub route: Option<u32>,
}

/// What `sources route` answers: where an item with the repositories asked about, written
/// to `source`, would land — read from configuration alone, never from a source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceRoute {
    /// The source the item would be written to.
    pub source: SourceName,
    /// Where it would land, and the route entry that would send it there.
    #[serde(flatten)]
    pub placement: Placement,
}

/// Every configured source's routes, by source name, and who each source is declared
/// readable by. A source with no routes has no entry in the first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routes(
    BTreeMap<SourceName, Vec<Route>>,
    BTreeMap<SourceName, SourceVisibility>,
);

impl Routes {
    /// Where a public item with `repositories` written to `source` lands.
    #[must_use]
    pub fn place(&self, source: &SourceName, repositories: &[Repository]) -> Placement {
        self.place_classified(source, Classification::Public, repositories)
    }

    /// Where an item of `classification` with `repositories`, written to `source`, lands —
    /// by the module's rule: safety first, then classification entries, then repository
    /// entries, then the source itself.
    #[must_use]
    pub fn place_classified(
        &self,
        source: &SourceName,
        classification: Classification,
        repositories: &[Repository],
    ) -> Placement {
        let entries: Vec<(usize, &Route)> = self
            .0
            .get(source)
            .map(|routes| {
                routes
                    .iter()
                    .enumerate()
                    .filter(|(_, route)| classification.is_public() || self.is_private(&route.to))
                    .collect()
            })
            .unwrap_or_default();
        let chosen = entries
            .iter()
            .find(|(_, route)| route.classification() == Some(classification))
            .or_else(|| {
                entries
                    .iter()
                    .find(|(_, route)| route.matches(repositories))
            });
        chosen.map_or_else(
            || Placement {
                destination: source.clone(),
                route: None,
            },
            |(index, route)| Placement {
                destination: route.to.clone(),
                route: Some(u32::try_from(*index).unwrap_or(u32::MAX)),
            },
        )
    }

    /// Whether `source` is declared private.
    #[must_use]
    pub fn is_private(&self, source: &SourceName) -> bool {
        self.1.get(source) == Some(&SourceVisibility::Private)
    }

    /// Who `source` is declared readable by: `unknown` for a source nothing declares.
    #[must_use]
    pub fn visibility(&self, source: &SourceName) -> SourceVisibility {
        self.1.get(source).copied().unwrap_or_default()
    }

    /// The same routes, holding each source to the visibility `declared` gives it.
    #[must_use]
    pub fn with_visibility(mut self, declared: BTreeMap<SourceName, SourceVisibility>) -> Self {
        self.1 = declared;
        self
    }

    /// Whether `source` declares any route.
    #[must_use]
    pub fn routes(&self, source: &SourceName) -> bool {
        self.0.get(source).is_some_and(|routes| !routes.is_empty())
    }

    /// Every source an item written to `source` could land in: the source itself first,
    /// then each distinct route target in the order its entries are written.
    #[must_use]
    pub fn reachable(&self, source: &SourceName) -> Vec<SourceName> {
        let mut reachable = vec![source.clone()];
        for route in self.0.get(source).into_iter().flatten() {
            if !reachable.contains(&route.to) {
                reachable.push(route.to.clone());
            }
        }
        reachable
    }

    /// One source's routes, in order.
    #[must_use]
    pub fn of(&self, source: &SourceName) -> &[Route] {
        self.0.get(source).map_or(&[], Vec::as_slice)
    }

    /// Routes built in code, for a caller holding sources it did not resolve from a
    /// configuration document. Checked by the same rules a document's are.
    ///
    /// # Errors
    ///
    /// As [`check`].
    pub fn new(
        routes: BTreeMap<SourceName, Vec<(Vec<String>, SourceName)>>,
        configured: &[SourceName],
    ) -> Result<Self, ConfigError> {
        let mut built = BTreeMap::new();
        for (source, entries) in routes {
            let mut parsed = Vec::new();
            for (index, (patterns, to)) in entries.into_iter().enumerate() {
                let key = entry_key(&source, index);
                parsed.push(Route {
                    matcher: Matcher::Repositories(patterns_of(&key, patterns)?),
                    to,
                });
            }
            built.insert(source, parsed);
        }
        let routes = Self(built, BTreeMap::new());
        check(&routes, configured)?;
        Ok(routes)
    }

    /// One source's classification entries, built in code beside [`Routes::new`]'s: each
    /// sends every item of its classification to its `to`, tried before any repository
    /// entry. Checked by the same rules, and a `private` entry sending to a source this
    /// value does not hold as private is refused.
    ///
    /// # Errors
    ///
    /// As [`check`] and [`check_classified`].
    pub fn classified(
        mut self,
        source: SourceName,
        entries: Vec<(Classification, SourceName)>,
        configured: &[SourceName],
    ) -> Result<Self, ConfigError> {
        let routes = self.0.entry(source).or_default();
        let mut classified: Vec<Route> = entries
            .into_iter()
            .map(|(classification, to)| Route {
                matcher: Matcher::Classification(classification),
                to,
            })
            .collect();
        classified.append(routes);
        *routes = classified;
        check(&self, configured)?;
        check_classified(&self, &self.1)?;
        Ok(self)
    }

    pub(super) fn insert(&mut self, source: SourceName, routes: Vec<Route>) {
        if !routes.is_empty() {
            self.0.insert(source, routes);
        }
    }
}

fn entry_key(source: &SourceName, index: usize) -> String {
    format!("sources.{source}.routes.{index}")
}

/// Read one source's `routes` as a document, an environment variable or `--set` spells it.
///
/// A list is the document's spelling. A mapping keyed by each entry's index is what the
/// environment and `--set` produce — `--set sources.plans.routes.0.to=linear` — because
/// neither can spell a list of objects, and both are read the same way. One pattern where
/// a list of them is expected is read as that one-pattern list, for the reason
/// `default_sources` accepts one name.
pub(super) fn parse(source: &SourceName, value: &Value) -> Result<Vec<Route>, ConfigError> {
    let base = format!("sources.{source}.routes");
    let entries: Vec<(usize, &Value)> = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(entries) => entries.iter().enumerate().collect(),
        Value::Object(entries) => {
            let mut indexed = Vec::new();
            for (key, entry) in entries {
                let index = key.parse::<usize>().map_err(|_| {
                    ConfigError::setting(
                        format!("{base}.{key}"),
                        "a route is addressed by its index in the list, and this is not one",
                        format!(
                            "write the routes as a list, or address each entry by its index — \
                             `--set {base}.0.to=<source>`."
                        ),
                    )
                })?;
                indexed.push((index, entry));
            }
            indexed.sort_by_key(|(index, _)| *index);
            for (position, (index, _)) in indexed.iter().enumerate() {
                if *index != position {
                    return Err(ConfigError::setting(
                        format!("{base}.{index}"),
                        format!(
                            "the routes are numbered from 0 with no gaps, and entry {position} \
                             is missing"
                        ),
                        format!("set {base}.{position} as well, or renumber the entries."),
                    ));
                }
            }
            indexed
        }
        _ => {
            return Err(ConfigError::setting(
                base,
                "routes is a list of entries, each naming `repositories` and `to`",
                "write it as a list — see the README's section on routes.",
            ));
        }
    };
    entries
        .into_iter()
        .map(|(index, entry)| parse_entry(&entry_key(source, index), entry))
        .collect()
}

/// One entry, refused naming its key when it is not `{repositories, to}` or
/// `{classification, to}`.
fn parse_entry(key: &str, entry: &Value) -> Result<Route, ConfigError> {
    let Value::Object(fields) = entry else {
        return Err(ConfigError::setting(
            key,
            "a route is a mapping holding `to` and one of `repositories` or `classification`",
            "write the entry as `{repositories: [host/owner/*], to: <source>}` or \
             `{classification: private, to: <source>}`.",
        ));
    };
    if let Some(unknown) = fields
        .keys()
        .find(|field| !["repositories", "classification", "to"].contains(&field.as_str()))
    {
        return Err(ConfigError::setting(
            format!("{key}.{unknown}"),
            "unknown field; a route holds `to` and one of `repositories` or `classification`, \
             and nothing else",
            "remove it.",
        ));
    }
    let to = destination_of(key, fields)?;
    if let Some(classification) = fields.get("classification") {
        if fields.contains_key("repositories") {
            return Err(ConfigError::setting(
                key,
                "a route matches on `repositories` or on `classification`, and this one names \
                 both",
                "split it into two entries, in the order they should be tried.",
            ));
        }
        let classification = classification
            .as_str()
            .and_then(|spelled| spelled.parse::<Classification>().ok())
            .ok_or_else(|| {
                ConfigError::setting(
                    format!("{key}.classification"),
                    format!("{classification} is not a classification"),
                    "write `public` or `private`.",
                )
            })?;
        return Ok(Route {
            matcher: Matcher::Classification(classification),
            to,
        });
    }
    let patterns = match fields.get("repositories") {
        None => {
            return Err(ConfigError::setting(
                format!("{key}.repositories"),
                "a route names the repositories it matches, and this one names none",
                "list at least one pattern, such as `github.com/example-org/*`.",
            ));
        }
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Array(many)) => many
            .iter()
            .map(|pattern| {
                pattern.as_str().map(str::to_owned).ok_or_else(|| {
                    ConfigError::setting(
                        format!("{key}.repositories"),
                        format!("{pattern} is not a pattern; each is a string"),
                        "write each pattern as host/owner/name, quoted if need be.",
                    )
                })
            })
            .collect::<Result<_, _>>()?,
        Some(other) => {
            return Err(ConfigError::setting(
                format!("{key}.repositories"),
                format!("{other} is not a list of patterns"),
                "write it as a list of host/owner/name patterns.",
            ));
        }
    };
    Ok(Route {
        matcher: Matcher::Repositories(patterns_of(key, patterns)?),
        to,
    })
}

/// The `to` of one entry, refused naming it when it is absent or not a source name.
fn destination_of(
    key: &str,
    fields: &serde_json::Map<String, Value>,
) -> Result<SourceName, ConfigError> {
    match fields.get("to") {
        Some(Value::String(to)) => SourceName::new(to.clone()).map_err(|error| {
            ConfigError::setting(
                format!("{key}.to"),
                error.to_string(),
                "name a configured source; `onetaskgraph config show` lists them.",
            )
        }),
        _ => Err(ConfigError::setting(
            format!("{key}.to"),
            "a route names the configured source it sends an item to, and this one names \
             none",
            "set `to` to a configured source's name.",
        )),
    }
}

/// The patterns of one entry, refused naming it when there are none or one is malformed.
fn patterns_of(key: &str, patterns: Vec<String>) -> Result<Vec<RepositoryPattern>, ConfigError> {
    if patterns.is_empty() {
        return Err(ConfigError::setting(
            format!("{key}.repositories"),
            "a route names the repositories it matches, and this one's list is empty — an \
             entry matching nothing is a mistake rather than a rule",
            "list at least one pattern, such as `github.com/example-org/*`, or remove the entry.",
        ));
    }
    patterns
        .into_iter()
        .map(|pattern| {
            RepositoryPattern::new(pattern).map_err(|problem| {
                ConfigError::setting(
                    format!("{key}.repositories"),
                    problem,
                    "write the pattern as host/owner/name, using `*` for one whole segment — \
                     `github.com/example-org/*`.",
                )
            })
        })
        .collect()
}

/// Refuse an entry whose `to` names no configured source, the source itself, or a source
/// that routes on its own.
///
/// No chains: a route's target places what it is given where it is, so where an item lands
/// is always one lookup away from where it was sent, and no cycle can be written down.
///
/// # Errors
///
/// [`ConfigError::Setting`] naming the source and the entry.
pub(super) fn check(routes: &Routes, configured: &[SourceName]) -> Result<(), ConfigError> {
    for (source, entries) in &routes.0 {
        for (index, route) in entries.iter().enumerate() {
            let key = format!("{}.to", entry_key(source, index));
            if !configured.contains(&route.to) {
                return Err(ConfigError::setting(
                    key,
                    format!(
                        "route {index} of source {source} sends to {:?}, which no source is \
                         configured as",
                        route.to.as_str()
                    ),
                    format!(
                        "name one of the configured sources ({}), or configure {:?}.",
                        configured
                            .iter()
                            .map(SourceName::as_str)
                            .collect::<Vec<_>>()
                            .join(", "),
                        route.to.as_str()
                    ),
                ));
            }
            if &route.to == source {
                return Err(ConfigError::setting(
                    key,
                    format!(
                        "route {index} of source {source} sends to {source} itself, which is \
                         where an item no route matches already stays"
                    ),
                    "name another source, or remove the entry.",
                ));
            }
            if routes.routes(&route.to) {
                return Err(ConfigError::setting(
                    key,
                    format!(
                        "route {index} of source {source} sends to {}, which has routes of its \
                         own; a route never chains",
                        route.to
                    ),
                    format!(
                        "send to the source {} would route to directly, or remove {}'s routes.",
                        route.to, route.to
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Refuse a `classification: private` entry whose `to` is not declared private.
///
/// Such an entry could never be taken — a private item is placed only where it may be
/// written — so writing one down is a mistake about where private work goes, refused when
/// the configuration is read rather than discovered at a write.
///
/// # Errors
///
/// [`ConfigError::Setting`] naming the source and the entry.
pub(super) fn check_classified(
    routes: &Routes,
    declared: &BTreeMap<SourceName, SourceVisibility>,
) -> Result<(), ConfigError> {
    for (source, entries) in &routes.0 {
        for (index, route) in entries.iter().enumerate() {
            if route.classification() == Some(Classification::Private)
                && declared.get(&route.to) != Some(&SourceVisibility::Private)
            {
                return Err(ConfigError::setting(
                    format!("{}.to", entry_key(source, index)),
                    format!(
                        "route {index} of source {source} sends private items to {}, which is \
                         not declared private",
                        route.to
                    ),
                    format!(
                        "declare `visibility: private` on {} if its backend is private, or \
                         send private items to a source that is.",
                        route.to
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(text: &str) -> Repository {
        Repository::try_from(text.to_owned()).expect("an origin")
    }

    #[test]
    fn a_star_matches_exactly_one_whole_segment() {
        let pattern = RepositoryPattern::new("github.com/example-org/*").expect("a pattern");
        assert!(pattern.matches(&origin("github.com/example-org/api")));
        assert!(!pattern.matches(&origin("github.com/example-org/api/sub")));
        assert!(!pattern.matches(&origin("github.com/example-orgx/api")));
        assert!(!pattern.matches(&origin("gitlab.com/example-org/api")));
    }

    #[test]
    fn a_malformed_pattern_is_refused() {
        for bad in [
            "github.com/example-org",
            "github.com/example*/api",
            "https://github.com/a/b",
            "github.com//b",
            "github.com/a/b.git",
        ] {
            assert!(RepositoryPattern::new(bad).is_err(), "{bad} is refused");
        }
    }

    #[test]
    fn an_entry_matches_only_when_every_repository_does() {
        let route = Route {
            matcher: Matcher::Repositories(vec![
                RepositoryPattern::new("github.com/example-org/*").unwrap(),
            ]),
            to: SourceName::new("linear").unwrap(),
        };
        assert!(route.matches(&[origin("github.com/example-org/a")]));
        assert!(!route.matches(&[
            origin("github.com/example-org/a"),
            origin("github.com/nickderobertis/b")
        ]));
        assert!(!route.matches(&[]));
    }
}
