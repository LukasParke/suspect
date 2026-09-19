//! Exact nonnegative size bounds. A bound beyond usize::MAX has known
//! comparison semantics for every resident collection/string; no expansion or
//! narrowing of the schema's mathematical integer is needed.

use std::fmt;

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::exec::{Ctx, Stack};

#[derive(Debug, Clone, Copy)]
pub(crate) enum CountBound {
    Finite(usize),
    BeyondAddressable,
}

impl CountBound {
    pub(crate) fn allows_min(self, count: usize) -> bool {
        matches!(self, Self::Finite(minimum) if count >= minimum)
    }

    pub(crate) fn allows_max(self, count: usize) -> bool {
        match self {
            Self::Finite(maximum) => count <= maximum,
            Self::BeyondAddressable => true,
        }
    }
}

impl fmt::Display for CountBound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finite(value) => value.fmt(f),
            Self::BeyondAddressable => write!(f, "a value greater than {}", usize::MAX),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn check_collection<'d>(
    ctx: &mut Ctx<'_, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    instance: NodeRef<'d>,
    bound: CountBound,
    kind: ValueKind,
    maximum: bool,
) -> bool {
    if instance.kind() != kind {
        return true;
    }
    let count = match kind {
        ValueKind::Array => instance.items().len(),
        ValueKind::Object => instance.entries().len(),
        _ => unreachable!("collection size checks apply only to arrays/objects"),
    };
    if !ctx.charge(st, at, count) {
        return false;
    }
    let valid = if maximum {
        bound.allows_max(count)
    } else {
        bound.allows_min(count)
    };
    if !valid {
        ctx.emit(
            st,
            at,
            format!(
                "collection size {count} violates {} {bound}",
                if maximum { "maximum" } else { "minimum" }
            ),
        );
    }
    valid
}
