//! The schema-less walks: what each candidate tree carries of a document, as one comparable
//! projection — every node in document order, with its path, the YAML type of its key and
//! the type and value of its scalar.
//!
//! `serde_norway::Value` is what `template/front_matter.rs` walks today. The candidates are
//! `serde_json::Value` as this workspace builds it (no `preserve_order`, so a `BTreeMap`),
//! read through `serde-saphyr`, and `saphyr`'s node tree. `serde_json::Value` with
//! `preserve_order` is read by `spikes/saphyr-ordered`, a separate build, whose answer comes
//! back already in this projection (its `walk` is `walk_json` verbatim).

/// One node of a projection.
pub type Line = String;

fn line(path: &str, key: &str, value: &str) -> Line {
    format!("{path} key:{key} = {value}")
}

fn json_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::Bool(b) => format!("bool({b})"),
        serde_json::Value::Number(n) if n.is_f64() => format!("float({n})"),
        serde_json::Value::Number(n) => format!("int({n})"),
        serde_json::Value::String(s) => format!("str({s:?})"),
        serde_json::Value::Array(_) => "seq".to_owned(),
        serde_json::Value::Object(_) => "map".to_owned(),
    }
}

/// `serde_json::Value` (sorted keys: no `preserve_order`).
pub fn project_json(value: &serde_json::Value) -> Vec<Line> {
    let mut out = Vec::new();
    walk_json(value, "$", "-", &mut out);
    out
}

fn walk_json(value: &serde_json::Value, path: &str, key: &str, out: &mut Vec<Line>) {
    out.push(line(path, key, &json_scalar(value)));
    match value {
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_json(item, &format!("{path}[{i}]"), "-", out);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                walk_json(v, &format!("{path}.{k}"), "str", out);
            }
        }
        _ => {}
    }
}

/// `serde_norway::Value`: what the front matter walk reads today.
pub fn project_norway(value: &serde_norway::Value) -> Vec<Line> {
    let mut out = Vec::new();
    walk_norway(value, "$", "-", &mut out);
    out
}

fn norway_scalar(value: &serde_norway::Value) -> (String, String) {
    use serde_norway::Value as Y;
    match value {
        Y::Null => ("null".into(), "null".into()),
        Y::Bool(b) => ("bool".into(), format!("bool({b})")),
        Y::Number(n) if n.is_f64() => ("float".into(), format!("float({})", float(n.as_f64()))),
        Y::Number(n) => ("int".into(), format!("int({n})")),
        Y::String(s) => ("str".into(), format!("str({s:?})")),
        Y::Sequence(_) => ("seq".into(), "seq".into()),
        Y::Mapping(_) => ("map".into(), "map".into()),
        Y::Tagged(t) => ("tagged".into(), format!("tagged({})", t.tag)),
    }
}

fn float(f: Option<f64>) -> String {
    f.and_then(serde_json::Number::from_f64)
        .map_or_else(|| "non-finite".to_owned(), |n| n.to_string())
}

fn norway_key(key: &serde_norway::Value) -> (String, String) {
    let (kind, _) = norway_scalar(key);
    let text = match key {
        serde_norway::Value::String(s) => s.clone(),
        other => serde_norway::to_string(other)
            .map(|s| s.trim_end().to_owned())
            .unwrap_or_default(),
    };
    (kind, text)
}

fn walk_norway(value: &serde_norway::Value, path: &str, key: &str, out: &mut Vec<Line>) {
    out.push(line(path, key, &norway_scalar(value).1));
    match value {
        serde_norway::Value::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_norway(item, &format!("{path}[{i}]"), "-", out);
            }
        }
        serde_norway::Value::Mapping(map) => {
            for (k, v) in map {
                let (kind, text) = norway_key(k);
                walk_norway(v, &format!("{path}.{text}"), &kind, out);
            }
        }
        _ => {}
    }
}

/// `saphyr::Yaml`, the node tree.
pub fn project_saphyr(value: &saphyr::Yaml<'_>) -> Vec<Line> {
    let mut out = Vec::new();
    walk_saphyr(value, "$", "-", &mut out);
    out
}

fn saphyr_scalar(value: &saphyr::Yaml<'_>) -> (String, String) {
    use saphyr::{Scalar, Yaml};
    match value {
        Yaml::Value(Scalar::Null) => ("null".into(), "null".into()),
        Yaml::Value(Scalar::Boolean(b)) => ("bool".into(), format!("bool({b})")),
        Yaml::Value(Scalar::Integer(i)) => ("int".into(), format!("int({i})")),
        Yaml::Value(Scalar::FloatingPoint(f)) => {
            ("float".into(), format!("float({})", float(Some(f.into_inner()))))
        }
        Yaml::Value(Scalar::String(s)) => ("str".into(), format!("str({s:?})")),
        Yaml::Sequence(_) => ("seq".into(), "seq".into()),
        Yaml::Mapping(_) => ("map".into(), "map".into()),
        Yaml::Tagged(tag, _) => ("tagged".into(), format!("tagged({tag})")),
        other => ("other".into(), format!("{other:?}")),
    }
}

fn walk_saphyr(value: &saphyr::Yaml<'_>, path: &str, key: &str, out: &mut Vec<Line>) {
    out.push(line(path, key, &saphyr_scalar(value).1));
    match value {
        saphyr::Yaml::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_saphyr(item, &format!("{path}[{i}]"), "-", out);
            }
        }
        saphyr::Yaml::Mapping(map) => {
            for (k, v) in map {
                let (kind, _) = saphyr_scalar(k);
                let text = match k {
                    saphyr::Yaml::Value(saphyr::Scalar::String(s)) => s.to_string(),
                    saphyr::Yaml::Value(saphyr::Scalar::Integer(i)) => i.to_string(),
                    saphyr::Yaml::Value(saphyr::Scalar::Boolean(b)) => b.to_string(),
                    saphyr::Yaml::Value(saphyr::Scalar::Null) => "null".to_owned(),
                    saphyr::Yaml::Value(saphyr::Scalar::FloatingPoint(f)) => {
                        float(Some(f.into_inner()))
                    }
                    // A collection key, spelled as norway's `to_string` spells it.
                    other => saphyr_flow(other),
                };
                walk_saphyr(v, &format!("{path}.{text}"), &kind, out);
            }
        }
        _ => {}
    }
}

/// The lines of `candidate` that differ from `today`, as a diff text; empty when equal.
pub fn compare(today: &[Line], candidate: &[Line]) -> String {
    crate::report::line_diff(
        &today.iter().map(|l| format!("{l}\n")).collect::<String>(),
        &candidate.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )
}

/// A collection node as a block YAML text, the way `serde_norway::to_string` renders a
/// key for the projection.
fn saphyr_flow(value: &saphyr::Yaml<'_>) -> String {
    let mut out = String::new();
    let mut emitter = saphyr::YamlEmitter::new(&mut out);
    let _ = emitter.dump(value);
    out.trim_start_matches("---").trim().to_owned()
}
