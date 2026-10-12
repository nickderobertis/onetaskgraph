//! Which serde-saphyr option, if any, reproduces serde_norway on each difference the
//! comparison found that the compat profile does not already close. Each input is read (or
//! written) by serde_norway and by serde-saphyr under the compat profile and under the compat
//! profile with one more option changed, and every variant whose result equals serde_norway's
//! is named. A difference no variant reproduces says so.

use std::collections::BTreeMap;
use std::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use onetaskgraph_local_md::yaml_backend::{compat_de, compat_ser};

/// A typed record with one field, refusing others, as local-md's front matter is read.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Record {
    title: String,
}

type Map = BTreeMap<String, Value>;

fn de_variants() -> Vec<(&'static str, serde_saphyr::Options)> {
    let base = compat_de;
    let with = |f: fn(&mut serde_saphyr::Options)| {
        let mut o = base();
        f(&mut o);
        o
    };
    vec![
        ("compat", base()),
        ("compat + duplicate_keys: Error", with(|o| o.duplicate_keys = serde_saphyr::DuplicateKeyPolicy::Error)),
        ("compat + duplicate_keys: FirstWins", with(|o| o.duplicate_keys = serde_saphyr::DuplicateKeyPolicy::FirstWins)),
        ("compat + legacy_octal_numbers", with(|o| o.legacy_octal_numbers = true)),
        ("compat + reject_non_finite_typeless_float: false", with(|o| o.reject_non_finite_typeless_float = false)),
        ("compat + ignore_binary_tag_for_string", with(|o| o.ignore_binary_tag_for_string = true)),
        ("compat + no_schema", with(|o| o.no_schema = true)),
        ("compat + reject_unsupported_tags", with(|o| o.reject_unsupported_tags = true)),
    ]
}

fn read_case<T: DeserializeOwned + Debug>(out: &mut String, name: &str, text: &str) {
    let norway = serde_norway::from_str::<T>(text).map(|v| format!("{v:?}")).map_err(|e| e.to_string());
    out.push_str(&format!("  read {name}: {text:?}\n    serde_norway: {}\n", shown(&norway)));
    let mut matched = Vec::new();
    for (label, options) in de_variants() {
        let saphyr = serde_saphyr::from_str_with_options::<T>(text, options)
            .map(|v| format!("{v:?}"))
            .map_err(|e| e.to_string());
        if same(&norway, &saphyr) {
            matched.push(label);
        } else {
            out.push_str(&format!("    {label}: {}\n", shown(&saphyr)));
        }
    }
    out.push_str(&verdict(&matched));
}

fn ser_variants() -> Vec<(&'static str, serde_saphyr::SerializerOptions)> {
    let base = compat_ser;
    let with = |f: fn(&mut serde_saphyr::SerializerOptions)| {
        let mut o = base();
        f(&mut o);
        o
    };
    vec![
        ("compat", base()),
        ("compat + yaml_12", with(|o| o.yaml_12 = true)),
        ("compat + quote_all", with(|o| o.quote_all = true)),
        ("compat + prefer_block_scalars: false", with(|o| o.prefer_block_scalars = false)),
    ]
}

fn write_case<T: Serialize>(out: &mut String, name: &str, value: &T) {
    let norway = serde_norway::to_string(value).map_err(|e| e.to_string());
    out.push_str(&format!("  write {name}\n    serde_norway: {}\n", shown(&norway)));
    let mut matched = Vec::new();
    for (label, options) in ser_variants() {
        let saphyr = serde_saphyr::to_string_with_options(value, options).map_err(|e| e.to_string());
        if same(&norway, &saphyr) {
            matched.push(label);
        } else {
            out.push_str(&format!("    {label}: {}\n", shown(&saphyr)));
        }
    }
    out.push_str(&verdict(&matched));
}

fn same(a: &Result<String, String>, b: &Result<String, String>) -> bool {
    match (a, b) {
        (Ok(a), Ok(b)) => a == b,
        // Two refusals agree on the outcome; their wording is compared elsewhere.
        (Err(_), Err(_)) => true,
        _ => false,
    }
}

fn shown(r: &Result<String, String>) -> String {
    match r {
        Ok(s) => format!("{s:?}"),
        Err(e) => format!("refused: {:?}", e.lines().next().unwrap_or_default()),
    }
}

fn verdict(matched: &[&str]) -> String {
    if matched.is_empty() {
        "    => no variant reproduces serde_norway\n".to_owned()
    } else {
        format!("    => reproduced by: {}\n", matched.join("; "))
    }
}

/// The probe, as report text.
pub fn run() -> String {
    let mut out = String::from(
        "\n=== option probe — which serde-saphyr option reproduces serde_norway on each difference compat leaves\n",
    );
    // Reads.
    read_case::<Map>(&mut out, "null key, as local-md metadata", "{null: null}\n");
    read_case::<Map>(&mut out, "tilde key, as a configuration", "~: x\n");
    read_case::<Map>(&mut out, "one metadata key spelled plain and quoted", "a.b: 1\n\"a.b\": 2\n");
    read_case::<Record>(&mut out, "a struct field repeated", "title: x\ntitle: y\n");
    read_case::<Map>(&mut out, "a map key repeated", "a: 1\na: 2\n");
    read_case::<Map>(&mut out, "octal-looking 010", "n: 010\n");
    read_case::<Map>(&mut out, ".inf", "i: .inf\n");
    read_case::<Map>(&mut out, "!!binary", "b: !!binary aGVsbG8=\n");
    read_case::<Map>(&mut out, "a local tag", "plugin: !in-memory\n");
    read_case::<Map>(&mut out, "an integer past u64", "n: 123456789012345678901234567890\n");
    read_case::<i64>(&mut out, "1_000 as an integer --var", "1_000\n");
    read_case::<Value>(&mut out, "`[bracketed] title` as a list --var", "[bracketed] title\n");
    read_case::<Map>(&mut out, "a next-line character", "value: \"a\u{85}b\"\n");
    // Writes.
    write_case(&mut out, "a string with `: `", &json!({"title": "Design: adopt the cost release"}));
    write_case(&mut out, "an empty string", &json!({"summary": ""}));
    write_case(&mut out, "a timestamp-shaped string", &json!({"approved_at": "2026-09-07T11:56:15.785012+00:00"}));
    write_case(&mut out, "a date-shaped string", &json!({"m": "2026-10-12"}));
    write_case(&mut out, "a YAML 1.1 boolean spelling", &json!({"a": "yes"}));
    write_case(&mut out, "a sexagesimal-shaped string", &json!({"t": "12:30"}));
    write_case(&mut out, "a multi-line string with a blank line", &json!({"b": "one\n\ntwo\n"}));
    write_case(&mut out, "a multi-line string opening with a newline", &json!({"b": "\nleading newline"}));
    write_case(&mut out, "a multi-line string with a whitespace-only line", &json!({"b": "one\n  \ntwo"}));
    write_case(&mut out, "a next-line character", &json!({"v": "a\u{85}b"}));
    out
}
