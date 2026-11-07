//! What a destination stores of the content it is written, taken from each write's own answer.
//!
//! The content is sent as the copy, the create or the render composed it, with nothing
//! predicted of what the backend will do to it. The write's own answer then says what the
//! backend stores — [`TaskSource::stored_content`](onetaskgraph_plugin_api::TaskSource::stored_content)
//! — and where that is what was sent the write is done. Where it is not, every fingerprint the
//! item keeps is recomputed over the stored content: a rendering's `body_digest`, moved onto it
//! only where the provenance vouched for what was sent, and a copy's
//! `onetaskgraph.copied_destination_version`. One follow-up write sends the stored content back
//! verbatim beside them, as its own request, and its own answer has to report that content
//! again; a backend that rewrites its own stored content is refused as `stored-content-diverged`.
//! A copy's follow-up also finishes links in that content, so what it sends holds text of the
//! store's own the backend has not seen: an answer differing from it only as Linear autolinks or
//! escapes such text ([`Normalized`]) is that text normalized rather than a divergence, and its
//! answer is what the copy then records, by one more write.
//! No read of the item follows either write: the answers are the record.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    AssetPayload, AssetUpload, AssetUploads, MetadataRecord, NativeId, body_digest,
    rewrite_asset_references,
};
use serde_json::Value;

use super::EngineError;
use crate::GlobalId;
use crate::resolve::ResolvedSource;
use crate::template::{Sha256Digest, TemplateProvenance};

/// Let go of what `destination` still holds from an earlier write of the `record` `id`, before
/// a write of it whose answer is the one to read.
pub(super) fn forget(destination: &ResolvedSource, record: MetadataRecord, id: &NativeId) {
    let _ = destination.source().stored_content(record, id);
}

/// The content `destination` reported storing for the `record` `id`, whose latest write sent
/// `sent` — `None` when its answer said nothing about it or stored exactly what was sent, so
/// the write is done.
pub(super) fn rewritten(
    destination: &ResolvedSource,
    record: MetadataRecord,
    id: &NativeId,
    sent: Option<&str>,
) -> Option<String> {
    let stored = destination.source().stored_content(record, id)?;
    (stored != sent.unwrap_or_default()).then_some(stored)
}

/// The provenance `metadata` records, moved onto `stored` where it vouched for one of `sent` —
/// the content as composed, or as the write's answer said it sent once its images were pointed
/// where the destination serves them — and `None` where it vouched for neither, so a hand
/// edit's entry is carried exactly as it came and never refreshed into valid provenance.
pub(super) fn restamped(
    metadata: &BTreeMap<String, Value>,
    sent: &[&str],
    stored: &str,
) -> Option<TemplateProvenance> {
    let mut provenance = TemplateProvenance::read(metadata).ok().flatten()?;
    if !sent
        .iter()
        .any(|sent| provenance.body_digest.as_str() == body_digest(sent))
    {
        return None;
    }
    provenance.body_digest =
        Sha256Digest::parse(body_digest(stored)).expect("`body_digest` spells a digest");
    Some(provenance)
}

/// [`restamped`], written into `metadata` in place: whether it moved.
pub(super) fn restamp(metadata: &mut BTreeMap<String, Value>, sent: &[&str], stored: &str) -> bool {
    match restamped(metadata, sent, stored) {
        Some(provenance) => {
            metadata.insert(TemplateProvenance::KEY.to_owned(), provenance.to_value());
            true
        }
        None => false,
    }
}

/// The refusal owed when the follow-up write of the `record` `id`, which sent back `stored` —
/// the content an earlier answer said `destination` stores — was answered with other content;
/// `None` when it reported `stored` again, or said nothing.
///
/// Compared as a read reports content, the metadata slot and its separator removed: a backend
/// may store the separator in its own way, which is no change to the content.
pub(super) fn unstable(
    destination: &ResolvedSource,
    record: MetadataRecord,
    id: &NativeId,
    stored: &str,
) -> Option<EngineError> {
    let again = destination.source().stored_content(record, id)?;
    let differs = first_difference_by(stored, &again, |sent, stored| sent == stored)?;
    Some(diverged(destination, id, differs))
}

/// The refusal naming where the `id` of `destination` stores other content than it was sent.
fn diverged(destination: &ResolvedSource, id: &NativeId, differs: Difference) -> EngineError {
    EngineError::StoredContentDiverged {
        item: GlobalId::new(destination.name().clone(), id.clone()),
        line: differs.line,
        sent: differs.sent,
        stored: differs.stored,
    }
}

/// What a follow-up's own answer said of the `sent` content, when a backend's normalizing of text
/// it had not seen before is to be told from a change: see [`normalized`].
pub(super) enum Normalized {
    /// It reported `sent` again, or said nothing.
    Kept,
    /// It reported this, which reads as `sent` line by line once [`denormalized`].
    Stored(String),
    /// It reported other content.
    Diverged(EngineError),
}

/// [`unstable`] for a follow-up whose `sent` holds text the backend had not seen before, where
/// Linear's autolinking or escaping of that text is told from a person's change.
pub(super) fn normalized(
    destination: &ResolvedSource,
    record: MetadataRecord,
    id: &NativeId,
    sent: &str,
) -> Normalized {
    let Some(again) = destination.source().stored_content(record, id) else {
        return Normalized::Kept;
    };
    let reads_as_sent = |sent_line: &str, stored_line: &str| {
        let plain = denormalized(sent_line, None);
        sent_line == stored_line || plain == denormalized(stored_line, Some(&plain))
    };
    let differs = first_difference_by(sent, &again, reads_as_sent);
    match differs {
        Some(differs) => Normalized::Diverged(diverged(destination, id, differs)),
        None if again == sent => Normalized::Kept,
        None => Normalized::Stored(again),
    }
}

/// `line` with what Linear does to text it is written undone: `\\[`, `\\]` and `\\~` unescaped, a
/// destination taken out of its angle brackets, and an autolink back to what was written. That is
/// a link whose destination is its text, or its text under `http://`, or, when `sent` is the line
/// as it was sent, a link whose destination `sent` holds bare, whatever its text: Linear links a
/// bare issue URL under the issue's identifier. Text a person changed reads otherwise.
fn denormalized(line: &str, sent: Option<&str>) -> String {
    let unescaped = line
        .replace("\\[", "[")
        .replace("\\]", "]")
        .replace("\\~", "~");
    let mut plain = String::with_capacity(unescaped.len());
    let mut rest = unescaped.as_str();
    while let Some(opening) = rest.find('[') {
        plain.push_str(&rest[..opening]);
        rest = &rest[opening..];
        let Some((link, text, target)) = link_at(rest) else {
            plain.push('[');
            rest = &rest[1..];
            continue;
        };
        if target == text || target.strip_prefix("http://") == Some(text) {
            plain.push_str(text);
        } else if sent.is_some_and(|sent| holds_bare(sent, target)) {
            plain.push_str(target);
        } else {
            plain.push_str(&format!("[{text}]({target})"));
        }
        rest = &rest[link..];
    }
    plain.push_str(rest);
    plain
}

/// Whether `line` holds `target` other than as a link's destination.
fn holds_bare(line: &str, target: &str) -> bool {
    line.match_indices(target)
        .any(|(at, _)| !line[..at].ends_with('('))
}

/// The inline link `text` opens with: its length, its text and its destination out of any angle
/// brackets.
fn link_at(text: &str) -> Option<(usize, &str, &str)> {
    let close = text.find("](")?;
    let inner = &text[1..close];
    if inner.contains('[') {
        return None;
    }
    let after = close + 2;
    let (target, end) = match text[after..].strip_prefix('<') {
        Some(angled) => {
            let end = angled.find(">)")?;
            (&angled[..end], after + 1 + end + 2)
        }
        None => {
            let end = text[after..].find(')')?;
            (&text[after..after + end], after + end + 1)
        }
    };
    Some((end, inner, target))
}

/// What a write that carried `payloads` uploaded, read off the content it said it sent: each
/// asset `composed` references, at the URL `sent` holds in that reference's place — `None` when
/// the two cannot be lined up reference by reference, so nothing is recorded that the write did
/// not say.
///
/// A follow-up write sends the stored content, whose references already point at those URLs,
/// and records them beside it as the asset write recorded them, so a later copy reuses each.
pub(super) fn served(
    composed: &str,
    sent: &str,
    payloads: &[AssetPayload],
) -> Option<AssetUploads> {
    // Each reference is replaced by a marker no Markdown holds, so what lies between two of them
    // is text the asset write carried unchanged; each URL then runs from the end of one stretch
    // of that text to the first character a reference's target cannot hold.
    const MARK: char = '\u{0}';
    let markers: BTreeMap<_, _> = payloads
        .iter()
        .map(|payload| {
            (
                payload.name.clone(),
                format!("{MARK}{}{MARK}", payload.name),
            )
        })
        .collect();
    let marked = rewrite_asset_references(composed, &markers);
    let mut uploads = AssetUploads::default();
    let mut pieces = marked.split(MARK);
    let mut rest = sent.strip_prefix(pieces.next()?)?;
    while let Some(name) = pieces.next() {
        let after = pieces.next()?;
        let end = rest
            .find(|character: char| character == ')' || character.is_whitespace())
            .unwrap_or(rest.len());
        let (url, tail) = rest.split_at(end);
        let payload = payloads
            .iter()
            .find(|payload| payload.name.as_str() == name)?;
        if url.is_empty() {
            return None;
        }
        if let Some(held) = uploads.0.get(&payload.name)
            && held.url != url
        {
            return None;
        }
        uploads.0.insert(
            payload.name.clone(),
            AssetUpload {
                sha256: payload.sha256.clone(),
                url: url.to_owned(),
            },
        );
        rest = tail.strip_prefix(after)?;
    }
    rest.is_empty().then_some(uploads)
}

/// The first line, counted from 1, at which `sent` and `stored` differ, with each side's text of
/// it — `None` for a side that ended before it.
struct Difference {
    line: usize,
    sent: Option<String>,
    stored: Option<String>,
}

fn first_difference_by(
    sent: &str,
    stored: &str,
    same: impl Fn(&str, &str) -> bool,
) -> Option<Difference> {
    if sent == stored {
        return None;
    }
    let mut sent_lines = sent.split('\n');
    let mut stored_lines = stored.split('\n');
    let mut line = 0;
    loop {
        line += 1;
        match (sent_lines.next(), stored_lines.next()) {
            (None, None) => return None,
            (Some(sent), Some(stored)) if same(sent, stored) => {}
            (sent, stored) => {
                return Some(Difference {
                    line,
                    sent: sent.map(str::to_owned),
                    stored: stored.map(str::to_owned),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, Capabilities, DependencyEdge, Direction, Health, Label,
        MetadataRecord, NativeId, Page, PageRequest, Project, ProjectQuery, SourceError,
        SourceName, Task, TaskQuery, TaskSource,
    };
    use serde_json::{Value, json};

    use super::{Normalized, forget, normalized, restamp, rewritten, served, unstable};
    use crate::engine::EngineError;
    use crate::resolve::ResolvedSource;
    use crate::template::body_digest;

    /// An empty source whose latest write's answer said it stores `stored`.
    struct Answering {
        inner: InMemorySource,
        stored: Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl TaskSource for Answering {
        fn kind(&self) -> &'static str {
            "answering"
        }
        fn capabilities(&self) -> Capabilities {
            self.inner.capabilities()
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

    /// A source whose last write's answer says it stores `stored`.
    fn answering(stored: Option<&str>) -> ResolvedSource {
        ResolvedSource::adopt(
            SourceName::new("loud".to_owned()).expect("a name"),
            Box::new(Answering {
                inner: InMemorySource::new(InMemoryConfig::default()).expect("a source"),
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
    fn what_a_write_stored_is_taken_once_and_only_where_it_differs_from_what_was_sent() {
        let id = NativeId("T-1".to_owned());
        assert_eq!(
            rewritten(
                &answering(Some("* one")),
                MetadataRecord::Task,
                &id,
                Some("- one")
            )
            .as_deref(),
            Some("* one")
        );
        assert_eq!(
            rewritten(
                &answering(Some("- one")),
                MetadataRecord::Task,
                &id,
                Some("- one")
            ),
            None
        );
        assert_eq!(
            rewritten(&answering(None), MetadataRecord::Task, &id, Some("- one")),
            None
        );
        assert_eq!(
            rewritten(&answering(Some("")), MetadataRecord::Task, &id, None),
            None
        );
        // Taken: an earlier write's answer is let go of before the one read.
        let earlier = answering(Some("stale"));
        forget(&earlier, MetadataRecord::Task, &id);
        assert_eq!(
            rewritten(&earlier, MetadataRecord::Task, &id, Some("- one")),
            None
        );
    }

    #[test]
    fn a_vouched_rendering_moves_onto_what_was_stored_and_a_hand_edit_is_carried_as_it_came() {
        let mut metadata = provenance("- one");
        assert!(restamp(&mut metadata, &["- one"], "* one"));
        assert_eq!(
            metadata["onetaskgraph.template"]["body_digest"],
            json!(body_digest("* one"))
        );
        // Vouching for what the write said it sent once its images were served is vouching too.
        let mut served_first = provenance("![a](https://x/a)");
        assert!(restamp(
            &mut served_first,
            &["![a](./a.png)", "![a](https://x/a)"],
            "![a](https://x/a)\n"
        ));
        let mut edited = provenance("rendered");
        assert!(!restamp(&mut edited, &["edited"], "EDITED"));
        assert_eq!(edited, provenance("rendered"));
        let mut none = BTreeMap::new();
        assert!(!restamp(&mut none, &["x"], "y"));
        assert!(none.is_empty());
    }

    #[test]
    fn a_follow_up_answered_otherwise_is_refused_naming_the_first_differing_line() {
        let id = NativeId("T-1".to_owned());
        assert!(unstable(&answering(Some("A\nB")), MetadataRecord::Task, &id, "A\nB").is_none());
        assert!(unstable(&answering(None), MetadataRecord::Task, &id, "A").is_none());
        let Some(EngineError::StoredContentDiverged {
            item,
            line,
            sent,
            stored,
        }) = unstable(
            &answering(Some("A\nB\nc")),
            MetadataRecord::Document,
            &id,
            "A\nB\nC",
        )
        else {
            panic!("a difference is refused");
        };
        assert_eq!((item.to_string(), line), ("loud:T-1".to_owned(), 3));
        assert_eq!((sent.as_deref(), stored.as_deref()), (Some("C"), Some("c")));
        // A line one side does not have is told apart from an empty one.
        let shown = unstable(&answering(Some("A\n")), MetadataRecord::Task, &id, "A")
            .expect("a difference is refused")
            .to_string();
        assert!(
            shown.contains("sent:   no line 2: the content ends at line 1\nstored: \"\"\n"),
            "{shown}"
        );
        assert!(!shown.contains("ruling"), "{shown}");
    }

    #[test]
    fn a_follow_up_answered_with_its_own_text_autolinked_is_told_from_a_change() {
        let id = NativeId("T-1".to_owned());
        let url = "https://linear.app/w/document/d#heading-a-12345678";
        let sent = format!("Intro\n| {url} |\n\\[1\\]: [x](y) ~");
        let stored = format!("Intro\n| [{url}](<{url}>) |\n\\[1\\]: [x](<y>) \\~");
        let answer =
            |stored: &str| normalized(&answering(Some(stored)), MetadataRecord::Task, &id, &sent);
        assert!(matches!(answer(&sent), Normalized::Kept));
        assert!(matches!(
            normalized(&answering(None), MetadataRecord::Task, &id, &sent),
            Normalized::Kept
        ));
        assert!(matches!(answer(&stored), Normalized::Stored(held) if held == stored));
        // A bare domain Linear links under `http://` reads as the domain.
        assert!(matches!(
            normalized(
                &answering(Some("[a.io](<http://a.io>)")),
                MetadataRecord::Task,
                &id,
                "a.io"
            ),
            Normalized::Stored(_)
        ));
        // A bare issue URL linked under its identifier, as real Linear links one.
        assert!(matches!(
            normalized(
                &answering(Some("| [TES2-1](https://linear.app/w/issue/TES2-1) |")),
                MetadataRecord::Task,
                &id,
                "| https://linear.app/w/issue/TES2-1 |"
            ),
            Normalized::Stored(_)
        ));
        // A link the store wrote keeps its text: a person's change of it is a change.
        assert!(matches!(
            normalized(
                &answering(Some("| [changed](<https://linear.app/w/issue/TES2-1>) |")),
                MetadataRecord::Task,
                &id,
                "| [budget](https://linear.app/w/issue/TES2-1) |"
            ),
            Normalized::Diverged(_)
        ));
        // A person's change is refused at its own line, past the autolink before it.
        let edited = format!("Intro\n| [{url}](<{url}>) |\n\\[1\\]: [x](<z>) \\~");
        let Normalized::Diverged(EngineError::StoredContentDiverged { line, .. }) = answer(&edited)
        else {
            panic!("a change is refused");
        };
        assert_eq!(line, 3);
        // A link to elsewhere is no autolink, and a line more is no normalization.
        assert!(matches!(
            normalized(
                &answering(Some("[a.io](<http://b.io>)")),
                MetadataRecord::Task,
                &id,
                "a.io"
            ),
            Normalized::Diverged(_)
        ));
        assert!(matches!(
            answer(&format!("{stored}\nA person's line.")),
            Normalized::Diverged(_)
        ));
    }

    #[test]
    fn what_an_asset_write_uploaded_is_read_off_the_content_it_said_it_sent() {
        let first = AssetPayload::of(AssetName::new("a.png").unwrap(), vec![1, 2, 3]);
        let second = AssetPayload::of(AssetName::new("b.png").unwrap(), vec![4, 5]);
        let payloads = [first.clone(), second.clone()];
        let composed = "# T\n![one](./a.png)\n![two](./b.png \"t\")\nand ![again](./a.png)";
        let sent = "# T\n![one](https://u/1)\n![two](https://u/2 \"t\")\nand ![again](https://u/1)";
        let uploads = served(composed, sent, &payloads).expect("lined up");
        assert_eq!(
            uploads.reusable(&first.name, &first.sha256),
            Some("https://u/1")
        );
        assert_eq!(
            uploads.reusable(&second.name, &second.sha256),
            Some("https://u/2")
        );
        // Text that does not line up records nothing.
        assert!(served(composed, "# T\nsomething else", &payloads).is_none());
        // One asset at two URLs is not what one upload answered.
        assert!(
            served(
                composed,
                "# T\n![one](https://u/1)\n![two](https://u/2 \"t\")\nand ![again](https://u/3)",
                &payloads
            )
            .is_none()
        );
        // Content referencing nothing uploads nothing.
        assert_eq!(
            served("plain", "plain", &payloads),
            Some(Default::default())
        );
    }
}
