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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use secrecy::SecretString;
    use serde_json::json;

    use super::*;

    struct Secrets;

    impl SecretResolver for Secrets {
        fn get(&self, variable: &str) -> Option<SecretString> {
            (variable == "GH_PROJECTS_TOKEN").then(|| "test-token".into())
        }
    }

    #[tokio::test]
    async fn reconcile_drives_the_adapter_through_its_real_http_boundary() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.contains("authorization: Bearer test-token"));
            assert!(request.contains("optionId"));
            let body = json!({"errors":[{"message":"fixture refusal"}]}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let config: GitHubProjectsConfig = serde_json::from_value(json!({
            "owner":"octo-org", "project_number":7, "endpoint":endpoint,
            "repository":"acme/work", "pacing":{"min_mutation_interval_ms":0,
            "retry_budget_ms":0}
        }))
        .unwrap();
        let error = reconcile(
            &SourceName::new("board").unwrap(),
            config,
            &Secrets,
            StatusOptionsMode::Plan,
        )
        .await
        .expect_err("the fixture refuses the request");
        assert!(error.to_string().contains("fixture refusal"));
    }
}
