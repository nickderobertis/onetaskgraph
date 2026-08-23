//! The `onetaskgraph` command.
//!
//! Every user-facing journey is proven by driving this binary as a subprocess and
//! asserting on its exit code, stdout and stderr — see `tests/`. `--help`,
//! `--version`, `schema` and `config show` answer today; the query verbs land with
//! the engine.

mod cli;

use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use onetaskgraph_core::config::Layer;
use onetaskgraph_core::{Environment, Loaded, OutputFormat};

use crate::cli::{Cli, Command, ConfigCommand};

/// Something went wrong while doing what was asked. Distinct from [`EXIT_USAGE`], so a
/// caller can tell "you typed it wrong" from "it broke".
const EXIT_FAILURE: u8 = 1;

/// The invocation itself was wrong. Clap's own code for that, used here too: a `--set`
/// that is not `PATH=VALUE` is the same kind of mistake as an unknown flag, and a caller
/// that branches on the code cannot be asked to know which of the two spotted it.
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
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
    match run(&cli.command, &flags, &mut io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
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
fn run(command: &Command, flags: &Layer, out: &mut impl Write) -> Result<(), String> {
    let (rendered, what) = match command {
        Command::Schema => (schema_bundle()?, "the schema bundle"),
        Command::Config { command } => match command {
            ConfigCommand::Show => (effective_config(flags)?, "the configuration"),
        },
    };
    emit(out, rendered.trim_end(), what)
}

/// The schema bundle as pretty-printed JSON.
fn schema_bundle() -> Result<String, String> {
    serde_json::to_string_pretty(&onetaskgraph_core::schema_bundle())
        .map_err(|error| format!("could not render the schema bundle: {error}"))
}

/// The effective configuration, in the format it asks for.
fn effective_config(flags: &Layer) -> Result<String, String> {
    let loaded = load(flags)?;
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

    /// The one verb whose rendering needs nothing of the host, so these tests are about
    /// the write path rather than about what happens to be on this machine.
    const SCHEMA: Command = Command::Schema;

    #[test]
    fn the_schema_verb_writes_a_bundle_with_every_contract_root() {
        let mut out = Vec::new();
        run(&SCHEMA, &Layer::default(), &mut out).expect("the bundle renders");

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
        let message = run(&SCHEMA, &Layer::default(), &mut sink).expect_err("writes refused");
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
        let message = run(&SCHEMA, &Layer::default(), &mut sink).expect_err("flushes refused");
        assert!(
            message.contains("could not write the schema bundle"),
            "{message}"
        );
    }
}
