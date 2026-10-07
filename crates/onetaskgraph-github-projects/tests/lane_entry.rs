//! Refused nominations and CI identities never reach the lane's HTTP boundary.

// This entry-only journey compiles the shared full journey, whose cleanup helpers other targets drive.
#[allow(dead_code)]
mod journey;
// Admission uses only part of the shared lane; lane_shape and sweep_gate drive its remaining helpers.
#[allow(dead_code)]
mod lane;

#[tokio::test]
async fn invalid_identity_or_core_nomination_sends_no_request_and_opens_no_session() {
    use onetaskgraph_github_live::{CORE_REPOSITORY, SCRATCH_REPOSITORY};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    journey::against(journey::Endpoints {
        graphql: format!("{host}/graphql"),
        rest_host: host,
        source: None,
    });
    for required in [None, Some("1")] {
        for repository in [
            CORE_REPOSITORY.to_owned(),
            format!("  {}  ", CORE_REPOSITORY.to_uppercase()),
        ] {
            let error = journey::enter(
                &|name| match name {
                    "GH_PROJECTS_TOKEN" => Some("placeholder".to_owned()),
                    "GH_PROJECTS_OWNER" => Some("nickderobertis".to_owned()),
                    "GH_PROJECTS_NUMBER" => Some("1".to_owned()),
                    "GH_PROJECTS_REPOSITORY" => Some(repository.clone()),
                    "ONETASKGRAPH_LIVE_REQUIRED" => required.map(str::to_owned),
                    _ => None,
                },
                |_| panic!("refusal must precede opening the session"),
            )
            .await
            .unwrap_err();
            assert!(error.contains("GH_PROJECTS_REPOSITORY"), "{error}");
        }
    }
    for variable in ["GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"] {
        let wide = if variable == "GITHUB_RUN_ID" {
            "123456789012345678"
        } else {
            "1234"
        };
        for invalid in [
            None,
            Some("nonsense"),
            Some("0"),
            Some("01"),
            Some("-1"),
            Some(wide),
        ] {
            let error = journey::enter(
                &|name| {
                    if name == variable {
                        return invalid.map(str::to_owned);
                    }
                    match name {
                        "GH_PROJECTS_TOKEN" => Some("placeholder".to_owned()),
                        "GH_PROJECTS_OWNER" => Some("nickderobertis".to_owned()),
                        "GH_PROJECTS_NUMBER" => Some("1".to_owned()),
                        "GH_PROJECTS_REPOSITORY" => Some(SCRATCH_REPOSITORY.to_owned()),
                        "GITHUB_ACTIONS" => Some("true".to_owned()),
                        "GITHUB_RUN_ID" | "GITHUB_RUN_ATTEMPT" => Some("1".to_owned()),
                        _ => None,
                    }
                },
                |_| panic!("invalid CI identity must precede opening the session"),
            )
            .await
            .unwrap_err();
            assert!(error.contains(variable), "{error}");
        }
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
