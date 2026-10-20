//! What a destination stores of the content it is written, on both sides of the write.
//!
//! Before: the destination's declared rewrite —
//! [`TaskSource::stored_form`](onetaskgraph_plugin_api::TaskSource::stored_form) — applied to the
//! content, so what is sent is what the backend keeps, with a rendering's `body_digest`
//! recorded over it where its provenance vouched for the content it started from. After: the
//! content the write's own answer says the backend stores, compared with what was sent byte for
//! byte, a difference being a rewrite the destination did not declare.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{MetadataRecord, NativeId, SourceError};
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
///
/// # Errors
///
/// What `destination` answered when it could not say what it stores; both are then left as
/// they were.
pub(super) async fn apply(
    destination: &ResolvedSource,
    content: &mut Option<String>,
    metadata: &mut BTreeMap<String, Value>,
) -> Result<(), SourceError> {
    let Some(read) = content.as_deref() else {
        return Ok(());
    };
    let written = destination.source().stored_form(read).await?;
    if written == read {
        return Ok(());
    }
    if let Some(provenance) = restamped(metadata, read, &written) {
        metadata.insert(TemplateProvenance::KEY.to_owned(), provenance.to_value());
    }
    *content = Some(written);
    Ok(())
}

/// Let go of what `destination` still holds from an earlier write of the `record` `id`, before
/// a write of it whose answer is the one to compare.
pub(super) fn forget(destination: &ResolvedSource, record: MetadataRecord, id: &NativeId) {
    let _ = destination.source().stored_content(record, id);
}

/// The refusal owed when the write of the `record` `id` just sent `sent` and its answer says
/// `destination` stores something else; `None` when they agree, or when the destination said
/// nothing about what it stores.
///
/// An answer that lacks, or blanks, a line that was sent as an image is not taken at its word:
/// one read of the stored item confirms it, and that read is compared instead. So a digest is
/// never recorded over content missing an image the write sent — the read either holds it, and
/// the item agrees, or does not, and the refusal names the line — while a write whose answer
/// merely left the image out of what it echoed is accepted. A read that fails or finds no item
/// confirms nothing, and the answer's difference is refused as it stands.
pub(super) async fn diverged(
    destination: &ResolvedSource,
    record: MetadataRecord,
    id: &NativeId,
    sent: Option<&str>,
) -> Option<EngineError> {
    let stored = destination.source().stored_content(record, id)?;
    let sent = sent.unwrap_or_default();
    let answered = first_difference(sent, &stored)?;
    let differs = if answered.lacks_an_image() {
        match confirmed(destination, record, id).await {
            Some(read) => first_difference(sent, &read)?,
            None => answered,
        }
    } else {
        answered
    };
    Some(EngineError::StoredContentDiverged {
        item: GlobalId::new(destination.name().clone(), id.clone()),
        line: differs.line,
        sent: differs.sent,
        stored: differs.stored,
    })
}

/// The first line, counted from 1, at which `sent` and `stored` differ, with each side's text of
/// it — `None` for a side that ended before it.
struct Difference {
    line: usize,
    sent: Option<String>,
    stored: Option<String>,
}

impl Difference {
    /// Whether the stored side lacks, or holds blank, a line that was sent holding an image.
    fn lacks_an_image(&self) -> bool {
        self.sent.as_deref().is_some_and(|line| line.contains("!["))
            && self
                .stored
                .as_deref()
                .is_none_or(|line| line.trim().is_empty())
    }
}

fn first_difference(sent: &str, stored: &str) -> Option<Difference> {
    if sent == stored {
        return None;
    }
    let mut sent_lines = sent.split('\n');
    let mut stored_lines = stored.split('\n');
    let mut line = 0;
    loop {
        line += 1;
        let (sent_line, stored_line) = (sent_lines.next(), stored_lines.next());
        if sent_line != stored_line {
            return Some(Difference {
                line,
                sent: sent_line.map(str::to_owned),
                stored: stored_line.map(str::to_owned),
            });
        }
    }
}

/// The content `destination` holds for the `record` `id`, read again; `None` when the read
/// fails or finds no such item.
async fn confirmed(
    destination: &ResolvedSource,
    record: MetadataRecord,
    id: &NativeId,
) -> Option<String> {
    let source = destination.source();
    let content = match record {
        MetadataRecord::Task => source.get_task(id).await.ok()??.content,
        MetadataRecord::Project => source.get_project(id).await.ok()??.content,
        MetadataRecord::Document => source.get_document(id).await.ok()??.content,
    };
    Some(content.unwrap_or_default())
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
        async fn stored_form(&self, content: &str) -> Result<String, SourceError> {
            Ok(content.to_uppercase())
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
        holding(InMemoryConfig::default(), stored)
    }

    /// A source over `held` whose last write's answer says it stores `stored`.
    fn holding(held: InMemoryConfig, stored: Option<&str>) -> ResolvedSource {
        ResolvedSource::adopt(
            SourceName::new("loud".to_owned()).expect("a name"),
            Box::new(Shouting {
                inner: InMemorySource::new(held).expect("a source"),
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

    #[tokio::test]
    async fn a_vouched_rendering_is_rewritten_and_restamped_and_a_hand_edit_only_rewritten() {
        let destination = shouting(None);
        let mut content = Some("one\ntwo".to_owned());
        let mut metadata = provenance("one\ntwo");
        apply(&destination, &mut content, &mut metadata)
            .await
            .expect("an in-process source answers");
        assert_eq!(content.as_deref(), Some("ONE\nTWO"));
        assert_eq!(
            metadata["onetaskgraph.template"]["body_digest"],
            json!(body_digest("ONE\nTWO"))
        );

        let mut edited = Some("edited".to_owned());
        let mut vouching_for_another = provenance("rendered");
        apply(&destination, &mut edited, &mut vouching_for_another)
            .await
            .expect("an in-process source answers");
        assert_eq!(edited.as_deref(), Some("EDITED"));
        assert_eq!(vouching_for_another, provenance("rendered"));

        let mut nothing = None;
        let mut none = BTreeMap::new();
        apply(&destination, &mut nothing, &mut none)
            .await
            .expect("an in-process source answers");
        assert_eq!((nothing, none), (None, BTreeMap::new()));
    }

    #[tokio::test]
    async fn the_first_differing_line_is_named_with_both_texts_and_an_agreeing_answer_is_none() {
        let id = NativeId("T-1".to_owned());
        assert!(
            diverged(
                &shouting(Some("A\nB")),
                MetadataRecord::Task,
                &id,
                Some("A\nB")
            )
            .await
            .is_none()
        );
        assert!(
            diverged(&shouting(None), MetadataRecord::Task, &id, Some("A"))
                .await
                .is_none()
        );
        assert!(
            diverged(&shouting(Some("")), MetadataRecord::Task, &id, None)
                .await
                .is_none()
        );
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
        .await
        else {
            panic!("a difference is refused");
        };
        assert_eq!((item.to_string(), line), ("loud:T-1".to_owned(), 3));
        assert_eq!((sent.as_deref(), stored.as_deref()), (Some("C"), Some("c")));
        // A stored content that runs on past the one sent names the line it alone holds.
        let Some(EngineError::StoredContentDiverged {
            line, sent, stored, ..
        }) = diverged(
            &shouting(Some("A\nmore")),
            MetadataRecord::Task,
            &id,
            Some("A"),
        )
        .await
        else {
            panic!("a difference is refused");
        };
        assert_eq!(
            (line, sent.as_deref(), stored.as_deref()),
            (2, None, Some("more"))
        );
        // A line one side does not have is told apart from an empty one.
        let shown = diverged(&shouting(Some("A\n")), MetadataRecord::Task, &id, Some("A"))
            .await
            .expect("a difference is refused")
            .to_string();
        assert!(
            shown.contains("sent:   no line 2: the content ends at line 1\nstored: \"\"\n"),
            "{shown}"
        );
        // What an earlier write left is let go of before the one compared.
        let earlier = shouting(Some("stale"));
        forget(&earlier, MetadataRecord::Task, &id);
        assert!(
            diverged(&earlier, MetadataRecord::Task, &id, Some("A"))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_answer_lacking_an_image_it_was_sent_is_confirmed_by_one_read_of_the_item() {
        let id = NativeId("T-1".to_owned());
        let sent = "![a](https://x/a)\n\n![b](https://x/b)";
        let held = |content: &str| -> InMemoryConfig {
            serde_json::from_value(json!({
                "tasks": [{"id": "T-1", "title": "t", "content": content, "url": "https://x/T-1",
                           "status": {"category": "todo", "name": "Todo"}, "labels": [],
                           "project": null, "created_at": null, "updated_at": null,
                           "metadata": {}}]
            }))
            .expect("an in-memory configuration")
        };
        // The answer left the second image out, the item holds it: accepted on the read.
        for answered in ["![a](https://x/a)\n", "![a](https://x/a)\n\n"] {
            assert!(
                diverged(
                    &holding(held(sent), Some(answered)),
                    MetadataRecord::Task,
                    &id,
                    Some(sent)
                )
                .await
                .is_none(),
                "{answered:?}"
            );
        }
        // The item really lacks it: refused naming the image line the read does not have.
        let Some(EngineError::StoredContentDiverged {
            line,
            sent: line_sent,
            stored,
            ..
        }) = diverged(
            &holding(held("![a](https://x/a)\n"), Some("![a](https://x/a)\n")),
            MetadataRecord::Task,
            &id,
            Some(sent),
        )
        .await
        else {
            panic!("a missing image is refused");
        };
        assert_eq!(
            (line, line_sent.as_deref(), stored.as_deref()),
            (3, Some("![b](https://x/b)"), None)
        );
        // An item the read cannot find confirms nothing: the answer's difference is refused.
        assert!(
            diverged(
                &shouting(Some("![a](https://x/a)\n")),
                MetadataRecord::Task,
                &id,
                Some(sent)
            )
            .await
            .is_some()
        );
        // A difference at a line that was not sent as an image is refused without a read, even
        // when the item holds what was sent.
        assert!(
            diverged(
                &holding(held("A\nB"), Some("A\nb")),
                MetadataRecord::Task,
                &id,
                Some("A\nB")
            )
            .await
            .is_some()
        );
    }
}
