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
    // `linear` from the first commit; the refusal has to say plainly that the
    // source is the missing part, not the configuration.
    let name = SourceName::new("work").expect("a valid source name");

    let Err(error) = onetaskgraph_linear::Plugin.build(&name, &serde_json::json!({}), &NoSecrets)
    else {
        panic!("the `linear` plugin is not implemented yet, so build must refuse");
    };

    let SourceError::Config { message } = &error else {
        panic!("a plugin that cannot be built yet reports a configuration error, got {error:?}");
    };
    assert!(message.contains("linear"), "{message}");
    assert!(message.contains("not implemented yet"), "{message}");
    assert!(
        message.contains("work"),
        "the refusal names the source: {message}"
    );
}

#[test]
fn the_registry_can_name_this_plugin_and_read_its_config_schema() {
    let plugin = onetaskgraph_linear::Plugin;
    assert_eq!(plugin.kind(), "linear");
    assert_eq!(plugin.kind(), onetaskgraph_linear::KIND);
    // The schema is what `onetaskgraph schema` publishes for this plugin's
    // `config:` block, so it has to be a real document even while empty.
    assert!(plugin.config_schema().as_value().is_object());
}
