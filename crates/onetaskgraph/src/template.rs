//! `onetaskgraph template`: read a template's variables, and render it from answers.
//!
//! The format, the chain and the render are the engine's (`onetaskgraph_core::template`);
//! what lives here is what only a command line has — `--var`, an answers file on a path or
//! standard input, and the prompts. Prompting is this crate's alone: nothing below the
//! command line depends on a prompt library, and a library caller never meets one.

use std::io::{self, IsTerminal as _, Read as _, Write};
use std::path::Path;

use dialoguer::Input;
use dialoguer::console::Term;
use onetaskgraph_core::{
    Answers, Failure, Loaded, LoaderDocument, OutputFormat, Template, TemplateError, TemplateInput,
    TemplateVariable,
};

use crate::cli::{TemplateCommand, TemplateRenderArgs, VarAssignment};
use crate::{EXIT_OK, EXIT_USAGE, emit, json, render};

/// The line that ends a multi-line answer at a prompt.
const END_OF_VALUE: &str = ".";

/// Drive one `template` verb and write what it answered.
///
/// Every refusal of the answers — an answer to no declared variable, one of the wrong type,
/// required variables left unanswered, and an interactive run with nothing to prompt on —
/// is the invocation's mistake, so it is reported here and exits [`EXIT_USAGE`]. A template
/// that cannot be loaded or rendered is a command that failed, and is returned as one.
pub(crate) fn run(
    out: &mut impl Write,
    loaded: &Loaded,
    command: &TemplateCommand,
) -> Result<u8, Failure> {
    match command {
        TemplateCommand::Variables(args) => {
            let input = match input(
                args.file.as_deref(),
                &args.search_path,
                args.template_loader.as_deref(),
                None,
            )
            .and_then(named)
            {
                Ok(input) => input,
                Err(Refusal::Answers(message)) => return Ok(refused(&message)),
                Err(Refusal::Failed(failure)) => return Err(failure),
            };
            let template = input.load().map_err(|error| failure(&error))?;
            // The declared set includes what an expression names when every variable takes
            // its default, which is the most this verb can know without answers.
            let described = template
                .expand(&Answers::new())
                .map_err(|error| failure(&error))?
                .describe();
            let rendered = match loaded.config.output() {
                OutputFormat::Text => render::template_variables(&described),
                OutputFormat::Json => json(&described, "the template's variables")?,
            };
            emit(out, rendered.trim_end(), "the template's variables")?;
            Ok(EXIT_OK)
        }
        TemplateCommand::Render(args) => match render_template(out, loaded, args) {
            Err(Refusal::Answers(message)) => Ok(refused(&message)),
            Err(Refusal::Failed(failure)) => Err(failure),
            Ok(()) => Ok(EXIT_OK),
        },
    }
}

/// Report a refusal of the invocation on standard error, and answer [`EXIT_USAGE`].
pub(crate) fn refused(message: &str) -> u8 {
    // Best effort, as `fail` writes: the exit code carries the refusal regardless.
    let _ = writeln!(io::stderr(), "onetaskgraph: {message}");
    EXIT_USAGE
}

/// Why a render did not happen.
pub(crate) enum Refusal {
    /// The answers were refused: exit `2`.
    Answers(String),
    /// The command failed: exit `1`.
    Failed(Failure),
}

impl From<Failure> for Refusal {
    fn from(failure: Failure) -> Self {
        Self::Failed(failure)
    }
}

impl From<TemplateError> for Refusal {
    fn from(error: TemplateError) -> Self {
        if error.refuses_answers() {
            Self::Answers(error.to_string())
        } else {
            Self::Failed(failure(&error))
        }
    }
}

pub(crate) fn failure(error: &TemplateError) -> Failure {
    Failure::decided(error.kind(), error.to_string())
}

/// The template a verb names: the file at `file` over `search_path`, or the one the loader
/// document at `loader` states — `None` when it names neither.
///
/// `answers` is the verb's `--answers`, so the one standard input is never promised to both:
/// `--answers -` beside `--template-loader -` is refused as the invocation's mistake.
pub(crate) fn input(
    file: Option<&Path>,
    search_path: &[std::path::PathBuf],
    loader: Option<&Path>,
    answers: Option<&Path>,
) -> Result<Option<TemplateInput>, Refusal> {
    if let Some(loader) = loader {
        if loader.as_os_str() == "-" && answers.is_some_and(|answers| answers.as_os_str() == "-") {
            return Err(Refusal::Answers(
                "--answers - and --template-loader - both read standard input, which holds one \
                 document\n\
                 next: pass one of them as a file."
                    .to_owned(),
            ));
        }
        return loader_document(loader).map(|document| Some(TemplateInput::Loader(document)));
    }
    Ok(file.map(|path| TemplateInput::File {
        path: path.to_owned(),
        search_path: search_path.to_vec(),
    }))
}

/// The template a `template` verb names, which clap has already required it to name.
fn named(input: Option<TemplateInput>) -> Result<TemplateInput, Refusal> {
    input.ok_or_else(|| {
        Refusal::Answers(
            "no template was named\nnext: name a template FILE, or pass --template-loader FILE."
                .to_owned(),
        )
    })
}

/// The loader document at `path`, or on standard input for `-`.
///
/// One that cannot be read, or is not a loader document, is refused as the command failing:
/// what it names is the template, not an answer.
fn loader_document(path: &Path) -> Result<LoaderDocument, Refusal> {
    let unreadable = |error: io::Error| {
        Refusal::Failed(Failure::decided(
            "template-loader-unreadable",
            format!(
                "--template-loader {}: could not read it: {error}\n\
                 next: name a readable JSON file, or `-` for standard input.",
                path.display()
            ),
        ))
    };
    let text = if path.as_os_str() == "-" {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text).map_err(unreadable)?;
        text
    } else {
        std::fs::read_to_string(path).map_err(unreadable)?
    };
    LoaderDocument::from_json(&text).map_err(|error| {
        Refusal::Failed(Failure::decided(
            error.kind(),
            format!("--template-loader {}: {error}", path.display()),
        ))
    })
}

/// `template render`: resolve the answers in C2's order, prompt for the rest when
/// interactive, render, and write the result.
fn render_template(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &TemplateRenderArgs,
) -> Result<(), Refusal> {
    let flags = var_answers(&args.var);
    let template = input(
        args.file.as_deref(),
        &args.search_path,
        args.template_loader.as_deref(),
        args.answers.as_deref(),
    )
    .and_then(named)?
    .load()?;
    let file = match &args.answers {
        Some(path) => answers_file(path)?,
        None => Answers::new(),
    };
    let answers = ask(
        loaded,
        &template,
        file.overlay(&flags),
        &std::collections::HashSet::new(),
    )?;
    let rendered = template.render(&answers)?;
    match loaded.config.output() {
        OutputFormat::Json => emit(
            out,
            &json(&rendered, "the rendered template")?,
            "the rendered template",
        )?,
        // The body exactly as it rendered — its trailing newlines are part of it, so it is
        // written as it is rather than through `emit`, which ends every answer with one.
        OutputFormat::Text => write_body(out, &rendered.body)?,
    }
    Ok(())
}

/// The answers a verb was given — `--answers` under `--var`, which [`answers_given`] reads —
/// with, when interactive, an answer asked for every variable of `template` they leave
/// unanswered.
///
/// A variable `settled` names is never asked for: its answer, or its absence, is already
/// decided.
pub(crate) fn ask(
    loaded: &Loaded,
    template: &Template,
    mut answers: Answers,
    settled: &std::collections::HashSet<String>,
) -> Result<Answers, Refusal> {
    // Asked in rounds: an answer can make an expression name a template that declares more,
    // and each round asks only what no earlier round asked.
    let mut asked: std::collections::HashSet<String> = std::collections::HashSet::new();
    while loaded.config.interactive() {
        let unanswered: Vec<TemplateVariable> = template
            .unanswered(&answers)?
            .into_iter()
            .filter(|variable| {
                !asked.contains(variable.name()) && !settled.contains(variable.name())
            })
            .collect();
        if unanswered.is_empty() {
            break;
        }
        // The answers are read from standard input and the prompts written to standard
        // error, so both have to be the terminal a person is at.
        let not_a_terminal = if !io::stdin().is_terminal() {
            Some("standard input is not a terminal")
        } else if !io::stderr().is_terminal() {
            Some("standard error, where the prompts are written, is not a terminal")
        } else {
            None
        };
        if let Some(not_a_terminal) = not_a_terminal {
            let names: Vec<&str> = unanswered.iter().map(|variable| variable.name()).collect();
            return Err(Refusal::Answers(format!(
                "template {}: {} unanswered ({}) and {not_a_terminal}, so there is nobody to \
                 ask\n\
                 next: answer {} with --var NAME=VALUE or an answers file (--answers FILE), \
                 and pass --no-interactive (or set `interactive: false`) so a variable with a \
                 default takes it instead of being asked for.",
                template.name(),
                if names.len() == 1 {
                    "a variable is"
                } else {
                    "variables are"
                },
                names.join(", "),
                if names.len() == 1 { "it" } else { "them" },
            )));
        }
        let prompted = prompt(template, &unanswered).map_err(|error| {
            Refusal::Failed(Failure::decided(
                "template-prompt",
                format!(
                    "the prompt for {}'s variables stopped: {error}\n\
                     next: run it again and answer every prompt, or pass the answers with \
                     --var or --answers and --no-interactive.",
                    template.name()
                ),
            ))
        })?;
        asked.extend(unanswered.iter().map(|variable| variable.name().to_owned()));
        answers = answers.overlay(&prompted);
    }
    Ok(answers)
}

/// The answers a verb's `--answers` file and `--var` flags give, the flags laid over the file.
pub(crate) fn answers_given(
    path: Option<&Path>,
    assignments: &[VarAssignment],
) -> Result<Answers, Refusal> {
    let file = match path {
        Some(path) => answers_file(path)?,
        None => Answers::new(),
    };
    Ok(file.overlay(&var_answers(assignments)))
}

/// Write a rendered body byte for byte, reporting a failed write as `emit` does.
pub(crate) fn write_body(out: &mut impl Write, body: &str) -> Result<(), Failure> {
    let unwritten = |error: io::Error| {
        Failure::decided(
            "write",
            format!("could not write the rendered template: {error}"),
        )
    };
    out.write_all(body.as_bytes()).map_err(unwritten)?;
    out.flush().map_err(unwritten)
}

/// Every `--var NAME=VALUE`, as answers kept as text until they meet their declarations.
fn var_answers(assignments: &[VarAssignment]) -> Answers {
    let mut answers = Answers::new();
    for assignment in assignments {
        answers.set_text(assignment.name(), assignment.value());
    }
    answers
}

/// The answers document at `path`, or on standard input for `-`.
fn answers_file(path: &Path) -> Result<Answers, Refusal> {
    let text = if path.as_os_str() == "-" {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text).map_err(|error| {
            Refusal::Answers(format!(
                "--answers -: could not read the answers from standard input: {error}\n\
                 next: pass UTF-8 YAML on standard input, or name a file."
            ))
        })?;
        text
    } else {
        std::fs::read_to_string(path).map_err(|error| {
            Refusal::Answers(format!(
                "--answers {}: could not read it: {error}\n\
                 next: name a readable UTF-8 YAML file, or `-` for standard input.",
                path.display()
            ))
        })?
    };
    Answers::from_yaml(&text)
        .map_err(|error| Refusal::Answers(format!("--answers {}: {error}", path.display())))
}

/// Ask for each of `unanswered`, one at a time and in declaration order, on the terminal.
///
/// A value that is not one of the variable's type is answered with why and the same
/// question again — never an exit. An empty answer takes the variable's default, or leaves
/// an optional one `none`; a required variable with no default is asked again.
fn prompt(template: &Template, unanswered: &[TemplateVariable]) -> io::Result<Answers> {
    let term = Term::stderr();
    term.write_line(&format!(
        "{} asks for {} variable{}.",
        template.name(),
        unanswered.len(),
        if unanswered.len() == 1 { "" } else { "s" }
    ))?;
    let mut answers = Answers::new();
    for variable in unanswered {
        term.write_line("")?;
        term.write_line(&heading(variable))?;
        let answer = if variable.multi_line() {
            multi_line(&term, variable)?
        } else {
            one_line(&term, variable)?
        };
        if let Some(value) = answer {
            answers.set(variable.name(), value);
        }
    }
    Ok(answers)
}

/// What a prompt shows before it asks: the name, the type, the description, and what an
/// empty answer means.
fn heading(variable: &TemplateVariable) -> String {
    let kind = match variable.items() {
        Some(items) => format!("{} of {}", variable.kind(), items.as_str()),
        None => variable.kind().to_string(),
    };
    let empty = match (variable.default(), variable.required()) {
        (Some(default), _) => format!(
            "default {}",
            serde_json::to_string(default).unwrap_or_default()
        ),
        (None, false) => "optional; empty leaves it none".to_owned(),
        (None, true) => "required".to_owned(),
    };
    format!(
        "{} ({kind}, {empty}): {}",
        variable.name(),
        variable.description()
    )
}

/// What an empty answer amounts to: `Ok(None)` for the default or `none`, and `Err` with
/// why when the variable is required and has no default.
fn empty_answer(variable: &TemplateVariable) -> Result<Option<serde_json::Value>, String> {
    if variable.required() && variable.default().is_none() {
        Err(format!("{} is required; enter a value", variable.name()))
    } else {
        Ok(None)
    }
}

/// One line, re-asked until it is a value of the variable's type.
fn one_line(term: &Term, variable: &TemplateVariable) -> io::Result<Option<serde_json::Value>> {
    let text = Input::<String>::new()
        .with_prompt(variable.name())
        .allow_empty(true)
        .report(false)
        .validate_with(|text: &String| -> Result<(), String> {
            if text.is_empty() {
                empty_answer(variable).map(|_| ())
            } else {
                variable
                    .parse(text)
                    .map(|_| ())
                    .map_err(|problem| format!("not {}: {problem}", variable.kind()))
            }
        })
        .interact_on(term)
        .map_err(|dialoguer::Error::IO(error)| error)?;
    if text.is_empty() {
        return Ok(None);
    }
    variable.parse(&text).map(Some).map_err(io::Error::other)
}

/// Several lines, ended by a line holding only [`END_OF_VALUE`], re-asked until they are a
/// value of the variable's type.
fn multi_line(term: &Term, variable: &TemplateVariable) -> io::Result<Option<serde_json::Value>> {
    let form = match variable.kind() {
        onetaskgraph_core::VariableType::Text => "text",
        _ => "YAML",
    };
    loop {
        term.write_line(&format!(
            "  Enter it as {form} over as many lines as it takes, then a line holding only `{END_OF_VALUE}`:"
        ))?;
        let mut lines = Vec::new();
        loop {
            let line = term.read_line()?;
            if line == END_OF_VALUE {
                break;
            }
            lines.push(line);
        }
        let text = lines.join("\n");
        let answer = if text.is_empty() {
            empty_answer(variable)
        } else {
            variable
                .parse(&text)
                .map(Some)
                .map_err(|problem| format!("not {}: {problem}", variable.kind()))
        };
        match answer {
            Ok(value) => return Ok(value),
            Err(problem) => term.write_line(&format!("  {problem}; try again."))?,
        }
    }
}
