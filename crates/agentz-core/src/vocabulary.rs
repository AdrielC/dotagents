//! [schema.org](https://schema.org) JSON-LD builders. Pure; no IO. Works on `&Path` references
//! without touching the filesystem.
//!
//! Every IRI / JSON-LD keyword that was previously spelled inline as a string literal now lives as
//! a typed constant or strong enum below. Add a new action-status variant or a new `@type` tag by
//! editing [`ActionStatus`] / [`SchemaType`], not by copying a URL.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Root schema.org vocabulary URL.
pub const SCHEMA_ORG: &str = "https://schema.org";

/// `agentz`'s own namespace for domain-specific terms (`agentz:SoftwareApplication`, …).
pub const AGENTZ_NAMESPACE: &str = "https://agentz.dev/ns#";

/// URN prefix for install-action identifiers: `urn:agentz:install:{project_key}`.
pub const URN_INSTALL_PREFIX: &str = "urn:agentz:install";

/// URN prefix for applied-paths datasets: `urn:agentz:applied-paths:{project_key}`.
pub const URN_APPLIED_PATHS_PREFIX: &str = "urn:agentz:applied-paths";

/// Canonical agentz `@id` used as the agent of install actions.
pub const AGENTZ_APP_ID: &str = "agentz:SoftwareApplication";

/// Encoding-format hint for arbitrary binary artifacts.
pub const ENCODING_OCTET_STREAM: &str = "application/octet-stream";

/// Encoding-format hint for JSON artifacts.
pub const ENCODING_JSON: &str = "application/json";

/// schema.org `ActionStatus` values we emit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionStatus {
    /// `schema:ActiveActionStatus` — used for dry-runs.
    Active,
    /// `schema:CompletedActionStatus` — used for applied runs.
    Completed,
}

impl ActionStatus {
    /// Absolute IRI for this status (suitable for `"actionStatus"` values).
    #[must_use]
    pub fn iri(self) -> &'static str {
        match self {
            ActionStatus::Active => "https://schema.org/ActiveActionStatus",
            ActionStatus::Completed => "https://schema.org/CompletedActionStatus",
        }
    }
}

/// schema.org `@type` strings we emit. Keeping these typed means a refactor catches every site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchemaType {
    SoftwareApplication,
    InstallAction,
    CreateAction,
    DigitalDocument,
    Dataset,
    Project,
    ItemList,
    ListItem,
    PropertyValue,
}

impl SchemaType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SchemaType::SoftwareApplication => "SoftwareApplication",
            SchemaType::InstallAction => "InstallAction",
            SchemaType::CreateAction => "CreateAction",
            SchemaType::DigitalDocument => "DigitalDocument",
            SchemaType::Dataset => "Dataset",
            SchemaType::Project => "Project",
            SchemaType::ItemList => "ItemList",
            SchemaType::ListItem => "ListItem",
            SchemaType::PropertyValue => "PropertyValue",
        }
    }
}

/// JSON-LD context block used by [`json_ld_install_report`].
#[must_use]
pub fn install_context() -> Value {
    json!({
        "@vocab": SCHEMA_ORG,
        "schema": SCHEMA_ORG,
        "agentz": AGENTZ_NAMESPACE
    })
}

fn digital_document_pair(source: &Path, dest: &Path, encoding_hint: &str) -> Value {
    json!({
        "@type": SchemaType::DigitalDocument.as_str(),
        "name": dest.file_name().and_then(|s| s.to_str()).unwrap_or("link"),
        "url": format!("file://{}", dest.display()),
        "isBasedOn": {
            "@type": SchemaType::DigitalDocument.as_str(),
            "url": format!("file://{}", source.display()),
            "encodingFormat": encoding_hint
        }
    })
}

/// JSON-LD graph describing an install run (planned or completed).
#[must_use]
pub fn json_ld_install_report(
    project_key: &str,
    project_path: &Path,
    dry_run: bool,
    applied_dest_paths: &[PathBuf],
    link_pairs: &[(PathBuf, PathBuf, &str)],
) -> Value {
    let status = if dry_run {
        ActionStatus::Active
    } else {
        ActionStatus::Completed
    }
    .iri();

    let items: Vec<Value> = link_pairs
        .iter()
        .enumerate()
        .map(|(i, (src, dest, link_kind))| {
            json!({
                "@type": SchemaType::ListItem.as_str(),
                "position": i + 1,
                "item": {
                    "@type": SchemaType::CreateAction.as_str(),
                    "name": format!("link-{link_kind}"),
                    "actionStatus": status,
                    "object": digital_document_pair(src, dest, ENCODING_OCTET_STREAM)
                }
            })
        })
        .collect();

    json!({
        "@context": install_context(),
        "@graph": [
            {
                "@type": SchemaType::SoftwareApplication.as_str(),
                "@id": AGENTZ_APP_ID,
                "name": "agentz",
                "applicationCategory": "DeveloperApplication",
                "description": "Unified AI agent workstream runtime: install, compile, MCP, ACP."
            },
            {
                "@type": SchemaType::InstallAction.as_str(),
                "@id": format!("{URN_INSTALL_PREFIX}:{project_key}"),
                "name": format!("Install agent config links for `{project_key}`"),
                "agent": { "@id": AGENTZ_APP_ID },
                "target": {
                    "@type": SchemaType::Project.as_str(),
                    "name": project_key,
                    "url": format!("file://{}", project_path.display())
                },
                "actionStatus": status,
                "result": {
                    "@type": SchemaType::ItemList.as_str(),
                    "numberOfItems": items.len(),
                    "itemListElement": items
                }
            },
            {
                "@type": SchemaType::Dataset.as_str(),
                "@id": format!("{URN_APPLIED_PATHS_PREFIX}:{project_key}"),
                "name": "Applied destination paths",
                "encodingFormat": ENCODING_JSON,
                "variableMeasured": applied_dest_paths.iter().map(|p| json!({
                    "@type": SchemaType::PropertyValue.as_str(),
                    "name": "path",
                    "value": p.display().to_string()
                })).collect::<Vec<_>>()
            }
        ]
    })
}
