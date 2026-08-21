//! Cycle census: classify the reference loops reachable from a document's
//! edges as legal schema recursion or illegal (non-recursive) looping.
//!
//! Graph model: nodes are a document's [`RefEdge`]s; edge `E1` has an
//! out-edge to `E2` when `E2`'s containing mapping lies inside `E1`'s local
//! target subtree (byte-range containment). External/plain-name edges are
//! terminal here: cross-file cycles are the resolver's job, not the census's.
//!
//! The walk is an iterative DFS with an explicit frame stack and on-stack
//! set — never native recursion. Depth is capped at [`CENSUS_CAP`]; paths
//! past the cap are abandoned (documented approximation, not an error).
//! Cycles are deduplicated by canonical rotation before classification.
//!
//! Classification: a cycle is [`CycleKind::LegalRecursion`] only if **every**
//! edge in it sits under a recursion-point keyword (`properties`, `items`,
//! `$defs`, `allOf`, ...) somewhere on its ancestor chain; anything else is
//! [`CycleKind::Illegal`] — e.g. a loop threaded through `required` arrays.

use suspect_syntax::SyntaxKind;
use std::collections::HashSet;
use std::ops::Range;
use rustc_hash::{FxHashMap, FxHashSet};
use crate::edges::{ParsedRef, RefEdge};
use crate::resolve::Step;
use crate::workspace::Workspace;

/// Maximum DFS depth of a census walk.
pub(crate) const CENSUS_CAP: usize = 512;

/// Keywords under which a self-reference describes recursive data rather
/// than an unresolvable cycle.
pub(crate) const RECURSION_POINTS: &[&str] = &[
    "properties",
    "items",
    "prefixItems",
    "additionalProperties",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "$defs",
    "definitions",
    "contains",
    "if",
    "then",
    "else",
    "dependentSchemas",
    "patternProperties",
];

/// Result of a per-document cycle census.
#[derive(Debug, Clone, Default)]
pub struct CycleReport {
    /// All unique cycles found, in discovery order.
    pub cycles: Vec<Cycle>,
}

/// One detected cycle.
#[derive(Debug, Clone)]
pub struct Cycle {
    /// The loop's steps in visit order (canonical rotation).
    pub steps: Box<[Step]>,
    /// Legal recursion vs. illegal loop.
    pub kind: CycleKind,
}

/// Whether a cycle represents legitimate recursive schema structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleKind {
    /// Every edge in the cycle nests under a recursion-point keyword.
    LegalRecursion,
    /// The loop passes through a non-recursion position; resolving it can
    /// never terminate in a value.
    Illegal,
}

/// Builds the edge-successor graph for one document and enumerates its
/// unique cycles.
pub(crate) fn census(ws: &Workspace, d: crate::DocId) -> CycleReport {
    let edges = ws.edges_of(d);
    let meta = ws.meta_of(d);
    let n = edges.len();
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];

    if let Some(doc) = ws.extend_doc(d) {
        // 1. Wanted target pointers (local, non-root).
        let mut wanted: FxHashSet<Box<str>> = FxHashSet::default();
        for edge in edges.iter() {
            if let ParsedRef::Local(p) = &edge.parsed
                && !p.is_root() {
                    wanted.insert(p.to_path().into_boxed_str());
                }
        }

        // 2. ONE pre-order descent resolving every wanted pointer to its
        //    byte range, carrying the incremental path (no per-edge walks).
        let mut resolved: FxHashMap<Box<str>, Range<usize>> = FxHashMap::default();
        let mut path: Vec<Box<str>> = Vec::new();
        // Frames: (node, own-token-plen, own-token). On pop the token is
        // appended, reconstructing this node's exact path.
        let mut stack: Vec<(suspect_syntax::SNode<'_>, usize, Option<Box<str>>)> =
            vec![(*doc.root().syntax(), 0, None)];
        while let Some((node, tok_plen, tok)) = stack.pop() {
            path.truncate(tok_plen);
            if let Some(t) = tok {
                path.push(t);
            }
            let key: Box<str> = join_path(&path).into_boxed_str();
            if wanted.contains(&key) && !resolved.contains_key(&key) {
                resolved.insert(key, node.byte_range());
            }
            let plen = path.len();
            match node.kind() {
                // transparent containers: descend without touching the path
                SyntaxKind::Stream | SyntaxKind::Document => {
                    let inner = node.content();
                    if inner.raw_kind() != node.raw_kind() {
                        stack.push((inner, plen, None));
                    }
                }
                _ if matches!(
                    node.raw_kind(),
                    "block_node" | "flow_node" | "_value" | "block_sequence_item"
                ) =>
                {
                    let inner = node.content();
                    stack.push((inner, plen, None));
                }
                SyntaxKind::Mapping => {
                    for (k, v) in node.mapping_entries() {
                        if let Some(v) = v {
                            let tok = String::from_utf8_lossy(k.scalar_bytes()).to_string().into_boxed_str();
                            stack.push((v, plen, Some(tok)));
                        }
                    }
                }
                SyntaxKind::Sequence => {
                    for (idx, item) in node.sequence_items().into_iter().enumerate() {
                        let tok = idx.to_string().into_boxed_str();
                        stack.push((item, plen, Some(tok)));
                    }
                }
                _ => {}
            }
        }

        // 3. Successor graph via laminar containment: mappings sorted by
        //    start; every mapping starting inside [ts, te) is contained.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_unstable_by_key(|&j| meta.mapping_ranges[j].start);
        for (i, edge) in edges.iter().enumerate() {
            let tr = match &edge.parsed {
                ParsedRef::Local(p) if !p.is_root() => {
                    match resolved.get(p.to_path().as_str()) {
                        Some(r) => r.clone(),
                        None => continue,
                    }
                }
                _ => continue,
            };
            let first = order.partition_point(|&j| meta.mapping_ranges[j].start < tr.start);
            for &j in &order[first..] {
                let mr_start = meta.mapping_ranges[j].start;
                if mr_start >= tr.end {
                    break;
                }
                succ[i].push(j);
            }
        }
    }

    let mut done = vec![false; n];
    let mut onstack = vec![false; n];
    let mut frames: Vec<(usize, usize)> = Vec::new();
    let mut seen: HashSet<Box<[Step]>> = HashSet::default();
    let mut cycles: Vec<Cycle> = Vec::new();

    for start in 0..n {
        if done[start] {
            continue;
        }
        frames.push((start, 0));
        onstack[start] = true;
        while let Some(&(node, _)) = frames.last() {
            let ci = frames.last().map_or(0, |f| f.1);
            if ci >= succ[node].len() {
                frames.pop();
                onstack[node] = false;
                done[node] = true;
                continue;
            }
            let child = succ[node][ci];
            if let Some(frame) = frames.last_mut() {
                frame.1 += 1;
            } else {
                break;
            }
            if onstack[child] {
                let pos = frames.iter().position(|&(nd, _)| nd == child).unwrap_or_default();
                let pairs: Vec<(Step, usize)> = frames[pos..]
                    .iter()
                    .map(|&(nd, _)| (Step { doc: d, at: edges[nd].at.clone() }, nd))
                    .collect();
                let canon = canonical_rotation(&pairs);
                if seen.insert(canon.clone()) {
                    let kind = classify(&pairs, &edges);
                    cycles.push(Cycle { steps: canon, kind });
                }
            } else if !done[child] && frames.len() < CENSUS_CAP {
                onstack[child] = true;
                frames.push((child, 0));
            }
        }
    }

    CycleReport { cycles }
}

/// Joins path tokens into pointer-path form for wanted-set matching.
fn join_path(tokens: &[Box<str>]) -> String {
    let mut out = String::new();
    for t in tokens {
        out.push('/');
        out.push_str(t);
    }
    out
}

fn canonical_rotation(pairs: &[(Step, usize)]) -> Box<[Step]> {
    let mut min = 0usize;
    for (i, (step, _)) in pairs.iter().enumerate() {
        if step < &pairs[min].0 {
            min = i;
        }
    }
    let mut out = Vec::with_capacity(pairs.len());
    out.extend(pairs[min..].iter().map(|(s, _)| s.clone()));
    out.extend(pairs[..min].iter().map(|(s, _)| s.clone()));
    out.into_boxed_slice()
}

fn classify(pairs: &[(Step, usize)], edges: &[RefEdge]) -> CycleKind {
    for (_, idx) in pairs {
        let edge = &edges[*idx];
        let legal = edge.path.tokens().iter().any(|t| RECURSION_POINTS.contains(&&**t));
        if !legal {
            return CycleKind::Illegal;
        }
    }
    CycleKind::LegalRecursion
}
