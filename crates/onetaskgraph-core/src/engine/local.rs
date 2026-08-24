//! The predicates the engine applies itself, over the wider set a source returned.
//!
//! Every function here runs only for a predicate the source declared
//! [`Unsupported`](onetaskgraph_plugin_api::Support::Unsupported), which is the engine's
//! side of the contract — rule 3. What makes narrowing here sound is the source's side,
//! rule 2: a source *ignores* such a predicate and returns the wider set. So the rows
//! this drops are rows the caller asked not to see, and the rows it keeps are all the
//! rows there were. A source that narrowed for a predicate it declared unsupported would
//! break rule 2, and nothing above the plugin could tell.
//!
//! This is not a copy of a plugin's filtering for the sake of it. A source that applies
//! a predicate natively never reaches this code, and a plugin's own evaluation is behind
//! its own crate boundary — the engine may not reach into one, and the moment it did,
//! the answer would depend on which plugin happened to be first in the list.

use onetaskgraph_plugin_api::{
    Label, LabelFilter, NativeId, Project, ProjectFilter, StatusCategory, Task, TextFields,
    TextQuery,
};

/// The task predicates this source left to the engine.
///
/// Each field is `None`/empty when the source applied that predicate itself, so a
/// filter built from a fully native source keeps everything and costs one comparison
/// per row.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LocalTasks {
    /// Label membership, when the source does not filter by label.
    pub labels: Option<LabelFilter>,
    /// Status categories, when the source does not filter by status.
    pub statuses: Vec<StatusCategory>,
    /// Free text, when the source does not search every field the query names.
    pub text: Option<TextQuery>,
    /// The owning project, when the source does not filter by it.
    pub project: Option<ProjectFilter>,
}

impl LocalTasks {
    /// Whether `task` survives every predicate left to the engine.
    pub fn keeps(&self, task: &Task) -> bool {
        if let Some(filter) = &self.labels
            && !labels_match(&task.labels, filter)
        {
            return false;
        }
        if !status_matches(task.status.category, &self.statuses) {
            return false;
        }
        if let Some(query) = &self.text
            && !text_matches(&task.title, task.content.as_deref(), query)
        {
            return false;
        }
        match &self.project {
            None | Some(ProjectFilter::Any) => true,
            Some(ProjectFilter::Orphans) => task.project.is_none(),
            Some(ProjectFilter::Is(id)) => task.project.as_ref() == Some(id),
        }
    }
}

/// The project predicates this source left to the engine.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LocalProjects {
    /// Label membership, when the source does not filter by label.
    pub labels: Option<LabelFilter>,
    /// Status categories, when the source does not filter by status.
    pub statuses: Vec<StatusCategory>,
    /// Free text, when the source does not search every field the query names.
    pub text: Option<TextQuery>,
}

impl LocalProjects {
    /// Whether `project` survives every predicate left to the engine.
    pub fn keeps(&self, project: &Project) -> bool {
        if let Some(filter) = &self.labels
            && !labels_match(&project.labels, filter)
        {
            return false;
        }
        if !status_matches(project.status.category, &self.statuses) {
            return false;
        }
        match &self.text {
            None => true,
            Some(query) => text_matches(&project.title, project.content.as_deref(), query),
        }
    }
}

/// Whether `labels` satisfies `filter`, matching by name, case-insensitively.
///
/// By name rather than by id because a label id is per-source and a user filtering
/// across sources types a word — the reason [`LabelFilter`] is spelled in names at all.
pub(crate) fn labels_match(labels: &[Label], filter: &LabelFilter) -> bool {
    let held: Vec<String> = labels
        .iter()
        .map(|label| label.name.to_lowercase())
        .collect();
    let holds = |name: &String| held.contains(&name.to_lowercase());

    if !filter.any_of.is_empty() && !filter.any_of.iter().any(holds) {
        return false;
    }
    if !filter.all_of.iter().all(holds) {
        return false;
    }
    !filter.none_of.iter().any(holds)
}

/// An empty list is unfiltered rather than "keeps nothing", which is what makes
/// `statuses: Vec<StatusCategory>` able to spell "no status filter at all".
pub(crate) fn status_matches(category: StatusCategory, statuses: &[StatusCategory]) -> bool {
    statuses.is_empty() || statuses.contains(&category)
}

/// Whether `title`/`content` satisfies `query`, matching case-insensitively.
pub(crate) fn text_matches(title: &str, content: Option<&str>, query: &TextQuery) -> bool {
    let terms = query.terms.to_lowercase();
    let in_title = title.to_lowercase().contains(&terms);
    let in_content = content.is_some_and(|body| body.to_lowercase().contains(&terms));
    match query.fields {
        TextFields::Title => in_title,
        TextFields::Content => in_content,
        TextFields::TitleOrContent => in_title || in_content,
    }
}

/// Which project a task must belong to, as a command line names it.
///
/// Qualified (`work:PROJ-1`) or bare (`PROJ-1`), because both are things a user types
/// and they mean different queries: a qualified id names one project of one source and
/// restricts the query to it, while a bare one is a native id every selected source is
/// asked about. Which of the two a string is depends on whether its prefix names a
/// **configured source**, so a native id full of colons is still a native id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProjectSelector {
    /// No constraint.
    #[default]
    Any,
    /// Only tasks belonging to no project at all.
    Orphans,
    /// One project of one source.
    Qualified(crate::GlobalId),
    /// A native id, asked of every selected source.
    Native(NativeId),
}
