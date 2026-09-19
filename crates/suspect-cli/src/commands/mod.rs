//! Command implementations, one module per subcommand.

use suspect_ref::Workspace;
pub mod acquire;
pub mod acquire_cmd;
pub mod bench;
pub mod check;
pub mod codegen_cmd;
pub mod codegen_compare;
pub mod codegen_session;
pub mod fmt;
pub mod fuzz;
pub mod gateway;
pub mod generate;
pub mod http;
pub mod lint;
pub mod overlay;
pub mod replay;
pub mod rules;
pub mod sdk;
pub mod sdk_profiles;
pub mod stats;
pub mod terraform_cmd;
pub mod test;
pub mod validate;
pub mod watch;

/// Opens the requested entry and the OpenAPI sources explicitly declared
/// by an Arazzo entry. References resolve on demand; neighboring files are
/// not workspace inputs merely because they share a directory.
///
/// # Errors
/// Propagates entry and declared-source URI or loading failures.
pub fn workspace_for_entry(spec: &std::path::Path) -> anyhow::Result<std::sync::Arc<Workspace>> {
    use anyhow::Context;
    use suspect_ref::WorkspaceBuilder;
    use suspect_source::Uri;

    // Use the same lexical identity as callers and preserve the requested
    // retrieval base for relative references, including symlink spellings.
    let uri = Uri::from_path(spec)?;
    let ws = WorkspaceBuilder::new().build()?;
    let entry = ws.open(uri.as_str())?;
    if entry.doc().sniff_family() == suspect_low::SpecFamily::Arazzo10 {
        let doc = suspect_arazzo::ArazzoDoc::new(entry.doc());
        for source in doc.source_descriptions() {
            if source.kind != suspect_arazzo::SourceType::OpenApi {
                continue;
            }
            let target = uri.join(source.url)?;
            ws.open(target.as_str()).with_context(|| {
                format!("load source description '{}' ({})", source.name, source.url)
            })?;
        }
    }
    Ok(std::sync::Arc::new(ws))
}
