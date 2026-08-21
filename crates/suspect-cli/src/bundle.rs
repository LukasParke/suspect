//! `suspect bundle`: bundle a document plus its `$ref` closure into one file.
//!
//! Two strategies:
//! - `keep` (default): load every reachable document, resolve every `$ref`
//!   edge as validation, and emit the input bytes unchanged (passthrough).
//! - `inline`: materialize the entry document, replacing every `$ref`
//!   mapping with a deep copy of its resolved target. Cycle-safe: a ref
//!   whose expansion would necessarily recurse — its target is already on
//!   the active expansion chain, or the target can reach itself through the
//!   reference graph — is emitted as the original
//!   `{"$ref": "<raw>", "x-suspect-cyclic": true}` mapping instead of
//!   being expanded.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use suspect_low::{NodeRef, Pointer, ValueKind};
use suspect_overlay::Value;
use suspect_ref::{DocId, Resolution, Workspace, WorkspaceBuilder};

use crate::output::{self, Finding, Severity};
use crate::{DocFormat, Strategy};

/// A materializable location: document slot plus in-document pointer.
type Loc = (DocId, Pointer);

/// Cycle-safe `$ref` inliner over a loaded workspace.
struct Inliner<'ws> {
    ws: &'ws Workspace,
    /// `Uri -> DocId` index (the workspace does not expose one directly).
    ids: HashMap<suspect_source::Uri, DocId>,
    /// Targets currently being expanded on this branch.
    stack: Vec<Loc>,
    /// Memoized edge targets keyed by (owning document, raw ref spelling):
    /// identical spellings from one document always resolve alike, while the
    /// workspace memo is per containing pointer and would not dedupe.
    #[allow(clippy::type_complexity)]
    by_raw: HashMap<(DocId, Box<str>), Option<Result<(Loc, Option<NodeRef<'ws>>), String>>>,
    /// Memoized `reaches_self` results per target location.
    participants: HashMap<Loc, bool>,
    refs_inlined: usize,
    cycles_inlined: usize,
    errors: Vec<Finding>,
}

impl<'ws> Inliner<'ws> {
    fn new(ws: &'ws Workspace) -> Self {
        let ids = ws.uris().into_iter().filter_map(|u| ws.get(&u).map(|h| (u, h.id()))).collect();
        Self {
            ws,
            ids,
            stack: Vec::new(),
            by_raw: HashMap::new(),
            participants: HashMap::new(),
            refs_inlined: 0,
            cycles_inlined: 0,
            errors: Vec::new(),
        }
    }

    /// `{"$ref": "<raw>", "x-suspect-cyclic": true}`
    fn cyclic_ref(raw: &str) -> Value {
        Value::Object(vec![
            ("$ref".into(), Value::Str(raw.into())),
            ("x-suspect-cyclic".into(), Value::Bool(true)),
        ])
    }

    fn doc_id_of(&self, uri: &suspect_source::Uri) -> Option<DocId> {
        self.ids.get(uri).copied()
    }

    /// Root node of loaded document `id`.
    fn root_of(&self, id: DocId) -> Option<NodeRef<'ws>> {
        for uri in self.ws.uris() {
            if let Some(handle) = self.ws.get(&uri)
                && handle.id() == id {
                    return Some(handle.doc().root());
                }
        }
        None
    }

    /// Records an unresolved ref at the ref value's source location.
    fn record_error(&mut self, node: NodeRef<'_>, raw: &str, message: String) {
        let (line, col) = node.line_col();
        let file = node.syntax().doc().uri().to_string();
        self.errors.push(Finding {
            file,
            severity: Severity::Error,
            code: "unresolved-ref".into(),
            message: format!("{raw}: {message}"),
            line,
            col: col + 1,
        });
    }

    /// Resolves one `$ref` value node to a materializable location plus the
    /// resolved node (absent for whole-document and error outcomes).
    fn resolve_target(
        &mut self,
        ref_value: NodeRef<'ws>,
    ) -> Option<Result<(Loc, Option<NodeRef<'ws>>), String>> {
        let uri = ref_value.syntax().doc().uri().clone();
        let handle = self.ws.get(&uri)?;
        let owner = self.ids.get(&uri).copied()?;
        let raw_key: Box<str> = String::from_utf8_lossy(ref_value.scalar_bytes()).into();
        if let Some(cached) = self.by_raw.get(&(owner, raw_key.clone())) {
            return cached.clone();
        }
        let resolved = match handle.resolve_ref_value(ref_value) {
            Ok(Resolution::Node(target)) => {
                let doc = self.doc_id_of(target.syntax().doc().uri())?;
                Some(Ok(((doc, target.path_from_root()), Some(target))))
            }
            Ok(Resolution::WholeDoc(id)) => Some(Ok(((id, Pointer::root()), None))),
            Ok(Resolution::Cycle { .. }) => None,
            Err(e) => Some(Err(e.to_string())),
        };
        self.by_raw.insert((owner, raw_key), resolved.clone());
        resolved
    }

    /// Precomputes every location whose expansion would recurse: builds the
    /// ref-target graph (location -> targets of the `$ref`s it contains,
    /// discovered via the workspace's own edge scan and interval
    /// containment) once, and marks locations on a directed cycle —
    /// nontrivial SCC or self-loop — with an iterative Tarjan pass. Total
    /// cost is O(E log E) resolutions and graph work, independent of how
    /// deep documents nest.
    fn precompute_participants(&mut self) {
        // One ref record per workspace edge, with its resolved target.
        struct RefRec {
            doc: DocId,
            range: std::ops::Range<usize>,
            target: Option<(Loc, std::ops::Range<usize>)>,
        }
        let mut refs: Vec<RefRec> = Vec::new();
        let mut doc_ranges: Vec<(DocId, std::ops::Range<usize>)> = Vec::new();
        for uri in self.ws.uris() {
            let Some(handle) = self.ws.get(&uri) else { continue };
            let doc = handle.id();
            let len = handle.doc().inner().bytes().len();
            doc_ranges.push((doc, 0..len.max(1)));
            for i in 0..handle.edges().len() {
                let (edge_at, edge_raw) = {
                    let e = &handle.edges()[i];
                    (e.at.clone(), e.raw.clone())
                };
                // Same spelling from the same document resolves identically.
                if self.by_raw.contains_key(&(doc, edge_raw)) {
                    continue;
                }
                let target = match handle.resolve_edge(i) {
                    Ok(Resolution::Node(t)) => {
                        let Some(d) = self.doc_id_of(t.syntax().doc().uri()) else { continue };
                        Some(((d, t.path_from_root()), t.byte_range()))
                    }
                    Ok(Resolution::WholeDoc(id)) => {
                        Some(((id, Pointer::root()), 0..len.max(1)))
                    }
                    // Chain loops and unresolved refs terminate via the
                    // marker/error paths in `inline_ref`; they add no edge.
                    _ => None,
                };
                refs.push(RefRec { doc, range: edge_at, target });
            }
        }

        // Per-document refs sorted by start byte for interval containment.
        let mut by_doc: HashMap<DocId, Vec<usize>> = HashMap::new();
        for (i, r) in refs.iter().enumerate() {
            by_doc.entry(r.doc).or_default().push(i);
        }
        for list in by_doc.values_mut() {
            list.sort_by_key(|&i| refs[i].range.start);
        }

        // Graph nodes: every distinct ref-target location plus doc roots.
        let mut nodes: Vec<(Loc, std::ops::Range<usize>)> = Vec::new();
        let mut seen: std::collections::HashSet<Loc> = std::collections::HashSet::new();
        for r in &refs {
            if let Some((loc, range)) = &r.target
                && seen.insert(loc.clone()) {
                    nodes.push((loc.clone(), range.clone()));
                }
        }
        for (doc, range) in &doc_ranges {
            let loc = (*doc, Pointer::root());
            if seen.insert(loc.clone()) {
                nodes.push((loc, range.clone()));
            }
        }

        // Adjacency: node -> targets of the refs contained in its byte range.
        // Ref intervals in a document are disjoint-or-nested, so every ref
        // starting inside a node's range lies fully inside it.
        let id_of: HashMap<&Loc, usize> =
            nodes.iter().enumerate().map(|(i, (l, _))| (l, i)).collect();
        let adj: Vec<Vec<usize>> = nodes
            .iter()
            .map(|(loc, range)| {
                let Some(idxs) = by_doc.get(&loc.0) else { return Vec::new() };
                let lo = idxs.partition_point(|&i| refs[i].range.start < range.start);
                idxs[lo..]
                    .iter()
                    .take_while(|&&i| refs[i].range.start < range.end)
                    .filter_map(|&i| refs[i].target.as_ref().and_then(|t| id_of.get(&t.0).copied()))
                    .collect()
            })
            .collect();

        self.tarjan_mark(&nodes, &adj);
    }

    /// Marks `self.participants` for every node on a directed cycle.
    fn tarjan_mark(&mut self, nodes: &[(Loc, std::ops::Range<usize>)], adj: &[Vec<usize>]) {
        let n = nodes.len();
        struct Frame {
            v: usize,
            child: usize,
        }
        const UNVISITED: usize = usize::MAX;
        let mut index = vec![UNVISITED; n];
        let mut low = vec![0usize; n];
        let mut on_stack = vec![false; n];
        let mut tarjan: Vec<usize> = Vec::new();
        let mut counter = 0usize;
        for start in 0..n {
            if index[start] != UNVISITED {
                continue;
            }
            let mut frames = vec![Frame { v: start, child: 0 }];
            while let Some(frame) = frames.last_mut() {
                if frame.child == 0 {
                    index[frame.v] = counter;
                    low[frame.v] = counter;
                    counter += 1;
                    tarjan.push(frame.v);
                    on_stack[frame.v] = true;
                }
                if frame.child < adj[frame.v].len() {
                    let w = adj[frame.v][frame.child];
                    frame.child += 1;
                    if index[w] == UNVISITED {
                        frames.push(Frame { v: w, child: 0 });
                    } else if on_stack[w] {
                        low[frame.v] = low[frame.v].min(index[w]);
                    }
                    continue;
                }
                let v = frame.v;
                frames.pop();
                if let Some(parent) = frames.last_mut() {
                    low[parent.v] = low[parent.v].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut scc = Vec::new();
                    loop {
                        let w = tarjan.pop().expect("tarjan stack non-empty");
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let cyclic = scc.len() > 1 || adj[v].contains(&v);
                    if cyclic {
                        for &w in &scc {
                            self.participants.insert(nodes[w].0.clone(), true);
                        }
                    }
                }
            }
        }
    }

    /// True when expanding the target at `loc` would necessarily recurse.
    fn reaches_self(&self, loc: &Loc) -> bool {
        self.participants.get(loc).copied().unwrap_or(false)
    }

    /// Materializes `node`, replacing `$ref` mappings with resolved targets.
    fn inline_node(&mut self, node: NodeRef<'ws>) -> Value {
        self.inline_inner(node, true)
    }

    /// Materializes `node`; `allow_refs` is cleared when re-walking a
    /// mapping whose `$ref` failed to resolve, so its raw ref is kept as a
    /// plain string instead of recursing forever.
    fn inline_inner(&mut self, node: NodeRef<'ws>, allow_refs: bool) -> Value {
        if node.kind() == ValueKind::Object {
            if allow_refs
                && let Some(ref_node) = node.get("$ref")
                    && ref_node.kind() == ValueKind::Str
                        && let Some(raw) = ref_node.as_str() {
                            return self.inline_ref(node, ref_node, raw);
                        }
            let entries = node
                .entries()
                .into_iter()
                .map(|e| {
                    let v =
                        e.value.map(|v| self.inline_inner(v, allow_refs)).unwrap_or(Value::Null);
                    (e.key.into(), v)
                })
                .collect();
            return Value::Object(entries);
        }
        if node.kind() == ValueKind::Array {
            return Value::Array(
                node.items().into_iter().map(|n| self.inline_inner(n, allow_refs)).collect(),
            );
        }
        Value::from_node(node)
    }

    /// Expands one `$ref` mapping, with the cycle guards described at the
    /// module level.
    fn inline_ref(&mut self, mapping: NodeRef<'ws>, ref_value: NodeRef<'ws>, raw: &str) -> Value {
        let Some(outcome) = self.resolve_target(ref_value) else {
            // Resolver-level cycle: the chain revisits a location.
            self.cycles_inlined += 1;
            return Self::cyclic_ref(raw);
        };
        let (loc, target) = match outcome {
            Ok(ok) => ok,
            Err(message) => {
                self.record_error(ref_value, raw, message);
                return self.inline_inner(mapping, false);
            }
        };
        let on_stack = self.stack.contains(&loc);
        if on_stack || self.reaches_self(&loc) {
            self.cycles_inlined += 1;
            return Self::cyclic_ref(raw);
        }
        let value = match target {
            Some(t) => {
                self.stack.push(loc);
                let v = self.inline_node(t);
                self.stack.pop();
                v
            }
            None => {
                // Whole-document target: inline the target document's root.
                let Some(root) = self.root_of(loc.0) else {
                    self.record_error(ref_value, raw, "whole-document target not loaded".into());
                    return self.inline_inner(mapping, false);
                };
                self.stack.push(loc);
                let v = self.inline_node(root);
                self.stack.pop();
                v
            }
        };
        self.refs_inlined += 1;
        value
    }
}

/// Aggregate bundle result.
#[derive(Debug, Clone, Serialize)]
pub struct BundleReport {
    pub strategy: String,
    pub docs: usize,
    pub edges: usize,
    pub refs_inlined: usize,
    pub cycles_inlined: usize,
    pub errors: Vec<Finding>,
}

/// `suspect bundle <IN> [-o OUT] [--strategy keep|inline] [--format json|yaml]`.
///
/// # Errors
/// IO, workspace load failures, or serialization failures.
pub fn bundle(
    input: &Path,
    out: Option<&Path>,
    strategy: Strategy,
    format: Option<DocFormat>,
) -> anyhow::Result<i32> {
    let shown = input.display().to_string();
    match strategy {
        Strategy::Keep => bundle_keep(input, &shown, out),
        Strategy::Inline => bundle_inline(input, &shown, out, format),
    }
}

/// `keep`: validate the whole closure, emit input bytes unchanged.
fn bundle_keep(input: &Path, shown: &str, out: Option<&Path>) -> anyhow::Result<i32> {
    let ws = WorkspaceBuilder::new().build()?;
    let docs = ws.load_all(shown)?;

    let mut edges = 0usize;
    let mut errors = Vec::new();
    for uri in ws.uris() {
        let Some(handle) = ws.get(&uri) else { continue };
        let doc = handle.doc();
        for (i, edge) in handle.edges().iter().enumerate() {
            edges += 1;
            if let Err(e) = handle.resolve_edge(i) {
                let (line, col) =
                    doc.inner().line_index().line_col(doc.inner().bytes(), edge.at.start);
                errors.push(Finding {
                    file: uri.to_string(),
                    severity: Severity::Error,
                    code: "unresolved-ref".into(),
                    message: format!("{}: {e}", edge.raw),
                    line,
                    col: col + 1,
                });
            }
        }
    }

    // Passthrough: emit the exact input bytes.
    let bytes = suspect_source::Source::from_path(input)?.bytes().to_vec();
    match out {
        Some(path) => std::fs::write(path, &bytes)?,
        None => {
            use std::io::Write;
            std::io::stdout().write_all(&bytes)?;
        }
    }

    eprintln!(
        "bundled {docs} document(s), {edges} $ref edge(s), {} unresolved",
        errors.len()
    );
    let has_error = errors.iter().any(|f| f.severity == Severity::Error);
    Ok(i32::from(has_error))
}

/// `inline`: materialize the entry document with refs expanded.
fn bundle_inline(
    input: &Path,
    shown: &str,
    out: Option<&Path>,
    format: Option<DocFormat>,
) -> anyhow::Result<i32> {
    let ws = Arc::new(WorkspaceBuilder::new().build()?);
    ws.load_all(shown)?;
    let handle = ws.open(shown)?;

    let mut inliner = Inliner::new(&ws);
    inliner.precompute_participants();
    let value = inliner.inline_node(handle.doc().root());

    let fmt = format.unwrap_or_else(|| output::pick_doc_format(out, input));
    let text = match fmt {
        DocFormat::Json => value.to_json_pretty(),
        DocFormat::Yaml => value.to_yaml(),
    };
    output::write_or_stdout(&text, out)?;

    eprintln!(
        "inlined {} $ref(s), {} cycle(s) emitted as x-suspect-cyclic, {} unresolved",
        inliner.refs_inlined, inliner.cycles_inlined, inliner.errors.len()
    );
    let has_error = inliner.errors.iter().any(|f| f.severity == Severity::Error);
    Ok(i32::from(has_error))
}
