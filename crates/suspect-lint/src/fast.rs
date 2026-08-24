//! Fast-path rule dispatch: ONE hand-rolled traversal over the document
//! feeding every classifiable rule, replacing N generic JSONPath walks.
//!
//! The builtin OAS pack's `given` expressions fall into a handful of shapes
//! (`$.info`, `$.paths.*`, `$.paths.*.get.summary`, `$..['$ref']`, ...).
//! [`Plan`] classifies each enabled rule's queries against those shapes;
//! rules with any unclassifiable expression are reported in
//! [`PlanSlots::generic`] for the caller to evaluate through the JSONPath
//! engine instead.
//!
//! The traversal runs in two phases: a serial descent over the top levels
//! of the tree collecting independent *sections*, then a rayon
//! parallel sweep of all sections at once. Wide object wrappers are split
//! into finer sections so the sweep load balances: `paths` expands per
//! path item, and `components` expands one level (with a large `schemas`
//! map split per schema). Each section fills its queries' buckets;
//! findings are sorted and deduplicated by the caller, so section
//! ordering never affects output.
//!
//! Any document feature the walk cannot represent faithfully (YAML aliases,
//! merge keys, non-object `paths`) aborts execution and the caller falls
//! back to the fully generic engine; results are identical either way.

use rustc_hash::FxHashMap;
use suspect_low::{NodeRef, ValueKind};
use suspect_syntax::{SNode, SyntaxKind};

use crate::engine::Finding;
use crate::functions::apply;
use crate::rule::Rule;

/// A `given` expression shape the fast walk can produce.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum FastGiven {
    /// `$.key` — value of a top-level pair.
    RootKey(String),
    /// `$.paths` — the paths object itself.
    Paths,
    /// `$.paths.*` — every path-item value.
    PathItems,
    /// `$.paths.*.<method>` — operation values.
    Method(String),
    /// `$.paths.*.<method>.<prop>` — a named child of every operation.
    MethodProp { method: String, prop: String },
    /// `$..key` (also `$..['key']`) — values of every pair named `key`,
    /// at any depth.
    DescendantKey(String),
}

impl FastGiven {
    /// Classifies a JSONPath expression string; `None` when the shape is
    /// outside the fast walk's coverage.
    pub(crate) fn parse(expr: &str) -> Option<Self> {
        let rest = expr.strip_prefix('$')?;
        if let Some(key) = rest.strip_prefix("..") {
            let key = unquote_segment(key)?;
            if key.is_empty() || key == "*" {
                return None;
            }
            return Some(Self::DescendantKey(key.to_owned()));
        }
        let segs: Vec<&str> = rest.split('.').collect();
        // `rest` starts with "" (`$.k`); `$..k` was handled above.
        match segs.as_slice() {
            ["", "paths"] => Some(Self::Paths),
            ["", "paths", "*"] => Some(Self::PathItems),
            ["", "paths", "*", method] => {
                if method.is_empty() || *method == "*" || method.starts_with('[') {
                    return None;
                }
                Some(Self::Method((*method).to_owned()))
            }
            ["", "paths", "*", method, prop] => {
                let prop = unquote_segment(prop)?;
                if method.is_empty() || *method == "*" || prop.is_empty() || prop == "*" {
                    return None;
                }
                Some(Self::MethodProp {
                    method: (*method).to_owned(),
                    prop: prop.to_owned(),
                })
            }
            ["", key] => {
                let key = unquote_segment(key)?;
                if key.is_empty() || key == "*" {
                    None
                } else {
                    Some(Self::RootKey(key.to_owned()))
                }
            }
            _ => None,
        }
    }
}

/// `$..['k']` / `$..["k"]` spellings; passes plain names through.
fn unquote_segment(seg: &str) -> Option<&str> {
    if let Some(inner) = seg.strip_prefix('[') {
        let inner = inner.strip_suffix(']')?;
        let q = inner.chars().next()?;
        if (q == '\'' || q == '"') && inner.len() >= 2 && inner.ends_with(q) {
            return Some(&inner[1..inner.len() - 1]);
        }
        return None;
    }
    Some(seg)
}

/// Bucket index keyed by a `$.paths.*.<method>[.<prop>]` shape.
type MethodIdx = FxHashMap<(Box<[u8]>, Option<Box<[u8]>>), Vec<usize>>;

/// A compiled execution plan: the distinct classified givens plus lookup
/// tables the traversal uses to route nodes into per-given buckets.
pub(crate) struct Plan {
    givens: Vec<FastGiven>,
    /// bucket index by top-level key (`$.key` givens).
    root_idx: FxHashMap<Box<[u8]>, Vec<usize>>,
    /// bucket index by descendant key (`$..key` givens).
    desc_idx: FxHashMap<Box<[u8]>, Vec<usize>>,
    /// bucket index by `(method, prop)` / `(method, None)` under `$.paths.*`.
    method_idx: MethodIdx,
    wants_paths: bool,
    wants_path_items: bool,
    wants_methods: bool,
}

/// A plan plus each rule's slot indices into the shared bucket list.
pub(crate) struct PlanSlots {
    plan: Plan,
    /// Per-rule list of bucket indices (parallel to `rule.given`); empty
    /// for rules routed to the generic engine.
    slots: Vec<Vec<usize>>,
    /// Indices (into the rule slice given to [`Plan::compile`]) of rules
    /// with unclassifiable queries.
    pub(crate) generic: Vec<usize>,
}

impl Plan {
    /// Compiles a plan over every enabled rule. Rules whose `given`
    /// expressions all classify are served by the fast walk; rules with any
    /// unclassifiable expression land in [`PlanSlots::generic`].
    pub(crate) fn compile(rules: &[&Rule]) -> PlanSlots {
        let mut givens: Vec<FastGiven> = Vec::new();
        let mut slots: Vec<Vec<usize>> = Vec::with_capacity(rules.len());
        let mut generic: Vec<usize> = Vec::new();
        for (ri, rule) in rules.iter().enumerate() {
            let mut mine = Vec::with_capacity(rule.given.len());
            let mut classifiable = true;
            for expr in &rule.given_exprs {
                let Some(given) = FastGiven::parse(expr) else {
                    classifiable = false;
                    break;
                };
                let idx = match givens.iter().position(|g| g == &given) {
                    Some(i) => i,
                    None => {
                        givens.push(given);
                        givens.len() - 1
                    }
                };
                mine.push(idx);
            }
            if classifiable {
                slots.push(mine);
            } else {
                slots.push(Vec::new());
                generic.push(ri);
            }
        }
        let mut plan = Plan {
            givens,
            root_idx: FxHashMap::default(),
            desc_idx: FxHashMap::default(),
            method_idx: FxHashMap::default(),
            wants_paths: false,
            wants_path_items: false,
            wants_methods: false,
        };
        for (i, g) in plan.givens.iter().enumerate() {
            match g {
                FastGiven::RootKey(k) => {
                    plan.root_idx
                        .entry(k.as_bytes().into())
                        .or_default()
                        .push(i);
                }
                FastGiven::DescendantKey(k) => {
                    plan.desc_idx
                        .entry(k.as_bytes().into())
                        .or_default()
                        .push(i);
                }
                FastGiven::Method(m) => {
                    plan.wants_methods = true;
                    plan.method_idx
                        .entry((m.as_bytes().into(), None))
                        .or_default()
                        .push(i);
                }
                FastGiven::MethodProp { method, prop } => {
                    plan.wants_methods = true;
                    plan.method_idx
                        .entry((method.as_bytes().into(), Some(prop.as_bytes().into())))
                        .or_default()
                        .push(i);
                }
                FastGiven::Paths => plan.wants_paths = true,
                FastGiven::PathItems => plan.wants_path_items = true,
            }
        }
        PlanSlots {
            plan,
            slots,
            generic,
        }
    }

    fn new_buckets<'d>(&self) -> Buckets<'d> {
        Buckets(vec![Vec::new(); self.givens.len()])
    }
}

/// Wrapper so parallel phases can pass whole bucket sets around.
pub(crate) struct Buckets<'d>(Vec<Vec<NodeRef<'d>>>);

impl PlanSlots {
    /// Runs the two-phase traversal over `root` (the document root).
    ///
    /// Returns `None` when the document contains something the fast walk
    /// does not faithfully represent and the caller must fall back to the
    /// generic engine.
    pub(crate) fn execute<'d>(&self, root: NodeRef<'d>) -> Option<(Buckets<'d>, PtrMap)> {
        let profile = std::env::var_os("SUSPECT_PROFILE").is_some();
        let t0 = profile.then(std::time::Instant::now);
        let root = root.resolved();
        if root.kind() != ValueKind::Object {
            return None;
        }
        let plan = &self.plan;

        // Phase A (serial): descend the top two levels, collecting
        // independent work sections.
        enum Section<'d> {
            /// plain subtree: descendant-key routing only
            Plain(NodeRef<'d>),
            /// a `paths.*` item: method/prop routing plus descendant routing
            PathItem(NodeRef<'d>),
        }
        let mut sections: Vec<Section<'d>> = Vec::new();
        let mut seed = plan.new_buckets();
        let root_start = root.syntax().start_byte();
        let mut ptrs = PtrMap::new(root_start);
        for (key_node, value) in root.syntax().mapping_entries() {
            let kb = key_node.scalar_bytes();
            if kb == b"<<" {
                return None; // merge keys need generic merge semantics
            }
            let Some(vnode) = value else { continue };
            let vref = NodeRef::new(vnode);
            ptrs.record_key(&vnode, root_start, kb);
            if let Some(idxs) = plan.root_idx.get(kb) {
                for &i in idxs {
                    seed.0[i].push(vref);
                }
            }
            if kb == b"paths" && (plan.wants_paths || plan.wants_path_items || plan.wants_methods) {
                let resolved = vref.resolved();
                if resolved.kind() != ValueKind::Object {
                    return None; // paths must be an object for the fast shape
                }
                if plan.wants_paths {
                    for (i, g) in plan.givens.iter().enumerate() {
                        if matches!(g, FastGiven::Paths) {
                            seed.0[i].push(vref);
                        }
                    }
                }
                for (path_key, item) in resolved.syntax().mapping_entries() {
                    if path_key.scalar_bytes() == b"<<" {
                        return None;
                    }
                    let Some(iv) = item else { continue };
                    let iref = NodeRef::new(iv);
                    ptrs.record_key(&iv, resolved.syntax().start_byte(), path_key.scalar_bytes());
                    if plan.wants_path_items {
                        for (i, g) in plan.givens.iter().enumerate() {
                            if matches!(g, FastGiven::PathItems) {
                                seed.0[i].push(iref);
                            }
                        }
                    }
                    sections.push(Section::PathItem(iref));
                }
            } else if kb == b"components" && vref.resolved().kind() == ValueKind::Object {
                // Expand one level: each `components.<group>` entry becomes
                // its own section so phase B parallelizes across them
                // instead of walking all of `components` as one serial-ish
                // giant subtree. The wrapper node itself is skipped as a
                // section but still gets its pointer edge recorded.
                let groups = vref.resolved();
                let comp_start = groups.syntax().start_byte();
                for (gk, gv) in groups.syntax().mapping_entries() {
                    let gkb = gk.scalar_bytes();
                    if gkb == b"<<" {
                        return None; // merge keys need generic merge semantics
                    }
                    let Some(gv) = gv else { continue };
                    let gref = NodeRef::new(gv);
                    ptrs.record_key(&gv, comp_start, gkb);
                    // `schemas` dominates wide documents: split it one more
                    // level into per-schema sections when it is large.
                    let rg = gref.resolved();
                    if gkb == b"schemas"
                        && rg.kind() == ValueKind::Object
                        && rg.syntax().mapping_entries().len() > 8
                    {
                        let schemas_start = rg.syntax().start_byte();
                        for (sk, sv) in rg.syntax().mapping_entries() {
                            let skb = sk.scalar_bytes();
                            if skb == b"<<" {
                                return None;
                            }
                            let Some(sv) = sv else { continue };
                            ptrs.record_key(&sv, schemas_start, skb);
                            sections.push(Section::Plain(NodeRef::new(sv)));
                        }
                    } else {
                        sections.push(Section::Plain(gref));
                    }
                }
            } else {
                sections.push(Section::Plain(vref));
            }
        }
        if let Some(t) = t0 {
            eprintln!(
                "[lint fast] phase A {:.2} ms, {} sections",
                t.elapsed().as_secs_f64() * 1000.0,
                sections.len()
            );
        }

        // Phase B (parallel): one task per section.
        let parts: Vec<Option<(Buckets<'d>, PtrMap)>> = sections
            .par_iter()
            .map(|section| {
                let mut b = plan.new_buckets();
                let mut p = PtrMap::new(root_start);
                let ok = match section {
                    Section::Plain(n) => walk_descendants(plan, *n, &mut b.0, &mut p),
                    Section::PathItem(n) => walk_path_item(plan, *n, &mut b.0, &mut p),
                };
                ok.then_some((b, p))
            })
            .collect();
        let n = seed.0.len();
        let mut merged = seed;
        for part in parts {
            let (mut pb, pp) = part?;
            for i in 0..n {
                merged.0[i].append(&mut pb.0[i]);
            }
            ptrs.merge(pp);
        }
        if let Some(t) = t0 {
            eprintln!(
                "[lint fast] phase B {:.2} ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
        Some((merged, ptrs))
    }

    /// Applies every classifiable rule to its buckets, appending findings.
    ///
    /// Rules are independent (bucket/pointer reads are read-only), so the
    /// work runs in parallel. Jobs are `(rule, bucket)` pairs emitted in
    /// rule-major order and large buckets are split into fixed-size node
    /// chunks, which keeps load balanced when a few descendant buckets
    /// dwarf the rest; collecting in job order reproduces the original
    /// rule order so output stays deterministic.
    pub(crate) fn apply<'d>(
        &self,
        rules: &[&Rule],
        buckets: &Buckets<'d>,
        ptrs: &PtrMap,
        out: &mut Vec<Finding<'d>>,
    ) {
        const CHUNK: usize = 512;
        let jobs: Vec<(&Rule, &[NodeRef<'d>])> = rules
            .iter()
            .zip(&self.slots)
            .flat_map(|(rule, mine)| mine.iter().map(move |&idx| (*rule, &*buckets.0[idx])))
            .collect();
        let per_job: Vec<Vec<Finding<'d>>> = jobs
            .par_iter()
            .map(|(rule, nodes)| {
                if nodes.len() <= CHUNK {
                    let mut found: Vec<Finding<'d>> = Vec::new();
                    for node in *nodes {
                        apply(rule, *node, ptrs, &mut found);
                    }
                    found
                } else {
                    nodes
                        .par_chunks(CHUNK)
                        .map(|chunk| {
                            let mut found: Vec<Finding<'d>> = Vec::new();
                            for node in chunk {
                                apply(rule, *node, ptrs, &mut found);
                            }
                            found
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                        .flatten()
                        .collect()
                }
            })
            .collect();
        out.extend(per_job.into_iter().flatten());
    }
}

use rayon::prelude::*;

/// Parent-pointer edges recorded during the walk: node start byte ->
/// (parent start byte, addressing token). Lets findings resolve their JSON
/// pointer with O(depth) hash lookups instead of the O(depth x width)
/// `path_from_root` rescan, which dominated apply time on wide documents.
pub(crate) struct PtrMap {
    root_start: usize,
    edges: FxHashMap<usize, (usize, Token)>,
}

#[derive(Debug)]
enum Token {
    Key(Box<[u8]>),
    Idx(u32),
}

impl PtrMap {
    fn new(root_start: usize) -> Self {
        Self {
            root_start,
            edges: FxHashMap::default(),
        }
    }

    fn record_key(&mut self, child: &SNode<'_>, parent_start: usize, key: &[u8]) {
        self.edges
            .entry(child.start_byte())
            .or_insert((parent_start, Token::Key(key.into())));
    }

    fn record_idx(&mut self, child: &SNode<'_>, parent_start: usize, idx: u32) {
        self.edges
            .entry(child.start_byte())
            .or_insert((parent_start, Token::Idx(idx)));
    }

    fn merge(&mut self, mut other: Self) {
        if self.edges.len() < other.edges.len() {
            std::mem::swap(self, &mut other);
        }
        for (k, v) in other.edges.drain() {
            self.edges.entry(k).or_insert(v);
        }
    }

    /// The pair key addressing `node` in its parent object, if recorded.
    /// O(1); `None` when unvisited or addressed by array index.
    pub(crate) fn own_key(&self, node: &NodeRef<'_>) -> Option<&[u8]> {
        let cur = node.resolved().syntax().start_byte();
        match self.edges.get(&cur)? {
            (_, Token::Key(k)) => Some(k),
            _ => None,
        }
    }

    /// JSON pointer from the document root to `node`, or `None` when the
    /// node was not visited by the walk (caller falls back).
    pub(crate) fn pointer_for(&self, node: &NodeRef<'_>) -> Option<suspect_low::Pointer> {
        let mut tokens: Vec<Box<str>> = Vec::new();
        let mut cur = node.resolved().syntax().start_byte();
        loop {
            if cur == self.root_start {
                tokens.reverse();
                return Some(suspect_low::Pointer::from_tokens(tokens));
            }
            let (parent, token) = self.edges.get(&cur)?;
            tokens.push(match token {
                Token::Key(k) => String::from_utf8_lossy(k).into_owned().into_boxed_str(),
                Token::Idx(i) => i.to_string().into_boxed_str(),
            });
            cur = *parent;
        }
    }
}

/// Routes one `paths.*` item: operation/method buckets plus the full
/// descendant sweep. Returns `false` on unrepresentable content.
fn walk_path_item<'d>(
    plan: &Plan,
    item: NodeRef<'d>,
    buckets: &mut [Vec<NodeRef<'d>>],
    ptrs: &mut PtrMap,
) -> bool {
    if plan.wants_methods {
        let op_obj = item.resolved();
        if op_obj.kind() == ValueKind::Object {
            for (method_key, op) in op_obj.syntax().mapping_entries() {
                let mb = method_key.scalar_bytes();
                if mb == b"<<" {
                    return false;
                }
                let Some(op) = op else { continue };
                let op_ref = NodeRef::new(op);
                ptrs.record_key(&op, op_obj.syntax().start_byte(), mb);
                if let Some(idxs) = plan.method_idx.get(&(Box::<[u8]>::from(mb), None)) {
                    for &i in idxs {
                        buckets[i].push(op_ref);
                    }
                }
                let op_resolved = op_ref.resolved();
                if op_resolved.kind() != ValueKind::Object {
                    continue;
                }
                for (prop_key, prop_val) in op_resolved.syntax().mapping_entries() {
                    if prop_key.scalar_bytes() == b"<<" {
                        return false;
                    }
                    let Some(pv) = prop_val else { continue };
                    ptrs.record_key(
                        &pv,
                        op_resolved.syntax().start_byte(),
                        prop_key.scalar_bytes(),
                    );
                    if let Some(idxs) = plan.method_idx.get(&(
                        Box::<[u8]>::from(mb),
                        Some(Box::<[u8]>::from(prop_key.scalar_bytes())),
                    )) {
                        for &i in idxs {
                            buckets[i].push(NodeRef::new(pv));
                        }
                    }
                }
            }
        }
    }
    walk_descendants(plan, item, buckets, ptrs)
}

/// Depth-first collection of `$..key` buckets across a whole subtree,
/// recording parent-pointer edges as it goes. Iterative over an explicit
/// stack; bails on aliases and merge keys.
fn walk_descendants<'d>(
    plan: &Plan,
    section: NodeRef<'d>,
    buckets: &mut [Vec<NodeRef<'d>>],
    ptrs: &mut PtrMap,
) -> bool {
    let top = *section.resolved().syntax();
    let mut stack: Vec<(SNode<'_>, usize)> = vec![(top, usize::MAX)];
    while let Some((node, parent_start)) = stack.pop() {
        match node.kind() {
            SyntaxKind::Alias => return false,
            SyntaxKind::Mapping => {
                let ms = node.start_byte();
                for (key_node, value) in node.mapping_entries() {
                    let kb = key_node.scalar_bytes();
                    if kb == b"<<" {
                        return false;
                    }
                    if let Some(v) = value {
                        ptrs.record_key(&v, ms, kb);
                        stack.push((v, ms));
                    }
                    if let Some(idxs) = plan.desc_idx.get(kb)
                        && let Some(v) = value
                    {
                        for &i in idxs {
                            buckets[i].push(NodeRef::new(v));
                        }
                    }
                    stack.push((key_node, ms));
                }
            }
            SyntaxKind::Sequence => {
                let ss = node.start_byte();
                for (idx, item) in node.sequence_items().into_iter().enumerate() {
                    ptrs.record_idx(&item, ss, idx as u32);
                    stack.push((item, ss));
                }
            }
            _ => {}
        }
        // link wrapper nodes to their structural parent so pointer climbs
        // that pass through wrappers still terminate at the mapping/sequence
        let _ = parent_start;
    }
    true
}

impl PtrMap {
    /// An empty map: every lookup misses and callers fall back to the
    /// slow pointer computation (used by the generic engine path).
    pub(crate) fn empty() -> Self {
        Self {
            root_start: usize::MAX,
            edges: FxHashMap::default(),
        }
    }
}
