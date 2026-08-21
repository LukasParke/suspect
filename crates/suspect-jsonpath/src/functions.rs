//! RFC 9535 §2.4 function extensions: `length`, `count`, `match`, `search`,
//! `value`.


use suspect_low::{NodeRef, ValueKind};

use crate::ast::{FArg, FunctionCall};
use crate::eval::{self, CVal};

/// Result of a function-extension call.
pub(crate) enum FRes<'d> {
    /// `match`/`search`.
    Logical(bool),
    /// `length`/`count`/`value`; `None` is the RFC's Nothing.
    Value(Option<CVal<'d>>),
}

/// A function argument reduced to what the built-in functions consume.
enum Arg<'d> {
    /// From filter queries (possibly non-singular): a nodelist.
    Nodes(Vec<NodeRef<'d>>),
    /// From literals, singular queries, or nested calls: a value or Nothing.
    Val(Option<CVal<'d>>),
}

/// Dispatches a parsed function call against the current filter node.
pub(crate) fn call<'d>(
    f: &FunctionCall,
    current: NodeRef<'d>,
    root: NodeRef<'d>,
) -> FRes<'d> {
    debug_assert!(f.args.len() == 1 || matches!(f.name, crate::ast::FuncName::Match | crate::ast::FuncName::Search));
    match f.name {
        FuncName::Length => FRes::Value(length(arg(f, 0, current, root))),
        FuncName::Count => FRes::Value(count(arg(f, 0, current, root))),
        FuncName::Value => FRes::Value(value(&f.args[0], current, root)),
        FuncName::Match | FuncName::Search => {
            // The pattern was compiled at parse time; arity and literal-ness
            // were validated there too.
            let re = match &f.regex {
                Some(re) => re,
                None => return FRes::Logical(false),
            };
            let subject = match arg(f, 0, current, root) {
                Arg::Nodes(ns) if ns.len() == 1 => node_str(ns[0]),
                Arg::Nodes(_) => None,
                Arg::Val(Some(CVal::Str(s))) => Some(s.into_owned()),
                Arg::Val(Some(CVal::Node(n))) => node_str(n),
                _ => None,
            };
            FRes::Logical(match subject {
                Some(s) => re.is_match(&s),
                None => false,
            })
        }
    }
}

use crate::ast::FuncName;

/// String content of a scalar node, if any.
fn node_str(node: NodeRef<'_>) -> Option<String> {
    match node.kind() {
        ValueKind::Str => node.as_str().map(str::to_owned),
        _ => None,
    }
}

/// Reduces argument `i` to a nodelist-or-value without forcing either shape.
fn arg<'d>(f: &FunctionCall, i: usize, current: NodeRef<'d>, root: NodeRef<'d>) -> Arg<'d> {
    match &f.args[i] {
        FArg::Query(q) => {
            let base = if q.absolute { root } else { current };
            Arg::Nodes(eval::run_query(&q.segments, base, root))
        }
        FArg::Comparable(c) => Arg::Val(eval::eval_comparable(c, current, root)),
        FArg::Logical(e) => Arg::Val(Some(CVal::Bool(eval::eval_bool(e, current, root)))),
    }
}

/// `length`: number of nodes in a nodelist, codepoints in a string, or
/// elements in an array; Nothing otherwise.
fn length(a: Arg<'_>) -> Option<CVal<'static>> {
    let n = match a {
        // A single string/array node reports its own length (task spec:
        // "string length in scalars or array length"); any other single
        // node counts as one; multi-node nodelists count nodes.
        Arg::Nodes(ns) => match ns.as_slice() {
            [] => return None,
            [only] => match only.kind() {
                ValueKind::Str => only.as_str()?.chars().count(),
                ValueKind::Array => only.items().len(),
                _ => 1,
            },
            _ => ns.len(),
        },
        Arg::Val(None) => return None,
        Arg::Val(Some(CVal::Str(s))) => s.chars().count(),
        Arg::Val(Some(CVal::Node(node))) => match node.kind() {
            ValueKind::Str => node.as_str()?.chars().count(),
            ValueKind::Array => node.items().len(),
            _ => return None,
        },
        _ => return None,
    };
    Some(CVal::Num(n as f64))
}

/// `count`: size of the argument's nodelist; Nothing for non-query args.
fn count(a: Arg<'_>) -> Option<CVal<'static>> {
    match a {
        Arg::Nodes(ns) => Some(CVal::Num(ns.len() as f64)),
        _ => None,
    }
}

/// `value`: unwraps a single-node nodelist to its value; Nothing when zero
/// or multiple nodes match.
fn value<'d>(arg: &FArg, current: NodeRef<'d>, root: NodeRef<'d>) -> Option<CVal<'d>> {
    let nodes = match arg {
        FArg::Query(q) => {
            let base = if q.absolute { root } else { current };
            eval::run_query(&q.segments, base, root)
        }
        FArg::Comparable(crate::ast::Comparable::Query(q)) => {
            let base = if q.absolute { root } else { current };
            eval::resolve_singular(q, base).into_iter().collect()
        }
        _ => return None,
    };
    match nodes.as_slice() {
        [only] => Some(eval::node_to_cval(*only)),
        _ => None,
    }
}
