//! `suspect stats`: structural counts for one document, family-aware.
//! Counts come from the low model so every family reports what it has:
//! OpenAPI paths/operations/components, Arazzo workflows, Overlay actions.

use std::path::Path;
use std::time::Instant;

use serde::Serialize;
use suspect_low::{NodeRef, SpecFamily};

use crate::commands::check::family_label;
use crate::output;
use crate::OutputFormat;

/// Count of a mapping node's entries (0 when absent or not an object).
fn entries_len(node: Option<NodeRef<'_>>) -> usize {
    node.map(|n| n.entries().len()).unwrap_or(0)
}

/// Count of a sequence node's items (0 when absent or not an array).
fn items_len(node: Option<NodeRef<'_>>) -> usize {
    node.map(|n| n.items().len()).unwrap_or(0)
}

/// HTTP operation methods recognized inside a path item.
const OPERATION_METHODS: &[&str] =
    &["get", "put", "post", "delete", "options", "head", "patch", "trace", "query"];

/// Structural counts for one document.
#[derive(Debug, Clone, Serialize)]
pub struct StatsReport {
    pub path: String,
    pub family: String,
    pub size_bytes: usize,
    pub parse_ms: f64,
    pub paths: usize,
    pub operations: usize,
    pub schemas: usize,
    pub parameters: usize,
    pub responses: usize,
    pub security_schemes: usize,
    pub tags: usize,
    pub webhooks: usize,
    pub workflows: usize,
    pub actions: usize,
}

/// Computes [`StatsReport`] for one document.
///
/// # Errors
/// IO or parse failures.
pub fn stats_of(path: &Path) -> anyhow::Result<StatsReport> {
    let shown = path.display().to_string();
    let start = Instant::now();
    let doc = crate::load_doc(path)?;
    let parse_ms = start.elapsed().as_secs_f64() * 1000.0;
    let size_bytes = doc.inner().bytes().len();
    let root = doc.root();

    // Operations: method keys inside each path item.
    let mut operations = 0usize;
    if let Some(paths) = root.get("paths") {
        for entry in paths.entries() {
            let Some(item) = entry.value else { continue };
            operations += item
                .entries()
                .iter()
                .filter(|e| OPERATION_METHODS.contains(&e.key))
                .count();
        }
    }

    let components = root.get("components");
    let schemas = entries_len(components.and_then(|c| c.get("schemas")))
        + entries_len(root.get("definitions"));
    let parameters = entries_len(components.and_then(|c| c.get("parameters")));
    let responses = entries_len(components.and_then(|c| c.get("responses")));
    let security_schemes = entries_len(components.and_then(|c| c.get("securitySchemes")))
        + entries_len(root.get("securityDefinitions"));
    let tags = items_len(root.get("tags"));
    let webhooks = entries_len(root.get("webhooks"));

    let mut workflows = 0usize;
    let mut actions = 0usize;
    match doc.sniff_family() {
        SpecFamily::Arazzo10 => {
            workflows = suspect_arazzo::ArazzoDoc::new(&doc).workflows().len();
        }
        SpecFamily::Overlay10 => {
            actions = suspect_overlay::OverlayDoc::parse(&doc)?.actions().len();
        }
        _ => {}
    }

    Ok(StatsReport {
        path: shown,
        family: family_label(doc.sniff_family()).into(),
        size_bytes,
        parse_ms,
        paths: entries_len(root.get("paths")),
        operations,
        schemas,
        parameters,
        responses,
        security_schemes,
        tags,
        webhooks,
        workflows,
        actions,
    })
}

/// `suspect stats <PATH>`: prints the counts as an aligned table or JSON.
///
/// # Errors
/// IO or parse failures; JSON serialization failures.
pub fn stats(path: &Path, format: OutputFormat) -> anyhow::Result<i32> {
    let report = stats_of(path)?;
    match format {
        OutputFormat::Text => {
            println!("{}", report.path);
            println!("  family:            {}", report.family);
            println!("  size bytes:        {}", report.size_bytes);
            println!("  parse ms:          {:.2}", report.parse_ms);
            println!("  paths:             {}", report.paths);
            println!("  operations:        {}", report.operations);
            println!("  schemas:           {}", report.schemas);
            println!("  parameters:        {}", report.parameters);
            println!("  responses:         {}", report.responses);
            println!("  security schemes:  {}", report.security_schemes);
            println!("  tags:              {}", report.tags);
            println!("  webhooks:          {}", report.webhooks);
            println!("  workflows:         {}", report.workflows);
            println!("  actions:           {}", report.actions);
        }
        OutputFormat::Json => output::print_json(&report)?,
    }
    Ok(0)
}
