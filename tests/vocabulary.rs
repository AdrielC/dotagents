use std::path::Path;

use agents_unified::{install_context, json_ld_install_report, SCHEMA_ORG};

#[test]
fn install_report_json_ld_uses_schema_org_context() {
    let ld = json_ld_install_report(
        "demo",
        Path::new("/tmp/demo"),
        false,
        &[],
        &[],
    );
    assert_eq!(ld["@context"], install_context());
    assert_eq!(install_context()["@vocab"], SCHEMA_ORG);
    let graph = ld["@graph"].as_array().unwrap();
    assert!(graph.iter().any(|n| n["@type"] == "SoftwareApplication"));
    assert!(graph.iter().any(|n| n["@type"] == "InstallAction"));
}
