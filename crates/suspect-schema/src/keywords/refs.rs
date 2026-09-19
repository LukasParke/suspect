//! `$ref` (lazy, cached, cycle-safe) and `$dynamicRef`/`$dynamicAnchor`
//! within the indexed schema resources.

use suspect_low::{NodeRef, Pointer};

use crate::CompileError;
use crate::compile::{Compiler, Prg, RefTarget, compile_program};
use crate::exec::Stack;
use crate::exec::{Ann, Ctx, eval};

/// Resolves a same-document pointer to its compiled program, compiling on
/// first use and memoizing the result (`None` = unresolvable). Because
/// resolution happens at execution time, recursive schemas compile fine:
/// the cycle is broken by the cache before the inner `$ref` resolves.
pub(crate) fn resolve_target<'a, 'd>(
    ctx: &Ctx<'a, 'd>,
    target: &Pointer,
) -> Result<Option<Prg<'d>>, CompileError> {
    if let Some(hit) = ctx.sch.cache.borrow().get(target) {
        return hit.clone();
    }
    let Some(node) = ctx.sch.root_node().pointer(target) else {
        return Ok(None);
    };
    let scan = ctx.sch.scan();
    let base = scan.base_for(target);
    let res_ptr = scan.resource_for(target);
    let compiler = Compiler::new(ctx.sch.config().clone());
    let compiled = compile_program(&compiler, node, target, base, scan, 0, &res_ptr).map(Some);
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
            ctx.fail_evaluation(st, at, "external schema resolution not configured".into());
            false
        }
        RefTarget::Local(ptr) => match resolve_target(ctx, ptr) {
            Ok(Some(p)) => {
                let o = eval(ctx, &p, *inst, st);
                if o.ok {
                    ctx.merge_annotations(st, at, ann, o.ann)
                } else {
                    false
                }
            }
            Ok(None) => {
                ctx.fail_evaluation(
                    st,
                    at,
                    format!("unresolvable $ref target `{}`", ptr.to_path()),
                );
                false
            }
            Err(error) => {
                ctx.fail_evaluation(
                    st,
                    at,
                    format!("cannot compile $ref target `{}`: {error}", ptr.to_path()),
                );
                false
            }
        },
    }
}

/// Only a statically resolved $dynamicAnchor fragment is eligible for
/// rebinding. The outermost resource declaring that name wins (2020-12
/// Core §8.2.3.2); pointers and ordinary $anchor fragments stay static.
pub(crate) fn dynamic_target<'d>(
    ctx: &mut Ctx<'_, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    target: &RefTarget,
    name: Option<&str>,
) -> Option<RefTarget> {
    if let Some(name) = name {
        for index in 0..ctx.dyn_scope.len() {
            if !ctx.step(st, at) {
                return None;
            }
            let resource = &ctx.dyn_scope[index];
            if let Some(pointer) = ctx.sch.scan().dynamic_anchor(resource, name) {
                return Some(RefTarget::Local(pointer.clone()));
            }
        }
    }
    Some(target.clone())
}
