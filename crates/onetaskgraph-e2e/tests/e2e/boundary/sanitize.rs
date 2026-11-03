//! Replacement on every screened write, driven the way a user drives it: a store whose
//! `write_policy` names the released onevcs `boundary sanitize` command, against an onevcs home
//! this run owns holding the invented private identities of [`super::registered`].
//!
//! The term every journey here declares a replacement for is `quietharbor`, the name of the
//! invented private repository `hiddenco/quietharbor`: onevcs derives it, and rejects it unless a
//! replacement says otherwise. A store's own `replacements`, the environment and `--replace`
//! each supply one, and what lands in the public folder is held to carrying none of it.

use serde_json::{Value, json};

use super::{PLAIN, Store, files_under, registered, tree};
use crate::common::{stderr, stdout};
use crate::onevcs::{BoundaryCommand, BoundaryRecord, OnevcsHome};

/// The invented private term every journey here replaces.
const TERM: &str = "quietharbor";
/// What the store's own replacement rewrites it to.
const WITH: &str = "harbor-x";

/// A store of `plan` (private) and `site` (public) whose policy runs the released onevcs's four
/// boundary commands through a recorder, with `replacements` as its own replacement layer.
fn sanitizing(replacements: Value) -> (Store, BoundaryRecord) {
    let mut record = None;
    let store = Store::laid(Some(registered), |_| json!({}));
    let home = store.onevcs.as_ref().expect("an onevcs home");
    let recorded = home.recorded_sanitizing(&store.sandbox, replacements);
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["write_policy"] = recorded.write_policy().clone();
    store.sandbox.project_document(&document.to_string());
    record.get_or_insert(recorded);
    store.public_project_p();
    (store, record.expect("a record"))
}

/// The store's own replacement of [`TERM`] by [`WITH`].
fn replaced() -> Value {
    json!([{ "term": TERM, "with": WITH }])
}

/// How many times the store ran `command` since `before` calls had been recorded.
fn ran(record: &BoundaryRecord, before: usize, command: BoundaryCommand) -> usize {
    record.calls()[before..]
        .iter()
        .filter(|call| call.command == command)
        .count()
}

/// Every file under `folder`, read as text, joined.
fn text_of(store: &Store, folder: &str) -> String {
    tree(&store.folder(folder))
        .into_values()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A template declaring a `goal` it renders and `notes` it keeps without rendering.
fn template(store: &Store) -> String {
    let path = store.sandbox.config_home().join("sanitize-template.md");
    std::fs::write(
        &path,
        "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What it is for\n  \
         notes:\n    description: Kept, never rendered\n    required: false\n---\n# {{ goal }}\n",
    )
    .expect("a template");
    path.display().to_string()
}

#[test]
fn every_rewritable_field_a_public_write_carries_lands_with_the_term_replaced_by_one_call() {
    let (store, record) = sanitizing(replaced());
    let body = store.body(&format!("Retry the way {TERM} does.\n"));

    // A create: body, title, label and a metadata value, rewritten by one sanitisation.
    let before = record.calls().len();
    let created: Value = serde_json::from_str(&store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        &format!("Port the {TERM} retry"),
        "--label",
        &format!("{TERM}-ops"),
        "--metadata",
        &format!("myapp.note=\"seen in {TERM}\""),
        "--body-file",
        &body,
        "--json",
    ]))
    .expect("the created task");
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    assert_eq!(ran(&record, before, BoundaryCommand::Check), 0);
    let sent = &record.calls()[record.calls().len() - 1].input;
    assert!(
        sent["replacements"] == replaced(),
        "the store's replacement layer is sent: {sent}"
    );
    assert_eq!(
        created["sanitization"]["replaced"],
        json!(4),
        "the create says how many spans were replaced: {created}"
    );
    assert_eq!(created["sanitization"]["side_effect"], json!([]));
    let task = &created["items"][0]["item"];
    assert_eq!(task["title"], json!(format!("Port the {WITH} retry")));
    assert_eq!(task["labels"][0]["name"], json!(format!("{WITH}-ops")));
    assert_eq!(
        task["metadata"]["myapp.note"],
        json!(format!("seen in {WITH}"))
    );
    assert!(
        task["content"]
            .as_str()
            .is_some_and(|content| content.contains(&format!("the way {WITH} does"))),
        "{task}"
    );
    let id = created["items"][0]["id"]
        .as_str()
        .expect("the created task's id")
        .to_owned();

    // An update and a comment: one sanitisation each, and what it answered is what is stored.
    let before = record.calls().len();
    store.ok(&["task", "update", &id, "--title", &format!("{TERM} again")]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    let comment = store.body(&format!("Asked the {TERM} team.\n"));
    let before = record.calls().len();
    store.ok(&["task", "comment", "add", &id, "--body-file", &comment]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    let answered = record.calls().last().expect("a call").clone();
    let shown: Value =
        serde_json::from_str(&store.ok(&["task", "show", &id, "--json"])).expect("a task");
    assert_eq!(
        shown["items"][0]["item"]["title"],
        json!(format!("{WITH} again"))
    );
    let comment_body = shown["comments"][0]["body"].as_str().expect("a comment");
    assert_eq!(comment_body, format!("Asked the {WITH} team.\n"));
    assert!(
        answered.input["text"]
            .as_array()
            .is_some_and(|text| text.contains(&json!(format!("Asked the {TERM} team.\n")))),
        "the comment's body was what the call was handed: {}",
        answered.input
    );

    // A document's and a project's titles.
    let before = record.calls().len();
    store.ok(&[
        "document",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        &format!("{TERM} notes"),
        "--body-file",
        &store.body(PLAIN),
    ]);
    store.ok(&[
        "project",
        "create",
        "site",
        "--id",
        "q",
        "--title",
        &format!("The {TERM} port"),
        "--body-file",
        &store.body(PLAIN),
    ]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 2);

    // A rendering's answers: the answer the template renders, and the one it only keeps.
    let template = template(&store);
    let before = record.calls().len();
    store.ok(&[
        "task",
        "create",
        "site",
        "--project",
        "p",
        "--title",
        "Rendered",
        "--template",
        &template,
        "--var",
        &format!("goal=Match {TERM}"),
        "--var",
        &format!("notes=Background from {TERM}"),
        "--no-interactive",
    ]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);

    // A copy out of the private folder: every field and the comment it carries, one call.
    store.record(
        "plan",
        "tasks",
        "carried",
        &format!(
            "title: Carry the {TERM} fix\nstatus: todo\nlabels: [{TERM}-carried]\nmetadata:\n  \
             why: from {TERM}"
        ),
        &format!(
            "Copied out of {TERM}.\n\n## Comments\n\n<!-- onetaskgraph:comment \
             id=\"20260913T151107Z-1\" author=\"ada\" created_at=\"2026-09-13T15:11:07Z\" \
             updated_at=\"2026-09-13T15:11:07Z\" -->\n### ada — 2026-09-13T15:11:07Z\n\n\
             Ask {TERM} first.\n\n<!-- /onetaskgraph:comment -->"
        ),
    );
    let before = record.calls().len();
    store.ok(&["task", "copy", "plan:carried", "--to", "site", "--comments"]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);

    let public = text_of(&store, "site");
    assert!(
        !public.to_lowercase().contains(TERM),
        "no public file carries the term:\n{public}"
    );
    for written in [
        format!("{WITH} again"),
        format!("{WITH}-ops"),
        format!("seen in {WITH}"),
        format!("Asked the {WITH} team."),
        format!("{WITH} notes"),
        format!("The {WITH} port"),
        format!("Match {WITH}"),
        format!("Background from {WITH}"),
        format!("Carry the {WITH} fix"),
        format!("{WITH}-carried"),
        format!("Ask {WITH} first."),
    ] {
        assert!(
            public.contains(&written),
            "the public folder holds {written:?}:\n{public}"
        );
    }
}

#[test]
fn a_term_in_a_field_replacement_never_rewrites_is_refused_and_nothing_is_written() {
    let (store, record) = sanitizing(replaced());
    let before_tree = tree(&store.folder("site"));
    let create = |extra: &[&str]| -> Vec<String> {
        let mut arguments: Vec<String> = [
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            "Plain",
            "--body-file",
        ]
        .iter()
        .map(|word| (*word).to_owned())
        .collect();
        arguments.push(store.body(PLAIN));
        arguments.extend(extra.iter().map(|word| (*word).to_owned()));
        arguments
    };
    let asset = store.sandbox.config_home().join(format!("{TERM}.png"));
    std::fs::write(&asset, onetaskgraph_e2e_support::images::png(7, 60_000)).expect("an image");
    let asset_body = store.body(&format!("![shot](./{TERM}.png)\n"));
    for (what, arguments) in [
        (
            "a metadata key",
            create(&["--metadata", &format!("{TERM}.key=\"x\"")]),
        ),
        (
            "a dependency id",
            create(&["--depends-on", &format!("site:{TERM}-dep")]),
        ),
        (
            "an asset name",
            vec![
                "task".to_owned(),
                "create".to_owned(),
                "site".to_owned(),
                "--project".to_owned(),
                "p".to_owned(),
                "--title".to_owned(),
                "Shot".to_owned(),
                "--body-file".to_owned(),
                asset_body.clone(),
                "--asset".to_owned(),
                asset.display().to_string(),
            ],
        ),
        (
            "a repository",
            create(&["--repository", "github.com/hiddenco/quietharbor"]),
        ),
    ] {
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let (kind, said) = store.refused(&arguments);
        assert!(
            [
                "boundary-refused",
                "not-private-destination",
                "private-member"
            ]
            .contains(&kind.as_str()),
            "{what}: {kind}: {said}"
        );
        assert!(
            !said.contains(TERM),
            "{what}: the refusal is neutral: {said}"
        );
        assert_eq!(
            tree(&store.folder("site")),
            before_tree,
            "{what}: nothing was written"
        );
    }
    // None of them was answered rewritten.
    for call in record.calls() {
        if call.command == BoundaryCommand::Sanitize {
            assert_ne!(
                call.exit,
                Some(0),
                "a refused write was not passed: {call:?}"
            );
        }
    }
}

#[test]
fn a_destination_declared_private_receives_the_original_text() {
    let (store, record) = sanitizing(replaced());
    store.record("plan", "projects", "p", "title: P\nstatus: todo", PLAIN);
    let before = record.calls().len();
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "p",
        "--title",
        &format!("Keep {TERM}"),
        "--body-file",
        &store.body(&format!("Exactly as {TERM} says.\n")),
    ]);
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 0);
    let private = text_of(&store, "plan");
    assert!(private.contains(&format!("Keep {TERM}")), "{private}");
    assert!(
        private.contains(&format!("Exactly as {TERM} says.")),
        "{private}"
    );
}

/// `quietharbor` as the store's own layer, the environment and `--replace` each set it.
#[test]
fn a_replacement_set_in_the_file_the_environment_or_the_flag_takes_effect_flag_over_environment_over_file()
 {
    let (store, _record) = sanitizing(json!([{ "term": TERM, "with": "file-word" }]));
    let title = |store: &Store, environment: &[(&str, &str)], flags: &[&str]| -> String {
        let mut command = store.sandbox.command();
        command.args([
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            &format!("From {TERM}"),
            "--body-file",
            &store.body(PLAIN),
            "--json",
        ]);
        command.args(flags);
        command.env("ONEVCS_HOME", store.onevcs.as_ref().expect("a home").path());
        for (name, value) in environment {
            command.env(name, value);
        }
        let output = command.assert().get_output().clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let created: Value = serde_json::from_str(&stdout(&output)).expect("a task");
        created["items"][0]["item"]["title"]
            .as_str()
            .expect("a title")
            .to_owned()
    };
    let environment = [
        ("ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__TERM", TERM),
        (
            "ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__WITH",
            "env-word",
        ),
    ];
    assert_eq!(title(&store, &[], &[]), "From file-word");
    assert_eq!(title(&store, &environment, &[]), "From env-word");
    assert_eq!(
        title(
            &store,
            &environment,
            &["--replace", &format!("{TERM}=flag-word")]
        ),
        "From flag-word"
    );
    // `config show` names the layer the replacement came from.
    let mut command = store.sandbox.command();
    command.args(["config", "show", "--json"]);
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.assert().get_output().clone();
    let shown: Value = serde_json::from_str(&stdout(&output)).expect("the configuration");
    let settings = shown["settings"].as_array().expect("settings");
    let layer_of = |key: &str| {
        settings
            .iter()
            .find(|setting| setting["key"] == json!(key))
            .map(|setting| setting["origin"].clone())
            .unwrap_or_else(|| panic!("config show reports {key}: {shown}"))
    };
    assert_eq!(
        layer_of("write_policy.replacements.0.with")["variable"],
        json!("ONETASKGRAPH_WRITE_POLICY__REPLACEMENTS__0__WITH"),
        "{shown}"
    );
    // A `--reject` after the store's replacement refuses the term again.
    let mut command = store.sandbox.command();
    command
        .args([
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            &format!("From {TERM}"),
            "--body-file",
            &store.body(PLAIN),
            "--reject",
            TERM,
        ])
        .env("ONEVCS_HOME", store.onevcs.as_ref().expect("a home").path());
    let output = command.assert().get_output().clone();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
}

#[test]
fn a_caller_replacement_overrides_every_declaration_level_in_both_directions() {
    let (store, _record) = sanitizing(json!([]));
    let home: &OnevcsHome = store.onevcs.as_ref().expect("a home");
    // Three invented terms, one declared at each level onevcs reads.
    home.declare_repository_terms(
        "hiddenco",
        "quietharbor",
        "schema_version = 2\nterms = [\"emberline\"]\n[[replacements]]\nterm = \"emberline\"\n\
         with = \"repo-word\"\n",
    );
    home.declare_directory_terms(
        "schema_version = 2\nterms = [\"duskfield\"]\n[[replacements]]\nterm = \"duskfield\"\n\
         reject = true\n",
    );
    home.declare_fallback_terms(
        "schema_version = 2\nterms = [\"pinecrest\"]\n[[replacements]]\nterm = \"pinecrest\"\n\
         with = \"fallback-word\"\n",
    );
    let write = |flags: &[&str]| -> (Option<i32>, String) {
        let mut command = store.sandbox.command();
        command.args([
            "task",
            "create",
            "site",
            "--project",
            "p",
            "--title",
            "Levels",
            "--body-file",
            &store.body("emberline duskfield pinecrest quietharbor\n"),
            "--json",
            "--replace",
            "quietharbor=caller-q",
        ]);
        command.args(flags);
        command.env("ONEVCS_HOME", home.path());
        let output = command.assert().get_output().clone();
        let created: Value = serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null);
        (
            output.status.code(),
            created["items"][0]["item"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    };
    // The directory level rejects one, so without the caller nothing is written.
    assert_eq!(write(&[]).0, Some(1));
    // A caller replacement over that level's reject, and over the repository's own word.
    let (code, content) = write(&[
        "--replace",
        "duskfield=caller-d",
        "--replace",
        "emberline=caller-e",
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(content, "caller-e caller-d fallback-word caller-q\n");
    // A caller reject over the fallback's replacement.
    let (code, _) = write(&["--replace", "duskfield=caller-d", "--reject", "pinecrest"]);
    assert_eq!(code, Some(1));
    assert!(
        !text_of(&store, "site").contains("pinecrest"),
        "the refused write left nothing"
    );
}

#[test]
fn replacements_without_a_sanitize_command_are_refused_at_load_naming_the_key() {
    let store = Store::new(Some(registered), json!({}));
    let home = store.onevcs.as_ref().expect("a home");
    let mut policy = home.write_policy();
    policy["replacements"] = replaced();
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(store.sandbox.project().join("onetaskgraph.yaml"))
            .expect("the project document"),
    )
    .expect("JSON");
    document["write_policy"] = policy;
    store.sandbox.project_document(&document.to_string());
    let output = store.run(&["task", "list"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("write_policy.sanitize_command"),
        "{}",
        stderr(&output)
    );
    // And `--replace` on a store with no sanitize command at all.
    let plain = Store::new(Some(registered), json!({}));
    let output = plain.run(&["task", "list", "--replace", "a=b"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("write_policy.sanitize_command"));
    assert!(files_under(&plain, "site").is_empty());
}

/// A linking caller's policy written before this store could sanitise: it implements only the
/// two methods that existed then, over the released onevcs, and records every input it checks.
struct OldOnly {
    inner: super::LinkedOnevcs,
    checked: std::sync::Mutex<Vec<onetaskgraph_core::boundary::PublicWriteInput>>,
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for OldOnly {
    async fn repository_visibility(
        &self,
        repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        self.inner.repository_visibility(repository).await
    }

    async fn check_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        self.checked.lock().expect("the record").push(input.clone());
        self.inner.check_public_write(input).await
    }
}

fn old_only(store: &Store) -> std::sync::Arc<OldOnly> {
    std::sync::Arc::new(OldOnly {
        inner: super::LinkedOnevcs {
            home: store
                .onevcs
                .as_ref()
                .expect("an onevcs home")
                .path()
                .to_path_buf(),
        },
        checked: std::sync::Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn an_old_only_linking_policy_answers_as_before_with_one_check_per_write_holding_every_locator()
 {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    let policy = old_only(&store);
    let engine = super::linked_engine(&store).with_write_policy(policy.clone());
    let created = engine
        .create_task(&super::task_on_site(PLAIN, &[]))
        .await
        .expect("a term-free create is written as before");
    let id = created.task.id.clone();
    assert_eq!(policy.checked.lock().expect("the record").len(), 1);
    engine
        .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
        .await
        .expect("a term-free narrow write is written as before");
    let checked = policy.checked.lock().expect("the record").clone();
    assert_eq!(checked.len(), 2, "one check per write: {checked:?}");
    // Every locator the write's diagnostics would print is in the one check's input.
    assert!(
        checked[0].metadata.contains(&"site".to_owned()),
        "{checked:?}"
    );
    assert!(
        checked[1].metadata.contains(&"site".to_owned())
            && checked[1].metadata.contains(&id.to_string()),
        "{checked:?}"
    );
    assert!(engine.take_sanitization().is_empty());
}

#[tokio::test]
async fn an_old_only_linking_policy_refuses_an_item_whose_id_carries_a_term_redacting_every_locator()
 {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    store.record(
        "site",
        "tasks",
        &format!("{TERM}-note"),
        "title: Note\nstatus: todo\nproject: p",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let id: onetaskgraph_core::GlobalId = format!("site:{TERM}-note").parse().expect("an id");
    let engine = super::linked_engine(&store).with_write_policy(old_only(&store));
    let refused = engine
        .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
        .await
        .expect_err("the id carries a term");
    let said = refused.to_string();
    assert_eq!(super::kind_of(&refused), "boundary-refused", "{said}");
    assert!(
        said.contains("<redacted item 2>") && said.contains("<redacted source 1>"),
        "{said}"
    );
    assert!(!said.contains(TERM), "{said}");
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");

    // A check that cannot be asked redacts every locator too.
    let unreachable =
        super::linked_engine(&store).with_write_policy(std::sync::Arc::new(super::Unreachable));
    let refused = unreachable
        .set_task_priority(&id, onetaskgraph_plugin_api::Priority::High)
        .await
        .expect_err("an unreachable check never passes");
    let said = refused.to_string();
    assert_eq!(super::kind_of(&refused), "boundary-unavailable", "{said}");
    assert!(
        said.contains("<redacted item 2>") && !said.contains(TERM),
        "{said}"
    );
}

/// A linking caller's policy that sanitises through the released onevcs itself, counting calls.
struct Sanitizing {
    home: std::path::PathBuf,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for Sanitizing {
    async fn repository_visibility(
        &self,
        _repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        Ok(onetaskgraph_core::boundary::RepositoryVisibility::Unknown)
    }

    async fn check_public_write(
        &self,
        _input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        panic!("a sanitising policy is never asked to check");
    }

    async fn sanitize_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicSanitizeInput,
    ) -> Result<
        onetaskgraph_core::boundary::SanitizeVerdict,
        onetaskgraph_core::boundary::PolicyError,
    > {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let output = std::process::Command::new(crate::onevcs::onevcs())
            .args(["boundary", "sanitize", "--input", "-"])
            .env("ONEVCS_HOME", &self.home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child
                    .stdin
                    .take()
                    .expect("stdin")
                    .write_all(&serde_json::to_vec(input).expect("serializes"))?;
                child.wait_with_output()
            })
            .expect("onevcs runs");
        Ok(onetaskgraph_core::boundary::sanitize_verdict(
            input,
            output.status.code(),
            &output.stdout,
        ))
    }
}

#[tokio::test]
async fn a_linking_caller_s_sanitising_policy_writes_what_its_one_call_answered() {
    let store = Store::new(Some(registered), json!({}));
    store.public_project_p();
    let policy = std::sync::Arc::new(Sanitizing {
        home: store.onevcs.as_ref().expect("a home").path().to_path_buf(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let engine = super::linked_engine(&store)
        .with_write_policy(policy.clone())
        .with_replacements(vec![onetaskgraph_core::boundary::Replacement::Replace {
            term: TERM.to_owned(),
            with: WITH.to_owned(),
        }]);
    let mut task = super::task_on_site(&format!("Retry as {TERM} does."), &[]);
    task.title = format!("Port {TERM}");
    let created = engine.create_task(&task).await.expect("written replaced");
    assert_eq!(policy.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let shown = engine
        .task(&created.task.id)
        .await
        .expect("the task reads back");
    let item = &shown.items[0].item;
    assert_eq!(item.title, format!("Port {WITH}"));
    assert_eq!(
        item.content.as_deref(),
        Some(&*format!("Retry as {WITH} does."))
    );
    assert_eq!(engine.take_sanitization().replaced, 2);
    assert!(!text_of(&store, "site").contains(TERM));
}

/// One comment in a folder's own comments section, as an older store wrote it.
fn comment(id: &str, said: &str) -> String {
    format!(
        "<!-- onetaskgraph:comment id=\"{id}\" author=\"ada\" created_at=\"2026-09-13T15:11:07Z\" \
         updated_at=\"2026-09-13T15:11:07Z\" -->\n### ada — 2026-09-13T15:11:07Z\n\n{said}\n\n\
         <!-- /onetaskgraph:comment -->"
    )
}

/// Stage the task `older` on `site` as a store before replacement could have written it: the
/// term in its body, title, a label, a metadata value and two comments.
fn stage_older(store: &Store) {
    store.record(
        "site",
        "tasks",
        "older",
        &format!(
            "title: Port the {TERM} retry\nstatus: todo\nproject: p\nlabels: [{TERM}-ops]\n\
             metadata:\n  myapp.note: seen in {TERM}"
        ),
        &format!(
            "Retry the way {TERM} does.\n\n## Comments\n\n{}\n\n{}",
            comment("20260913T151107Z-1", &format!("Asked {TERM} first.")),
            comment("20260913T151107Z-2", &format!("And {TERM} again."))
        ),
    );
}

const SIDE_EFFECTS: [&str; 6] = [
    "content",
    "title",
    "label",
    "metadata:myapp.note",
    "comment:20260913T151107Z-1",
    "comment:20260913T151107Z-2",
];

#[test]
fn a_narrow_write_to_an_item_already_carrying_a_term_sanitises_it_first_and_says_so() {
    let (store, record) = sanitizing(replaced());
    stage_older(&store);
    let before = record.calls().len();
    let answer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "priority",
        "set",
        "site:older",
        "high",
        "--json",
    ]))
    .expect("the answer");
    // One call screened the write with the task and its comments as they stood.
    assert_eq!(ran(&record, before, BoundaryCommand::Sanitize), 1);
    assert_eq!(answer["priority"], json!("high"), "{answer}");
    let fields: Vec<String> = answer["sanitization"]["side_effect"]
        .as_array()
        .expect("side effects")
        .iter()
        .map(|effect| {
            assert_eq!(effect["item"], json!("site:older"), "{answer}");
            effect["field"].as_str().expect("a field").to_owned()
        })
        .collect();
    let mut expected: Vec<String> = SIDE_EFFECTS
        .iter()
        .map(|field| (*field).to_owned())
        .collect();
    expected.sort();
    let mut fields_sorted = fields.clone();
    fields_sorted.sort();
    assert_eq!(fields_sorted, expected, "{answer}");
    assert_eq!(answer["sanitization"]["dry_run"], json!(false));
    let public = text_of(&store, "site");
    assert!(!public.contains(TERM), "{public}");
    for written in [
        format!("Retry the way {WITH} does."),
        format!("Port the {WITH} retry"),
        format!("{WITH}-ops"),
        format!("seen in {WITH}"),
        format!("Asked {WITH} first."),
        format!("And {WITH} again."),
    ] {
        assert!(public.contains(&written), "{written:?}:\n{public}");
    }
    let shown: Value =
        serde_json::from_str(&store.ok(&["task", "show", "site:older", "--json"])).expect("a task");
    assert_eq!(shown["items"][0]["item"]["priority"], json!("high"));
}

#[test]
fn a_sanitising_write_s_dry_run_writes_nothing_and_says_what_it_would_change() {
    let (store, _record) = sanitizing(replaced());
    stage_older(&store);
    let before = tree(&store.folder("site"));
    let answer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "priority",
        "set",
        "site:older",
        "high",
        "--json",
        "--sanitize-dry-run",
    ]))
    .expect("the answer");
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
    assert_eq!(answer["written"], json!(false), "{answer}");
    assert_eq!(answer["would_pass"], json!(true), "{answer}");
    assert_eq!(answer["sanitization"]["dry_run"], json!(true), "{answer}");
    assert_eq!(
        answer["sanitization"]["side_effect"]
            .as_array()
            .expect("side effects")
            .len(),
        SIDE_EFFECTS.len(),
        "{answer}"
    );
    assert!(!answer.to_string().contains(TERM));
}

#[test]
fn a_sanitising_write_s_human_answer_is_today_s_line_then_one_sanitised_line_per_field() {
    let (store, _record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "clean",
        "title: Clean\nstatus: todo\nproject: p",
        PLAIN,
    );
    let today = store.ok(&["task", "priority", "set", "site:clean", "high"]);
    stage_older(&store);
    let said = store.ok(&["task", "priority", "set", "site:older", "high"]);
    let lines: Vec<&str> = said.trim_end().lines().collect();
    let success: Vec<String> = today
        .trim_end()
        .lines()
        .map(|line| line.replace("site:clean", "site:older"))
        .collect();
    assert_eq!(&lines[..success.len()], success.as_slice(), "{said}");
    let mut rest: Vec<&str> = lines[success.len()..].to_vec();
    rest.sort_unstable();
    let mut expected: Vec<String> = SIDE_EFFECTS
        .iter()
        .map(|field| format!("sanitised site:older {field}"))
        .collect();
    expected.sort();
    assert_eq!(rest, expected, "{said}");
    assert!(!said.contains(TERM));
}

#[test]
fn an_item_whose_stored_repositories_already_name_a_private_repository_is_refused_saying_so() {
    let (store, record) = sanitizing(replaced());
    store.record(
        "site",
        "tasks",
        "leaked",
        "title: Leaked\nstatus: todo\nproject: p\nrepositories: [github.com/hiddenco/quietharbor]",
        PLAIN,
    );
    let before = tree(&store.folder("site"));
    let calls = record.calls().len();
    let output = store.run(&["task", "priority", "set", "site:leaked", "high", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], json!("not-private-destination"));
    assert_eq!(failure["predates_write"], json!(true), "{failure}");
    let said = stderr(&output);
    for words in [
        "is private, and source site is not declared private",
        "already named a private repository before this write",
        "replacement cannot fix `repositories`",
        "remove that repository from the item's `repositories`",
        "move the item to a source declared private",
    ] {
        assert!(said.contains(words), "{words:?}: {said}");
    }
    assert!(!said.contains(TERM) && !said.contains("hiddenco"), "{said}");
    assert_eq!(tree(&store.folder("site")), before, "nothing was written");
    // No sanitisation was attempted: the one call screened the refusal's locators alone.
    let after = &record.calls()[calls..];
    assert!(
        after
            .iter()
            .filter(|call| call.command == BoundaryCommand::Sanitize)
            .all(|call| call.input["text"].as_array().is_none_or(Vec::is_empty)),
        "{after:?}"
    );

    // A write that itself adds a private repository is refused as before, with no clause.
    store.record(
        "plan",
        "tasks",
        "adds",
        "title: Adds\nstatus: todo\nrepositories: [github.com/hiddenco/quietharbor]",
        PLAIN,
    );
    let output = store.run(&["task", "copy", "plan:adds", "--to", "site", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], json!("not-private-destination"));
    assert!(failure.get("predates_write").is_none(), "{failure}");
    assert!(!stderr(&output).contains("before this write"));
}
