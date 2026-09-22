//! Object keywords: `properties`, `patternProperties`, `additionalProperties`,
//! `propertyNames`, `dependentSchemas`, `dependentRequired` and
//! `unevaluatedProperties`.

use std::{borrow::Cow, collections::HashSet, rc::Rc};

use suspect_low::{NodeRef, Pointer, ValueKind};

use crate::PatternProgram;
use crate::compile::{Kind, Prg, RefTarget, TypeBits};
use crate::exec::Stack;
use crate::exec::{Ann, Ctx, eval};
use crate::keywords::{formats, refs};

fn pattern_matches<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    program: &PatternProgram,
    text: &str,
) -> Option<bool> {
    match crate::pattern::is_match(program, text, &mut ctx.remaining_steps) {
        Ok(matched) => Some(matched),
        Err(()) => {
            ctx.fail_evaluation(
                st,
                at,
                format!(
                    "schema evaluation exceeds {} evaluation steps",
                    ctx.sch.config().max_evaluation_steps
                ),
            );
            None
        }
    }
}

pub(crate) fn check_properties<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    subs: &[(String, Prg<'d>)],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for e in inst.entries() {
        if !ctx.step(st, at) {
            return false;
        }
        let Some(key) = ctx.text(e.key_node, st, at) else {
            return false;
        };
        let Some(val) = e.value else { continue };
        let mut found = None;
        for (name, sub) in subs {
            if !ctx.step(st, at) {
                return false;
            }
            if *name == key {
                found = Some(sub);
                break;
            }
        }
        let Some(sub) = found else { continue };
        st.push_key(key.clone());
        let o = eval(ctx, sub, val, st);
        st.pop();
        if o.ok {
            ann.prop(key.clone());
        } else {
            ok = false;
        }
    }
    ok
}

pub(crate) fn check_pattern_properties<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    subs: &[(Rc<PatternProgram>, Prg<'d>)],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    for e in inst.entries() {
        if !ctx.step(st, at) {
            return false;
        }
        let Some(key) = ctx.text(e.key_node, st, at) else {
            return false;
        };
        let Some(val) = e.value else { continue };
        for (re, sub) in subs {
            if !ctx.step(st, at) {
                return false;
            }
            let Some(matched) = pattern_matches(ctx, st, at, re, &key) else {
                return false;
            };
            if !matched {
                continue;
            }
            st.push_key(key.clone());
            let o = eval(ctx, sub, val, st);
            st.pop();
            if o.ok {
                ann.prop(key.clone());
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
    except_keys: &[String],
    except_patterns: &[Rc<PatternProgram>],
    schema: Option<&Prg<'d>>,
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let mut ok = true;
    'entries: for e in inst.entries() {
        if !ctx.step(st, at) {
            return false;
        }
        let Some(key) = ctx.text(e.key_node, st, at) else {
            return false;
        };
        for name in except_keys {
            if !ctx.step(st, at) {
                return false;
            }
            if name == &key {
                continue 'entries;
            }
        }
        for pattern in except_patterns {
            if !ctx.step(st, at) {
                return false;
            }
            let Some(matched) = pattern_matches(ctx, st, at, pattern, &key) else {
                return false;
            };
            if matched {
                continue 'entries;
            }
        }
        let Some(val) = e.value else { continue };
        match schema {
            None => {
                st.push_key(key.clone());
                ctx.emit(
                    st,
                    at,
                    format!(
                        "property `{}` is not allowed by `additionalProperties: false`",
                        key
                    ),
                );
                st.pop();
                ok = false;
            }
            Some(sub) => {
                st.push_key(key.clone());
                let o = eval(ctx, sub, val, st);
                st.pop();
                if o.ok {
                    ann.prop(key.clone());
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
    for entry in inst.entries() {
        if !ctx.step(st, at) {
            return false;
        }
        let Some(key) = ctx.text(entry.key_node, st, at) else {
            return false;
        };
        if !string_schema_ok(ctx, st, sub, &key, 0) {
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
fn string_schema_ok<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    prog: &Prg<'d>,
    s: &str,
    depth: usize,
) -> bool {
    if !ctx.step(st, &prog.path) {
        return false;
    }
    if ctx.depth.saturating_add(depth) > ctx.sch.config().max_depth {
        ctx.fail_evaluation(
            st,
            &prog.path,
            "property-name schema evaluation depth exceeded".into(),
        );
        return false;
    }
    let pushed = ctx.dyn_scope.last() != Some(&prog.resource);
    if pushed {
        ctx.dyn_scope.push(prog.resource.clone());
    }
    let mut ok = true;
    for chk in prog.checks.iter().chain(&prog.tail) {
        if !ctx.step(st, &chk.at) {
            ok = false;
            break;
        }
        match &chk.kind {
            Kind::Always(b) => ok &= *b,
            Kind::Type(bits) => ok &= bits.0 & TypeBits::STR != 0,
            Kind::Enum(vals) => {
                let mut matched = false;
                for value in vals {
                    if !ctx.step(st, &chk.at) {
                        break;
                    }
                    if value.kind() == ValueKind::Str
                        && ctx
                            .text(*value, st, &chk.at)
                            .is_some_and(|value| value == s)
                    {
                        matched = true;
                        break;
                    }
                }
                ok &= matched;
            }
            Kind::Const(v) => {
                ok &= v.kind() == ValueKind::Str
                    && ctx.text(*v, st, &chk.at).is_some_and(|value| value == s)
            }
            Kind::MinLength(n) => ok &= n.allows_min(s.chars().count()),
            Kind::MaxLength(n) => ok &= n.allows_max(s.chars().count()),
            Kind::Pattern(program) => {
                ok &= pattern_matches(ctx, st, &chk.at, program, s).unwrap_or(false)
            }
            Kind::Format(name) => ok &= formats::validate(name, s),
            Kind::AllOf(subs) => {
                for sub in subs {
                    if !string_schema_ok(ctx, st, sub, s, depth + 1) {
                        ok = false;
                        break;
                    }
                }
            }
            Kind::AnyOf(subs) => {
                let mut matched = false;
                for sub in subs {
                    matched = string_schema_ok(ctx, st, sub, s, depth + 1);
                    if matched || ctx.aborted {
                        break;
                    }
                }
                ok &= matched;
            }
            Kind::OneOf(subs) => {
                let mut matches = 0;
                for sub in subs {
                    matches += usize::from(string_schema_ok(ctx, st, sub, s, depth + 1));
                    if ctx.aborted {
                        break;
                    }
                }
                ok &= matches == 1;
            }
            Kind::Not(inner) => ok &= !string_schema_ok(ctx, st, inner, s, depth + 1),
            Kind::Ref(RefTarget::Local(ptr)) => {
                ok &= match refs::resolve_target(ctx, ptr) {
                    Ok(Some(p)) => string_schema_ok(ctx, st, &p, s, depth + 1),
                    Ok(None) => {
                        ctx.fail_evaluation(st, &chk.at, "unresolvable property-name $ref".into());
                        false
                    }
                    Err(error) => {
                        ctx.fail_evaluation(
                            st,
                            &chk.at,
                            format!("cannot compile property-name $ref: {error}"),
                        );
                        false
                    }
                };
            }
            Kind::If { cond, then, alt } => {
                let c = string_schema_ok(ctx, st, cond, s, depth + 1);
                let branch = if c { then } else { alt };
                if !ctx.aborted
                    && let Some(b) = branch
                {
                    ok &= string_schema_ok(ctx, st, b, s, depth + 1);
                }
            }
            Kind::Ref(RefTarget::External) => {
                ctx.fail_evaluation(
                    st,
                    &chk.at,
                    "external schema resolution not configured".into(),
                );
                ok = false;
            }
            Kind::DynamicRef { target, anchor } => {
                let resolved = refs::dynamic_target(ctx, st, &chk.at, target, anchor.as_deref());
                ok &= match resolved {
                    Some(RefTarget::Local(pointer)) => match refs::resolve_target(ctx, &pointer) {
                        Ok(Some(program)) => string_schema_ok(ctx, st, &program, s, depth + 1),
                        result => {
                            ctx.fail_evaluation(
                                st,
                                &chk.at,
                                match result {
                                    Err(error) => {
                                        format!("cannot compile property-name $dynamicRef: {error}")
                                    }
                                    _ => "unresolvable property-name $dynamicRef".into(),
                                },
                            );
                            false
                        }
                    },
                    Some(RefTarget::External) => {
                        ctx.fail_evaluation(
                            st,
                            &chk.at,
                            "external schema resolution not configured".into(),
                        );
                        false
                    }
                    None => false,
                };
            }
            // String-inapplicable keywords pass vacuously.
            _ => {}
        }
        if ctx.aborted {
            ok = false;
            break;
        }
    }
    if pushed {
        ctx.dyn_scope.pop();
    }
    ok
}

pub(crate) fn check_dependent_schemas<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &mut Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    subs: &[(String, Prg<'d>)],
    ann: &mut Ann<'d>,
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let Some(keys) = present_keys(ctx, st, at, inst) else {
        return false;
    };
    let mut ok = true;
    for (key, sub) in subs {
        if !ctx.step(st, at) {
            return false;
        }
        if !keys.contains(key.as_str()) {
            continue;
        }
        // Applied to the whole object; its inner evaluations count.
        let o = eval(ctx, sub, *inst, st);
        if o.ok {
            ok &= ctx.merge_annotations(st, at, ann, o.ann);
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
    reqs: &[(String, Vec<Box<str>>)],
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let Some(keys) = present_keys(ctx, st, at, inst) else {
        return false;
    };
    let mut ok = true;
    for (key, deps) in reqs {
        if !ctx.step(st, at) {
            return false;
        }
        if !keys.contains(key.as_str()) {
            continue;
        }
        for d in deps {
            if !ctx.step(st, at) {
                return false;
            }
            if !keys.contains(d.as_ref()) {
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

/// `required` checks presence only. It produces no evaluated-property
/// annotation; that belongs to applicable schema applicators.
pub(crate) fn check_required<'a, 'd>(
    ctx: &mut Ctx<'a, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
    names: &[String],
) -> bool {
    if inst.kind() != ValueKind::Object {
        return true;
    }
    let Some(keys) = present_keys(ctx, st, at, inst) else {
        return false;
    };
    let mut ok = true;
    for n in names {
        if !ctx.step(st, at) {
            return false;
        }
        if !keys.contains(n.as_str()) {
            ctx.emit(st, at, format!("required property `{n}` is missing"));
            ok = false;
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
        if !ctx.step(st, at) {
            return false;
        }
        let Some(key) = ctx.text(e.key_node, st, at) else {
            return false;
        };
        if ann.has_prop(&key) {
            continue;
        }
        st.push_key(key.clone());
        match sub {
            None => {
                ctx.emit(st, at, format!("property `{key}` is unevaluated"));
                ok = false;
            }
            Some(p) => {
                let Some(val) = e.value else {
                    st.pop();
                    continue;
                };
                let o = eval(ctx, p, val, st);
                if o.ok {
                    ann.prop(key.clone());
                } else {
                    ok = false;
                }
            }
        }
        st.pop();
    }
    ok
}

/// Index presence once per keyword instead of repeatedly using LowDoc's
/// unmetered linear lookup for every required/dependent property.
fn present_keys<'d>(
    ctx: &mut Ctx<'_, 'd>,
    st: &Stack<'d>,
    at: &Pointer,
    inst: &NodeRef<'d>,
) -> Option<HashSet<Cow<'d, str>>> {
    let mut keys = HashSet::new();
    for entry in inst.entries() {
        if !ctx.step(st, at) {
            return None;
        }
        let key = ctx.text(entry.key_node, st, at)?;
        if entry.value.is_some() {
            keys.insert(key);
        }
    }
    Some(keys)
}
