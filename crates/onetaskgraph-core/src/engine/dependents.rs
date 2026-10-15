//! A dependency of a task in a source not declared private on a task in a source that is,
//! kept on the private side.
//!
//! A write anywhere not declared private withholds every id naming an item of a source that
//! is (see [`Engine::withholds`]), so the public task cannot hold such an edge itself: holding
//! it would name the private task where the private task must not be named. The private task
//! may name the public one freely, so the copy that withholds the edge records it there
//! instead, under [`DEPENDED_ON_BY_KEY`], and every read of the graph — `task deps` from
//! either end, `project graph`, and so the plan a caller loads from them — merges what the
//! private side records with what each source holds natively.
//!
//! The record lives in the private source's own store, at the user's explicit request — a
//! copy — and is read back only by asking that source again, so it is a write into a plugin
//! rather than state the engine keeps.

use std::collections::{BTreeMap, BTreeSet};

use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, DependencyKind, ItemKind, MetadataMatch, NativeId,
    SourceName, Task,
};
use serde_json::{Map, Value, json};

use super::local::ProjectSelector;
use super::{Engine, EngineError, Filters, Paging, TaskRequest};
use crate::GlobalId;
use crate::config::SourceVisibility;

/// The reserved key a task of a source declared private records, under it, the tasks of
/// sources not declared private that depend on it.
///
/// A JSON object keyed by each dependent's qualified id, whose value is that dependent as a
/// qualified endpoint — `{"github:12": {"id": "github:12", "kind": "task"}}` — so a read can
/// ask a source, by an ordinary metadata match, which of its tasks one id depends on. Store
/// owned like every key in the namespace: a copy keeps it, nothing a caller types can name it,
/// and a copy never carries one item's entry onto another.
pub(crate) const DEPENDED_ON_BY_KEY: &str = "onetaskgraph.depended_on_by";

/// The dependents `metadata` records, by qualified id, or a message saying what about the
/// entry cannot be read.
///
/// # Errors
///
/// An entry that is not an object, or holds a value that is not that same id as a task
/// endpoint: an edge that cannot be read is refused rather than read as none.
pub(crate) fn recorded(metadata: &BTreeMap<String, Value>) -> Result<BTreeSet<String>, String> {
    let Some(held) = metadata.get(DEPENDED_ON_BY_KEY) else {
        return Ok(BTreeSet::new());
    };
    let Some(entries) = held.as_object() else {
        return Err(format!(
            "{DEPENDED_ON_BY_KEY} is not an object of the tasks that depend on it"
        ));
    };
    entries
        .iter()
        .map(|(id, endpoint)| {
            let names_itself = endpoint.get("id").and_then(Value::as_str) == Some(id.as_str())
                && endpoint.get("kind").and_then(Value::as_str) == Some("task");
            match id.parse::<GlobalId>() {
                Ok(_) if names_itself => Ok(id.clone()),
                _ => Err(format!(
                    "{DEPENDED_ON_BY_KEY} records {id:?} as {endpoint}, which is not that \
                     qualified task id as a task endpoint"
                )),
            }
        })
        .collect()
}

/// `metadata` recording exactly `dependents`, and without the key when there are none.
pub(crate) fn record(metadata: &mut BTreeMap<String, Value>, dependents: &BTreeSet<String>) {
    if dependents.is_empty() {
        metadata.remove(DEPENDED_ON_BY_KEY);
        return;
    }
    let entries: Map<String, Value> = dependents
        .iter()
        .map(|id| (id.clone(), json!({"id": id, "kind": "task"})))
        .collect();
    metadata.insert(DEPENDED_ON_BY_KEY.to_owned(), Value::Object(entries));
}

/// The match a task recording `dependent` among its dependents holds.
pub(crate) fn naming(dependent: &GlobalId) -> MetadataMatch {
    let id = dependent.to_string();
    MetadataMatch::new(DEPENDED_ON_BY_KEY, vec![id.clone(), "id".to_owned()], id)
        .expect("the key and a qualified id are never empty")
}

/// The edge `dependent` → `prerequisite` that a task of the prerequisite's source records,
/// with the prerequisite native to the source reporting it.
pub(crate) fn edge(dependent: &GlobalId, prerequisite: NativeId) -> DependencyEdge {
    DependencyEdge {
        from: DependencyEndpoint::new(dependent.to_string(), ItemKind::Task)
            .expect("a qualified id is a valid endpoint"),
        to: DependencyEndpoint::from_native(prerequisite, ItemKind::Task),
        kind: DependencyKind::Blocks,
    }
}

impl Engine {
    /// Every configured source that may record a dependent from `near`: each one declared
    /// private but `near` itself, and none at all when `near` is declared private, because a
    /// private task holds its own edges wherever they lead.
    pub(crate) fn recording_sources(&self, near: &SourceName) -> Vec<SourceName> {
        if self.declared(near) == SourceVisibility::Private {
            return Vec::new();
        }
        self.sources
            .iter()
            .map(|source| source.name().clone())
            .filter(|name| name != near && self.declared(name) == SourceVisibility::Private)
            .collect()
    }

    /// Every task of a source declared private that records `dependent` as depending on it,
    /// read whole from each of [`recording_sources`](Self::recording_sources).
    ///
    /// # Errors
    ///
    /// A source that could not be built or failed a read: a prerequisite that cannot be read
    /// is refused rather than reported absent.
    pub(crate) async fn recorded_prerequisites(
        &self,
        dependent: &GlobalId,
    ) -> Result<Vec<GlobalId>, EngineError> {
        let mut prerequisites = Vec::new();
        for source in self.recording_sources(&dependent.source) {
            let mut request = TaskRequest {
                sources: vec![source],
                filters: Filters::default(),
                project: ProjectSelector::Any,
                priorities: Vec::new(),
                commented_since: None,
                metadata: vec![naming(dependent)],
                origin: None,
                include_members: false,
                paging: Paging {
                    limit: super::copy::PROJECT_PAGE,
                    token: None,
                },
            };
            loop {
                let response = self.tasks(&request).await?;
                if let Some(failure) = response.errors.into_iter().next() {
                    return Err(EngineError::SourceFailed {
                        name: failure.source.to_string(),
                        error: failure.error,
                    });
                }
                prerequisites.extend(response.items.into_iter().map(|task| task.id));
                match response.next {
                    Some(token) => request.paging.token = Some(token),
                    None => break,
                }
            }
        }
        Ok(prerequisites)
    }
}

/// The edges `task`'s own record says depend on it, each from a qualified dependent to
/// `task` native to its source.
///
/// # Errors
///
/// As [`recorded`].
pub(crate) fn recorded_edges(task: &Task) -> Result<Vec<DependencyEdge>, String> {
    recorded(&task.metadata)?
        .iter()
        .map(|dependent| {
            let dependent: GlobalId = dependent.parse().map_err(|error| format!("{error}"))?;
            Ok(edge(&dependent, task.id.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::{DEPENDED_ON_BY_KEY, naming, record, recorded};
    use crate::GlobalId;

    /// What one write records reads back, and the match for each dependent holds of it.
    #[test]
    fn a_recorded_dependent_reads_back_and_is_found_by_its_match() {
        let dependents: BTreeSet<String> = ["board:12", "notes:roll.out"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut metadata = BTreeMap::new();
        record(&mut metadata, &dependents);
        assert_eq!(recorded(&metadata), Ok(dependents.clone()));
        for dependent in &dependents {
            let dependent: GlobalId = dependent.parse().expect("a qualified id");
            assert!(naming(&dependent).holds(&metadata), "{metadata:?}");
        }
        assert!(!naming(&"board:13".parse().expect("an id")).holds(&metadata));
        record(&mut metadata, &BTreeSet::new());
        assert!(metadata.is_empty(), "no dependents leaves no key");
    }

    /// An entry that does not say what it names is refused, never read as no dependents.
    #[test]
    fn an_entry_that_is_not_its_own_task_endpoint_is_refused() {
        for held in [
            json!(["board:12"]),
            json!({"board:12": "board:12"}),
            json!({"board:12": {"id": "board:13", "kind": "task"}}),
            json!({"board:12": {"id": "board:12", "kind": "project"}}),
            json!({"unqualified": {"id": "unqualified", "kind": "task"}}),
        ] {
            let metadata = BTreeMap::from([(DEPENDED_ON_BY_KEY.to_owned(), held.clone())]);
            assert!(recorded(&metadata).is_err(), "{held} is refused");
        }
    }
}
