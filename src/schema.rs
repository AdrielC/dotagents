//! Declarative **plugin schemas**: register JSON Schema documents for `plugins.config.<id>`
//! so extensions stay structured and validated without pulling heavyweight deps on older toolchains.
//!
//! Validation implements a **practical subset** of JSON Schema (Draft-07 style): `type`, `properties`,
//! `required`, `items`, `enum`, `const`, `minimum` / `maximum` for numbers, `minLength` / `maxLength`
//! for strings, `additionalProperties`, and nested `allOf` / `anyOf` / `oneOf` (first matching branch
//! for `oneOf`). For full spec compliance on newer Rust versions, gate a `jsonschema` crate behind a
//! Cargo feature in your application.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::AgentsConfig;

/// Declares how a plugin uses configuration and which JSON Schema validates it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginSchemaEntry {
    /// Stable id (matches `plugins.config` key and [`crate::plugins::ProjectLinker::id`] when applicable).
    pub id: String,
    /// Logical kind: `linker`, `transform`, `validator`, or a custom string for your registry.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    /// Optional JSON Schema meta URI (`$schema` on the instance document).
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema_uri: Option<String>,
    /// Inline JSON Schema object used to validate `plugins.config.<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
}

/// Top-level `plugins` object stored in `config.json` next to dot-agents fields.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginsSection {
    /// Registered plugin schema declarations (merge into [`PluginSchemaRegistry`] at install time).
    #[serde(default)]
    pub schemas: Vec<PluginSchemaEntry>,
    /// Per-plugin JSON configuration; each value is validated when a matching schema is registered.
    #[serde(default)]
    pub config: BTreeMap<String, serde_json::Value>,
}

/// Read `plugins` from merged `AgentsConfig` (top-level `plugins` key).
pub fn plugins_section_from_config(cfg: &AgentsConfig) -> PluginsSection {
    cfg.extra
        .get("plugins")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("unknown plugin id for validation: {0}")]
    UnknownPlugin(String),
    #[error("plugin {id}: config kind mismatch (expected {expected}, got {got})")]
    KindMismatch {
        id: String,
        expected: String,
        got: String,
    },
    #[error("{0}")]
    Validation(String),
}

/// Runtime registry of plugin schemas (from config and/or code).
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

    pub fn merge_from_config(&mut self, section: &PluginsSection) {
        for e in &section.schemas {
            self.register(e.clone());
        }
    }

    pub fn get(&self, id: &str) -> Option<&PluginSchemaEntry> {
        self.by_id.get(id)
    }

    /// Validate every `plugins.config` entry that has a registered schema with a `schema` body.
    pub fn validate_all_configs(&self, section: &PluginsSection) -> Result<(), SchemaError> {
        for (id, value) in &section.config {
            self.validate_config_value(id, value)?;
        }
        Ok(())
    }

    /// Validate one config blob against the registered schema for `id` (if any).
    pub fn validate_config_value(&self, id: &str, value: &serde_json::Value) -> Result<(), SchemaError> {
        let Some(entry) = self.by_id.get(id) else {
            return Ok(());
        };
        let Some(schema) = &entry.schema else {
            return Ok(());
        };
        validate_json_subset(schema, value).map_err(|m| SchemaError::Validation(format!("{id}: {m}")))
    }

    /// Ensure `value` validates and, if the schema entry sets `kind`, that `expected_kind` matches.
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
        if let Some(schema) = &entry.schema {
            validate_json_subset(schema, value)
                .map_err(|m| SchemaError::Validation(format!("{id}: {m}")))?;
        }
        Ok(())
    }
}

fn validate_json_subset(schema: &serde_json::Value, instance: &serde_json::Value) -> Result<(), String> {
    if let Some(obj) = schema.as_object() {
        if let Some(ref_) = obj.get("$ref") {
            return Err(format!("$ref not supported in built-in validator: {ref_}"));
        }
    }
    validate_node(schema, instance)
}

fn validate_node(schema: &serde_json::Value, instance: &serde_json::Value) -> Result<(), String> {
    let Some(s) = schema.as_object() else {
        return Ok(());
    };

    if let Some(all) = s.get("allOf").and_then(|v| v.as_array()) {
        for sub in all {
            validate_node(sub, instance)?;
        }
        return Ok(());
    }
    if let Some(any) = s.get("anyOf").and_then(|v| v.as_array()) {
        if any.is_empty() {
            return Ok(());
        }
        let ok = any.iter().any(|sub| validate_node(sub, instance).is_ok());
        return if ok {
            Ok(())
        } else {
            Err("instance matched no anyOf branch".into())
        };
    }
    if let Some(one) = s.get("oneOf").and_then(|v| v.as_array()) {
        let mut matches = 0usize;
        let mut last_err = String::new();
        for sub in one {
            match validate_node(sub, instance) {
                Ok(()) => matches += 1,
                Err(e) => last_err = e,
            }
        }
        return match matches {
            1 => Ok(()),
            0 => Err(format!("oneOf: no branch matched ({last_err})")),
            _ => Err("oneOf: more than one branch matched".into()),
        };
    }

    if let Some(t) = s.get("type") {
        match t {
            serde_json::Value::String(ts) => check_type(ts, instance)?,
            serde_json::Value::Array(types) => {
                let ok = types.iter().filter_map(|v| v.as_str()).any(|ts| type_matches(ts, instance));
                if !ok {
                    return Err(format!("type mismatch: expected {types:?}, got {instance}"));
                }
            }
            _ => {}
        }
    }

    if let Some(c) = s.get("const") {
        if instance != c {
            return Err(format!("const mismatch: expected {c}, got {instance}"));
        }
    }
    if let Some(en) = s.get("enum").and_then(|v| v.as_array()) {
        if !en.iter().any(|v| v == instance) {
            return Err(format!("enum mismatch: {instance} not in {en:?}"));
        }
    }

    match instance {
        serde_json::Value::Object(map) => {
            if let Some(props) = s.get("properties").and_then(|v| v.as_object()) {
                for (k, subschema) in props {
                    if let Some(child) = map.get(k) {
                        validate_node(subschema, child)?;
                    }
                }
            }
            if let Some(req) = s.get("required").and_then(|v| v.as_array()) {
                for key in req.iter().filter_map(|v| v.as_str()) {
                    if !map.contains_key(key) {
                        return Err(format!("missing required property `{key}`"));
                    }
                }
            }
            if s.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
                if let Some(props) = s.get("properties").and_then(|v| v.as_object()) {
                    for k in map.keys() {
                        if !props.contains_key(k) {
                            return Err(format!("additional property `{k}` not allowed"));
                        }
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            if let Some(items) = s.get("items") {
                for (i, el) in arr.iter().enumerate() {
                    validate_node(items, el).map_err(|e| format!("items[{i}]: {e}"))?;
                }
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(min) = s.get("minimum").and_then(|v| v.as_f64()) {
                if n.as_f64().unwrap_or(f64::NAN) < min {
                    return Err(format!("below minimum {min}"));
                }
            }
            if let Some(max) = s.get("maximum").and_then(|v| v.as_f64()) {
                if n.as_f64().unwrap_or(f64::NAN) > max {
                    return Err(format!("above maximum {max}"));
                }
            }
        }
        serde_json::Value::String(st) => {
            if let Some(min) = s.get("minLength").and_then(|v| v.as_u64()) {
                if (st.chars().count() as u64) < min {
                    return Err(format!("string shorter than minLength {min}"));
                }
            }
            if let Some(max) = s.get("maxLength").and_then(|v| v.as_u64()) {
                if (st.chars().count() as u64) > max {
                    return Err(format!("string longer than maxLength {max}"));
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn type_matches(ts: &str, instance: &serde_json::Value) -> bool {
    match ts {
        "null" => instance.is_null(),
        "boolean" => instance.is_boolean(),
        "number" => instance.is_number(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "string" => instance.is_string(),
        "array" => instance.is_array(),
        "object" => instance.is_object(),
        _ => false,
    }
}

fn check_type(ts: &str, instance: &serde_json::Value) -> Result<(), String> {
    if type_matches(ts, instance) {
        Ok(())
    } else {
        Err(format!("type mismatch: expected `{ts}`, got {instance}"))
    }
}
