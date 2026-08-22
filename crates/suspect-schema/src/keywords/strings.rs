//! `maxLength`/`minLength` (Unicode scalar counts) and `pattern`.

use std::rc::Rc;

use regex::Regex;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::exec::Ctx;
use crate::exec::Stack;

pub(crate) fn check_length<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    n: usize,
    is_max: bool,
) -> bool {
    if inst.kind() != ValueKind::Str {
        return true;
    }
    let Some(s) = inst.as_str() else { return true };
    let len = s.chars().count();
    let ok = if is_max { len <= n } else { len >= n };
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
    re: &Rc<Regex>,
) -> bool {
    if inst.kind() != ValueKind::Str {
        return true;
    }
    let Some(s) = inst.as_str() else { return true };
    if re.is_match(s) {
        true
    } else {
        ctx.emit(
            st,
            at,
            format!("string does not match `pattern` `{}`", re.as_str()),
        );
        false
    }
}
