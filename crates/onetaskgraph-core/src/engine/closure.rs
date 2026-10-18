//! A card brought in line with its issue's closure: `task status reconcile`, on demand over one
//! source, and `task status complete`, by the issue references a landing closed, over every
//! configured source that can hold an item for one.
//!
//! Both are for a source whose status is a card on a board beside an issue that closes on its
//! own. A source whose status *is* its closure — a Linear workflow state, a file's own
//! `status:`, a task held in memory — declares neither capability, is sent no request, and is
//! reported as having nothing to do.

use onetaskgraph_plugin_api::{
    Completion, IssueRef, SourceName, Status, StatusCategory, StatusContradiction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::boundary::Exposure;
use super::delivery::{source_failed, targets};
use super::{ConfiguredSource, Delivered, Engine, EngineError};
use crate::resolve::ResolvedSource;
use crate::{Failure, GlobalId};

/// What `task status reconcile` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusesReconciled {
    /// One entry per item whose card contradicted its closure, in the order the source read
    /// them. Empty for a source with nothing out of line, and for one whose status is its
    /// closure.
    pub items: Vec<StatusReconciled>,
    /// Whether the source's status is its closure, so nothing could need reconciling — and it
    /// was sent no request. Not part of the document: what the human rendering says instead
    /// of a count.
    #[serde(skip)]
    pub status_is_closure: bool,
}

/// One item `task status reconcile` brought in line, or would, or could not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusReconciled {
    /// The item, by its qualified id.
    pub source: GlobalId,
    /// The option its card sat at, or `null` for a card with none.
    pub from: Option<String>,
    /// The status its closure implies, which its card was moved to.
    pub to: Status,
    /// What became of it.
    pub action: ReconcileAction,
    /// Why it failed — the failure object a failure document carries — present only when it
    /// did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
}

/// What reconciling one item came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReconcileAction {
    /// Its card was moved.
    Updated,
    /// Its card would be moved; a dry run wrote nothing.
    WouldUpdate,
    /// Its card could not be moved; `error` says why.
    Failed,
}

impl StatusesReconciled {
    /// Whether any item could not be brought in line, which is what exits `4`.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.action == ReconcileAction::Failed)
    }
}

/// What `task status complete` and [`Engine::complete_issues`] answer with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssuesCompleted {
    /// One entry per distinct reference for each source asked, sources in configuration order
    /// and references in the order given.
    pub items: Vec<IssueCompleted>,
    /// Every source that would have been asked and holds no item by issue reference — its
    /// status is its closure, or its plugin does not declare `complete_by_issue` — each sent
    /// no request.
    pub unsupported: Vec<SourceName>,
}

impl IssuesCompleted {
    /// Whether any item failed, or a task one of them delivers could not be kept in step with
    /// it — which is what exits `4`.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.items.iter().any(|item| {
            item.action == CompleteAction::Failed || item.delivered.iter().any(Delivered::failed)
        })
    }
}

/// What one source holds for one issue reference, and what moving it to `done` came to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssueCompleted {
    /// The issue, as `owner/name#N`.
    pub issue: IssueRef,
    /// The source asked.
    pub source: SourceName,
    /// The task that source holds for the issue, by its qualified id — `null` when it holds
    /// none.
    pub task: Option<GlobalId>,
    /// The option the task's card sat at, or `null`.
    pub from: Option<String>,
    /// The status the task reads at once written, would read under a dry run, or already
    /// reads — `null` when nothing is held or the closure says otherwise.
    pub to: Option<Status>,
    /// What became of it.
    pub action: CompleteAction,
    /// Why it was skipped — `closed as <reason>` — present only then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Why it failed — the failure object a failure document carries — present only then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
    /// Every task the written task delivers, kept in step with it exactly as `task status set`
    /// keeps them — left out when it delivers none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(!skip_serializing_if)]
    pub delivered: Vec<Delivered>,
}

/// What moving one issue's item to `done` came to at one source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CompleteAction {
    /// The task was written `done`.
    Updated,
    /// The task would be written `done`; a dry run wrote nothing.
    WouldUpdate,
    /// The issue is closed as completed and its card already reads `done`.
    Unchanged,
    /// The issue is closed some other way, which a landing does not overrule.
    Skipped,
    /// The source holds no item for the issue, or cannot see it.
    Absent,
    /// The task could not be written, or the source could not be read; `error` says why.
    Failed,
}

impl Engine {
    /// Bring every item of `source` whose card contradicts its issue's closure in line, writing
    /// only its card through the source's own status write — or, under `dry_run`, nothing.
    ///
    /// A source that does not declare
    /// [`reconcile_status`](onetaskgraph_plugin_api::Capabilities::reconcile_status) is sent no
    /// request and answers no items.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::SourceUnavailable`] for one that did not build, and
    /// [`EngineError::SourceFailed`] when its read of what is out of line fails. One item whose
    /// write fails is not an error: it is reported `failed` beside the others.
    pub async fn reconcile_statuses(
        &self,
        source: &SourceName,
        dry_run: bool,
    ) -> Result<StatusesReconciled, EngineError> {
        let built = self.built(source)?;
        if !built.source().capabilities().reconcile_status.is_native() {
            return Ok(StatusesReconciled {
                items: Vec::new(),
                status_is_closure: true,
            });
        }
        let contradictions = built
            .source()
            .status_contradictions()
            .await
            .map_err(|error| source_failed(built, error))?;
        let mut items = Vec::with_capacity(contradictions.len());
        for contradiction in contradictions {
            items.push(self.reconciled(built, contradiction, dry_run).await);
        }
        Ok(StatusesReconciled {
            items,
            status_is_closure: false,
        })
    }

    /// One contradiction brought in line, or reported as it would be.
    async fn reconciled(
        &self,
        source: &ResolvedSource,
        contradiction: StatusContradiction,
        dry_run: bool,
    ) -> StatusReconciled {
        let StatusContradiction { task, option, to } = contradiction;
        let id = GlobalId::new(source.name().clone(), task.id.clone());
        let entry = |action, to, error| StatusReconciled {
            source: id.clone(),
            from: option.clone(),
            to,
            action,
            error,
        };
        if dry_run {
            return entry(ReconcileAction::WouldUpdate, to, None);
        }
        let written = self
            .write_category(
                source,
                &id,
                task.classification,
                &task.repositories,
                to.category,
            )
            .await;
        match written {
            Ok(status) => entry(ReconcileAction::Updated, status, None),
            Err(error) => entry(ReconcileAction::Failed, to, Some(Failure::from(&error))),
        }
    }

    /// Move to `done` every task the configured sources hold for `issues`, by issue reference,
    /// and keep every task each one delivers in step — or, under `dry_run`, say which would
    /// move and write nothing.
    ///
    /// The sources asked are every one the configuration declares, in configuration order —
    /// not only the default selection — or, when `only` names some, only those. Each that
    /// declares [`complete_by_issue`](onetaskgraph_plugin_api::Capabilities::complete_by_issue)
    /// is asked once per fifty references, one after another, so an issue held on two boards is
    /// closed by the first source's write and read closed by the second, which moves only its
    /// card; every other source is sent no request and listed in
    /// [`IssuesCompleted::unsupported`]. An empty `issues` sends no request at all.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] when `only` names a source nothing configures,
    /// before anything is asked. A source that fails to read, and a task whose write fails, are
    /// not errors: each is reported `failed` beside the others.
    pub async fn complete_issues(
        &self,
        issues: &[IssueRef],
        only: &[SourceName],
        dry_run: bool,
    ) -> Result<IssuesCompleted, EngineError> {
        for name in only {
            self.known(name)?;
        }
        let mut distinct: Vec<IssueRef> = Vec::with_capacity(issues.len());
        for issue in issues {
            if !distinct.contains(issue) {
                distinct.push(issue.clone());
            }
        }
        let mut asked: Vec<&ConfiguredSource> = self
            .sources
            .iter()
            .filter(|source| only.is_empty() || only.contains(source.name()))
            .collect();
        asked.sort_by(|a, b| a.name().cmp(b.name()));
        let mut completed = IssuesCompleted {
            items: Vec::new(),
            unsupported: Vec::new(),
        };
        for configured in asked {
            let source = match configured {
                ConfiguredSource::Ready(source) => source,
                ConfiguredSource::Unavailable(_) => {
                    let error = self
                        .built(configured.name())
                        .err()
                        .unwrap_or(EngineError::NoSources);
                    completed.items.extend(
                        distinct
                            .iter()
                            .map(|issue| failed_completion(issue, configured.name(), &error)),
                    );
                    continue;
                }
            };
            if !source.source().capabilities().complete_by_issue.is_native() {
                completed.unsupported.push(source.name().clone());
                continue;
            }
            if distinct.is_empty() {
                continue;
            }
            let holdings = match source.source().issue_holdings(&distinct).await {
                Ok(holdings) => holdings,
                Err(error) => {
                    let error = source_failed(source, error);
                    completed.items.extend(
                        distinct
                            .iter()
                            .map(|issue| failed_completion(issue, source.name(), &error)),
                    );
                    continue;
                }
            };
            for holding in holdings {
                completed
                    .items
                    .push(self.completion(source, holding, dry_run).await);
            }
        }
        Ok(completed)
    }

    /// What one holding came to: written `done` when it is due and this is not a dry run.
    async fn completion(
        &self,
        source: &ResolvedSource,
        holding: onetaskgraph_plugin_api::IssueHolding,
        dry_run: bool,
    ) -> IssueCompleted {
        let task = holding
            .task
            .as_ref()
            .map(|native| GlobalId::new(source.name().clone(), native.clone()));
        let mut entry = IssueCompleted {
            issue: holding.issue.clone(),
            source: source.name().clone(),
            task: task.clone(),
            from: holding.option.clone(),
            to: None,
            action: CompleteAction::Absent,
            reason: None,
            error: None,
            delivered: Vec::new(),
        };
        let (Some(id), completion) = (task, holding.completion) else {
            return entry;
        };
        match completion {
            Completion::Absent => {}
            Completion::Skipped { reason } => {
                entry.action = CompleteAction::Skipped;
                entry.reason = Some(reason);
            }
            Completion::Unchanged => {
                entry.action = CompleteAction::Unchanged;
                entry.to = holding.option.map(|name| Status {
                    category: StatusCategory::Done,
                    name,
                });
            }
            Completion::Due { to } if dry_run => {
                entry.action = CompleteAction::WouldUpdate;
                entry.to = Some(to);
            }
            Completion::Due { to } => {
                match self
                    .write_category(
                        source,
                        &id,
                        holding.classification,
                        &holding.repositories,
                        StatusCategory::Done,
                    )
                    .await
                {
                    Ok(status) => {
                        let delivers = targets(&holding.delivers, &id.source);
                        entry.delivered = self
                            .deliver(
                                &id,
                                holding.classification,
                                status.category,
                                &delivers,
                                &delivers,
                            )
                            .await;
                        entry.action = CompleteAction::Updated;
                        entry.to = Some(status);
                    }
                    Err(error) => {
                        entry.action = CompleteAction::Failed;
                        entry.to = Some(to);
                        entry.error = Some(Failure::from(&error));
                    }
                }
            }
        }
        entry
    }

    /// One task's status written through its source's own status write, after the public
    /// boundary has admitted it, answering the status as the source reads it back.
    async fn write_category(
        &self,
        source: &ResolvedSource,
        id: &GlobalId,
        classification: onetaskgraph_plugin_api::Classification,
        repositories: &[onetaskgraph_plugin_api::Repository],
        category: StatusCategory,
    ) -> Result<Status, EngineError> {
        self.admit_known(
            source,
            &id.to_string(),
            &id.native,
            classification,
            repositories,
            &Exposure::metadata([category_word(category)]),
        )
        .await?;
        source
            .source()
            .set_task_status(&id.native, category)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })
    }
}

/// A reference a source could not be asked about, reported failed with why.
fn failed_completion(issue: &IssueRef, source: &SourceName, error: &EngineError) -> IssueCompleted {
    IssueCompleted {
        issue: issue.clone(),
        source: source.clone(),
        task: None,
        from: None,
        to: None,
        action: CompleteAction::Failed,
        reason: None,
        error: Some(Failure::from(error)),
        delivered: Vec::new(),
    }
}

/// A category as the wire spells it, for the boundary's check of what a status write carries.
fn category_word(category: StatusCategory) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}
