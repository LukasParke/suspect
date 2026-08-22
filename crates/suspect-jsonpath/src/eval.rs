//! Query evaluation: segment application, filters, normalization.

use std::borrow::Cow;

use suspect_low::{NodeRef, ValueKind};

use crate::ast::{
    Comparable, Comparator, FunctionCall, Lit, LogicalExpr, QueryAst, Segment, Selector, Testable,
};
use crate::functions;

/// Compiled query: parsed once, evaluated many times.
///
/// Full RFC 9535 selector coverage: name, wildcard (`*`), array index
/// (negative counts from the end), slice (`start:end:step`), and filter
/// (`?...`) selectors, in both child (`.x`) and descendant (`..x`) segments.
/// All five RFC 9535 function extensions are supported in filters:
/// `length()`, `count()`, `match()`, `search()` (regexes precompiled at
/// parse time), and `value()`. Non-singular queries are accepted anywhere a
/// function argument allows them (`count(@..*)`).
#[derive(Debug)]
pub struct Path {
    pub(crate) segments: Vec<Segment>,
}

impl Path {
    /// Canonical form of the query (used as a cache key). Debug formatting
    /// is deterministic for the parsed AST, which is all a cache key needs.
    #[must_use]
    pub fn as_key(&self) -> String {
        format!("{:?}", self.segments)
    }
}

impl Path {
    /// Parses an RFC 9535 JSONPath query string.
    ///
    /// # Errors
    /// Returns [`crate::PathError`] with the offending offset for any syntactic
    /// violation (bad selector, zero step, unterminated bracket, trailing
    /// characters, unknown function, invalid regex literal).
    pub fn parse(input: &str) -> Result<Path, crate::PathError> {
        let ast = crate::parser::parse(input)?;
        Ok(Path {
            segments: ast.segments,
        })
    }

    /// Runs the query against `root` and returns the normalized node list:
    /// deduplicated and ordered by document position (byte offset; ties keep
    /// first-discovery order). `$` inside filters refers to `root`.
    #[must_use]
    pub fn query<'d>(&self, root: NodeRef<'d>) -> NodeList<'d> {
        NodeList {
            nodes: run_query(&self.segments, root, root),
        }
    }
}

/// Result of [`Path::query`]: nodes in document order, deduplicated by
/// source position.
pub struct NodeList<'d> {
    pub(crate) nodes: Vec<NodeRef<'d>>,
}

impl<'d> NodeList<'d> {
    /// Iterates over the matched nodes in normalized (document) order.
    pub fn iter(&self) -> impl Iterator<Item = NodeRef<'d>> + '_ {
        self.nodes.iter().copied()
    }

    /// Number of matched nodes after deduplication.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    /// True when the query matched nothing.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    /// Returns the node at index `i` in document order.
    pub fn get(&self, i: usize) -> Option<NodeRef<'d>> {
        self.nodes.get(i).copied()
    }

    #[must_use]
    /// Returns the first matched node, if any.
    pub fn first(&self) -> Option<NodeRef<'d>> {
        self.nodes.first().copied()
    }
}

impl<'d> IntoIterator for NodeList<'d> {
    type Item = NodeRef<'d>;
    type IntoIter = std::vec::IntoIter<NodeRef<'d>>;
    fn into_iter(self) -> Self::IntoIter {
        self.nodes.into_iter()
    }
}

// ---- segment machinery ---------------------------------------------------

/// Applies segments from `base`, with `root` available to `$` in filters,
/// then normalizes (dedup by position, document order).
pub(crate) fn run_query<'d>(
    segments: &[Segment],
    base: NodeRef<'d>,
    root: NodeRef<'d>,
) -> Vec<NodeRef<'d>> {
    let nodes = run_segments(segments, base, root);
    let mut seen = std::collections::HashSet::with_capacity(nodes.len());
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        if seen.insert(n.byte_range()) {
            out.push(n);
        }
    }
    out.sort_by_key(|n| n.byte_range().start);
    out
}

fn run_segments<'d>(
    segments: &[Segment],
    base: NodeRef<'d>,
    root: NodeRef<'d>,
) -> Vec<NodeRef<'d>> {
    let mut current: Vec<NodeRef<'d>> = vec![base];
    for seg in segments {
        let mut next = Vec::new();
        for node in &current {
            if seg.descendant {
                descend(seg, *node, root, &mut next);
            } else {
                apply_segment(seg, *node, root, &mut next);
            }
        }
        current = next;
    }
    current
}

/// Descendant segment: apply selectors at every node of the subtree rooted
/// at `node` (inclusive), via an explicit stack — no recursion, so deeply
/// nested documents cannot overflow the stack.
fn descend<'d>(seg: &Segment, node: NodeRef<'d>, root: NodeRef<'d>, out: &mut Vec<NodeRef<'d>>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        apply_segment(seg, n, root, out);
        push_children(n, &mut stack);
    }
}

fn apply_segment<'d>(
    seg: &Segment,
    node: NodeRef<'d>,
    root: NodeRef<'d>,
    out: &mut Vec<NodeRef<'d>>,
) {
    for sel in &seg.selectors {
        match sel {
            Selector::Name(name) => {
                if let Some(v) = node.get(name) {
                    out.push(v);
                }
            }
            Selector::Wildcard => push_children(node, out),
            Selector::Index(i) => {
                if let Some(v) = index_node(node, *i) {
                    out.push(v);
                }
            }
            Selector::Slice { start, end, step } => {
                slice_node(node, *start, *end, *step, out);
            }
            Selector::Filter(expr) => {
                // Filters iterate children in document order.
                let mut children = Vec::new();
                push_children(node, &mut children);
                for child in children {
                    if eval_bool(expr, child, root) {
                        out.push(child);
                    }
                }
            }
        }
    }
}

/// Array index with RFC negative-index semantics (`-1` = last element).
pub(crate) fn index_node<'d>(node: NodeRef<'d>, i: i64) -> Option<NodeRef<'d>> {
    let items = node.items();
    let len = items.len() as i64;
    let idx = if i < 0 { len + i } else { i };
    if idx < 0 || idx >= len {
        return None;
    }
    items.get(idx as usize).copied()
}

/// RFC 9535 appendix A.4 array-slice normalization: defaults `0/:/1`,
/// negative bounds measured from the end and clamped into range, `step < 0`
/// walks backwards with an exclusive lower bound of `-1`.
fn slice_node<'d>(
    node: NodeRef<'d>,
    start: Option<i64>,
    end: Option<i64>,
    step: i64,
    out: &mut Vec<NodeRef<'d>>,
) {
    let items = node.items();
    let len = items.len() as i64;
    if step > 0 {
        let s = clamp_bound(start.unwrap_or(0), len, 0, len);
        let e = clamp_bound(end.unwrap_or(len), len, 0, len);
        let mut i = s;
        while i < e {
            out.push(items[i as usize]);
            i += step;
        }
    } else {
        // step < 0 (step == 0 rejected at parse time). The default end of
        // -1 means "before the first element" and must NOT be
        // negative-shifted like a user-supplied bound.
        let s = clamp_bound(start.unwrap_or(len - 1), len, -1, len - 1);
        let e = match end {
            Some(v) => clamp_bound(v, len, -1, len - 1),
            None => -1,
        };
        let mut i = s;
        while i > e {
            out.push(items[i as usize]);
            i += step;
        }
    }
}

/// Measures negative `v` from `len`, then clamps into `[lo, hi]`.
fn clamp_bound(v: i64, len: i64, lo: i64, hi: i64) -> i64 {
    let v = if v < 0 { v + len } else { v };
    v.clamp(lo, hi)
}

/// Child nodes of a container in document order: object values, array
/// items; scalars have no children.
fn push_children<'d>(node: NodeRef<'d>, out: &mut Vec<NodeRef<'d>>) {
    match node.kind() {
        ValueKind::Object => {
            for entry in node.entries() {
                if let Some(v) = entry.value {
                    out.push(v);
                }
            }
        }
        ValueKind::Array => out.extend(node.items()),
        _ => {}
    }
}

// ---- expression evaluation ------------------------------------------------

/// A comparison operand value. `Node` marks a non-scalar ValueType
/// (object/array); it never compares equal to a scalar, only to itself.
#[derive(Clone)]
pub(crate) enum CVal<'d> {
    Num(f64),
    Str(Cow<'d, str>),
    Bool(bool),
    Null,
    Node(NodeRef<'d>),
}

/// Evaluates a filter expression to its logical result for `current`.
pub(crate) fn eval_bool(expr: &LogicalExpr, current: NodeRef<'_>, root: NodeRef<'_>) -> bool {
    match expr {
        LogicalExpr::Or(a, b) => eval_bool(a, current, root) || eval_bool(b, current, root),
        LogicalExpr::And(a, b) => eval_bool(a, current, root) && eval_bool(b, current, root),
        LogicalExpr::Not(a) => !eval_bool(a, current, root),
        LogicalExpr::Compare(l, op, r) => compare(
            *op,
            eval_comparable(l, current, root),
            eval_comparable(r, current, root),
        ),
        LogicalExpr::Test(t) => match t {
            Testable::Query(q) => test_query(q, current, root),
            Testable::Func(f) => test_function(f, current, root),
        },
    }
}

/// Existence semantics of a query used as a bare test: singular queries
/// exist when they resolve; general queries exist when their nodelist
/// (deduplicated) is non-empty.
fn test_query(q: &QueryAst, current: NodeRef<'_>, root: NodeRef<'_>) -> bool {
    let base = if q.absolute { root } else { current };
    if q.is_singular() {
        resolve_singular(q, base).is_some()
    } else {
        !run_query(&q.segments, base, root).is_empty()
    }
}

/// Resolves a singular query (all child segments of name/index selectors)
/// to its single target, or nothing.
pub(crate) fn resolve_singular<'d>(q: &QueryAst, base: NodeRef<'d>) -> Option<NodeRef<'d>> {
    debug_assert!(q.is_singular());
    let mut node = base;
    for seg in &q.segments {
        for sel in &seg.selectors {
            node = match sel {
                Selector::Name(name) => node.get(name)?,
                Selector::Index(i) => index_node(node, *i)?,
                _ => return None,
            };
        }
    }
    Some(node)
}

/// Evaluates a comparison operand to a value or Nothing.
pub(crate) fn eval_comparable<'d>(
    c: &Comparable,
    current: NodeRef<'d>,
    root: NodeRef<'d>,
) -> Option<CVal<'d>> {
    match c {
        Comparable::Lit(lit) => Some(lit_to_cval(lit)),
        Comparable::Query(q) => {
            let base = if q.absolute { root } else { current };
            resolve_singular(q, base).map(node_to_cval)
        }
        Comparable::Func(f) => match functions::call(f, current, root) {
            functions::FRes::Logical(b) => Some(CVal::Bool(b)),
            functions::FRes::Value(v) => v,
        },
    }
}

fn lit_to_cval(lit: &Lit) -> CVal<'static> {
    match lit {
        Lit::Int(i) => CVal::Num(*i as f64),
        Lit::Float(f) => CVal::Num(*f),
        Lit::Str(s) => CVal::Str(Cow::Owned(s.clone())),
        Lit::Bool(b) => CVal::Bool(*b),
        Lit::Null => CVal::Null,
    }
}

/// Scalar view of a node for comparison purposes; containers keep identity
/// via [`CVal::Node`] so they never compare equal to scalars.
pub(crate) fn node_to_cval<'d>(node: NodeRef<'d>) -> CVal<'d> {
    match node.kind() {
        ValueKind::Str => CVal::Str(match node.as_str() {
            Some(s) => Cow::Borrowed(s),
            None => Cow::Owned(String::from_utf8_lossy(node.scalar_bytes()).into_owned()),
        }),
        ValueKind::Int | ValueKind::Float => CVal::Num(node.as_f64().unwrap_or(f64::NAN)),
        ValueKind::Bool => CVal::Bool(node.as_bool().unwrap_or(false)),
        ValueKind::Null => CVal::Null,
        ValueKind::Object | ValueKind::Array => CVal::Node(node),
    }
}

/// RFC 9535 §2.2 comparison table. Nothing compares equal only to Nothing;
/// ordering holds only within numbers and within strings (codepoint order —
/// UTF-8 byte order equals codepoint order).
fn compare<'d>(op: Comparator, l: Option<CVal<'d>>, r: Option<CVal<'d>>) -> bool {
    match (l, r) {
        (None, None) => matches!(op, Comparator::Eq),
        (None, Some(_)) | (Some(_), None) => false,
        (Some(a), Some(b)) => match op {
            Comparator::Eq => eqv(&a, &b),
            Comparator::Ne => !eqv(&a, &b),
            _ => ord(op, &a, &b),
        },
    }
}

fn eqv(l: &CVal<'_>, r: &CVal<'_>) -> bool {
    match (l, r) {
        (CVal::Num(a), CVal::Num(b)) => a == b,
        (CVal::Str(a), CVal::Str(b)) => a == b,
        (CVal::Bool(a), CVal::Bool(b)) => a == b,
        (CVal::Null, CVal::Null) => true,
        (CVal::Node(a), CVal::Node(b)) => a.byte_range() == b.byte_range(),
        _ => false,
    }
}

fn ord(op: Comparator, l: &CVal<'_>, r: &CVal<'_>) -> bool {
    let ord = match (l, r) {
        (CVal::Num(a), CVal::Num(b)) => a.partial_cmp(b),
        (CVal::Str(a), CVal::Str(b)) => Some(a.cmp(b)),
        _ => None,
    };
    match (op, ord) {
        (Comparator::Lt, Some(o)) => o.is_lt(),
        (Comparator::Le, Some(o)) => o.is_le(),
        (Comparator::Gt, Some(o)) => o.is_gt(),
        (Comparator::Ge, Some(o)) => o.is_ge(),
        _ => false,
    }
}

/// Existence/logical result of a function call used as a bare test:
/// `match`/`search` yield their boolean; numeric results and single values
/// exist when they were produced; nodelists exist when non-empty.
fn test_function(f: &FunctionCall, current: NodeRef<'_>, root: NodeRef<'_>) -> bool {
    match functions::call(f, current, root) {
        functions::FRes::Logical(b) => b,
        functions::FRes::Value(v) => v.is_some(),
    }
}
