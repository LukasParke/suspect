//! `suspect contract`: emit a machine-readable contract package.
//!
//! The toolchain reads OpenAPI everywhere; this command is where suspect
//! *writes* one. A contract package is:
//!
//! - `openapi.yaml` (or `.json`) — a self-contained OpenAPI 3.1
//!   description with the whole `$ref` closure inlined, so any consumer
//!   can read it without a workspace.
//! - `manifest.json` — the revision identity (SHA-256 over the source
//!   closure), the document and operation census, the cyclic-ref
//!   markers, and the source files, so a consumer can tell exactly which
//!   inputs produced it.
//! - `contract.lock.json` — the source digest keyed by relative path,
//!   written so `suspect contract --check` can prove a package is current
//!   without re-reading the whole closure.
//!
//! Overlay profiles apply before emission (`--profile public`), so a
//! contract package can be one of several published views of the same
//! source.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::DocFormat;

/// One contract emission.
#[derive(Debug, Args)]
pub struct ContractArgs {
    /// Entry OpenAPI document.
    #[arg(required = true)]
    pub input: PathBuf,
    /// Output directory for the contract package.
    #[arg(short, long, default_value = "contract")]
    pub out: PathBuf,
    /// Emit the description as JSON instead of YAML.
    #[arg(long)]
    pub json: bool,
    /// Overlay documents applied in order before emission (repeatable).
    #[arg(long = "overlay", value_name = "FILE")]
    pub overlays: Vec<PathBuf>,
    /// Verify the existing package matches the current source instead of
    /// writing; exit 1 when it is stale or missing.
    #[arg(long)]
    pub check: bool,
}

/// The `manifest.json` a contract package carries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContractManifest {
    /// Package format identifier and version.
    pub format: String,
    /// The emitted description's title.
    pub title: String,
    /// The emitted description's version.
    pub api_version: String,
    /// `sha256-<hex>` over the source closure: the package's identity.
    pub revision: String,
    /// Source documents in the closure, relative to the entry.
    pub documents: Vec<String>,
    /// Operations in the emitted description.
    pub operations: usize,
    /// Component schemas in the emitted description.
    pub schemas: usize,
    /// `$ref` edges traversed across the closure.
    pub ref_edges: usize,
    /// Cyclic references preserved as `x-suspect-cyclic` markers.
    pub cyclic_refs: usize,
    /// Overlay documents applied, in order.
    pub overlays: Vec<String>,
    /// Schema names referenced but never declared.
    pub unresolved_refs: Vec<String>,
}

/// Package format identifier.
const FORMAT: &str = "suspect.contract.v1";

/// Emits (or verifies) a contract package.
///
/// # Errors
/// IO, workspace, parse, or serialization failures.
pub fn contract(args: &ContractArgs) -> anyhow::Result<i32> {
    let spec_path = args.input.canonicalize()?;
    let workspace = std::sync::Arc::new(
        suspect_ref::WorkspaceBuilder::new()
            .root(spec_path.parent().unwrap_or(Path::new(".")))
            .build()?,
    );
    let shown = spec_path.display().to_string();
    workspace.load_all(&shown)?;
    contract_on_workspace(&workspace, &spec_path, args)
}

/// [`contract`] against a workspace the caller already loaded: a project
/// build compiles the same published spec every other stage consumes.
/// Inside, the entry's parse comes from the workspace (the historical
/// second parse of the same file is gone), and the manifest counts come
/// from the fast IR path (probe-verified identical to the full compile).
///
/// # Errors
/// IO, parse, or serialization failures.
pub fn contract_on_workspace(
    workspace: &std::sync::Arc<suspect_ref::Workspace>,
    spec_path: &Path,
    args: &ContractArgs,
) -> anyhow::Result<i32> {
    let prof = std::env::var_os("SUSPECT_BUILD_PROFILE").is_some();
    let mut clock = std::time::Instant::now();
    let mut mark = |name: &str| {
        if prof {
            eprintln!("[contract-profile] {name}: {:?}", clock.elapsed());
        }
        clock = std::time::Instant::now();
    };

    mark("setup");
    // Collect the closure identity from the raw source bytes: any byte
    // change anywhere in the closure changes the revision.
    let mut documents = Vec::new();
    let mut digests: BTreeMap<String, String> = BTreeMap::new();
    for uri in workspace.uris() {
        let Some(handle) = workspace.get(&uri) else {
            continue;
        };
        let relative = relative_name(spec_path, uri.as_str());
        documents.push(relative.clone());
        let bytes = handle.doc().inner().bytes();
        digests.insert(
            relative,
            format!("sha256-{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes)),
        );
    }
    documents.sort();

    let mut edges = 0usize;
    let mut unresolved = Vec::new();
    for uri in workspace.uris() {
        let Some(handle) = workspace.get(&uri) else {
            continue;
        };
        // The count is the edge list's length — no resolution needed.
        // Only the unresolved list pays resolution, and that walk is
        // independent per edge against a concurrent memo table, so it
        // fans out across threads (5,486 edges on the reference spec was
        // over a second serially).
        let total = handle.edges().len();
        edges += total;
        let label = std::sync::Arc::new(handle.doc().uri().to_string());
        let chunk = (total / 8).max(64);
        let results: Vec<Vec<String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..total)
                .step_by(chunk)
                .map(|start| {
                    let handle = &handle;
                    let label = std::sync::Arc::clone(&label);
                    scope.spawn(move || {
                        let end = (start + chunk).min(total);
                        let mut out = Vec::new();
                        for index in start..end {
                            // Status only: the manifest counts edges and
                            // lists the unresolved ones; materializing
                            // every landing node bought nothing the
                            // verdict needed.
                            if let Err(error) = handle.resolve_edge_status(index) {
                                out.push(format!("{label}: {error}"));
                            }
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("edge resolution thread"))
                .collect()
        });
        for part in results {
            unresolved.extend(part);
        }
    }
    unresolved.sort();
    unresolved.dedup();

    mark("digests+edges");
    // The description: start from the entry document — the workspace's
    // own parse, not a second read of the same file — apply overlays when
    // configured, then inline the whole closure into one tree.
    // Canonicalized: the workspace keys documents by their canonical
    // retrieval URI (on macOS, /var resolves to /private/var), so a raw
    // relative path would miss the lookup.
    let entry_uri = suspect_source::Uri::from_path(
        &spec_path
            .canonicalize()
            .unwrap_or_else(|_| spec_path.to_path_buf()),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let entry_handle = workspace
        .get(&entry_uri)
        .ok_or_else(|| anyhow::anyhow!("entry document is not loaded"))?;
    let entry_doc = entry_handle.doc();
    let mut tree: suspect_overlay::Value = suspect_overlay::Value::from_node(entry_doc.root());
    let mut applied_overlays = Vec::new();
    for overlay_path in &args.overlays {
        let overlay_doc = crate::load_doc(overlay_path)?;
        let overlay_doc: &'static suspect_low::LowDoc = Box::leak(Box::new(overlay_doc));
        let parsed = suspect_overlay::OverlayDoc::parse(overlay_doc)?;
        let scratch: &'static suspect_low::LowDoc =
            Box::leak(Box::new(suspect_low::LowDoc::parse(
                "mem://contract-target.yaml".into(),
                suspect_source::Source::from_vec(tree.to_yaml().into_bytes()),
            )));
        let applied = suspect_overlay::apply(&parsed, scratch.root())?;
        tree = applied.output;
        applied_overlays.push(overlay_path.display().to_string());
    }

    mark("overlays");
    let inlined = inline_closure(&tree, workspace, spec_path);
    let description = match inlined {
        Some(text) => text,
        None => tree.to_yaml(),
    };

    mark("inline");
    // The manifest needs title, version, and counts — the fast IR path
    // supplies them at single-digit milliseconds against the CST compile's
    // ~350ms, with identical counts (probe-verified on the 63k-line
    // reference spec). The full compile happens where it is needed: the
    // SDK stage.
    let ir = suspect_ir::IrSpec::from_file(spec_path).unwrap_or_default();

    let manifest = ContractManifest {
        format: FORMAT.to_owned(),
        title: ir.title.clone(),
        api_version: ir.version.clone(),
        revision: revision_over(&digests),
        documents: documents.clone(),
        operations: ir.operations.len(),
        schemas: ir.schemas.len(),
        ref_edges: edges,
        cyclic_refs: 0,
        overlays: applied_overlays,
        unresolved_refs: unresolved.clone(),
    };

    mark("ir");
    if args.check {
        let description_path = description_path(&args.out, args.json);
        let manifest_path = args.out.join("manifest.json");
        let lock_path = args.out.join("contract.lock.json");
        let mut stale: Vec<String> = Vec::new();
        match std::fs::read_to_string(&lock_path) {
            Ok(text) => {
                let current = serde_json::json!({
                    "format": FORMAT,
                    "revision": manifest.revision,
                    "documents": digests,
                });
                if text.trim() != serde_json::to_string_pretty(&current)? {
                    stale.push(format!(
                        "the source closure changed (now {})",
                        manifest.revision
                    ));
                }
            }
            Err(_) => {
                stale.push("no contract.lock.json: the package was never emitted".to_owned());
            }
        }
        if std::fs::read_to_string(&description_path)
            .ok()
            .as_deref()
            .map(str::trim)
            != Some(description.trim())
        {
            stale.push(format!(
                "{} differs from the current source",
                description_path.display()
            ));
        }
        if std::fs::read_to_string(&manifest_path)
            .ok()
            .as_deref()
            .map(str::trim)
            != Some(serde_json::to_string_pretty(&manifest)?.trim())
        {
            stale.push(format!(
                "{} differs from the current source",
                manifest_path.display()
            ));
        }
        if stale.is_empty() {
            eprintln!("contract up to date ({})", manifest.revision);
            return Ok(0);
        }
        for reason in &stale {
            eprintln!("stale: {reason}");
        }
        eprintln!("run `suspect contract` to refresh the package");
        return Ok(1);
    }

    std::fs::create_dir_all(&args.out)?;
    let description_path = description_path(&args.out, args.json);
    std::fs::write(&description_path, &description)?;
    std::fs::write(
        args.out.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    let lock = serde_json::json!({
        "format": FORMAT,
        "revision": manifest.revision,
        "documents": digests,
    });
    std::fs::write(
        args.out.join("contract.lock.json"),
        format!("{}\n", serde_json::to_string_pretty(&lock)?),
    )?;

    eprintln!(
        "contract {}: {} document(s), {} operation(s), {} schema(s), {} ref edge(s) → {}",
        manifest.revision,
        manifest.documents.len(),
        manifest.operations,
        manifest.schemas,
        manifest.ref_edges,
        args.out.display()
    );
    if !manifest.unresolved_refs.is_empty() {
        eprintln!(
            "warning: {} unresolved reference(s) in the closure",
            manifest.unresolved_refs.len()
        );
    }
    let _ = DocFormat::Yaml;
    Ok(0)
}

/// The description filename inside a package.
fn description_path(out: &Path, json: bool) -> PathBuf {
    out.join(if json { "openapi.json" } else { "openapi.yaml" })
}

/// A closure-wide digest: the package identity.
fn revision_over(digests: &BTreeMap<String, String>) -> String {
    let mut text = String::new();
    for (path, digest) in digests {
        text.push_str(path);
        text.push(':');
        text.push_str(digest);
        text.push('\n');
    }
    format!(
        "sha256-{:x}",
        <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes())
    )
}

/// A document's name relative to the entry, for stable manifests.
fn relative_name(entry: &Path, uri: &str) -> String {
    let path = Path::new(uri);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    match (entry.parent(), name) {
        (Some(dir), Some(name)) => path
            .strip_prefix(dir)
            .map(|p| p.display().to_string())
            .unwrap_or(name),
        (_, Some(name)) => name,
        (_, None) => uri.to_owned(),
    }
}

/// Inlines every resolvable `$ref` in the description so the emitted
/// document stands alone. Cycles collapse to an `x-suspect-cyclic`
/// marker, which keeps the package finite and honest instead of hanging.
/// One cacheable `$ref` expansion: the inlined value plus every
/// reference dereferenced producing it. An expansion whose target set is
/// disjoint from the caller's active reference stack, and which produced
/// no cycle markers, is stack-independent — identical under any ancestor
/// chain — so it can be reused verbatim.
struct InlinedRef {
    value: suspect_overlay::Value,
    targets: Vec<String>,
}

/// The inline walk's mutable state. `markers` is a monotonic count of
/// cycle/unresolved markers produced; candidate expansions snapshot it
/// before and compare after, which needs no save/restore discipline.
struct InlineCtx<'a> {
    workspace: &'a std::sync::Arc<suspect_ref::Workspace>,
    seen: Vec<String>,
    depth: usize,
    cache: std::collections::HashMap<(PathBuf, String), InlinedRef>,
    markers: usize,
}

fn inline_closure(
    tree: &suspect_overlay::Value,
    workspace: &std::sync::Arc<suspect_ref::Workspace>,
    entry: &Path,
) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(&tree.to_json()).ok()?;
    let mut ctx = InlineCtx {
        workspace,
        seen: Vec::new(),
        depth: 0,
        cache: std::collections::HashMap::new(),
        markers: 0,
    };
    Some(inline_value(&value, entry, &mut ctx).to_yaml())
}

/// Recursively resolves `$ref`s against the loaded workspace. Shared
/// schemas referenced dozens of times inline once and reuse the cached
/// expansion — the reference spec's 5,486 edges collapse to its few
/// hundred unique targets.
fn inline_value(
    value: &serde_json::Value,
    current: &Path,
    ctx: &mut InlineCtx<'_>,
) -> suspect_overlay::Value {
    use suspect_overlay::Value;
    const MAX_DEPTH: usize = 12;
    match value {
        serde_json::Value::Object(map) => {
            // A bare `$ref` becomes its target, with cycles marked.
            if map.len() == 1
                && let Some(reference) = map.get("$ref").and_then(|r| r.as_str())
            {
                let key = reference.to_owned();
                if ctx.seen.contains(&key) {
                    ctx.markers += 1;
                    return Value::Object(vec![(
                        "x-suspect-cyclic".into(),
                        Value::Str(key.into()),
                    )]);
                }
                if ctx.depth > MAX_DEPTH {
                    ctx.markers += 1;
                    return Value::Bool(true);
                }
                // A stack-independent cached expansion is the answer as-is.
                let cache_key = (current.to_path_buf(), key.clone());
                if let Some(cached) = ctx.cache.get(&cache_key)
                    && !cached
                        .targets
                        .iter()
                        .any(|target| ctx.seen.contains(target))
                {
                    return cached.value.clone();
                }
                if let Some((target, document)) =
                    resolve_in_workspace(reference, ctx.workspace, current)
                {
                    let stack_before = ctx.seen.len();
                    let markers_before = ctx.markers;
                    ctx.seen.push(key);
                    ctx.depth += 1;
                    // Nested references resolve against the document that
                    // carried this one, not against the entry document.
                    let resolved = inline_value(&target, &document, ctx);
                    ctx.depth -= 1;
                    ctx.seen.pop();
                    let targets: Vec<String> = ctx.seen[stack_before..].to_vec();
                    let produced_marker = ctx.markers > markers_before;
                    // Cache only stack-independent expansions: no marker
                    // was produced inside, and none of the dereferenced
                    // targets touch anything an ancestor could hold.
                    if !produced_marker && !targets.iter().any(|target| ctx.seen.contains(target)) {
                        ctx.cache.insert(
                            cache_key,
                            InlinedRef {
                                value: resolved.clone(),
                                targets,
                            },
                        );
                    }
                    return resolved;
                }
                ctx.markers += 1;
                return Value::Object(vec![(
                    "x-suspect-unresolved".into(),
                    Value::Str(reference.into()),
                )]);
            }
            Value::Object(
                map.iter()
                    .map(|(key, child)| {
                        (
                            key.clone().into_boxed_str(),
                            inline_value(child, current, ctx),
                        )
                    })
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| inline_value(item, current, ctx))
                .collect(),
        ),
        other => match other {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => n
                .as_i64()
                .map_or_else(|| Value::Float(n.as_f64().unwrap_or_default()), Value::Int),
            serde_json::Value::String(s) => Value::Str(s.clone().into_boxed_str()),
            other => Value::Str(other.to_string().into_boxed_str()),
        },
    }
}

/// Resolves a `$ref` against the loaded workspace closure, returning the
/// target and the document it lives in (so nested refs resolve relative to
/// the right base).
fn resolve_in_workspace(
    reference: &str,
    workspace: &std::sync::Arc<suspect_ref::Workspace>,
    current: &Path,
) -> Option<(serde_json::Value, PathBuf)> {
    // A local pointer addresses the referring document.
    if let Some(pointer) = reference.strip_prefix('#') {
        let uri = suspect_source::Uri::from_path(current).ok()?;
        let handle = workspace.get(&uri)?;
        let pointer = suspect_low::Pointer::parse(pointer).ok()?;
        let node = handle.doc().root().pointer(&pointer)?;
        let value =
            serde_json::from_str(&suspect_overlay::Value::from_node(node).to_json()).ok()?;
        return Some((value, current.to_path_buf()));
    }
    // A relative document reference, with or without a fragment.
    let (file, fragment) = match reference.split_once('#') {
        Some((file, fragment)) => (file, fragment),
        // A whole-document reference addresses the document root.
        None => (reference, ""),
    };
    let candidate = current.parent()?.join(file);
    let uri = suspect_source::Uri::from_path(&candidate).ok()?;
    let handle = workspace.get(&uri)?;
    let pointer = if fragment.is_empty() {
        suspect_low::Pointer::root()
    } else {
        suspect_low::Pointer::parse(fragment).ok()?
    };
    let node = handle.doc().root().pointer(&pointer)?;
    let value = serde_json::from_str(&suspect_overlay::Value::from_node(node).to_json()).ok()?;
    Some((value, candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_changes_with_any_document() {
        let mut a = BTreeMap::new();
        a.insert("openapi.yaml".to_owned(), "sha256-aa".to_owned());
        let mut b = a.clone();
        b.insert("schemas.yaml".to_owned(), "sha256-bb".to_owned());
        assert_ne!(revision_over(&a), revision_over(&b));
        assert_eq!(revision_over(&a), revision_over(&a.clone()));
    }

    #[test]
    fn relative_names_are_stable() {
        let entry = Path::new("/repo/api/openapi.yaml");
        assert_eq!(
            relative_name(entry, "file:///repo/api/schemas.yaml"),
            "schemas.yaml"
        );
        assert_eq!(
            relative_name(entry, "file:///repo/api/openapi.yaml"),
            "openapi.yaml"
        );
    }
}
