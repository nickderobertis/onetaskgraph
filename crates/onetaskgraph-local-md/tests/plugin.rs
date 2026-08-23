//! The factory this crate ships ahead of its source.
//!
//! Driven through the public trait, the way the registry drives it.

use onetaskgraph_plugin_api::{SecretResolver, SourceError, SourceName, SourcePlugin};
use secrecy::SecretString;

/// This plugin refuses before it reads a credential, so nothing defines one.
struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

#[test]
fn build_refuses_with_a_configuration_error_naming_the_plugin() {
    // Shipping the factory ahead of the source is what lets the registry name
    // `local-md` from the first commit; the refusal has to say plainly that the
    // source is the missing part, not the configuration.
    let name = SourceName::new("work").expect("a valid source name");

    let Err(error) = onetaskgraph_local_md::Plugin.build(&name, &serde_json::json!({}), &NoSecrets)
    else {
        panic!("the `local-md` plugin is not implemented yet, so build must refuse");
    };

    let SourceError::Config { message } = &error else {
        panic!("a plugin that cannot be built yet reports a configuration error, got {error:?}");
    };
    assert!(message.contains("local-md"), "{message}");
    assert!(message.contains("not implemented yet"), "{message}");
    assert!(
        message.contains("work"),
        "the refusal names the source: {message}"
    );
}

#[test]
fn the_plugin_reports_its_kind_and_a_config_schema_the_registry_can_publish() {
    let plugin = onetaskgraph_local_md::Plugin;
    assert_eq!(plugin.kind(), "local-md");
    assert_eq!(plugin.kind(), onetaskgraph_local_md::KIND);
    // The schema is what `onetaskgraph schema` publishes for this plugin's `config:`
    // block. Empty is not the same as permissive: a configuration written against a
    // shape this plugin does not have yet must be refused at load, not ignored.
    let schema = serde_json::to_value(plugin.config_schema()).expect("the schema renders");
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["additionalProperties"],
        serde_json::Value::Bool(false),
        "an unknown config field must be refused, not silently dropped: {schema}"
    );
}
