//! The one rule deciding what a source's `metadata_fields` entry shows of an item's metadata.
//!
//! A `github-projects` source writes the value onto a board text field and a `linear` source
//! into a collapsed `Fields` section of the item's description; both read the value at an
//! entry's `key` and `path` through [`ProjectedValue::read`], and refuse a configuration entry
//! through [`ProjectedValue::entry_refusal`], so the two sources cannot accept different
//! shapes. How each one renders a value is its own. `docs/metadata.md` states the rule for a
//! caller.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::MetadataKey;

/// What an item's metadata holds at one projected key and path, when it holds a value a
/// projection can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectedValue {
    /// A string, the empty string included.
    Text(String),
    /// An array whose every element is a string, in array order.
    List(Vec<String>),
    /// An object whose every value is a string, in ascending byte order of its keys whatever
    /// order the JSON was written in.
    Map(BTreeMap<String, String>),
}

impl ProjectedValue {
    /// The value `metadata` holds at `key` and then each object key of `path` in turn:
    /// `Ok(None)` for nothing there or `null`, and otherwise the value, or — for any other
    /// shape — `Err` with what was found, as a refusal spells it: `a boolean`, `a number`, or
    /// for a mixed array or object `an array whose element 2 is a number` or
    /// `an object whose value at key "b" is an array`.
    ///
    /// # Errors
    ///
    /// The shape found, when it is none a projection can show.
    pub fn read(
        metadata: &BTreeMap<String, Value>,
        key: &str,
        path: &[String],
    ) -> Result<Option<Self>, String> {
        let mut value = metadata.get(key);
        for step in path {
            value = value.and_then(|held| held.get(step.as_str()));
        }
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(Self::Text(text.clone()))),
            Some(Value::Array(elements)) => elements
                .iter()
                .enumerate()
                .map(|(index, element)| match element {
                    Value::String(text) => Ok(text.clone()),
                    other => Err(format!(
                        "an array whose element {index} is {}",
                        type_name(other)
                    )),
                })
                .collect::<Result<_, _>>()
                .map(|list| Some(Self::List(list))),
            // `Map` iterates in key order, so the first offending key is the first in byte order.
            Some(Value::Object(entries)) => entries
                .iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .map(|(name, held)| match held {
                    Value::String(text) => Ok((name.clone(), text.clone())),
                    other => Err(format!(
                        "an object whose value at key {name:?} is {}",
                        type_name(other)
                    )),
                })
                .collect::<Result<_, _>>()
                .map(|map| Some(Self::Map(map))),
            Some(other) => Err(type_name(other).to_owned()),
        }
    }

    /// Whether the value shows nothing: the empty string, the empty array or the empty object.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::List(list) => list.is_empty(),
            Self::Map(map) => map.is_empty(),
        }
    }

    /// Why one `metadata_fields` entry naming `field` and `key` cannot stand beside the
    /// `earlier` fields of the same source, compared ignoring ASCII case — or `None` when it
    /// can: a blank field, a blank key, a key under [`MetadataKey::RESERVED_NAMESPACE`], or a
    /// field an earlier entry names, which the refusal calls a `noun` (`board field`, say). A
    /// source refusing field names of its own checks them after this.
    #[must_use]
    pub fn entry_refusal<'a>(
        field: &str,
        key: &str,
        earlier: impl IntoIterator<Item = &'a str>,
        noun: &str,
    ) -> Option<String> {
        if field.trim().is_empty() {
            return Some("names a blank field".to_owned());
        }
        if key.trim().is_empty() {
            return Some("names a blank key".to_owned());
        }
        if key.split('.').next() == Some(MetadataKey::RESERVED_NAMESPACE) {
            return Some(format!(
                "names a key under the reserved \"{}.\" prefix, which this product keeps for \
                 its own keys; next: project a caller key instead",
                MetadataKey::RESERVED_NAMESPACE
            ));
        }
        earlier
            .into_iter()
            .position(|other| other.eq_ignore_ascii_case(field))
            .map(|at| {
                format!(
                    "names the same {noun} as metadata_fields[{at}]; one field cannot hold two \
                     values"
                )
            })
    }
}

/// A JSON value's type, with its article, as a refusal names it.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectedValue;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn at(value: serde_json::Value) -> Result<Option<ProjectedValue>, String> {
        let metadata = BTreeMap::from([("acme.record".to_owned(), json!({"inner": value}))]);
        ProjectedValue::read(&metadata, "acme.record", &["inner".to_owned()])
    }

    #[test]
    fn every_accepted_shape_reads_and_everything_else_is_named() {
        assert_eq!(at(json!(null)), Ok(None));
        assert_eq!(at(json!("")), Ok(Some(ProjectedValue::Text(String::new()))));
        assert_eq!(
            at(json!(["b", "a"])),
            Ok(Some(ProjectedValue::List(vec!["b".into(), "a".into()])))
        );
        let Ok(Some(ProjectedValue::Map(map))) = at(json!({"b": "2", "a": "1"})) else {
            panic!("an object of strings reads as a map");
        };
        assert_eq!(map.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(at(json!(true)), Err("a boolean".into()));
        assert_eq!(at(json!(1)), Err("a number".into()));
        assert_eq!(
            at(json!(["a", ["b"]])),
            Err("an array whose element 1 is an array".into())
        );
        assert_eq!(
            at(json!({"z": "1", "b": 2, "c": {}})),
            Err("an object whose value at key \"b\" is a number".into())
        );
    }

    #[test]
    fn an_entry_is_refused_for_each_reason_and_otherwise_kept() {
        assert!(ProjectedValue::entry_refusal(" ", "a.b", [], "field").is_some());
        assert!(ProjectedValue::entry_refusal("F", " ", [], "field").is_some());
        assert!(ProjectedValue::entry_refusal("F", "onetaskgraph.x", [], "field").is_some());
        assert_eq!(
            ProjectedValue::entry_refusal("host", "a.b", ["Other", "HOST"], "field").as_deref(),
            Some("names the same field as metadata_fields[1]; one field cannot hold two values")
        );
        assert_eq!(
            ProjectedValue::entry_refusal("F", "a.b", ["G"], "field"),
            None
        );
    }
}
