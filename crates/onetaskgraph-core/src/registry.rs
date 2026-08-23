//! The compile-time registry of plugin kinds.
//!
//! Every plugin this binary can build is named here, whether or not its source is
//! implemented yet. That is deliberate: a configuration naming `linear` gets the
//! plugin's own "not implemented yet" message rather than "unknown plugin", and
//! landing the real source is an additive change to that one crate with no edit
//! here.

use onetaskgraph_plugin_api::SourcePlugin;

/// Every plugin kind this build knows, in a stable order.
#[must_use]
pub fn registry() -> Vec<Box<dyn SourcePlugin>> {
    vec![
        Box::new(onetaskgraph_github_projects::Plugin),
        Box::new(onetaskgraph_in_memory::Plugin),
        Box::new(onetaskgraph_linear::Plugin),
        Box::new(onetaskgraph_local_md::Plugin),
    ]
}

/// The kind names in [`registry`], for help text and error messages.
#[must_use]
pub fn plugin_kinds() -> Vec<&'static str> {
    registry().iter().map(|plugin| plugin.kind()).collect()
}

/// The plugin registered for `kind`, or `None` when nothing answers to that name.
#[must_use]
pub fn plugin_for(kind: &str) -> Option<Box<dyn SourcePlugin>> {
    registry().into_iter().find(|plugin| plugin.kind() == kind)
}
