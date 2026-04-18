use std::sync::{Arc, Mutex};

use agents_unified::plugins::{InstallContext, PluginRegistry, ProjectLinker};
use agents_unified::schema::{
    plugins_section_from_config, PluginSchemaEntry, PluginSchemaRegistry, PluginsSection,
};
use agents_unified::{read_config, write_config, AgentsConfig, PlannedLink};
use serde_json::json;
use tempfile::tempdir;

struct EchoLinker {
    last: Arc<Mutex<Option<serde_json::Value>>>,
}

impl ProjectLinker for EchoLinker {
    fn id(&self) -> &'static str {
        "echo"
    }

    fn plan(&self, _ctx: &InstallContext<'_>) -> Vec<PlannedLink> {
        Vec::new()
    }

    fn configure(&mut self, config: serde_json::Value) {
        *self.last.lock().expect("lock") = Some(config);
    }
}

#[test]
fn registry_validates_config_against_inline_schema() {
    let mut reg = PluginSchemaRegistry::new();
    reg.register(PluginSchemaEntry {
        id: "echo".into(),
        schema_org_type: None,
        kind: "linker".into(),
        description: String::new(),
        schema_uri: None,
        schema: Some(json!({
            "type": "object",
            "required": ["level"],
            "properties": {
                "level": { "type": "integer", "minimum": 1, "maximum": 3 }
            },
            "additionalProperties": false
        })),
    });

    let mut section = PluginsSection::default();
    section.config.insert("echo".into(), json!({ "level": 2 }));
    reg.validate_all_configs(&section).unwrap();

    section.config.insert("echo".into(), json!({ "level": 9 }));
    assert!(reg.validate_all_configs(&section).is_err());
}

#[test]
fn plugin_registry_sync_passes_validated_config_to_linker() {
    let dir = tempdir().unwrap();
    let agents = dir.path().join("agents");
    std::fs::create_dir_all(&agents).unwrap();

    let mut cfg = AgentsConfig::default();
    cfg.extra.insert(
        "plugins".into(),
        json!({
            "schemas": [{
                "id": "echo",
                "kind": "linker",
                "schema": {
                    "type": "object",
                    "required": ["msg"],
                    "properties": { "msg": { "type": "string", "minLength": 1 } },
                    "additionalProperties": false
                }
            }],
            "config": {
                "echo": { "msg": "hi" }
            }
        }),
    );
    write_config(&agents.join("config.json"), &cfg).unwrap();

    let loaded = read_config(&agents.join("config.json")).unwrap();
    let section = plugins_section_from_config(&loaded);

    let last = Arc::new(Mutex::new(None));
    let mut reg = PluginRegistry::new();
    reg.register(Box::new(EchoLinker {
        last: Arc::clone(&last),
    }));
    reg.sync_from_agents_config(&section).unwrap();
    assert_eq!(*last.lock().unwrap(), Some(json!({ "msg": "hi" })));

    let mut bad = section;
    bad.config.insert("echo".into(), json!({ "msg": "" }));
    let mut reg2 = PluginRegistry::new();
    reg2.register(Box::new(EchoLinker {
        last: Arc::new(Mutex::new(None)),
    }));
    assert!(reg2.sync_from_agents_config(&bad).is_err());
}
