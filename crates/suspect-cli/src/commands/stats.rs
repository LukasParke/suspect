//! `suspect stats`: structural counts for one document, family-aware.
//! Counts come from the low model so every family reports what it has:
//! OpenAPI paths/operations/components, Arazzo workflows, Overlay actions.
//! The parse census (`$ref` edges, the cycle census and the loaded
//! workspace) is here too; `check` used to print it.

use std::path::Path;
use std::time::Instant;

use serde::Serialize;
use suspect_low::{NodeRef, SpecFamily};
use suspect_ref::{CycleKind, WorkspaceBuilder};

use crate::ReportFormat;
use crate::output;

/// Maps a spec family to its display label.
#[must_use]
pub fn family_label(family: SpecFamily) -> &'static str {
    match family {
        SpecFamily::Oas2 => "Oas2",
        SpecFamily::Oas30 => "Oas30",
        SpecFamily::Oas31 => "Oas31",
        SpecFamily::Oas32 => "Oas32",
        SpecFamily::Arazzo10 => "Arazzo10",
        SpecFamily::Overlay10 => "Overlay10",
        SpecFamily::Load10 => "Load10",
        SpecFamily::Unknown => "Unknown",
    }
}

/// Count of a mapping node's entries (0 when absent or not an object).
fn entries_len(node: Option<NodeRef<'_>>) -> usize {
    node.map(|n| n.entries().len()).unwrap_or(0)
}

/// Count of a sequence node's items (0 when absent or not an array).
fn items_len(node: Option<NodeRef<'_>>) -> usize {
    node.map(|n| n.items().len()).unwrap_or(0)
}

/// HTTP operation methods recognized inside a path item.
const OPERATION_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
];

/// Structural counts for one document.
#[derive(Debug, Clone, Serialize)]
pub struct StatsReport {
    /// Path as given on the command line.
    pub path: String,
    /// Display label of the detected spec family.
    pub family: String,
    /// Size of the input file in bytes.
    pub size_bytes: usize,
    /// Wall-clock milliseconds spent parsing.
    pub parse_ms: f64,
    /// Number of path entries (`/paths`); 0 for non-OpenAPI families.
    pub paths: usize,
    /// Number of HTTP operations across all path items.
    pub operations: usize,
    /// Number of entries under `components/schemas`.
    pub schemas: usize,
    /// Number of entries under `components/parameters`.
    pub parameters: usize,
    /// Number of entries under `components/responses`.
    pub responses: usize,
    /// Number of entries under `components/securitySchemes`.
    pub security_schemes: usize,
    /// Number of top-level tags.
    pub tags: usize,
    /// Number of top-level webhooks.
    pub webhooks: usize,
    /// Number of Arazzo workflows; 0 for non-Arazzo families.
    pub workflows: usize,
    /// Number of Overlay actions; 0 for non-Overlay families.
    pub actions: usize,
    /// Number of syntax errors found while parsing.
    pub syntax_errors: usize,
    /// Number of `$ref` edges leaving this document.
    pub ref_edges: usize,
    /// Cycles in the reference graph that are legal (schema recursion).
    pub legal_cycles: usize,
    /// Cycles in the reference graph that are illegal (unresolvable expansion).
    pub illegal_cycles: usize,
    /// Documents in the loaded workspace closure.
    pub workspace_docs: usize,
    /// `$ref` edges across the loaded workspace closure.
    pub workspace_edges: usize,
}

/// The `$ref` census of `path`: edges, legal and illegal cycles, and the
/// closure's document and edge counts. Zero when the workspace cannot
/// load it (the parse counts still report).
fn census(path: &Path) -> (usize, usize, usize, usize, usize) {
    let Ok(ws) = WorkspaceBuilder::new().build() else {
        return (0, 0, 0, 0, 0);
    };
    let Ok(handle) = ws.open(&path.display().to_string()) else {
        return (0, 0, 0, 0, 0);
    };
    let edges = handle.edges().len();
    let cycles = handle.cycles();
    let count = |kind: CycleKind| cycles.cycles.iter().filter(|c| c.kind == kind).count();
    let stats = ws.stats();
    (
        edges,
        count(CycleKind::LegalRecursion),
        count(CycleKind::Illegal),
        stats.docs,
        stats.edges,
    )
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

    let (ref_edges, legal_cycles, illegal_cycles, workspace_docs, workspace_edges) = census(path);
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
        syntax_errors: doc.syntax_errors().len(),
        ref_edges,
        legal_cycles,
        illegal_cycles,
        workspace_docs,
        workspace_edges,
    })
}

/// `suspect stats <PATH>`: prints the counts as an aligned table or JSON.
///
/// # Errors
/// IO or parse failures; JSON serialization failures.
pub fn stats(path: &Path, format: ReportFormat) -> anyhow::Result<i32> {
    let report = stats_of(path)?;
    match format {
        ReportFormat::Text => {
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
            println!("  syntax errors:     {}", report.syntax_errors);
            println!("  $ref edges:        {}", report.ref_edges);
            println!(
                "  cycles:            {} legal, {} illegal",
                report.legal_cycles, report.illegal_cycles
            );
            println!(
                "  workspace:         {} docs, {} edges",
                report.workspace_docs, report.workspace_edges
            );
        }
        ReportFormat::Json => output::print_json(&report)?,
    }
    Ok(0)
}
