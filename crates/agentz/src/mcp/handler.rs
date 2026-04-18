//! The actual [`rmcp::ServerHandler`] wrapping a shared [`PluginSchemaRegistry`].

use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::transport::io::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, Json, ServerHandler, ServiceExt};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};

use super::tools::{
    DeleteSchemaRequest, ListSchemasResponse, RegisterSchemaRequest, SchemaOrgAuditResponse,
    ValidateConfigRequest, ValidateConfigResponse,
};
use agentz_core::schema::PluginSchemaRegistry;

/// MCP server exposing the plugin schema registry to agents.
#[derive(Clone)]
pub struct PluginMcpServer {
    registry: Arc<Mutex<PluginSchemaRegistry>>,
    last_audit: Arc<Mutex<Option<serde_json::Value>>>,
    tool_router: ToolRouter<Self>,
}

impl std::fmt::Debug for PluginMcpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginMcpServer").finish_non_exhaustive()
    }
}

impl PluginMcpServer {
    pub fn new(registry: Arc<Mutex<PluginSchemaRegistry>>) -> Self {
        Self {
            registry,
            last_audit: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    /// Update the cached schema.org audit graph (e.g. the last `InstallReport.schema_org_json_ld`).
    pub fn set_audit(&self, graph: serde_json::Value) {
        *self.last_audit.lock().expect("audit lock") = Some(graph);
    }

    /// Serve over a generic async read/write pair. Use this to expose the server over a
    /// Zenoh duplex, a unix pipe, or any other [`AsyncRead`] + [`AsyncWrite`] transport.
    /// Serve over a generic async read/write pair. Use this to expose the server over a
    /// Zenoh duplex, a unix pipe, or any other [`AsyncRead`] + [`AsyncWrite`] transport.
    pub async fn serve_transport<R, W>(
        self,
        read: R,
        write: W,
    ) -> Result<(), rmcp::RmcpError>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let service = ServiceExt::serve(self, (read, write)).await?;
        service.waiting().await?;
        Ok(())
    }
}

#[tool_router]
impl PluginMcpServer {
    #[tool(description = "List plugin schemas currently registered with the server.")]
    async fn plugins_list_schemas(&self) -> Result<Json<ListSchemasResponse>, ErrorData> {
        let reg = self.registry.lock().expect("registry");
        let entries = reg.entries().cloned().collect();
        Ok(Json(ListSchemasResponse { entries }))
    }

    #[tool(description = "Register or replace a plugin schema entry.")]
    async fn plugins_register_schema(
        &self,
        Parameters(req): Parameters<RegisterSchemaRequest>,
    ) -> Result<Json<serde_json::Value>, ErrorData> {
        let mut reg = self.registry.lock().expect("registry");
        if !req.replace_existing && reg.get(&req.entry.id).is_some() {
            return Err(ErrorData::invalid_params(
                format!("plugin schema `{}` already exists", req.entry.id),
                None,
            ));
        }
        let id = req.entry.id.clone();
        reg.register(req.entry);
        Ok(Json(json!({ "ok": true, "id": id })))
    }

    #[tool(description = "Delete a plugin schema entry by id.")]
    async fn plugins_delete_schema(
        &self,
        Parameters(req): Parameters<DeleteSchemaRequest>,
    ) -> Result<Json<serde_json::Value>, ErrorData> {
        let mut reg = self.registry.lock().expect("registry");
        let removed = reg.remove(&req.id).is_some();
        Ok(Json(json!({ "ok": true, "removed": removed })))
    }

    #[tool(
        description = "Validate a JSON config blob against the plugin schema registered for `id`."
    )]
    async fn plugins_validate_config(
        &self,
        Parameters(req): Parameters<ValidateConfigRequest>,
    ) -> Result<Json<ValidateConfigResponse>, ErrorData> {
        let reg = self.registry.lock().expect("registry");
        match reg.validate_config_value(&req.id, &req.value) {
            Ok(()) => Ok(Json(ValidateConfigResponse {
                ok: true,
                errors: Vec::new(),
            })),
            Err(err) => Ok(Json(ValidateConfigResponse {
                ok: false,
                errors: vec![err.to_string()],
            })),
        }
    }

    #[tool(description = "Return the last captured schema.org install audit graph (JSON-LD).")]
    async fn audit_schema_org(&self) -> Result<Json<SchemaOrgAuditResponse>, ErrorData> {
        let graph = self
            .last_audit
            .lock()
            .expect("audit")
            .clone()
            .unwrap_or(serde_json::Value::Null);
        Ok(Json(SchemaOrgAuditResponse { graph }))
    }
}

#[tool_handler]
impl ServerHandler for PluginMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "agents_unified plugin MCP server — author, register, validate, and audit plugin \
                 schemas. Every tool uses schemars-generated JSON Schema and validation via the \
                 `jsonschema` crate so agents can trust the shapes they see."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Convenience: run the server on the process's stdio.
pub async fn run_stdio(server: PluginMcpServer) -> Result<(), rmcp::RmcpError> {
    let (read, write) = stdio();
    server.serve_transport(read, write).await
}
