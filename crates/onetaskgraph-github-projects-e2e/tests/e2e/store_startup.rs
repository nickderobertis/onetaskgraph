//! What one store process costs before it does a command's work, measured from outside it.
//!
//! The commands a host runs around a plan copy and an approval — `project show`, a
//! project-scoped `task list` and `document list`, `task deps` and the `project copy` itself —
//! are each a whole process of the real binary. This harness times those processes and nothing
//! inside them, and attributes each command's wall time by difference between invocations it
//! controls:
//!
//! - **process start** is a `--version`, which reads no configuration and builds no source;
//! - **configuration and source initialisation** is the command over [`BARE`], a folder holding
//!   the fewest records the command can succeed on — one project, one task filed under it, one
//!   document, none with content or edges — less process start. It is taken once with
//!   [`UNRELATED`] configured beside it and once without, and the difference is what configuring
//!   sources a command does not touch costs it;
//! - **plan loading** is the command over the generated 10-item plan of [`plan_copy`], less the
//!   same command over [`BARE`].
//!
//! [`UNRELATED`] are a GitHub Projects board and a Linear workspace, each a loopback double that
//! answers every request 503 and counts what it was sent. Configured beside the plan with a
//! credential each, they are what a host's other boards and trackers are to a command reading a
//! folder: [`unrelated_hosted_sources_are_never_contacted`] holds every read to sending them
//! nothing, and the copy to sending nothing anywhere but the board it copies to.
//!
//! No assertion here reads a clock. The figures are printed for a reader, and the comparison of
//! two builds — [`compare_store_startup`] — is run by hand, because what it measures is this
//! host's and not the code's.

use super::plan_copy::{Plan, SMALL};
use onetaskgraph_e2e_support::{
    binary,
    common::{assert_spawnable_on_windows, stderr, stdout},
    loopback::{After, serve_with},
};
use serde_json::{Value, json};
use std::{
    io::Write as _,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// The folder holding the fewest records each command can succeed on.
const BARE: &str = "bare";

/// The two hosted sources configured beside the plan and never named by a command.
const UNRELATED: [&str; 2] = ["unrelated-board", "unrelated-tracker"];

/// The credentials those two read, set on every process so both build as a host's would.
const UNRELATED_CREDENTIALS: [(&str, &str); 2] = [
    ("SYNTHETIC_UNRELATED_BOARD_TOKEN", "synthetic-board-token"),
    ("SYNTHETIC_UNRELATED_TRACKER_KEY", "synthetic-tracker-key"),
];

/// The commands a host runs around a plan copy, each over the folder `source` names, by the
/// name the report gives it.
fn commands(source: &str) -> Vec<(&'static str, Vec<String>)> {
    // The plan's last task carries two of its ten edges; the bare folder's one task none.
    let task = if source == BARE { "T-1" } else { "T-8" };
    let project = format!("{source}:P");
    [
        ("project show", vec!["project", "show", &project]),
        (
            "task list --project",
            vec!["task", "list", "--project", &project],
        ),
        (
            "task deps",
            vec!["task", "deps", &format!("{source}:{task}")],
        ),
        (
            "document list --project",
            vec!["document", "list", "--project", &project],
        ),
        (
            "project copy",
            vec!["project", "copy", &project, "--to", "board"],
        ),
    ]
    .into_iter()
    .map(|(name, arguments)| {
        let mut arguments: Vec<String> = arguments.into_iter().map(str::to_owned).collect();
        arguments.push("--json".to_owned());
        (name, arguments)
    })
    .collect()
}

/// A hosted source's loopback double: every request answered 503, and counted.
struct Double {
    endpoint: String,
    requests: Arc<AtomicUsize>,
}

impl Double {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&requests);
        serve_with(listener, move |_, stream| {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ =
                stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n");
            After::KeepOpen
        });
        Self { endpoint, requests }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// Which remote sources one process sent anything to, by the name the configuration gives them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Contacted(Vec<&'static str>);

/// The plan beside the bare folder and the board it is copied to, with the two unrelated
/// sources configured or not.
struct Host {
    plan: Plan,
    unrelated: [Double; 2],
    /// The configuration with every source, and with the two unrelated ones left out.
    with_unrelated: String,
    without_unrelated: String,
}

impl Host {
    fn new() -> Self {
        let plan = Plan::new(SMALL, Vec::new());
        let bare = plan.sandbox.subdirectory(BARE);
        for directory in ["projects", "tasks", "documents"] {
            std::fs::create_dir_all(bare.join(directory)).unwrap();
        }
        let bare = std::fs::canonicalize(bare).unwrap();
        for (path, text) in [
            (
                "projects/P.md",
                "---\ntitle: A bare project\nstatus: todo\n---\n",
            ),
            (
                "tasks/T-1.md",
                "---\ntitle: A bare task\nstatus: todo\nproject: P\n---\n",
            ),
            (
                "documents/D.md",
                "---\ntitle: A bare document\nproject: P\n---\n",
            ),
        ] {
            std::fs::write(bare.join(path), text).unwrap();
        }
        let written = plan.sandbox.project().join("onetaskgraph.yaml");
        let mut document: Value =
            serde_json::from_str(&std::fs::read_to_string(&written).unwrap()).unwrap();
        document["sources"][BARE] = json!({"plugin":"local-md","config":{"root":bare}});
        let without_unrelated = serde_json::to_string_pretty(&document).unwrap();
        let unrelated = [Double::start(), Double::start()];
        document["sources"][UNRELATED[0]] = json!({"plugin":"github-projects","config":{
            "owner":"synthetic-owner","project_number":3,"repository":"synthco/public-widgets",
            "token_env":UNRELATED_CREDENTIALS[0].0,"endpoint":unrelated[0].endpoint}});
        document["sources"][UNRELATED[1]] = json!({"plugin":"linear","config":{
            "api_key_env":UNRELATED_CREDENTIALS[1].0,"team":"SYN",
            "endpoint":unrelated[1].endpoint}});
        let with_unrelated = serde_json::to_string_pretty(&document).unwrap();
        let host = Self {
            plan,
            unrelated,
            with_unrelated,
            without_unrelated,
        };
        host.configure(true);
        host
    }

    /// Write the configuration with the unrelated sources, or without them.
    fn configure(&self, unrelated: bool) {
        self.plan.sandbox.project_document(if unrelated {
            &self.with_unrelated
        } else {
            &self.without_unrelated
        });
    }

    /// One process of `executable` with `arguments`, its whole wall time, and every remote
    /// source it sent a request to.
    fn run(&self, executable: &Path, arguments: &[String]) -> (Output, Duration, Contacted) {
        let before = (
            self.plan.board.calls().len(),
            self.unrelated.each_ref().map(Double::requests),
        );
        let lease = self.plan.clock.lease(0);
        let mut command = self.plan.sandbox.subprocess(executable);
        command
            .args(arguments)
            .envs(self.plan.clock.client_env(0))
            .envs(UNRELATED_CREDENTIALS)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        assert_spawnable_on_windows(&command);
        let started = Instant::now();
        let output = command.spawn().unwrap().wait_with_output().unwrap();
        let elapsed = started.elapsed();
        lease.wait_for_detach();
        drop(lease);
        let mut contacted = Vec::new();
        if self.plan.board.calls().len() > before.0 {
            contacted.push("board");
        }
        for (index, double) in self.unrelated.iter().enumerate() {
            if double.requests() > before.1[index] {
                contacted.push(UNRELATED[index]);
            }
        }
        (output, elapsed, Contacted(contacted))
    }
}

/// What a command answered, refusing a run that did not succeed.
fn answered(output: &Output, what: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{what}\n{}\n{}",
        stdout(output),
        stderr(output)
    );
    serde_json::from_str(&stdout(output)).unwrap()
}

/// The sources a command over `source` may send to: the board, for a copy onto it, and none
/// for a read of a folder.
fn allowed(command: &str) -> Contacted {
    Contacted(if command == "project copy" {
        vec!["board"]
    } else {
        Vec::new()
    })
}

/// The variants each command is run in, by the name the report gives each: the plan with the
/// unrelated sources configured, the bare folder with them, and the bare folder without.
const VARIANTS: [(&str, &str, bool); 3] = [
    ("plan", "notes", true),
    ("bare", BARE, true),
    ("bare, no unrelated", BARE, false),
];

#[test]
fn unrelated_hosted_sources_are_never_contacted() {
    let host = Host::new();
    // The first copy of each project lands it; every copy after it is the unchanged re-copy a
    // host runs again, which is what the variants time and what is asserted below.
    for source in ["notes", BARE] {
        let (_, arguments) = commands(source).pop().unwrap();
        let (output, _, contacted) = host.run(binary(), &arguments);
        answered(&output, "the first copy");
        assert_eq!(
            contacted,
            allowed("project copy"),
            "the first copy of {source}"
        );
    }
    let mut seconds = Vec::new();
    for (variant, source, unrelated) in VARIANTS {
        host.configure(unrelated);
        for (command, arguments) in commands(source) {
            let (output, elapsed, contacted) = host.run(binary(), &arguments);
            let answer = answered(&output, &format!("{command} over {variant}"));
            assert_eq!(
                contacted,
                allowed(command),
                "{command} over {variant} sent requests to sources it does not read"
            );
            if command == "project copy" {
                let items = answer["items"].as_array().unwrap();
                assert!(
                    !items.is_empty() && items.iter().all(|item| item["action"] == "unchanged"),
                    "{command} over {variant} is not the unchanged re-copy: {answer}"
                );
            }
            seconds.push((variant, command, elapsed));
        }
    }
    // Configuring sources a command does not read changes nothing it answers.
    for (command, arguments) in commands(BARE) {
        host.configure(true);
        let with = answered(&host.run(binary(), &arguments).0, command);
        host.configure(false);
        let without = answered(&host.run(binary(), &arguments).0, command);
        assert_eq!(
            with, without,
            "{command} answers differently beside unrelated sources"
        );
    }
    let version = host.run(binary(), &["--version".to_owned()]);
    assert_eq!(version.0.status.code(), Some(0), "{}", stderr(&version.0));
    assert_eq!(version.2, Contacted::default(), "--version sent a request");
    // One sample of each, for a reader: the comparison below is what decides anything.
    let samples = |variant: &str, command: &str| {
        vec![
            seconds
                .iter()
                .find(|(v, c, _)| *v == variant && *c == command)
                .unwrap()
                .2,
        ]
    };
    for (command, _) in commands(BARE) {
        eprintln!(
            "{}",
            attribution(
                command,
                &[version.1],
                &samples("plan", command),
                &samples("bare", command),
                &samples("bare, no unrelated", command)
            )
        );
    }
}

/// The median and the interquartile range of `samples`, in milliseconds.
fn spread(samples: &[Duration]) -> (f64, f64) {
    let mut millis: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1e3).collect();
    millis.sort_by(f64::total_cmp);
    let at = |q: f64| {
        let position = q * (millis.len() - 1) as f64;
        let (low, high) = (position.floor() as usize, position.ceil() as usize);
        millis[low] + (millis[high] - millis[low]) * (position - low as f64)
    };
    (at(0.5), at(0.75) - at(0.25))
}

/// One command's wall time attributed by difference, as the module's documentation says.
fn attribution(
    command: &str,
    version: &[Duration],
    plan: &[Duration],
    bare: &[Duration],
    lean: &[Duration],
) -> String {
    let (start, _) = spread(version);
    let (plan, plan_iqr) = spread(plan);
    let (bare, _) = spread(bare);
    let (lean, _) = spread(lean);
    format!(
        "{command:<24} whole {plan:7.1} ms (IQR {plan_iqr:5.1}) = process start {start:6.1} \
         + configuration and source initialisation {:6.1} (of which unrelated sources {:6.1}) \
         + plan loading {:6.1}",
        bare - start,
        bare - lean,
        plan - bare
    )
}

/// The executable an environment variable of this test names, refused by name when unset.
fn named(variable: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(variable)
            .unwrap_or_else(|| panic!("set {variable} to the onetaskgraph executable to time")),
    );
    assert!(
        path.is_file(),
        "{variable}: {} is not a file",
        path.display()
    );
    path
}

/// The base and the finished build, each run [`ROUNDS`] times per command and variant,
/// interleaved — every round runs each invocation once under each build, alternating which goes
/// first — and reported by median, interquartile range, attribution and remote contacts.
///
/// Run by hand, naming both executables:
///
/// ```text
/// STORE_STARTUP_BASE=<base onetaskgraph> STORE_STARTUP_FINISHED=<finished onetaskgraph> \
///   cargo test -p onetaskgraph-github-projects-e2e --test e2e -- --ignored \
///   --nocapture compare_store_startup
/// ```
#[test]
#[ignore = "run by hand with STORE_STARTUP_BASE and STORE_STARTUP_FINISHED naming two builds"]
fn compare_store_startup() {
    const ROUNDS: usize = 9;
    let builds = [
        ("base", named("STORE_STARTUP_BASE")),
        ("finished", named("STORE_STARTUP_FINISHED")),
    ];
    let host = Host::new();
    for source in ["notes", BARE] {
        let (_, arguments) = commands(source).pop().unwrap();
        answered(&host.run(&builds[1].1, &arguments).0, "the first copy");
    }
    // (build, variant or "version", command) -> samples, and the contacts of every run.
    let mut samples: std::collections::BTreeMap<(usize, &str, &str), Vec<Duration>> =
        Default::default();
    let mut contacts: std::collections::BTreeMap<(usize, &str), Vec<Contacted>> =
        Default::default();
    let version = vec!["--version".to_owned()];
    for round in 0..ROUNDS {
        let order: [usize; 2] = if round % 2 == 0 { [0, 1] } else { [1, 0] };
        for build in order {
            let (output, elapsed, _) = host.run(&builds[build].1, &version);
            assert_eq!(output.status.code(), Some(0));
            samples
                .entry((build, "version", ""))
                .or_default()
                .push(elapsed);
        }
        for (variant, source, unrelated) in VARIANTS {
            host.configure(unrelated);
            for (command, arguments) in commands(source) {
                for build in order {
                    let (output, elapsed, contacted) = host.run(&builds[build].1, &arguments);
                    answered(
                        &output,
                        &format!("{} {command} over {variant}", builds[build].0),
                    );
                    samples
                        .entry((build, variant, command))
                        .or_default()
                        .push(elapsed);
                    if variant == "plan" {
                        contacts
                            .entry((build, command))
                            .or_default()
                            .push(contacted);
                    }
                }
            }
        }
    }
    let mut report = String::new();
    for (command, _) in commands(BARE) {
        let base = spread(&samples[&(0, "plan", command)]);
        let finished = spread(&samples[&(1, "plan", command)]);
        let margin = base.1.max(finished.1);
        report.push_str(&format!(
            "{command:<24} base median {:7.1} ms IQR {:5.1} | finished median {:7.1} ms IQR \
             {:5.1} | base - finished {:7.1} ms {} larger IQR {margin:5.1}\n",
            base.0,
            base.1,
            finished.0,
            finished.1,
            base.0 - finished.0,
            if base.0 - finished.0 > margin {
                ">"
            } else {
                "NOT >"
            },
        ));
    }
    for (build, (name, _)) in builds.iter().enumerate() {
        report.push_str(&format!("\n{name}: attribution by difference (medians)\n"));
        for (command, _) in commands(BARE) {
            report.push_str(&attribution(
                command,
                &samples[&(build, "version", "")],
                &samples[&(build, "plan", command)],
                &samples[&(build, "bare", command)],
                &samples[&(build, "bare, no unrelated", command)],
            ));
            report.push('\n');
        }
        report.push_str(&format!(
            "{name}: remote sources contacted per run, over the plan\n"
        ));
        for (command, _) in commands(BARE) {
            let seen = &contacts[&(build, command)];
            let mut distinct: Vec<&'static str> =
                seen.iter().flat_map(|c| c.0.iter().copied()).collect();
            distinct.sort_unstable();
            distinct.dedup();
            report.push_str(&format!(
                "{command:<24} {} of 3 configured remote sources ({distinct:?}) over {} runs\n",
                distinct.len(),
                seen.len()
            ));
        }
    }
    eprintln!("{report}");
}
