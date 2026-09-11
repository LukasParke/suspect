//! Composition: `allOf`, `anyOf`, `oneOf`, `not`, `if`/`then`/`else`.

use suspect_low::{NodeRef, Pointer};

use crate::compile::Prg;
use crate::exec::Stack;
use crate::exec::{Ann, Ctx, Out, eval};

pub(crate) fn check_all_of<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    inst: &NodeRef<'d>,
    subs: &[Prg<'d>],
    ann: &mut Ann<'d>,
) -> bool {
    let mut ok = true;
    for sub in subs {
        if !ctx.step(st, &sub.path) {
            return false;
        }
        let o = eval(ctx, sub, *inst, st);
        if o.ok {
            ok &= ctx.merge_annotations(st, &sub.path, ann, o.ann);
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
        if !ctx.step(st, &sub.path) {
            break;
        }
        let (o, errs) = ctx.divert(|c| eval(c, sub, *inst, st));
        if ctx.aborted {
            break;
        }
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

fn keep_annotations<'d>(
    ctx: &mut Ctx<'_, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    passing: Vec<Out<'d>>,
    ann: &mut Ann<'d>,
) -> bool {
    for o in passing {
        if !ctx.step(st, at) || !ctx.merge_annotations(st, at, ann, o.ann) {
            return false;
        }
    }
    true
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
    if ctx.aborted {
        return false;
    }
    if passing.is_empty() {
        let mut msg = String::from("instance does not match any `anyOf` branch");
        if !reports.is_empty() {
            msg.push_str("; ");
            msg.push_str(&reports.join("; "));
        }
        ctx.emit(st, at, msg);
        false
    } else {
        keep_annotations(ctx, st, at, passing, ann)
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
    if ctx.aborted {
        return false;
    }
    match passing.len() {
        1 => keep_annotations(ctx, st, at, passing, ann),
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
    if ctx.aborted {
        return false;
    }
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
    // A successful condition contributes annotations even without then/else.
    // A failed condition selects else but contributes no annotations.
    let (cond_out, _errs) = ctx.divert(|c| eval(c, cond, *inst, st));
    if ctx.aborted {
        return false;
    }
    let cond_ok = cond_out.ok;
    if cond_ok && !ctx.merge_annotations(st, &cond.path, ann, cond_out.ann) {
        return false;
    }
    let branch = if cond_ok { then } else { alt };
    let Some(branch) = branch else { return true };
    let o = eval(ctx, branch, *inst, st);
    if o.ok {
        ctx.merge_annotations(st, &branch.path, ann, o.ann)
    } else {
        false
    }
}
