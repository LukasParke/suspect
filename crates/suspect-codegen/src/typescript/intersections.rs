//! Admission guard for simultaneous native representations at one wire position.
//!
//! This does not reconcile representations or prove schema satisfiability. It
//! conservatively rejects numeric conflicts until a shared conversion plan can
//! represent every applicable branch. Containers do not hide these conflicts.

use std::collections::{BTreeMap, BTreeSet};

use suspect_ir::contract::{Contract, SchemaId};

use super::{DiagnosticKind, Expr, Literal, ModelDiagnostic, ModelSymbol, Primitive};

const MAX_RELATIONS: usize = 100_000;
const MAX_DEPTH: usize = 256;

/// Inspect the same planned expressions used by declarations and codecs.
pub(super) fn check(contract: &Contract, symbols: &[ModelSymbol]) -> Vec<ModelDiagnostic> {
    let mut checker = Checker {
        contract,
        symbols: symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol))
            .collect(),
        visited: BTreeSet::new(),
        compared: BTreeSet::new(),
        disjoint: BTreeMap::new(),
        findings: BTreeMap::new(),
        exhausted: false,
    };
    for symbol in symbols {
        checker.walk(&symbol.expression, &symbol.source, 0);
    }
    checker.findings.into_values().collect()
}

struct Checker<'a> {
    contract: &'a Contract,
    symbols: BTreeMap<&'a str, &'a ModelSymbol>,
    visited: BTreeSet<usize>,
    compared: BTreeSet<(usize, usize)>,
    disjoint: BTreeMap<(usize, usize), bool>,
    findings: BTreeMap<(SchemaId, &'static str, String), ModelDiagnostic>,
    exhausted: bool,
}

fn located<'a>(mut expression: &'a Expr, mut source: &'a SchemaId) -> (&'a Expr, &'a SchemaId) {
    while let Expr::At(at, inner) = expression {
        expression = inner;
        source = at;
    }
    (expression, source)
}

/// Masks describe permitted native representations of a mathematical number.
/// Numeric singleton spellings are already canonical exact integers in the AST.
fn numeric(expression: &Expr) -> Option<(u8, Option<&str>)> {
    match expression {
        Expr::Primitive(primitive) => match primitive {
            Primitive::SafeInteger => Some((1, None)),
            Primitive::Integer => Some((2, None)),
            Primitive::Number => Some((4, None)),
            Primitive::AnyNumber => Some((7, None)),
            _ => None,
        },
        Expr::Literal(Literal::Integer { value, safe }) => {
            Some((if *safe { 1 } else { 2 }, Some(value)))
        }
        _ => None,
    }
}

// Native numeric alternatives share a single mathematical JSON domain.
fn domain(expression: &Expr) -> Option<u8> {
    match expression {
        Expr::Any => Some(63),
        Expr::Never => Some(0),
        Expr::Primitive(Primitive::Null) => Some(1),
        Expr::Primitive(Primitive::Boolean) | Expr::Literal(Literal::Boolean(_)) => Some(2),
        Expr::Primitive(Primitive::String) | Expr::Literal(Literal::String(_)) => Some(4),
        Expr::Primitive(_) | Expr::Literal(Literal::Integer { .. }) => Some(8),
        Expr::Object(_, _) => Some(16),
        Expr::Array(_) => Some(32),
        _ => None,
    }
}

impl<'a> Checker<'a> {
    fn report(&mut self, source: &SchemaId, code: &'static str, message: String) {
        let key = (source.clone(), code, message.clone());
        self.findings.entry(key).or_insert_with(|| ModelDiagnostic {
            source: source.clone(),
            at: self.contract.source_span(source).unwrap_or(0..0),
            code,
            kind: DiagnosticKind::Error,
            message,
        });
    }

    fn admit(&mut self, source: &SchemaId, depth: usize) -> bool {
        if self.exhausted {
            return false;
        }
        if depth > MAX_DEPTH
            || self.visited.len() + self.compared.len() + self.disjoint.len() >= MAX_RELATIONS
        {
            self.exhausted = true;
            self.report(source, "intersection-analysis-limit", format!(
                "native intersection analysis exceeds its {MAX_RELATIONS} relation or {MAX_DEPTH} depth budget; a complete shared representation plan is required before emission"
            ));
            return false;
        }
        true
    }

    fn walk(&mut self, expression: &'a Expr, source: &'a SchemaId, depth: usize) {
        let (expression, source) = located(expression, source);
        let identity = std::ptr::from_ref(expression) as usize;
        if self.visited.contains(&identity) || !self.admit(source, depth) {
            return;
        }
        self.visited.insert(identity);
        match expression {
            Expr::Intersection(items) => {
                for (index, left) in items.iter().enumerate() {
                    for right in &items[index + 1..] {
                        self.compare(left, source, right, source, depth + 1);
                    }
                    self.walk(left, source, depth + 1);
                }
            }
            Expr::Union(items) => {
                for item in items {
                    self.walk(item, source, depth + 1);
                }
            }
            Expr::Object(fields, extra) => {
                for field in fields {
                    self.walk(&field.expression, &field.source, depth + 1);
                }
                if let Some(extra) = extra {
                    self.walk(extra, source, depth + 1);
                }
            }
            Expr::Array(item) => self.walk(item, source, depth + 1),
            Expr::Reference(name) => {
                if let Some(symbol) = self.symbols.get(name.as_str()).copied() {
                    self.walk(&symbol.expression, &symbol.source, depth + 1);
                }
            }
            Expr::Any | Expr::Never | Expr::Primitive(_) | Expr::Literal(_) => {}
            Expr::At(_, _) => unreachable!("located unwraps source annotations"),
        }
    }

    /// A small proof of mutually exclusive domains/literals. Unknown or
    /// recursive relationships remain potentially applicable; this is not a
    /// second validator and does not infer satisfiability from bounds.
    fn proven_disjoint(
        &mut self,
        left: &'a Expr,
        left_source: &'a SchemaId,
        right: &'a Expr,
        right_source: &'a SchemaId,
        depth: usize,
    ) -> bool {
        let (left, left_source) = located(left, left_source);
        let (right, right_source) = located(right, right_source);
        let a = std::ptr::from_ref(left) as usize;
        let b = std::ptr::from_ref(right) as usize;
        let key = (a.min(b), a.max(b));
        if let Some(known) = self.disjoint.get(&key) {
            return *known;
        }
        if !self.admit(right_source, depth) {
            return false;
        }
        // A recursive proof cannot assume its own conclusion.
        self.disjoint.insert(key, false);
        let result = match (left, right) {
            (Expr::Reference(name), _) => {
                self.symbols
                    .get(name.as_str())
                    .copied()
                    .is_some_and(|symbol| {
                        self.proven_disjoint(
                            &symbol.expression,
                            &symbol.source,
                            right,
                            right_source,
                            depth + 1,
                        )
                    })
            }
            (_, Expr::Reference(name)) => {
                self.symbols
                    .get(name.as_str())
                    .copied()
                    .is_some_and(|symbol| {
                        self.proven_disjoint(
                            left,
                            left_source,
                            &symbol.expression,
                            &symbol.source,
                            depth + 1,
                        )
                    })
            }
            (Expr::Union(items), _) => items.iter().all(|item| {
                self.proven_disjoint(item, left_source, right, right_source, depth + 1)
            }),
            (_, Expr::Union(items)) => items
                .iter()
                .all(|item| self.proven_disjoint(left, left_source, item, right_source, depth + 1)),
            (Expr::Intersection(items), _) => items.iter().any(|item| {
                self.proven_disjoint(item, left_source, right, right_source, depth + 1)
            }),
            (_, Expr::Intersection(items)) => items
                .iter()
                .any(|item| self.proven_disjoint(left, left_source, item, right_source, depth + 1)),
            (Expr::Literal(Literal::Boolean(a)), Expr::Literal(Literal::Boolean(b))) => a != b,
            (Expr::Literal(Literal::String(a)), Expr::Literal(Literal::String(b))) => a != b,
            (
                Expr::Literal(Literal::Integer { value: a, .. }),
                Expr::Literal(Literal::Integer { value: b, .. }),
            ) => a != b,
            _ => matches!((domain(left), domain(right)), (Some(a), Some(b)) if a & b == 0),
        };
        self.disjoint.insert(key, result);
        result
    }

    fn compare(
        &mut self,
        left: &'a Expr,
        left_source: &'a SchemaId,
        right: &'a Expr,
        right_source: &'a SchemaId,
        depth: usize,
    ) {
        let (left, left_source) = located(left, left_source);
        let (right, right_source) = located(right, right_source);
        if std::ptr::eq(left, right)
            || matches!(left, Expr::Any | Expr::Never)
            || matches!(right, Expr::Any | Expr::Never)
        {
            return;
        }
        let a = std::ptr::from_ref(left) as usize;
        let b = std::ptr::from_ref(right) as usize;
        let key = (a.min(b), a.max(b));
        if self.compared.contains(&key) || !self.admit(right_source, depth) {
            return;
        }
        self.compared.insert(key);

        if let (Some((a, a_literal)), Some((b, b_literal))) = (numeric(left), numeric(right)) {
            // Distinct exact singleton values cannot apply simultaneously.
            if a & b == 0 && !(a_literal.is_some() && b_literal.is_some() && a_literal != b_literal)
            {
                self.report(right_source, "numeric-intersection-representation", format!(
                    "simultaneous constraints require incompatible native numeric representations `{}` and `{}` at one instance position (other source: {}#{}); a shared representation plan is not implemented",
                    left.render(), right.render(), left_source.document(), left_source.pointer()
                ));
            }
            return;
        }

        match (left, right) {
            (Expr::Reference(name), _) => {
                if let Some(symbol) = self.symbols.get(name.as_str()).copied() {
                    self.compare(
                        &symbol.expression,
                        &symbol.source,
                        right,
                        right_source,
                        depth + 1,
                    );
                }
            }
            (_, Expr::Reference(name)) => {
                if let Some(symbol) = self.symbols.get(name.as_str()).copied() {
                    self.compare(
                        left,
                        left_source,
                        &symbol.expression,
                        &symbol.source,
                        depth + 1,
                    );
                }
            }
            // An incompatible potentially applicable alternative is refused,
            // rather than distributing an exponential product of branches or
            // selecting a convenient representation without schema evidence.
            (Expr::Intersection(items) | Expr::Union(items), _) => {
                for item in items {
                    self.compare(item, left_source, right, right_source, depth + 1);
                }
            }
            (_, Expr::Intersection(items) | Expr::Union(items)) => {
                for item in items {
                    self.compare(left, left_source, item, right_source, depth + 1);
                }
            }
            (Expr::Array(a), Expr::Array(b)) => {
                self.compare(a, left_source, b, right_source, depth + 1);
            }
            (Expr::Object(a, a_extra), Expr::Object(b, b_extra)) => {
                let a_fields: BTreeMap<_, _> =
                    a.iter().map(|field| (field.name.as_str(), field)).collect();
                let b_fields: BTreeMap<_, _> =
                    b.iter().map(|field| (field.name.as_str(), field)).collect();
                // A required shared discriminator with disjoint values
                // proves these object alternatives cannot both apply. Avoid
                // inventing numeric conflicts between those alternatives.
                if a_fields.iter().any(|(name, field)| {
                    b_fields.get(name).is_some_and(|other| {
                        (field.required || other.required)
                            && self.proven_disjoint(
                                &field.expression,
                                &field.source,
                                &other.expression,
                                &other.source,
                                depth + 1,
                            )
                    })
                }) {
                    return;
                }
                for (name, field) in &a_fields {
                    if let Some(other) = b_fields.get(name) {
                        self.compare(
                            &field.expression,
                            &field.source,
                            &other.expression,
                            &other.source,
                            depth + 1,
                        );
                    } else if let Some(extra) = b_extra {
                        self.compare(
                            &field.expression,
                            &field.source,
                            extra,
                            right_source,
                            depth + 1,
                        );
                    }
                }
                for (name, field) in &b_fields {
                    if !a_fields.contains_key(name)
                        && let Some(extra) = a_extra
                    {
                        self.compare(
                            extra,
                            left_source,
                            &field.expression,
                            &field.source,
                            depth + 1,
                        );
                    }
                }
                if let (Some(a), Some(b)) = (a_extra, b_extra) {
                    self.compare(a, left_source, b, right_source, depth + 1);
                }
            }
            _ => {}
        }
    }
}
