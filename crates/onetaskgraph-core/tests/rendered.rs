//! Tasks and documents created from a template, their stored answers read, and both
//! regenerated in place — through the crate-root API alone, without the binary.
//!
//! Every test drives a real `local-md` folder, which keeps answers, and asserts on what a
//! later read answers and on the bytes the file holds.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use onetaskgraph_core::{
    Answers, Body, Config, DocumentCreate, Engine, EngineError, GlobalId, LoaderDocument,
    RenderRequest, RenderedRecord, TaskCreate, TemplateError, TemplateInput, TemplateProvenance,
};
use onetaskgraph_plugin_api::{MetadataKey, NativeId, SecretResolver, SourceName, StatusCategory};
use secrecy::SecretString;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// The entry template, which extends a base the loader document registers inline.
const ENTRY: &str = "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What the task is for\n    type: string\n  steps:\n    description: How it is done\n    type: list\n    default: []\n  owner:\n    description: Who does it\n    type: string\n    required: false\n---\n{% extends \"base.md\" %}{% block body %}Goal: {{ goal }}\n{% for step in steps %}- {{ step }}\n{% endfor %}{% endblock %}";

const BASE: &str = "# Task\n\n{% block body %}{% endblock %}";

/// A folder of Markdown called `work`, one in-memory source called `memory`, and a directory
/// holding the entry template.
struct Fixture {
    root: tempfile::TempDir,
    templates: tempfile::TempDir,
    engine: Engine,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("a folder of notes");
        fs::create_dir_all(root.path().join("tasks")).unwrap();
        fs::create_dir_all(root.path().join("documents")).unwrap();
        let templates = tempfile::tempdir().expect("a template directory");
        fs::write(templates.path().join("task.md"), ENTRY).unwrap();
        let config = Config::from_document(json!({"sources": {
            "work": {"plugin": "local-md", "config": {"root": root.path()}},
            "memory": {"plugin": "in-memory", "config": {"capabilities": {"documents": "native"}}},
        }}))
        .expect("a valid configuration");
        let engine = Engine::build(&config, &NoSecrets);
        Self {
            root,
            templates,
            engine,
        }
    }

    /// The loader document a caller holding its own layering would state.
    fn loader(&self, base: &str) -> LoaderDocument {
        LoaderDocument::from_json(
            &json!({
                "reference": "caller:task/default",
                "entry": "task.md",
                "search_path": [self.templates.path()],
                "templates": [{"name": "base.md", "source": base}],
                "ignored": "every other key",
            })
            .to_string(),
        )
        .expect("a loader document")
    }

    fn file(&self, id: &GlobalId) -> String {
        fs::read_to_string(self.root.path().join(format!("tasks/{}.md", id.native)))
            .expect("the task's file")
    }

    fn path(&self, id: &GlobalId) -> std::path::PathBuf {
        self.root.path().join(format!("tasks/{}.md", id.native))
    }

    async fn create(&self, template: &TemplateInput, answers: &Answers) -> GlobalId {
        let rendered = template.load().unwrap().render(answers).unwrap();
        let created = self
            .engine
            .create_task(&TaskCreate {
                source: name("work"),
                project: NativeId::from("launch"),
                title: "Ship the release".to_owned(),
                body: Body::rendered(template, rendered),
                status: Some(StatusCategory::Todo),
                labels: vec!["release".to_owned()],
                repositories: Vec::new(),
                depends_on: Vec::new(),
                delivers: Vec::new(),
                metadata: BTreeMap::from([(
                    MetadataKey::new("caller.kept").unwrap(),
                    json!({"nested": [1, true]}),
                )]),
            })
            .await
            .expect("the task is created");
        created.task.id
    }
}

fn name(value: &str) -> SourceName {
    SourceName::new(value).unwrap()
}

fn answers(value: Value) -> Answers {
    let mut answers = Answers::new();
    for (name, value) in value.as_object().unwrap() {
        answers.set(name.clone(), value.clone());
    }
    answers
}

fn sha256(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

async fn task(engine: &Engine, id: &GlobalId) -> onetaskgraph_plugin_api::Task {
    engine.task(id).await.unwrap().items.remove(0).item
}

#[tokio::test]
async fn a_task_created_from_a_loader_document_records_provenance_and_stores_its_answers() {
    let fixture = Fixture::new();
    let template = TemplateInput::Loader(fixture.loader(BASE));
    let id = fixture
        .create(
            &template,
            &answers(json!({"goal": "Ship it", "steps": ["build"]})),
        )
        .await;

    let read = task(&fixture.engine, &id).await;
    let content = read.content.clone().unwrap();
    assert_eq!(content, "# Task\n\nGoal: Ship it\n- build\n");
    let provenance = TemplateProvenance::read(&read.metadata).unwrap().unwrap();
    assert_eq!(
        provenance.template, "caller:task/default",
        "the reference, verbatim"
    );
    assert_eq!(provenance.body_digest, sha256(&content));
    assert_eq!(
        provenance.answers_digest,
        sha256(r#"{"goal":"Ship it","owner":null,"steps":["build"]}"#)
    );
    assert_eq!(provenance.digest, template.load().unwrap().digest());
    assert_eq!(read.metadata["caller.kept"], json!({"nested": [1, true]}));

    let stored = fixture
        .engine
        .template_answers(RenderedRecord::Task, &id)
        .await
        .unwrap();
    assert_eq!(
        stored.0,
        BTreeMap::from([
            ("goal".to_owned(), json!("Ship it")),
            ("owner".to_owned(), Value::Null),
            ("steps".to_owned(), json!(["build"])),
        ])
    );
    assert!(!read.metadata.contains_key("goal"), "no answer is metadata");
}

#[tokio::test]
async fn a_regenerate_overlays_new_answers_and_an_unchanged_one_writes_nothing() {
    let fixture = Fixture::new();
    let template = TemplateInput::Loader(fixture.loader(BASE));
    let id = fixture
        .create(
            &template,
            &answers(json!({"goal": "Ship it", "steps": ["build"]})),
        )
        .await;

    let mut overlay = Answers::new();
    overlay.set_text("goal", "Ship it twice");
    let regenerated = fixture
        .engine
        .render_task(
            &id,
            &RenderRequest {
                template: Some(template.clone()),
                answers: overlay,
                ..RenderRequest::default()
            },
        )
        .await
        .unwrap();
    let fresh = template
        .load()
        .unwrap()
        .render(&answers(
            json!({"goal": "Ship it twice", "steps": ["build"]}),
        ))
        .unwrap();
    assert!(regenerated.changed);
    assert_eq!(
        regenerated.body, fresh.body,
        "a fresh render of the overlaid answers"
    );
    assert_eq!(regenerated.digest, fresh.digest, "the chain did not change");
    assert_eq!(
        task(&fixture.engine, &id).await.content.as_deref(),
        Some(fresh.body.as_str())
    );

    let before = fixture.file(&id);
    let again = fixture
        .engine
        .render_task(
            &id,
            &RenderRequest {
                template: Some(template),
                ..RenderRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(!again.changed);
    assert_eq!(fixture.file(&id), before, "byte-identical");
}

#[tokio::test]
async fn an_edited_loader_document_changes_the_digest_and_the_rendering() {
    let fixture = Fixture::new();
    let id = fixture
        .create(
            &TemplateInput::Loader(fixture.loader(BASE)),
            &answers(json!({"goal": "Ship it"})),
        )
        .await;
    let before = TemplateProvenance::read(&task(&fixture.engine, &id).await.metadata)
        .unwrap()
        .unwrap();

    let edited =
        TemplateInput::Loader(fixture.loader("# Task, edited\n\n{% block body %}{% endblock %}"));
    let regenerated = fixture
        .engine
        .render_task(
            &id,
            &RenderRequest {
                template: Some(edited),
                ..RenderRequest::default()
            },
        )
        .await
        .unwrap();

    assert_ne!(
        regenerated.digest, before.digest,
        "an inline pair is a chain source"
    );
    assert_eq!(regenerated.body, "# Task, edited\n\nGoal: Ship it\n");
    let after = TemplateProvenance::read(&task(&fixture.engine, &id).await.metadata)
        .unwrap()
        .unwrap();
    assert_eq!(after.digest, regenerated.digest);
    assert_eq!(
        after.answers_digest, before.answers_digest,
        "the same answers"
    );
}

#[tokio::test]
async fn answers_out_of_step_with_the_provenance_need_every_required_answer() {
    let fixture = Fixture::new();
    let template = TemplateInput::Loader(fixture.loader(BASE));
    let id = fixture
        .create(
            &template,
            &answers(json!({"goal": "Ship it", "steps": ["build"]})),
        )
        .await;
    let path = fixture.path(&id);
    let edited = fixture
        .file(&id)
        .replace("goal: Ship it", "goal: Forged by hand");
    fs::write(&path, &edited).unwrap();

    let refused = fixture
        .engine
        .render_task(
            &id,
            &RenderRequest {
                template: Some(template.clone()),
                answers: answers(json!({"steps": ["x"]})),
                ..RenderRequest::default()
            },
        )
        .await
        .expect_err("partial answers are refused");
    let EngineError::MissingAnswers { names, reason, .. } = &refused else {
        panic!("expected missing answers, got {refused:?}");
    };
    assert_eq!(names, &["goal".to_owned()]);
    assert!(reason.contains("answers_digest"), "{reason}");
    assert!(
        refused
            .to_string()
            .starts_with("supply every required answer")
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        edited,
        "nothing written"
    );

    let regenerated = fixture
        .engine
        .render_task(
            &id,
            &RenderRequest {
                template: Some(template),
                answers: answers(json!({"goal": "Ship it"})),
                ..RenderRequest::default()
            },
        )
        .await
        .expect("every required answer succeeds");
    assert_eq!(
        regenerated.body, "# Task\n\nGoal: Ship it\n",
        "steps took their default"
    );
    assert_eq!(
        fixture
            .engine
            .template_answers(RenderedRecord::Task, &id)
            .await
            .unwrap()
            .0["goal"],
        json!("Ship it")
    );
}

#[tokio::test]
async fn a_regenerate_needs_a_template_it_can_read_and_never_turns_a_reference_into_one() {
    let fixture = Fixture::new();
    let loader = TemplateInput::Loader(fixture.loader(BASE));
    let from_loader = fixture
        .create(&loader, &answers(json!({"goal": "Ship it"})))
        .await;
    let plain = fixture
        .engine
        .create_task(&TaskCreate {
            source: name("work"),
            project: NativeId::from("launch"),
            title: "Plain".to_owned(),
            body: Body::Plain("By hand.".to_owned()),
            status: None,
            labels: Vec::new(),
            repositories: Vec::new(),
            depends_on: Vec::new(),
            delivers: Vec::new(),
            metadata: BTreeMap::new(),
        })
        .await
        .unwrap()
        .task;
    assert!(!plain.item.metadata.contains_key(TemplateProvenance::KEY));

    let none = fixture
        .engine
        .render_task(&plain.id, &RenderRequest::default())
        .await
        .expect_err("no provenance and no template");
    assert!(matches!(none, EngineError::NoTemplate { .. }), "{none:?}");

    let reference = fixture
        .engine
        .render_task(&from_loader, &RenderRequest::default())
        .await
        .expect_err("a recorded reference is not a file");
    let EngineError::TemplateNotAFile { reference, .. } = &reference else {
        panic!("expected the recorded reference named, got {reference:?}");
    };
    assert_eq!(reference, "caller:task/default");

    // A recorded file path, by contrast, is re-read as a file.
    let file = TemplateInput::File {
        path: fixture.templates.path().join("task.md"),
        search_path: Vec::new(),
    };
    fs::write(
        fixture.templates.path().join("task.md"),
        ENTRY.replace("{% extends \"base.md\" %}", ""),
    )
    .unwrap();
    let from_file = fixture
        .create(&file, &answers(json!({"goal": "From a file"})))
        .await;
    let recorded = TemplateProvenance::read(&task(&fixture.engine, &from_file).await.metadata)
        .unwrap()
        .unwrap();
    assert!(Path::new(&recorded.template).is_absolute());
    fs::write(
        fixture.templates.path().join("task.md"),
        ENTRY.replace("{% extends \"base.md\" %}", "Edited. "),
    )
    .unwrap();
    let regenerated = fixture
        .engine
        .render_task(&from_file, &RenderRequest::default())
        .await
        .unwrap();
    assert!(regenerated.body.starts_with("Edited. Goal: From a file"));
    assert_ne!(regenerated.digest, recorded.digest, "the file changed");
}

#[tokio::test]
async fn a_dry_run_renders_and_writes_nothing() {
    let fixture = Fixture::new();
    let template = TemplateInput::Loader(fixture.loader(BASE));
    let id = fixture
        .create(&template, &answers(json!({"goal": "Ship it"})))
        .await;
    let before = fixture.file(&id);

    let mut overlay = Answers::new();
    overlay.set_text("goal", "Changed");
    let dry = fixture
        .engine
        .render_task(
            &id,
            &RenderRequest {
                template: Some(template),
                answers: overlay,
                dry_run: true,
                ..RenderRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(dry.changed, "it would change");
    assert!(dry.body.contains("Goal: Changed"));
    assert_eq!(fixture.file(&id), before);
}

#[tokio::test]
async fn a_loader_document_is_refused_by_name_when_it_is_malformed_or_its_digest_differs() {
    let fixture = Fixture::new();
    for (document, named) in [
        ("[]", "not a JSON object"),
        ("{\"entry\": \"task.md\"}", "`reference`"),
        ("{\"reference\": \"r\"}", "`entry`"),
        (
            "{\"reference\": \"r\", \"entry\": \"e\", \"search_path\": [\"relative\"]}",
            "absolute",
        ),
        (
            "{\"reference\": \"r\", \"entry\": \"e\", \"templates\": [{\"name\": \"n\"}]}",
            "`source`",
        ),
    ] {
        let refused = LoaderDocument::from_json(document).expect_err(document);
        assert!(refused.to_string().contains(named), "{document}: {refused}");
    }
    let stated = format!("sha256:{}", "0".repeat(64));
    let refused = fixture
        .loader(BASE)
        .with_digest(stated.clone())
        .load()
        .expect_err("a digest the chain does not compute");
    let TemplateError::LoaderDigest {
        stated: named,
        computed,
    } = &refused
    else {
        panic!("expected both digests, got {refused:?}");
    };
    assert_eq!(named, &stated);
    assert_eq!(computed, fixture.loader(BASE).load().unwrap().digest());
    let missing = LoaderDocument::new("r", "task.md")
        .unwrap()
        .with_directory(fixture.templates.path().join("missing"))
        .unwrap()
        .load()
        .expect_err("an unreadable directory");
    assert!(missing.to_string().contains("missing"), "{missing}");
    // The builders refuse exactly what the JSON boundary refuses.
    assert!(LoaderDocument::new("", "task.md").is_err());
    assert!(LoaderDocument::new("r", "").is_err());
    assert!(
        LoaderDocument::new("r", "task.md")
            .unwrap()
            .with_directory("relative")
            .is_err()
    );
}

#[tokio::test]
async fn documents_are_created_replaced_and_regenerated_and_a_source_keeping_none_says_so() {
    let fixture = Fixture::new();
    let template = TemplateInput::Loader(fixture.loader(BASE));
    let rendered = template
        .load()
        .unwrap()
        .render(&answers(json!({"goal": "Design it"})))
        .unwrap();
    let request = DocumentCreate {
        source: name("work"),
        project: NativeId::from("launch"),
        title: "Design".to_owned(),
        id: Some(NativeId::from("design")),
        body: Body::rendered(&template, rendered),
        labels: Vec::new(),
        repositories: Vec::new(),
        metadata: BTreeMap::new(),
    };
    let created = fixture.engine.create_document(&request).await.unwrap();
    assert_eq!(created.id.to_string(), "work:design");
    let replaced = fixture
        .engine
        .create_document(&DocumentCreate {
            title: "Design, replaced".to_owned(),
            ..request
        })
        .await
        .unwrap();
    assert_eq!(replaced.id, created.id, "the same document, replaced");
    assert_eq!(replaced.item.title, "Design, replaced");
    assert_eq!(
        fixture
            .engine
            .template_answers(RenderedRecord::Document, &created.id)
            .await
            .unwrap()
            .0["goal"],
        json!("Design it")
    );

    let regenerated = fixture
        .engine
        .render_document(
            &created.id,
            &RenderRequest {
                template: Some(template),
                answers: answers(json!({"goal": "Design it again"})),
                ..RenderRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(regenerated.changed);
    let read = fixture
        .engine
        .document(&created.id)
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        read.item.content.as_deref(),
        Some(regenerated.body.as_str())
    );
    assert_eq!(read.item.title, "Design, replaced", "the title is kept");

    let memory = fixture
        .engine
        .create_document(&DocumentCreate {
            source: name("memory"),
            project: NativeId::from("launch"),
            title: "Memo".to_owned(),
            id: None,
            body: Body::Plain("Memo.".to_owned()),
            labels: Vec::new(),
            repositories: Vec::new(),
            metadata: BTreeMap::new(),
        })
        .await
        .unwrap();
    let none = fixture
        .engine
        .template_answers(RenderedRecord::Document, &memory.id)
        .await
        .expect_err("a source keeping no answers");
    assert!(
        none.to_string().contains(&format!(
            "document {} has no stored template answers",
            memory.id
        )),
        "{none}"
    );
}
