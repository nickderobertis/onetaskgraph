//! Transport-only creation accounting, shared by both live entry paths.
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
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
            journey: Arc::new(Mutex::new("bootstrap")),
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
        *self.residue.lock().unwrap() = None;
    }
    pub fn count(&self) -> usize {
        self.operations.lock().unwrap().len()
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
                exe.parent()
                    .expect("deps")
                    .parent()
                    .expect("profile")
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
