//! MCP tool request/response types. Kept in a non-rmcp module so they can be reused in tests
//! without the macro-generated code and so that their JSON Schemas are produced by `schemars`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use agentz_core::schema::PluginSchemaEntry;

/// `plugins.register_schema` — author/replace a plugin schema at runtime.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RegisterSchemaRequest {
    /// Full schema entry. The `schema` field is an inline JSON Schema (2020-12).
    pub entry: PluginSchemaEntry,
    /// When true, replace an existing entry with the same `id` (default).
    #[serde(default = "default_true")]
    pub replace_existing: bool,
}

fn default_true() -> bool {
    true
}

/// `plugins.list_schemas` response.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListSchemasResponse {
    pub entries: Vec<PluginSchemaEntry>,
}

/// `plugins.delete_schema` request.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeleteSchemaRequest {
    pub id: String,
}

/// `plugins.validate_config` request — check one config blob against a registered schema.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ValidateConfigRequest {
    pub id: String,
    pub value: serde_json::Value,
}

/// `plugins.validate_config` response.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ValidateConfigResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// `audit.schema_org` response — the schema.org `@graph` for the last install.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SchemaOrgAuditResponse {
    pub graph: serde_json::Value,
}
