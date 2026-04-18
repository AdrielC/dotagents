//! **Plugin schemas** (pure types + registry).
//!
//! Plugins author their config type with `#[derive(JsonSchema)]` from [`schemars`], and this
//! module stores the generated JSON Schema and exposes a registry that validates arbitrary JSON
//! against it via the [`jsonschema`] crate. No IO lives here — the IO crate wires this type into
//! MCP tools, config loaders, and the like.

use std::collections::BTreeMap;

use jsonschema::Validator;
use schemars::{schema_for, JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginSchemaEntry {
    pub id: String,

    /// Optional [schema.org](https://schema.org) `@type` IRI for this plugin's domain role.
    #[serde(default, rename = "schemaOrgType", skip_serializing_if = "Option::is_none")]
    pub schema_org_type: Option<String>,

    /// Logical kind: `linker`, `transform`, `validator`, or a custom string.
    #[serde(default)]
    pub kind: String,

    #[serde(default)]
    pub description: String,

    /// Optional `$schema` URI on the instance document.
    #[serde(default, rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema_uri: Option<String>,

    /// JSON Schema (draft 2020-12). Stored loose so authored-by-agent and authored-by-schemars
    /// entries share one shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
}

impl PluginSchemaEntry {
    /// Build an entry whose `schema` is derived from a Rust type's [`JsonSchema`] impl.
    pub fn from_type<T: JsonSchema>(
        id: impl Into<String>,
        kind: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let schema: Schema = schema_for!(T);
        Self {
            id: id.into(),
            schema_org_type: None,
            kind: kind.into(),
            description: description.into(),
            schema_uri: None,
            schema: Some(serde_json::to_value(&schema).unwrap_or(serde_json::Value::Null)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginsSection {
    #[serde(default)]
    pub schemas: Vec<PluginSchemaEntry>,
    #[serde(default)]
    pub config: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("unknown plugin id for validation: {0}")]
    UnknownPlugin(String),
    #[error("plugin {id}: config kind mismatch (expected {expected}, got {got})")]
    KindMismatch { id: String, expected: String, got: String },
    #[error("plugin {id}: {errors:?}")]
    Validation { id: String, errors: Vec<String> },
    #[error("invalid plugin schema for `{id}`: {message}")]
    InvalidSchema { id: String, message: String },
}

#[derive(Clone, Debug, Default)]
pub struct PluginSchemaRegistry {
    by_id: BTreeMap<String, PluginSchemaEntry>,
}

impl PluginSchemaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, entry: PluginSchemaEntry) {
        self.by_id.insert(entry.id.clone(), entry);
    }

    pub fn remove(&mut self, id: &str) -> Option<PluginSchemaEntry> {
        self.by_id.remove(id)
    }

    pub fn merge_from_config(&mut self, section: &PluginsSection) {
        for e in &section.schemas {
            self.register(e.clone());
        }
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.by_id.keys().map(|s| s.as_str())
    }

    pub fn entries(&self) -> impl Iterator<Item = &PluginSchemaEntry> {
        self.by_id.values()
    }

    pub fn get(&self, id: &str) -> Option<&PluginSchemaEntry> {
        self.by_id.get(id)
    }

    pub fn validate_all_configs(&self, section: &PluginsSection) -> Result<(), SchemaError> {
        for (id, value) in &section.config {
            self.validate_config_value(id, value)?;
        }
        Ok(())
    }

    pub fn validate_config_value(
        &self,
        id: &str,
        value: &serde_json::Value,
    ) -> Result<(), SchemaError> {
        let Some(entry) = self.by_id.get(id) else {
            return Ok(());
        };
        let Some(schema_json) = &entry.schema else {
            return Ok(());
        };
        validate_against_json_schema(id, schema_json, value)
    }

    pub fn validate_plugin_payload(
        &self,
        id: &str,
        expected_kind: &str,
        value: &serde_json::Value,
    ) -> Result<(), SchemaError> {
        let Some(entry) = self.by_id.get(id) else {
            return Err(SchemaError::UnknownPlugin(id.to_string()));
        };
        if !entry.kind.is_empty() && entry.kind != expected_kind {
            return Err(SchemaError::KindMismatch {
                id: id.to_string(),
                expected: entry.kind.clone(),
                got: expected_kind.to_string(),
            });
        }
        if let Some(schema_json) = &entry.schema {
            validate_against_json_schema(id, schema_json, value)?;
        }
        Ok(())
    }
}

fn validate_against_json_schema(
    id: &str,
    schema_json: &serde_json::Value,
    value: &serde_json::Value,
) -> Result<(), SchemaError> {
    let validator = Validator::new(schema_json).map_err(|e| SchemaError::InvalidSchema {
        id: id.to_string(),
        message: e.to_string(),
    })?;
    let errors: Vec<String> = validator.iter_errors(value).map(|e| e.to_string()).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(SchemaError::Validation { id: id.to_string(), errors })
    }
}
