//! One script run through either SDK — Python's or TypeScript's — against a sandbox, the way a
//! caller of that SDK reads through the binary.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::common::{Sandbox, stderr, stdout};

/// Run `script` through the TypeScript SDK when `typescript`, else the Python one, in
/// `sandbox`'s project with `cache_home` as its cache home, answering the JSON it printed.
///
/// The script sees a client over the binary — `client` in TypeScript, `c` in Python — the task
/// `task` as `id` and `task` respectively, and `pin`, absent unless given.
///
/// # Panics
///
/// When the SDK cannot be run, exits non-zero, or prints anything but JSON.
#[must_use]
pub fn run(
    sandbox: &Sandbox,
    cache_home: &Path,
    typescript: bool,
    script: &str,
    task: &str,
    pin: Option<&str>,
) -> Value {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = if typescript {
        let mut command = std::process::Command::new("bun");
        command.args([
            "-e",
            &format!(
                "import {{ OnetaskgraphClient }} from {}; delete process.env.HOME; \
                 const client = new OnetaskgraphClient({{binaryPath: process.env.CACHED_BINARY, \
                 cwd: process.cwd()}}); const id = process.env.CACHED_TASK; \
                 const pin = process.env.CACHED_PIN; {script}",
                serde_json::to_string(&workspace.join("sdks/typescript/src/client.ts")).unwrap()
            ),
        ]);
        command
    } else {
        let mut command = std::process::Command::new("uv");
        command
            .args(["run", "--frozen", "--project"])
            .arg(workspace.join("sdks/python"))
            .args([
                "python",
                "-c",
                &format!(
                    "import asyncio,os\nfrom onetaskgraph_sdk import Client\nasync def run():\n \
                     os.environ.pop('HOME',None)\n c=Client(os.environ['CACHED_BINARY'],cwd=os.getcwd())\n \
                     task=os.environ['CACHED_TASK']\n pin=os.environ.get('CACHED_PIN')\n {script}\n\
                     asyncio.run(run())"
                ),
            ]);
        command
    };
    // `uv` and `bun` keep HOME to find their runtimes; each script drops it before its SDK
    // spawns the binary.
    for (key, _) in std::env::vars() {
        if key.starts_with("ONETASKGRAPH_") {
            command.env_remove(key);
        }
    }
    command
        .current_dir(sandbox.project())
        .env("XDG_CONFIG_HOME", sandbox.config_home())
        .env("XDG_CACHE_HOME", cache_home)
        .env("CACHED_BINARY", crate::binary())
        .env("CACHED_TASK", task);
    if let Some(pin) = pin {
        command.env("CACHED_PIN", pin);
    }
    let output = command.output().expect("the SDK runs");
    assert!(output.status.success(), "{}", stderr(&output));
    serde_json::from_str(&stdout(&output)).expect("the SDK printed JSON")
}
