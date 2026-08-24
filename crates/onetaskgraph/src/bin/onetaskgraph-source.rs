//! Host one of this build's plugins as a subprocess source.
//!
//! The engine spawns this program, speaks `docs/plugin-protocol.md` to it over stdio, and
//! never learns that the source on the other end is a plugin it could also have built
//! in-process. That is the point of it: it makes the protocol's two halves testable
//! against each other, and it is the reference a plugin written in another language is
//! read beside.
//!
//! It takes no arguments and reads no environment. Both are deliberate — §1.2 says the
//! plugin reads its configuration from the `initialize` request rather than from either,
//! and §3.1 forbids it reading credentials from its own process environment.

use std::io::{BufReader, Write, stderr, stdin, stdout};
use std::process::ExitCode;

/// Serve one connection, reporting a stream that failed on standard error.
///
/// Standard output carries response lines and nothing else (§1), which is why the one
/// diagnostic this program can produce goes to standard error.
#[tokio::main]
async fn main() -> ExitCode {
    let input = BufReader::new(stdin().lock());
    let output = stdout().lock();
    match onetaskgraph_core::serve(input, output).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr(), "onetaskgraph-source: {error}");
            ExitCode::FAILURE
        }
    }
}
