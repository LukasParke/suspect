//! Object keywords: `properties`, `patternProperties`, `additionalProperties`,
//! `propertyNames`, `dependentSchemas`, `dependentRequired` and
//! `unevaluatedProperties`.

use std::rc::Rc;

use regex::Regex;
use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::compile::{Kind, Prg, RefTarget, TypeBits};
use crate::keywords::{formats, refs};
use crate::exec::{eval, Ann, Ctx};
use crate::exec::Stack;

pub(crate) fn check_properties<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    inst: &NodeRef<'d>,
    subs: &[(&'d str, Prg<'d>)],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for e in inst.entries() {
        let Some(val) = e.value else { continue };
        let Some((_, sub)) = subs.iter().find(|(k, _)| *k == e.key) else { continue };
        st.push_key(e.key);
        let o = eval(ctx, sub, val, st);
        st.pop();
        if o.ok {
            ctx.masks.record(val, o.ann);
            ann.prop(e.key);
        } else {
            ok = false;
        }
    }
    ok
}

pub(crate) fn check_pattern_properties<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    inst: &NodeRef<'d>,
    subs: &[(Rc<Regex>, Prg<'d>)],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for e in inst.entries() {
        let Some(val) = e.value else { continue };
        for (re, sub) in subs {
            if !re.is_match(e.key) {
                continue;
            }
            st.push_key(e.key);
            let o = eval(ctx, sub, val, st);
            st.pop();
            if o.ok {
                ctx.masks.record(val, o.ann);
                ann.prop(e.key);
            } else {
                ok = false;
            }
        }
    }
    ok
}

#[allow(clippy::too_many_arguments)] // evaluator context threading is uniform
pub(crate) fn check_additional<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    except_keys: &[&'d str],
    except_patterns: &[Rc<Regex>],
    schema: Option<&Prg<'d>>,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for e in inst.entries() {
        if except_keys.contains(&e.key)
            || except_patterns.iter().any(|re| re.is_match(e.key))
        {
            continue;
        }
        let Some(val) = e.value else { continue };
        match schema {
            None => {
                st.push_key(e.key);
                ctx.emit(
                    st,
                    at,
                    format!("property `{}` is not allowed by `additionalProperties: false`", e.key),
                );
                st.pop();
                ok = false;
            }
            Some(sub) => {
                st.push_key(e.key);
                let o = eval(ctx, sub, val, st);
                st.pop();
                if o.ok {
                    ctx.masks.record(val, o.ann);
                    ann.prop(e.key);
                } else {
                    ok = false;
                }
            }
        }
    }
    ok
}

pub(crate) fn check_property_names<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    sub: &Prg<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for (key_node, _) in inst.resolved().syntax().mapping_entries() {
        let Ok(key) = std::str::from_utf8(key_node.scalar_bytes()) else { continue };
        if !string_schema_ok(ctx, sub, key, 0) {
            ctx.emit(
                st,
                at,
                format!("property name `{key}` does not match `propertyNames`"),
            );
            ok = false;
        }
    }
    ok
}

/// Evaluates a compiled program against a bare property-name string.
///
/// The spine offers no way to synthesize a [`NodeRef`] for a name that is
/// not itself an instance node, so this walks the program directly and
/// asserts every keyword that can constrain a string, recursing through
/// composition and `$ref`. Keywords that apply only to non-string types
/// (`items`, numeric bounds, …) pass vacuously, which matches JSON Schema
/// semantics for string instances.
fn string_schema_ok(ctx: &mut Ctx<'_, '_>, prog: &Prg<'_>, s: &str, depth: usize) -> bool {
    if depth > 128 {
        return false;
    }
    let mut ok = true;
    for chk in &prog.checks {
        match &chk.kind {
            Kind::Always(b) => ok &= *b,
            Kind::Type(bits) => ok &= bits.0 & TypeBits::STR != 0,
            Kind::Enum(vals) => ok &= vals
                .iter()
                .any(|v| v.kind() == ValueKind::Str && v.as_str() == Some(s)),
            Kind::Const(v) => ok &= v.kind() == ValueKind::Str && v.as_str() == Some(s),
            Kind::MinLength(n) => ok &= s.chars().count() >= *n,
            Kind::MaxLength(n) => ok &= s.chars().count() <= *n,
            Kind::Pattern(re) => ok &= re.is_match(s),
            Kind::Format(name) => ok &= formats::validate(name, s),
            Kind::AllOf(subs) => {
                ok &= subs.iter().all(|p| string_schema_ok(ctx, p, s, depth + 1));
            }
            Kind::AnyOf(subs) => {
                ok &= subs.iter().any(|p| string_schema_ok(ctx, p, s, depth + 1));
            }
            Kind::OneOf(subs) => {
                ok &= subs.iter().filter(|p| string_schema_ok(ctx, p, s, depth + 1)).count() == 1;
            }
            Kind::Not(inner) => ok &= !string_schema_ok(ctx, inner, s, depth + 1),
            Kind::Ref(RefTarget::Local(ptr)) => {
                ok &= match refs::resolve_target(ctx, ptr) {
                    Some(p) => string_schema_ok(ctx, &p, s, depth + 1),
                    None => false,
                };
            }
            Kind::If { cond, then, alt } => {
                let c = string_schema_ok(ctx, cond, s, depth + 1);
                let branch = if c { then } else { alt };
                if let Some(b) = branch {
                    ok &= string_schema_ok(ctx, b, s, depth + 1);
                }
            }
            Kind::Ref(RefTarget::External) | Kind::DynamicRef(_) => ok &= false,
            // String-inapplicable keywords pass vacuously.
            _ => {}
        }
    }
    ok
}

pub(crate) fn check_dependent_schemas<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    inst: &NodeRef<'d>,
    subs: &[(&'d str, Prg<'d>)],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for (key, sub) in subs {
        if inst.get(key).is_none() {
            continue;
        }
        // Applied to the whole object; its inner evaluations count.
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

pub(crate) fn check_dependent_required<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    reqs: &[(&'d str, Vec<Box<str>>)],
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for (key, deps) in reqs {
        if inst.get(key).is_none() {
            continue;
        }
        for d in deps {
            if inst.get(d).is_none() {
                ctx.emit(
                    st,
                    at,
                    format!("property `{d}` is required when `{key}` is present"),
                );
                ok = false;
            }
        }
    }
    ok
}

/// `required`: every name must be present; present names count as
/// evaluated for `unevaluatedProperties` (2020-12 §10.2.2 annotations).
pub(crate) fn check_required<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    names: &[&'d str],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for n in names {
        match inst.get(n) {
            Some(_) => ann.prop(n),
            None => {
                ctx.emit(st, at, format!("required property `{n}` is missing"));
                ok = false;
            }
        }
    }
    ok
}

pub(crate) fn check_unevaluated_props<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    sub: Option<&Prg<'d>>,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for e in inst.entries() {
        if ctx.masks.has_prop(*inst, e.key) {
            continue;
        }
        st.push_key(e.key);
        match sub {
            None => {
                ctx.emit(st, at, format!("property `{}` is unevaluated", e.key));
                ok = false;
            }
            Some(p) => {
                let Some(val) = e.value else {
                    st.pop();
                    continue;
                };
                let o = eval(ctx, p, val, st);
                if o.ok {
                    ctx.masks.record(val, o.ann);
                    ann.prop(e.key);
                } else {
                    ok = false;
                }
            }
        }
        st.pop();
    }
    ok
}
