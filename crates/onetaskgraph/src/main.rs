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

/// Something went wrong while doing what was asked. Distinct from clap's own exit code
/// for a malformed invocation, so a caller can tell "you typed it wrong" from "it broke".
const EXIT_FAILURE: u8 = 1;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("onetaskgraph: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Run one command, returning a message the caller prints to stderr on failure.
///
/// The configuration flags are global — they parse on every verb, because every verb
/// the engine adds will read them — so the layer they make is built *before* the verb
/// is dispatched. A malformed `--set` is then refused wherever it was written rather
/// than only on the verbs that happen to read the configuration today, which is the
/// difference between a flag that is checked at the boundary and one that is checked
/// wherever somebody remembered to.
fn run(cli: &Cli) -> Result<(), String> {
    let flags = cli.overrides.layer()?;
    match &cli.command {
        Command::Schema => emit_schema(&mut io::stdout().lock()),
        Command::Config { command } => match command {
            ConfigCommand::Show => show_config(&flags, &mut io::stdout().lock()),
        },
    }
}

/// Write the schema bundle to `out` as pretty-printed JSON.
fn emit_schema(out: &mut impl Write) -> Result<(), String> {
    let bundle = onetaskgraph_core::schema_bundle();
    let rendered = serde_json::to_string_pretty(&bundle)
        .map_err(|error| format!("could not render the schema bundle: {error}"))?;
    emit(out, &rendered, "the schema bundle")
}

/// Write the effective configuration to `out`, in the format it asks for.
fn show_config(flags: &Layer, out: &mut impl Write) -> Result<(), String> {
    let loaded = load(flags)?;
    let rendered = match loaded.config.output {
        OutputFormat::Text => loaded.effective.render_text(),
        OutputFormat::Json => serde_json::to_string_pretty(&loaded.effective)
            .map_err(|error| format!("could not render the configuration: {error}"))?,
    };
    emit(out, rendered.trim_end(), "the configuration")
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

    #[test]
    fn emit_schema_writes_a_bundle_with_every_contract_root() {
        let mut out = Vec::new();
        emit_schema(&mut out).expect("the bundle renders");

        let bundle: serde_json::Value =
            serde_json::from_slice(&out).expect("the bundle is valid JSON");
        assert!(bundle["roots"]["Task"].is_object());
        assert!(bundle["plugin_config"]["in-memory"].is_object());
    }

    /// A sink that refuses the write when `fail_on_write` is set and the flush
    /// otherwise, standing in for a reader that went away mid-write and for one that
    /// went away between the write and the flush — the two ways
    /// `onetaskgraph schema | head -1` ends.
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
    fn emit_schema_reports_a_failed_write_rather_than_panicking() {
        let mut sink = Failing {
            fail_on_write: true,
        };
        let message = emit_schema(&mut sink).expect_err("the sink refuses writes");
        assert!(
            message.contains("could not write the schema bundle"),
            "{message}"
        );
    }

    #[test]
    fn emit_schema_reports_a_failed_flush_rather_than_exiting_zero_on_a_truncated_bundle() {
        // The dangerous case: the write is buffered and "succeeds", so without an
        // explicit flush the process would exit zero having emitted nothing.
        let mut sink = Failing {
            fail_on_write: false,
        };
        let message = emit_schema(&mut sink).expect_err("the sink refuses flushes");
        assert!(
            message.contains("could not write the schema bundle"),
            "{message}"
        );
    }
}
