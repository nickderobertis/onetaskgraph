//! The plugin's half of the protocol, for a source this build already has.
//!
//! This is the reference implementation of the other side of
//! `docs/plugin-protocol.md`, and it exists for two reasons. It is what the
//! `onetaskgraph-source` program runs, so any registered plugin can be hosted in a child
//! process without being rewritten. And it is what makes the journeys real: the shared
//! fixture table configures a source through it, so every journey in the suite runs a
//! second time over a genuine pipe to a genuine second process rather than over a double
//! standing in for one.
//!
//! It answers strictly in order, which §1.1 names as the simpler correct choice, and it
//! never crashes on a bad line from the other side (§6.3).

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use onetaskgraph_plugin_api::{SecretResolver, SourceError, SourceName, TaskSource};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::connection::{Line, MAX_LINE, read_line};
use super::wire::{
    DependencyParams, HandshakePluginKind, IdParams, InitializeParams, InitializeResult,
    LabelParams, PROTOCOL_VERSION, ProjectQueryParams, ProjectWriteParams, Request, Response,
    TaskQueryParams, TaskWriteParams,
};
use crate::registry::PluginKind;

/// What this reference host needs in the `config` the handshake hands it.
///
/// The protocol says `config` is "this source's `config:` block, verbatim" and says
/// nothing about its contents, because they are the plugin's own business. This host's
/// business is to run one of *this build's* registered plugins, so its settings name
/// which one and hand over that plugin's block untouched.
#[derive(Debug, Clone, Deserialize)]
struct HostedSettings {
    /// The registered plugin kind to build.
    kind: PluginKind,
    /// That plugin's own `config:` block.
    #[serde(default)]
    config: Value,
}

/// Serve one connection until the engine closes its input.
///
/// # Errors
///
/// Returns the underlying [`std::io::Error`] when this process can no longer read its
/// input or write its output. Everything else — an unusable configuration, a request this
/// version has no method for, a line that is not JSON — is answered on the wire or
/// reported on standard error, because a plugin that exits on a bad line takes every
/// other in-flight request with it (§6.3).
pub async fn serve(input: impl BufRead, output: impl Write) -> std::io::Result<()> {
    serve_kind(input, output, None).await
}

/// Serve one connection as the registered plugin `kind`.
///
/// Unlike [`serve`], the initialize request's `config` is handed directly to that plugin;
/// the process command has already selected the kind it hosts.
pub async fn serve_plugin(
    input: impl BufRead,
    output: impl Write,
    kind: PluginKind,
) -> std::io::Result<()> {
    serve_kind(input, output, Some(kind)).await
}

async fn serve_kind(
    mut input: impl BufRead,
    mut output: impl Write,
    kind: Option<PluginKind>,
) -> std::io::Result<()> {
    let mut source: Option<Box<dyn TaskSource>> = None;
    loop {
        let line = match read_line(&mut input) {
            Line::Read(line) => line,
            Line::Ended => return Ok(()),
            Line::Failed(error) => return Err(error),
            // Nothing after an unterminated line can be framed — the rest of it would be
            // read as further requests it is not — so this side says why and stops rather
            // than answering questions nobody asked. The engine sees the closed stream.
            Line::TooLong => {
                eprintln!(
                    "onetaskgraph-source: a request ran past {MAX_LINE} bytes without \
                     ending its line; closing the connection"
                );
                return Ok(());
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let Some(id) = addressed(&line) else {
            eprintln!("onetaskgraph-source: ignoring a line with no request id: {line}");
            continue;
        };
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => answer(&mut source, request, kind).await,
            Err(error) => Response::failed(
                id,
                SourceError::Malformed {
                    message: format!("that is not a request envelope: {error}"),
                },
            ),
        };
        let finished = ended_the_connection(&response);
        writeln!(
            output,
            "{}",
            // A response is built from contract types that all serialize.
            serde_json::to_string(&response).expect("a response is plain data")
        )?;
        output.flush()?;
        if finished {
            return Ok(());
        }
    }
}

/// The `id` a line is addressed with, if it has one at all.
///
/// Read from the raw JSON rather than from a parsed [`Request`] because §6.3 turns on
/// exactly this difference: a request this side cannot otherwise understand is *answered*
/// when an id can be associated with it, and only reported on standard error when one
/// cannot.
fn addressed(line: &str) -> Option<String> {
    serde_json::from_str::<Value>(line)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

/// Whether this response is a refusal §6.2 says the plugin exits after.
fn ended_the_connection(response: &Response) -> bool {
    matches!(
        response.error.as_ref(),
        Some(SourceError::Config { message }) if message.starts_with(VERSION_REFUSAL)
    )
}

/// The opening of the one message §6.2 spells out, so the refusal and the exit that
/// follows it cannot drift apart.
const VERSION_REFUSAL: &str = "protocol version ";

/// Answer one well-formed request.
async fn answer(
    source: &mut Option<Box<dyn TaskSource>>,
    request: Request,
    kind: Option<PluginKind>,
) -> Response {
    let Request { id, method, params } = request;
    if method == "initialize" {
        return match source {
            Some(_) => Response::failed(
                id,
                SourceError::Malformed {
                    message: "this connection was already initialized".to_owned(),
                },
            ),
            None => initialize(source, id, params, kind),
        };
    }
    let Some(built) = source.as_deref() else {
        return Response::failed(
            id,
            SourceError::Malformed {
                message: format!("{method} arrived before the handshake"),
            },
        );
    };
    match dispatch(built, &method, params).await {
        Ok(result) => Response::ok(id, result),
        Err(error) => Response::failed(id, error),
    }
}

/// The handshake (§3), including the version refusal §6.2 spells out.
fn initialize(
    source: &mut Option<Box<dyn TaskSource>>,
    id: String,
    params: Value,
    kind: Option<PluginKind>,
) -> Response {
    let params: InitializeParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(error) => {
            return Response::failed(
                id,
                SourceError::Config {
                    message: format!("that is not an initialize request: {error}"),
                },
            );
        }
    };
    if params.protocol_version != PROTOCOL_VERSION {
        return Response::failed(
            id,
            SourceError::Config {
                message: format!(
                    "{VERSION_REFUSAL}{} is not supported by this plugin; it speaks \
                     version {PROTOCOL_VERSION}",
                    params.protocol_version
                ),
            },
        );
    }
    match build(&params, kind) {
        Ok(built) => {
            let kind = match HandshakePluginKind::new(built.kind()) {
                Ok(kind) => kind,
                Err(error) => {
                    return Response::failed(
                        id,
                        SourceError::Malformed {
                            message: format!("the hosted plugin reported an invalid kind: {error}"),
                        },
                    );
                }
            };
            let result = InitializeResult {
                protocol_version: Some(PROTOCOL_VERSION),
                kind,
                capabilities: built.capabilities(),
                writes: Some(built.writes()),
            };
            *source = Some(built);
            // An `InitializeResult` is a string, an integer and a `Capabilities`.
            Response::ok(
                id,
                serde_json::to_value(&result).expect("a result is plain data"),
            )
        }
        Err(error) => Response::failed(id, error),
    }
}

/// Build the registered plugin these settings name.
fn build(
    params: &InitializeParams,
    selected: Option<PluginKind>,
) -> Result<Box<dyn TaskSource>, SourceError> {
    let (kind, config) = match selected {
        Some(kind) => (kind, &params.config),
        None => {
            let settings: HostedSettings = serde_json::from_value(params.config.clone()).map_err(
                |error| SourceError::Config {
                    message: format!(
                        "this host serves a plugin of this build, and its settings must name one \
                         as {{\"kind\": …, \"config\": …}}: {error}"
                    ),
                },
            )?;
            return build_plugin(params, settings.kind, &settings.config);
        }
    };
    build_plugin(params, kind, config)
}

fn build_plugin(
    params: &InitializeParams,
    kind: PluginKind,
    config: &Value,
) -> Result<Box<dyn TaskSource>, SourceError> {
    let name = SourceName::new(params.source_name.clone())?;
    kind.plugin()
        .build(&name, config, &Handshake(&params.secrets))
}

/// The credentials the handshake forwarded, and nothing else.
///
/// §3.1 is the whole of this type: a plugin must not read credentials from its own
/// process environment, because doing so makes it work on a host where the engine's own
/// resolution would have failed — and that difference is exactly what `config show`
/// reports and a user relies on.
struct Handshake<'a>(&'a BTreeMap<String, String>);

impl SecretResolver for Handshake<'_> {
    fn get(&self, var: &str) -> Option<SecretString> {
        self.0
            .get(var)
            .map(|value| SecretString::from(value.clone()))
    }
}

/// One method call against the built source (§4).
async fn dispatch(
    source: &dyn TaskSource,
    method: &str,
    params: Value,
) -> Result<Value, SourceError> {
    match method {
        "health" => encode(source.health().await?),
        "get_task" => {
            let params: IdParams = decode(method, params)?;
            encode(json!({ "task": source.get_task(&params.id).await? }))
        }
        "get_project" => {
            let params: IdParams = decode(method, params)?;
            encode(json!({ "project": source.get_project(&params.id).await? }))
        }
        "query_tasks" => {
            let params: TaskQueryParams = decode(method, params)?;
            encode(source.query_tasks(&params.query, &params.page).await?)
        }
        "query_projects" => {
            let params: ProjectQueryParams = decode(method, params)?;
            encode(source.query_projects(&params.query, &params.page).await?)
        }
        "labels" => {
            let params: LabelParams = decode(method, params)?;
            encode(source.labels(&params.page).await?)
        }
        "task_dependencies" => {
            let params: DependencyParams = decode(method, params)?;
            encode(
                source
                    .task_dependencies(&params.id, params.direction, &params.page)
                    .await?,
            )
        }
        "project_dependencies" => {
            let params: DependencyParams = decode(method, params)?;
            encode(
                source
                    .project_dependencies(&params.id, params.direction, &params.page)
                    .await?,
            )
        }
        "write_task" => {
            let params: TaskWriteParams = decode(method, params)?;
            encode(json!({ "id": source.write_task(&params.write).await? }))
        }
        "write_project" => {
            let params: ProjectWriteParams = decode(method, params)?;
            encode(json!({ "id": source.write_project(&params.write).await? }))
        }
        other => Err(SourceError::Malformed {
            message: format!("protocol version {PROTOCOL_VERSION} has no method called {other:?}"),
        }),
    }
}

/// One method's parameters, or the reason they could not be read.
fn decode<T: for<'de> Deserialize<'de>>(method: &str, params: Value) -> Result<T, SourceError> {
    serde_json::from_value(params).map_err(|error| SourceError::Malformed {
        message: format!("the parameters of {method} are not the shape it takes: {error}"),
    })
}

/// One method's result as the value the envelope carries.
fn encode<T: serde::Serialize>(value: T) -> Result<Value, SourceError> {
    serde_json::to_value(value).map_err(|error| SourceError::Malformed {
        message: format!("this source returned data that will not serialize: {error}"),
    })
}
