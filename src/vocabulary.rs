//! [schema.org](https://schema.org) vocabulary for machine-readable install and configuration audits.
//!
//! We emit **JSON-LD** so the same document is human-inspectable and usable by tools that understand
//! schema.org without a custom ontology. IRIs use the `https://schema.org` namespace per schema.org guidance.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Canonical schema.org namespace prefix for `@type` and compact IRIs.
pub const SCHEMA_ORG: &str = "https://schema.org";

/// JSON-LD `@context` mapping common terms to schema.org.
pub fn install_context() -> Value {
    json!({
        "@vocab": SCHEMA_ORG,
        "schema": SCHEMA_ORG,
        "agents": "https://agents-unified.dev/ns#"
    })
}

/// Describes one filesystem link as a [`DigitalDocument`](https://schema.org/DigitalDocument) relationship.
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

/// Build a JSON-LD graph describing a completed or planned project install.
///
/// Uses [`InstallAction`](https://schema.org/InstallAction) as the top-level activity and
/// [`ItemList`](https://schema.org/ItemList) of [`CreateAction`](https://schema.org/CreateAction) /
/// [`UpdateAction`](https://schema.org/UpdateAction)-like entries for each applied link (represented as
/// `DigitalDocument` with `isBasedOn` for provenance).
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
                "@id": "agents-unified:SoftwareApplication",
                "name": "agents_unified",
                "applicationCategory": "DeveloperApplication",
                "description": "Unified AI agent config install and link orchestration."
            },
            {
                "@type": "InstallAction",
                "@id": format!("urn:agents-unified:install:{project_key}"),
                "name": format!("Install agent config links for `{project_key}`"),
                "agent": { "@id": "agents-unified:SoftwareApplication" },
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
                "@id": format!("urn:agents-unified:applied-paths:{project_key}"),
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
