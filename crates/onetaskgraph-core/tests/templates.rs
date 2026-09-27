//! Task templates through the crate-root API, with no binary in sight.
//!
//! What only a library caller can reach is proven here: templates registered in-process as
//! `(name, source)` pairs, `Answers` built and overlaid in code, and the typed
//! `TemplateError` a Rust caller branches on. Everything a command line can reach is proven
//! again through the binary, in `crates/onetaskgraph/tests/e2e/templates.rs`.

use std::path::Path;

use onetaskgraph_core::{
    Answers, ChainField, DECLARATION_KEYS, FRONT_MATTER_KEYS, ItemType, TemplateError,
    TemplateLoader, VariableType,
};
use serde_json::json;
use sha2::{Digest as _, Sha256};

/// Write `files` into a fresh directory and return it.
fn directory(files: &[(&str, &str)]) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a temporary directory");
    for (name, source) in files {
        let path = directory.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the parent exists");
        std::fs::write(path, source).expect("the file is written");
    }
    directory
}

/// The digest C1 defines, computed here independently of the engine.
fn expected_digest(files: &[(&str, &str)]) -> String {
    let mut hasher = Sha256::new();
    for (name, source) in files {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    let hash = hasher.finalize();
    format!(
        "sha256:{}",
        hash.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

const BASE: &str = "---\n\
onetaskgraph_template: 1\n\
description: the shape every task shares\n\
variables:\n  \
  title:\n    \
    description: The task's title\n  \
  criteria:\n    \
    description: What has to be true when it is done\n    \
    type: list\n  \
  owner:\n    \
    description: Who owns it\n    \
    default: nobody\n\
---\n\
# {{ title }}\n\
{% block body %}{% endblock %}\n\
{% include \"criteria.md\" %}\n";

const CRITERIA: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  criteria:\n    \
    description: Acceptance criteria, one per entry\n    \
    type: list\n\
---\n\
{% import \"macros.md\" as m %}\n\
## Acceptance criteria\n\
{% for item in criteria %}\n\
{{ m.bullet(item) }}\n\
{% endfor %}\n";

const MACROS: &str = "{% macro bullet(text) %}- {{ text }}{% endmacro %}\n";

const TASK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  title:\n    \
    description: What this task is called\n  \
  estimate:\n    \
    description: Days of work\n    \
    type: integer\n    \
    required: false\n\
---\n\
{% extends \"base.md\" %}\n\
{% block body %}\n\
Owner: {{ owner }}; estimate: {{ 'none' if estimate is none else estimate }}\n\
{% endblock %}\n";

fn answers() -> Answers {
    let mut answers = Answers::new();
    answers
        .set("title", json!("Ship it"))
        .set("criteria", json!(["it builds", "it ships"]));
    answers
}

const RENDERED: &str = "# Ship it\n\
Owner: nobody; estimate: none\n\
## Acceptance criteria\n\
- it builds\n\
- it ships\n";

#[test]
fn a_chain_found_through_a_search_path_directory_renders_and_merges_its_declarations() {
    let files = [
        ("task.md", TASK),
        ("base.md", BASE),
        ("criteria.md", CRITERIA),
        ("macros.md", MACROS),
    ];
    let tree = directory(&files);
    let template = TemplateLoader::new()
        .with_directory(tree.path())
        .load_name("task.md")
        .expect("the chain loads");

    assert_eq!(
        template.chain().collect::<Vec<_>>(),
        ["task.md", "base.md", "criteria.md", "macros.md"],
        "first-load order is depth first, in the order each file names the next"
    );
    let declared: Vec<(&str, VariableType, bool, &str, &str)> = template
        .variables()
        .iter()
        .map(|variable| {
            (
                variable.name(),
                variable.kind(),
                variable.required(),
                variable.description(),
                variable.declared_in(),
            )
        })
        .collect();
    assert_eq!(
        declared,
        [
            (
                "title",
                VariableType::String,
                true,
                "What this task is called",
                "task.md"
            ),
            (
                "estimate",
                VariableType::Integer,
                false,
                "Days of work",
                "task.md"
            ),
            (
                "criteria",
                VariableType::List,
                true,
                "What has to be true when it is done",
                "base.md"
            ),
            (
                "owner",
                VariableType::String,
                false,
                "Who owns it",
                "base.md"
            ),
        ],
        "the nearer file's description wins, and each variable sits where it was first declared"
    );

    let rendered = template.render(&answers()).expect("it renders");
    assert_eq!(rendered.body, RENDERED);
    assert_eq!(rendered.digest, expected_digest(&files));
    assert_eq!(template.digest(), rendered.digest);
    assert_eq!(
        serde_json::to_value(&rendered.answers).expect("answers serialise"),
        json!({
            "title": "Ship it",
            "criteria": ["it builds", "it ships"],
            "owner": "nobody",
            "estimate": null,
        })
    );
}

#[test]
fn a_chain_of_registered_pairs_renders_with_no_directory_at_all() {
    let loader = TemplateLoader::new()
        .with_template("task.md", TASK)
        .with_template("base.md", BASE)
        .with_template("criteria.md", CRITERIA)
        .with_template("macros.md", MACROS);
    let template = loader.load_name("task.md").expect("the chain loads");

    let rendered = template.render(&answers()).expect("it renders");
    assert_eq!(rendered.body, RENDERED);
    assert_eq!(
        rendered.digest,
        expected_digest(&[
            ("task.md", TASK),
            ("base.md", BASE),
            ("criteria.md", CRITERIA),
            ("macros.md", MACROS),
        ])
    );
}

#[test]
fn a_directory_is_searched_before_a_registered_pair_of_the_same_name() {
    let tree = directory(&[(
        "macros.md",
        "{% macro bullet(text) %}* {{ text }}{% endmacro %}\n",
    )]);
    let template = TemplateLoader::new()
        .with_directory(tree.path())
        .with_template("task.md", TASK)
        .with_template("base.md", BASE)
        .with_template("criteria.md", CRITERIA)
        .with_template("macros.md", MACROS)
        .load_name("task.md")
        .expect("the chain loads");
    let body = template.render(&answers()).expect("it renders").body;
    assert!(body.contains("* it builds"), "{body}");
}

#[test]
fn the_working_directory_is_never_searched_unless_it_is_given() {
    let error = TemplateLoader::new()
        .load_name("Cargo.toml")
        .expect_err("nothing is on the search path");
    assert!(matches!(error, TemplateError::NotFound { .. }), "{error}");

    let error = TemplateLoader::new()
        .with_template("root.md", "{% include \"Cargo.toml\" %}")
        .load_name("root.md")
        .expect_err("an include resolves over the search path alone");
    assert!(
        error
            .to_string()
            .contains("\"Cargo.toml\" was not found (named by root.md)"),
        "{error}"
    );
}

#[test]
fn a_redeclaration_that_changes_type_or_items_is_refused_naming_both_files() {
    let loader = TemplateLoader::new()
        .with_template(
            "child.md",
            "---\nonetaskgraph_template: 1\nvariables:\n  criteria:\n    description: x\n    type: text\n---\n{% extends \"base.md\" %}",
        )
        .with_template("base.md", BASE)
        .with_template("criteria.md", CRITERIA)
        .with_template("macros.md", MACROS);
    let error = loader.load_name("child.md").expect_err("type changed");
    let message = error.to_string();
    assert!(
        matches!(&error, TemplateError::ChainConflict { variable, field: ChainField::Type, nearer, farther, .. }
            if variable == "criteria" && nearer == "child.md" && farther == "base.md"),
        "{message}"
    );
    assert!(
        message.contains("child.md") && message.contains("base.md"),
        "{message}"
    );

    let loader = TemplateLoader::new()
        .with_template(
            "child.md",
            "---\nonetaskgraph_template: 1\nvariables:\n  criteria:\n    description: x\n    type: list\n    items: object\n---\n{% extends \"base.md\" %}",
        )
        .with_template("base.md", BASE)
        .with_template("criteria.md", CRITERIA)
        .with_template("macros.md", MACROS);
    let error = loader.load_name("child.md").expect_err("items changed");
    assert!(
        matches!(
            &error,
            TemplateError::ChainConflict {
                field: ChainField::Items,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn front_matter_is_refused_by_key() {
    let cases = [
        (
            "---\nonetaskgraph_template: 1\ncolour: blue\n---\n",
            "front matter key `colour`",
        ),
        (
            "---\ndescription: x\n---\n",
            "front matter key `onetaskgraph_template`",
        ),
        (
            "---\nonetaskgraph_template: 2\n---\n",
            "front matter key `onetaskgraph_template`",
        ),
        (
            "---\nonetaskgraph_template: 1\nvariables:\n  title: {type: string}\n---\n",
            "front matter key `variables.title.description`",
        ),
        (
            "---\nonetaskgraph_template: 1\nvariables:\n  title: {description: ''}\n---\n",
            "front matter key `variables.title.description`",
        ),
        (
            "---\nonetaskgraph_template: 1\nvariables:\n  title: {description: x, required: true, default: y}\n---\n",
            "front matter key `variables.title.required`",
        ),
        (
            "---\nonetaskgraph_template: 1\nvariables:\n  title: {description: x, shape: y}\n---\n",
            "front matter key `variables.title.shape`",
        ),
        (
            "---\nonetaskgraph_template: 1\nvariables:\n  Title: {description: x}\n---\n",
            "front matter key `variables.Title`",
        ),
        (
            "---\nonetaskgraph_template: 1\nvariables:\n  n: {description: x, type: integer, default: many}\n---\n",
            "front matter key `variables.n.default`",
        ),
        (
            "---\nonetaskgraph_template: 1\nvariables:\n  n: {description: x, items: string}\n---\n",
            "front matter key `variables.n.items`",
        ),
        (
            "---\nonetaskgraph_template: 1\n",
            "no later line reading `---`",
        ),
        ("{% if %}", "its body does not parse"),
    ];
    for (source, expected) in cases {
        let error = TemplateLoader::new()
            .with_template("t.md", source)
            .load_name("t.md")
            .expect_err(source);
        assert!(
            matches!(error, TemplateError::Malformed { .. }),
            "{source}: {error}"
        );
        let message = error.to_string();
        assert!(message.contains(expected), "{source}: {message}");
        assert!(message.contains("template t.md"), "{source}: {message}");
    }
}

#[test]
fn a_file_without_front_matter_declares_nothing_and_renders_as_it_is() {
    let template = TemplateLoader::new()
        .with_template("plain.md", "just text\n{{ 1 + 1 }}\n")
        .load_name("plain.md")
        .expect("it loads");
    assert!(template.variables().is_empty());
    assert_eq!(
        template.render(&Answers::new()).expect("renders").body,
        "just text\n2\n"
    );
}

#[test]
fn full_minijinja_uses_of_a_variable_render_as_minijinja_renders_them() {
    let source = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  name: {description: a name}\n  \
  items: {description: things, type: list}\n  \
  flag: {description: a switch, type: boolean}\n  \
  unused: {description: never read, type: text}\n  \
  config: {description: settings, type: object}\n\
---\n\
{{ name | upper }} and {{ name }} again, {{ name | length }} letters\n\
{% for item in items %}\n\
{{ loop.index }}. {{ item | title }}{% if not loop.last %},{% endif %}\n\
{% endfor %}\n\
{% if flag %}\n\
on for {{ name }}\n\
{% else %}\n\
off\n\
{% endif %}\n\
{% set shout = name ~ '!' %}{{ shout }} {{ config.level * 2 }} {{ config | tojson }}\n";
    let template = TemplateLoader::new()
        .with_template("t.md", source)
        .load_name("t.md")
        .expect("it loads");
    let mut answers = Answers::new();
    answers
        .set("name", json!("ada"))
        .set("items", json!(["one", "two"]))
        .set("flag", json!(true))
        .set("unused", json!("never\nread"))
        .set("config", json!({"level": 3}));
    assert_eq!(
        template.render(&answers).expect("it renders").body,
        // `trim_blocks` eats the newline after each block tag, so the loop and the
        // conditional run on into one line — which is what minijinja renders.
        "ADA and ada again, 3 letters\n1. One,2. Twoon for ada\nada! 6 {\"level\":3}\n"
    );
}

#[test]
fn an_undeclared_name_fails_the_render_naming_the_name_and_the_file() {
    let loader = TemplateLoader::new()
        .with_template(
            "root.md",
            "---\nonetaskgraph_template: 1\nvariables:\n  title: {description: x}\n---\n{{ title }}\n{% include \"part.md\" %}\n",
        )
        .with_template("part.md", "line one\n{{ mystery }}\n");
    let template = loader.load_name("root.md").expect("it loads");
    let mut answers = Answers::new();
    answers.set("title", json!("t"));
    let error = template.render(&answers).expect_err("mystery is undefined");
    assert!(
        matches!(&error, TemplateError::Render { file: Some(file), name: Some(name), line: Some(2), .. }
            if file == "part.md" && name == "mystery"),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("part.md") && message.contains("mystery"),
        "{message}"
    );

    // A line number counts the front matter, so it is the file's own.
    let template = TemplateLoader::new()
        .with_template(
            "root.md",
            "---\nonetaskgraph_template: 1\n---\nfine\n{{ absent }}\n",
        )
        .load_name("root.md")
        .expect("it loads");
    let error = template
        .render(&Answers::new())
        .expect_err("absent is undefined");
    assert!(
        matches!(&error, TemplateError::Render { line: Some(5), .. }),
        "{error:?}"
    );
}

#[test]
fn the_digest_is_stable_and_moves_when_any_chain_file_changes() {
    let files = [
        ("task.md", TASK),
        ("base.md", BASE),
        ("criteria.md", CRITERIA),
        ("macros.md", MACROS),
    ];
    let tree = directory(&files);
    let load = || {
        TemplateLoader::new()
            .with_directory(tree.path())
            .load_path(&tree.path().join("task.md"))
            .expect("it loads")
            .digest()
            .to_owned()
    };
    let first = load();
    assert_eq!(first, load());
    assert_eq!(first, expected_digest(&files));

    std::fs::write(tree.path().join("macros.md"), MACROS.replace("- ", "+ ")).expect("rewritten");
    let changed = load();
    assert_ne!(
        first, changed,
        "the deepest chain file changing moves the digest"
    );

    // Front matter alone is part of it too.
    std::fs::write(tree.path().join("macros.md"), MACROS).expect("restored");
    assert_eq!(load(), first);
    std::fs::write(
        tree.path().join("base.md"),
        BASE.replace("the shape every task shares", "a different description"),
    )
    .expect("rewritten");
    assert_ne!(load(), first);
}

fn one_of_each() -> onetaskgraph_core::Template {
    TemplateLoader::new()
        .with_template(
            "t.md",
            "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  title: {description: a title}\n  \
  body: {description: a body, type: text}\n  \
  count: {description: a count, type: integer}\n  \
  tags: {description: tags, type: list, default: [a]}\n  \
  note: {description: a note, required: false}\n\
---\n\
{{ title }}|{{ body }}|{{ count }}|{{ tags | join(',') }}|{{ note if note is not none else '<none>' }}\n",
        )
        .load_name("t.md")
        .expect("it loads")
}

#[test]
fn answers_overlay_later_over_earlier_and_a_named_key_can_be_unset() {
    let template = one_of_each();
    let file =
        Answers::from_yaml("title: From file\nbody: |\n  two\n  lines\ncount: 3\nnote: kept\n")
            .expect("a mapping");
    let mut flags = Answers::new();
    flags
        .set_text("count", "7")
        .set_text("title", "From flag")
        .unset("note");

    let resolved = file.overlay(&flags);
    let rendered = template.render(&resolved).expect("it renders");
    assert_eq!(rendered.body, "From flag|two\nlines\n|7|a|<none>\n");

    // The overlay replaced, it did not merge: the earlier answers are untouched.
    assert_eq!(
        template.render(&file).expect("renders").body,
        "From file|two\nlines\n|3|a|kept\n"
    );
    assert_eq!(
        template
            .unanswered(&resolved)
            .expect("typed")
            .iter()
            .map(|variable| variable.name())
            .collect::<Vec<_>>(),
        ["tags", "note"]
    );
}

#[test]
fn every_answer_refusal_is_typed_and_names_what_it_refuses() {
    let template = one_of_each();

    let mut undeclared = Answers::new();
    undeclared
        .set("zeta", json!(1))
        .set("alpha", json!(2))
        .set("title", json!("t"));
    let error = template.render(&undeclared).expect_err("undeclared");
    assert_eq!(
        error,
        TemplateError::UnknownAnswer {
            names: vec!["alpha".to_owned(), "zeta".to_owned()]
        }
    );
    assert!(error.refuses_answers());

    let mut mistyped = Answers::new();
    mistyped.set_text("count", "three");
    let error = template.render(&mistyped).expect_err("mistyped");
    assert!(
        matches!(&error, TemplateError::MistypedAnswer { name, kind: VariableType::Integer, .. } if name == "count"),
        "{error:?}"
    );

    let mut multi_line = Answers::new();
    multi_line.set("title", json!("one\ntwo"));
    let error = template
        .render(&multi_line)
        .expect_err("a string is one line");
    assert!(error.to_string().contains("one line"), "{error}");

    let mut wrong_items = Answers::new();
    wrong_items.set("tags", json!(["a", 2]));
    let error = template
        .render(&wrong_items)
        .expect_err("items are strings");
    assert!(error.to_string().contains("entry 1"), "{error}");

    let error = template
        .render(&Answers::new())
        .expect_err("nothing answered");
    assert_eq!(
        error,
        TemplateError::MissingRequired {
            names: vec!["title".to_owned(), "body".to_owned(), "count".to_owned()]
        },
        "every unanswered required variable, in one refusal, in declaration order"
    );
    assert!(error.refuses_answers());

    let error = Answers::from_yaml("- not\n- a mapping\n").expect_err("a list");
    assert!(
        matches!(error, TemplateError::MalformedAnswers { .. }),
        "{error}"
    );
    assert!(
        Answers::from_yaml("")
            .expect("empty")
            .names()
            .next()
            .is_none()
    );
}

#[test]
fn a_template_is_found_by_path_under_its_file_name() {
    let tree = directory(&[(
        "nested/one.md",
        "---\nonetaskgraph_template: 1\n---\nhello\n",
    )]);
    let template = TemplateLoader::new()
        .load_path(&tree.path().join("nested/one.md"))
        .expect("it loads");
    assert_eq!(template.name(), "one.md");
    let described = serde_json::to_value(template.describe()).expect("serialises");
    assert_eq!(described["template"], "one.md");
    assert!(
        described["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 71 && digest.starts_with("sha256:"))
    );
    assert_eq!(
        TemplateLoader::new()
            .load_path(Path::new("/definitely/not/here.md"))
            .expect_err("missing")
            .kind(),
        "template-not-found"
    );
}

#[test]
fn a_list_include_loads_its_first_candidate_that_resolves_and_only_that_one() {
    let later = "---\nonetaskgraph_template: 1\nvariables:\n  from_later: {description: x}\n---\nlater {{ from_later }}\n";
    let loader = TemplateLoader::new()
        .with_template(
            "root.md",
            "{% include [\"absent.md\", \"later.md\", \"last.md\"] %}",
        )
        .with_template("later.md", later)
        .with_template("last.md", "never read");
    let template = loader.load_name("root.md").expect("the chain loads");
    assert_eq!(
        template.chain().collect::<Vec<_>>(),
        ["root.md", "later.md"]
    );
    assert_eq!(
        template
            .variables()
            .iter()
            .map(|variable| variable.name())
            .collect::<Vec<_>>(),
        ["from_later"],
        "the candidate the render loads is the one whose front matter counts"
    );
    let mut answers = Answers::new();
    answers.set("from_later", json!("yes"));
    assert_eq!(
        template.render(&answers).expect("renders").body,
        "later yes\n"
    );

    let error = TemplateLoader::new()
        .with_template("root.md", "{% include [\"a.md\", \"b.md\"] %}")
        .load_name("root.md")
        .expect_err("no candidate resolves");
    assert!(
        error
            .to_string()
            .contains("\"a.md or b.md\" was not found (named by root.md)"),
        "{error}"
    );

    let template = TemplateLoader::new()
        .with_template(
            "root.md",
            "[{% include [\"a.md\", \"b.md\"] ignore missing %}]",
        )
        .load_name("root.md")
        .expect("`ignore missing` makes a missing list no error");
    assert_eq!(
        template.render(&Answers::new()).expect("renders").body,
        "[]"
    );
}

#[test]
fn a_template_named_by_an_expression_is_loaded_as_the_render_reaches_it() {
    let root = "---\nonetaskgraph_template: 1\nvariables:\n  kind: {description: which part}\n---\n{% include kind ~ \".md\" %}";
    let loader = TemplateLoader::new()
        .with_template("root.md", root)
        .with_template("plain.md", "a plain part\n")
        .with_template(
            "declaring.md",
            "---\nonetaskgraph_template: 1\nvariables:\n  more: {description: y}\n---\nx\n",
        );
    let template = loader.load_name("root.md").expect("the chain loads");
    assert_eq!(
        template.chain().collect::<Vec<_>>(),
        ["root.md"],
        "an expression names nothing before rendering"
    );

    let mut answers = Answers::new();
    answers.set("kind", json!("plain"));
    let rendered = template.render(&answers).expect("renders");
    assert_eq!(rendered.body, "a plain part\n");
    // What the render read is in its digest: the chain, then the file the expression named.
    assert_eq!(template.digest(), expected_digest(&[("root.md", root)]));
    assert_eq!(
        rendered.digest,
        expected_digest(&[("root.md", root), ("plain.md", "a plain part\n")])
    );

    answers.set("kind", json!("declaring"));
    let error = template
        .render(&answers)
        .expect_err("its declarations came too late");
    assert!(
        matches!(&error, TemplateError::Render { message, .. }
            if message.contains("declaring.md is named by an expression and declares variables")),
        "{error:?}"
    );

    answers.set("kind", json!("nowhere"));
    let error = template.render(&answers).expect_err("nothing resolves it");
    assert!(matches!(error, TemplateError::Render { .. }), "{error:?}");
    assert!(error.to_string().contains("nowhere.md"), "{error}");
}

#[test]
fn search_path_directories_are_searched_in_the_order_given() {
    let first = directory(&[("part.md", "from the first\n")]);
    let second = directory(&[
        ("part.md", "from the second\n"),
        ("only.md", "only in the second\n"),
    ]);
    let root = "{% include \"part.md\" %}{% include \"only.md\" %}";
    let render = |loader: TemplateLoader| {
        loader
            .with_template("root.md", root)
            .load_name("root.md")
            .expect("the chain loads")
            .render(&Answers::new())
            .expect("renders")
            .body
    };
    assert_eq!(
        render(
            TemplateLoader::new()
                .with_directory(first.path())
                .with_directory(second.path())
        ),
        "from the first\nonly in the second\n"
    );
    assert_eq!(
        render(
            TemplateLoader::new()
                .with_directory(second.path())
                .with_directory(first.path())
        ),
        "from the second\nonly in the second\n"
    );
}

#[test]
fn boolean_and_object_list_answers_are_typed_as_declared() {
    let template = TemplateLoader::new()
        .with_template(
            "t.md",
            "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  urgent: {description: whether it is urgent, type: boolean}\n  \
  people: {description: who, type: list, items: object}\n\
---\n\
{% if urgent %}URGENT {% endif %}{% for person in people %}{{ person.name }}{% if not loop.last %}, {% endif %}{% endfor %}\n",
        )
        .load_name("t.md")
        .expect("it loads");

    let mut answers = Answers::new();
    answers
        .set_text("urgent", "true")
        .set("people", json!([{"name": "ada"}, {"name": "bo"}]));
    assert_eq!(
        template.render(&answers).expect("renders").body,
        "URGENT ada, bo"
    );
    answers.set("urgent", json!(false));
    assert_eq!(template.render(&answers).expect("renders").body, "ada, bo");

    let mut refused = answers.clone();
    refused.set_text("urgent", "maybe");
    let error = template.render(&refused).expect_err("not a boolean");
    assert!(
        matches!(&error, TemplateError::MistypedAnswer { name, kind: VariableType::Boolean, .. } if name == "urgent"),
        "{error:?}"
    );

    let mut refused = answers.clone();
    refused.set("people", json!([{"name": "ada"}, "bo"]));
    let error = template
        .render(&refused)
        .expect_err("an entry is not a mapping");
    assert!(
        matches!(&error, TemplateError::MistypedAnswer { name, kind: VariableType::List, problem, .. }
            if name == "people" && problem.contains("entry 1") && problem.contains("objects")),
        "{error:?}"
    );
    assert_eq!(template.variables()[1].items(), Some(ItemType::Object));
}

/// The README restates the front matter's keys and vocabularies for a reader; this holds that
/// statement to the parser both ways, so neither can move without the other.
#[test]
fn the_readme_front_matter_is_the_one_the_parser_reads() {
    let readme =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"))
            .expect("the README is readable");
    let section = readme
        .split_once("### Task templates")
        .expect("the README has a task templates section")
        .1;
    let block = section
        .split_once("```jinja\n")
        .and_then(|(_, rest)| rest.split_once("```"))
        .expect("the section opens with a template")
        .0;
    let matter = block
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .expect("the template opens with front matter")
        .0;

    let key = |line: &str| line.trim_start().split(':').next().unwrap_or("").to_owned();
    let mut top: Vec<String> = matter
        .lines()
        .filter(|line| !line.starts_with(' '))
        .map(key)
        .collect();
    let mut declaration: Vec<String> = matter
        .lines()
        .filter(|line| line.starts_with("    ") && !line.starts_with("     "))
        .map(key)
        .collect();
    top.sort();
    declaration.sort();
    let mut expected_top: Vec<String> = FRONT_MATTER_KEYS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    let mut expected_declaration: Vec<String> = DECLARATION_KEYS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    expected_top.sort();
    expected_declaration.sort();
    assert_eq!(
        top, expected_top,
        "the README's top-level front matter keys"
    );
    assert_eq!(
        declaration, expected_declaration,
        "the README's declaration keys"
    );

    let vocabulary = |field: &str, after: &str| -> Vec<String> {
        let line = matter
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{field}:")))
            .unwrap_or_else(|| panic!("the README declares `{field}`"));
        let comment = line.split_once(after).expect("a commented vocabulary").1;
        comment
            .split(';')
            .next()
            .expect("a vocabulary")
            .split('|')
            .map(|word| word.trim().to_owned())
            .collect()
    };
    assert_eq!(
        vocabulary("type", "# "),
        VariableType::ALL.map(|kind| kind.as_str().to_owned()),
        "the README's types"
    );
    assert_eq!(
        vocabulary("items", "list only: "),
        ItemType::ALL.map(|items| items.as_str().to_owned()),
        "the README's item types"
    );
}

#[test]
fn a_rendered_digest_moves_when_a_file_an_expression_named_changes() {
    let tree = directory(&[("part.md", "one\n")]);
    let root = "---\nonetaskgraph_template: 1\nvariables:\n  kind: {description: which}\n---\n{% include kind ~ \".md\" %}";
    let render = || {
        let template = TemplateLoader::new()
            .with_directory(tree.path())
            .with_template("root.md", root)
            .load_name("root.md")
            .expect("it loads");
        let mut answers = Answers::new();
        answers.set("kind", json!("part"));
        let rendered = template.render(&answers).expect("renders");
        (template.digest().to_owned(), rendered.digest)
    };
    let (chain, first) = render();
    assert_ne!(
        chain, first,
        "the render read a file the chain does not name"
    );
    assert_eq!(render().1, first, "stable across runs");
    std::fs::write(tree.path().join("part.md"), "two\n").expect("rewritten");
    let (unchanged, moved) = render();
    assert_eq!(unchanged, chain, "the chain itself did not change");
    assert_ne!(moved, first, "the file the render read did");
}
