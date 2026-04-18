//! [schema.org](https://schema.org) JSON-LD builders. Pure; no IO. Works on `&Path` references
//! without touching the filesystem.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const SCHEMA_ORG: &str = "https://schema.org";

pub fn install_context() -> Value {
    json!({
        "@vocab": SCHEMA_ORG,
        "schema": SCHEMA_ORG,
        "agentz": "https://agentz.dev/ns#"
    })
}

fn digital_document_pair(source: &Path, dest: &Path, encoding_hint: &str) -> Value {
    json!({
        "@type": "DigitalDocument",
        "name": dest.file_name().and_then(|s| s.to_str()).unwrap_or("link"),
        "url": format!("file://{}", dest.display()),
        "isBasedOn": {
            "@type": "DigitalDocument",
            "url": format!("file://{}", source.display()),
            "encodingFormat": encoding_hint
        }
    })
}

/// JSON-LD graph describing an install run (planned or completed).
pub fn json_ld_install_report(
    project_key: &str,
    project_path: &Path,
    dry_run: bool,
    applied_dest_paths: &[PathBuf],
    link_pairs: &[(PathBuf, PathBuf, &str)],
) -> Value {
    let status = if dry_run {
        "https://schema.org/ActiveActionStatus"
    } else {
        "https://schema.org/CompletedActionStatus"
    };

    let items: Vec<Value> = link_pairs
        .iter()
        .enumerate()
        .map(|(i, (src, dest, link_kind))| {
            json!({
                "@type": "ListItem",
                "position": i + 1,
                "item": {
                    "@type": "CreateAction",
                    "name": format!("link-{link_kind}"),
                    "actionStatus": status,
                    "object": digital_document_pair(src, dest, "application/octet-stream")
                }
            })
        })
        .collect();

    json!({
        "@context": install_context(),
        "@graph": [
            {
                "@type": "SoftwareApplication",
                "@id": "agentz:SoftwareApplication",
                "name": "agentz",
                "applicationCategory": "DeveloperApplication",
                "description": "Unified AI agent workstream runtime: install, compile, MCP, ACP."
            },
            {
                "@type": "InstallAction",
                "@id": format!("urn:agentz:install:{project_key}"),
                "name": format!("Install agent config links for `{project_key}`"),
                "agent": { "@id": "agentz:SoftwareApplication" },
                "target": {
                    "@type": "Project",
                    "name": project_key,
                    "url": format!("file://{}", project_path.display())
                },
                "actionStatus": status,
                "result": {
                    "@type": "ItemList",
                    "numberOfItems": items.len(),
                    "itemListElement": items
                }
            },
            {
                "@type": "Dataset",
                "@id": format!("urn:agentz:applied-paths:{project_key}"),
                "name": "Applied destination paths",
                "encodingFormat": "application/json",
                "variableMeasured": applied_dest_paths.iter().map(|p| json!({
                    "@type": "PropertyValue",
                    "name": "path",
                    "value": p.display().to_string()
                })).collect::<Vec<_>>()
            }
        ]
    })
}
