//! `project graph`: one project's tasks and the dependency edges between them, laid out
//! once and handed back in the two forms the verb prints.
//!
//! The layout is the contract, and it is stated whole in the verb's own `--help` and in the
//! README's "Drawing a project's dependency graph": the order the nodes come in, how they
//! are numbered, the order of the edges, which way `auto` lays the graph out, how a group is
//! read off a task's metadata and how a label is escaped. Everything below is a function of
//! what the project's source reports and of the request, so two calls over an unchanged
//! project agree byte for byte.
//!
//! Nothing is written down: the tasks are read through the source's own project listing and
//! each task's forward edges through its own dependency read, exactly as `task list
//! --project` and `task deps` read them, and the whole graph lives for one call.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::num::NonZeroU32;

use onetaskgraph_plugin_api::{ItemKind, Task};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::copy::{Level, forward_edges};
use super::{
    Engine, EngineError, Filters, Paging, ProjectSelector, Qualified, TaskRequest, qualify_endpoint,
};
use crate::GlobalId;

/// How many of a project's tasks one engine page asks for while the graph is read.
///
/// The widest page any bundled source serves, so a source answering from one listing of the
/// project is asked as few times as it can be.
const GRAPH_PAGE: NonZeroU32 = NonZeroU32::new(100).expect("100 is not zero");

/// The version of the document [`ProjectGraph`] serialises to, which its consumers read
/// before anything else in it.
pub const PROJECT_GRAPH_SCHEMA_VERSION: u32 = 1;

/// Which way a graph is laid out: top to bottom, or left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GraphDirection {
    /// Top to bottom: Mermaid's `flowchart TD`.
    Td,
    /// Left to right: Mermaid's `flowchart LR`.
    Lr,
}

impl GraphDirection {
    /// The word Mermaid's `flowchart` header takes for this direction.
    #[must_use]
    pub const fn mermaid(self) -> &'static str {
        match self {
            Self::Td => "TD",
            Self::Lr => "LR",
        }
    }

    /// The direction `auto` resolves to for a graph of `ranks` ranks whose widest holds
    /// `widest` tasks: left to right exactly when it is wider than it is deep.
    ///
    /// A graph much wider than it is deep is unreadable once a renderer fits it to a page
    /// top-down, and readable laid out the other way: on an 880-pixel page the smallest label
    /// of a 48-task plan over ranks 1/28/15/4 measured 1.6 pixels top-down and 11.5 left to
    /// right, while a 10-task plan over ranks 1/4/2/1/2 measured 11.5 top-down and 9.1 left to
    /// right.
    #[must_use]
    pub const fn auto(ranks: usize, widest: usize) -> Self {
        if widest > ranks { Self::Lr } else { Self::Td }
    }
}

/// What `project graph` is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGraphRequest {
    /// The project whose tasks are drawn.
    pub project: GlobalId,
    /// The direction to lay it out in, or `None` for `auto`.
    pub direction: Option<GraphDirection>,
    /// The task metadata key whose non-empty string value groups the project's tasks, or
    /// `None` for no groups.
    pub group_by: Option<String>,
}

/// One project's dependency graph, laid out: the document `project graph --format json`
/// prints, and everything its Mermaid form is drawn from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectGraph {
    /// The version of this document's shape; `1`.
    pub schema_version: u32,
    /// The project whose tasks these are.
    pub project: GlobalId,
    /// The direction the graph is laid out in — `auto` already resolved.
    pub direction: GraphDirection,
    /// The `--group-by` key, or `null` when none was given.
    pub group_by: Option<String>,
    /// Every task of the project in topological order, then every task outside it that one
    /// of them depends on, ordered by title and then qualified id.
    pub nodes: Vec<GraphNode>,
    /// Every dependency, ordered by its prerequisite's position and then its dependent's.
    pub edges: Vec<GraphEdge>,
}

/// One node of a [`ProjectGraph`]: one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphNode {
    /// The Mermaid form's node id: `n<k>` for the project's own tasks and `x<k>` for a task
    /// outside it, each counted from 1.
    pub key: String,
    /// The task's qualified id.
    pub id: GlobalId,
    /// The task's title, exactly as its source reports it — unescaped and unaltered.
    pub title: String,
    /// Whether the task is outside the project: `true` exactly for the `x<k>` nodes.
    pub external: bool,
    /// The task's group under `group_by`, or `null` — for a task with none, for every
    /// external task, and for every task when no key was given.
    pub group: Option<String>,
}

/// One edge of a [`ProjectGraph`], pointing from the task depended on to the task that
/// depends on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphEdge {
    /// The prerequisite: the task that has to finish first.
    pub from: GlobalId,
    /// The dependent: the project task that waits on it.
    pub to: GlobalId,
}

impl ProjectGraph {
    /// The graph as Mermaid flowchart text, every line ending in `\n`.
    ///
    /// Without a `group_by` key: the header, the project's node lines, the external node
    /// lines, the edge lines, and the `classDef` line when an external node was printed.
    /// With one, each group's members and the edges between two of them sit inside that
    /// group's `subgraph` block, ahead of everything else — mermaid 11's layout crashed on a
    /// 100-task plan grouped into 57 subgraphs whose edges were all declared after the
    /// blocks, and rendered the same graph with each block's own edges declared inside it.
    #[must_use]
    pub fn mermaid(&self) -> String {
        let key: HashMap<&GlobalId, &GraphNode> =
            self.nodes.iter().map(|node| (&node.id, node)).collect();
        let node_line = |node: &GraphNode, indent: &str| {
            if node.external {
                format!(
                    "{indent}{}[\"{} ({})\"]:::external\n",
                    node.key,
                    label(&node.title),
                    node.id
                )
            } else {
                format!("{indent}{}[\"{}\"]\n", node.key, label(&node.title))
            }
        };
        let edge_line = |edge: &GraphEdge, indent: &str| {
            format!(
                "{indent}{} --> {}\n",
                key[&edge.from].key, key[&edge.to].key
            )
        };
        let group_of = |id: &GlobalId| key[id].group.as_deref();
        let mut out = format!("flowchart {}\n", self.direction.mermaid());
        // Groups in the order their first member is drawn in, which is the order `nodes`
        // holds the project's tasks in.
        let mut groups: Vec<&str> = Vec::new();
        for group in self.nodes.iter().filter_map(|node| node.group.as_deref()) {
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        let inside = |edge: &GraphEdge| {
            group_of(&edge.from).is_some() && group_of(&edge.from) == group_of(&edge.to)
        };
        for (number, group) in groups.iter().enumerate() {
            out.push_str(&format!(
                "  subgraph g{}[\"{}\"]\n",
                number + 1,
                label(group)
            ));
            for node in self
                .nodes
                .iter()
                .filter(|node| node.group.as_deref() == Some(group))
            {
                out.push_str(&node_line(node, "    "));
            }
            for edge in self
                .edges
                .iter()
                .filter(|edge| inside(edge) && group_of(&edge.from) == Some(group))
            {
                out.push_str(&edge_line(edge, "    "));
            }
            out.push_str("  end\n");
        }
        for node in self.nodes.iter().filter(|node| node.group.is_none()) {
            out.push_str(&node_line(node, "  "));
        }
        for edge in self.edges.iter().filter(|edge| !inside(edge)) {
            out.push_str(&edge_line(edge, "  "));
        }
        if self.nodes.iter().any(|node| node.external) {
            out.push_str("  classDef external stroke-dasharray: 5 5\n");
        }
        out
    }
}

/// A title or a group value as a Mermaid label: each line break (`\r\n`, `\n` or `\r`) one
/// space, then `#`, `"`, `<` and `>` as the entity codes Mermaid reads, in that order — `#`
/// first, so the `#` each later code opens with is not escaped again. Nothing else changes.
#[must_use]
pub fn label(text: &str) -> String {
    text.replace("\r\n", " ")
        .replace(['\n', '\r'], " ")
        .replace('#', "#35;")
        .replace('"', "#quot;")
        .replace('<', "#lt;")
        .replace('>', "#gt;")
}

/// A task's group under `key`: its metadata value there when that is a non-empty string,
/// none when it is absent, `null` or `""`, and a refusal naming the task and the key for any
/// other JSON value.
fn group_of(task: &Qualified<Task>, key: &str) -> Result<Option<String>, EngineError> {
    match task.item.metadata.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(other) => Err(EngineError::GraphGroupNotText {
            task: task.id.to_string(),
            key: key.to_owned(),
            value: other.to_string(),
        }),
    }
}

/// The order a title tie is broken in: the title, then the qualified id, each by its bytes.
fn tie(title: &str, id: &GlobalId) -> (String, String) {
    (title.to_owned(), id.to_string())
}

impl Engine {
    /// One project's tasks and their dependency edges, laid out as `project graph` prints
    /// them.
    ///
    /// Every page of the project's tasks is read, then each task's forward edges, then the
    /// title of each task outside the project one of them depends on. Any read failing fails
    /// the whole graph rather than answering part of it.
    ///
    /// # Errors
    ///
    /// [`EngineError::NoSuchProject`] for an id naming no project;
    /// [`EngineError::NoSuchTask`] for a dependency naming a task its source does not hold;
    /// [`EngineError::GraphGroupNotText`] for a group value that is not a string;
    /// [`EngineError::DependencyCycle`] for tasks of the project that depend on each other
    /// in a cycle, which no order can put each after what it depends on; and the engine's
    /// own errors for a source that is not configured, could not be built or failed a read.
    pub async fn project_graph(
        &self,
        request: &ProjectGraphRequest,
    ) -> Result<ProjectGraph, EngineError> {
        let source = self.built(&request.project.source)?;
        let name = source.name().clone();
        let failed = |error| EngineError::SourceFailed {
            name: name.to_string(),
            error,
        };
        let held = source
            .source()
            .get_project(&request.project.native)
            .await
            .map_err(failed)?;
        if held.is_none() {
            return Err(EngineError::NoSuchProject {
                id: request.project.to_string(),
            });
        }
        let tasks = self.graph_tasks(&request.project).await?;
        let groups = match &request.group_by {
            Some(key) => tasks
                .iter()
                .map(|task| group_of(task, key))
                .collect::<Result<Vec<_>, _>>()?,
            None => vec![None; tasks.len()],
        };

        // Every dependency, as (prerequisite, dependent). A set, because a source reporting
        // one relationship twice still has one edge to draw.
        let mut edges: HashSet<(GlobalId, GlobalId)> = HashSet::new();
        for task in &tasks {
            let read = forward_edges(source, &task.id.native, Level::Task)
                .await
                .map_err(|error| match error {
                    EngineError::SourceRefused { name, error } => {
                        EngineError::SourceFailed { name, error }
                    }
                    other => other,
                })?;
            for edge in read {
                let to = qualify_endpoint(&name, edge.to);
                // A task that depends on a whole project is not drawn: the graph is of tasks.
                if to.kind == ItemKind::Task {
                    edges.insert((to.id, task.id.clone()));
                }
            }
        }

        let own: HashMap<&GlobalId, usize> = tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (&task.id, index))
            .collect();
        let order = topological(&tasks, &edges, &own)?;

        // The rank of each project task, in `order`: one more than the highest rank among
        // its in-project prerequisites, and 0 when it has none.
        let mut rank = vec![0_usize; tasks.len()];
        for &index in &order {
            let id = &tasks[index].id;
            rank[index] = edges
                .iter()
                .filter(|(_, dependent)| dependent == id)
                .filter_map(|(prerequisite, _)| own.get(prerequisite))
                .map(|&before| rank[before] + 1)
                .max()
                .unwrap_or(0);
        }
        let ranks = rank.iter().max().map_or(0, |highest| highest + 1);
        let mut widths = vec![0_usize; ranks];
        for &at in &rank {
            widths[at] += 1;
        }
        let widest = widths.iter().copied().max().unwrap_or(0);
        let direction = request
            .direction
            .unwrap_or_else(|| GraphDirection::auto(ranks, widest));

        let mut externals: Vec<(String, GlobalId)> = Vec::new();
        let outside: HashSet<&GlobalId> = edges
            .iter()
            .map(|(prerequisite, _)| prerequisite)
            .filter(|prerequisite| !own.contains_key(prerequisite))
            .collect();
        for id in outside {
            externals.push((self.graph_title(id).await?, id.clone()));
        }
        externals.sort_by_key(|(title, id)| tie(title, id));

        let mut nodes: Vec<GraphNode> = order
            .iter()
            .enumerate()
            .map(|(position, &index)| GraphNode {
                key: format!("n{}", position + 1),
                id: tasks[index].id.clone(),
                title: tasks[index].item.title.clone(),
                external: false,
                group: groups[index].clone(),
            })
            .collect();
        nodes.extend(
            externals
                .into_iter()
                .enumerate()
                .map(|(position, (title, id))| GraphNode {
                    key: format!("x{}", position + 1),
                    id,
                    title,
                    external: true,
                    group: None,
                }),
        );
        let position: HashMap<&GlobalId, usize> = nodes
            .iter()
            .enumerate()
            .map(|(at, node)| (&node.id, at))
            .collect();
        let mut drawn: Vec<GraphEdge> = edges
            .iter()
            .map(|(prerequisite, dependent)| GraphEdge {
                from: prerequisite.clone(),
                to: dependent.clone(),
            })
            .collect();
        drawn.sort_by_key(|edge| (position[&edge.from], position[&edge.to]));

        Ok(ProjectGraph {
            schema_version: PROJECT_GRAPH_SCHEMA_VERSION,
            project: request.project.clone(),
            direction,
            group_by: request.group_by.clone(),
            nodes,
            edges: drawn,
        })
    }

    /// Every task the source holds in `project`, across every page it answers.
    ///
    /// Paged by this engine's own token, as a copy reads a project's tasks; a source failing
    /// any page fails the graph.
    async fn graph_tasks(&self, project: &GlobalId) -> Result<Vec<Qualified<Task>>, EngineError> {
        let mut request = TaskRequest {
            sources: vec![project.source.clone()],
            filters: Filters::default(),
            project: ProjectSelector::Qualified(project.clone()),
            priorities: Vec::new(),
            commented_since: None,
            metadata: Vec::new(),
            origin: None,
            include_members: false,
            paging: Paging {
                limit: GRAPH_PAGE,
                token: None,
            },
        };
        let mut tasks = Vec::new();
        loop {
            let response = self.tasks(&request).await?;
            if let Some(failure) = response.errors.into_iter().next() {
                return Err(EngineError::SourceFailed {
                    name: failure.source.to_string(),
                    error: failure.error,
                });
            }
            tasks.extend(response.items);
            match response.next {
                Some(token) if Some(&token) != request.paging.token.as_ref() => {
                    request.paging.token = Some(token);
                }
                Some(_) => {
                    return Err(EngineError::SourceFailed {
                        name: project.source.to_string(),
                        error: onetaskgraph_plugin_api::SourceError::Malformed {
                            message: "the tasks of a project were being read for a graph, and \
                                      the source answered the same page twice"
                                .into(),
                        },
                    });
                }
                None => return Ok(tasks),
            }
        }
    }

    /// The title of one task outside the project, read from its own source.
    async fn graph_title(&self, id: &GlobalId) -> Result<String, EngineError> {
        let response = self.task(id).await?;
        if let Some(failure) = response.errors.into_iter().next() {
            return Err(EngineError::SourceFailed {
                name: failure.source.to_string(),
                error: failure.error,
            });
        }
        response
            .items
            .into_iter()
            .next()
            .map(|task| task.item.title)
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })
    }
}

/// The project's tasks, as indices into `tasks`, in topological order: each after every task
/// of the project it depends on, ties broken by title and then qualified id.
fn topological(
    tasks: &[Qualified<Task>],
    edges: &HashSet<(GlobalId, GlobalId)>,
    own: &HashMap<&GlobalId, usize>,
) -> Result<Vec<usize>, EngineError> {
    let inside: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|(prerequisite, dependent)| {
            Some((*own.get(prerequisite)?, *own.get(dependent)?))
        })
        .collect();
    let mut waiting = vec![0_usize; tasks.len()];
    for &(_, dependent) in &inside {
        waiting[dependent] += 1;
    }
    let key = |index: usize| (tie(&tasks[index].item.title, &tasks[index].id), index);
    let mut ready: BTreeSet<((String, String), usize)> = (0..tasks.len())
        .filter(|&index| waiting[index] == 0)
        .map(key)
        .collect();
    let mut order = Vec::with_capacity(tasks.len());
    while let Some(next) = ready.pop_first() {
        let index = next.1;
        order.push(index);
        for &(_, dependent) in inside.iter().filter(|(before, _)| *before == index) {
            waiting[dependent] -= 1;
            if waiting[dependent] == 0 {
                ready.insert(key(dependent));
            }
        }
    }
    if order.len() < tasks.len() {
        let mut stuck: Vec<String> = (0..tasks.len())
            .filter(|index| !order.contains(index))
            .map(|index| tasks[index].id.to_string())
            .collect();
        stuck.sort();
        return Err(EngineError::DependencyCycle {
            tasks: stuck.join(", "),
        });
    }
    Ok(order)
}
