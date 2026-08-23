//! The command line, and the configuration layer it contributes.
//!
//! Flags are the highest of the three layers, and every setting is reachable here —
//! including every field of every named source, through `--set`. That is what makes
//! the product scriptable: a caller who can name a setting in a document can name the
//! same setting on the command line, at the same dotted path.

use clap::{Args, Parser, Subcommand, ValueEnum};
use onetaskgraph_core::OutputFormat;
use onetaskgraph_core::config::{Layer, Origin, Setting, SettingPath, value_from_text};
use serde_json::Value;

/// One interface over the ticketing systems your work lives in.
///
/// Exit codes: `0` on success, `1` when a command failed while running, `2` when the
/// invocation itself was wrong (clap's own code for that). `4` is reserved for a query
/// that succeeded for some sources and failed for others without `--allow-partial`.
#[derive(Debug, Parser)]
#[command(name = "onetaskgraph", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub overrides: Overrides,
}

/// The verbs this binary answers.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the JSON Schema bundle the contract types generate.
    ///
    /// Both SDKs are generated from this document, so it is emitted from the
    /// running binary rather than committed: the schema and the types that
    /// serialise cannot drift when they are the same types.
    Schema,

    /// Work with the configuration this command reads.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

/// What `onetaskgraph config` can do.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show every setting, with the layer its value came from.
    ///
    /// The layer is named exactly: which document, which environment variable, or
    /// which flag. Rendered as JSON when `output` is `json` — which `--json` sets.
    Show,
}

/// The command-line layer: any setting, at the top of the stack.
#[derive(Debug, Args)]
pub struct Overrides {
    /// Set any setting: --set sources.work.config.root=/tmp/tasks
    ///
    /// The path is the same dotted path a document uses and the same one the
    /// `ONETASKGRAPH_` variables encode, so one name works at all three layers.
    #[arg(long = "set", value_name = "PATH=VALUE", global = true)]
    pub set: Vec<String>,

    /// How many items one page holds.
    ///
    /// Refused at zero here rather than at load: this flag parses on every verb, so a
    /// value only the configuration loader would have caught is one the verbs that do
    /// not load a configuration would accept in silence.
    #[arg(
        long,
        value_name = "N",
        global = true,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub page_size: Option<u32>,

    /// Which sources answer when a command names none.
    #[arg(long, value_name = "NAMES", value_delimiter = ',', global = true)]
    pub default_sources: Option<Vec<String>>,

    /// How output is rendered.
    #[arg(long, value_name = "FORMAT", global = true, conflicts_with = "json")]
    pub output: Option<Format>,

    /// Shorthand for --output json.
    #[arg(long, global = true)]
    pub json: bool,
}

impl Overrides {
    /// These flags as one configuration layer.
    ///
    /// # Errors
    ///
    /// Returns a message naming the flag when a `--set` argument is not
    /// `PATH=VALUE`, or its path addresses nothing.
    pub fn layer(&self) -> Result<Layer, String> {
        let mut settings = Vec::new();

        if let Some(page_size) = self.page_size {
            settings.push(at("page_size", Value::from(page_size), "--page-size"));
        }
        if let Some(names) = &self.default_sources {
            let names: Vec<Value> = names.iter().map(|name| Value::from(name.clone())).collect();
            settings.push(at(
                "default_sources",
                Value::Array(names),
                "--default-sources",
            ));
        }
        if let Some(format) = self.output {
            settings.push(at("output", format.setting(), "--output"));
        }
        if self.json {
            settings.push(at("output", Value::from("json"), "--json"));
        }

        // Last, so `--set output=text` beats `--json`: the general form is the more
        // specific instruction, and a caller who spells a path out means it.
        for assignment in &self.set {
            settings.push(assignment_setting(assignment)?);
        }

        Ok(Layer::new(settings))
    }
}

/// How output is rendered, as the command line accepts it.
///
/// A command-line mirror of [`OutputFormat`] rather than that type itself, because
/// deriving clap's `ValueEnum` on it would put clap into the engine's dependencies for
/// the sake of one flag. The two cannot disagree about *spelling*: [`Format::setting`]
/// produces its value by serialising the `OutputFormat` it stands for, so what reaches
/// the configuration is whatever the engine's own type writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// For a person reading a terminal.
    Text,
    /// For a program.
    Json,
}

impl Format {
    /// This format as the `output` setting's value.
    #[must_use]
    pub fn setting(self) -> Value {
        let format = match self {
            Self::Text => OutputFormat::Text,
            Self::Json => OutputFormat::Json,
        };
        serde_json::to_value(format).expect("an output format renders as JSON")
    }
}

/// One setting at a path this binary spells itself.
fn at(key: &str, value: Value, flag: &str) -> Setting {
    Setting {
        key: SettingPath::parse(key).expect("a path this binary spells has no empty segment"),
        value,
        origin: Origin::Flag {
            flag: flag.to_owned(),
        },
    }
}

/// One `--set PATH=VALUE` argument.
fn assignment_setting(assignment: &str) -> Result<Setting, String> {
    let Some((path, value)) = assignment.split_once('=') else {
        return Err(format!(
            "--set {assignment}: that is not an assignment\n\
             next: write it as --set PATH=VALUE, for example --set page_size=10."
        ));
    };
    Ok(Setting {
        key: SettingPath::parse(path).map_err(|error| format!("--set {error}"))?,
        value: value_from_text(value),
        origin: Origin::Flag {
            flag: format!("--set {path}"),
        },
    })
}
