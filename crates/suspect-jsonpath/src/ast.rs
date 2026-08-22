//! AST for RFC 9535 JSONPath queries.
//!
//! Compiled once by [`crate::parser`]; evaluated many times by
//! [`crate::eval`]. Regexes for `match()`/`search()` are compiled at parse
//! time so query evaluation never pays regex-compilation cost.

/// A parsed (absolute or relative) query: `$seg seg...` or `@seg seg...`.
#[derive(Debug, Clone)]
pub(crate) struct QueryAst {
    /// `true` when rooted at `$`, `false` for filter-relative `@`.
    pub absolute: bool,
    pub segments: Vec<Segment>,
}

impl QueryAst {
    /// A singular query per RFC 9535 §2.3.3: every segment is a child
    /// segment consisting only of name/index selectors.
    pub fn is_singular(&self) -> bool {
        self.segments.iter().all(|s| {
            !s.descendant
                && s.selectors
                    .iter()
                    .all(|sel| matches!(sel, Selector::Name(_) | Selector::Index(_)))
        })
    }
}

/// One segment: a (possibly empty at eval time) set of selectors applied to
/// every input node, optionally descended through the whole subtree first.
#[derive(Debug, Clone)]
pub(crate) struct Segment {
    /// `..` descendant segment: apply selectors at every node of the subtree.
    pub descendant: bool,
    pub selectors: Vec<Selector>,
}

#[derive(Debug, Clone)]
pub(crate) enum Selector {
    Name(Box<str>),
    Wildcard,
    Index(i64),
    Slice {
        start: Option<i64>,
        end: Option<i64>,
        step: i64,
    },
    Filter(LogicalExpr),
}

#[derive(Debug, Clone)]
pub(crate) enum LogicalExpr {
    Or(Box<LogicalExpr>, Box<LogicalExpr>),
    And(Box<LogicalExpr>, Box<LogicalExpr>),
    Not(Box<LogicalExpr>),
    Compare(Comparable, Comparator, Comparable),
    /// Existence test: bare singular/non-singular query or function call.
    Test(Testable),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comparator {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone)]
pub(crate) enum Testable {
    Query(QueryAst),
    Func(FunctionCall),
}

#[derive(Debug, Clone)]
pub(crate) enum Comparable {
    Lit(Lit),
    /// Must be singular; enforced at parse time.
    Query(QueryAst),
    Func(FunctionCall),
}

#[derive(Debug, Clone)]
pub(crate) enum Lit {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FuncName {
    Length,
    Count,
    Match,
    Search,
    Value,
}

/// A function-extension call. `regex` is precompiled for `match`/`search`.
#[derive(Debug, Clone)]
pub(crate) struct FunctionCall {
    pub name: FuncName,
    pub args: Vec<FArg>,
    #[allow(dead_code)] // read via Debug/tests; evaluation reuses `name`
    pub regex: Option<regex::Regex>,
}

/// Function argument: RFC 9535 `function-argument`. Queries keep their full
/// shape (`count(@..*)`, `value(@..a)` take non-singular queries); other
/// logical expressions are kept whole so comparisons can appear as args.
#[derive(Debug, Clone)]
pub(crate) enum FArg {
    Comparable(Comparable),
    Query(QueryAst),
    Logical(LogicalExpr),
}
