//! Composition: `allOf`, `anyOf`, `oneOf`, `not`, `if`/`then`/`else`.

use suspect_low::{NodeRef, Pointer};

use crate::compile::Prg;
use crate::exec::{eval, Ann, Ctx, Out};
use crate::exec::Stack;

pub(crate) fn check_all_of<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    inst: &NodeRef<'d>,
    subs: &[Prg<'d>],
    ann: &mut Ann<'d>,
) -> bool {
    let mut ok = true;
    for sub in subs {
        let o = eval(ctx, sub, *inst, st);
        if o.ok {
            ctx.masks.record(*inst, o.ann.clone());
            ann.merge(o.ann);
        } else {
            ok = false;
        }
    }
    ok
}

/// Tries every branch with errors diverted; returns the passing results plus
/// a per-branch failure report for error messages.
fn try_branches<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    inst: &NodeRef<'d>,
    subs: &[Prg<'d>],
) -> (Vec<Out<'d>>, Vec<String>) {
    let mut passing = Vec::new();
    let mut reports = Vec::new();
    for (i, sub) in subs.iter().enumerate() {
        let (o, errs) = ctx.divert(|c| eval(c, sub, *inst, st));
        if o.ok {
            passing.push(o);
        } else {
            reports.push(format!(
                "branch {} at `{}` failed: {}",
                i,
                sub.path.to_path(),
                Ctx::first_msg(&errs)
            ));
        }
    }
    (passing, reports)
}

fn keep_annotations<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    inst: &NodeRef<'d>,
    passing: Vec<Out<'d>>,
    ann: &mut Ann<'d>,
) {
    for o in passing {
        let mut a = o.ann;
        ctx.masks.record(*inst, a.clone());
        ann.merge(std::mem::take(&mut a));
    }
}

pub(crate) fn check_any_of<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    subs: &[Prg<'d>],
    ann: &mut Ann<'d>,
) -> bool {
    let (passing, reports) = try_branches(ctx, st, inst, subs);
    if passing.is_empty() {
        let mut msg = String::from("instance does not match any `anyOf` branch");
        if !reports.is_empty() {
            msg.push_str("; ");
            msg.push_str(&reports.join("; "));
        }
        ctx.emit(st, at, msg);
        false
    } else {
        keep_annotations(ctx, inst, passing, ann);
        true
    }
}

pub(crate) fn check_one_of<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    subs: &[Prg<'d>],
    ann: &mut Ann<'d>,
) -> bool {
    let (passing, reports) = try_branches(ctx, st, inst, subs);
    match passing.len() {
        1 => {
            keep_annotations(ctx, inst, passing, ann);
            true
        }
        0 => {
            let mut msg = String::from("instance does not match any `oneOf` branch");
            if !reports.is_empty() {
                msg.push_str("; ");
                msg.push_str(&reports.join("; "));
            }
            ctx.emit(st, at, msg);
            false
        }
        n => {
            let mut matched = passing
                .iter()
                .map(|_| String::new())
                .collect::<Vec<_>>();
            let _ = &mut matched;
            // Re-derive which branches passed from the reports we did NOT get:
            // simpler to list count only.
            ctx.emit(
                st,
                at,
                format!("instance matches {n} `oneOf` branches (exactly one is required)"),
            );
            false
        }
    }
}

pub(crate) fn check_not<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    sub: &Prg<'d>,
) -> bool {
    // The inner schema's errors and annotations are discarded entirely.
    let (o, _errs) = ctx.divert(|c| eval(c, sub, *inst, st));
    if o.ok {
        ctx.emit(st, at, "instance matches `not` schema".into());
        false
    } else {
        true
    }
}

pub(crate) fn check_if<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    inst: &NodeRef<'d>,
    cond: &Prg<'d>,
    then: Option<&Prg<'d>>,
    alt: Option<&Prg<'d>>,
    ann: &mut Ann<'d>,
) -> bool {
    // `if` asserts nothing and its annotations are discarded.
    let (cond_out, _errs) = ctx.divert(|c| eval(c, cond, *inst, st));
    let cond_ok = cond_out.ok;
    let branch = if cond_ok { then } else { alt };
    let Some(branch) = branch else { return true };
    let o = eval(ctx, branch, *inst, st);
    if o.ok {
        ctx.masks.record(*inst, o.ann.clone());
        ann.merge(o.ann);
        true
    } else {
        false
    }
}
