//! The synthetic cases beside the real documents: the YAML corners the outside evaluation
//! found the two crates part on, put in each document kind's own shape.

use std::collections::BTreeMap;

use serde_json::{Value, json};

fn s(a: &str, b: &str) -> (String, String) {
    (a.to_owned(), b.to_owned())
}

/// Configurations, well-formed and malformed.
pub fn configs() -> Vec<(String, String)> {
    vec![
        s("minimal", "sources:\n  a:\n    plugin: in-memory\n"),
        s("empty document", ""),
        s("comment only", "# nothing\n"),
        s("explicit document start only", "---\n"),
        s("bad indentation", "sources:\n  a:\n    plugin: local-md\n   config: {}\n"),
        s("unclosed flow sequence", "default_sources: [a, b\nsources: {}\n"),
        s("unclosed quote", "output: \"json\nsources: {}\n"),
        s("tab indentation", "sources:\n\ta:\n\t\tplugin: in-memory\n"),
        s("duplicate key", "output: json\noutput: text\n"),
        s("duplicate source", "sources:\n  a:\n    plugin: in-memory\n  a:\n    plugin: local-md\n"),
        s("yaml 1.1 boolean as a value", "output: yes\n"),
        s("on/off", "sources:\n  a:\n    plugin: in-memory\n    config:\n      flag: on\n"),
        s("anchors and aliases", "sources:\n  a: &x\n    plugin: in-memory\n  b: *x\n"),
        s("merge key", "sources:\n  a: &x\n    plugin: in-memory\n  b:\n    <<: *x\n"),
        s("non-string key", "1: x\n"),
        s("null key", "~: x\n"),
        s("top-level sequence", "- a\n- b\n"),
        s("top-level scalar", "just text\n"),
        s("multi-document", "output: json\n---\noutput: text\n"),
        s("unknown setting", "bogus: 1\n"),
        s("unknown plugin", "sources:\n  a:\n    plugin: nope\n"),
        s("octal-looking", "sources:\n  a:\n    plugin: in-memory\n    config:\n      n: 010\n      m: 0o10\n"),
        s("hex, exponent, inf", "sources:\n  a:\n    plugin: in-memory\n    config:\n      h: 0x1F\n      e: 1e3\n      i: .inf\n      u: 1_000\n"),
        s("null spellings", "sources:\n  a:\n    plugin: in-memory\n    config:\n      a: ~\n      b: Null\n      c: NULL\n      d: null\n"),
        s("timestamp", "sources:\n  a:\n    plugin: in-memory\n    config:\n      at: 2026-10-12\n"),
        s("byte order mark", "\u{feff}sources:\n  a:\n    plugin: in-memory\n"),
        s("CRLF", "sources:\r\n  a:\r\n    plugin: in-memory\r\n"),
        s("tagged value", "sources:\n  a:\n    plugin: !custom in-memory\n"),
        s("binary tag", "sources:\n  a:\n    plugin: in-memory\n    config:\n      b: !!binary aGVsbG8=\n"),
        s("explicit str tag", "sources:\n  a:\n    plugin: !!str in-memory\n"),
        s("complex key", "? [a, b]\n: x\n"),
        s("very large integer", "sources:\n  a:\n    plugin: in-memory\n    config:\n      n: 123456789012345678901234567890\n"),
    ]
}

/// Template front matters (the text between the `---` lines).
pub fn front_matters() -> Vec<(String, String)> {
    vec![
        s("order: z before a", "onetaskgraph_template: 1\nvariables:\n  zeta: {description: z}\n  alpha: {description: a}\n  mid: {description: m}\n"),
        s("integer key under variables", "onetaskgraph_template: 1\nvariables:\n  1: {description: one}\n"),
        s("boolean key under variables", "onetaskgraph_template: 1\nvariables:\n  true: {description: t}\n"),
        s("null key under variables", "onetaskgraph_template: 1\nvariables:\n  ~: {description: n}\n"),
        s("sequence key", "onetaskgraph_template: 1\n? [a]\n: x\n"),
        s("version as float", "onetaskgraph_template: 1.0\n"),
        s("version as string", "onetaskgraph_template: '1'\n"),
        s("required: yes", "onetaskgraph_template: 1\nvariables:\n  a: {description: d, required: yes}\n"),
        s("defaults of each type", "onetaskgraph_template: 1\nvariables:\n  i: {description: d, type: integer, default: 7}\n  f: {description: d, type: object, default: {x: 1.5, y: .inf, z: 1e3}}\n  b: {description: d, type: boolean, default: false}\n  l: {description: d, type: list, default: [a, 1, null, true]}\n  t: {description: d, type: text, default: \"line\\nline\\n\"}\n  d: {description: d, default: 2026-10-12}\n  o: {description: d, type: object, default: {b: 1, a: 2}}\n"),
        s("block scalar default", "onetaskgraph_template: 1\nvariables:\n  t:\n    description: d\n    type: text\n    default: |+\n      kept\n\n"),
        s("duplicate variable", "onetaskgraph_template: 1\nvariables:\n  a: {description: one}\n  a: {description: two}\n"),
        s("anchors", "onetaskgraph_template: 1\nvariables:\n  a: &d {description: shared}\n  b: *d\n"),
        s("merge key", "onetaskgraph_template: 1\nvariables:\n  a: &d {description: shared, type: text}\n  b:\n    <<: *d\n    required: false\n"),
        s("malformed: bad indentation", "onetaskgraph_template: 1\nvariables:\n  a:\n    description: d\n   type: text\n"),
        s("malformed: unclosed flow", "onetaskgraph_template: 1\nvariables: {a: {description: d}\n"),
        s("empty", ""),
        s("yes/no/on/off as strings", "onetaskgraph_template: 1\ndescription: yes\nvariables:\n  a: {description: no}\n  b: {description: on, default: off}\n"),
    ]
}

/// Scalars: `--var` text for every typed variable, and text the title/status splice renders.
pub fn scalars() -> Vec<String> {
    [
        "42", "-7", "+1", "007", "0o17", "0x1F", "1_000", "1e3", "1.0", "1.", ".5", ".inf",
        "-.inf", ".nan", "NaN", "123456789012345678901234567890", "yes", "no", "on", "off", "y",
        "n", "true", "false", "True", "TRUE", "~", "null", "Null", "NULL", "", " ", "[a, b]",
        "[]", "{}", "{a: 1}", "{a: 1, a: 2}", "- a\n- b", "a: 1\nb: 2", "[{a: 1}, {b: 2}]",
        "'quoted'", "\"double\"", "it's", "a: b", "a:b", "#comment", "a #b", "- x", "-x", "? x",
        ": x", "*x", "&x", "!x", "!!str 5", "%x", "@x", "`x", "|", ">", "| x", "=", "<<", ",",
        "[x", "{x", "x]", "2026-10-12", "2026-10-12T10:00:00Z", "12:30", "1:2:3", " lead",
        "trail ", "tab\there", "multi\nline", "trailing newline\n", "two newlines\n\n",
        "\nleading newline", "crlf\r\nline", "unicode é ✓ 日本", "emoji 🙂", "\u{85}nel",
        "\u{2028}ls", "\u{7f}del", "\u{1b}esc", "back\\slash", "fix(scope): a title with: colons",
        "feat!: breaking", "[bracketed] title", "100%", "a, b", "&anchor-like",
        "a very long line that goes on and on past eighty columns so that a serializer that folds long scalars would fold it somewhere here",
        "in-progress", "Needs attention", "backlog", "0", "-0", "0.0", "1e-3", "0b101",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// Answers documents.
pub fn answers() -> Vec<(String, String)> {
    vec![
        s("empty", ""),
        s("null", "~\n"),
        s("sequence", "- a\n"),
        s("scalar", "x\n"),
        s("integer key", "1: x\n"),
        s("bool key", "true: x\n"),
        s("duplicate key", "a: 1\na: 2\n"),
        s("yaml 1.1 booleans", "a: yes\nb: no\nc: on\nd: off\n"),
        s("numbers", "a: 007\nb: 0o7\nc: 0x1F\nd: 1_000\ne: 1e3\nf: .inf\ng: .nan\n"),
        s("timestamp", "a: 2026-10-12\n"),
        s("block scalars", "a: |\n  one\n  two\nb: |+\n  kept\n\nc: >\n  folded\n  text\nd: |-\n  stripped\n"),
        s("anchors", "a: &x [1, 2]\nb: *x\n"),
        s("merge", "a: &x {k: 1}\nb:\n  <<: *x\n  j: 2\n"),
        s("malformed", "a: [1, 2\n"),
        s("tabs", "a:\n\t- x\n"),
        s("binary", "a: !!binary aGVsbG8=\n"),
        s("tagged", "a: !custom x\n"),
    ]
}

/// Answer maps a stored answers block or `TemplateAnswers::to_yaml` writes.
pub fn answer_maps() -> Vec<(String, BTreeMap<String, Value>)> {
    let map = |v: Value| -> BTreeMap<String, Value> {
        serde_json::from_value(v).expect("an object")
    };
    vec![
        ("strings needing quotes".into(), map(json!({"a": "yes", "b": "no", "c": "on", "d": "123", "e": "1.0", "f": "", "g": "~", "h": "null", "i": "true", "j": "a: b", "k": "#x", "l": "- x", "m": "2026-10-12", "n": "0o17", "o": "0x1F", "p": " lead", "q": "trail "}))),
        ("multi-line strings".into(), map(json!({"a": "one\ntwo", "b": "kept\n\n", "c": "ends\n", "d": "\nleading", "e": "crlf\r\nline", "f": "trailing spaces  \nx"}))),
        ("long line".into(), map(json!({"a": "a very long line that goes on and on past eighty columns so that a serializer that folds long scalars would fold it somewhere around here and then some more"}))),
        ("empty collections".into(), map(json!({"a": [], "b": {}, "c": null}))),
        ("nested".into(), map(json!({"a": [{"k": 1, "j": [1, 2]}, {"x": {"y": "z"}}], "b": {"z": 1, "a": 2}}))),
        ("numbers and bools".into(), map(json!({"a": 1, "b": -1, "c": 1.5, "d": 1e300, "e": true, "f": false, "g": 0, "h": 1.0}))),
        ("unicode and control".into(), map(json!({"a": "é ✓ 日本 🙂", "b": "tab\there", "c": "\u{85}", "d": "\u{2028}", "e": "\u{1b}"}))),
        ("keys needing quotes".into(), map(json!({"yes": 1, "1": 2, "a b": 3, "": 4, "a: b": 5, "null": 6}))),
        ("list of strings".into(), map(json!({"steps": ["one", "two: three", "- four", "five\nsix"]}))),
    ]
}

/// Metadata values a `metadata set` writes.
pub fn metadata_values() -> Vec<(String, Value)> {
    vec![
        ("string".into(), json!("plain")),
        ("yes string".into(), json!("yes")),
        ("numeric string".into(), json!("123")),
        ("integer".into(), json!(42)),
        ("float".into(), json!(1.5)),
        ("float whole".into(), json!(1.0)),
        ("bool".into(), json!(true)),
        ("null".into(), json!(null)),
        ("list".into(), json!(["a", 1, null])),
        ("object".into(), json!({"by": "review-plan", "at": "2026-09-29T15:20:40+00:00", "n": {"z": 1, "a": 2}})),
        ("multi-line string".into(), json!("one\ntwo")),
        ("unicode".into(), json!("é ✓ 🙂")),
        ("control character".into(), json!("a\u{1b}b")),
        ("line separator".into(), json!("a\u{2028}b")),
        ("next line".into(), json!("a\u{85}b")),
        ("big integer".into(), json!(u64::MAX)),
        ("negative".into(), json!(-9_007_199_254_740_993_i64)),
        ("exponent float".into(), json!(1e300)),
    ]
}

/// Malformed records: (name, directory, file text).
pub fn malformed_records() -> Vec<(String, &'static str, String)> {
    let t = |a: &str, d: &'static str, b: &str| (a.to_owned(), d, b.to_owned());
    vec![
        t("unknown front-matter field on a task", "tasks", "---\ntitle: x\nbogus: 1\n---\nbody\n"),
        t("unknown field on a document (status)", "documents", "---\ntitle: x\nstatus: todo\n---\nbody\n"),
        t("bad indentation", "tasks", "---\ntitle: x\nmetadata:\n  a: 1\n b: 2\n---\n"),
        t("unclosed quote", "tasks", "---\ntitle: \"x\n---\n"),
        t("unclosed flow", "tasks", "---\ntitle: x\nlabels: [a, b\n---\n"),
        t("tab indentation", "tasks", "---\ntitle: x\nmetadata:\n\ta: 1\n---\n"),
        t("duplicate key", "tasks", "---\ntitle: x\ntitle: y\n---\n"),
        t("wrong type: labels a number", "tasks", "---\ntitle: x\nlabels: 5\n---\n"),
        t("wrong type: title a list", "tasks", "---\ntitle: [a]\n---\n"),
        t("wrong type: status a number", "tasks", "---\ntitle: x\nstatus: 5\n---\n"),
        t("title yes (bool-looking)", "tasks", "---\ntitle: yes\n---\n"),
        t("title 1.0 (number-looking)", "tasks", "---\ntitle: 1.0\n---\n"),
        t("project as a number", "tasks", "---\ntitle: x\nproject: 123\n---\n"),
        t("priority invalid", "tasks", "---\ntitle: x\npriority: 7\n---\n"),
        t("priority null", "tasks", "---\ntitle: x\npriority: null\n---\n"),
        t("depends_on bad kind", "tasks", "---\ntitle: x\ndepends_on:\n  - {id: a, kind: nope}\n---\n"),
        t("label missing name", "tasks", "---\ntitle: x\nlabels:\n  - {id: a}\n---\n"),
        t("metadata not a map", "tasks", "---\ntitle: x\nmetadata: [1]\n---\n"),
        t("repositories bad", "tasks", "---\ntitle: x\nrepositories: [\"not a repo\"]\n---\n"),
        t("classification unknown", "tasks", "---\ntitle: x\nclassification: secret\n---\n"),
        t("anchor in metadata", "tasks", "---\ntitle: x\nmetadata:\n  a.b: &v {k: 1}\n  c.d: *v\n---\n"),
        t("merge key in metadata", "tasks", "---\ntitle: x\nmetadata:\n  a.b: &v {k: 1}\n  c.d:\n    <<: *v\n---\n"),
        t("yes in metadata", "tasks", "---\ntitle: x\nmetadata:\n  a.b: yes\n---\n"),
        t("octal in metadata", "tasks", "---\ntitle: x\nmetadata:\n  a.b: 010\n---\n"),
        t("timestamp in metadata", "tasks", "---\ntitle: x\nmetadata:\n  a.b: 2026-10-12\n---\n"),
        t("unknown field on a project", "projects", "---\ntitle: x\nowner: me\n---\n"),
        t("empty front matter", "tasks", "---\n---\nbody\n"),
        t("front matter a sequence", "tasks", "---\n- a\n---\n"),
        t("stored answers block malformed", "tasks", "---\ntitle: x\n---\nbody\n\n<!-- onetaskgraph:template-answers\na: [1\n-->\n"),
        t("stored answers block a sequence", "tasks", "---\ntitle: x\n---\nbody\n\n<!-- onetaskgraph:template-answers\n- a\n-->\n"),
        t("stored answers block integer key", "tasks", "---\ntitle: x\n---\nbody\n\n<!-- onetaskgraph:template-answers\n1: a\n-->\n"),
    ]
}
