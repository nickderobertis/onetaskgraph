//! The guarded, plugin-specific `sources status-options` operation.
//!
//! Keeping this orchestration outside the CLI preserves the repository's project boundary:
//! the binary wraps commands, while this independently selectable project owns the one command
//! that constructs and invokes the GitHub Projects adapter directly.

use onetaskgraph_github_projects::GitHubProjectsSource;
use onetaskgraph_plugin_api::{SecretResolver, SourceError, SourceName};

pub use onetaskgraph_github_projects::{
    GitHubProjectsConfig, StatusOptionsMode, StatusOptionsOutcome, StatusOptionsReport,
};

/// Plan or apply the guarded Status-option additions for one configured source.
pub async fn reconcile(
    name: &SourceName,
    config: GitHubProjectsConfig,
    secrets: &impl SecretResolver,
    mode: StatusOptionsMode,
) -> Result<StatusOptionsReport, SourceError> {
    GitHubProjectsSource::new(name, config, secrets)?
        .status_options(mode)
        .await
}
