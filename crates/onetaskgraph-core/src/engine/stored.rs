//! What a destination stores of the content it is written, on both sides of the write.
//!
//! Before: the destination's declared rewrite —
//! [`TaskSource::stored_form`](onetaskgraph_plugin_api::TaskSource::stored_form) — applied to the
//! content, so what is sent is what the backend keeps, with a rendering's `body_digest`
//! recorded over it where its provenance vouched for the content it started from. After: the
//! content the write's own answer says the backend stores, compared with what was sent byte for
//! byte, a difference being a rewrite the destination did not declare.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{MetadataRecord, NativeId};
use serde_json::Value;

use super::EngineError;
use super::copy::restamped;
use crate::GlobalId;
use crate::resolve::ResolvedSource;
use crate::template::TemplateProvenance;

/// Make `content` what `destination` will store, recording a rendering's provenance in
/// `metadata` over the rewritten content where it vouched for `content` — a hand edit's entry is
/// carried as it was, never laundered.
///
/// A destination declaring no rewrite leaves both exactly as they were.
pub(super) fn apply(
    destination: &ResolvedSource,
    content: &mut Option<String>,
    metadata: &mut BTreeMap<String, Value>,
) {
    let Some(read) = content.as_deref() else {
        return;
    };
    let written = destination.source().stored_form(read);
    if written == read {
        return;
    }
    if let Some(provenance) = restamped(metadata, read, &written) {
        metadata.insert(TemplateProvenance::KEY.to_owned(), provenance.to_value());
    }
    *content = Some(written);
}

/// Let go of what `destination` still holds from an earlier write of the `record` `id`, before
/// a write of it whose answer is the one to compare.
pub(super) fn forget(destination: &ResolvedSource, record: MetadataRecord, id: &NativeId) {
    let _ = destination.source().stored_content(record, id);
}

/// The refusal owed when the write of the `record` `id` just sent `sent` and its answer says
/// `destination` stores something else; `None` when they agree, or when the destination said
/// nothing about what it stores.
pub(super) fn diverged(
    destination: &ResolvedSource,
    record: MetadataRecord,
    id: &NativeId,
    sent: Option<&str>,
) -> Option<EngineError> {
    let stored = destination.source().stored_content(record, id)?;
    let sent = sent.unwrap_or_default();
    if stored == sent {
        return None;
    }
    let mut sent_lines = sent.split('\n');
    let mut stored_lines = stored.split('\n');
    let mut line = 0;
    loop {
        line += 1;
        let (sent_line, stored_line) = (sent_lines.next(), stored_lines.next());
        if sent_line != stored_line {
            return Some(EngineError::StoredContentDiverged {
                item: GlobalId::new(destination.name().clone(), id.clone()),
                line,
                sent: sent_line.unwrap_or_default().to_owned(),
                stored: stored_line.unwrap_or_default().to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
    use onetaskgraph_plugin_api::{
        Capabilities, DependencyEdge, Direction, Health, Label, MetadataRecord, NativeId, Page,
        PageRequest, Project, ProjectQuery, SourceError, SourceName, Task, TaskQuery, TaskSource,
    };
    use serde_json::{Value, json};

    use super::{apply, diverged, forget};
    use crate::engine::EngineError;
    use crate::resolve::ResolvedSource;
    use crate::template::body_digest;

    /// An empty source whose backend upper-cases what it is written and answers `stored` as
    /// what it now holds.
    struct Shouting {
        inner: InMemorySource,
        stored: Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl TaskSource for Shouting {
        fn kind(&self) -> &'static str {
            "shouting"
        }
        fn capabilities(&self) -> Capabilities {
            self.inner.capabilities()
        }
        fn stored_form(&self, content: &str) -> String {
            content.to_uppercase()
        }
        fn stored_content(&self, _record: MetadataRecord, _id: &NativeId) -> Option<String> {
            self.stored.lock().expect("the answer").take()
        }
        async fn health(&self) -> Result<Health, SourceError> {
            self.inner.health().await
        }
        async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
            self.inner.get_task(id).await
        }
        async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
            self.inner.get_project(id).await
        }
        async fn query_tasks(
            &self,
            query: &TaskQuery,
            page: &PageRequest,
        ) -> Result<Page<Task>, SourceError> {
            self.inner.query_tasks(query, page).await
        }
        async fn query_projects(
            &self,
            query: &ProjectQuery,
            page: &PageRequest,
        ) -> Result<Page<Project>, SourceError> {
            self.inner.query_projects(query, page).await
        }
        async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
            self.inner.labels(page).await
        }
        async fn task_dependencies(
            &self,
            id: &NativeId,
            direction: Direction,
            page: &PageRequest,
        ) -> Result<Page<DependencyEdge>, SourceError> {
            self.inner.task_dependencies(id, direction, page).await
        }
        async fn project_dependencies(
            &self,
            id: &NativeId,
            direction: Direction,
            page: &PageRequest,
        ) -> Result<Page<DependencyEdge>, SourceError> {
            self.inner.project_dependencies(id, direction, page).await
        }
    }

    fn shouting(stored: Option<&str>) -> ResolvedSource {
        ResolvedSource::adopt(
            SourceName::new("loud".to_owned()).expect("a name"),
            Box::new(Shouting {
                inner: InMemorySource::new(InMemoryConfig::default()).expect("an empty source"),
                stored: Mutex::new(stored.map(str::to_owned)),
            }),
        )
    }

    fn provenance(content: &str) -> BTreeMap<String, Value> {
        BTreeMap::from([(
            "onetaskgraph.template".to_owned(),
            json!({"template": "t", "digest": format!("sha256:{}", "a".repeat(64)),
                   "body_digest": body_digest(content),
                   "answers_digest": format!("sha256:{}", "b".repeat(64))}),
        )])
    }

    #[test]
    fn a_vouched_rendering_is_rewritten_and_restamped_and_a_hand_edit_only_rewritten() {
        let destination = shouting(None);
        let mut content = Some("one\ntwo".to_owned());
        let mut metadata = provenance("one\ntwo");
        apply(&destination, &mut content, &mut metadata);
        assert_eq!(content.as_deref(), Some("ONE\nTWO"));
        assert_eq!(
            metadata["onetaskgraph.template"]["body_digest"],
            json!(body_digest("ONE\nTWO"))
        );

        let mut edited = Some("edited".to_owned());
        let mut vouching_for_another = provenance("rendered");
        apply(&destination, &mut edited, &mut vouching_for_another);
        assert_eq!(edited.as_deref(), Some("EDITED"));
        assert_eq!(vouching_for_another, provenance("rendered"));

        let mut nothing = None;
        let mut none = BTreeMap::new();
        apply(&destination, &mut nothing, &mut none);
        assert_eq!((nothing, none), (None, BTreeMap::new()));
    }

    #[test]
    fn the_first_differing_line_is_named_with_both_texts_and_an_agreeing_answer_is_none() {
        let id = NativeId("T-1".to_owned());
        assert!(
            diverged(
                &shouting(Some("A\nB")),
                MetadataRecord::Task,
                &id,
                Some("A\nB")
            )
            .is_none()
        );
        assert!(diverged(&shouting(None), MetadataRecord::Task, &id, Some("A")).is_none());
        assert!(diverged(&shouting(Some("")), MetadataRecord::Task, &id, None).is_none());
        let Some(EngineError::StoredContentDiverged {
            item,
            line,
            sent,
            stored,
        }) = diverged(
            &shouting(Some("A\nB\nc")),
            MetadataRecord::Document,
            &id,
            Some("A\nB\nC"),
        )
        else {
            panic!("a difference is refused");
        };
        assert_eq!((item.to_string(), line), ("loud:T-1".to_owned(), 3));
        assert_eq!((sent.as_str(), stored.as_str()), ("C", "c"));
        // A stored content that runs on past the one sent names the line it alone holds.
        let Some(EngineError::StoredContentDiverged {
            line, sent, stored, ..
        }) = diverged(
            &shouting(Some("A\nmore")),
            MetadataRecord::Task,
            &id,
            Some("A"),
        )
        else {
            panic!("a difference is refused");
        };
        assert_eq!((line, sent.as_str(), stored.as_str()), (2, "", "more"));
        // What an earlier write left is let go of before the one compared.
        let earlier = shouting(Some("stale"));
        forget(&earlier, MetadataRecord::Task, &id);
        assert!(diverged(&earlier, MetadataRecord::Task, &id, Some("A")).is_none());
    }
}
