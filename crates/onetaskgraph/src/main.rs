//! The `onetaskgraph` command.
//!
//! Every user-facing journey is proven by driving this binary as a subprocess and
//! asserting on its exit code, stdout and stderr — see `tests/`. Only `--help`,
//! `--version` and `schema` answer yet; the query verbs land with the engine.

use std::io::{self, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Something went wrong while doing what was asked. Distinct from clap's own exit code
/// for a malformed invocation, so a caller can tell "you typed it wrong" from "it broke".
const EXIT_FAILURE: u8 = 1;

/// One interface over the ticketing systems your work lives in.
///
/// Exit codes: `0` on success, `1` when a command failed while running, `2` when the
/// invocation itself was wrong (clap's own code for that). `4` is reserved for a query
/// that succeeded for some sources and failed for others without `--allow-partial`.
#[derive(Debug, Parser)]
#[command(name = "onetaskgraph", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the JSON Schema bundle the contract types generate.
    ///
    /// Both SDKs are generated from this document, so it is emitted from the
    /// running binary rather than committed: the schema and the types that
    /// serialise cannot drift when they are the same types.
    Schema,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("onetaskgraph: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Run one command, returning a message the caller prints to stderr on failure.
fn run(command: &Command) -> Result<(), String> {
    match command {
        Command::Schema => emit_schema(&mut io::stdout().lock()),
    }
}

/// Write the schema bundle to `out` as pretty-printed JSON.
fn emit_schema(out: &mut impl Write) -> Result<(), String> {
    let bundle = onetaskgraph_core::schema_bundle();
    let rendered = serde_json::to_string_pretty(&bundle)
        .map_err(|error| format!("could not render the schema bundle: {error}"))?;
    writeln!(out, "{rendered}")
        .map_err(|error| format!("could not write the schema bundle: {error}"))?;
    // Flush explicitly: a user piping into `head` closes the reader early, and a
    // buffered write that fails at drop would exit zero having emitted a truncated
    // bundle that a generator would then happily consume.
    out.flush()
        .map_err(|error| format!("could not write the schema bundle: {error}"))
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

    /// A sink that fails at the point `mode` names, standing in for a reader that
    /// went away mid-write and for one that went away between the write and the
    /// flush — the two ways `onetaskgraph schema | head -1` ends.
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
