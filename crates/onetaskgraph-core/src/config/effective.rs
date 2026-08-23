//! The configuration as it ended up, with the layer each setting came from.
//!
//! This is what makes precedence something a user can see rather than something the
//! tests alone know. It is built from the merge itself, not reconstructed from the
//! final [`Config`], so it cannot claim a layer the merge did not actually take the
//! value from.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::secrets::SecretsReport;

use super::Config;
use super::layer::{Origin, Setting, SettingPath};

/// Every setting this build reads, with its value and where the value came from.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct EffectiveConfig {
    /// Every setting, in key order.
    pub settings: Vec<Setting>,
    /// What the credentials file supplied, by name, and which layer answers for each.
    ///
    /// Beside the settings because it is the same question — where does this value
    /// come from — asked of the one kind of value that may never be printed. So the
    /// name and the layer are reported and the value never is.
    pub secrets: SecretsReport,
}

impl EffectiveConfig {
    /// Combine what the layers set with the built-in values for what they did not.
    ///
    /// A setting nothing set still appears, carrying [`Origin::Default`] and the value
    /// the run will actually use — the point of the verb is to answer "what is this
    /// command going to do", and a silent omission answers it wrongly.
    #[must_use]
    pub fn new(
        merged: &BTreeMap<SettingPath, Setting>,
        config: &Config,
        secrets: SecretsReport,
    ) -> Self {
        let mut settings: Vec<Setting> = merged.values().cloned().collect();

        for (key, value) in [
            ("page_size", Value::from(config.page_size.get())),
            (
                "output",
                serde_json::to_value(config.output).expect("an output format renders as JSON"),
            ),
            (
                "default_sources",
                serde_json::to_value(config.selected_sources())
                    .expect("source names render as JSON"),
            ),
        ] {
            let key = SettingPath::parse(key).expect("a literal path with no empty segment");
            if !settings.iter().any(|setting| setting.key == key) {
                settings.push(Setting {
                    key,
                    value,
                    origin: Origin::Default,
                });
            }
        }

        settings.sort_by(|left, right| left.key.cmp(&right.key));
        Self { settings, secrets }
    }

    /// The table a person reads, one setting per line.
    ///
    /// Values render as compact JSON rather than bare: this table's whole job is to
    /// answer what a setting *is*, and a bare `50` beside a bare `"50"` would hide the
    /// difference between a number a document set and a string an environment variable
    /// spelled.
    #[must_use]
    pub fn render_text(&self) -> String {
        let key_width = self
            .settings
            .iter()
            .map(|setting| setting.key.to_string().chars().count())
            .max()
            .unwrap_or(0);
        let values: Vec<String> = self
            .settings
            .iter()
            .map(|setting| render_value(&setting.value))
            .collect();
        let value_width = values
            .iter()
            .map(|value| value.chars().count())
            .max()
            .unwrap_or(0);

        let mut rendered = String::new();
        for (setting, value) in self.settings.iter().zip(values) {
            rendered.push_str(&format!(
                "{:key_width$}  {:value_width$}  {}\n",
                setting.key.to_string(),
                value,
                setting.origin
            ));
        }

        rendered.push_str(&match self.secrets.path.as_ref() {
            Some(path) => format!("\nsecrets file  {}\n", path.display()),
            None => "\nsecrets file  none — neither XDG_CONFIG_HOME nor HOME is set\n".to_owned(),
        });
        if self.secrets.variables.is_empty() {
            rendered.push_str("  (it defines no variables, or is not there)\n");
        }
        for credential in &self.secrets.variables {
            rendered.push_str(&format!(
                "  {}  resolved from the {}\n",
                credential.variable, credential.resolved_from
            ));
        }
        rendered
    }
}

/// One value as compact JSON.
fn render_value(value: &Value) -> String {
    serde_json::to_string(value).expect("a value that was deserialized from JSON re-renders")
}
