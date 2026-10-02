//! The command line, and the configuration layer it contributes.
//!
//! Flags are the highest of the three layers, and every setting is reachable here —
//! including every field of every named source, through `--set`. That is what makes
//! the product scriptable: a caller who can name a setting in a document can name the
//! same setting on the command line, at the same dotted path.

use std::num::NonZeroU32;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use onetaskgraph_core::config::{Layer, Origin, Setting, SettingPath, value_from_text};
use onetaskgraph_core::{GlobalId, OutputFormat, PluginKind, SearchKind};
use onetaskgraph_plugin_api::{
    Direction, MetadataMatch, NativeId, Priority, StatusCategory, TextFields,
};
use serde_json::Value;

/// One interface over the ticketing systems your work lives in.
///
/// Exit codes: `0` on success, `1` when a command failed while running, `2` when the
/// invocation itself was wrong (clap's own code for that), `4` when a query succeeded
/// for some sources and failed for others without `--allow-partial`, or when a write landed
/// and a task it delivers could not be kept in step with it. `0` means success
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
    /// Host one compiled-in source over the stdio plugin protocol.
    #[command(hide = true)]
    PluginServe {
        /// The compiled-in plugin kind to host.
        source: PluginKind,
    },

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

    /// List, show and copy documents.
    ///
    /// A document is not work: it has no status and takes part in no dependency graph, so
    /// this group carries no `--status` filter and no `deps` verb.
    Document {
        #[command(subcommand)]
        command: DocumentCommand,
    },

    /// List the labels the sources know.
    Label {
        #[command(subcommand)]
        command: LabelCommand,
    },

    /// Search tasks, projects, or both.
    Search(SearchArgs),

    /// Read a task template's variables, and render it from answers.
    ///
    /// A template is minijinja with YAML front matter declaring its variables. `extends`,
    /// `include` and `import` resolve over the --search-path directories alone.
    Template {
        #[command(subcommand)]
        command: TemplateCommand,
    },
}

/// What `onetaskgraph template` can do.
#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// List every variable a template's chain declares, and the chain's digest.
    Variables(TemplateVariablesArgs),
    /// Render a template from answers, asking for the rest when interactive.
    ///
    /// Each variable takes the first of: --var, the answers file, a prompt (when
    /// interactive), its default. Every refusal of an answer exits 2.
    Render(TemplateRenderArgs),
}

/// `onetaskgraph template variables`.
// llmlint: ignore-block[invalid_states_unrepresentable] clap's derive has no one-field spelling for mutually exclusive options, so the template's sources are separate optional fields; the ArgGroup and `conflicts_with` on them refuse any two together where they are typed (exit 2), and `template::input` converts them to the one `TemplateInput` enum before anything else reads them.
#[derive(Debug, Args)]
#[command(group = clap::ArgGroup::new("template_source").required(true).args(["file", "template_loader"]))]
pub struct TemplateVariablesArgs {
    /// The template to read, at a path. It is known to its chain, and in its digest, by its
    /// file name; its own directory is not searched unless it is also a --search-path.
    #[arg(value_name = "FILE")]
    pub file: Option<std::path::PathBuf>,

    /// Resolve `extends`, `include` and `import` names in this directory. Repeat for several,
    /// searched in order; the working directory is never searched unless it is named.
    #[arg(
        long = "search-path",
        value_name = "DIR",
        conflicts_with = "template_loader"
    )]
    pub search_path: Vec<std::path::PathBuf>,

    /// Read the template from a loader document in place of FILE: a JSON object naming its
    /// `entry`, the `search_path` and inline `templates` its chain resolves over, and a
    /// `reference`; `-` reads standard input.
    #[arg(long = "template-loader", value_name = "FILE")]
    pub template_loader: Option<std::path::PathBuf>,
}
// llmlint: ignore-end[invalid_states_unrepresentable]

/// `onetaskgraph template render`.
// llmlint: ignore-block[invalid_states_unrepresentable] clap's derive has no one-field spelling for mutually exclusive options, so the template's sources are separate optional fields; the ArgGroup and `conflicts_with` on them refuse any two together where they are typed (exit 2), and `template::input` converts them to the one `TemplateInput` enum before anything else reads them.
#[derive(Debug, Args)]
#[command(group = clap::ArgGroup::new("template_source").required(true).args(["file", "template_loader"]))]
pub struct TemplateRenderArgs {
    /// The template to read, at a path. It is known to its chain, and in its digest, by its
    /// file name; its own directory is not searched unless it is also a --search-path.
    #[arg(value_name = "FILE")]
    pub file: Option<std::path::PathBuf>,

    /// Resolve `extends`, `include` and `import` names in this directory. Repeat for several,
    /// searched in order; the working directory is never searched unless it is named.
    #[arg(
        long = "search-path",
        value_name = "DIR",
        conflicts_with = "template_loader"
    )]
    pub search_path: Vec<std::path::PathBuf>,

    /// Read the template from a loader document in place of FILE; `-` reads standard input.
    #[arg(long = "template-loader", value_name = "FILE")]
    pub template_loader: Option<std::path::PathBuf>,

    /// Read answers from this YAML file, a mapping from variable name to value; `-` reads
    /// standard input.
    #[arg(long, value_name = "FILE")]
    pub answers: Option<std::path::PathBuf>,

    /// Answer one variable, over the answers file: literal text for a `string` or `text`
    /// variable, YAML for any other type. Repeat for several.
    #[arg(
        long = "var",
        value_name = "NAME=VALUE",
        allow_hyphen_values = true,
        value_parser = var_assignment
    )]
    pub var: Vec<VarAssignment>,
}
// llmlint: ignore-end[invalid_states_unrepresentable]

/// One `--var NAME=VALUE`, split where it was typed.
///
/// The value stays text: whether it is taken literally or read as YAML is decided by the type
/// the template declares for `name`, which is not known until the template is loaded.
#[derive(Debug, Clone)]
pub struct VarAssignment {
    /// The variable answered: a name a variable could be declared with. Private, so the only
    /// way to build one is `var_assignment` below, which refuses any other name.
    name: String,
    /// The answer, as typed.
    value: String,
}

impl VarAssignment {
    /// The variable answered.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The answer, as typed.
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Split one `--var` at its first `=`, refusing one with none, or whose name no variable
/// could be declared with.
fn var_assignment(raw: &str) -> Result<VarAssignment, String> {
    let (name, value) = raw.split_once('=').ok_or_else(|| {
        "that is not NAME=VALUE; next: write it as --var NAME=VALUE, for example \
         --var title=\"Ship it\""
            .to_owned()
    })?;
    if !onetaskgraph_core::template::is_variable_name(name) {
        return Err(format!(
            "{name:?} is not a variable name, which matches ^[a-z][a-z0-9_]*$; next: name a \
             variable the template declares — `onetaskgraph template variables` lists them"
        ));
    }
    Ok(VarAssignment {
        name: name.to_owned(),
        value: value.to_owned(),
    })
}

/// What `onetaskgraph sources` can do.
#[derive(Debug, Subcommand)]
pub enum SourcesCommand {
    /// List every configured source, its plugin, and what it declares it can do.
    ///
    /// A source that could not be built is listed too, with the reason — one broken
    /// credential is a source you can see is broken, not a command that stops working.
    List,
    /// Safely report or add configured GitHub Projects board Status options.
    ///
    /// The Status-only form of `sources fields`, which supersedes it. The default is a
    /// read-only plan. `--apply` preserves every existing option id and verifies every
    /// existing item assignment after GitHub replaces the option list.
    StatusOptions(SetupArgs),
    /// Safely report or set up every board field a GitHub Projects source's configuration
    /// names: its Status options, and its Priority field when `priority_mapping` is set.
    ///
    /// The default is a read-only plan. `--apply` adds missing options, creates a missing
    /// Priority field holding the mapped options, preserves every existing option's id,
    /// name, color and description, and verifies every item's values afterwards.
    Fields(SetupArgs),
}

/// Which configured source a guarded board setup inspects, and whether to apply its plan.
#[derive(Debug, Args)]
pub struct SetupArgs {
    /// The configured `github-projects` source name.
    // llmlint: ignore[invalid_states_unrepresentable] Clap collects this token as text;
    // the command converts it to `SourceName` before the configuration lookup or I/O.
    pub source: String,
    /// Add missing configured options and verify existing ids and assignments afterwards.
    #[arg(long)]
    // llmlint: ignore[invalid_states_unrepresentable] A presence-only CLI flag is
    // intrinsically boolean; the command immediately maps it to `SetupMode`.
    pub apply: bool,
}

/// What `onetaskgraph task` can do.
#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// List tasks across the selected sources.
    List(TaskListArgs),
    /// Show one task by its qualified id, `<source>:<native-id>`.
    Show(TaskShowArgs),
    /// Show several tasks by their qualified ids, each as `task show` reports it.
    ///
    /// One detail per id, in the order given, and the ids may span sources. An id that cannot
    /// be read — no such task, a source nothing configures, comments a source refuses — says
    /// why in its own detail's `errors` and does not refuse the others; the command exits
    /// non-zero exactly when some detail carries an error. Each source is asked for its ids
    /// together, so a source that reads many items in one request answers in few.
    ShowMany(TaskShowManyArgs),
    /// Walk one task's dependency edges.
    Deps(DependencyArgs),
    /// Copy tasks into another configured source, by qualified id.
    Copy(TaskCopyArgs),
    /// Add, list, edit and delete one task's comments.
    ///
    /// A copy never reads or writes a comment, at either end: these verbs are the only way
    /// one is written.
    Comment {
        #[command(subcommand)]
        command: CommentCommand,
    },
    /// Set one task's status, and nothing else about it.
    ///
    /// Every task it delivers is kept in step with it afterwards, and each is reported.
    Status {
        #[command(subcommand)]
        command: StatusCommand,
    },
    /// Set one task's priority, and nothing else about it.
    ///
    /// Priority is not status: no task it delivers is re-evaluated.
    Priority {
        #[command(subcommand)]
        command: PriorityCommand,
    },
    /// Replace one task's content, and nothing else about it.
    ///
    /// Content is not status: no task it delivers is re-evaluated.
    Content {
        #[command(subcommand)]
        command: ContentCommand,
    },
    /// Set one key of one task's metadata, and nothing else about it.
    ///
    /// Metadata is not status: no task it delivers is re-evaluated.
    Metadata {
        #[command(subcommand)]
        command: MetadataCommand,
    },
    /// Update one task: every field named, and nothing else.
    ///
    /// A field already holding the value named is not written, and an update in which nothing
    /// differs writes nothing at all. Naming a status or a list of tasks it delivers keeps every
    /// task it delivers in step with it afterwards, and each is reported; naming neither
    /// re-evaluates nothing.
    Update(TaskUpdateArgs),
    /// Create a task in one source, its body rendered from a template or given as it is.
    ///
    /// Rendered from a template, it records where it came from under the reserved
    /// `onetaskgraph.template` metadata key, and a source that keeps an authoring file
    /// (local-md) stores the answers beside it for a later `task render`.
    Create(TaskCreateArgs),
    /// Regenerate one task's content from its template in place, and nothing else about it.
    ///
    /// The answers start from the ones stored beside it when they are in step with its
    /// provenance; --var, --answers and --unset are laid over them.
    Render(RenderArgs),
    /// Print the template answers stored beside one task.
    Answers(AnswersArgs),
}

/// Where a created item's body comes from: exactly one of a template file, a loader
/// document, a body file, or — with none of them — standard input.
// llmlint: ignore-block[invalid_states_unrepresentable] clap's derive has no one-field spelling for mutually exclusive options, so the template's sources are separate optional fields; the ArgGroup and `conflicts_with` on them refuse any two together where they are typed (exit 2), and `template::input` converts them to the one `TemplateInput` enum before anything else reads them.
#[derive(Debug, Args)]
pub struct CreateBodyArgs {
    /// Render the body from this template file, recorded by its absolute path.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["template_loader", "body_file"])]
    pub template: Option<std::path::PathBuf>,

    /// Resolve the template's `extends`, `include` and `import` names in this directory.
    /// Repeat for several.
    #[arg(long = "search-path", value_name = "DIR", requires = "template")]
    pub search_path: Vec<std::path::PathBuf>,

    /// Render the body from the template a loader document states, recorded by its
    /// `reference`; `-` reads standard input.
    #[arg(
        long = "template-loader",
        value_name = "FILE",
        conflicts_with = "body_file"
    )]
    pub template_loader: Option<std::path::PathBuf>,

    /// Read the template's answers from this YAML file; `-` reads standard input.
    #[arg(long, value_name = "FILE", requires = "rendered")]
    pub answers: Option<std::path::PathBuf>,

    /// Answer one template variable, over the answers file. Repeat for several.
    #[arg(
        long = "var",
        value_name = "NAME=VALUE",
        allow_hyphen_values = true,
        value_parser = var_assignment,
        requires = "rendered"
    )]
    pub var: Vec<VarAssignment>,

    /// Read the body from this file, byte for byte, rather than rendering it.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: Option<std::path::PathBuf>,
}
// llmlint: ignore-end[invalid_states_unrepresentable]

/// What every create names about the item besides its body.
#[derive(Debug, Args)]
pub struct CreateItemArgs {
    /// The configured source to create it in.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `CopyArgs::to`: `main` converts
    /// it through `SourceName::new` with the next action a user needs.
    #[arg(value_name = "SOURCE")]
    pub source: String,

    /// The project to file it under, by the source's own id (or qualified with that source).
    #[arg(long, value_name = "P")]
    pub project: String,

    /// Its title.
    #[arg(long, value_name = "TITLE")]
    pub title: String,

    /// Give it this label. Repeat for several.
    #[arg(long = "label", value_name = "L")]
    pub label: Vec<String>,

    /// A repository its work changes, as a normalized origin (`github.com/owner/name`). Repeat
    /// for several.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — `main` converts each through the
    /// contract's own `Repository` and refuses one naming the problem.
    #[arg(long = "repository", value_name = "R")]
    pub repository: Vec<String>,

    /// Set one caller-owned metadata key to one JSON value. Repeat for several; a key in the
    /// reserved `onetaskgraph.` namespace is refused.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `MetadataSetArgs::key`: `main`
    /// converts each through `MetadataKey::new` and parses its value as JSON before anything
    /// is built.
    #[arg(long = "metadata", value_name = "KEY=JSON", allow_hyphen_values = true)]
    pub metadata: Vec<String>,

    #[command(flatten)]
    pub body: CreateBodyArgs,
}

/// `onetaskgraph task create`.
#[derive(Debug, Args)]
#[command(group = clap::ArgGroup::new("rendered").args(["template", "template_loader"]).multiple(false))]
pub struct TaskCreateArgs {
    #[command(flatten)]
    pub item: CreateItemArgs,

    /// Its status category; `todo` when none is given.
    #[arg(long = "status", value_name = "CATEGORY")]
    pub status: Option<StatusArg>,

    /// A task it depends on, qualified (`work:T-1`). Repeat for several.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `ShowArgs::id`.
    #[arg(long = "depends-on", value_name = "ID")]
    pub depends_on: Vec<String>,

    /// A task it delivers, qualified. Repeat for several; each is kept in step with it.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `ShowArgs::id`.
    #[arg(long = "delivers", value_name = "ID")]
    pub delivers: Vec<String>,
}

/// `onetaskgraph document create`.
#[derive(Debug, Args)]
#[command(group = clap::ArgGroup::new("rendered").args(["template", "template_loader"]).multiple(false))]
pub struct DocumentCreateArgs {
    #[command(flatten)]
    pub item: CreateItemArgs,

    /// Write it under this id: a document the source holds by it is replaced, and otherwise
    /// one is created under it.
    #[arg(long = "id", value_name = "DOC", value_parser = native_id)]
    pub id: Option<NativeId>,
}

/// `onetaskgraph task render` and `onetaskgraph document render`.
// llmlint: ignore-block[invalid_states_unrepresentable] clap's derive has no one-field spelling for mutually exclusive options, so the template's sources are separate optional fields; the ArgGroup and `conflicts_with` on them refuse any two together where they are typed (exit 2), and `template::input` converts them to the one `TemplateInput` enum before anything else reads them.
#[derive(Debug, Args)]
pub struct RenderArgs {
    /// The item's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `ShowArgs::id`.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Render this template file rather than the one the item records.
    #[arg(long, value_name = "FILE", conflicts_with = "template_loader")]
    pub template: Option<std::path::PathBuf>,

    /// Resolve the template's `extends`, `include` and `import` names in this directory —
    /// the given template's, or the recorded file's. Repeat for several.
    #[arg(
        long = "search-path",
        value_name = "DIR",
        conflicts_with = "template_loader"
    )]
    pub search_path: Vec<std::path::PathBuf>,

    /// Render the template a loader document states, which a recorded reference that is not
    /// a readable file requires; `-` reads standard input.
    #[arg(long = "template-loader", value_name = "FILE")]
    pub template_loader: Option<std::path::PathBuf>,

    /// Read answers from this YAML file, laid over the base; `-` reads standard input.
    #[arg(long, value_name = "FILE")]
    pub answers: Option<std::path::PathBuf>,

    /// Answer one variable, over the answers file. Repeat for several.
    #[arg(
        long = "var",
        value_name = "NAME=VALUE",
        allow_hyphen_values = true,
        value_parser = var_assignment
    )]
    pub var: Vec<VarAssignment>,

    /// Drop the answer this variable held, so it takes its default. Repeat for several.
    #[arg(long = "unset", value_name = "NAME", value_parser = variable_name)]
    pub unset: Vec<String>,

    /// Render and report, and write nothing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}
// llmlint: ignore-end[invalid_states_unrepresentable]

/// `onetaskgraph task answers` and `onetaskgraph document answers`.
#[derive(Debug, Args)]
pub struct AnswersArgs {
    /// The item's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `ShowArgs::id`.
    #[arg(value_name = "ID")]
    pub id: String,
}

/// One `--unset NAME`, refused unless a variable could be declared with it.
fn variable_name(raw: &str) -> Result<String, String> {
    if onetaskgraph_core::template::is_variable_name(raw) {
        Ok(raw.to_owned())
    } else {
        Err(format!(
            "{raw:?} is not a variable name, which matches ^[a-z][a-z0-9_]*$; next: name a \
             variable the template declares — `onetaskgraph template variables` lists them"
        ))
    }
}

/// `onetaskgraph task update`.
///
/// Every flag names one field of the task, and at least one is required — refused by
/// `update::update_task` rather than by a required group, which would spell every flag into the
/// usage line. A list flag given once or more replaces that list whole; its `--no-…` twin
/// replaces it with none.
// llmlint: ignore-block[invalid_states_unrepresentable] clap's derive has no one-field spelling for "a list, or explicitly none, or not named", so each replaceable list is a repeated option beside a `--no-…` flag; `conflicts_with` refuses the two together where they are typed (exit 2), and `update::request` folds each pair into the contract's own `Option<Vec<_>>` before anything else reads them.
#[derive(Debug, Args)]
pub struct TaskUpdateArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Its title.
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,

    /// Replace its content with this file's bytes.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: Option<std::path::PathBuf>,

    /// Its status category.
    #[arg(long = "status", value_name = "CATEGORY")]
    pub status: Option<StatusArg>,

    /// The status's own word, where the source keeps one — a folder of Markdown writes it as
    /// it is, and a source mapping statuses by category writes the category's option. The
    /// category's own word when left out.
    #[arg(long = "status-name", value_name = "NAME", requires = "status")]
    pub status_name: Option<String>,

    /// Its priority; `none` clears it.
    #[arg(long = "priority", value_name = "PRIORITY")]
    pub priority: Option<PriorityArg>,

    /// Set one caller-owned metadata key to one JSON value. Repeat for several; every other
    /// key is kept.
    #[arg(long = "metadata", value_name = "KEY=JSON", allow_hyphen_values = true)]
    pub metadata: Vec<String>,

    /// Remove one caller-owned metadata key. Repeat for several; a key the task does not hold
    /// is no write.
    #[arg(long = "remove-metadata", value_name = "KEY")]
    pub remove_metadata: Vec<String>,

    /// A task it delivers, qualified. Repeat for several; together they replace the list.
    #[arg(long = "delivers", value_name = "ID", conflicts_with = "no_delivers")]
    pub delivers: Vec<String>,

    /// Replace the list of tasks it delivers with none.
    #[arg(long = "no-delivers")]
    pub no_delivers: bool,

    /// A task it depends on, qualified. Repeat for several; together they replace its
    /// dependencies.
    #[arg(
        long = "depends-on",
        value_name = "ID",
        conflicts_with = "no_depends_on"
    )]
    pub depends_on: Vec<String>,

    /// Replace its dependencies with none.
    #[arg(long = "no-depends-on")]
    pub no_depends_on: bool,
}
// llmlint: ignore-end[invalid_states_unrepresentable]

/// What `onetaskgraph task priority` can do.
#[derive(Debug, Subcommand)]
pub enum PriorityCommand {
    /// Set one task's priority, and nothing else about it; `none` clears it.
    Set(PrioritySetArgs),
}

/// `onetaskgraph task priority set`.
#[derive(Debug, Args)]
pub struct PrioritySetArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `StatusSetArgs::id`: a `GlobalId`
    /// here would refuse an unqualified id as a bad invocation under clap's wording, and
    /// `qualified` in `main` converts it immediately with the next action a user needs.
    #[arg(value_name = "ID")]
    pub id: String,

    /// The priority to set.
    #[arg(value_name = "PRIORITY")]
    pub priority: PriorityArg,
}

/// What `onetaskgraph task content` can do.
#[derive(Debug, Subcommand)]
pub enum ContentCommand {
    /// Replace one task's content with a file's bytes, and nothing else about it.
    ///
    /// Status, priority, metadata, labels, repositories and dependencies are left as they
    /// were. There is no compare-and-set: the file's bytes replace whatever the task holds.
    Set(ContentSetArgs),
}

/// `onetaskgraph task content set`.
#[derive(Debug, Args)]
pub struct ContentSetArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `StatusSetArgs::id`.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Read the new content from this file, byte for byte.
    #[arg(long, value_name = "PATH")]
    pub file: std::path::PathBuf,
}

/// What `onetaskgraph task metadata`, `project metadata` and `document metadata` can do.
#[derive(Debug, Subcommand)]
pub enum MetadataCommand {
    /// Set one metadata key to one JSON value, adding the key or replacing what it holds.
    ///
    /// Every other key, and every other field of the record, is left as it was; a key already
    /// holding the value is not written at all.
    Set(MetadataSetArgs),
}

/// `onetaskgraph <task|project|document> metadata set`.
#[derive(Debug, Args)]
pub struct MetadataSetArgs {
    /// The record's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `StatusSetArgs::id`: a `GlobalId`
    /// here would refuse an unqualified id as a bad invocation under clap's wording, and
    /// `qualified` in `main` converts it immediately with the next action a user needs.
    #[arg(value_name = "ID")]
    pub id: String,

    /// The key, `<namespace>.<name>`: two or more non-empty dot-separated segments, never in
    /// the `onetaskgraph.` namespace.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `id` above: `metadata_key` in
    /// `main` converts it through `MetadataKey::new` before anything is built, and a clap
    /// value parser would report the same refusal as a bad invocation without its next action.
    #[arg(value_name = "KEY")]
    pub key: String,

    /// The value, as exactly one JSON value: `null`, `true`, `3`, `"text"`, `[…]` or `{…}`.
    ///
    /// Parsed strictly as JSON and never as YAML, so a bare word such as `yes` or a date such
    /// as `2026-01-01` is refused rather than silently given a type; quote a string.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `key` above: `metadata_value` in
    /// `main` parses it before anything is built, naming the parse error and the next action.
    #[arg(value_name = "VALUE", allow_hyphen_values = true)]
    pub value: String,
}

/// What `onetaskgraph task status` can do.
#[derive(Debug, Subcommand)]
pub enum StatusCommand {
    /// Set one task's status to a category, and nothing else about it.
    Set(StatusSetArgs),
}

/// `onetaskgraph task status set`.
#[derive(Debug, Args)]
pub struct StatusSetArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `CommentAddArgs::id`: a `GlobalId`
    /// here would refuse an unqualified id as a bad invocation under clap's wording, and
    /// `qualified` in `main` converts it immediately with the next action a user needs.
    #[arg(value_name = "ID")]
    pub id: String,

    /// The status category to set.
    #[arg(value_name = "CATEGORY")]
    pub category: StatusArg,
}

/// What `onetaskgraph task comment` can do.
///
/// A body is read from `--body-file`, or from standard input when that flag is absent, and
/// never from a word of the command line: a comment quoting a command must not pass through
/// a shell to reach the source.
#[derive(Debug, Subcommand)]
pub enum CommentCommand {
    /// Add a comment to a task. The body comes from --body-file, or from standard input.
    Add(CommentAddArgs),
    /// List a task's comments, oldest first.
    List(CommentListArgs),
    /// Replace one comment's body. The body comes from --body-file, or from standard input.
    Edit(CommentEditArgs),
    /// Delete one comment from a task.
    Delete(CommentDeleteArgs),
}

/// `onetaskgraph task comment add`.
#[derive(Debug, Args)]
pub struct CommentAddArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `ShowArgs::id`: a `GlobalId`
    /// here would refuse an unqualified id as a bad invocation under clap's wording, and
    /// `qualified` in `main` converts it immediately with the next action a user needs.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Read the body from this file rather than from standard input, byte for byte.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: Option<std::path::PathBuf>,

    /// Who wrote it. A source that records the author itself refuses this, saying why.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — an author is each source's own
    /// spelling of a person, so there is no narrower type every source agrees on; the
    /// source refuses what it cannot record, naming why, as `NewComment::author` records.
    #[arg(long, value_name = "NAME")]
    pub author: Option<String>,
}

/// `onetaskgraph task comment list`.
#[derive(Debug, Args)]
pub struct CommentListArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `CommentAddArgs::id`.
    #[arg(value_name = "ID")]
    pub id: String,
}

/// `onetaskgraph task comment edit`.
#[derive(Debug, Args)]
pub struct CommentEditArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `CommentAddArgs::id`.
    #[arg(value_name = "ID")]
    pub id: String,

    /// The comment's own id, exactly as `list` or `add` reported it.
    #[arg(value_name = "COMMENT-ID", value_parser = native_id)]
    pub comment_id: NativeId,

    /// Read the new body from this file rather than from standard input, byte for byte.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: Option<std::path::PathBuf>,
}

/// `onetaskgraph task comment delete`.
#[derive(Debug, Args)]
pub struct CommentDeleteArgs {
    /// The task's qualified id, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `CommentAddArgs::id`.
    #[arg(value_name = "ID")]
    pub id: String,

    /// The comment's own id, exactly as `list` or `add` reported it.
    #[arg(value_name = "COMMENT-ID", value_parser = native_id)]
    pub comment_id: NativeId,
}

/// A source's own id, as a command line hands one over.
///
/// Never refused: a native id is whatever its source issued, so the one thing this does is
/// give the value the contract's own type where it enters.
fn native_id(value: &str) -> Result<NativeId, std::convert::Infallible> {
    Ok(NativeId::from(value))
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
    /// Copy one project, and the tasks in it, into another configured source.
    Copy(ProjectCopyArgs),
    /// Set one key of one project's metadata, and nothing else about it.
    Metadata {
        #[command(subcommand)]
        command: MetadataCommand,
    },
}

/// What `onetaskgraph document` can do.
///
/// No `deps`: a document takes part in no dependency graph, so there is nothing for a
/// dependency verb here to walk.
#[derive(Debug, Subcommand)]
pub enum DocumentCommand {
    /// List documents across the selected sources.
    List(DocumentListArgs),
    /// Show one document by its qualified id, `<source>:<native-id>`.
    Show(ShowArgs),
    /// Copy documents into another configured source, by qualified id.
    Copy(DocumentCopyArgs),
    /// Set one key of one document's metadata, and nothing else about it.
    Metadata {
        #[command(subcommand)]
        command: MetadataCommand,
    },
    /// Create — or, with --id naming one it holds, replace — a project document, its body
    /// rendered from a template or given as it is.
    Create(DocumentCreateArgs),
    /// Regenerate one document's content from its template in place, and nothing else about
    /// it.
    Render(RenderArgs),
    /// Print the template answers stored beside one document.
    Answers(AnswersArgs),
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

/// The filters a document list carries.
///
/// [`FilterArgs`] without `--status`, and its own type rather than a shared one for the
/// reason [`DocumentFilters`](onetaskgraph_core::DocumentFilters) is: a document has no
/// status, so a status flag here could only be accepted and ignored.
#[derive(Debug, Args)]
pub struct DocumentFilterArgs {
    /// Keep documents carrying this label. Repeat to require several at once.
    #[arg(long = "label", value_name = "L")]
    pub label: Vec<String>,

    /// Drop documents carrying this label. Repeat for several.
    #[arg(long = "not-label", value_name = "L")]
    pub not_label: Vec<String>,

    /// Keep documents matching this text.
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

    /// Keep tasks with this priority. Repeat for several; a task matching any one is kept.
    #[arg(long = "priority", value_name = "PRIORITY")]
    pub priority: Vec<PriorityArg>,

    /// Keep tasks with a comment created or last edited at or after this RFC 3339 instant,
    /// offset included (`2026-09-28T12:00:00Z`). A task with no comments is never kept.
    ///
    /// A source that does not apply this itself has its comments read task by task.
    #[arg(long = "commented-since", value_name = "RFC3339", value_parser = instant)]
    pub commented_since: Option<DateTime<Utc>>,

    /// Keep tasks holding this metadata value: `<KEY>[/<SEGMENT>…]=<VALUE>`. Repeat for
    /// several; a task is kept when it holds every one.
    ///
    /// `/` splits the top-level key — which may contain dots, such as
    /// `orchestrator.follow-up` — from nested object keys under it, and the first `=` splits
    /// that location from the value. The value there must be a JSON string equal to VALUE,
    /// case-sensitively: `--metadata orchestrator.follow-up/root_cause=stale-cache`.
    #[arg(long = "metadata", value_name = "KEY[/SEGMENT...]=VALUE", value_parser = metadata_match)]
    pub metadata: Vec<MetadataMatch>,

    /// Keep tasks copied from this item: those whose recorded copy origin is exactly this
    /// qualified id, `<source>:<native-id>`.
    #[arg(long = "origin", value_name = "SOURCE:ID", value_parser = origin)]
    pub origin: Option<GlobalId>,

    #[command(flatten)]
    pub paging: PageArgs,
}

/// A metadata match as a command line hands one over: `<KEY>[/<SEGMENT>…]=<VALUE>`.
///
/// The first `=` ends the location, so a value may hold `=` and `/` freely; a key or a
/// segment may hold neither. An empty key or an empty segment names no location, so it is
/// refused rather than read as one.
fn metadata_match(value: &str) -> Result<MetadataMatch, String> {
    let Some((location, wanted)) = value.split_once('=') else {
        return Err("no `=`; expected <KEY>[/<SEGMENT>...]=<VALUE>, such as \
             orchestrator.follow-up/root_cause=stale-cache"
            .to_owned());
    };
    let mut segments = location.split('/').map(str::to_owned);
    let key = segments.next().unwrap_or_default();
    MetadataMatch::new(key, segments.collect(), wanted).map_err(|refused| {
        format!(
            "{refused}; expected <KEY>[/<SEGMENT>...]=<VALUE>, such as \
             orchestrator.follow-up/root_cause=stale-cache"
        )
    })
}

/// A copy origin as a command line hands one over: a qualified id, refused before any
/// source is asked when it is not one.
fn origin(value: &str) -> Result<GlobalId, String> {
    GlobalId::from_str(value).map_err(|error| error.to_string())
}

/// An instant as a command line hands one over: RFC 3339, with its offset.
///
/// An instant with no offset names a different moment in every time zone, so it is refused
/// rather than read as UTC or as local time.
fn instant(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| {
            format!(
                "{error}; expected an RFC 3339 instant with its offset, such as \
                 2026-09-28T12:00:00Z or 2026-09-28T08:00:00-04:00"
            )
        })
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

/// `onetaskgraph document list`.
#[derive(Debug, Args)]
pub struct DocumentListArgs {
    #[command(flatten)]
    pub selection: SelectionArgs,

    #[command(flatten)]
    pub filters: DocumentFilterArgs,

    /// Keep documents in this project, qualified (`work:PROJ-1`) or by native id.
    ///
    /// A qualified id names one project of one source, so it narrows the query to that
    /// source. A bare id is asked of every selected source.
    #[arg(long, value_name = "P", conflicts_with = "no_project")]
    pub project: Option<String>,

    /// Keep only documents belonging to no project at all.
    #[arg(long = "no-project")]
    pub no_project: bool,

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

/// `onetaskgraph task show`, optionally reading only the record.
#[derive(Debug, Args)]
pub struct TaskShowArgs {
    #[command(flatten)]
    pub item: ShowArgs,
    /// Read the record without requesting its comments.
    #[arg(long)]
    pub no_comments: bool,
}

/// `onetaskgraph task show-many`.
#[derive(Debug, Args)]
pub struct TaskShowManyArgs {
    /// The qualified ids, `<source>:<native-id>`, in the order their details are reported.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `TaskCopyArgs::id`: `qualified`
    /// in `main` converts each through `GlobalId::from_str` and says what a qualified id is.
    #[arg(value_name = "ID", required = true)]
    pub ids: Vec<String>,
    /// Read each record without requesting its comments.
    #[arg(long)]
    pub no_comments: bool,
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

/// The arguments both copy verbs share.
///
/// A copy is one write into one destination, so there is no paging, no `--explain` and no
/// `--allow-partial` here: a partial write is not an answer a caller could act on.
#[derive(Debug, Args)]
pub struct CopyArgs {
    /// The configured source to copy into. A source name, never a qualified id.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — a `SourceName` here would move
    /// the refusal into clap, which reports it as an invalid *invocation* (exit 2) under
    /// clap's own wording, for the reason recorded on `SelectionArgs::source`. A name
    /// that cannot be a source name and one that names no configured source are the same
    /// typo, and both owe the same next action.
    #[arg(long, value_name = "SOURCE")]
    pub to: String,

    /// Re-establish a lost correspondence by matching on `title` or on a metadata key.
    #[arg(long = "match-by", value_name = "KEY")]
    pub match_by: Option<String>,

    /// Create a new destination item when a recorded origin names nothing there.
    #[arg(long)]
    pub recreate: bool,

    /// Perform every read, write nothing, and report what would have happened.
    ///
    /// A dry run keeps no delivered task in step, so its `delivered` list is always empty —
    /// which says nothing about whether any delivered task would have moved.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

/// `onetaskgraph task copy`.
#[derive(Debug, Args)]
pub struct TaskCopyArgs {
    /// The qualified ids to copy, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — a `GlobalId` here would refuse an
    /// unqualified id as a bad invocation, under clap's wording, for the reason recorded
    /// on `ShowArgs::id`. `qualified` in `main` converts through `GlobalId::from_str` and
    /// says what a qualified id is and where to read the configured names.
    #[arg(value_name = "ID", required = true)]
    pub id: Vec<String>,

    #[command(flatten)]
    pub copy: CopyArgs,
}

/// `onetaskgraph project copy`.
#[derive(Debug, Args)]
pub struct ProjectCopyArgs {
    /// The qualified id to copy, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `TaskCopyArgs::id`.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Copy the project alone, leaving the tasks in it where they are.
    #[arg(long = "no-tasks")]
    pub no_tasks: bool,

    /// Copy the project and exactly this task of it, `<source>:<native-id>`; repeat it to
    /// name more. A task not named is not read at the destination, not written and not
    /// reported, and nothing is reported orphaned.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `TaskCopyArgs::id`.
    #[arg(long = "member", value_name = "TASK-ID", conflicts_with = "no_tasks")]
    pub member: Vec<String>,

    #[command(flatten)]
    pub copy: CopyArgs,
}

/// `onetaskgraph document copy`.
#[derive(Debug, Args)]
pub struct DocumentCopyArgs {
    /// The qualified ids to copy, `<source>:<native-id>`.
    ///
    /// llmlint: ignore[invalid_states_unrepresentable] — as `TaskCopyArgs::id`.
    #[arg(value_name = "ID", required = true)]
    pub id: Vec<String>,

    #[command(flatten)]
    pub copy: CopyArgs,
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
    /// Written down but not yet committed to as work.
    Draft,
    /// Known about, not yet accepted as ready to work.
    Backlog,
    /// Accepted and ready to be picked up, and nothing has claimed it.
    Todo,
    /// Claimed by work that will do it, and not yet started.
    Queued,
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
            Self::Draft => StatusCategory::Draft,
            Self::Backlog => StatusCategory::Backlog,
            Self::Todo => StatusCategory::Todo,
            Self::Queued => StatusCategory::Queued,
            Self::InProgress => StatusCategory::InProgress,
            Self::Done => StatusCategory::Done,
            Self::Cancelled => StatusCategory::Cancelled,
            Self::Unknown => StatusCategory::Unknown,
        }
    }
}

/// A priority, as the command line spells it.
///
/// A command-line mirror of [`Priority`] rather than that type itself, for the reason
/// [`StatusArg`] is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PriorityArg {
    /// No priority is set.
    None,
    /// Drop everything for it.
    Urgent,
    /// Next, before the rest.
    High,
    /// In its turn.
    Medium,
    /// When there is nothing more pressing.
    Low,
}

impl PriorityArg {
    /// The contract's own priority.
    #[must_use]
    pub fn priority(self) -> Priority {
        match self {
            Self::None => Priority::None,
            Self::Urgent => Priority::Urgent,
            Self::High => Priority::High,
            Self::Medium => Priority::Medium,
            Self::Low => Priority::Low,
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

    // llmlint: ignore-block[invalid_states_unrepresentable] Two presence-only flags, as `json` beside `output` above: clap's derive has no one-field spelling for a pair of opposing switches. Both fields are private, so only clap builds them, `conflicts_with` refuses both at once where they are typed (exit 2), and `layer` maps each to the one `interactive` setting immediately.
    /// Prompt for what a command was not given (the `interactive` setting's default).
    #[arg(long, global = true, conflicts_with = "no_interactive")]
    interactive: bool,

    /// Never prompt: refuse what a command was not given instead. For scripts and automation.
    #[arg(long = "no-interactive", global = true)]
    no_interactive: bool,
    // llmlint: ignore-end[invalid_states_unrepresentable]
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
        if self.interactive {
            settings.push(at("interactive", Value::Bool(true), "--interactive"));
        }
        if self.no_interactive {
            settings.push(at("interactive", Value::Bool(false), "--no-interactive"));
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
