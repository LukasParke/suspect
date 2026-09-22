//! `multipleOf`, `maximum`/`exclusiveMaximum`, `minimum`/`exclusiveMinimum`.

use std::cmp::Ordering;

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::exec::{Ctx, Stack};
use crate::number::{Divisor, ExactNumber};

fn instance_number<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
) -> Result<Option<ExactNumber>, ()> {
    if !matches!(inst.kind(), ValueKind::Int | ValueKind::Float) {
        return Ok(None);
    }
    ExactNumber::parse(inst.scalar_bytes(), ctx.sch.config().max_number_bytes)
        .map(Some)
        .map_err(|error| ctx.fail_evaluation(st, at, error.to_string()))
}

/// Divisibility over exact decimal coefficients and symbolic exponents.
pub(crate) fn check_multiple_of<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    d: &Divisor,
) -> bool {
    let x = match instance_number(ctx, st, at, inst) {
        Ok(Some(x)) => x,
        Ok(None) => return true,
        Err(()) => return false,
    };
    let ok = d.contains(&x);
    if !ok {
        ctx.emit(st, at, format!("value {x} is not a multiple of {d}"));
    }
    ok
}

/// Bound check; `upper` selects maximum vs minimum semantics.
pub(crate) fn check_bound<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    bound: &ExactNumber,
    exclusive: bool,
    upper: bool,
) -> bool {
    let x = match instance_number(ctx, st, at, inst) {
        Ok(Some(x)) => x,
        Ok(None) => return true,
        Err(()) => return false,
    };
    let ord = x.cmp(bound);
    let ok = match (upper, exclusive) {
        (true, false) => ord != Ordering::Greater,
        (true, true) => ord == Ordering::Less,
        (false, false) => ord != Ordering::Less,
        (false, true) => ord == Ordering::Greater,
    };
    if !ok {
        let what = match (upper, exclusive) {
            (true, false) => format!("maximum {bound}"),
            (true, true) => format!("exclusive maximum {bound}"),
            (false, false) => format!("minimum {bound}"),
            (false, true) => format!("exclusive minimum {bound}"),
        };
        ctx.emit(st, at, format!("value {x} violates `{what}`"));
    }
    ok
}
