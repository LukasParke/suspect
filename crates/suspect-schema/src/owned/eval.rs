//! Per-call evaluation state; compiled nodes and source values never mutate.

use rustc_hash::{FxHashMap, FxHashSet};
use suspect_low::ValueKind;

use super::*;
use crate::equality::{EqualityBudget, EqualityValue};

// Keep the error arm small in every recursive frame. Physical SourceIds carry
// URI/pointer data; reserving an inline finding for every `?` can exhaust an
// ordinary debug-build stack before the configured depth limit is reached.
type Failure = Box<OwnedFinding>;

struct State<'a> {
    config: &'a Config,
    out: Vec<OwnedFinding>,
    equality: EqualityBudget,
    numbers: FxHashMap<usize, Result<ExactNumber, String>>,
    active: FxHashSet<(usize, usize, usize)>,
    depth: usize,
    remaining_steps: usize,
    collect: bool,
    resource_stack: Vec<usize>,
    entered_resources: FxHashSet<usize>,
    context: usize,
    contexts: FxHashMap<(usize, usize), usize>,
}

#[derive(Default)]
struct Annotations {
    properties: BTreeSet<String>,
    items: BTreeSet<usize>,
}

struct Evaluated {
    valid: bool,
    annotations: Annotations,
}

impl State<'_> {
    fn enter_resource(
        &mut self,
        resource: usize,
        source: &SourceId,
        path: &Pointer,
    ) -> Result<Option<usize>, Failure> {
        if self.entered_resources.contains(&resource) {
            return Ok(None);
        }
        self.step(source, path)?;
        let previous = self.context;
        let next = self.contexts.len().checked_add(1).ok_or_else(|| {
            finding(
                source,
                path,
                "resource context identity exceeds addressable capacity",
            )
        })?;
        self.context = *self.contexts.entry((previous, resource)).or_insert(next);
        self.entered_resources.insert(resource);
        self.resource_stack.push(resource);
        Ok(Some(previous))
    }

    fn leave_resource(&mut self, previous: Option<usize>) {
        if let Some(previous) = previous {
            let resource = self.resource_stack.pop().expect("entered resource");
            self.entered_resources.remove(&resource);
            self.context = previous;
        }
    }

    fn merge(
        &mut self,
        into: &mut Annotations,
        other: Annotations,
        source: &SourceId,
        path: &Pointer,
    ) -> Result<(), Failure> {
        if self.collect {
            for name in other.properties {
                self.step(source, path)?;
                into.properties.insert(name);
            }
            for index in other.items {
                self.step(source, path)?;
                into.items.insert(index);
            }
        }
        Ok(())
    }

    fn mark_property(&self, into: &mut Annotations, name: &str) {
        if self.collect {
            into.properties.insert(name.to_owned());
        }
    }

    fn mark_item(&self, into: &mut Annotations, index: usize) {
        if self.collect {
            into.items.insert(index);
        }
    }

    fn pattern(
        &mut self,
        program: &crate::PatternProgram,
        text: &str,
        source: &SourceId,
        path: &Pointer,
    ) -> Result<bool, Failure> {
        crate::pattern::is_match(program, text, &mut self.remaining_steps).map_err(|()| {
            failure(
                source,
                path,
                &format!(
                    "schema evaluation exceeds {} evaluation steps",
                    self.config.max_evaluation_steps
                ),
            )
        })
    }

    fn step(&mut self, source: &SourceId, path: &Pointer) -> Result<(), Failure> {
        if self.remaining_steps == 0 {
            return Err(failure(
                source,
                path,
                &format!(
                    "schema evaluation exceeds {} evaluation steps",
                    self.config.max_evaluation_steps
                ),
            ));
        }
        self.remaining_steps -= 1;
        Ok(())
    }

    fn emit(&mut self, source: &SourceId, path: &Pointer, message: &str) {
        if self.config.max_errors == 0 || self.out.len() < self.config.max_errors {
            self.out.push(finding(source, path, message));
        }
    }

    fn number(&mut self, value: &Value) -> Result<&ExactNumber, String> {
        self.numbers
            .entry(value as *const Value as usize)
            .or_insert_with(|| {
                ExactNumber::parse(
                    value
                        .as_number()
                        .expect("numeric assertion instance")
                        .as_str()
                        .as_bytes(),
                    self.config.max_number_bytes,
                )
                .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

fn finding(source: &SourceId, path: &Pointer, message: &str) -> OwnedFinding {
    OwnedFinding {
        source: source.clone(),
        instance_path: path.clone(),
        message: message.into(),
    }
}

fn failure(source: &SourceId, path: &Pointer, message: &str) -> Failure {
    Box::new(finding(source, path, message))
}

pub(super) fn validate(program: &OwnedSchema, root: usize, instance: &Value) -> OwnedOutcome {
    let mut state = State {
        config: &program.config,
        out: Vec::new(),
        equality: EqualityBudget::new(&program.config),
        numbers: FxHashMap::default(),
        active: FxHashSet::default(),
        depth: 0,
        remaining_steps: program.config.max_evaluation_steps,
        collect: program.applicators,
        resource_stack: Vec::new(),
        entered_resources: FxHashSet::default(),
        context: 0,
        contexts: FxHashMap::default(),
    };
    match eval(program, root, instance, &Pointer::root(), &mut state) {
        Ok(result) if result.valid => OwnedOutcome::Valid,
        Ok(_) => OwnedOutcome::Invalid(state.out),
        Err(error) => OwnedOutcome::EvaluationFailure(*error),
    }
}

fn eval(
    program: &OwnedSchema,
    index: usize,
    instance: &Value,
    path: &Pointer,
    state: &mut State<'_>,
) -> Result<Evaluated, Failure> {
    let node = &program.nodes[index];
    state.step(&node.source, path)?;
    if state.depth >= state.config.max_depth {
        return Err(failure(
            &node.source,
            path,
            &format!("schema evaluation depth exceeds {}", state.config.max_depth),
        ));
    }
    let entered = if let Some(resources) = &program.resources {
        state.enter_resource(resources.node_scopes[index].0, &node.source, path)?
    } else {
        None
    };
    let identity = (index, instance as *const Value as usize, state.context);
    if !state.active.insert(identity) {
        state.leave_resource(entered);
        return Err(failure(
            &node.source,
            path,
            "recursive schema evaluation revisited the same schema and instance without progress",
        ));
    }
    state.depth += 1;
    let result = run(program, node, instance, path, state);
    state.depth -= 1;
    state.active.remove(&identity);
    state.leave_resource(entered);
    result
}

fn trial(
    program: &OwnedSchema,
    index: usize,
    instance: &Value,
    path: &Pointer,
    state: &mut State<'_>,
) -> Result<Evaluated, Failure> {
    let findings = std::mem::take(&mut state.out);
    let result = eval(program, index, instance, path, state);
    state.out = findings;
    result
}

// Each opcode owns a small call frame. A monolithic debug-build match reserves
// every opcode's locals at each recursive entry and can exhaust a normal stack
// before max_depth. The macro keeps one copy of each rule and uniform context.
type Handler = fn(
    &OwnedSchema,
    &Node,
    &Check,
    &Value,
    &Pointer,
    &mut State<'_>,
    &mut Annotations,
) -> Result<bool, Failure>;

macro_rules! instructions {
    (context($program:ident,$node:ident,$check:ident,$instance:ident,$path:ident,$state:ident,$annotations:ident,$produced:ident,$at:ident);
        $($handler:ident: $pattern:pat => $body:block),* $(,)?) => {
        #[allow(unused_variables)] // patterns bind operands only in their handler
        fn run(program:&OwnedSchema,node:&Node,instance:&Value,path:&Pointer,state:&mut State<'_>) -> Result<Evaluated,Failure> {
            let mut valid=true;
            let mut annotations=Annotations::default();
            for check in &node.checks {
                state.step(&check.source,path)?;
                let handler:Handler=match &check.kind { $($pattern => self::$handler),* };
                valid &= handler(program,node,check,instance,path,state,&mut annotations)?;
            }
            Ok(Evaluated {valid,annotations:if valid {annotations} else {Annotations::default()}})
        }
        $(
            #[inline(never)]
            #[allow(unused_variables)] // uniform opcode context; some rules need only a subset
            fn $handler($program:&OwnedSchema,$node:&Node,$check:&Check,$instance:&Value,$path:&Pointer,$state:&mut State<'_>,$annotations:&mut Annotations) -> Result<bool,Failure> {
                let $at=&$check.source;
                #[allow(unused_mut, unused_assignments)] // scalar rules keep it empty; ref rules move child sets
                let mut $produced=Annotations::default();
                let ok=match &$check.kind { $pattern => $body, _=>unreachable!("opcode dispatcher") };
                if ok { $state.merge($annotations,$produced,$at,$path)?; }
                Ok(ok)
            }
        )*
    };
}

instructions! {
    context(program,node,check,instance,path,state,annotations,produced,at);
            always: Kind::Always(value) => {
                if !value {
                    state.emit(at, path, "value is rejected by a false schema");
                }
                *value
            },
            type_check: Kind::Type(bits) => {
                let kind = instance.kind();
                let integral = if kind == ValueKind::Float
                    && bits.0 & TypeBits::INT != 0
                    && bits.0 & TypeBits::NUM == 0
                {
                    state
                        .number(instance)
                        .map_err(|e| finding(at, path, &e))?
                        .is_integral()
                } else {
                    false
                };
                let ok = bits.matches(kind, integral);
                if !ok {
                    state.emit(at, path, "instance does not match the declared type");
                }
                ok
            },
            pattern: Kind::Pattern(program_pattern) => {
                if let Some(text) = instance.as_str() {
                    match crate::pattern::is_match(
                        program_pattern,
                        text,
                        &mut state.remaining_steps,
                    ) {
                        Ok(true) => true,
                        Ok(false) => {
                            state.emit(at, path, "string does not match `pattern`");
                            false
                        }
                        Err(()) => {
                            return Err(failure(
                                at,
                                path,
                                &format!(
                                    "schema evaluation exceeds {} evaluation steps",
                                    state.config.max_evaluation_steps
                                ),
                            ));
                        }
                    }
                } else {
                    true
                }
            },
            reference: Kind::Reference(index) => {
                let result=eval(program,*index,instance,path,state)?;
                produced=result.annotations;
                result.valid
            },
            dynamic_reference: Kind::DynamicReference { target, initial_resource:_, anchor } => {
                let resources=program.resources.as_ref().expect("compiled v3 resources");
                let mut selected=*target;
                if let Some(name)=anchor {
                    'lookup: for position in 0..state.resource_stack.len() {
                        let resource=state.resource_stack[position];
                        state.step(at,path)?;
                        for (candidate,_,target) in &resources.resources[resource].dynamic_anchors {
                            state.step(at,path)?;
                            if candidate==name {
                                selected = *target;
                                break 'lookup;
                            }
                        }
                    }
                }
                let result=eval(program,selected,instance,path,state)?;
                produced=result.annotations;
                result.valid
            },
            properties: Kind::Properties(properties) => {
                let mut ok = true;
                if let Some(object) = instance.as_object() {
                    for (name, child) in properties {
                        state.step(at, path)?;
                        if let Some(value) = object.get(name) {
                            ok &= eval(program, *child, value, &path.push(name), state)?.valid;
                            state.mark_property(&mut produced,name);
                        }
                    }
                }
                ok
            },
            additional_properties: Kind::AdditionalProperties { declared, schema } => {
                let mut ok = true;
                if let Some(object) = instance.as_object() {
                    for (name, value) in object {
                        state.step(at, path)?;
                        if !declared.contains(name) {
                            ok &= eval(program, *schema, value, &path.push(name), state)?.valid;
                            state.mark_property(&mut produced,name);
                        }
                    }
                }
                ok
            },
            required: Kind::Required(names) => {
                let mut ok = true;
                if let Some(object) = instance.as_object() {
                    for name in names {
                        state.step(at, path)?;
                        if !object.contains_key(name) {
                            state.emit(at, path, &format!("required property {name:?} is absent"));
                            ok = false;
                        }
                    }
                }
                ok
            },
            items: Kind::Items { schema, start } => {
                let mut ok = true;
                if let Some(array) = instance.as_array() {
                    for (index, value) in array.iter().enumerate().skip(*start) {
                        state.step(at, path)?;
                        ok &= eval(
                            program,
                            *schema,
                            value,
                            &path.push(&index.to_string()),
                            state,
                        )?.valid;
                        state.mark_item(&mut produced,index);
                    }
                }
                ok
            },
            prefix_items: Kind::PrefixItems(schemas) => {
                let mut ok = true;
                if let Some(array) = instance.as_array() {
                    for (index, (value, schema)) in array.iter().zip(schemas).enumerate() {
                        state.step(at, path)?;
                        ok &= eval(
                            program,
                            *schema,
                            value,
                            &path.push(&index.to_string()),
                            state,
                        )?.valid;
                        state.mark_item(&mut produced,index);
                    }
                }
                ok
            },
            all_of: Kind::AllOf(schemas) => {
                let mut ok = true;
                let mut passing=Vec::new();
                for schema in schemas {
                    state.step(at, path)?;
                    let result=eval(program, *schema, instance, path, state)?;
                    ok &= result.valid;
                    if state.collect && result.valid { passing.push(result.annotations); }
                }
                if ok {
                    for ann in passing { state.merge(&mut produced,ann,at,path)?; }
                }
                ok
            },
            alternatives: Kind::AnyOf(schemas) | Kind::OneOf(schemas) => {
                let mut accepted = 0;
                let mut passing=Vec::new();
                for schema in schemas {
                    state.step(at, path)?;
                    let result=trial(program, *schema, instance, path, state)?;
                    if result.valid {
                        accepted += 1;
                        if state.collect { passing.push(result.annotations); }
                    }
                }
                let ok = if matches!(check.kind, Kind::AnyOf(_)) {
                    accepted != 0
                } else {
                    accepted == 1
                };
                if !ok {
                    state.emit(
                        at,
                        path,
                        &format!("composition matched {accepted} alternatives"),
                    );
                } else {
                    for ann in passing { state.merge(&mut produced,ann,at,path)?; }
                }
                ok
            },
            not: Kind::Not(schema) => {
                let ok = !trial(program, *schema, instance, path, state)?.valid;
                if !ok {
                    state.emit(at, path, "instance matches the negated schema");
                }
                ok
            },
            conditional: Kind::If { condition, then_target, else_target } => {
                let condition=trial(program,*condition,instance,path,state)?;
                let selected=if condition.valid { then_target } else { else_target };
                // if and then/else are separate keywords: a passing condition
                // contributes independently, but never seeds the branch scope.
                if condition.valid { state.merge(annotations,condition.annotations,at,path)?; }
                if let Some(target)=selected {
                    let result=eval(program,*target,instance,path,state)?;
                    produced=result.annotations;
                    result.valid
                } else { true }
            },
            dependent_required: Kind::DependentRequired(dependencies) => {
                let mut ok=true;
                if let Some(object)=instance.as_object() {
                    for (trigger,names) in dependencies {
                        state.step(at,path)?;
                        if object.contains_key(trigger) {
                            for name in names {
                                state.step(at,path)?;
                                if !object.contains_key(name) {
                                    state.emit(&at.child(trigger),path,&format!("required property {name:?} is absent while {trigger:?} is present"));
                                    ok=false;
                                }
                            }
                        }
                    }
                }
                ok
            },
            dependent_schemas: Kind::DependentSchemas(dependencies) => {
                let mut ok=true;
                let mut passing=Vec::new();
                if let Some(object)=instance.as_object() {
                    for (trigger,target) in dependencies {
                        state.step(at,path)?;
                        if object.contains_key(trigger) {
                            let result=eval(program,*target,instance,path,state)?;
                            ok &= result.valid;
                            if result.valid { passing.push(result.annotations); }
                        }
                    }
                }
                if ok { for ann in passing { state.merge(&mut produced,ann,at,path)?; } }
                ok
            },
            contains: Kind::Contains { schema, minimum, maximum } => {
                if let Some(array)=instance.as_array() {
                    let mut matched=0;
                    for (index,value) in array.iter().enumerate() {
                        state.step(at,path)?;
                        if trial(program,*schema,value,&path.push(&index.to_string()),state)?.valid {
                            matched+=1;
                            state.mark_item(&mut produced,index);
                        }
                    }
                    let min=minimum.as_ref().map_or(CountBound::Finite(1),|limit|limit.bound);
                    let contains_ok=matched != 0 || matches!(min,CountBound::Finite(0));
                    if contains_ok {
                        state.merge(annotations,std::mem::take(&mut produced),at,path)?;
                    }
                    let lower=min.allows_min(matched);
                    let upper=maximum.as_ref().is_none_or(|limit|limit.bound.allows_max(matched));
                    if !lower {
                        let source=if minimum.is_some() { node.source.child("minContains") } else { at.clone() };
                        state.emit(&source,path,&format!("array has {matched} contains matches, fewer than required"));
                    }
                    if !upper {
                        state.emit(&node.source.child("maxContains"),path,&format!("array has {matched} contains matches, more than allowed"));
                    }
                    lower && upper
                } else { true }
            },
            pattern_properties: Kind::PatternProperties(patterns) => {
                let mut ok=true;
                if let Some(object)=instance.as_object() {
                    for (name,value) in object {
                        state.step(at,path)?;
                        for (_,pattern,target) in patterns {
                            state.step(at,path)?;
                            if state.pattern(pattern,name,at,path)? {
                                ok &= eval(program,*target,value,&path.push(name),state)?.valid;
                                state.mark_property(&mut produced,name);
                            }
                        }
                    }
                }
                ok
            },
            additional_with_patterns: Kind::AdditionalPropertiesWithPatterns { declared, schema } => {
                let patterns=node.checks.iter().find_map(|check| {
                    if let Kind::PatternProperties(patterns)=&check.kind { Some(patterns) } else { None }
                }).expect("compiled adjacent patterns");
                let mut ok=true;
                if let Some(object)=instance.as_object() {
                    'members: for (name,value) in object {
                        state.step(at,path)?;
                        if declared.contains(name) { continue }
                        for (_,pattern,_) in patterns {
                            state.step(at,path)?;
                            if state.pattern(pattern,name,at,path)? { continue 'members }
                        }
                        ok &= eval(program,*schema,value,&path.push(name),state)?.valid;
                        state.mark_property(&mut produced,name);
                    }
                }
                ok
            },
            property_names: Kind::PropertyNames(target) => {
                let mut ok=true;
                if let Some(object)=instance.as_object() {
                    for name in object.keys() {
                        state.step(at,path)?;
                        // A string instance is not a schema identity. Its
                        // temporary address is live for the entire child trial.
                        let key=Value::String(name.clone());
                        ok &= eval(program,*target,&key,&path.push(name),state)?.valid;
                    }
                }
                ok
            },
            unevaluated_properties: Kind::UnevaluatedProperties(target) => {
                let mut ok=true;
                if let Some(object)=instance.as_object() {
                    for (name,value) in object {
                        state.step(at,path)?;
                        if !annotations.properties.contains(name) {
                            ok &= eval(program,*target,value,&path.push(name),state)?.valid;
                            state.mark_property(&mut produced,name);
                        }
                    }
                }
                ok
            },
            unevaluated_items: Kind::UnevaluatedItems(target) => {
                let mut ok=true;
                if let Some(array)=instance.as_array() {
                    for (index,value) in array.iter().enumerate() {
                        state.step(at,path)?;
                        if !annotations.items.contains(&index) {
                            ok &= eval(program,*target,value,&path.push(&index.to_string()),state)?.valid;
                            state.mark_item(&mut produced,index);
                        }
                    }
                }
                ok
            },
            bound: Kind::Bound {
                number,
                maximum,
                exclusive,
            } => {
                if instance.is_number() {
                    let order = state
                        .number(instance)
                        .map_err(|e| finding(at, path, &e))?
                        .cmp(number);
                    let ok = if *maximum {
                        order.is_lt() || (order.is_eq() && !exclusive)
                    } else {
                        order.is_gt() || (order.is_eq() && !exclusive)
                    };
                    if !ok {
                        state.emit(at, path, &format!("numeric value violates bound {number}"));
                    }
                    ok
                } else {
                    true
                }
            },
            multiple_of: Kind::MultipleOf(divisor) => {
                if instance.is_number() {
                    let ok = divisor
                        .contains(state.number(instance).map_err(|e| finding(at, path, &e))?);
                    if !ok {
                        state.emit(
                            at,
                            path,
                            &format!("numeric value is not a multiple of {divisor}"),
                        );
                    }
                    ok
                } else {
                    true
                }
            },
            count: Kind::Count {
                bound,
                maximum,
                target,
                ..
            } => {
                let count = match target {
                    CountTarget::String => instance.as_str().map(|text| text.chars().count()),
                    CountTarget::Array => instance.as_array().map(Vec::len),
                    CountTarget::Object => instance.as_object().map(serde_json::Map::len),
                };
                if let Some(count) = count {
                    let ok = if *maximum {
                        bound.allows_max(count)
                    } else {
                        bound.allows_min(count)
                    };
                    if !ok {
                        state.emit(
                            at,
                            path,
                            &format!("size {count} violates cardinality {bound}"),
                        );
                    }
                    ok
                } else {
                    true
                }
            },
            enumeration: Kind::Enum => {
                let values = program
                    .contract
                    .source(at)
                    .expect("retained enum")
                    .as_array()
                    .expect("compiled enum");
                let mut ok = false;
                for value in values {
                    state.step(at, path)?;
                    if state
                        .equality
                        .compare(instance, value)
                        .map_err(|e| finding(at, path, &e))?
                    {
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    state.emit(at, path, "instance does not equal any enum value");
                }
                ok
            },
            constant: Kind::Const => {
                let value = program.contract.source(at).expect("retained const");
                let ok = state
                    .equality
                    .compare(instance, value)
                    .map_err(|e| finding(at, path, &e))?;
                if !ok {
                    state.emit(at, path, "instance does not equal the const value");
                }
                ok
            },
            unique_items: Kind::UniqueItems => {
                let mut ok = true;
                if let Some(array) = instance.as_array() {
                    'items: for (index, value) in array.iter().enumerate() {
                        state.step(at, path)?;
                        for previous in &array[..index] {
                            state.step(at, path)?;
                            if state
                                .equality
                                .compare(value, previous)
                                .map_err(|e| finding(at, path, &e))?
                            {
                                state.emit(at, path, "array contains equal items");
                                ok = false;
                                break 'items;
                            }
                        }
                    }
                }
                ok
            },
}
