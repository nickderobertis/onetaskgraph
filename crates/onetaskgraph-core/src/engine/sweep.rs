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

/// The classes of record a search-only sweep cannot establish, as the report names them.
const UNCOVERED: [&str; 3] = ["metadata", "comments", "normalized-spellings"];

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
    /// Search or listing pages.
    pub searches: u64,
    /// Reads of an item and its comments.
    pub reads: u64,
    /// Writes of an item or a comment.
    pub writes: u64,
}

/// Which classes of record a sweep could not establish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SweepCoverage {
    /// Whether it covered every class.
    pub complete: bool,
    /// `metadata`, `comments`, `normalized-spellings`: each class it did not cover.
    pub uncovered: Vec<String>,
}

/// What a sweep did with one item.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SweepItem {
    /// The item's qualified id, as screened.
    #[schemars(with = "serde_json::Value")]
    pub id: Locator,
    /// `task`.
    pub kind: String,
    /// `rewritten`, `would-rewrite`, `unchanged`, `unfixable`, `conflict` or `not-a-match`.
    pub outcome: String,
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
    /// `rewritten`, `would-rewrite` or `unfixable`.
    pub outcome: String,
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
            .any(|item| matches!(item.outcome.as_str(), "unfixable" | "conflict"))
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
    /// Sweep `source`: find its items already carrying a declared private term, and rewrite
    /// every field and comment replacement can fix, unless `dry_run`.
    ///
    /// # Errors
    ///
    /// [`EngineError::UnknownSource`] for a source nothing configures;
    /// [`EngineError::TermsUnlisted`] when the policy cannot list its terms — no
    /// `terms_command`; [`EngineError::BoundaryUnavailable`] when the policy could not decide,
    /// every locator redacted; and [`EngineError::SourceRefused`] when the source refuses a
    /// read or a write.
    pub async fn sanitize_source(
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
                for task in page.items {
                    self.sweep_item(destination, task.id, false, dry_run, &mut sweep)
                        .await?;
                }
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
                    for task in page.items {
                        if seen.insert(task.id.clone()) {
                            self.sweep_item(destination, task.id, true, dry_run, &mut sweep)
                                .await?;
                        }
                    }
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
                    UNCOVERED.iter().map(|class| (*class).to_owned()).collect()
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

    /// Decide, and unless `dry_run` write, one item: read it with its comments, put it to the
    /// policy's sanitisation once, and rewrite what that answer rewrote.
    async fn sweep_item(
        &self,
        destination: &ResolvedSource,
        id: NativeId,
        searched: bool,
        dry_run: bool,
        sweep: &mut Sweep,
    ) -> Result<(), EngineError> {
        let item = format!("{}:{id}", destination.name());
        let task: Task = self.read_held_task(destination, &id).await?;
        let comments = every_comment(destination, &id).await;
        sweep.requests.reads += 1;
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
                outcome: "unfixable".to_owned(),
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
                    kind: "task".to_owned(),
                    outcome: "unfixable".to_owned(),
                    fields: vec![SweepField {
                        field: Locator::Plain("id".to_owned()),
                        outcome: "unfixable".to_owned(),
                        why: Some(IDENTIFIER.to_owned()),
                    }],
                });
                return Ok(());
            }
            if fields.is_empty() {
                fields.push(SweepField {
                    field: Locator::Plain("content".to_owned()),
                    outcome: "unfixable".to_owned(),
                    why: Some("carries a term no replacement is declared for".to_owned()),
                });
            }
        }
        let Some(sanitised) = sanitised_task(&task, &comments, &rewrite) else {
            if unfixable {
                sweep.items.push(SweepItem {
                    id: shown_item,
                    kind: "task".to_owned(),
                    outcome: "unfixable".to_owned(),
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
                    kind: "task".to_owned(),
                    outcome: if replaced { "unchanged" } else { "not-a-match" }.to_owned(),
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
                    kind: "task".to_owned(),
                    outcome: "conflict".to_owned(),
                    fields,
                });
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if !dry_run {
            sweep.requests.writes += 1;
        }
        let word = if dry_run {
            "would-rewrite"
        } else {
            "rewritten"
        };
        let outcome = if written.is_empty() {
            "unchanged"
        } else if unfixable {
            "unfixable"
        } else {
            word
        };
        for field in &written {
            fields.push(SweepField {
                field: screened_field(field, &stored.exposure, &answer.locators),
                outcome: word.to_owned(),
                why: None,
            });
        }
        sweep.items.push(SweepItem {
            id: shown_item,
            kind: "task".to_owned(),
            outcome: outcome.to_owned(),
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
