//! `items`, `prefixItems`, `contains` (+`minContains`/`maxContains`) and
//! `unevaluatedItems`.

use smallvec::SmallVec;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::compile::Prg;
use crate::exec::Stack;
use crate::exec::{Ann, Ctx, eval};
use crate::keywords::cardinality::CountBound;

pub(crate) fn check_items<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    sub: &Prg<'d>,
    skip: usize,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Array {
        return true;
    }
    let mut ok = true;
    for (i, el) in inst.items().into_iter().enumerate() {
        if !ctx.step(st, at) {
            return false;
        }
        if i < skip {
            continue;
        }
        st.push_idx(i);
        let o = eval(ctx, sub, el, st);
        st.pop();
        if o.ok {
            ann.idx(i);
        } else {
            ok = false;
        }
    }
    ok
}

pub(crate) fn check_prefix_items<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    subs: &[Prg<'d>],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Array || subs.is_empty() {
        return true;
    }
    let mut ok = true;
    for (i, el) in inst.items().into_iter().enumerate() {
        if i >= subs.len() {
            break;
        }
        if !ctx.step(st, at) {
            return false;
        }
        st.push_idx(i);
        let o = eval(ctx, &subs[i], el, st);
        st.pop();
        if o.ok {
            ann.idx(i);
        } else {
            ok = false;
        }
    }
    ok
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn check_contains<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    schema: &Prg<'d>,
    min: CountBound,
    max: Option<CountBound>,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Array {
        // Applicability is independent from an explicit type constraint.
        // contains/minContains/maxContains do not constrain non-arrays.
        return true;
    }
    let mut matched = 0usize;
    let mut matched_idxs = SmallVec::<[usize; 8]>::new();
    for (i, el) in inst.items().into_iter().enumerate() {
        if max.is_some_and(|mx| !mx.allows_max(matched)) {
            break; // already too many; fail fast below
        }
        if !ctx.step(st, at) {
            return false;
        }
        st.push_idx(i);
        let (o, _errs) = ctx.divert(|c| eval(c, schema, el, st));
        st.pop();
        if ctx.aborted {
            return false;
        }
        if o.ok {
            matched += 1;
            matched_idxs.push(i);
        }
    }
    let ok = min.allows_min(matched) && max.is_none_or(|mx| mx.allows_max(matched));
    if ok {
        for i in matched_idxs {
            if !ctx.step(st, at) {
                return false;
            }
            ann.idx(i);
        }
    } else if !min.allows_min(matched) {
        ctx.emit(
            st,
            at,
            format!(
                "array has {matched} item(s) matching `contains`, fewer than `minContains` {min}"
            ),
        );
    } else {
        ctx.emit(
            st,
            at,
            format!(
                "array has {matched} item(s) matching `contains`, more than `maxContains` {}",
                max.expect("failure with a satisfied minimum requires a maximum")
            ),
        );
    }
    ok
}

pub(crate) fn check_unevaluated_items<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    sub: Option<&Prg<'d>>,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Array {
        return true;
    }
    let mut ok = true;
    for (i, el) in inst.items().into_iter().enumerate() {
        if !ctx.step(st, at) {
            return false;
        }
        if ann.has_idx(i) {
            continue;
        }
        st.push_idx(i);
        match sub {
            None => {
                ctx.emit(st, at, format!("array item {i} is unevaluated"));
                ok = false;
            }
            Some(p) => {
                let o = eval(ctx, p, el, st);
                if o.ok {
                    ann.idx(i);
                } else {
                    ok = false;
                }
            }
        }
        st.pop();
    }
    ok
}
