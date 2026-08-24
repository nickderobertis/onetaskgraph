//! The compile-time registry of plugin kinds.
//!
//! Every plugin this binary can build is named here, whether or not its source is
//! implemented yet. That is deliberate: a configuration naming `linear` gets the
//! plugin's own "not implemented yet" message rather than "unknown plugin", and
//! landing the real source is an additive change to that one crate with no edit
//! here.

use std::fmt;

use onetaskgraph_plugin_api::SourcePlugin;

/// One of the plugin kinds this build has.
///
/// A [`SourceConfig`](crate::SourceConfig) holds one of these rather than the string a
/// document spelled, so a configuration naming a plugin nothing answers to cannot exist
/// past [`Config::from_document`](crate::Config::from_document). Resolution therefore has
/// no "what if the registry does not have it" branch left to get wrong, and the refusal
/// happens at the one place that can name the offending key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PluginKind {
    /// GitHub Projects.
    GithubProjects,
    /// The in-memory source the journeys are written against.
    InMemory,
    /// Linear.
    Linear,
    /// A folder of Markdown files.
    LocalMd,
    /// A program of its own, speaking `docs/plugin-protocol.md` over stdio.
    Subprocess,
}

impl PluginKind {
    /// Every kind, in the stable order [`registry`] reports them in.
    pub const ALL: [Self; 5] = [
        Self::GithubProjects,
        Self::InMemory,
        Self::Linear,
        Self::LocalMd,
        Self::Subprocess,
    ];

    /// The name a configuration document's `plugin:` field names this kind by.
    ///
    /// Spelled here rather than read from the plugin so that matching a name costs no
    /// allocation. `every_plugin_kind_names_the_kind_its_own_plugin_reports` is what
    /// keeps the two from drifting.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GithubProjects => "github-projects",
            Self::InMemory => "in-memory",
            Self::Linear => "linear",
            Self::LocalMd => "local-md",
            Self::Subprocess => "subprocess",
        }
    }

    /// The kind called `name`, or `None` when nothing in this build answers to it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    /// This kind's factory.
    ///
    /// Total, which is the point of the type: a `PluginKind` is one of exactly five
    /// things, so there is no absent-plugin case for a caller to handle or forget.
    #[must_use]
    pub fn plugin(self) -> Box<dyn SourcePlugin> {
        match self {
            Self::GithubProjects => Box::new(onetaskgraph_github_projects::Plugin),
            Self::InMemory => Box::new(onetaskgraph_in_memory::Plugin),
            Self::Linear => Box::new(onetaskgraph_linear::Plugin),
            Self::LocalMd => Box::new(onetaskgraph_local_md::Plugin),
            Self::Subprocess => Box::new(crate::subprocess::SubprocessPlugin),
        }
    }
}

impl fmt::Display for PluginKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every plugin kind this build knows, in a stable order.
#[must_use]
pub fn registry() -> Vec<Box<dyn SourcePlugin>> {
    PluginKind::ALL.map(PluginKind::plugin).into()
}

/// The kind names in [`registry`], for help text and error messages.
#[must_use]
pub fn plugin_kinds() -> Vec<&'static str> {
    registry().iter().map(|plugin| plugin.kind()).collect()
}

/// The plugin registered for `kind`, or `None` when nothing answers to that name.
#[must_use]
pub fn plugin_for(kind: &str) -> Option<Box<dyn SourcePlugin>> {
    PluginKind::parse(kind).map(PluginKind::plugin)
}
