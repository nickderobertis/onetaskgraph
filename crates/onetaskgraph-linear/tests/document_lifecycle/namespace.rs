//! The one run's namespace in the nominated Linear team, and the guard that holds a live
//! lane's traffic to it.
//!
//! Every live-test identity in the team says whose it is in its title: a standing pool
//! member names its member, a lease names its member and holder, a disposable artifact names
//! the stamp of the run that wrote it (`live_artifact`, `cleanup`). So which records belong to
//! *another* run is readable off what Linear answers, and this guard learns it from every
//! answer it relays. A request that then names one of those records — by id, as the target of
//! a write or a read, or by writing a title into another run's identity — is refused before
//! it is sent, and the refusal is recorded so a test can assert on it. Ids this guard has not
//! learned are not refused: a record that carries no live-test identity is not another run's.
use onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX;
use onetaskgraph_linear::live_artifact::{LeaseHolder, classify_lease, classify_retained};
use onetaskgraph_live::artifact::{Run, Stamp};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Mutex;

/// Whose namespace this is: a run, and the pool member it leased (`None` before the lease
/// is held, and for a local set, which its run's stamp already identifies).
pub struct Namespace {
    run: Run,
    team: String,
    member: Mutex<Option<u8>>,
    foreign: Mutex<BTreeSet<String>>,
    refused: Mutex<Vec<String>>,
}

impl Namespace {
    pub fn new(run: Run, team: &str) -> Self {
        Self {
            run,
            team: team.to_owned(),
            member: Mutex::new(None),
            foreign: Mutex::default(),
            refused: Mutex::default(),
        }
    }

    /// The pool member this run leased: from here on, every other member is foreign.
    pub fn leased(&self, index: u8) {
        *self.member.lock().unwrap() = Some(index);
    }

    /// Whether `title` is a live-test identity of some run other than this one.
    pub fn is_foreign_title(&self, title: &str) -> bool {
        let Some(suffix) = title.strip_prefix(LIVE_ARTIFACT_TITLE_PREFIX) else {
            return false;
        };
        let member = *self.member.lock().unwrap();
        if let Some(retained) = classify_retained(title) {
            if !retained.scope.team().eq_ignore_ascii_case(&self.team) {
                return true;
            }
            return match (retained.scope.local_stamp(), retained.scope.pool_index()) {
                (Some(stamp), _) => !self.owns_stamp(stamp),
                // The pre-pool standing set is nobody's to use any more.
                (None, None) => true,
                (None, Some(index)) => member != Some(index),
            };
        }
        if let Some(lease) = classify_lease(title) {
            return match lease.holder() {
                LeaseHolder::Held(stamp) => !self.owns_stamp(stamp.as_str()),
                LeaseHolder::Released => member != Some(lease.index()),
            };
        }
        // A disposable artifact: its stamp is the token right after the prefix.
        let stamp = suffix.split(' ').next().unwrap_or_default();
        !self.owns_stamp(stamp)
    }

    fn owns_stamp(&self, stamp: &str) -> bool {
        Stamp::read(stamp).is_some_and(|stamp| stamp.run() == self.run)
    }

    /// Records every record `response` names that is another run's.
    pub fn learn(&self, response: &Value) {
        let mut found = Vec::new();
        collect_foreign(self, response, &mut found);
        self.foreign.lock().unwrap().extend(found);
    }

    /// Refuses `request` when it names another run's record, recording why.
    pub fn admit(&self, request: &Value) -> Result<(), String> {
        let variables = &request["variables"];
        let mut named = Vec::new();
        ids_named(variables, &mut named);
        let foreign = self.foreign.lock().unwrap();
        let problem = named
            .iter()
            .find(|id| foreign.contains(*id))
            .map(|id| format!("names {id}, a record of another run's namespace"))
            .or_else(|| {
                titles_written(variables)
                    .into_iter()
                    .find(|title| self.is_foreign_title(title))
                    .map(|title| format!("writes {title:?}, another run's identity"))
            });
        drop(foreign);
        match problem {
            None => Ok(()),
            Some(problem) => {
                let refusal = format!("refused before sending: this live run's request {problem}");
                self.refused.lock().unwrap().push(refusal.clone());
                Err(refusal)
            }
        }
    }

    /// Every refusal so far.
    pub fn refusals(&self) -> Vec<String> {
        self.refused.lock().unwrap().clone()
    }
}

fn collect_foreign(namespace: &Namespace, value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            let title = object
                .get("title")
                .or_else(|| object.get("name"))
                .and_then(Value::as_str);
            if let (Some(id), Some(title)) = (object.get("id").and_then(Value::as_str), title)
                && namespace.is_foreign_title(title)
            {
                found.push(id.to_owned());
            }
            for child in object.values() {
                collect_foreign(namespace, child, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_foreign(namespace, item, found);
            }
        }
        _ => {}
    }
}

/// Every string a request's variables carry under a key that names a record: `id`, and every
/// `…Id` or `…Ids`, nested inputs and filters included.
fn ids_named(value: &Value, named: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let names_record = key == "id" || key.ends_with("Id") || key.ends_with("Ids");
                match child {
                    Value::String(id) if names_record => named.push(id.clone()),
                    Value::Array(items) if names_record => {
                        named.extend(items.iter().filter_map(Value::as_str).map(str::to_owned))
                    }
                    _ => ids_named(child, named),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                ids_named(item, named);
            }
        }
        _ => {}
    }
}

/// The titles and names a request's input would write.
fn titles_written(variables: &Value) -> Vec<&str> {
    ["title", "name"]
        .iter()
        .filter_map(|key| {
            variables
                .pointer(&format!("/input/{key}"))
                .and_then(Value::as_str)
        })
        .collect()
}
