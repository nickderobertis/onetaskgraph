//! The guarded, plugin-specific `sources status-options` operation.
//!
//! Keeping this orchestration outside the CLI preserves the repository's project boundary:
//! the binary wraps commands, while this independently selectable project owns the one command
//! that constructs and invokes the GitHub Projects adapter directly.

use onetaskgraph_core::{Failure, Loaded, PluginKind};
use onetaskgraph_github_projects::{GitHubProjectsConfig, GitHubProjectsSource};
use onetaskgraph_plugin_api::SourceName;

pub use onetaskgraph_github_projects::{
    StatusOptionsMode, StatusOptionsOutcome, StatusOptionsReport,
};

/// Plan or apply the guarded Status-option additions for one configured source.
pub async fn reconcile(
    loaded: &Loaded,
    name: &SourceName,
    mode: StatusOptionsMode,
) -> Result<StatusOptionsReport, Failure> {
    let configured = loaded.config.sources().get(name).ok_or_else(|| {
        Failure::decided(
            "status-options",
            format!("no configured source is named {name}"),
        )
    })?;
    if configured.plugin() != PluginKind::GithubProjects {
        return Err(Failure::decided(
            "status-options",
            format!(
                "source {name} uses plugin {}, not github-projects; status-options is only available for github-projects sources",
                configured.plugin()
            ),
        ));
    }
    let config: GitHubProjectsConfig = serde_json::from_value(configured.config().clone())
        .map_err(|error| Failure::decided("status-options", format!("source {name}: {error}")))?;
    GitHubProjectsSource::new(name, config, &loaded.secrets)
        .map_err(|error| Failure::decided("status-options", format!("source {name}: {error}")))?
        .status_options(mode)
        .await
        .map_err(|error| Failure::decided("status-options", format!("source {name}: {error}")))
}
