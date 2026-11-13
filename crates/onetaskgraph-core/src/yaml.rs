//! Every YAML read the engine makes, through `serde-saphyr`: a configuration, an answers
//! file, a template's front matter and a typed `--var` value.
//!
//! serde-saphyr's own behaviour with three options: `yes`, `no`, `on` and `off` stay strings,
//! as the local Markdown source writes them plain and every document this engine reads was
//! written to be read that way; `<<` is an ordinary key; and a refusal is one line, which
//! every message here names its file or its variable beside. The local Markdown source reads
//! with the same three, in its own crate, because a plugin never depends on the engine;
//! `tests/yaml.rs` reads one document through both and fails the moment they part.

use serde::de::DeserializeOwned;

fn options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        strict_booleans: true,
        merge_keys: serde_saphyr::MergeKeyPolicy::AsOrdinary,
        with_snippet: false,
    }
}

pub(crate) fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, serde_saphyr::Error> {
    serde_saphyr::from_str_with_options(text, options())
}
