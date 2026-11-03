//! `sources sanitize`: find the items of one source already carrying a declared private term,
//! and fix what replacement can.
//!
//! The terms are the policy's ([`WritePolicy::declared_terms`]) and nothing here matches one:
//! a search per term narrows which items are read, and every item read is put to the policy's
//! own sanitisation before anything is rewritten. A term, and any query built from one, is held
//! in memory for the sweep alone — never written to a report, a log, a cache key or standard
//! error.
//!
//! **Coverage is reported, never assumed.** A sweep that searches cannot establish three
//! classes of record: a hit only in stored metadata, which no search of this contract reaches;
//! a hit only in a comment, which no search of this contract reaches either; and a spelling the
//! policy matches but a backend's tokenizer does not. A search sweep names each under
//! `coverage.uncovered`. `--complete` reads the source whole — every item, page by page, with
//! its comments — and so covers all three; a source whose requests spend no quota, a folder of
//! Markdown, is always read whole.
//!
//! [`WritePolicy::declared_terms`]: super::boundary::WritePolicy::declared_terms

use onetaskgraph_plugin_api::{
    Classification, NativeId, PageRequest, SourceName, Task, TaskQuery, TextFields, TextQuery,
};
use schemars::JsonSchema;
use serde::Serialize;

use super::boundary::{
    Exposure, Locator, LocatorKind, Rewrite, SanitizeSurface, SanitizeVerdict, StoredTask,
    TermsRequest, WriteVerdict, every_comment, refusal, sanitised_task, screened_field,
    screened_locator,
};
use super::{Engine, EngineError};
use crate::resolve::ResolvedSource;

/// A class of record a search cannot establish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Uncovered {
    /// A hit only in stored metadata, which no search of the plugin contract reaches.
    Metadata,
    /// A hit only in a comment, which no search of the plugin contract reaches.
    Comments,
    /// A spelling the policy matches and a backend's index does not.
    NormalizedSpellings,
}

/// The classes of record a search-only sweep cannot establish.
const UNCOVERED: [Uncovered; 3] = [
    Uncovered::Metadata,
    Uncovered::Comments,
    Uncovered::NormalizedSpellings,
];

/// What kind of record a sweep entry is about: a task, the one kind a sweep reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SweepKind {
    /// A task.
    Task,
}

/// What a sweep did with one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ItemOutcome {
    /// Every field it carried a term in was rewritten.
    Rewritten,
    /// A dry run: every field it carries a term in would be rewritten.
    WouldRewrite,
    /// Nothing in it needed rewriting.
    Unchanged,
    /// Something in it replacement cannot fix.
    Unfixable,
    /// It moved twice under the sweep, and nothing was overwritten.
    Conflict,
    /// A search returned it, and the policy finds no term in it.
    NotAMatch,
}

/// What a sweep did with one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FieldOutcome {
    /// It was rewritten.
    Rewritten,
    /// A dry run: it would be rewritten.
    WouldRewrite,
    /// Replacement cannot fix it.
    Unfixable,
}

/// What one `sources sanitize` did, naming every record by its screened locator and never by
/// what it said.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SweepReport {
    /// The source swept, as screened.
    #[schemars(with = "serde_json::Value")]
    pub source: Locator,
    /// Whether nothing was written.
    pub dry_run: bool,
    /// Whether the source was read whole rather than searched.
    pub complete: bool,
    /// What the sweep cost, from the source's own request accounting.
    pub requests: SweepRequests,
    /// Which classes of record the sweep could not establish.
    pub coverage: SweepCoverage,
    /// One entry per item a search returned, and per item a whole read found carrying a term.
    pub items: Vec<SweepItem>,
}

/// What a sweep sent its source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SweepRequests {
    /// Every request the source counted, `0` for one that counts none.
    pub total: u64,
    /// Search or listing pages the sweep asked its source for — each one call of the plugin
    /// contract, which a source may answer with more than one request.
    pub searches: u64,
    /// Batched reads of items with their comments, and further comment reads, the sweep made.
    pub reads: u64,
    /// Items the sweep wrote back, each with every comment of it that changed.
    pub writes: u64,
}

/// Which classes of record a sweep could not establish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
// llmlint: ignore[invalid_states_unrepresentable] The report's JSON is the task's stated contract, `{"complete": bool, "uncovered": [...]}`, and both SDKs are generated from it; the one constructor, `sanitize_tasks`, sets `complete` exactly when `uncovered` is empty.
pub struct SweepCoverage {
    /// Whether it covered every class.
    pub complete: bool,
    /// Each class it did not cover.
    pub uncovered: Vec<Uncovered>,
}

/// What a sweep did with one item.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SweepItem {
    /// The item's qualified id, as screened.
    #[schemars(with = "serde_json::Value")]
    pub id: Locator,
    /// What kind of record it is.
    pub kind: SweepKind,
    /// What the sweep did with it.
    pub outcome: ItemOutcome,
    /// What it did with each field it changed or could not.
    pub fields: Vec<SweepField>,
}

/// What a sweep did with one field of an item.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SweepField {
    /// `content`, `title`, `label`, `repositories`, `metadata:<key>` or `comment:<id>`, its
    /// key or id as screened.
    #[schemars(with = "serde_json::Value")]
    pub field: Locator,
    /// What the sweep did with it.
    pub outcome: FieldOutcome,
    /// Why a field is unfixable, in neutral words; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl SweepReport {
    /// The exit status the command line answers with: `1` when anything was left unfixable or
    /// in conflict, `3` when the sweep could not cover every class of record and nothing else
    /// failed, and `0` otherwise. An unavailable policy is a refusal, exiting `2`, before any
    /// report.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if self
            .items
            .iter()
            .any(|item| matches!(item.outcome, ItemOutcome::Unfixable | ItemOutcome::Conflict))
        {
            1
        } else if !self.coverage.complete {
            3
        } else {
            0
        }
    }

    /// Number every redacted locator from 1, in the order the report emits them.
    fn renumber(&mut self) {
        let mut next = 1;
        let mut number = |locator: &mut Locator| {
            if let Locator::Redacted { ordinal, .. } = locator {
                *ordinal = next;
                next += 1;
            }
        };
        number(&mut self.source);
        for item in &mut self.items {
            number(&mut item.id);
            for field in &mut item.fields {
                number(&mut field.field);
            }
        }
    }
}

/// Why an item whose locator carries a term is unfixable.
const IDENTIFIER: &str = privateterms::LOCATOR_UNFIXABLE_REASON;

impl Engine {
    /// Sweep the tasks of `source`: find those already carrying a declared private term, and
    /// rewrite every field and comment of theirs replacement can fix, unless `dry_run`. Its
    /// projects and documents are not read, and `complete` is coverage of its tasks.
    ///
    /// # Errors
    ///
    /// [`EngineError::UnknownSource`] for a source nothing configures;
    /// [`EngineError::TermsUnlisted`] when the policy cannot list its terms — no
    /// `terms_command`; [`EngineError::BoundaryUnavailable`] when the policy could not decide,
    /// every locator redacted; and [`EngineError::SourceRefused`] when the source refuses a
    /// read or a write.
    pub async fn sanitize_tasks(
        &self,
        source: &SourceName,
        dry_run: bool,
        complete: bool,
    ) -> Result<SweepReport, EngineError> {
        let destination = self.built(source)?;
        let terms = self
            .boundary_policy()
            .declared_terms(&TermsRequest {
                scope: self.term_scope(),
                replacements: self.replacements().to_vec(),
            })
            .await
            .map_err(|error| EngineError::TermsUnlisted {
                name: source.to_string(),
                why: error.message,
            })?
            .ok_or_else(|| EngineError::TermsUnlisted {
                name: source.to_string(),
                why: "no write_policy.terms_command is configured".to_owned(),
            })?;
        let before = destination.source().metering().await.ok().flatten();
        let whole = complete || before.is_none();
        let mut sweep = Sweep {
            requests: SweepRequests::default(),
            items: Vec::new(),
            source: None,
        };
        if whole {
            let mut cursor = None;
            loop {
                let page = destination
                    .source()
                    .query_tasks(
                        &TaskQuery::default(),
                        &PageRequest {
                            cursor: cursor.take(),
                            limit: 100,
                        },
                    )
                    .await
                    .map_err(|error| refused(destination, error))?;
                sweep.requests.searches += 1;
                // Each item of the page is decided, and written, before the next is asked for.
                let ids = page.items.into_iter().map(|task| task.id).collect();
                self.sweep_batch(destination, ids, false, dry_run, &mut sweep)
                    .await?;
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
        } else {
            let mut seen = std::collections::BTreeSet::new();
            for term in &terms {
                let mut cursor = None;
                loop {
                    let page = destination
                        .source()
                        .query_tasks(
                            &TaskQuery {
                                text: Some(TextQuery {
                                    terms: term.rule.term.clone(),
                                    fields: TextFields::TitleOrContent,
                                }),
                                ..TaskQuery::default()
                            },
                            &PageRequest {
                                cursor: cursor.take(),
                                limit: 100,
                            },
                        )
                        .await
                        .map_err(|error| refused(destination, error))?;
                    sweep.requests.searches += 1;
                    let ids = page
                        .items
                        .into_iter()
                        .map(|task| task.id)
                        .filter(|id| seen.insert(id.clone()))
                        .collect();
                    self.sweep_batch(destination, ids, true, dry_run, &mut sweep)
                        .await?;
                    match page.next {
                        Some(next) => cursor = Some(next),
                        None => break,
                    }
                }
            }
        }
        let shown_source = match sweep.source.take() {
            Some(shown) => shown,
            None => {
                // No item's call screened the source's name, so one call on the report's path.
                let exposure = Exposure::default().locate(LocatorKind::Source, source.as_str());
                let answer = self.screen(&exposure).await;
                if let WriteVerdict::Unavailable { .. } = answer.verdict {
                    return Err(refusal(destination, "", &exposure, &answer));
                }
                screened_locator(
                    &exposure,
                    &answer.locators,
                    LocatorKind::Source,
                    source.as_str(),
                )
            }
        };
        let after = destination.source().metering().await.ok().flatten();
        sweep.requests.total = match (before, after) {
            (Some(before), Some(after)) => after.requests.saturating_sub(before.requests),
            _ => 0,
        };
        if sweep.requests.total == 0 {
            sweep.requests = SweepRequests::default();
        }
        let mut report = SweepReport {
            source: shown_source,
            dry_run,
            complete: whole,
            requests: sweep.requests,
            coverage: SweepCoverage {
                complete: whole,
                uncovered: if whole {
                    Vec::new()
                } else {
                    UNCOVERED.to_vec()
                },
            },
            items: sweep.items,
        };
        if report.items.iter().any(|item| {
            matches!(item.id, Locator::Redacted { .. })
                || item
                    .fields
                    .iter()
                    .any(|field| matches!(field.field, Locator::Redacted { .. }))
        }) || matches!(report.source, Locator::Redacted { .. })
        {
            report.renumber();
        }
        Ok(report)
    }

    /// Read `ids` with the first page of their comments in the source's own batched read, and
    /// decide each in turn — the rest of an item's comments read only where it has more.
    async fn sweep_batch(
        &self,
        destination: &ResolvedSource,
        ids: Vec<NativeId>,
        searched: bool,
        dry_run: bool,
        sweep: &mut Sweep,
    ) -> Result<(), EngineError> {
        if ids.is_empty() {
            return Ok(());
        }
        let commented = destination.source().capabilities().comments.is_native();
        let first = PageRequest {
            cursor: None,
            limit: 20,
        };
        let details = destination
            .source()
            .get_task_details(&ids, commented.then_some(&first))
            .await;
        sweep.requests.reads += 1;
        for (id, detail) in ids.into_iter().zip(details) {
            let Some(read) = detail.map_err(|error| refused(destination, error))? else {
                continue;
            };
            let mut comments = Vec::new();
            match read.comments {
                Some(Err(error)) => return Err(refused(destination, error)),
                Some(Ok(Some(page))) => {
                    comments.extend(page.items);
                    if page.next.is_some() {
                        // Every page of one revision, read again from the start rather than
                        // resumed, so no comment is counted twice.
                        comments = every_comment(destination, &id).await?;
                        sweep.requests.reads += 1;
                    }
                }
                Some(Ok(None)) | None => {}
            }
            self.sweep_item(
                destination,
                id,
                read.task,
                comments,
                searched,
                dry_run,
                sweep,
            )
            .await?;
        }
        Ok(())
    }

    /// Decide, and unless `dry_run` write, one item read with its comments: put it to the
    /// policy's sanitisation once, and rewrite what that answer rewrote.
    #[expect(
        clippy::too_many_arguments,
        reason = "one item as read, where it lives, and how the sweep was asked to treat it"
    )]
    async fn sweep_item(
        &self,
        destination: &ResolvedSource,
        id: NativeId,
        task: Task,
        comments: Vec<onetaskgraph_plugin_api::Comment>,
        searched: bool,
        dry_run: bool,
        sweep: &mut Sweep,
    ) -> Result<(), EngineError> {
        let item = format!("{}:{id}", destination.name());
        let stored = StoredTask::of(destination, &item, &task, &comments);
        let answer: SanitizeVerdict = self.screen(&stored.exposure).await;
        if let WriteVerdict::Unavailable { .. } = answer.verdict {
            return Err(refusal(destination, &item, &stored.exposure, &answer));
        }
        let shown = |kind: LocatorKind, value: &str| {
            screened_locator(&stored.exposure, &answer.locators, kind, value)
        };
        if sweep.source.is_none() {
            sweep.source = Some(shown(LocatorKind::Source, destination.name().as_str()));
        }
        let shown_item = shown(LocatorKind::Item, &item);
        let rewrite = Rewrite::of(&stored.exposure, &answer);
        let mut fields = Vec::new();
        let mut unfixable = false;
        // A private repository among its own: the item is private by classification, which no
        // replacement changes.
        if self
            .classify(Classification::Public, &task.repositories)
            .await
            == Classification::Private
        {
            unfixable = true;
            fields.push(SweepField {
                field: Locator::Plain("repositories".to_owned()),
                outcome: FieldOutcome::Unfixable,
                why: Some("names a private repository".to_owned()),
            });
        }
        if let WriteVerdict::Refuse { .. } = answer.verdict {
            unfixable = true;
            if answer
                .unfixed
                .iter()
                .any(|unfixed| unfixed.surface == SanitizeSurface::Locators)
                || matches!(shown_item, Locator::Redacted { .. })
            {
                sweep.items.push(SweepItem {
                    id: shown_item,
                    kind: SweepKind::Task,
                    outcome: ItemOutcome::Unfixable,
                    fields: vec![SweepField {
                        field: Locator::Plain("id".to_owned()),
                        outcome: FieldOutcome::Unfixable,
                        why: Some(IDENTIFIER.to_owned()),
                    }],
                });
                return Ok(());
            }
            if fields.is_empty() {
                fields.push(SweepField {
                    field: Locator::Plain("content".to_owned()),
                    outcome: FieldOutcome::Unfixable,
                    why: Some("carries a term no replacement is declared for".to_owned()),
                });
            }
        }
        let Some(sanitised) = sanitised_task(&task, &comments, &rewrite) else {
            if unfixable {
                sweep.items.push(SweepItem {
                    id: shown_item,
                    kind: SweepKind::Task,
                    outcome: ItemOutcome::Unfixable,
                    fields,
                });
            } else if searched {
                // A hit the policy finds nothing in: one an earlier sanitisation already
                // rewrote — it carries a declared replacement — is unchanged, and any other is
                // a hit the policy does not confirm.
                let replaced = self.replacements().iter().any(|entry| match entry {
                    super::boundary::Replacement::Replace { with, .. } => {
                        !with.is_empty() && carries(&task, &comments, with)
                    }
                    super::boundary::Replacement::Reject { .. } => false,
                });
                sweep.items.push(SweepItem {
                    id: shown_item,
                    kind: SweepKind::Task,
                    outcome: if replaced {
                        ItemOutcome::Unchanged
                    } else {
                        ItemOutcome::NotAMatch
                    },
                    fields: Vec::new(),
                });
            }
            return Ok(());
        };
        let written = match self
            .sanitise_written(destination, &item, &id, &task, sanitised, dry_run)
            .await
        {
            Ok(written) => written,
            Err(EngineError::SanitizeConflict { .. }) => {
                sweep.items.push(SweepItem {
                    id: shown_item,
                    kind: SweepKind::Task,
                    outcome: ItemOutcome::Conflict,
                    fields,
                });
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if !dry_run {
            sweep.requests.writes += 1;
        }
        let (word, whole) = if dry_run {
            (FieldOutcome::WouldRewrite, ItemOutcome::WouldRewrite)
        } else {
            (FieldOutcome::Rewritten, ItemOutcome::Rewritten)
        };
        let outcome = if written.is_empty() {
            ItemOutcome::Unchanged
        } else if unfixable {
            ItemOutcome::Unfixable
        } else {
            whole
        };
        for field in &written {
            fields.push(SweepField {
                field: screened_field(field, &stored.exposure, &answer.locators),
                outcome: word,
                why: None,
            });
        }
        sweep.items.push(SweepItem {
            id: shown_item,
            kind: SweepKind::Task,
            outcome,
            fields,
        });
        Ok(())
    }
}

/// What a sweep has gathered so far.
struct Sweep {
    requests: SweepRequests,
    items: Vec<SweepItem>,
    /// The source's name as the first item's call screened it.
    source: Option<Locator>,
}

fn refused(
    destination: &ResolvedSource,
    error: onetaskgraph_plugin_api::SourceError,
) -> EngineError {
    EngineError::SourceRefused {
        name: destination.name().to_string(),
        error,
    }
}

/// Whether `task` or one of `comments` carries `text` anywhere a sanitisation rewrites.
fn carries(task: &Task, comments: &[onetaskgraph_plugin_api::Comment], text: &str) -> bool {
    task.title.contains(text)
        || task
            .content
            .as_deref()
            .is_some_and(|content| content.contains(text))
        || task.labels.iter().any(|label| label.name.contains(text))
        || task
            .metadata
            .values()
            .any(|value| value.to_string().contains(text))
        || comments.iter().any(|comment| comment.body.contains(text))
}

/// Each vocabulary's word, as the report spells it.
macro_rules! spelled {
    ($kind:ty) => {
        impl std::fmt::Display for $kind {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match serde_json::to_value(self) {
                    Ok(serde_json::Value::String(word)) => formatter.write_str(&word),
                    _ => Err(std::fmt::Error),
                }
            }
        }
    };
}

spelled!(Uncovered);
spelled!(ItemOutcome);
spelled!(FieldOutcome);
