//! A source's `routes`: where an item written to it goes instead, chosen by the item's
//! repositories.
//!
//! Engine-owned rather than plugin-owned: `routes` sits beside `plugin` and `config` in a
//! source's entry, and a plugin's own block never sees it. Every writer reaches the store,
//! so the rule is held here once rather than restated, and drifting, in each of them.
//!
//! An entry matches an item when **every** one of the item's repositories matches at least
//! one of its patterns; an item with no repositories matches none. The first entry that
//! matches wins, and no match leaves the item in the source itself.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{Repository, SourceName};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ConfigError;

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
    /// Returns why when the pattern is not three or more `/`-separated segments, when a
    /// segment is empty, `.` or `..`, or holds a `*` beside other characters, or when it
    /// carries a scheme, a `.git` suffix or whitespace — anything a normalized origin
    /// could never match.
    pub fn new(pattern: impl Into<String>) -> Result<Self, String> {
        let pattern = pattern.into();
        let segments: Vec<&str> = pattern.split('/').collect();
        let valid = !pattern.contains("://")
            && !pattern.ends_with(".git")
            && !pattern.chars().any(char::is_whitespace)
            && segments.len() >= 3
            && segments.iter().all(|segment| {
                !segment.is_empty()
                    && *segment != "."
                    && *segment != ".."
                    && (*segment == "*" || !segment.contains('*'))
            });
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
    repositories: Vec<RepositoryPattern>,
    to: SourceName,
}

impl Route {
    /// The patterns, in the order written. Never empty.
    #[must_use]
    pub fn repositories(&self) -> &[RepositoryPattern] {
        &self.repositories
    }

    /// The configured source an item this entry matches goes to.
    #[must_use]
    pub fn to(&self) -> &SourceName {
        &self.to
    }

    /// Whether every one of `repositories` matches one of this entry's patterns — `false`
    /// for an item naming none.
    #[must_use]
    pub fn matches(&self, repositories: &[Repository]) -> bool {
        !repositories.is_empty()
            && repositories.iter().all(|origin| {
                self.repositories
                    .iter()
                    .any(|pattern| pattern.matches(origin))
            })
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

/// Every configured source's routes, by source name. A source with none has no entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routes(BTreeMap<SourceName, Vec<Route>>);

impl Routes {
    /// Where an item with `repositories` written to `source` lands.
    #[must_use]
    pub fn place(&self, source: &SourceName, repositories: &[Repository]) -> Placement {
        self.0
            .get(source)
            .and_then(|routes| {
                routes
                    .iter()
                    .enumerate()
                    .find(|(_, route)| route.matches(repositories))
            })
            .map_or_else(
                || Placement {
                    destination: source.clone(),
                    route: None,
                },
                |(index, route)| Placement {
                    destination: route.to.clone(),
                    route: Some(u32::try_from(index).unwrap_or(u32::MAX)),
                },
            )
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
                    repositories: patterns_of(&key, patterns)?,
                    to,
                });
            }
            built.insert(source, parsed);
        }
        let routes = Self(built);
        check(&routes, configured)?;
        Ok(routes)
    }

    pub(super) fn insert(&mut self, source: SourceName, routes: Vec<Route>) {
        if !routes.is_empty() {
            self.0.insert(source, routes);
        }
    }
}

/// The dotted key one entry is refused under.
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

/// One entry, refused naming its key when it is not `{repositories, to}`.
fn parse_entry(key: &str, entry: &Value) -> Result<Route, ConfigError> {
    let Value::Object(fields) = entry else {
        return Err(ConfigError::setting(
            key,
            "a route is a mapping holding `repositories` and `to`",
            "write the entry as `{repositories: [host/owner/*], to: <source>}`.",
        ));
    };
    if let Some(unknown) = fields
        .keys()
        .find(|field| !["repositories", "to"].contains(&field.as_str()))
    {
        return Err(ConfigError::setting(
            format!("{key}.{unknown}"),
            "unknown field; a route holds `repositories` and `to` and nothing else",
            "remove it.",
        ));
    }
    let patterns = match fields.get("repositories") {
        None => {
            return Err(ConfigError::setting(
                format!("{key}.repositories"),
                "a route names the repositories it matches, and this one names none",
                "list at least one pattern, such as `github.com/petsinc/*`.",
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
    let to = match fields.get("to") {
        Some(Value::String(to)) => SourceName::new(to.clone()).map_err(|error| {
            ConfigError::setting(
                format!("{key}.to"),
                error.to_string(),
                "name a configured source; `onetaskgraph config show` lists them.",
            )
        })?,
        _ => {
            return Err(ConfigError::setting(
                format!("{key}.to"),
                "a route names the configured source it sends an item to, and this one names \
                 none",
                "set `to` to a configured source's name.",
            ));
        }
    };
    Ok(Route {
        repositories: patterns_of(key, patterns)?,
        to,
    })
}

/// The patterns of one entry, refused naming it when there are none or one is malformed.
fn patterns_of(key: &str, patterns: Vec<String>) -> Result<Vec<RepositoryPattern>, ConfigError> {
    if patterns.is_empty() {
        return Err(ConfigError::setting(
            format!("{key}.repositories"),
            "a route names the repositories it matches, and this one's list is empty — an \
             entry matching nothing is a mistake rather than a rule",
            "list at least one pattern, such as `github.com/petsinc/*`, or remove the entry.",
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
                     `github.com/petsinc/*`.",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(text: &str) -> Repository {
        Repository::try_from(text.to_owned()).expect("an origin")
    }

    #[test]
    fn a_star_matches_exactly_one_whole_segment() {
        let pattern = RepositoryPattern::new("github.com/petsinc/*").expect("a pattern");
        assert!(pattern.matches(&origin("github.com/petsinc/api")));
        assert!(!pattern.matches(&origin("github.com/petsinc/api/sub")));
        assert!(!pattern.matches(&origin("github.com/petsincx/api")));
        assert!(!pattern.matches(&origin("gitlab.com/petsinc/api")));
    }

    #[test]
    fn a_malformed_pattern_is_refused() {
        for bad in [
            "github.com/petsinc",
            "github.com/pets*/api",
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
            repositories: vec![RepositoryPattern::new("github.com/petsinc/*").unwrap()],
            to: SourceName::new("linear").unwrap(),
        };
        assert!(route.matches(&[origin("github.com/petsinc/a")]));
        assert!(!route.matches(&[
            origin("github.com/petsinc/a"),
            origin("github.com/nickderobertis/b")
        ]));
        assert!(!route.matches(&[]));
    }
}
