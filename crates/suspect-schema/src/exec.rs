//! Schema evaluation: walks a compiled [`Program`] against an instance with
//! annotation tracking for `unevaluatedProperties`/`unevaluatedItems`.
//!
//! Recursion is bounded by [`Config::max_depth`]; exceeding it produces a
//! clean [`SchemaError`] instead of a stack overflow (recursive `$ref`
//! schemas against deep instances are the intended use). Array elements and
//! object members are iterated in loops, so only genuine schema descent
//! costs stack frames (~2 per instance-nesting level).
//! [`Config::max_evaluation_steps`] additionally bounds schema, keyword and
//! collection visits across the whole call, including trials and references.

use std::borrow::Cow;
use std::collections::HashSet;

use smallvec::SmallVec;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::Schema;
use crate::compile::{Kind, Prg};
use crate::errors::{SchemaError, SchemaErrorKind};
use crate::keywords::{
    arrays, cardinality, composition, formats, numeric, objects, refs, strings, types,
};

/// One token of the instance location under construction.
#[derive(Clone)]
pub(crate) enum Tok<'d> {
    Key(Cow<'d, str>),
    Idx(usize),
}

/// Zero-allocation instance path stack; a [`Pointer`] is materialized only
/// when an error is actually emitted.
pub(crate) struct Stack<'d>(SmallVec<[Tok<'d>; 24]>);

impl<'d> Stack<'d> {
    pub(crate) fn new() -> Self {
        Self(SmallVec::new())
    }
    pub(crate) fn push_key(&mut self, k: Cow<'d, str>) {
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
                    Tok::Key(k) => k.to_string().into_boxed_str(),
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
    props: HashSet<Cow<'d, str>>,
    idxs: HashSet<usize>,
}

impl<'d> Ann<'d> {
    pub(crate) fn prop(&mut self, k: Cow<'d, str>) {
        self.props.insert(k);
    }
    pub(crate) fn idx(&mut self, i: usize) {
        self.idxs.insert(i);
    }
    pub(crate) fn has_prop(&self, k: &str) -> bool {
        self.props.contains(k)
    }
    pub(crate) fn has_idx(&self, i: usize) -> bool {
        self.idxs.contains(&i)
    }
}

/// Per-`validate` evaluation context.
pub(crate) struct Ctx<'a, 'd> {
    pub sch: &'a Schema<'d>,
    /// Error cap for this run (`max_errors`, or 1 for `validate_first`).
    pub cap: usize,
    pub out: Vec<SchemaError>,
    /// Outside `out` so branch diversion cannot swallow evaluation failures.
    pub evaluation_error: Option<SchemaError>,
    pub equality: types::EqualityBudget,
    /// Shared across every branch, collection and lazy reference in this call.
    pub remaining_steps: usize,
    pub aborted: bool,
    pub depth: usize,
    /// Resource scope in evaluation order, searched outermost first.
    pub dyn_scope: Vec<Pointer>,
}

impl<'a, 'd> Ctx<'a, 'd> {
    pub(crate) fn step(&mut self, st: &Stack<'d>, at: &Pointer) -> bool {
        self.charge(st, at, 1)
    }

    /// Bulk accounting for already-materialized collection lengths. This is
    /// a visit limit; LowDoc's collection materialization is not memory-bounded
    /// by this counter, nor is schema compilation or string byte work.
    pub(crate) fn charge(&mut self, st: &Stack<'d>, at: &Pointer, steps: usize) -> bool {
        if self.aborted {
            return false;
        }
        let Some(remaining) = self.remaining_steps.checked_sub(steps) else {
            self.remaining_steps = 0;
            self.fail_evaluation(
                st,
                at,
                format!(
                    "schema evaluation exceeds {} evaluation steps",
                    self.sch.config().max_evaluation_steps
                ),
            );
            return false;
        };
        self.remaining_steps = remaining;
        true
    }

    pub(crate) fn merge_annotations(
        &mut self,
        st: &Stack<'d>,
        at: &Pointer,
        into: &mut Ann<'d>,
        other: Ann<'d>,
    ) -> bool {
        for key in other.props {
            if !self.step(st, at) {
                return false;
            }
            into.prop(key);
        }
        for index in other.idxs {
            if !self.step(st, at) {
                return false;
            }
            into.idx(index);
        }
        !self.aborted
    }

    pub(crate) fn text(
        &mut self,
        node: NodeRef<'d>,
        st: &Stack<'d>,
        at: &Pointer,
    ) -> Option<Cow<'d, str>> {
        let result = crate::resources::decoded_text(node);
        if result.is_none() {
            self.fail_evaluation(st, at, "cannot decode malformed string text".into());
        }
        result
    }
    pub(crate) fn emit(&mut self, st: &Stack<'d>, at: &Pointer, message: String) {
        if self.aborted {
            return;
        }
        if self.cap != 0 && self.out.len() >= self.cap {
            self.aborted = true;
            return;
        }
        self.out.push(SchemaError {
            kind: SchemaErrorKind::Invalid,
            instance_path: st.to_pointer(),
            schema_path: at.clone(),
            message,
        });
    }

    pub(crate) fn fail_evaluation(&mut self, st: &Stack<'d>, at: &Pointer, message: String) {
        if self.evaluation_error.is_none() {
            self.evaluation_error = Some(SchemaError {
                kind: SchemaErrorKind::Evaluation,
                instance_path: st.to_pointer(),
                schema_path: at.clone(),
                message,
            });
        }
        self.aborted = true;
    }

    /// Runs a trial branch with error reporting diverted; returns the result
    /// plus the diverted errors (used by `anyOf`/`oneOf`/`not`/`if`, where
    /// failing branches must not pollute the report).
    pub(crate) fn divert<F, R>(&mut self, f: F) -> (R, Vec<SchemaError>)
    where
        F: FnOnce(&mut Self) -> R,
    {
        let real = std::mem::take(&mut self.out);
        let real_aborted = self.aborted;
        self.out = Vec::new();
        let r = f(self);
        let trial = std::mem::replace(&mut self.out, real);
        // A capped mismatch report in one trial must not abort sibling
        // alternatives or suppress the final union diagnostic. Evaluation
        // failures remain fatal and survive all logical branch diversion.
        self.aborted = real_aborted || self.evaluation_error.is_some();
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
    if !ctx.step(st, &prog.path) {
        return Out::fail();
    }
    ctx.depth += 1;
    let r = if ctx.depth > ctx.sch.config().max_depth {
        ctx.fail_evaluation(
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

    // Every resource contributes its anchor registry, including anchors in
    // $defs that have not themselves been evaluated.
    let pushed = ctx.dyn_scope.last() != Some(&prog.resource);
    if pushed {
        ctx.dyn_scope.push(prog.resource.clone());
    }

    for chk in &prog.checks {
        if !ctx.step(st, &chk.at) {
            ok = false;
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
                // LowDoc also represents YAML .inf/.nan as Float. Such
                // values lie outside JSON's finite-number domain, and a
                // logical branch must not invert that evaluation failure.
                if k == ValueKind::Float && !inst.scalar_bytes().iter().any(u8::is_ascii_digit) {
                    ctx.fail_evaluation(st, &chk.at, "expected a finite numeric value".into());
                    ok = false;
                    continue;
                }
                let float_is_int = k == ValueKind::Float && inst.is_integral_number();
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
                ok &= types::check_enum(ctx, st, &chk.at, inst, vals);
            }
            Kind::Const(v) => {
                ok &= types::check_const(ctx, st, &chk.at, inst, *v);
            }
            Kind::UniqueItems => {
                ok &= types::check_unique_items(ctx, st, &chk.at, inst);
            }
            Kind::MultipleOf(d) => ok &= numeric::check_multiple_of(ctx, st, &chk.at, &inst, d),
            Kind::Maximum(b, ex) => {
                ok &= numeric::check_bound(ctx, st, &chk.at, &inst, b, *ex, true);
            }
            Kind::Minimum(b, ex) => {
                ok &= numeric::check_bound(ctx, st, &chk.at, &inst, b, *ex, false);
            }
            Kind::MaxLength(n) => ok &= strings::check_length(ctx, st, &chk.at, &inst, *n, true),
            Kind::MinLength(n) => ok &= strings::check_length(ctx, st, &chk.at, &inst, *n, false),
            Kind::MaxItems(n) => {
                ok &= cardinality::check_collection(
                    ctx,
                    st,
                    &chk.at,
                    inst,
                    *n,
                    ValueKind::Array,
                    true,
                )
            }
            Kind::MinItems(n) => {
                ok &= cardinality::check_collection(
                    ctx,
                    st,
                    &chk.at,
                    inst,
                    *n,
                    ValueKind::Array,
                    false,
                )
            }
            Kind::MaxProperties(n) => {
                ok &= cardinality::check_collection(
                    ctx,
                    st,
                    &chk.at,
                    inst,
                    *n,
                    ValueKind::Object,
                    true,
                )
            }
            Kind::MinProperties(n) => {
                ok &= cardinality::check_collection(
                    ctx,
                    st,
                    &chk.at,
                    inst,
                    *n,
                    ValueKind::Object,
                    false,
                )
            }
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
                ok &= objects::check_properties(ctx, st, &chk.at, &inst, subs, &mut ann);
            }
            Kind::PatternProperties(subs) => {
                ok &= objects::check_pattern_properties(ctx, st, &chk.at, &inst, subs, &mut ann);
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
                ok &= objects::check_dependent_schemas(ctx, st, &chk.at, &inst, subs, &mut ann);
            }
            Kind::DependentRequired(reqs) => {
                ok &= objects::check_dependent_required(ctx, st, &chk.at, &inst, reqs);
            }
            Kind::Required(names) => {
                ok &= objects::check_required(ctx, st, &chk.at, &inst, names);
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
            Kind::DynamicRef { target, anchor } => {
                match refs::dynamic_target(ctx, st, &chk.at, target, anchor.as_deref()) {
                    Some(target) => {
                        ok &= refs::check_ref(ctx, st, &chk.at, &inst, &target, &mut ann)
                    }
                    None => ok = false,
                }
            }
            Kind::Format(name) => {
                ok &= formats::check_format(ctx, st, &chk.at, &inst, name);
            }
        }
    }

    // Only this schema application's adjacent keywords and successful
    // in-place applicators contribute. Each child application starts fresh;
    // cousins, failed alternatives and negated schemas cannot leak annotations.
    for chk in &prog.tail {
        if !ctx.step(st, &chk.at) {
            ok = false;
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

    if pushed {
        ctx.dyn_scope.pop();
    }
    Out {
        ok: ok && !ctx.aborted,
        ann,
    }
}
