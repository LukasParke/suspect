//! Schema evaluation: walks a compiled [`Program`] against an instance with
//! annotation tracking for `unevaluatedProperties`/`unevaluatedItems`.
//!
//! Recursion is bounded by [`Config::max_depth`]; exceeding it produces a
//! clean [`SchemaError`] instead of a stack overflow (recursive `$ref`
//! schemas against deep instances are the intended use). Array elements and
//! object members are iterated in loops, so only genuine schema descent
//! costs stack frames (~2 per instance-nesting level).

use smallvec::SmallVec;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::Schema;
use crate::compile::{Kind, Num, Prg};
use crate::errors::SchemaError;
use crate::keywords::{arrays, composition, formats, numeric, objects, refs, strings, types};

/// One token of the instance location under construction.
#[derive(Clone, Copy)]
pub(crate) enum Tok<'d> {
    Key(&'d str),
    Idx(usize),
}

/// Zero-allocation instance path stack; a [`Pointer`] is materialized only
/// when an error is actually emitted.
pub(crate) struct Stack<'d>(SmallVec<[Tok<'d>; 24]>);

impl<'d> Stack<'d> {
    pub(crate) fn new() -> Self {
        Self(SmallVec::new())
    }
    pub(crate) fn push_key(&mut self, k: &'d str) {
        self.0.push(Tok::Key(k));
    }
    pub(crate) fn push_idx(&mut self, i: usize) {
        self.0.push(Tok::Idx(i));
    }
    pub(crate) fn pop(&mut self) {
        self.0.pop();
    }
    pub(crate) fn to_pointer(&self) -> Pointer {
        Pointer::from_tokens(
            self.0
                .iter()
                .map(|t| match t {
                    Tok::Key(k) => (*k).to_owned().into_boxed_str(),
                    Tok::Idx(i) => i.to_string().into_boxed_str(),
                })
                .collect(),
        )
    }
}

/// Annotations produced by applying one program to one instance: the
/// property names and array indices that application evaluated at that
/// instance level (2020-12 §11).
#[derive(Clone, Default)]
pub(crate) struct Ann<'d> {
    props: SmallVec<[&'d str; 8]>,
    idxs: SmallVec<[u32; 8]>,
}

impl<'d> Ann<'d> {
    pub(crate) fn prop(&mut self, k: &'d str) {
        if !self.props.contains(&k) {
            self.props.push(k);
        }
    }
    pub(crate) fn idx(&mut self, i: usize) {
        let i = i as u32;
        if !self.idxs.contains(&i) {
            self.idxs.push(i);
        }
    }
    pub(crate) fn merge(&mut self, other: Ann<'d>) {
        for p in other.props {
            self.prop(p);
        }
        for i in other.idxs {
            self.idxs.push(i);
        }
    }
}

/// Union of annotations per instance node, keyed by the node's byte-range
/// start (unique within a document). Applications record into this table
/// only when they succeed, per §11.2 ("successfully applied").
#[derive(Default)]
pub(crate) struct Masks<'d> {
    map: rustc_hash::FxHashMap<usize, Ann<'d>>,
}

impl<'d> Masks<'d> {
    pub(crate) fn record(&mut self, inst: NodeRef<'d>, ann: Ann<'d>) {
        self.map
            .entry(inst.byte_range().start)
            .or_default()
            .merge(ann);
    }
    pub(crate) fn has_prop(&self, inst: NodeRef<'d>, k: &str) -> bool {
        self.map
            .get(&inst.byte_range().start)
            .is_some_and(|a| a.props.contains(&k))
    }
    pub(crate) fn has_idx(&self, inst: NodeRef<'d>, i: usize) -> bool {
        self.map
            .get(&inst.byte_range().start)
            .is_some_and(|a| a.idxs.contains(&(i as u32)))
    }
}

/// Per-`validate` evaluation context.
pub(crate) struct Ctx<'a, 'd> {
    pub sch: &'a Schema<'d>,
    /// Error cap for this run (`max_errors`, or 1 for `validate_first`).
    pub cap: usize,
    pub masks: Masks<'d>,
    pub out: Vec<SchemaError>,
    pub aborted: bool,
    pub depth: usize,
    /// Dynamic scope for `$dynamicRef`: `(anchor, program)` pairs pushed
    /// outermost-first (RFC 3093 basic semantics).
    pub dyn_scope: Vec<(std::rc::Rc<str>, Prg<'d>)>,
}

impl<'a, 'd> Ctx<'a, 'd> {
    pub(crate) fn emit(&mut self, st: &Stack<'d>, at: &Pointer, message: String) {
        if self.aborted {
            return;
        }
        if self.cap != 0 && self.out.len() >= self.cap {
            self.aborted = true;
            return;
        }
        self.out.push(SchemaError {
            instance_path: st.to_pointer(),
            schema_path: at.clone(),
            message,
        });
    }

    /// Runs a trial branch with error reporting diverted; returns the result
    /// plus the diverted errors (used by `anyOf`/`oneOf`/`not`/`if`, where
    /// failing branches must not pollute the report).
    pub(crate) fn divert<F, R>(&mut self, f: F) -> (R, Vec<SchemaError>)
    where
        F: FnOnce(&mut Self) -> R,
    {
        let real = std::mem::take(&mut self.out);
        self.out = Vec::new();
        let r = f(self);
        let trial = std::mem::replace(&mut self.out, real);
        (r, trial)
    }

    pub(crate) fn first_msg(errs: &[SchemaError]) -> String {
        errs.first()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "invalid".into())
    }
}

/// Result of applying one program to one instance.
pub(crate) struct Out<'d> {
    pub ok: bool,
    pub ann: Ann<'d>,
}

impl<'d> Out<'d> {
    fn fail() -> Self {
        Self {
            ok: false,
            ann: Ann::default(),
        }
    }
}

pub(crate) fn eval<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    prog: &Prg<'d>,
    inst: NodeRef<'d>,
    st: &mut Stack<'d>,
) -> Out<'d> {
    ctx.depth += 1;
    let r = if ctx.depth > ctx.sch.config().max_depth {
        ctx.emit(
            st,
            &prog.path,
            format!(
                "schema evaluation depth exceeds {}",
                ctx.sch.config().max_depth
            ),
        );
        Out::fail()
    } else {
        run(ctx, prog, inst, st)
    };
    ctx.depth -= 1;
    r
}

fn run<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    prog: &Prg<'d>,
    inst: NodeRef<'d>,
    st: &mut Stack<'d>,
) -> Out<'d> {
    if ctx.aborted {
        return Out::fail();
    }
    let mut ok = true;
    let mut ann = Ann::default();

    // $dynamicAnchor scope frame (RFC 3093): programs declaring anchors
    // enter the dynamic scope while they are being evaluated.
    let pushed = prog.dyn_anchors.len();
    for name in &prog.dyn_anchors {
        ctx.dyn_scope.push((name.clone(), prog.clone()));
    }

    for chk in &prog.checks {
        if ctx.aborted {
            break;
        }
        match &chk.kind {
            Kind::Always(true) => {}
            Kind::Always(false) => {
                ctx.emit(st, &chk.at, "value matches `false` schema".into());
                ok = false;
            }
            Kind::Type(bits) => {
                let k = inst.kind();
                let float_is_int =
                    k == ValueKind::Float && inst.as_f64().is_some_and(|f| f.fract() == 0.0);
                if !bits.matches(k, float_is_int) {
                    ctx.emit(
                        st,
                        &chk.at,
                        format!(
                            "value has type {}, expected `{}`",
                            types::kind_name(k),
                            types::type_names(*bits)
                        ),
                    );
                    ok = false;
                }
            }
            Kind::Enum(vals) => {
                if !vals.iter().any(|v| types::value_eq(inst, *v, 0)) {
                    ctx.emit(st, &chk.at, "value does not match any `enum` entry".into());
                    ok = false;
                }
            }
            Kind::Const(v) => {
                if !types::value_eq(inst, *v, 0) {
                    ctx.emit(st, &chk.at, "value does not equal the `const` value".into());
                    ok = false;
                }
            }
            Kind::MultipleOf(d) => ok &= numeric::check_multiple_of(ctx, st, &chk.at, &inst, *d),
            Kind::Maximum(b, ex) => {
                ok &= numeric::check_bound(ctx, st, &chk.at, &inst, *b, *ex, true);
            }
            Kind::Minimum(b, ex) => {
                ok &= numeric::check_bound(ctx, st, &chk.at, &inst, *b, *ex, false);
            }
            Kind::MaxLength(n) => ok &= strings::check_length(ctx, st, &chk.at, &inst, *n, true),
            Kind::MinLength(n) => ok &= strings::check_length(ctx, st, &chk.at, &inst, *n, false),
            Kind::Pattern(re) => ok &= strings::check_pattern(ctx, st, &chk.at, &inst, re),
            Kind::Items(sub, skip) => {
                ok &= arrays::check_items(ctx, st, &chk.at, &inst, sub, *skip, &mut ann);
            }
            Kind::PrefixItems(subs) => {
                ok &= arrays::check_prefix_items(ctx, st, &chk.at, &inst, subs, &mut ann);
            }
            Kind::Contains { schema, min, max } => {
                ok &= arrays::check_contains(ctx, st, &chk.at, &inst, schema, *min, *max, &mut ann);
            }
            Kind::Properties(subs) => {
                ok &= objects::check_properties(ctx, st, &inst, subs, &mut ann);
            }
            Kind::PatternProperties(subs) => {
                ok &= objects::check_pattern_properties(ctx, st, &inst, subs, &mut ann);
            }
            Kind::AdditionalProperties {
                except_keys,
                except_patterns,
                schema,
            } => {
                ok &= objects::check_additional(
                    ctx,
                    st,
                    &chk.at,
                    &inst,
                    except_keys,
                    except_patterns,
                    schema.as_ref(),
                    &mut ann,
                );
            }
            Kind::PropertyNames(sub) => {
                ok &= objects::check_property_names(ctx, st, &chk.at, &inst, sub);
            }
            Kind::DependentSchemas(subs) => {
                ok &= objects::check_dependent_schemas(ctx, st, &inst, subs, &mut ann);
            }
            Kind::DependentRequired(reqs) => {
                ok &= objects::check_dependent_required(ctx, st, &chk.at, &inst, reqs);
            }
            Kind::Required(names) => {
                ok &= objects::check_required(ctx, st, &chk.at, &inst, names, &mut ann);
            }
            Kind::UnevaluatedProperties(sub) => {
                ok &= objects::check_unevaluated_props(
                    ctx,
                    st,
                    &chk.at,
                    &inst,
                    sub.as_ref(),
                    &mut ann,
                );
            }
            Kind::UnevaluatedItems(sub) => {
                ok &= arrays::check_unevaluated_items(
                    ctx,
                    st,
                    &chk.at,
                    &inst,
                    sub.as_ref(),
                    &mut ann,
                );
            }
            Kind::AllOf(subs) => ok &= composition::check_all_of(ctx, st, &inst, subs, &mut ann),
            Kind::AnyOf(subs) => {
                ok &= composition::check_any_of(ctx, st, &chk.at, &inst, subs, &mut ann);
            }
            Kind::OneOf(subs) => {
                ok &= composition::check_one_of(ctx, st, &chk.at, &inst, subs, &mut ann);
            }
            Kind::Not(sub) => ok &= composition::check_not(ctx, st, &chk.at, &inst, sub),
            Kind::If { cond, then, alt } => {
                ok &= composition::check_if(
                    ctx,
                    st,
                    &inst,
                    cond,
                    then.as_ref(),
                    alt.as_ref(),
                    &mut ann,
                );
            }
            Kind::Ref(target) => {
                ok &= refs::check_ref(ctx, st, &chk.at, &inst, target, &mut ann);
            }
            Kind::DynamicRef(name) => {
                ok &= refs::check_dynamic_ref(ctx, st, &chk.at, &inst, name, &mut ann);
            }
            Kind::Format(name) => {
                ok &= formats::check_format(ctx, st, &chk.at, &inst, name);
            }
        }
    }

    // Persist this application's own annotations BEFORE running
    // `unevaluated*`, so the tail sees every sibling's contribution (the
    // caller above will re-record on success; recording is idempotent).
    if !(ann.props.is_empty() && ann.idxs.is_empty()) {
        ctx.masks.record(inst, ann.clone());
    }

    // unevaluated* run after every sibling (and after sibling applicator
    // branches recorded their annotations).
    for chk in &prog.tail {
        if ctx.aborted {
            break;
        }
        match &chk.kind {
            Kind::UnevaluatedProperties(sub) => {
                ok &= objects::check_unevaluated_props(
                    ctx,
                    st,
                    &chk.at,
                    &inst,
                    sub.as_ref(),
                    &mut ann,
                );
            }
            Kind::UnevaluatedItems(sub) => {
                ok &= arrays::check_unevaluated_items(
                    ctx,
                    st,
                    &chk.at,
                    &inst,
                    sub.as_ref(),
                    &mut ann,
                );
            }
            // Tail only ever holds unevaluated* checks (compiler invariant).
            _ => {}
        }
    }

    for _ in 0..pushed {
        ctx.dyn_scope.pop();
    }
    Out { ok, ann }
}

/// Numeric value of an instance node, if it has one.
pub(crate) fn inst_num(inst: &NodeRef<'_>) -> Option<Num> {
    match inst.kind() {
        ValueKind::Int => inst.as_i64().map(Num::I),
        ValueKind::Float => inst.as_f64().map(Num::F),
        _ => None,
    }
}
