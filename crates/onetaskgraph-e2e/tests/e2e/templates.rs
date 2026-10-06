//! `onetaskgraph template`: a template's variables, and its rendering from answers, driven
//! through the binary the way a person or a script drives it.
//!
//! No source is configured and none is reached: a template is read from the files the
//! command names and nothing else. The prompting journey runs the binary with a real
//! pseudo-terminal as its standard input and error, which only a Unix host can open; every
//! other journey here runs everywhere.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::machine::{bundle, validates};

const TASK: &str = "---\n\
onetaskgraph_template: 1\n\
description: One task\n\
variables:\n  \
  title:\n    \
    description: What the task is called\n  \
  count:\n    \
    description: How many of it\n    \
    type: integer\n  \
  steps:\n    \
    description: What to do, in order\n    \
    type: list\n  \
  notes:\n    \
    description: Anything else\n    \
    type: text\n    \
    required: false\n\
---\n\
{% extends \"base.md\" %}\n\
{% block body %}\n\
{{ title | upper }} x{{ count }}\n\
{% include \"steps.md\" %}\n\
{% if notes %}\n\
Notes: {{ notes }}\n\
{% endif %}\n\
{% endblock %}\n";

const BASE: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  owner:\n    \
    description: Who owns it\n    \
    default: nobody\n  \
  title:\n    \
    description: The base's own word for it\n\
---\n\
# {{ title }} ({{ owner }})\n\
{% block body %}{% endblock %}\n";

const STEPS: &str = "{% import \"macros.md\" as m %}\n\
{% for step in steps %}\n\
{{ m.step(loop.index, step) }}\n\
{% endfor %}\n";

const MACROS: &str = "{% macro step(n, text) %}{{ n }}. {{ text }}{% endmacro %}\n";

/// A sandbox holding the task template, with its chain in a directory of its own.
fn chain(sandbox: &Sandbox) -> (PathBuf, PathBuf) {
    let library = sandbox.subdirectory("library");
    for (name, source) in [
        ("base.md", BASE),
        ("steps.md", STEPS),
        ("macros.md", MACROS),
    ] {
        std::fs::write(library.join(name), source).expect("a chain file");
    }
    let task = sandbox.project().join("task.md");
    std::fs::write(&task, TASK).expect("the template");
    (task, library)
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .output()
        .expect("the binary runs")
}

fn path(value: &Path) -> &str {
    value.to_str().expect("a UTF-8 path")
}

const RENDERED: &str = "# Ship it (nobody)\nSHIP IT x2\n1. build\n2. release\n";

#[test]
fn variables_lists_the_declared_set_merged_down_the_chain_with_its_digest() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);

    let output = run(
        &sandbox,
        &[
            "template",
            "variables",
            path(&task),
            "--search-path",
            path(&library),
            "--json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let described: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    validates(
        &bundle(&sandbox),
        "TemplateVariables",
        &described,
        "template variables",
    );
    assert_eq!(described["template"], "task.md");
    assert_eq!(
        described["variables"],
        json!([
            {"name": "title", "description": "What the task is called", "type": "string",
             "required": true, "declared_in": "task.md"},
            {"name": "count", "description": "How many of it", "type": "integer",
             "required": true, "declared_in": "task.md"},
            {"name": "steps", "description": "What to do, in order", "type": "list",
             "items": "string", "required": true, "declared_in": "task.md"},
            {"name": "notes", "description": "Anything else", "type": "text",
             "required": false, "declared_in": "task.md"},
            {"name": "owner", "description": "Who owns it", "type": "string",
             "required": false, "default": "nobody", "declared_in": "base.md"},
        ])
    );

    let text = run(
        &sandbox,
        &[
            "template",
            "variables",
            path(&task),
            "--search-path",
            path(&library),
        ],
    );
    assert!(text.status.success(), "{}", stderr(&text));
    let text = stdout(&text);
    let digest = described["digest"].as_str().expect("a digest");
    assert_eq!(
        text,
        format!(
            "template:  task.md\n\
             digest:    {digest}\n\
             \n\
             title  string        required          task.md  What the task is called\n\
             count  integer       required          task.md  How many of it\n\
             steps  list<string>  required          task.md  What to do, in order\n\
             notes  text          optional          task.md  Anything else\n\
             owner  string        default \"nobody\"  base.md  Who owns it\n"
        )
    );

    // A template declaring nothing says so rather than printing an empty table.
    let plain = sandbox.project().join("plain.md");
    std::fs::write(&plain, "---\nonetaskgraph_template: 1\n---\nplain\n").expect("written");
    let output = run(&sandbox, &["template", "variables", path(&plain)]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.starts_with("template:  plain.md\ndigest:    sha256:"),
        "{text}"
    );
    assert!(text.ends_with("\n\n(it declares no variables)\n"), "{text}");
}

#[test]
fn render_follows_extends_include_and_import_through_the_search_path() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);

    let output = run(
        &sandbox,
        &[
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--var",
            "title=Ship it",
            "--var",
            "count=2",
            "--var",
            "steps=[build, release]",
            "--no-interactive",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), RENDERED, "the body, byte for byte");
    assert!(output.stderr.is_empty());

    let machine = run(
        &sandbox,
        &[
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--var",
            "title=Ship it",
            "--var",
            "count=2",
            "--var",
            "steps=[build, release]",
            "--no-interactive",
            "--json",
        ],
    );
    assert!(machine.status.success(), "{}", stderr(&machine));
    let rendered: Value = serde_json::from_str(&stdout(&machine)).expect("JSON");
    validates(
        &bundle(&sandbox),
        "RenderedTemplate",
        &rendered,
        "template render",
    );
    assert_eq!(rendered["body"], RENDERED);
    assert_eq!(
        rendered["answers"],
        json!({"title": "Ship it", "count": 2, "steps": ["build", "release"],
               "notes": null, "owner": "nobody"})
    );

    // Without the directory the chain cannot be found: the template's own directory and
    // the working directory are never searched implicitly.
    let unfound = run(
        &sandbox,
        &[
            "template",
            "render",
            path(&task),
            "--var",
            "title=x",
            "--no-interactive",
        ],
    );
    assert_eq!(unfound.status.code(), Some(1));
    assert!(
        stderr(&unfound).contains("\"base.md\" was not found (named by task.md)"),
        "{}",
        stderr(&unfound)
    );
}

#[test]
fn the_digest_is_stable_across_runs_and_moves_when_any_chain_file_changes() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);
    let digest = || {
        let output = run(
            &sandbox,
            &[
                "template",
                "variables",
                path(&task),
                "--search-path",
                path(&library),
                "--json",
            ],
        );
        assert!(output.status.success(), "{}", stderr(&output));
        let described: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        described["digest"].as_str().expect("a digest").to_owned()
    };
    let first = digest();
    assert_eq!(first.len(), "sha256:".len() + 64);
    assert!(first.starts_with("sha256:"));
    assert_eq!(first, digest(), "stable across runs");

    std::fs::write(library.join("macros.md"), MACROS.replace(". ", ") ")).expect("rewritten");
    let changed = digest();
    assert_ne!(first, changed, "a file two references deep changed");
    std::fs::write(library.join("macros.md"), MACROS).expect("restored");
    assert_eq!(digest(), first);
}

#[test]
fn answers_resolve_flag_over_file_over_default() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);
    let answers = sandbox.project().join("answers.yaml");
    std::fs::write(
        &answers,
        "title: From the file\ncount: 1\nsteps:\n  - only\nowner: someone\nnotes: |\n  two\n  lines\n",
    )
    .expect("an answers file");

    let output = run(
        &sandbox,
        &[
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--answers",
            path(&answers),
            "--var",
            "count=5",
            "--no-interactive",
            "--json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let rendered: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert_eq!(
        rendered["answers"],
        json!({"title": "From the file", "count": 5, "steps": ["only"],
               "notes": "two\nlines\n", "owner": "someone"})
    );
    assert_eq!(
        rendered["body"],
        "# From the file (someone)\nFROM THE FILE x5\n1. only\nNotes: two\nlines\n\n"
    );

    // And the answers file on standard input, with a `string` --var taken literally even
    // where YAML would read something else.
    let mut command = sandbox.command();
    let output = command
        .args([
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--answers",
            "-",
            "--var",
            "title=[not a list]",
            "--no-interactive",
        ])
        .write_stdin("count: 3\nsteps: [a]\n")
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "# [not a list] (nobody)\n[NOT A LIST] x3\n1. a\n"
    );
}

#[test]
fn a_variable_given_by_several_vars_takes_the_last_one_typed() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);
    let answers = sandbox.project().join("answers.yaml");
    std::fs::write(&answers, "title: From the file\ncount: 1\n").expect("an answers file");

    let output = run(
        &sandbox,
        &[
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--answers",
            path(&answers),
            "--var",
            "title=First",
            "--var",
            "count=2",
            "--var",
            "steps=[a]",
            "--var",
            "title=Second",
            "--var",
            "count=3",
            "--no-interactive",
            "--json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let rendered: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert_eq!(rendered["answers"]["title"], "Second");
    assert_eq!(rendered["answers"]["count"], 3);
    assert_eq!(rendered["body"], "# Second (nobody)\nSECOND x3\n1. a\n");
}

/// Every C2 refusal: exit 2, the problem named on standard error, nothing rendered.
#[test]
fn every_answer_refusal_exits_two_naming_what_it_refuses() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);
    let base = [
        "template",
        "render",
        path(&task),
        "--search-path",
        path(&library),
    ];
    let malformed = sandbox.project().join("malformed.yaml");
    std::fs::write(&malformed, "- a list\n- not a mapping\n").expect("written");

    let cases: Vec<(Vec<&str>, Vec<&str>)> = vec![
        (
            vec![
                "--no-interactive",
                "--var",
                "title=t",
                "--var",
                "count=1",
                "--var",
                "steps=[a]",
                "--var",
                "colour=blue",
                "--var",
                "zeal=9",
            ],
            vec!["no variable the template declares: colour, zeal"],
        ),
        (
            vec![
                "--no-interactive",
                "--var",
                "title=t",
                "--var",
                "count=many",
                "--var",
                "steps=[a]",
            ],
            vec!["\"count\" is not an integer", "the string \"many\""],
        ),
        (
            vec![
                "--no-interactive",
                "--var",
                "title=t",
                "--var",
                "count=1",
                "--var",
                "steps=just one",
            ],
            vec!["\"steps\" is not a list"],
        ),
        (
            vec!["--no-interactive"],
            vec!["required variables are unanswered: title, count, steps"],
        ),
        (
            vec!["--no-interactive", "--var", "count"],
            vec!["'count' for '--var <NAME=VALUE>': that is not NAME=VALUE"],
        ),
        (
            vec!["--no-interactive", "--var", "=nameless", "--var", "Title=x"],
            vec!["\"\" is not a variable name"],
        ),
        (
            vec!["--no-interactive", "--answers", path(&malformed)],
            vec!["the answers document is refused", "not a mapping"],
        ),
        (
            vec!["--no-interactive", "--answers", "missing.yaml"],
            vec!["--answers missing.yaml: could not read it"],
        ),
    ];
    for (extra, expected) in cases {
        let mut arguments = base.to_vec();
        arguments.extend(&extra);
        let output = run(&sandbox, &arguments);
        let problem = stderr(&output);
        assert_eq!(output.status.code(), Some(2), "{extra:?}: {problem}");
        assert!(output.stdout.is_empty(), "{extra:?} rendered something");
        for needle in expected {
            assert!(
                problem.contains(needle),
                "{extra:?} does not say {needle:?}:\n{problem}"
            );
        }
        assert!(problem.contains("next:"), "{problem}");
    }
}

#[test]
fn an_interactive_render_without_a_terminal_is_refused_naming_the_way_out_and_never_hangs() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);

    // `interactive` is on by default, and standard input here is a closed pipe.
    let output = sandbox
        .command()
        .args([
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--var",
            "title=t",
            "--var",
            "count=1",
        ])
        .write_stdin("")
        .timeout(std::time::Duration::from_secs(30))
        .output()
        .expect("the binary runs");
    let problem = stderr(&output);
    assert_eq!(output.status.code(), Some(2), "{problem}");
    assert!(
        problem.contains("unanswered (steps, notes, owner)"),
        "{problem}"
    );
    assert!(
        problem.contains("standard input is not a terminal"),
        "{problem}"
    );
    assert!(
        problem.contains("--no-interactive") && problem.contains("--answers"),
        "{problem}"
    );

    // Everything answered, there is nothing to ask, so interactivity does not matter.
    let output = sandbox
        .command()
        .args([
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--var",
            "title=t",
            "--var",
            "count=1",
            "--var",
            "steps=[]",
            "--var",
            "notes=",
            "--var",
            "owner=o",
        ])
        .write_stdin("")
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{}", stderr(&output));

    // The setting at the document layer turns the prompt off exactly as the flag does.
    sandbox.project_document("interactive: false\n");
    let output = sandbox
        .command()
        .args([
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--var",
            "title=t",
            "--var",
            "count=1",
            "--var",
            "steps=[x]",
        ])
        .write_stdin("")
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "# t (nobody)\nT x1\n1. x\n");
}

#[test]
fn answers_on_standard_input_that_are_not_text_are_refused_with_exit_two() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);
    let output = sandbox
        .command()
        .args([
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--answers",
            "-",
            "--no-interactive",
        ])
        .write_stdin(vec![0xff, 0xfe, 0x00])
        .output()
        .expect("the binary runs");
    let problem = stderr(&output);
    assert_eq!(output.status.code(), Some(2), "{problem}");
    assert!(
        problem.contains("--answers -: could not read the answers from standard input"),
        "{problem}"
    );
    assert!(output.stdout.is_empty());
}

/// A reader that is gone, the way `onetaskgraph template render … > /dev/full` ends.
#[cfg(target_os = "linux")]
#[test]
fn a_rendered_body_that_cannot_be_written_exits_one_naming_it() {
    let sandbox = Sandbox::new();
    let (task, library) = chain(&sandbox);
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full exists on Linux");
    let output = sandbox
        .subprocess(onetaskgraph_e2e_support::binary())
        .current_dir(sandbox.project())
        .env("XDG_CONFIG_HOME", sandbox.config_home())
        .args([
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--var",
            "title=t",
            "--var",
            "count=1",
            "--var",
            "steps=[a]",
            "--no-interactive",
        ])
        .stdout(std::process::Stdio::from(full))
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("the binary runs");
    let problem = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{problem}");
    assert!(
        problem.contains("could not write the rendered template"),
        "{problem}"
    );
    assert!(!problem.contains("panicked"), "{problem}");
}

/// A template an expression names — here by a variable's value — is part of the chain: its
/// front matter joins the declared set, its answers are held to it, and its bytes to the
/// digest.
#[test]
fn a_template_an_expression_names_declares_variables_like_any_chain_file() {
    let sandbox = Sandbox::new();
    let library = sandbox.subdirectory("library");
    std::fs::write(
        library.join("detail.md"),
        "---\nonetaskgraph_template: 1\nvariables:\n  owner: {description: who owns it}\n---\nowned by {{ owner }}\n",
    )
    .expect("written");
    std::fs::write(
        library.join("retyped.md"),
        "---\nonetaskgraph_template: 1\nvariables:\n  title: {description: t, type: integer}\n---\nx\n",
    )
    .expect("written");
    let root = sandbox.project().join("root.md");
    std::fs::write(
        &root,
        "---\nonetaskgraph_template: 1\nvariables:\n  kind: {description: which part, default: detail}\n  title: {description: the title}\n---\n# {{ title }}\n{% include kind ~ \".md\" %}",
    )
    .expect("written");

    let output = run(
        &sandbox,
        &[
            "template",
            "variables",
            path(&root),
            "--search-path",
            path(&library),
            "--json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let described: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    validates(
        &bundle(&sandbox),
        "TemplateVariables",
        &described,
        "template variables",
    );
    assert_eq!(
        described["variables"][2],
        json!({"name": "owner", "description": "who owns it", "type": "string",
               "required": true, "declared_in": "detail.md"})
    );

    let render = |extra: &[&str]| {
        let mut arguments = vec![
            "template",
            "render",
            path(&root),
            "--search-path",
            path(&library),
            "--var",
            "title=Ship it",
            "--no-interactive",
            "--json",
        ];
        arguments.extend(extra);
        run(&sandbox, &arguments)
    };
    let output = render(&["--var", "owner=ada"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let rendered: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert_eq!(rendered["body"], "# Ship it\nowned by ada\n");
    assert_eq!(rendered["answers"]["owner"], "ada");
    assert_eq!(
        rendered["digest"], described["digest"],
        "the file the expression named is in both digests, in first-load order"
    );

    let output = render(&[]);
    let problem = stderr(&output);
    assert_eq!(output.status.code(), Some(2), "{problem}");
    assert!(
        problem.contains("a required variable is unanswered: owner"),
        "{problem}"
    );

    let output = render(&["--var", "kind=retyped"]);
    let problem = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{problem}");
    assert!(
        problem.contains(
            "\"title\" is declared with type string in root.md and with type integer in retyped.md"
        ),
        "{problem}"
    );
}

#[test]
fn an_undeclared_name_fails_the_render_naming_the_name_and_the_file() {
    let sandbox = Sandbox::new();
    let library = sandbox.subdirectory("library");
    std::fs::write(library.join("part.md"), "first\n{{ mystery }}\n").expect("written");
    let task = sandbox.project().join("t.md");
    std::fs::write(
        &task,
        "---\nonetaskgraph_template: 1\n---\n{% include \"part.md\" %}",
    )
    .expect("written");

    let output = run(
        &sandbox,
        &[
            "template",
            "render",
            path(&task),
            "--search-path",
            path(&library),
            "--no-interactive",
        ],
    );
    let problem = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{problem}");
    assert!(problem.contains("template part.md line 2"), "{problem}");
    assert!(problem.contains("mystery is undefined"), "{problem}");
}

#[test]
fn a_malformed_template_and_a_chain_type_conflict_are_refused_by_name() {
    let sandbox = Sandbox::new();
    let library = sandbox.subdirectory("library");
    let task = sandbox.project().join("t.md");

    std::fs::write(
        &task,
        "---\nonetaskgraph_template: 1\nvariables:\n  x: {description: y, colour: red}\n---\n",
    )
    .expect("written");
    let output = run(&sandbox, &["template", "variables", path(&task)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("front matter key `variables.x.colour`"),
        "{}",
        stderr(&output)
    );

    std::fs::write(
        library.join("base.md"),
        "---\nonetaskgraph_template: 1\nvariables:\n  x: {description: y, type: list}\n---\n",
    )
    .expect("written");
    std::fs::write(&task, "---\nonetaskgraph_template: 1\nvariables:\n  x: {description: y, type: object}\n---\n{% extends \"base.md\" %}").expect("written");
    let output = run(
        &sandbox,
        &[
            "template",
            "variables",
            path(&task),
            "--search-path",
            path(&library),
            "--json",
        ],
    );
    let problem = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{problem}");
    assert!(
        problem
            .contains("\"x\" is declared with type object in t.md and with type list in base.md"),
        "{problem}"
    );
    // Under machine output the failure is the one document a caller branches on.
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], "template-chain-conflict");
}

#[test]
fn loop_controls_and_the_urlencode_filter_render_as_minijinja_renders_them() {
    let sandbox = Sandbox::new();
    let task = sandbox.project().join("t.md");
    std::fs::write(
        &task,
        "---\nonetaskgraph_template: 1\nvariables:\n  steps: {description: s, type: list}\n  \
         query: {description: q}\n---\n\
         {% for step in steps %}{% if step == \"skip\" %}{% continue %}{% endif %}\
         {% if step == \"stop\" %}{% break %}{% endif %}{{ step }};{% endfor %}\n\
         https://example.test/?q={{ query | urlencode }}\n",
    )
    .expect("written");
    let output = run(
        &sandbox,
        &[
            "template",
            "render",
            path(&task),
            "--var",
            "steps=[build, skip, test, stop, release]",
            "--var",
            "query=a b&c",
            "--no-interactive",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        // `trim_blocks` takes the newline after `{% endfor %}`, as C1 turns it on.
        "build;test;https://example.test/?q=a%20b%26c\n"
    );
}

#[test]
fn a_search_path_that_is_not_a_directory_is_refused_naming_it() {
    let sandbox = Sandbox::new();
    let task = sandbox.project().join("t.md");
    std::fs::write(&task, "no chain\n").expect("written");
    let missing = sandbox.project().join("no-such-library");

    for search in [missing.as_path(), task.as_path()] {
        let output = run(
            &sandbox,
            &[
                "template",
                "render",
                path(&task),
                "--search-path",
                path(search),
                "--no-interactive",
                "--json",
            ],
        );
        let problem = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{problem}");
        assert!(
            problem.contains(&format!("search path {} cannot be searched", path(search))),
            "{problem}"
        );
        assert!(problem.contains("next: "), "{problem}");
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        assert_eq!(failure["failure"]["kind"], "template-search-path");
    }

    // Without the bad directory the same template renders.
    let output = run(
        &sandbox,
        &["template", "render", path(&task), "--no-interactive"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "no chain\n");
}

/// The prompting journey: a real pseudo-terminal on standard input and error.
#[cfg(unix)]
mod prompting {
    use std::ffi::OsStr;
    use std::io::{Read as _, Write as _};
    use std::os::unix::ffi::OsStrExt as _;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

    use super::{Sandbox, path, stderr};

    const PROMPTED: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  title: {description: The title}\n  \
  count: {description: How many, type: integer}\n  \
  notes: {description: Free notes, type: text}\n  \
  tags: {description: Some tags, type: list}\n  \
  meta: {description: Some settings, type: object}\n  \
  owner: {description: Who owns it, default: me}\n\
---\n\
{{ title }}|{{ count }}|{{ owner }}\n\
{{ notes }}\n\
{% for tag in tags %}\n\
- {{ tag }}\n\
{% endfor %}\n\
{{ meta.level }}/{{ meta.name }}\n";

    /// Everything the child wrote to the terminal so far, read on a thread of its own.
    struct Screen {
        text: Arc<Mutex<String>>,
        seen: usize,
    }

    impl Screen {
        /// Wait for `needle` after everything already waited for, and move past it.
        fn expect(&mut self, needle: &str) {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let text = self.text.lock().expect("the screen").clone();
                if let Some(at) = text[self.seen..].find(needle) {
                    self.seen += at + needle.len();
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "the terminal never showed {needle:?}; it shows:\n{text}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    /// A terminal to type on and nowhere a person can read the prompt: refused, not asked.
    #[test]
    fn a_terminal_on_standard_input_with_standard_error_redirected_is_refused_rather_than_asked() {
        let sandbox = Sandbox::new();
        let task = sandbox.project().join("prompted.md");
        std::fs::write(&task, PROMPTED).expect("the template");
        let (master, terminal) = pseudo_terminal();

        let mut command = Command::new(onetaskgraph_e2e_support::binary());
        for (variable, _) in std::env::vars() {
            if variable.starts_with("ONETASKGRAPH_") {
                command.env_remove(variable);
            }
        }
        let output = command
            .current_dir(sandbox.project())
            .env("XDG_CONFIG_HOME", sandbox.config_home())
            .env_remove("HOME")
            .args(["template", "render", path(&task)])
            .stdin(terminal)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("the binary runs");
        drop(master);
        let problem = stderr(&output);
        assert_eq!(output.status.code(), Some(2), "{problem}");
        assert!(
            problem.contains("standard error, where the prompts are written, is not a terminal"),
            "{problem}"
        );
        assert!(problem.contains("--no-interactive"), "{problem}");
        assert!(output.stdout.is_empty());
    }

    /// A terminal that hangs up mid-prompt: the render fails, and nothing is rendered.
    #[test]
    fn a_prompt_whose_terminal_hangs_up_fails_rather_than_rendering() {
        let sandbox = Sandbox::new();
        let task = sandbox.project().join("prompted.md");
        std::fs::write(&task, PROMPTED).expect("the template");
        let (master, terminal) = pseudo_terminal();

        let mut command = Command::new(onetaskgraph_e2e_support::binary());
        for (variable, _) in std::env::vars() {
            if variable.starts_with("ONETASKGRAPH_") {
                command.env_remove(variable);
            }
        }
        let child = command
            .current_dir(sandbox.project())
            .env("XDG_CONFIG_HOME", sandbox.config_home())
            .env_remove("HOME")
            .args(["template", "render", path(&task)])
            .stdin(terminal.try_clone().expect("a second handle"))
            .stderr(terminal)
            .stdout(Stdio::piped())
            .spawn()
            .expect("the binary starts");
        drop(command);

        // The one handle on the terminal's own side reads until the first question is on the
        // screen and is then dropped — a hang-up, as when the window a person typed in closes.
        let reader = std::thread::spawn(move || {
            let mut master = master;
            let mut screen = String::new();
            let mut buffer = [0u8; 4096];
            while !screen.contains("title (string, required): The title") {
                match master.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => screen.push_str(&String::from_utf8_lossy(&buffer[..read])),
                }
            }
            screen
        });

        let screen = reader.join().expect("the reader ends");
        assert!(screen.contains("title (string, required)"), "{screen}");
        let output = child.wait_with_output().expect("the binary exits");
        // 1, not a panic's 101: the failure is reported, to wherever it can still go.
        assert_eq!(
            output.status.code(),
            Some(1),
            "a prompt that cannot be answered fails"
        );
        assert!(
            output.stdout.is_empty(),
            "nothing is rendered from half the answers"
        );
    }

    /// A fresh pseudo-terminal: the side this test holds, and the side a child is handed.
    fn pseudo_terminal() -> (std::fs::File, std::fs::File) {
        // Close-on-exec, or a child another test spawns meanwhile inherits this side, holds it
        // open, and a hang-up here never reaches this test's own child.
        #[cfg(target_os = "linux")]
        let flags = OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = OpenptFlags::RDWR | OpenptFlags::NOCTTY;
        let master = openpt(flags).expect("a pseudo-terminal");
        #[cfg(not(target_os = "linux"))]
        rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC).expect("close-on-exec");
        grantpt(&master).expect("granted");
        unlockpt(&master).expect("unlocked");
        let name = ptsname(&master, Vec::new()).expect("its name");
        let terminal = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(OsStr::from_bytes(name.as_bytes()))
            .expect("the terminal side");
        (std::fs::File::from(master), terminal)
    }

    #[test]
    fn each_unanswered_variable_is_asked_in_order_and_a_malformed_value_is_asked_again() {
        let sandbox = Sandbox::new();
        let task = sandbox.project().join("prompted.md");
        std::fs::write(&task, PROMPTED).expect("the template");

        let (mut master, terminal) = pseudo_terminal();

        // A plain process rather than the sandbox's `assert_cmd` one, which cannot hand its
        // child a terminal: the same sandboxed environment, spelled out.
        let mut command = Command::new(onetaskgraph_e2e_support::binary());
        for (variable, _) in std::env::vars() {
            if variable.starts_with("ONETASKGRAPH_") {
                command.env_remove(variable);
            }
        }
        let child = command
            .current_dir(sandbox.project())
            .env("XDG_CONFIG_HOME", sandbox.config_home())
            .env_remove("HOME")
            .args(["template", "render", path(&task), "--var", "title=Given"])
            .stdin(terminal.try_clone().expect("a second handle"))
            .stderr(terminal)
            .stdout(Stdio::piped())
            .spawn()
            .expect("the binary starts");
        // Only the child holds the terminal side now, so the reader below ends when it does.
        drop(command);

        let text = Arc::new(Mutex::new(String::new()));
        let reader = {
            let text = Arc::clone(&text);
            let mut master = master.try_clone().expect("a reading handle");
            std::thread::spawn(move || {
                let mut buffer = [0u8; 4096];
                while let Ok(read) = master.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    text.lock()
                        .expect("the screen")
                        .push_str(&String::from_utf8_lossy(&buffer[..read]));
                }
            })
        };
        let mut screen = Screen {
            text: Arc::clone(&text),
            seen: 0,
        };
        let mut type_in = |keys: &str| {
            master.write_all(keys.as_bytes()).expect("typed");
            master.flush().expect("flushed");
        };

        screen.expect("prompted.md asks for 5 variables.");
        screen.expect("count (integer, required): How many");
        type_in("lots\r");
        screen.expect("not integer");
        type_in("4\r");

        screen.expect("notes (text, required): Free notes");
        screen.expect("then a line holding only `.`:");
        // Nothing at all, for a required variable with no default: asked again.
        type_in(".\r");
        screen.expect("notes is required; enter a value; try again.");
        screen.expect("then a line holding only `.`:");
        type_in("first line\r  second, indented\r.\r");

        screen.expect("tags (list of string, required): Some tags");
        screen.expect("then a line holding only `.`:");
        type_in("[unclosed\r.\r");
        screen.expect("not list");
        screen.expect("then a line holding only `.`:");
        type_in("- alpha\r- beta\r.\r");

        screen.expect("meta (object, required): Some settings");
        screen.expect("then a line holding only `.`:");
        type_in("- a list\r.\r");
        screen.expect("not object");
        screen.expect("then a line holding only `.`:");
        type_in("level: 3\rname: deep\r.\r");

        screen.expect("owner (string, default \"me\"): Who owns it");
        type_in("\r");

        let output = child.wait_with_output().expect("the binary exits");
        drop(master);
        reader.join().expect("the reader ends");
        let transcript = text.lock().expect("the screen").clone();
        assert!(output.status.success(), "{}\n{transcript}", stderr(&output));
        assert!(
            !transcript.contains("title ("),
            "a variable --var answered is not asked for:\n{transcript}"
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("UTF-8"),
            "Given|4|me\nfirst line\n  second, indented\n- alpha\n- beta\n3/deep\n"
        );
    }
}
