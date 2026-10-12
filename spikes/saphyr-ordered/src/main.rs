//! SPIKE (serde-yaml-replacement, not for landing): read each front matter the harness hands
//! over through serde-saphyr into `serde_json::Value` with `preserve_order`, and answer the
//! same projection `saphyr-diff`'s `trees::project_json` makes.
//!
//! stdin: `{"profile": "default"|"compat", "cases": [[name, text], ...]}`.
//! stdout: `{"preserve_order": bool, "results": [{"case", "ok": [lines]} | {"case", "err"}]}`.

use std::io::Read;

use serde_json::{Value, json};

/// The harness's `compat` deserializer options. Defined once in onetaskgraph-local-md's
/// `yaml_backend::compat_de`, which this crate cannot depend on without joining the root's
/// build; `saphyr-diff` refuses a result whose nodes differ from its own reading of the same
/// text under that definition, so a drift here cannot pass silently.
fn compat() -> serde_saphyr::Options {
    serde_saphyr::options! {
        strict_booleans: true,
        merge_keys: serde_saphyr::MergeKeyPolicy::AsOrdinary,
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins,
        with_snippet: false,
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => format!("bool({b})"),
        Value::Number(n) if n.is_f64() => format!("float({n})"),
        Value::Number(n) => format!("int({n})"),
        Value::String(s) => format!("str({s:?})"),
        Value::Array(_) => "seq".to_owned(),
        Value::Object(_) => "map".to_owned(),
    }
}

fn walk(value: &Value, path: &str, key: &str, out: &mut Vec<String>) {
    out.push(format!("{path} key:{key} = {}", scalar(value)));
    match value {
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk(item, &format!("{path}[{i}]"), "-", out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                walk(v, &format!("{path}.{k}"), "str", out);
            }
        }
        _ => {}
    }
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("stdin");
    let request: Value = serde_json::from_str(&input).expect("a request");
    let compat_profile = request["profile"] == "compat";
    // The feature is really on in this build: a map keeps the order it was written in.
    let probe: Value = serde_json::from_str(r#"{"b": 1, "a": 2}"#).expect("json");
    let preserve_order = probe.as_object().expect("map").keys().next().map(String::as_str) == Some("b");
    let mut results = Vec::new();
    for case in request["cases"].as_array().expect("cases") {
        let name = case[0].as_str().expect("name");
        let text = case[1].as_str().expect("text");
        let parsed = if compat_profile {
            serde_saphyr::from_str_with_options::<Value>(text, compat())
        } else {
            serde_saphyr::from_str::<Value>(text)
        };
        results.push(match parsed {
            Ok(v) => {
                let mut lines = Vec::new();
                walk(&v, "$", "-", &mut lines);
                json!({"case": name, "ok": lines})
            }
            Err(e) => json!({"case": name, "err": e.to_string()}),
        });
    }
    println!("{}", json!({"preserve_order": preserve_order, "results": results}));
}
