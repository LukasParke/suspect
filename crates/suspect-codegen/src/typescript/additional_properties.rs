//! Conservative value-representation proof for named fields and index signatures.
//!
//! Additional-property assertions never apply to named fields. We can use a
//! native index signature only when each named field's decoded representation
//! fits its advertised value type. Native compiler assignability alone misses
//! optional-property/map counterexamples, so this checks represented values too.

use std::collections::{BTreeMap, BTreeSet};

use suspect_ir::contract::{Contract, SchemaId};

use super::{DiagnosticKind, Expr, FieldPlan, Literal, ModelDiagnostic, ModelSymbol, Primitive};

const MAX_VISITS: usize = 100_000;
const MAX_DEPTH: usize = 256;

pub(super) fn check(contract: &Contract, symbols: &[ModelSymbol]) -> Vec<ModelDiagnostic> {
    let mut checker = Checker {
        contract,
        symbols: symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), &symbol.expression))
            .collect(),
        walked: BTreeSet::new(),
        proven: BTreeMap::new(),
        remaining: MAX_VISITS,
        exhausted: false,
        findings: Vec::new(),
    };
    for symbol in symbols {
        checker.walk(&symbol.expression, &symbol.source, 0);
        if checker.exhausted {
            break;
        }
    }
    checker.findings
}

struct Checker<'a> {
    contract: &'a Contract,
    symbols: BTreeMap<&'a str, &'a Expr>,
    walked: BTreeSet<usize>,
    proven: BTreeMap<(usize, usize), bool>,
    remaining: usize,
    exhausted: bool,
    findings: Vec<ModelDiagnostic>,
}

fn located<'a>(mut expression: &'a Expr, mut source: &'a SchemaId) -> (&'a Expr, &'a SchemaId) {
    while let Expr::At(at, inner) = expression {
        expression = inner;
        source = at;
    }
    (expression, source)
}

fn unlocated(mut expression: &Expr) -> &Expr {
    while let Expr::At(_, inner) = expression {
        expression = inner;
    }
    expression
}

impl<'a> Checker<'a> {
    fn spend(&mut self, depth: usize, amount: usize) -> bool {
        if self.exhausted || depth > MAX_DEPTH || amount > self.remaining {
            self.exhausted = true;
            return false;
        }
        self.remaining -= amount;
        true
    }

    fn report(&mut self, source: &SchemaId, code: &'static str, message: String) {
        self.findings.push(ModelDiagnostic {
            source: source.clone(),
            at: self.contract.source_span(source).unwrap_or(0..0),
            code,
            kind: DiagnosticKind::Error,
            message,
        });
    }

    fn limit(&mut self, source: &SchemaId) {
        self.report(source, "additional-properties-analysis-limit", format!("index-signature representation analysis exceeds {MAX_VISITS} visits or depth {MAX_DEPTH}; complete admission could not be established"));
    }

    fn walk(&mut self, expression: &'a Expr, source: &'a SchemaId, depth: usize) {
        let (expression, source) = located(expression, source);
        let key = std::ptr::from_ref(expression) as usize;
        if self.walked.contains(&key) || self.exhausted {
            return;
        }
        if !self.spend(depth, 1) {
            self.limit(source);
            return;
        }
        self.walked.insert(key);
        match expression {
            Expr::Object(fields, extra) => {
                if let Some(extra) = extra {
                    for field in fields {
                        if !self.assignable(&field.expression, extra, depth + 1) {
                            let at = source.child("additionalProperties");
                            if self.exhausted {
                                self.limit(&at);
                                return;
                            }
                            self.report(&at, "typed-extra-fields-representation", format!("named property {:?} is not proven compatible with the native index-signature value type; a separate extra-field representation is required before this model can be emitted", field.name));
                        }
                    }
                }
                for field in fields {
                    self.walk(&field.expression, &field.source, depth + 1);
                    if self.exhausted {
                        return;
                    }
                }
                if let Some(extra) = extra {
                    self.walk(extra, source, depth + 1);
                }
            }
            Expr::Union(items) | Expr::Intersection(items) => {
                for item in items {
                    self.walk(item, source, depth + 1);
                    if self.exhausted {
                        return;
                    }
                }
            }
            Expr::Array(item) => self.walk(item, source, depth + 1),
            Expr::Reference(name) => {
                if let Some(target) = self.symbols.get(name.as_str()).copied() {
                    self.walk(target, source, depth + 1);
                }
            }
            Expr::Any | Expr::Never | Expr::Primitive(_) | Expr::Literal(_) => {}
            Expr::At(_, _) => unreachable!("located unwraps source identities"),
        }
    }

    fn assignable(&mut self, source: &'a Expr, target: &'a Expr, depth: usize) -> bool {
        if !self.spend(depth, 1) {
            return false;
        }
        let source = unlocated(source);
        let target = unlocated(target);
        let a = std::ptr::from_ref(source) as usize;
        let b = std::ptr::from_ref(target) as usize;
        if a == b || matches!(source, Expr::Never) || matches!(target, Expr::Any) {
            return true;
        }
        if let Some(proved) = self.proven.get(&(a, b)) {
            return *proved;
        }
        // An unfinished recursive relation is unknown. Only an independent
        // sufficient proof may promote it; cycles cannot assume their result.
        self.proven.insert((a, b), false);
        let result = match (source, target) {
            (Expr::Reference(a), Expr::Reference(b)) if a == b => true,
            (Expr::Reference(name), _) => self
                .symbols
                .get(name.as_str())
                .copied()
                .is_some_and(|source| self.assignable(source, target, depth + 1)),
            (_, Expr::Reference(name)) => self
                .symbols
                .get(name.as_str())
                .copied()
                .is_some_and(|target| self.assignable(source, target, depth + 1)),
            (Expr::Union(items), _) => items
                .iter()
                .all(|item| self.assignable(item, target, depth + 1)),
            (_, Expr::Intersection(items)) => items
                .iter()
                .all(|item| self.assignable(source, item, depth + 1)),
            (_, Expr::Union(items)) => items
                .iter()
                .any(|item| self.assignable(source, item, depth + 1)),
            (Expr::Intersection(items), _) => items
                .iter()
                .any(|item| self.assignable(item, target, depth + 1)),
            (Expr::Primitive(a), Expr::Primitive(b)) => {
                a == b
                    || *b == Primitive::AnyNumber
                        && matches!(
                            a,
                            Primitive::SafeInteger | Primitive::Integer | Primitive::Number
                        )
            }
            (Expr::Literal(Literal::Boolean(_)), Expr::Primitive(Primitive::Boolean))
            | (Expr::Literal(Literal::String(_)), Expr::Primitive(Primitive::String))
            | (Expr::Literal(Literal::Integer { .. }), Expr::Primitive(Primitive::AnyNumber)) => {
                true
            }
            (Expr::Literal(Literal::Integer { safe, .. }), Expr::Primitive(kind)) => matches!(
                (safe, kind),
                (true, Primitive::SafeInteger) | (false, Primitive::Integer)
            ),
            (Expr::Literal(a), Expr::Literal(b)) => match (a, b) {
                (Literal::Boolean(a), Literal::Boolean(b)) => a == b,
                (Literal::String(a), Literal::String(b)) => a == b,
                (
                    Literal::Integer { value: a, safe: ar },
                    Literal::Integer { value: b, safe: br },
                ) => a == b && ar == br,
                _ => false,
            },
            (Expr::Array(source), Expr::Array(target)) => {
                self.assignable(source, target, depth + 1)
            }
            (Expr::Object(source, source_extra), Expr::Object(target, target_extra)) => self
                .objects(
                    source,
                    source_extra.as_deref(),
                    target,
                    target_extra.as_deref(),
                    depth + 1,
                ),
            _ => false,
        };
        self.proven.insert((a, b), result);
        result
    }

    fn objects(
        &mut self,
        source: &'a [FieldPlan],
        source_extra: Option<&'a Expr>,
        target: &'a [FieldPlan],
        target_extra: Option<&'a Expr>,
        depth: usize,
    ) -> bool {
        if !self.spend(depth, source.len().saturating_add(target.len())) {
            return false;
        }
        let source_fields: BTreeMap<_, _> = source
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        // TypeScript weak object targets require a common declared property
        // for a nonempty source without an index signature. Value compatibility
        // alone would otherwise admit declarations rejected by native tsc.
        if !target.is_empty()
            && target.iter().all(|field| !field.required)
            && target_extra.is_none()
            && !source.is_empty()
            && source_extra.is_none()
            && !target
                .iter()
                .any(|field| source_fields.contains_key(field.name.as_str()))
        {
            return false;
        }
        for target in target {
            if let Some(source) = source_fields.get(target.name.as_str()) {
                if target.required && !source.required
                    || !self.assignable(&source.expression, &target.expression, depth + 1)
                {
                    return false;
                }
            } else {
                // A map type does not guarantee presence, and an optional target
                // field may still occur with a value from the source's map.
                if target.required {
                    return false;
                }
                if let Some(extra) = source_extra
                    && !self.assignable(extra, &target.expression, depth + 1)
                {
                    return false;
                }
            }
        }
        if let Some(extra) = target_extra {
            for field in source {
                if !self.assignable(&field.expression, extra, depth + 1) {
                    return false;
                }
            }
            if let Some(source) = source_extra
                && !self.assignable(source, extra, depth + 1)
            {
                return false;
            }
        }
        true
    }
}
