//! `$ref` (lazy, cached, cycle-safe) and `$dynamicRef`/`$dynamicAnchor`
//! (RFC 3093 basic semantics).

use std::rc::Rc;

use suspect_low::{NodeRef, Pointer};

use crate::compile::{
    Compiler, Prg, RefTarget, base_for_pointer, compile_program, resource_root_for,
};
use crate::exec::Stack;
use crate::exec::{Ann, Ctx, eval};

/// Resolves a same-document pointer to its compiled program, compiling on
/// first use and memoizing the result (`None` = unresolvable). Because
/// resolution happens at execution time, recursive schemas compile fine:
/// the cycle is broken by the cache before the inner `$ref` resolves.
pub(crate) fn resolve_target<'a, 'd>(ctx: &Ctx<'a, 'd>, target: &Pointer) -> Option<Prg<'d>> {
    if let Some(hit) = ctx.sch.cache.borrow().get(target) {
        return hit.clone();
    }
    let node = ctx.sch.root_node().pointer(target)?;
    let scan = ctx.sch.scan();
    let base = base_for_pointer(scan, ctx.sch.root_base(), target);
    let res_ptr = resource_root_for(scan, target);
    let compiler = Compiler::new(ctx.sch.config().clone());
    let compiled = compile_program(&compiler, node, target, &base, scan, 0, &res_ptr).ok();
    ctx.sch
        .cache
        .borrow_mut()
        .insert(target.clone(), compiled.clone());
    compiled
}

pub(crate) fn check_ref<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    target: &RefTarget,
    ann: &mut Ann<'d>,
) -> bool {
    match target {
        RefTarget::External => {
            ctx.emit(st, at, "external schema resolution not configured".into());
            false
        }
        RefTarget::Local(ptr) => match resolve_target(ctx, ptr) {
            Some(p) => {
                let o = eval(ctx, &p, *inst, st);
                if o.ok {
                    ctx.masks.record(*inst, o.ann.clone());
                    ann.merge(o.ann);
                    true
                } else {
                    false
                }
            }
            None => {
                ctx.emit(
                    st,
                    at,
                    format!("unresolvable $ref target `{}`", ptr.to_path()),
                );
                false
            }
        },
    }
}

/// RFC 3093 basic semantics, documented simplification: the dynamic scope is
/// walked outermost-first and the first fragment declaring the anchor wins;
/// if no dynamic scope declares it, we fall back to static resolution
/// through the document's `$dynamicAnchor` registry.
pub(crate) fn check_dynamic_ref<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    name: &Rc<str>,
    ann: &mut Ann<'d>,
) -> bool {
    let mut scoped: Option<Prg<'d>> = None;
    for (anchor, prog) in &ctx.dyn_scope {
        if anchor.as_ref() == name.as_ref() {
            scoped = Some(prog.clone());
            break;
        }
    }
    let target = match scoped {
        Some(t) => t,
        None => {
            let Some(ptr) = ctx.sch.scan().dyn_anchors.get(name.as_ref()) else {
                ctx.emit(st, at, format!("unresolvable $dynamicRef `#{name}`"));
                return false;
            };
            let ptr = ptr.clone();
            match resolve_target(ctx, &ptr) {
                Some(t) => t,
                None => {
                    ctx.emit(st, at, format!("unresolvable $dynamicRef `#{name}`"));
                    return false;
                }
            }
        }
    };
    let o = eval(ctx, &target, *inst, st);
    if o.ok {
        ctx.masks.record(*inst, o.ann.clone());
        ann.merge(o.ann);
        true
    } else {
        false
    }
}
