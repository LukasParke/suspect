//! Command implementations, one module per subcommand.

use suspect_ref::Workspace;
pub mod acquire;
pub mod acquire_cmd;
pub mod admission;
pub mod application_cmd;
pub mod arazzo_diff;
pub mod auth;
pub mod bench;
pub mod breaking;
pub mod bridge;
pub mod check;
pub mod ci;
pub mod codegen_cmd;
pub mod codegen_compare;
pub mod codegen_session;
pub mod contract;
pub mod docs_gen_cmd;
pub mod evidence;
pub mod fmt;
pub mod fuzz;
pub mod gateway;
pub mod generate;
pub mod http;
pub mod impact;
pub mod lint;
pub mod overlay;
pub mod overlay_dry_run;
pub mod project;
pub mod publish;
pub mod release;
pub mod replay;
pub mod reverse;
pub mod rules;
pub mod sdk;
pub mod sdk_profiles;
pub mod stateful;
pub mod stats;
pub mod stubs;
pub mod terraform_cmd;
pub mod test;
pub mod upgrade;
pub mod validate;
pub mod watch;
pub mod why;

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
            // Arazzo 1.1 also names asyncapi descriptions, which carry the
            // channel/message metadata message steps compile against.
            if !matches!(
                source.kind,
                suspect_arazzo::SourceType::OpenApi | suspect_arazzo::SourceType::AsyncApi
            ) {
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

/// Builds the journal sink for a `--journal <file>` flag: append-to-file
/// when given, stdout otherwise.
///
/// # Errors
/// Propagates open failures for the journal file.
pub fn sink(journal: Option<&std::path::Path>) -> anyhow::Result<Box<dyn suspect_journal::Sink>> {
    match journal {
        Some(path) => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            Ok(Box::new(suspect_journal::FileSink::open(path)?))
        }
        None => Ok(Box::new(suspect_journal::StdoutSink)),
    }
}
