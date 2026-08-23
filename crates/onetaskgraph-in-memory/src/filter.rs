//! In-source predicate evaluation.
//!
//! Each function here is applied only when the configured [`Support`] for that
//! predicate is `Native`. When it is `Unsupported` the caller skips the function
//! entirely and the wider result set is returned — rule 2 of the contract.

use onetaskgraph_plugin_api::{Label, LabelFilter, StatusCategory, TextFields, TextQuery};

/// Whether `labels` satisfies `filter`, matching by name, case-insensitively.
pub(crate) fn labels_match(labels: &[Label], filter: &LabelFilter) -> bool {
    let held: Vec<String> = labels.iter().map(|l| l.name.to_lowercase()).collect();
    let holds = |name: &String| held.contains(&name.to_lowercase());

    if !filter.any_of.is_empty() && !filter.any_of.iter().any(holds) {
        return false;
    }
    if !filter.all_of.iter().all(holds) {
        return false;
    }
    !filter.none_of.iter().any(holds)
}

/// Whether `category` is one of `statuses`. An empty list means unfiltered.
pub(crate) fn status_matches(category: StatusCategory, statuses: &[StatusCategory]) -> bool {
    statuses.is_empty() || statuses.contains(&category)
}

/// Whether `title`/`content` satisfies `query`, matching case-insensitively.
///
/// Only called once the caller has established that this source applies every
/// half `query.fields` asks about; see `text_survives`.
pub(crate) fn text_matches(title: &str, content: Option<&str>, query: &TextQuery) -> bool {
    let terms = query.terms.to_lowercase();
    let in_title = title.to_lowercase().contains(&terms);
    let in_content = content.is_some_and(|c| c.to_lowercase().contains(&terms));
    match query.fields {
        TextFields::Title => in_title,
        TextFields::Content => in_content,
        TextFields::TitleOrContent => in_title || in_content,
    }
}
