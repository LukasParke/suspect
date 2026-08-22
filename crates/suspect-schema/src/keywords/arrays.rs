//! `items`, `prefixItems`, `contains` (+`minContains`/`maxContains`) and
//! `unevaluatedItems`.

use smallvec::SmallVec;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::compile::Prg;
use crate::exec::Stack;
use crate::exec::{Ann, Ctx, eval};

pub(crate) fn check_items<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    _at: &Pointer,
    inst: &NodeRef<'d>,
    sub: &Prg<'d>,
    skip: usize,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Array {
        return true;
    }
    let mut ok = true;
    for (i, el) in inst.items().into_iter().enumerate().skip(skip) {
        st.push_idx(i);
        let o = eval(ctx, sub, el, st);
        st.pop();
        if o.ok {
            ctx.masks.record(el, o.ann);
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
    _at: &Pointer,
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
        st.push_idx(i);
        let o = eval(ctx, &subs[i], el, st);
        st.pop();
        if o.ok {
            ctx.masks.record(el, o.ann);
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
    min: usize,
    max: Option<usize>,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Array {
        // minContains > 0 with no array present still fails.
        if min == 0 {
            return true;
        }
        ctx.emit(
            st,
            at,
            format!("instance is not an array but `minContains` requires {min} match(es)"),
        );
        return false;
    }
    let mut matched = 0usize;
    let mut matched_idxs = SmallVec::<[usize; 8]>::new();
    for (i, el) in inst.items().into_iter().enumerate() {
        if max.is_some_and(|mx| matched > mx) {
            break; // already too many; fail fast below
        }
        st.push_idx(i);
        let (o, _errs) = ctx.divert(|c| eval(c, schema, el, st));
        st.pop();
        if o.ok {
            matched += 1;
            matched_idxs.push(i);
            ctx.masks.record(el, o.ann);
        }
    }
    let ok = matched >= min && max.is_none_or(|mx| matched <= mx);
    if ok {
        for i in matched_idxs {
            ann.idx(i);
        }
    } else if matched < min {
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
                max.unwrap_or_default()
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
        if ctx.masks.has_idx(*inst, i) {
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
                    ctx.masks.record(el, o.ann);
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
