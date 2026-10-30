//! Transport-only creation accounting, shared by both live entry paths.
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
/// The journey every create made while a lane prepares its standing set is filed under:
/// in an ordinary journal it is a cold rebuild, reported beside that pass and never in it.
pub const COLD_REBUILD: &str = "cold-rebuild";
/// The variable `scripts/ci-job.sh` sets to `1` where the instrumented run stands in for a
/// live crate's `test`, so these journals are written by a build under `llvm-cov-target`.
pub const TESTS_UNDER_COVERAGE: &str = "ONETASKGRAPH_TESTS_UNDER_COVERAGE";

/// The build directory a journal belongs under: the test executable's own, or, under
/// [`TESTS_UNDER_COVERAGE`], the plain build of the same profile, which is where the budgets'
/// report runs read it. The same rule as `onetaskgraph_e2e_support::telemetry::build_directory`,
/// restated because a plugin's tests may not depend on the engine's harness;
/// `onetaskgraph-linear-e2e`'s `journal_build_directory_agrees_with_the_harness` holds the two to
/// one answer.
pub fn build_directory(
    executable: &std::path::Path,
    under_coverage: bool,
) -> Option<std::path::PathBuf> {
    let mut directory = executable.parent()?.to_path_buf();
    if directory.file_name().is_some_and(|leaf| leaf == "deps") {
        directory.pop();
    }
    if under_coverage
        && let (Some(profile), Some(instrumented)) = (directory.file_name(), directory.parent())
        && instrumented
            .file_name()
            .is_some_and(|leaf| leaf == "llvm-cov-target")
        && let Some(target) = instrumented.parent()
    {
        return Some(target.join(profile));
    }
    Some(directory)
}

#[derive(Clone)]
pub struct Journal {
    lane: &'static str,
    directory: Option<std::path::PathBuf>,
    residue: Arc<Mutex<Option<usize>>>,
    mode: Arc<Mutex<&'static str>>,
    operations: Arc<Mutex<Vec<Value>>>,
    journey: Arc<Mutex<&'static str>>,
}
impl Journal {
    pub fn new(lane: &'static str, mode: &'static str) -> Self {
        Self {
            lane,
            directory: None,
            residue: Arc::default(),
            mode: Arc::new(Mutex::new(mode)),
            operations: Arc::default(),
            journey: Arc::new(Mutex::new(COLD_REBUILD)),
        }
    }
    pub fn durable(mut self, directory: std::path::PathBuf) -> Result<Self, String> {
        self.directory = Some(directory);
        self.write_in(self.directory.as_ref().unwrap(), false)?;
        Ok(self)
    }
    pub fn observe(&self, request: &Value, response: &Value) {
        use graphql_parser::query::{Definition, OperationDefinition, Selection, parse_query};
        let Some(query) = request["query"].as_str() else {
            return;
        };
        let Ok(query) = parse_query::<String>(query) else {
            return;
        };
        let mut created = 0;
        for definition in &query.definitions {
            let Definition::Operation(OperationDefinition::Mutation(mutation)) = definition else {
                continue;
            };
            for selection in &mutation.selection_set.items {
                let Selection::Field(field) = selection else {
                    continue;
                };
                let response_key = field.alias.as_ref().unwrap_or(&field.name);
                if field.name == "documentCreate"
                    && response["data"][response_key]["success"] == true
                {
                    created += 1;
                }
            }
        }
        if created == 0 {
            return;
        }
        {
            let mut operations = self.operations.lock().unwrap();
            for _ in 0..created {
                operations.push(json!({"journey":*self.journey.lock().unwrap(),"operation":"documentCreate","success":true}));
            }
        }
        if let Some(directory) = &self.directory {
            self.write_in(directory, false)
                .expect("persist successful create before proceeding");
        }
    }
    pub fn residue(&self, documents: usize) {
        *self.residue.lock().unwrap() = Some(documents);
    }
    pub fn journey(&self, journey: &'static str) {
        *self.journey.lock().unwrap() = journey;
    }
    pub fn begin(&self, mode: &'static str) {
        *self.mode.lock().unwrap() = mode;
        self.operations.lock().unwrap().clear();
        *self.journey.lock().unwrap() = COLD_REBUILD;
        *self.residue.lock().unwrap() = None;
    }
    pub fn count(&self) -> usize {
        self.operations.lock().unwrap().len()
    }
    /// The successful creates filed under one journey.
    pub fn count_in(&self, journey: &str) -> usize {
        self.operations
            .lock()
            .unwrap()
            .iter()
            .filter(|operation| operation["journey"] == journey)
            .count()
    }
    pub fn finish(&self) -> Result<(), String> {
        if let Some(directory) = &self.directory {
            return self.finish_in(directory);
        }

        let variable = if self.lane == "plugin" {
            "LINEAR_PLUGIN_TELEMETRY_DIR"
        } else {
            "LINEAR_BUDGET_TELEMETRY_DIR"
        };
        let directory = std::env::var_os(variable)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let exe = std::env::current_exe().expect("test executable");
                let under_coverage =
                    std::env::var_os(TESTS_UNDER_COVERAGE).is_some_and(|value| value == "1");
                build_directory(&exe, under_coverage)
                    .expect("a test executable sits inside a build directory")
                    .join("telemetry")
                    .join(if self.lane == "plugin" {
                        "onetaskgraph-linear"
                    } else {
                        "onetaskgraph-linear-e2e"
                    })
            });
        self.finish_in(&directory)
    }
    pub fn finish_in(&self, directory: &std::path::Path) -> Result<(), String> {
        self.write_in(directory, true)
    }
    fn write_in(&self, directory: &std::path::Path, completed: bool) -> Result<(), String> {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let mode = *self.mode.lock().unwrap();
        let path = directory.join(format!("linear-live-documents-{mode}.json"));
        let pending = path.with_extension("pending");
        let mut body = json!({"schema_version":1,"lane":self.lane,"mode":mode,"completed":completed,"operations":*self.operations.lock().unwrap()});
        if let Some(residue) = *self.residue.lock().unwrap() {
            body["residue"] = json!(residue);
        }
        std::fs::write(&pending, body.to_string()).map_err(|e| e.to_string())?;
        std::fs::rename(pending, path).map_err(|e| e.to_string())
    }
}
