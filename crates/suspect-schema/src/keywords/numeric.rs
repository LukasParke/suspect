//! `multipleOf`, `maximum`/`exclusiveMaximum`, `minimum`/`exclusiveMinimum`.

use std::cmp::Ordering;

use suspect_low::{NodeRef, Pointer};

use crate::compile::Num;
use crate::exec::{Ctx, Stack, inst_num};

/// Integer path uses exact modulo; the float path checks that the quotient
/// is (near-)whole with a relative epsilon, so `0.07 / 0.01` counts as a
/// multiple despite binary floating point.
pub(crate) fn check_multiple_of<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    d: Num,
) -> bool {
    let Some(x) = inst_num(inst) else { return true };
    let ok = match (x, d) {
        (Num::I(a), Num::I(b)) => a % b == 0,
        _ => {
            let q = x.as_f64() / d.as_f64();
            let m = (q - q.round()).abs();
            // Relative epsilon: absolute error of division grows with |q|.
            m <= 1e-9 * q.abs().max(1.0)
        }
    };
    if !ok {
        ctx.emit(
            st,
            at,
            format!("value {} is not a multiple of {}", x.as_f64(), d.as_f64()),
        );
    }
    ok
}

/// Bound check; `upper` selects maximum vs minimum semantics.
pub(crate) fn check_bound<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    bound: Num,
    exclusive: bool,
    upper: bool,
) -> bool {
    let Some(x) = inst_num(inst) else { return true };
    let ord = cmp_num(x, bound);
    let ok = match (upper, exclusive) {
        (true, false) => ord != Ordering::Greater,
        (true, true) => ord == Ordering::Less,
        (false, false) => ord != Ordering::Less,
        (false, true) => ord == Ordering::Greater,
    };
    if !ok {
        let what = match (upper, exclusive) {
            (true, false) => format!("maximum {}", bound.as_f64()),
            (true, true) => format!("exclusive maximum {}", bound.as_f64()),
            (false, false) => format!("minimum {}", bound.as_f64()),
            (false, true) => format!("exclusive minimum {}", bound.as_f64()),
        };
        ctx.emit(st, at, format!("value {} violates `{}`", x.as_f64(), what));
    }
    ok
}

fn cmp_num(a: Num, b: Num) -> Ordering {
    match (a, b) {
        (Num::I(x), Num::I(y)) => x.cmp(&y),
        _ => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .unwrap_or(Ordering::Equal),
    }
}
