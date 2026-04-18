#![cfg(feature = "mcp")]

use std::sync::{Arc, Mutex};

use agentz::mcp::PluginMcpServer;
use agentz_core::schema::PluginSchemaRegistry;
use rmcp::model::{CallToolRequestParam, ClientInfo};
use rmcp::object;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::json;

#[derive(Clone)]
struct DummyClient;

impl ClientHandler for DummyClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_registers_and_validates_plugin_schemas() {
    let registry = Arc::new(Mutex::new(PluginSchemaRegistry::new()));
    let server = PluginMcpServer::new(Arc::clone(&registry));
    server.set_audit(json!({ "@context": "https://schema.org", "@graph": [] }));

    let (client_transport, server_transport) = tokio::io::duplex(4 * 1024);
    let (client_read, client_write) = tokio::io::split(client_transport);
    let (server_read, server_write) = tokio::io::split(server_transport);

    let server_task = tokio::spawn(async move {
        server
            .serve_transport(server_read, server_write)
            .await
            .unwrap();
    });

    let client = DummyClient
        .serve((client_read, client_write))
        .await
        .unwrap();

    let list_initial = client
        .call_tool(CallToolRequestParam {
            name: "plugins_list_schemas".into(),
            arguments: None,
        })
        .await
        .unwrap();
    let entries = list_initial
        .structured_content
        .as_ref()
        .expect("structured");
    assert_eq!(entries["entries"].as_array().unwrap().len(), 0);

    let register = client
        .call_tool(CallToolRequestParam {
            name: "plugins_register_schema".into(),
            arguments: Some(object!({
                "entry": {
                    "id": "echo",
                    "kind": "linker",
                    "description": "echo linker",
                    "schema": {
                        "type": "object",
                        "required": ["level"],
                        "properties": {
                            "level": { "type": "integer", "minimum": 1, "maximum": 3 }
                        },
                        "additionalProperties": false
                    }
                },
                "replace_existing": true
            })),
        })
        .await
        .unwrap();
    assert_eq!(
        register.structured_content.as_ref().unwrap()["ok"],
        json!(true)
    );

    let ok = client
        .call_tool(CallToolRequestParam {
            name: "plugins_validate_config".into(),
            arguments: Some(object!({ "id": "echo", "value": { "level": 2 } })),
        })
        .await
        .unwrap();
    assert_eq!(
        ok.structured_content.as_ref().unwrap()["ok"],
        json!(true)
    );

    let bad = client
        .call_tool(CallToolRequestParam {
            name: "plugins_validate_config".into(),
            arguments: Some(object!({ "id": "echo", "value": { "level": 99 } })),
        })
        .await
        .unwrap();
    assert_eq!(
        bad.structured_content.as_ref().unwrap()["ok"],
        json!(false)
    );

    let audit = client
        .call_tool(CallToolRequestParam {
            name: "audit_schema_org".into(),
            arguments: None,
        })
        .await
        .unwrap();
    assert_eq!(
        audit.structured_content.as_ref().unwrap()["graph"]["@context"],
        json!("https://schema.org")
    );

    drop(client);
    let _ = server_task.await;

    let reg = registry.lock().unwrap();
    assert!(reg.get("echo").is_some());
}
