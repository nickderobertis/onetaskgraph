//! The command line, and the configuration layer it contributes.
//!
//! Flags are the highest of the three layers, and every setting is reachable here —
//! including every field of every named source, through `--set`. That is what makes
//! the product scriptable: a caller who can name a setting in a document can name the
//! same setting on the command line, at the same dotted path.

use std::num::NonZeroU32;

use clap::{Args, Parser, Subcommand, ValueEnum};
use onetaskgraph_core::config::{Layer, Origin, Setting, SettingPath, value_from_text};
use onetaskgraph_core::{OutputFormat, SearchKind};
use onetaskgraph_plugin_api::{Direction, StatusCategory, TextFields};
use serde_json::Value;

/// One interface over the ticketing systems your work lives in.
///
/// Exit codes: `0` on success, `1` when a command failed while running, `2` when the
/// invocation itself was wrong (clap's own code for that), `4` when a query succeeded
/// for some sources and failed for others without `--allow-partial`. `0` means success
/// and nothing else: a run that reached no source, or lost one, never exits `0` unless
/// you asked for a partial answer.
#[derive(Debug, Parser)]
// `bin_name` is pinned rather than left to clap, which takes it from argv[0] — and on
// Windows argv[0] is `onetaskgraph.exe`, so the usage line would name a different command
// there than the one this declares and than the one the documentation tells a user to type.
#[command(name = "onetaskgraph", bin_name = "onetaskgraph", version)]
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

    /// Work with the sources this configuration names.
    Sources {
        #[command(subcommand)]
        command: SourcesCommand,
    },

    /// List, show and walk tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },

    /// List, show and walk projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },

    /// List the labels the sources know.
    Label {
        #[command(subcommand)]
        command: LabelCommand,
    },

    /// Search tasks, projects, or both.
    Search(SearchArgs),
}

/// What `onetaskgraph sources` can do.
#[derive(Debug, Subcommand)]
pub enum SourcesCommand {
    /// List every configured source, its plugin, and what it declares it can do.
    ///
    /// A source that could not be built is listed too, with the reason — one broken
    /// credential is a source you can see is broken, not a command that stops working.
    List,
}

/// What `onetaskgraph task` can do.
#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// List tasks across the selected sources.
    List(TaskListArgs),
    /// Show one task by its qualified id, `<source>:<native-id>`.
    Show(ShowArgs),
    /// Walk one task's dependency edges.
    Deps(DependencyArgs),
}

/// What `onetaskgraph project` can do.
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List projects across the selected sources.
    List(ProjectListArgs),
    /// Show one project by its qualified id, `<source>:<native-id>`.
    Show(ShowArgs),
    /// Walk one project's dependency edges.
    Deps(DependencyArgs),
}

/// What `onetaskgraph label` can do.
#[derive(Debug, Subcommand)]
pub enum LabelCommand {
    /// List every label the selected sources know.
    List(LabelListArgs),
}

/// Which sources a query addresses.
#[derive(Debug, Args)]
pub struct SelectionArgs {
    /// Address this source. Repeat for several; omit for the configured selection.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — a `SourceName` here would move
    /// the refusal into clap, which reports it as an invalid *invocation* (exit 2) under
    /// clap's own wording. A name that cannot be a source name and one that names no
    /// configured source are the same typo to the user, and both owe the same next
    /// action; `selection` in `main` converts through `SourceName::new` immediately and
    /// attaches it, at the exit code the documented table gives that mistake.
    #[arg(long = "source", value_name = "S")]
    pub source: Vec<String>,
}

/// The filters the list verbs share.
#[derive(Debug, Args)]
pub struct FilterArgs {
    /// Keep items carrying this label. Repeat to require several at once.
    #[arg(long = "label", value_name = "L")]
    pub label: Vec<String>,

    /// Drop items carrying this label. Repeat for several.
    #[arg(long = "not-label", value_name = "L")]
    pub not_label: Vec<String>,

    /// Keep items in this status category. Repeat for several.
    #[arg(long = "status", value_name = "S")]
    pub status: Vec<StatusArg>,

    /// Keep items matching this text.
    #[arg(long, value_name = "TEXT")]
    pub search: Option<String>,

    /// Which fields --search looks in.
    #[arg(long = "in", value_name = "FIELDS", default_value = "both")]
    pub fields: FieldsArg,
}

/// How much of a result set to return, and how much to say about it.
#[derive(Debug, Args)]
pub struct PageArgs {
    /// How many items this page holds. Defaults to the `page_size` setting.
    ///
    /// A `NonZeroU32` rather than a range-checked `u32`: a page of no rows is not a page,
    /// and typing it should be refused where it was typed rather than carried inwards as
    /// a number some later layer has to remember to check.
    #[arg(long, value_name = "N")]
    pub limit: Option<NonZeroU32>,

    /// Resume from a token a previous page reported.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — a `PageToken` here would only
    /// move the *encoding* check to parse time, and a token is refused for three reasons
    /// beyond its encoding — a configuration it cannot address, a query it did not come
    /// from, a stream it does not resume — none of which is decidable before the
    /// configuration is loaded. Splitting one mistake ("I pasted the wrong token") across
    /// clap's exit 2 and the run's exit 1 is what that would buy.
    #[arg(long = "page", value_name = "TOKEN")]
    pub page: Option<String>,

    /// Report what each source was asked and what the engine did itself.
    #[arg(long)]
    pub explain: bool,

    /// Accept an answer some sources could not contribute to, and exit 0.
    #[arg(long = "allow-partial")]
    pub allow_partial: bool,
}

/// `onetaskgraph task list`.
#[derive(Debug, Args)]
pub struct TaskListArgs {
    #[command(flatten)]
    pub selection: SelectionArgs,

    #[command(flatten)]
    pub filters: FilterArgs,

    /// Keep tasks in this project, qualified (`work:PROJ-1`) or by native id.
    ///
    /// A qualified id names one project of one source, so it narrows the query to that
    /// source. A bare id is asked of every selected source.
    #[arg(long, value_name = "P", conflicts_with = "no_project")]
    pub project: Option<String>,

    /// Keep only tasks belonging to no project at all.
    #[arg(long = "no-project")]
    pub no_project: bool,

    #[command(flatten)]
    pub paging: PageArgs,
}

/// `onetaskgraph project list`.
#[derive(Debug, Args)]
pub struct ProjectListArgs {
    #[command(flatten)]
    pub selection: SelectionArgs,

    #[command(flatten)]
    pub filters: FilterArgs,

    #[command(flatten)]
    pub paging: PageArgs,
}

/// `onetaskgraph label list`.
#[derive(Debug, Args)]
pub struct LabelListArgs {
    #[command(flatten)]
    pub selection: SelectionArgs,

    #[command(flatten)]
    pub paging: PageArgs,
}

/// `onetaskgraph task show` and `onetaskgraph project show`.
#[derive(Debug, Args)]
pub struct ShowArgs {
    /// The qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — a `GlobalId` here would refuse an
    /// unqualified id as a bad invocation, under clap's wording. `qualified` in `main`
    /// converts through `GlobalId::from_str` immediately and says what a qualified id is
    /// and where to read the configured names, which is the answer a user typing `T-1`
    /// needs and the one this repository's failure journeys assert on.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Report what the source was asked.
    #[arg(long)]
    pub explain: bool,

    /// Accept an answer the source could not contribute to, and exit 0.
    #[arg(long = "allow-partial")]
    pub allow_partial: bool,
}

/// `onetaskgraph task deps` and `onetaskgraph project deps`.
#[derive(Debug, Args)]
pub struct DependencyArgs {
    /// The qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — a `GlobalId` here would refuse an
    /// unqualified id as a bad invocation, under clap's wording. `qualified` in `main`
    /// converts through `GlobalId::from_str` immediately and says what a qualified id is
    /// and where to read the configured names, which is the answer a user typing `T-1`
    /// needs and the one this repository's failure journeys assert on.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Which way to walk. Reverse is emulated for a forward-only source.
    #[arg(long, value_name = "DIRECTION", default_value = "depends-on")]
    pub direction: DirectionArg,

    #[command(flatten)]
    pub paging: PageArgs,
}

/// `onetaskgraph search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// What to look for.
    #[arg(value_name = "TEXT")]
    pub text: String,

    /// Which fields to look in.
    #[arg(long = "in", value_name = "FIELDS", default_value = "both")]
    pub fields: FieldsArg,

    /// Which entities to search.
    #[arg(long, value_name = "KIND", default_value = "both")]
    pub kind: KindArg,

    #[command(flatten)]
    pub selection: SelectionArgs,

    #[command(flatten)]
    pub paging: PageArgs,
}

/// A status category, as the command line spells it.
///
/// A command-line mirror of [`StatusCategory`] rather than that type itself, for the
/// reason [`Format`] carries: deriving clap's `ValueEnum` on a contract type would put
/// clap into the plugin contract's dependencies for the sake of one flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StatusArg {
    /// Known about, not yet queued.
    Backlog,
    /// Queued, not yet started.
    Todo,
    /// Being worked on.
    InProgress,
    /// Finished.
    Done,
    /// Abandoned.
    Cancelled,
    /// The source reported a status this vocabulary cannot place.
    Unknown,
}

impl StatusArg {
    /// The contract's own category.
    #[must_use]
    pub fn category(self) -> StatusCategory {
        match self {
            Self::Backlog => StatusCategory::Backlog,
            Self::Todo => StatusCategory::Todo,
            Self::InProgress => StatusCategory::InProgress,
            Self::Done => StatusCategory::Done,
            Self::Cancelled => StatusCategory::Cancelled,
            Self::Unknown => StatusCategory::Unknown,
        }
    }
}

/// Which fields a search covers, as the command line spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FieldsArg {
    /// Titles only.
    Title,
    /// Bodies only.
    Content,
    /// Either one matching is a match.
    Both,
}

impl FieldsArg {
    /// The contract's own field selector.
    #[must_use]
    pub fn fields(self) -> TextFields {
        match self {
            Self::Title => TextFields::Title,
            Self::Content => TextFields::Content,
            Self::Both => TextFields::TitleOrContent,
        }
    }
}

/// Which way a dependency walk goes, as the command line spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DirectionArg {
    /// What this item depends on.
    DependsOn,
    /// What depends on this item.
    DependedOnBy,
}

impl DirectionArg {
    /// The contract's own direction.
    #[must_use]
    pub fn direction(self) -> Direction {
        match self {
            Self::DependsOn => Direction::DependsOn,
            Self::DependedOnBy => Direction::DependedOnBy,
        }
    }
}

/// Which entities a search covers, as the command line spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KindArg {
    /// Tasks only.
    Task,
    /// Projects only.
    Project,
    /// Both, interleaved.
    Both,
}

impl KindArg {
    /// The engine's own search scope.
    #[must_use]
    pub fn kind(self) -> SearchKind {
        match self {
            Self::Task => SearchKind::Tasks,
            Self::Project => SearchKind::Projects,
            Self::Both => SearchKind::Both,
        }
    }
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
