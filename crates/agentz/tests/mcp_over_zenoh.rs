#![cfg(all(feature = "mcp", feature = "zenoh-bus"))]
//! Run the plugin MCP server over a [`ZenohDuplex`] and call a tool from the client side over
//! a second duplex whose keys are swapped. Proves that MCP-over-Zenoh works end-to-end without
//! any stdio, sockets, or extra glue.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentz::mcp::PluginMcpServer;
use agentz::zenoh_bus::{open_local_peer, ZenohBus};
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_tools_roundtrip_over_zenoh_duplex() {
    // Single in-process peer suffices: it routes to itself.
    let session = open_local_peer().await.unwrap();
    let bus = ZenohBus::new(session);

    let server_inbound = "cyberdyne/ws/demo/mcp/c2s";
    let server_outbound = "cyberdyne/ws/demo/mcp/s2c";

    let server_duplex = bus.duplex(server_inbound, server_outbound).await.unwrap();
    let client_duplex = bus.duplex(server_outbound, server_inbound).await.unwrap();

    let registry = Arc::new(Mutex::new(PluginSchemaRegistry::new()));
    let server = PluginMcpServer::new(Arc::clone(&registry));
    server.set_audit(json!({ "@context": "https://schema.org", "@graph": [] }));

    let (server_read, server_write) = tokio::io::split(server_duplex);
    let (client_read, client_write) = tokio::io::split(client_duplex);

    let server_task = tokio::spawn(async move {
        server.serve_transport(server_read, server_write).await.unwrap();
    });

    // Give zenoh a tick to propagate subscriptions so the client's initialize lands.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = DummyClient
        .serve((client_read, client_write))
        .await
        .unwrap();

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

    drop(client);
    let _ = tokio::time::timeout(Duration::from_secs(2), server_task).await;

    let reg = registry.lock().unwrap();
    assert!(reg.get("echo").is_some());
}
