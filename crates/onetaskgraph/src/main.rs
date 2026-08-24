//! The `onetaskgraph` command.
//!
//! Every user-facing journey is proven by driving this binary as a subprocess and
//! asserting on its exit code, stdout and stderr — see `tests/`. Nothing in here is
//! proven by calling it in-process, because the thing being checked is what a user's
//! shell sees.

mod cli;
mod render;

use std::io::{self, Write};
use std::process::ExitCode;
use std::str::FromStr as _;

use clap::Parser;
use onetaskgraph_core::config::Layer;
use onetaskgraph_core::{
    DependencyRequest, Engine, Environment, Filters, GlobalId, LabelRequest, Loaded, OutputFormat,
    PageToken, Paging, ProjectRequest, ProjectSelector, QueryResponse, SearchRequest,
    SourceFailure, TaskRequest,
};
use onetaskgraph_plugin_api::{LabelFilter, NativeId, SourceName, TextQuery};
use serde::Serialize;

use crate::cli::{
    Cli, Command, ConfigCommand, DependencyArgs, FilterArgs, LabelCommand, PageArgs,
    ProjectCommand, SelectionArgs, ShowArgs, SourcesCommand, TaskCommand,
};

/// Everything asked for was answered, by every source asked. Nothing else exits `0`.
const EXIT_OK: u8 = 0;

/// Something went wrong while doing what was asked. Distinct from [`EXIT_USAGE`], so a
/// caller can tell "you typed it wrong" from "it broke".
const EXIT_FAILURE: u8 = 1;

/// The invocation itself was wrong. Clap's own code for that, used here too: a `--set`
/// that is not `PATH=VALUE` is the same kind of mistake as an unknown flag, and a caller
/// that branches on the code cannot be asked to know which of the two spotted it.
const EXIT_USAGE: u8 = 2;

/// The query ran, some sources answered and at least one did not. A distinct code
/// because a caller scripting around this has to be able to tell a partial answer from a
/// complete one without parsing prose — `--allow-partial` is how a caller says it will
/// accept the answer anyway, and then this run exits `EXIT_OK` instead.
const EXIT_PARTIAL: u8 = 4;

/// One thread is enough: the concurrency the engine needs is several sources waiting on
/// I/O at once, which one thread interleaves, not several sources computing at once.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // The configuration flags are global — they parse on every verb, because every verb
    // the engine adds will read them — so the layer they make is built before any verb
    // runs. A malformed `--set` is then refused wherever it was written rather than only
    // on the verbs that happen to read the configuration today, and it is refused as the
    // typing mistake it is.
    let flags = match cli.overrides.layer() {
        Ok(flags) => flags,
        Err(message) => return fail(&message, EXIT_USAGE),
    };
    match run(&cli.command, &flags, &mut io::stdout().lock()).await {
        Ok(code) => ExitCode::from(code),
        Err(message) => fail(&message, EXIT_FAILURE),
    }
}

/// Report `message` on stderr and exit with `code`.
fn fail(message: &str, code: u8) -> ExitCode {
    eprintln!("onetaskgraph: {message}");
    ExitCode::from(code)
}

/// Render what one command answers with and write it to `out`.
///
/// Rendering and writing are separate on purpose, and every verb shares the one write:
/// a closed reader is the same failure whichever verb was writing, and one path is one
/// path to get right.
async fn run(command: &Command, flags: &Layer, out: &mut impl Write) -> Result<u8, String> {
    // Every verb validates the configuration it was handed, including the verbs that do
    // not read it. An unknown field, an unusable value, a plugin this build does not
    // have and a source name that breaks the pattern are mistakes wherever they were
    // written, and a verb that answered anyway would drop them in silence — which is the
    // one outcome this configuration layer is not allowed to have.
    let loaded = load(flags)?;
    let loaded = &loaded;

    // One match over every verb rather than a pre-dispatch and a second match: two
    // matches over one enum means one of them owes an answer for arms it never receives,
    // and an arm nothing can reach is an arm nothing checks. The engine is built inside
    // the arms that need one, so `schema` and `config show` still answer on a host where
    // a source cannot be built at all.
    match command {
        Command::Schema => {
            emit(out, schema_bundle()?.trim_end(), "the schema bundle")?;
            Ok(EXIT_OK)
        }

        Command::Config {
            command: ConfigCommand::Show,
        } => {
            emit(
                out,
                effective_config(loaded)?.trim_end(),
                "the configuration",
            )?;
            Ok(EXIT_OK)
        }

        Command::Sources {
            command: SourcesCommand::List,
        } => {
            let listings = engine(loaded).listing();
            let rendered = match loaded.config.output() {
                OutputFormat::Text => render::sources(&listings),
                OutputFormat::Json => json(&listings, "the sources")?,
            };
            emit(out, rendered.trim_end(), "the sources")?;
            Ok(EXIT_OK)
        }

        Command::Task {
            command: TaskCommand::List(args),
        } => {
            let request = TaskRequest {
                sources: selection(&args.selection)?,
                filters: filters(&args.filters)?,
                project: selector(args.project.as_deref(), args.no_project)?,
                paging: paging(loaded, &args.paging)?,
            };
            let response = engine(loaded)
                .tasks(&request)
                .await
                .map_err(|error| error.to_string())?;
            respond(out, loaded, response, render::tasks, &args.paging, "tasks")
        }

        Command::Task {
            command: TaskCommand::Show(args),
        } => {
            let response = engine(loaded)
                .task(&qualified(&args.id)?)
                .await
                .map_err(|error| error.to_string())?;
            show(out, loaded, response, render::task_detail, args, "task")
        }

        Command::Task {
            command: TaskCommand::Deps(args),
        } => {
            let request = dependency_request(loaded, args)?;
            let response = engine(loaded)
                .task_dependencies(&request)
                .await
                .map_err(|error| error.to_string())?;
            respond(
                out,
                loaded,
                response,
                render::edges,
                &args.paging,
                "dependencies",
            )
        }

        Command::Project {
            command: ProjectCommand::List(args),
        } => {
            let request = ProjectRequest {
                sources: selection(&args.selection)?,
                filters: filters(&args.filters)?,
                paging: paging(loaded, &args.paging)?,
            };
            let response = engine(loaded)
                .projects(&request)
                .await
                .map_err(|error| error.to_string())?;
            respond(
                out,
                loaded,
                response,
                render::projects,
                &args.paging,
                "projects",
            )
        }

        Command::Project {
            command: ProjectCommand::Show(args),
        } => {
            let response = engine(loaded)
                .project(&qualified(&args.id)?)
                .await
                .map_err(|error| error.to_string())?;
            show(
                out,
                loaded,
                response,
                render::project_detail,
                args,
                "project",
            )
        }

        Command::Project {
            command: ProjectCommand::Deps(args),
        } => {
            let request = dependency_request(loaded, args)?;
            let response = engine(loaded)
                .project_dependencies(&request)
                .await
                .map_err(|error| error.to_string())?;
            respond(
                out,
                loaded,
                response,
                render::edges,
                &args.paging,
                "dependencies",
            )
        }

        Command::Label {
            command: LabelCommand::List(args),
        } => {
            let request = LabelRequest {
                sources: selection(&args.selection)?,
                paging: paging(loaded, &args.paging)?,
            };
            let response = engine(loaded)
                .labels(&request)
                .await
                .map_err(|error| error.to_string())?;
            respond(
                out,
                loaded,
                response,
                render::labels,
                &args.paging,
                "labels",
            )
        }

        Command::Search(args) => {
            let request = SearchRequest {
                sources: selection(&args.selection)?,
                text: TextQuery {
                    terms: args.text.clone(),
                    fields: args.fields.fields(),
                },
                kind: args.kind.kind(),
                paging: paging(loaded, &args.paging)?,
            };
            let response = engine(loaded)
                .search(&request)
                .await
                .map_err(|error| error.to_string())?;
            respond(
                out,
                loaded,
                response,
                render::hits,
                &args.paging,
                "the search",
            )
        }
    }
}

/// The sources this configuration resolves to.
fn engine(loaded: &Loaded) -> Engine {
    Engine::build(&loaded.config, &loaded.secrets)
}

/// Write one page, report the sources that could not contribute, and say what the run
/// amounts to.
///
/// The failures go to standard error rather than into the rendered rows, so a text
/// answer piped into another program stays rows and nothing else while the person
/// running it still learns that a source was missing.
fn respond<T: Serialize>(
    out: &mut impl Write,
    loaded: &Loaded,
    response: QueryResponse<T>,
    text: impl FnOnce(&[T]) -> String,
    paging: &PageArgs,
    what: &str,
) -> Result<u8, String> {
    let rendered = match loaded.config.output() {
        OutputFormat::Text => {
            let mut rendered = text(&response.items);
            if paging.explain {
                rendered.push('\n');
                rendered.push_str(&render::plan(&response.plan));
            }
            // Without this a caller reading a terminal has no way to reach page two: the
            // token is the engine's own encoding and there is nothing else to type.
            if let Some(next) = &response.next {
                rendered.push_str(&format!("\nnext page: --page {next}\n"));
            }
            rendered
        }
        // The plan and the failures are fields of the response itself here, so
        // `--explain` adds nothing a machine reader did not already have.
        OutputFormat::Json => json(&response, what)?,
    };
    emit(out, rendered.trim_end(), what)?;
    Ok(report(&response.errors, paging.allow_partial))
}

/// Write one item, or say plainly that there is no such item.
fn show<T: Serialize>(
    out: &mut impl Write,
    loaded: &Loaded,
    response: QueryResponse<T>,
    text: impl FnOnce(&T) -> String,
    args: &ShowArgs,
    what: &str,
) -> Result<u8, String> {
    match (response.items.first(), response.errors.is_empty()) {
        (None, true) => Err(format!(
            "no {what} with that id\n\
             next: check the id, or list what is there — `onetaskgraph {what} list` \
             reports every {what} the configured sources hold."
        )),
        _ => {
            let rendered = match loaded.config.output() {
                OutputFormat::Text => {
                    let mut rendered = response.items.first().map(text).unwrap_or_default();
                    if args.explain {
                        rendered.push('\n');
                        rendered.push_str(&render::plan(&response.plan));
                    }
                    rendered
                }
                OutputFormat::Json => json(&response, what)?,
            };
            emit(out, rendered.trim_end(), what)?;
            Ok(report(&response.errors, args.allow_partial))
        }
    }
}

/// Name every source that could not answer, and say what the exit code will mean.
fn report(errors: &[SourceFailure], allow_partial: bool) -> u8 {
    if errors.is_empty() {
        return EXIT_OK;
    }
    for failure in errors {
        eprintln!(
            "onetaskgraph: source {} could not answer: {}",
            failure.source, failure.error
        );
    }
    if allow_partial {
        eprintln!(
            "onetaskgraph: the answer above is partial, and --allow-partial says that is \
             acceptable."
        );
        return EXIT_OK;
    }
    eprintln!(
        "onetaskgraph: next: fix the source(s) named above — `onetaskgraph sources list` \
         reports each one's state — or re-run with --allow-partial to accept an answer \
         without them."
    );
    EXIT_PARTIAL
}

/// The sources a request addresses, checked against the pattern a name must match.
fn selection(args: &SelectionArgs) -> Result<Vec<SourceName>, String> {
    args.source
        .iter()
        .map(|name| {
            SourceName::new(name.clone()).map_err(|error| {
                format!(
                    "--source {name}: {error}\n\
                     next: name a configured source — `onetaskgraph sources list` reports \
                     them."
                )
            })
        })
        .collect()
}

/// The filters a list verb was given.
fn filters(args: &FilterArgs) -> Result<Filters, String> {
    Ok(Filters {
        text: args.search.as_ref().map(|terms| TextQuery {
            terms: terms.clone(),
            fields: args.fields.fields(),
        }),
        labels: LabelFilter {
            // Repeating `--label` narrows rather than widens: a second one is a second
            // requirement, which is what a repeated filter flag means everywhere else.
            all_of: args.label.clone(),
            none_of: args.not_label.clone(),
            any_of: Vec::new(),
        },
        statuses: args.status.iter().map(|status| status.category()).collect(),
    })
}

/// Which project a `task list` was narrowed to.
fn selector(project: Option<&str>, orphans: bool) -> Result<ProjectSelector, String> {
    if orphans {
        return Ok(ProjectSelector::Orphans);
    }
    let Some(project) = project else {
        return Ok(ProjectSelector::Any);
    };
    // A qualified id names one project of one source; anything else is a native id every
    // selected source is asked about. Trying the qualified form first is what lets a
    // native id contain colons and still be a native id: only a prefix that is itself a
    // usable source name qualifies.
    Ok(match GlobalId::from_str(project) {
        Ok(id) => ProjectSelector::Qualified(id),
        Err(_) => ProjectSelector::Native(NativeId::from(project)),
    })
}

/// One qualified id, as a verb that takes one reads it.
fn qualified(id: &str) -> Result<GlobalId, String> {
    GlobalId::from_str(id).map_err(|error| {
        format!(
            "{error}\n\
             next: qualify the id with the source it belongs to — `onetaskgraph sources \
             list` reports the configured names."
        )
    })
}

/// Which page a verb was asked for.
fn paging(loaded: &Loaded, args: &PageArgs) -> Result<Paging, String> {
    let limit = args.limit.unwrap_or_else(|| loaded.config.page_size());
    let token = args
        .page
        .as_ref()
        .map(|raw| {
            PageToken::parse(raw.clone()).map_err(|error| {
                format!(
                    "--page: {error}\n\
                     next: pass a token exactly as a previous page reported it, or drop \
                     --page to start the walk again."
                )
            })
        })
        .transpose()?;
    Ok(Paging { limit, token })
}

/// One dependency walk, as a verb that takes one reads it.
fn dependency_request(loaded: &Loaded, args: &DependencyArgs) -> Result<DependencyRequest, String> {
    Ok(DependencyRequest {
        id: qualified(&args.id)?,
        direction: args.direction.direction(),
        paging: paging(loaded, &args.paging)?,
    })
}

/// One value as pretty-printed JSON.
fn json(value: &impl Serialize, what: &str) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| format!("could not render {what}: {error}"))
}

/// The schema bundle as pretty-printed JSON.
fn schema_bundle() -> Result<String, String> {
    serde_json::to_string_pretty(&onetaskgraph_core::schema_bundle())
        .map_err(|error| format!("could not render the schema bundle: {error}"))
}

/// The effective configuration, in the format it asks for.
fn effective_config(loaded: &Loaded) -> Result<String, String> {
    match loaded.config.output() {
        OutputFormat::Text => Ok(loaded.effective.render_text()),
        OutputFormat::Json => serde_json::to_string_pretty(&loaded.effective)
            .map_err(|error| format!("could not render the configuration: {error}")),
    }
}

/// Load the configuration: documents, then the environment, then these flags.
fn load(flags: &Layer) -> Result<Loaded, String> {
    let working_directory = std::env::current_dir().map_err(|error| {
        format!(
            "could not read the working directory: {error}\n\
             next: run this from a directory that still exists."
        )
    })?;
    onetaskgraph_core::config::load(&working_directory, &Environment::from_process(), flags)
        .map_err(|error| error.to_string())
}

/// Write `rendered` and a newline, reporting a failed write rather than dying at drop.
///
/// The flush is explicit: a user piping into `head` closes the reader early, and a
/// buffered write that fails at drop would exit zero having emitted a truncated
/// document that a generator would then happily consume.
fn emit(out: &mut impl Write, rendered: &str, what: &str) -> Result<(), String> {
    writeln!(out, "{rendered}").map_err(|error| format!("could not write {what}: {error}"))?;
    out.flush()
        .map_err(|error| format!("could not write {what}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render and write the schema bundle exactly as [`run`] does for that verb.
    ///
    /// The two steps rather than `run` itself, because `run` first loads the
    /// configuration — real documents, the real environment — and these tests are about
    /// the rendering and the write path, not about what happens to be on this machine.
    /// What `run` does with the configuration is proven where it can be proven honestly:
    /// by the journeys in `tests/configuration.rs`, which drive this binary as a
    /// subprocess against a sandboxed host.
    fn write_schema_bundle(out: &mut impl Write) -> Result<(), String> {
        emit(out, schema_bundle()?.trim_end(), "the schema bundle")
    }

    #[test]
    fn the_schema_verb_writes_a_bundle_with_every_contract_root() {
        let mut out = Vec::new();
        write_schema_bundle(&mut out).expect("the bundle renders");

        let bundle: serde_json::Value =
            serde_json::from_slice(&out).expect("the bundle is valid JSON");
        assert!(bundle["roots"]["Task"].is_object());
        assert!(bundle["plugin_config"]["in-memory"].is_object());
    }

    /// A sink that refuses the write when `fail_on_write` is set and the flush
    /// otherwise, standing in for a reader that went away mid-write and for one that
    /// went away between the write and the flush — the two ways
    /// `onetaskgraph <verb> | head -1` ends. Every verb writes through the one `emit`
    /// below, so driving it for one verb covers the path all of them take.
    struct Failing {
        fail_on_write: bool,
    }

    impl Write for Failing {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.fail_on_write {
                Err(io::Error::other("the pipe is closed"))
            } else {
                Ok(buf.len())
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("the pipe closed before the flush"))
        }
    }

    #[test]
    fn a_verb_reports_a_failed_write_rather_than_panicking() {
        let mut sink = Failing {
            fail_on_write: true,
        };
        let message = write_schema_bundle(&mut sink).expect_err("writes refused");
        assert!(
            message.contains("could not write the schema bundle"),
            "{message}"
        );
    }

    #[test]
    fn a_verb_reports_a_failed_flush_rather_than_exiting_zero_on_a_truncated_document() {
        // The dangerous case: the write is buffered and "succeeds", so without an
        // explicit flush the process would exit zero having emitted nothing.
        let mut sink = Failing {
            fail_on_write: false,
        };
        let message = write_schema_bundle(&mut sink).expect_err("flushes refused");
        assert!(
            message.contains("could not write the schema bundle"),
            "{message}"
        );
    }
}
