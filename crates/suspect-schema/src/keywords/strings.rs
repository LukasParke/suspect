//! `maxLength`/`minLength` (Unicode scalar counts) and `pattern`.

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::PatternProgram;
use crate::exec::Ctx;
use crate::exec::Stack;
use crate::keywords::cardinality::CountBound;

pub(crate) fn check_length<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    n: CountBound,
    is_max: bool,
) -> bool {
    if inst.kind() != ValueKind::Str {
        return true;
    }
    let Some(decoded) = inst.try_decoded_scalar() else {
        ctx.fail_evaluation(st, at, "string escape decoding failed".into());
        return false;
    };
    let Ok(s) = std::str::from_utf8(&decoded) else {
        ctx.fail_evaluation(st, at, "string is not valid UTF-8".into());
        return false;
    };
    let len = s.chars().count();
    let ok = if is_max {
        n.allows_max(len)
    } else {
        n.allows_min(len)
    };
    if !ok {
        let kw = if is_max { "maxLength" } else { "minLength" };
        ctx.emit(
            st,
            at,
            format!("string length {len} violates `{kw}` {n} (Unicode scalar count)"),
        );
    }
    ok
}

pub(crate) fn check_pattern<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    program: &PatternProgram,
) -> bool {
    if inst.kind() != ValueKind::Str {
        return true;
    }
    let Some(s) = ctx.text(*inst, st, at) else {
        return false;
    };
    match crate::pattern::is_match(program, &s, &mut ctx.remaining_steps) {
        Ok(true) => true,
        Ok(false) => {
            ctx.emit(st, at, "string does not match `pattern`".into());
            false
        }
        Err(()) => {
            ctx.fail_evaluation(
                st,
                at,
                format!(
                    "schema evaluation exceeds {} evaluation steps",
                    ctx.sch.config().max_evaluation_steps
                ),
            );
            false
        }
    }
}
