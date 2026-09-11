//! Experimental TypeScript validation artifacts from a compiled owned program.
//!
//! This boundary never discovers or reinterprets OpenAPI keywords. It emits
//! only the proved static instructions supplied by `OwnedCompiler`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{Value, json};
use suspect_schema::{OwnedProgram, ProgramInstruction, ProgramSource};

use crate::OutFile;

/// A portable program cannot be represented faithfully by this runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationEmissionError {
    /// Original source identity, when one instruction or schema is responsible.
    pub source: Option<ProgramSource>,
    /// Located reason emission was refused.
    pub message: String,
}

impl fmt::Display for ValidationEmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ValidationEmissionError {}

fn error(source: Option<&ProgramSource>, message: impl Into<String>) -> ValidationEmissionError {
    ValidationEmissionError {
        source: source.cloned(),
        message: message.into(),
    }
}

/// Emits a standalone exact JSON runtime, validator, compiled program and API
/// documentation. No model conversion or HTTP client is supplied by this seam.
///
/// The program must come from successful `OwnedCompiler` compilation. Public
/// snapshot fields are checked for supported version/profile, source identity,
/// safe metadata integers, valid target indices and numeric operands before
/// any artifact is returned. Enum/const values are parsed from lossless JSON
/// text in the generated module, never evaluated as numeric object literals.
pub fn emit(program: &OwnedProgram) -> Result<Vec<OutFile>, ValidationEmissionError> {
    check_program(program)?;
    let mut nodes = Vec::new();
    for node in &program.nodes {
        let mut checks = Vec::new();
        for check in &node.checks {
            let mut encoded = serde_json::to_value(check).expect("portable instruction serializes");
            let extra = match &check.instruction {
                ProgramInstruction::Enum { values } => {
                    encoded.as_object_mut().unwrap().remove("values");
                    Some(format!(
                        "\"values\":[{}]",
                        values.iter().map(operand).collect::<Vec<_>>().join(",")
                    ))
                }
                ProgramInstruction::Const { value } => {
                    encoded.as_object_mut().unwrap().remove("value");
                    Some(format!("\"value\":{}", operand(value)))
                }
                _ => None,
            };
            let mut text = encoded.to_string();
            if let Some(extra) = extra {
                text.pop();
                text.push(',');
                text.push_str(&extra);
                text.push('}');
            }
            checks.push(text);
        }
        nodes.push(format!(
            "{{\"source\":{},\"checks\":[{}]}}",
            serde_json::to_string(&node.source).unwrap(),
            checks.join(",")
        ));
    }
    let declarations = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            format!("const node{index}: NonNullable<ValidationProgram['nodes'][number]> = {node};")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let all_nodes = (0..nodes.len())
        .map(|index| format!("node{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let resource_declaration = program.resource_context.as_ref().map_or_else(
        String::new,
        |context| {
            format!(
                "\nconst resourceContext: NonNullable<ValidationProgram['resourceContext']> = {};",
                serde_json::to_string(context).unwrap()
            )
        },
    );
    let resource_field = if program.resource_context.is_some() {
        ",resourceContext"
    } else {
        ""
    };
    let mut root_validators = String::new();
    for (index, root) in program.roots.iter().enumerate() {
        // V3 bindings may target a node outside the ordinary static closure.
        // Retain its complete checked resource registry; unentered candidates
        // remain inert at execution. Frozen v1/v2 sparse root slices are unchanged.
        let reachable = if program.resource_context.is_some() {
            (0..nodes.len()).collect()
        } else {
            reachable_nodes(program, root.target)
        };
        let table = reachable
            .iter()
            .map(|target| format!("{target}:node{target}"))
            .collect::<Vec<_>>()
            .join(",");
        let root_notice = if program.resource_context.is_some() {
            "Validate this selected root with the checked resource registry and runtime-selected bindings.".to_owned()
        } else {
            format!("Validate only the finite node closure reachable from selected root {index}.")
        };
        root_validators.push_str(&format!(
            "/** {root_notice} */\nexport const validateRoot{index} = /* @__PURE__ */ createLazyValidator(() => createValidator({{version:{},profile:{},roots:[{}],nodes:Object.assign([],{{{table}}}) as ValidationProgram['nodes'],limits:{}{resource_field}}}));\n",
            quoted(program.version), quoted(program.profile), serde_json::to_string(root).unwrap(), serde_json::to_string(&program.limits).unwrap()
        ));
    }
    let module = format!(
        "// Generated from a proved compiled validation program.\nimport {{ parseJson }} from './json.js';\nimport {{ createLazyValidator, createValidator, type ValidationProgram }} from './validation.js';\n\n{declarations}{resource_declaration}\n\nconst program: ValidationProgram = {{\nversion:{},\nprofile:{},\nroots:{},\nnodes:[{all_nodes}],\nlimits:{}{resource_field}\n}};\n\n/** Selected original OpenAPI schema identities. */\nexport const validationRoots = /* @__PURE__ */ (() => Object.freeze(program.roots.map(root => Object.freeze({{...root.source}}))))();\n/** Validate exact wire values; does not decode language models or send HTTP. */\nexport const validate = /* @__PURE__ */ createLazyValidator(() => createValidator(program));\n{root_validators}",
        quoted(program.version),
        quoted(program.profile),
        serde_json::to_string(&program.roots).unwrap(),
        serde_json::to_string(&program.limits).unwrap(),
    );
    Ok(vec![
        super::json::runtime(),
        OutFile {
            path: "typescript/validation.ts".into(),
            content: include_str!("validation.ts").into(),
        },
        OutFile {
            path: "typescript/pattern.ts".into(),
            content: include_str!("pattern.ts").into(),
        },
        OutFile {
            path: "typescript/validation-resources.ts".into(),
            content: include_str!("validation-resources.ts").into(),
        },
        OutFile {
            path: "typescript/uri.ts".into(),
            content: include_str!("uri.ts").into(),
        },
        OutFile {
            path: "typescript/validation-program.ts".into(),
            content: module,
        },
        OutFile {
            path: "typescript/validation.md".into(),
            content: documentation(program),
        },
    ])
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text)
        .unwrap()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn operand(value: &Value) -> String {
    let text = value.to_string();
    // UTF-8 byte length safely bounds UTF-16 units, JSON nesting, node count,
    // and numeric token length. Representation parsing must not pre-apply the
    // separate numeric/equality schema limits to an unvisited literal operand.
    let cap = text.len();
    format!(
        "/* @__PURE__ */ parseJson({},{{maxLength:{cap},maxDepth:{cap},maxNodes:{cap},maxNumberLength:{cap}}})",
        quoted(&text)
    )
}

fn reachable_nodes(program: &OwnedProgram, root: usize) -> BTreeSet<usize> {
    let mut reached = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if !reached.insert(index) {
            continue;
        }
        for check in &program.nodes[index].checks {
            match &check.instruction {
                ProgramInstruction::Ref { target }
                | ProgramInstruction::Not { target }
                | ProgramInstruction::AdditionalProperties { target, .. }
                | ProgramInstruction::AdditionalPropertiesWithPatterns { target, .. }
                | ProgramInstruction::Contains { target, .. }
                | ProgramInstruction::PropertyNames { target }
                | ProgramInstruction::UnevaluatedProperties { target }
                | ProgramInstruction::UnevaluatedItems { target }
                | ProgramInstruction::Items { target, .. } => pending.push(*target),
                ProgramInstruction::Properties { properties }
                | ProgramInstruction::DependentSchemas {
                    dependencies: properties,
                } => pending.extend(properties.iter().map(|property| property.target)),
                ProgramInstruction::If {
                    condition,
                    then_target,
                    else_target,
                } => {
                    pending.push(*condition);
                    pending.extend(then_target);
                    pending.extend(else_target);
                }
                ProgramInstruction::PatternProperties { patterns } => {
                    pending.extend(patterns.iter().map(|(_, _, target)| *target))
                }
                ProgramInstruction::PrefixItems { targets }
                | ProgramInstruction::AllOf { targets }
                | ProgramInstruction::AnyOf { targets }
                | ProgramInstruction::OneOf { targets } => pending.extend(targets.iter().copied()),
                _ => {}
            }
        }
    }
    reached
}

/// The checked root closure used by a native codec, with graph-local indices.
/// Locations are retained by the containing model record, not interface equality.
/// Only instruction targets are remapped: enum/const instance data is opaque.
pub(super) fn interface(program: &OwnedProgram, root: usize) -> Value {
    let mut indices = BTreeMap::from([(root, 0)]);
    let mut pending = vec![root];
    let mut resource_indices = BTreeMap::new();
    let mut resources = Vec::new();
    let mut scopes = Vec::new();
    let mut nodes = Vec::new();
    let mut next = 0;
    while next < pending.len() {
        let index = pending[next];
        next += 1;
        if let Some(context) = &program.resource_context {
            let resource = context.node_scopes[index].0;
            scopes.push(*resource_indices.entry(resource).or_insert_with(|| {
                resources.push(resource);
                resources.len() - 1
            }));
            for (_, _, target) in &context.resources[resource].dynamic_anchors {
                indices.entry(*target).or_insert_with(|| {
                    pending.push(*target);
                    pending.len() - 1
                });
            }
        }
        let mut checks = Vec::new();
        for check in &program.nodes[index].checks {
            let mut instruction = check.instruction.clone();
            let mut target = |value: &mut usize| {
                let index = *indices.entry(*value).or_insert_with(|| {
                    pending.push(*value);
                    pending.len() - 1
                });
                *value = index;
            };
            match &mut instruction {
                ProgramInstruction::DynamicRef {
                    target: value,
                    initial_resource,
                    ..
                } => {
                    target(value);
                    *initial_resource =
                        *resource_indices
                            .entry(*initial_resource)
                            .or_insert_with(|| {
                                resources.push(*initial_resource);
                                resources.len() - 1
                            });
                }
                ProgramInstruction::Ref { target: value }
                | ProgramInstruction::Not { target: value }
                | ProgramInstruction::AdditionalProperties { target: value, .. }
                | ProgramInstruction::AdditionalPropertiesWithPatterns { target: value, .. }
                | ProgramInstruction::Items { target: value, .. }
                | ProgramInstruction::Contains { target: value, .. }
                | ProgramInstruction::PropertyNames { target: value }
                | ProgramInstruction::UnevaluatedProperties { target: value }
                | ProgramInstruction::UnevaluatedItems { target: value } => target(value),
                ProgramInstruction::Properties { properties }
                | ProgramInstruction::DependentSchemas {
                    dependencies: properties,
                } => {
                    for property in properties {
                        target(&mut property.target);
                    }
                }
                ProgramInstruction::If {
                    condition,
                    then_target,
                    else_target,
                } => {
                    target(condition);
                    for value in then_target.iter_mut().chain(else_target) {
                        target(value);
                    }
                }
                ProgramInstruction::PatternProperties { patterns } => {
                    for (_, _, value) in patterns {
                        target(value);
                    }
                }
                ProgramInstruction::PrefixItems { targets }
                | ProgramInstruction::AllOf { targets }
                | ProgramInstruction::AnyOf { targets }
                | ProgramInstruction::OneOf { targets } => {
                    for value in targets {
                        target(value);
                    }
                }
                _ => {}
            }
            checks.push(instruction);
        }
        nodes.push(json!({"checks":checks}));
    }
    let mut value = json!({"version":program.version,"profile":program.profile,"root":0,"nodes":nodes,"limits":program.limits});
    if let Some(context) = &program.resource_context {
        value["resourceContext"] = json!({"nodeScopes":scopes,"resources":resources.iter().map(|resource| {
            let resource = &context.resources[*resource];
            json!({"kind":resource.kind,"dynamicAnchors":resource.dynamic_anchors.iter().map(|(name,_,target)|json!({"name":name,"target":indices[target]})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>()});
    }
    value
}

fn safe(
    value: usize,
    source: Option<&ProgramSource>,
    name: &str,
) -> Result<(), ValidationEmissionError> {
    if value as u128 > 9_007_199_254_740_991 {
        return Err(error(
            source,
            format!("{name} is not a JavaScript safe integer"),
        ));
    }
    Ok(())
}

fn check_program(program: &OwnedProgram) -> Result<(), ValidationEmissionError> {
    program.check().map_err(|error| ValidationEmissionError {
        source: error.source,
        message: error.message,
    })?;
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(error(
            None,
            "unsupported TypeScript validation version/profile",
        ));
    }
    for (name, value) in [
        ("maxDepth", program.limits.max_depth),
        ("maxErrors", program.limits.max_errors),
        ("maxNumberBytes", program.limits.max_number_bytes),
        ("maxEqualitySteps", program.limits.max_equality_steps),
        ("maxEvaluationSteps", program.limits.max_evaluation_steps),
        ("node count", program.nodes.len()),
    ] {
        safe(value, None, name)?;
    }
    for root in &program.roots {
        safe(root.target, Some(&root.source), "root target")?;
    }
    if let Some(context) = &program.resource_context {
        safe(context.resources.len(), None, "resource count")?;
        for (index, source, _) in &context.node_scopes {
            safe(*index, Some(source), "resource index")?;
        }
        for resource in &context.resources {
            for (_, source, target) in &resource.dynamic_anchors {
                safe(*target, Some(source), "dynamic anchor target")?;
            }
        }
    }
    for node in &program.nodes {
        for check in &node.checks {
            let at = Some(&check.source);
            match &check.instruction {
                ProgramInstruction::DynamicRef {
                    target,
                    initial_resource,
                    ..
                } => {
                    safe(*target, at, "dynamic target")?;
                    safe(*initial_resource, at, "initial resource")?;
                }
                ProgramInstruction::Ref { target }
                | ProgramInstruction::Not { target }
                | ProgramInstruction::AdditionalProperties { target, .. }
                | ProgramInstruction::AdditionalPropertiesWithPatterns { target, .. }
                | ProgramInstruction::Contains { target, .. }
                | ProgramInstruction::PropertyNames { target }
                | ProgramInstruction::UnevaluatedProperties { target }
                | ProgramInstruction::UnevaluatedItems { target } => {
                    safe(*target, at, "schema target")?
                }
                ProgramInstruction::Properties { properties }
                | ProgramInstruction::DependentSchemas {
                    dependencies: properties,
                } => {
                    for property in properties {
                        safe(property.target, at, "property target")?;
                    }
                }
                ProgramInstruction::If {
                    condition,
                    then_target,
                    else_target,
                } => {
                    safe(*condition, at, "condition target")?;
                    for target in then_target.iter().chain(else_target) {
                        safe(*target, at, "conditional target")?;
                    }
                }
                ProgramInstruction::PatternProperties { patterns } => {
                    for (_, _, target) in patterns {
                        safe(*target, at, "pattern property target")?;
                    }
                }
                ProgramInstruction::Items { target, start } => {
                    safe(*target, at, "items target")?;
                    safe(*start, at, "items start")?;
                }
                ProgramInstruction::PrefixItems { targets }
                | ProgramInstruction::AllOf { targets }
                | ProgramInstruction::AnyOf { targets }
                | ProgramInstruction::OneOf { targets } => {
                    for target in targets {
                        safe(*target, at, "branch target")?;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn documentation(program: &OwnedProgram) -> String {
    let mut text = String::from(
        "# Compiled TypeScript validation\n\nThis experimental validator executes the compiled OAS 3.1 / JSON Schema 2020-12 static subset. It does not decode language models or provide an HTTP client.\n\n```ts\nimport { parseJson } from './json.js';\nimport { validate, validationRoots } from './validation-program.js';\nconst result = validate(validationRoots[0]!, parseJson(responseText));\nif (result.kind === 'invalid') console.error(result.findings);\nif (result.kind === 'evaluationFailure') console.error(result.finding);\n```\n\n`valid` means every compiled assertion passed. `invalid` contains capped schema mismatches. `evaluationFailure` means validity could not be determined; logical branches cannot suppress or invert it. Each finding carries the original document URI, schema pointer and RFC 6901 instance pointer. Only selected roots may be requested.\n\nNumbers remain exact `JsonNumber` values. No defaults, coercion, model conversion or unknown-field stripping occurs. Pass values from `parseJson`, or construct values satisfying `WireJsonValue`. Trusted generated modules and JavaScript builtins must remain intact; this API is not isolation from application mutation of imported code.\n\nLimits apply independently to each call. The compiled visit budget covers schema, keyword, branch and collection visits; equality and numeric operand limits remain separate. These are visit/representation limits, not a total byte-work or allocation bound.\n\nThe optional third argument is a trace observer receiving `(source, instancePath, valid)` after each completed schema application, including trials. It receives no incomplete application, must not mutate the instance, and does not receive a fresh budget. Use traces only after the overall result is valid. Observer exceptions produce evaluation failure.\n\nSelected sources:\n\n",
    );
    for root in &program.roots {
        text.push_str(&format!(
            "- <code>{}</code> at <code>{}</code>\n",
            inert_text(&root.source.document),
            inert_text(&root.source.pointer)
        ));
    }
    if program.version == OwnedProgram::V2_VERSION {
        text.push_str(SCOPED_DOCUMENTATION);
    } else if program.version == OwnedProgram::V3_VERSION {
        text.push_str(RESOURCE_DOCUMENTATION);
    }
    text
}

pub(super) const SCOPED_DOCUMENTATION: &str = "\n## Scoped v2 validation\n\nThe checked `suspect.validation.experimental.v2` / `oas31-jsonschema202012-static-applicators` program implements if/then/else, dependentRequired, dependentSchemas, contains with exact minContains/maxContains, patternProperties and pattern-aware additionalProperties, propertyNames, unevaluatedProperties and unevaluatedItems. Each child starts with fresh evaluated-property/item sets; only the documented successful sets propagate. Conditional branches and dependent schemas are not flattened into property lists. Numeric, equality, depth and shared work failures remain evaluation failures through every trial and logical branch. Collection visits, NFA transitions and annotation-set merge candidates spend the same per-call work allowance.\n\nDeclared native fields retain their types. Pattern-matched extras and heterogeneous prefixItems use a checked JsonValue carrier where static types cannot express the constraint. Values retain exact numbers, all permitted keys, absence and null. Decode validates the source schema; encode validates the current wire value again after mutation. Required properties, pattern overlaps, key-name constraints and unevaluated locations are enforced by the codec. Ordinary base closures keep the v1 program. Resource/dynamic schema programs require their separately verified executable profile.\n";

pub(super) const RESOURCE_DOCUMENTATION: &str = "\n## Resources and dynamic references\n\nThe checked `suspect.validation.experimental.v3` / `oas31-jsonschema202012-resources-dynamic` program retains the indexed physical resource boundaries, logical URI names and finite dynamic bindings. The runtime performs no source loading, URI lookup or acquisition. Entering a nested schema enters its indexed resource without evaluating an unselected resource root. The outermost actually entered matching resource wins; unentered candidates are inert. Pointer, empty-fragment and ordinary-anchor fallbacks remain static. Every return and trial restores resource scope. Context-sensitive cycle identities and per-call resource-entry/binding-scan work are exact; failures cannot be inverted or suppressed.\n\nV3 includes the scoped v2 applicators. A dynamically selected target starts with fresh evaluated-property/item sets and propagates only successful annotations. Dynamic positions use checked JsonValue carriers instead of assuming the initial target's native type. Known fields retain their native representations, exact numbers and absence/null distinctions. Decode validates the selected root; encode validates the current wire value again after mutation. Model-only rendering retains explicit codec obligations. Ordinary closures keep their established v1/v2 programs; the planner selects compile_v3 only for source-required resource semantics.\n\nHTTP relative servers use their effective physical retrieval document, independently of logical $self/$id names. Caller documentURL overrides stay explicit. Credential callbacks receive effectiveServerURL for relative OAuth/OIDC endpoint metadata; the SDK does not acquire those endpoints. Native Fetch rewrites of encoded dot segments are refused before sending; a capable explicit transport receives the exact URI spelling.\n";

fn inert_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == ' ' {
                ch.to_string()
            } else {
                format!("&#{};", u32::from(ch))
            }
        })
        .collect()
}
