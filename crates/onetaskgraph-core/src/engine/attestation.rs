//! Recording and checking attestations: the engine's half of [`crate::attestation`].
//!
//! Recording one is one read of the item — what its digest is computed over, and the version
//! `if_version` is held to — then one narrow metadata write of the whole
//! `onetaskgraph.attestations` entry, through the same seam `metadata set` writes through, so
//! a public write is screened exactly as that one is. A document's write takes the read rather
//! than reading again. Checking one is the read alone. A task's or a project's own forward
//! edges are read beside it, and each far end once, for the title the digest names it by.

use onetaskgraph_plugin_api::{
    DependencyEdge, Document, ItemKind, MetadataKey, MetadataRecord, Repository, SourceName,
};
use serde_json::Value;

use super::copy::{Level, forward_edges};
use super::delivery::source_failed;
use super::{Engine, EngineError};
use crate::GlobalId;
use crate::attestation::{
    Attestation, AttestationCheck, AttestationKind, AttestationLabel, AttestationNote, Attested,
    Authored, AuthoredEdge, FarEnd, ProjectAttestationCheck, TaskAttestationCheck, check,
    content_digest, now, record_of, records,
};
use crate::resolve::ResolvedSource;
use crate::template::Sha256Digest;

/// One attestation to record.
#[derive(Debug, Clone, PartialEq)]
pub struct AttestRequest {
    /// The item to record it on.
    pub id: GlobalId,
    /// Its kind: a record of this kind the item already holds is replaced.
    pub kind: AttestationKind,
    /// Who attests, as the caller labels them.
    pub by: AttestationLabel,
    /// The fingerprint of the bar the content is cleared against.
    pub bar: AttestationLabel,
    /// Caller-owned metadata keys to leave out of the digest.
    pub excludes: Vec<MetadataKey>,
    /// What the attester adds, if anything.
    pub note: Option<AttestationNote>,
    /// Record it only while the item is at this version: the read it is composed from is
    /// refused, writing nothing, when it answers another.
    pub if_version: Option<String>,
}

/// One item as the digest needs it.
struct Read {
    title: String,
    content: Option<String>,
    repositories: Vec<Repository>,
    metadata: std::collections::BTreeMap<String, Value>,
    version: Option<String>,
    edges: Vec<AuthoredEdge>,
    /// The document as it was read, which its write takes rather than reading again.
    document: Option<Document>,
}

impl Read {
    fn digest(&self, excludes: &[MetadataKey]) -> Sha256Digest {
        content_digest(
            &Authored {
                title: &self.title,
                content: self.content.as_deref(),
                repositories: &self.repositories,
                metadata: &self.metadata,
                depends_on: self.edges.clone(),
            },
            excludes,
        )
    }
}

impl Engine {
    /// Record one attestation on one task, project or document, replacing the record of its
    /// kind the item holds; every other kind's record is left as it was.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::MetadataNotWritable`] for one with no write side,
    /// [`EngineError::NoSuchTask`], [`EngineError::NoSuchProject`] or
    /// [`EngineError::NoSuchDocument`] when the item is not there,
    /// [`EngineError::VersionMoved`] when `if_version` is not the version the item is at, and
    /// every refusal a narrow metadata write can answer — the public boundary's among them.
    pub async fn attest(
        &self,
        record: MetadataRecord,
        request: &AttestRequest,
    ) -> Result<Attested, EngineError> {
        let id = &request.id;
        let source = self.metadata_writable(id, record)?;
        let read = self.read_authored(source, record, id).await?;
        if let Some(expected) = &request.if_version {
            let at = read.version.clone().unwrap_or_default();
            if source.source().order_versions(&at, expected) != Some(std::cmp::Ordering::Equal) {
                return Err(EngineError::VersionMoved {
                    id: id.to_string(),
                    expected: expected.clone(),
                    found: at,
                });
            }
        }
        let mut excludes = request.excludes.clone();
        excludes.sort();
        excludes.dedup();
        let attestation = Attestation {
            kind: request.kind.clone(),
            by: request.by.clone(),
            at: now(),
            content_digest: read.digest(&excludes),
            bar: request.bar.clone(),
            note: request.note.clone(),
            excludes,
        };
        let mut entries: serde_json::Map<String, Value> =
            records(&read.metadata).into_iter().collect();
        entries.insert(request.kind.as_str().to_owned(), attestation.to_value());
        let key = MetadataKey::attestations();
        let value = Value::Object(entries);
        let set = match (record, read.document) {
            (MetadataRecord::Task, _) => self.set_task_metadata(id, &key, &value).await?,
            (MetadataRecord::Project, _) => self.set_project_metadata(id, &key, &value).await?,
            (MetadataRecord::Document, Some(document)) => {
                self.set_read_document_metadata(source, id, &key, &value, &document)
                    .await?
            }
            (MetadataRecord::Document, None) => {
                self.set_document_metadata(id, &key, &value).await?
            }
        };
        // What the source holds under the key was read back by the write itself — a write that
        // answers without the key is refused there — so the record answered is the one written.
        let _ = set;
        Ok(Attested {
            id: id.clone(),
            record: attestation,
        })
    }

    /// Check one kind of attestation on one task, project or document: `valid` when its
    /// record's content is the item's and — when `bar` is given — its bar is `bar`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::NoDocuments`] for a document of a source holding none,
    /// [`EngineError::NoSuchTask`], [`EngineError::NoSuchProject`] or
    /// [`EngineError::NoSuchDocument`] when the item is not there, and
    /// [`EngineError::SourceFailed`] when a read fails.
    pub async fn check_attestation(
        &self,
        record: MetadataRecord,
        id: &GlobalId,
        kind: &AttestationKind,
        bar: Option<&str>,
    ) -> Result<AttestationCheck, EngineError> {
        let source = self.built(&id.source)?;
        if record == MetadataRecord::Document
            && !source.source().capabilities().documents.is_native()
        {
            return Err(EngineError::NoDocuments {
                name: source.name().to_string(),
                kind: source.kind().to_owned(),
            });
        }
        let read = self.read_authored(source, record, id).await?;
        Ok(check(
            kind,
            record_of(&read.metadata, kind),
            |excludes| read.digest(excludes),
            bar,
        ))
    }

    /// Check one kind of attestation on every task filed under one project, in the order the
    /// source lists them.
    ///
    /// # Errors
    ///
    /// As [`check_attestation`](Self::check_attestation), and
    /// [`EngineError::SourceRefused`] when the project's tasks cannot be listed whole.
    pub async fn check_project_attestations(
        &self,
        project: &GlobalId,
        kind: &AttestationKind,
        bar: Option<&str>,
    ) -> Result<ProjectAttestationCheck, EngineError> {
        let source = self.built(&project.source)?;
        source
            .source()
            .get_project(&project.native)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchProject {
                id: project.to_string(),
            })?;
        let listed = self
            .project_member_tasks(project, "for an attestation check")
            .await?;
        let mut tasks = Vec::with_capacity(listed.len());
        for task in listed {
            let read = self
                .authored_from(
                    source,
                    &task.id,
                    ItemKind::Task,
                    Authoring {
                        title: task.item.title,
                        content: task.item.content,
                        repositories: task.item.repositories,
                        metadata: task.item.metadata,
                        version: task.item.version,
                        document: None,
                    },
                )
                .await?;
            tasks.push(TaskAttestationCheck {
                id: task.id,
                check: check(
                    kind,
                    record_of(&read.metadata, kind),
                    |excludes| read.digest(excludes),
                    bar,
                ),
            });
        }
        Ok(ProjectAttestationCheck {
            project: project.clone(),
            kind: kind.clone(),
            tasks,
        })
    }

    /// Read the item `id` names and its own forward edges, as the digest reads them.
    async fn read_authored(
        &self,
        source: &ResolvedSource,
        record: MetadataRecord,
        id: &GlobalId,
    ) -> Result<Read, EngineError> {
        let missing = || match record {
            MetadataRecord::Task => EngineError::NoSuchTask { id: id.to_string() },
            MetadataRecord::Project => EngineError::NoSuchProject { id: id.to_string() },
            MetadataRecord::Document => EngineError::NoSuchDocument { id: id.to_string() },
        };
        let (kind, authoring) = match record {
            MetadataRecord::Task => {
                let task = source
                    .source()
                    .get_task(&id.native)
                    .await
                    .map_err(|error| source_failed(source, error))?
                    .ok_or_else(missing)?;
                (
                    ItemKind::Task,
                    Authoring {
                        title: task.title,
                        content: task.content,
                        repositories: task.repositories,
                        metadata: task.metadata,
                        version: task.version,
                        document: None,
                    },
                )
            }
            MetadataRecord::Project => {
                let project = source
                    .source()
                    .get_project(&id.native)
                    .await
                    .map_err(|error| source_failed(source, error))?
                    .ok_or_else(missing)?;
                (
                    ItemKind::Project,
                    Authoring {
                        title: project.title,
                        content: project.content,
                        repositories: project.repositories,
                        metadata: project.metadata,
                        version: project.version,
                        document: None,
                    },
                )
            }
            MetadataRecord::Document => {
                let document = source
                    .source()
                    .get_document(&id.native)
                    .await
                    .map_err(|error| source_failed(source, error))?
                    .ok_or_else(missing)?;
                return Ok(Read {
                    title: document.title.clone(),
                    content: document.content.clone(),
                    repositories: document.repositories.clone(),
                    metadata: document.metadata.clone(),
                    version: document.version.clone(),
                    edges: Vec::new(),
                    document: Some(document),
                });
            }
        };
        self.authored_from(source, id, kind, authoring).await
    }

    /// `authoring`, with its forward edges read and each far end named.
    async fn authored_from(
        &self,
        source: &ResolvedSource,
        id: &GlobalId,
        kind: ItemKind,
        authoring: Authoring,
    ) -> Result<Read, EngineError> {
        let level = match kind {
            ItemKind::Task => Level::Task,
            ItemKind::Project => Level::Project,
        };
        let mut edges = Vec::new();
        for edge in forward_edges(source, &id.native, level).await? {
            edges.push(self.edge_entry(source.name(), &edge).await?);
        }
        Ok(Read {
            title: authoring.title,
            content: authoring.content,
            repositories: authoring.repositories,
            metadata: authoring.metadata,
            version: authoring.version,
            edges,
            document: authoring.document,
        })
    }

    /// One edge as the digest names it: its kind, its far end's kind, and its far end by the
    /// title the store reads it at — or by its id, when no configured source can say.
    async fn edge_entry(
        &self,
        near: &SourceName,
        edge: &DependencyEdge,
    ) -> Result<AuthoredEdge, EngineError> {
        let entry = |end: FarEnd| AuthoredEdge {
            kind: edge.kind,
            far: edge.to.kind,
            end,
        };
        let (source_name, native) = if edge.to.is_native() {
            (near.to_string(), edge.to.id().to_owned())
        } else if let Some((source, native)) = edge.to.id().split_once(':')
            && edge.to.is_qualified()
        {
            (source.to_owned(), native.to_owned())
        } else {
            return Ok(entry(FarEnd::Id(edge.to.id().to_owned())));
        };
        let named = SourceName::new(source_name.clone()).ok();
        let far = match named.as_ref().map(|name| self.built(name)) {
            Some(Ok(far)) => Some(far),
            _ => None,
        };
        let title = match far {
            Some(far) => {
                let native = onetaskgraph_plugin_api::NativeId::from(native.as_str());
                match edge.to.kind {
                    ItemKind::Task => far
                        .source()
                        .get_task(&native)
                        .await
                        .map_err(|error| source_failed(far, error))?
                        .map(|task| task.title),
                    ItemKind::Project => far
                        .source()
                        .get_project(&native)
                        .await
                        .map_err(|error| source_failed(far, error))?
                        .map(|project| project.title),
                }
            }
            None => None,
        };
        Ok(entry(title.map_or_else(
            || FarEnd::Id(format!("{source_name}:{native}")),
            FarEnd::Title,
        )))
    }
}

/// The members of one item the digest reads, before its edges are.
struct Authoring {
    title: String,
    content: Option<String>,
    repositories: Vec<Repository>,
    metadata: std::collections::BTreeMap<String, Value>,
    version: Option<String>,
    document: Option<Document>,
}
