//! The default status mapping, over a real folder through the real plugin.
//!
//! The subject is the *vocabulary* rather than a list of words: the categories are
//! enumerated from [`StatusCategory`]'s own schema, which is derived from the type, so a
//! category added later with no word in the default mapping fails here instead of quietly
//! reading back as unknown — which is how `in-progress` itself came to be missing.

use std::fs;

use onetaskgraph_plugin_api::{
    NativeId, SecretResolver, SourceName, SourcePlugin, Status, StatusCategory, TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

fn folder(statuses: &[(String, String)]) -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks")).expect("the task folder");
    for (native, word) in statuses {
        fs::write(
            root.path().join(format!("tasks/{native}.md")),
            format!("---\ntitle: {native}\nstatus: {word}\n---\nbody\n"),
        )
        .expect("a task file");
    }
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &json!({"root": root.path()}),
            &NoSecrets,
        )
        .expect("the folder builds");
    (root, source)
}

/// Every normalized category, as the contract type itself spells it.
///
/// Read off `schema_for!(StatusCategory)` rather than written out, because a written list
/// is a second copy of the vocabulary that a new variant leaves stale — and a stale copy
/// here would pass while the mapping it is meant to hold to account had a hole in it.
fn every_category() -> Vec<(String, StatusCategory)> {
    let schema = schemars::schema_for!(StatusCategory);
    let value: &Value = schema.as_value();
    let words: Vec<String> = match (value.get("enum"), value.get("oneOf")) {
        (Some(Value::Array(words)), _) => words
            .iter()
            .map(|word| word.as_str().expect("a variant is a string").to_owned())
            .collect(),
        (_, Some(Value::Array(branches))) => branches
            .iter()
            .map(|branch| {
                branch["const"]
                    .as_str()
                    .expect("a unit variant is a constant string")
                    .to_owned()
            })
            .collect(),
        _ => panic!("StatusCategory's schema enumerates its variants: {value}"),
    };
    assert!(
        words.len() >= 8,
        "the vocabulary lost variants rather than gained them: {words:?}"
    );
    words
        .into_iter()
        .map(|word| {
            let category = serde_json::from_value(Value::String(word.clone()))
                .expect("a variant word reads back as its category");
            (word, category)
        })
        .collect()
}

fn stem(word: &str) -> String {
    word.replace([' ', '-'], "_")
}

async fn status_of(source: &dyn TaskSource, native: &str) -> Status {
    source
        .get_task(&NativeId(native.to_owned()))
        .await
        .expect("the folder answers")
        .expect("the task is held")
        .status
}

#[tokio::test]
async fn every_normalized_category_word_reads_back_as_itself() {
    let categories = every_category();
    let files: Vec<(String, String)> = categories
        .iter()
        .map(|(word, _)| (stem(word), word.clone()))
        .collect();
    let (_root, source) = folder(&files);

    for (word, category) in &categories {
        assert_eq!(
            status_of(source.as_ref(), &stem(word)).await,
            Status {
                category: *category,
                name: word.clone(),
            },
            "a task whose status is the canonical word {word:?} must read back as that \
             category, and keep the word it wrote; add it to `default_statuses`"
        );
    }
}

#[tokio::test]
async fn the_display_aliases_for_work_in_progress_and_cancellation_still_classify() {
    let (_root, source) = folder(&[
        ("spaced".to_owned(), "in progress".to_owned()),
        ("doing".to_owned(), "doing".to_owned()),
        ("shouted".to_owned(), "In Progress".to_owned()),
        ("american".to_owned(), "canceled".to_owned()),
    ]);

    for (native, word) in [
        ("spaced", "in progress"),
        ("doing", "doing"),
        ("shouted", "In Progress"),
    ] {
        assert_eq!(
            status_of(source.as_ref(), native).await,
            Status {
                category: StatusCategory::InProgress,
                name: word.to_owned(),
            }
        );
    }
    assert_eq!(
        status_of(source.as_ref(), "american").await,
        Status {
            category: StatusCategory::Cancelled,
            name: "canceled".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_word_the_vocabulary_cannot_place_still_reads_as_unknown_keeping_its_word() {
    let (_root, source) = folder(&[("odd".to_owned(), "percolating".to_owned())]);
    assert_eq!(
        status_of(source.as_ref(), "odd").await,
        Status {
            category: StatusCategory::Unknown,
            name: "percolating".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_replaced_mapping_is_the_whole_mapping_and_the_canonical_words_go_with_it() {
    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks")).expect("the task folder");
    fs::write(
        root.path().join("tasks/a.md"),
        "---\ntitle: A\nstatus: in-progress\n---\nbody\n",
    )
    .expect("a task file");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &json!({"root": root.path(), "status_mapping": {"active": "in-progress"}}),
            &NoSecrets,
        )
        .expect("the folder builds");
    assert_eq!(
        status_of(source.as_ref(), "a").await,
        Status {
            category: StatusCategory::Unknown,
            name: "in-progress".to_owned(),
        },
        "`status_mapping` replaces the default outright; it does not add to it"
    );
}
