//! Dependency-free Rust execution of checked, source-linked validation programs.
use std::fmt;

use serde_json::Value;
use suspect_schema::{OwnedProgram, PatternState, ProgramInstruction, ProgramSource};

use crate::OutFile;

/// A malformed or unsupported compiled program, rejected before artifacts exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationEmissionError {
    /// Responsible original schema or keyword, when available.
    pub source: Option<ProgramSource>,
    /// Reason portable emission was refused.
    pub message: String,
}
impl fmt::Display for ValidationEmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ValidationEmissionError {}

/// Emits a complete native evaluator with no compiler or third-party dependency.
///
/// Admission checks the version, profile, source identities, targets, numeric
/// operands and portable pattern programs before returning any files. Generated
/// `validation::validate` accepts selected root *node indices*, not root ordinals.
/// The generated crate supplies the canonical exact JSON container types.
/// Native recursion admits `max_depth <= 512`; resource counters must fit a
/// 32-bit target. Greater policies fail explicitly rather than being clamped.
/// Numeric division receives a separate shared work allowance equal to
/// `max_evaluation_steps`; exhausting it returns located evaluation failure.
pub fn emit(program: &OwnedProgram) -> Result<Vec<OutFile>, ValidationEmissionError> {
    program.check().map_err(|e| ValidationEmissionError {
        source: e.source,
        message: e.message,
    })?;
    // Shared admission can grow independently of this native adapter. A newer
    // checked envelope must never fall through to the frozen v1 runtime.
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(ValidationEmissionError {
            source: program.roots.first().map(|root| root.source.clone()),
            message:
                "portable Rust validation requires a recognized v1, v2 or v3 version/profile pair"
                    .into(),
        });
    }
    // Bound recursive native stack requests without silently changing policy.
    // The compiler's default 512-level policy remains supported.
    if program.limits.max_depth > 512 {
        return Err(ValidationEmissionError {
            source: program.roots.first().map(|r| r.source.clone()),
            message: "portable Rust evaluation requires max_depth at most 512".into(),
        });
    }
    let at = program.roots.first().map(|root| root.source.clone());
    for (name, value) in [
        ("max_errors", program.limits.max_errors),
        ("max_number_bytes", program.limits.max_number_bytes),
        ("max_equality_steps", program.limits.max_equality_steps),
        ("max_evaluation_steps", program.limits.max_evaluation_steps),
        ("node count", program.nodes.len()),
    ] {
        if u32::try_from(value).is_err() {
            return Err(ValidationEmissionError {
                source: at.clone(),
                message: format!("portable Rust {name} exceeds 32-bit metadata capacity"),
            });
        }
    }
    if let Some(context) = &program.resource_context {
        for (name, value) in [
            ("resource count", context.resources.len()),
            ("node scope count", context.node_scopes.len()),
        ] {
            if u32::try_from(value).is_err() {
                return Err(ValidationEmissionError {
                    source: at.clone(),
                    message: format!("portable Rust {name} exceeds 32-bit metadata capacity"),
                });
            }
        }
        for resource in &context.resources {
            if u32::try_from(resource.dynamic_anchors.len()).is_err() {
                return Err(ValidationEmissionError {
                    source: Some(resource.source.clone()),
                    message: "portable Rust dynamic binding count exceeds 32-bit metadata capacity"
                        .into(),
                });
            }
        }
    }
    let mut text = String::from(
        "//! Source-linked, exact compiled schema validation.\n//!\n//! `Valid` proves all assertions; `Invalid` reports completed mismatches.\n//! `EvaluationFailure` means validity is unknown and cannot be inverted by\n//! logical alternatives. Each call has fresh finite budgets; codec sessions\n//! share those budgets across root validation and branch selection.\nmod number;\nmod pattern;\nmod runtime;\npub use runtime::{validate, ValidationFinding, ValidationOutcome};\n#[allow(unused_imports)]\npub(crate) use runtime::ValidationSession;\nuse runtime::*;\n#[allow(unused_imports)]\nuse pattern::{PatternProgram, PatternState};\n#[allow(unused_imports)]\nuse crate::{JsonNonNullValue, Nullable};\n\nfn program() -> &'static Program { &PROGRAM }\nstatic PROGRAM: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| Program {\n",
    );
    let l = &program.limits;
    text.push_str(&format!("limits: Limits {{ max_depth: {}, max_errors: {}, max_number_bytes: {}, max_equality_steps: {}, max_evaluation_steps: {} }},\nroots: &{:?},\nnodes: vec![\n", l.max_depth,l.max_errors,l.max_number_bytes,l.max_equality_steps,l.max_evaluation_steps,program.roots.iter().map(|r|r.target).collect::<Vec<_>>()));
    for node in &program.nodes {
        text.push_str(&format!(
            "Node {{ source: {}, checks: vec![\n",
            source(&node.source)
        ));
        for check in &node.checks {
            text.push_str(&format!(
                "Check {{ source: {}, instruction: {} }},\n",
                source(&check.source),
                instruction(&check.instruction, &check.source, &node.source)?
            ));
        }
        text.push_str("] },\n");
    }
    if let Some(context) = &program.resource_context {
        text.push_str("],\nresources: &[\n");
        for resource in &context.resources {
            text.push_str(&format!("Resource {{ source: {}, kind: {:?}, canonical_uri: {:?}, base_uri: {:?}, aliases: &{:?}, declaration_source: {}, dynamic_anchors: &[{}] }},\n",
                source(&resource.source),resource.kind,resource.canonical_uri,resource.base_uri,resource.aliases,
                resource.declaration_source.as_ref().map(|s|format!("Some({})",source(s))).unwrap_or_else(||"None".into()),
                resource.dynamic_anchors.iter().map(|(name,at,target)|format!("DynamicBinding {{ name: {name:?}, source: {}, target: {target} }}",source(at))).collect::<Vec<_>>().join(",")));
        }
        text.push_str("],\nnode_scopes: &[\n");
        for (resource, root, address) in &context.node_scopes {
            text.push_str(&format!("NodeScope {{ resource: {resource}, schema_root: {}, canonical_address: {address:?} }},\n",source(root)));
        }
        text.push_str("]\n});\npub use runtime::{Source, Resource, DynamicBinding, NodeScope, resources, node_scopes};\n");
    } else {
        text.push_str("]\n});\n");
    }
    Ok(vec![
        OutFile {
            path: "rust/src/validation.rs".into(),
            content: text,
        },
        OutFile {
            path: "rust/src/validation/runtime.rs".into(),
            content: if program.version == OwnedProgram::V3_VERSION {
                include_str!("rust_validation/runtime_v3.rs")
            } else if program.version == OwnedProgram::V2_VERSION {
                include_str!("rust_validation/runtime_v2.rs")
            } else {
                include_str!("rust_validation/runtime.rs")
            }
            .into(),
        },
        OutFile {
            path: "rust/src/validation/number.rs".into(),
            content: include_str!("rust_validation/number.rs").into(),
        },
        OutFile {
            path: "rust/src/validation/pattern.rs".into(),
            content: include_str!("rust_validation/pattern.rs").into(),
        },
    ])
}
fn source(s: &ProgramSource) -> String {
    format!(
        "Source {{ document: {:?}, pointer: {:?} }}",
        s.document, s.pointer
    )
}
fn literal(value: &Value) -> String {
    match value {
        Value::Null => return "Nullable::Null".into(),
        Value::Bool(v) => return format!("Nullable::Value(JsonNonNullValue::Bool({v}))"),
        Value::Number(v) => {
            return format!(
                "Nullable::Value(JsonNonNullValue::Number({:?}.parse().expect(\"checked JSON literal\")))",
                v.to_string()
            );
        }
        Value::String(v) => {
            return format!("Nullable::Value(JsonNonNullValue::String({v:?}.into()))");
        }
        Value::Array(_) | Value::Object(_) => {}
    }
    // A flat initializer avoids imposing the Rust parser's expression-recursion
    // limit on retained JSON literals. Operands are built once, not per trial.
    let mut text = String::from("{ let mut values: Vec<crate::JsonValue> = Vec::new();\n");
    let mut pending = vec![(value, false)];
    while let Some((value, ready)) = pending.pop() {
        match value {
            Value::Array(values) if !ready => {
                pending.push((value, true));
                pending.extend(values.iter().rev().map(|v| (v, false)));
            }
            Value::Object(values) if !ready => {
                pending.push((value, true));
                pending.extend(values.values().rev().map(|v| (v, false)));
            }
            Value::Array(values) => text.push_str(&format!("let children = values.split_off(values.len() - {}); values.push(Nullable::Value(JsonNonNullValue::Array(children)));\n", values.len())),
            Value::Object(values) => {
                text.push_str(if values.is_empty() { "let object = std::collections::BTreeMap::new();\n" } else { "let mut object = std::collections::BTreeMap::new();\n" });
                for key in values.keys().rev() {
                    text.push_str(&format!("object.insert({key:?}.into(), values.pop().expect(\"compiled literal child\"));\n"));
                }
                text.push_str("values.push(Nullable::Value(JsonNonNullValue::Object(object)));\n");
            }
            Value::Null => text.push_str("values.push(Nullable::Null);\n"),
            Value::Bool(v) => text.push_str(&format!("values.push(Nullable::Value(JsonNonNullValue::Bool({v})));\n")),
            Value::Number(v) => text.push_str(&format!("values.push(Nullable::Value(JsonNonNullValue::Number({:?}.parse().expect(\"checked JSON literal\"))));\n", v.to_string())),
            Value::String(v) => text.push_str(&format!("values.push(Nullable::Value(JsonNonNullValue::String({v:?}.into())));\n")),
        }
    }
    text.push_str("values.pop().expect(\"compiled literal root\") }");
    text
}
fn instruction(
    i: &ProgramInstruction,
    at: &ProgramSource,
    node: &ProgramSource,
) -> Result<String, ValidationEmissionError> {
    use ProgramInstruction as I;
    let body = match i {
        I::Always { value } => format!("Always({value})"),
        I::Type { types } => format!(
            "Type(&[{}])",
            types
                .iter()
                .map(|t| format!("JsonType::{t:?}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
        I::Ref { target } => format!("Ref({target})"),
        I::Not { target } => format!("Not({target})"),
        I::Properties { properties } => format!(
            "Properties(&[{}])",
            properties
                .iter()
                .map(|p| format!("({:?},{})", p.name, p.target))
                .collect::<Vec<_>>()
                .join(",")
        ),
        I::AdditionalProperties { declared, target } => {
            format!("AdditionalProperties {{ declared: &{declared:?}, target: {target} }}")
        }
        I::Required { names } => format!("Required(&{names:?})"),
        I::Items { target, start } => format!("Items {{ target: {target}, start: {start} }}"),
        I::PrefixItems { targets } => format!("PrefixItems(&{targets:?})"),
        I::AllOf { targets } => format!("AllOf(&{targets:?})"),
        I::AnyOf { targets } => format!("AnyOf(&{targets:?})"),
        I::OneOf { targets } => format!("OneOf(&{targets:?})"),
        I::Bound {
            value,
            maximum,
            exclusive,
        } => format!("Bound {{ value: {value:?}, maximum: {maximum}, exclusive: {exclusive} }}"),
        I::MultipleOf { value } => format!("MultipleOf({value:?})"),
        I::Count {
            value,
            maximum,
            target,
        } => format!(
            "Count {{ value: {value:?}, maximum: {maximum}, target: CountTarget::{target:?} }}"
        ),
        I::Enum { values } => format!(
            "Enum(vec![{}])",
            values.iter().map(literal).collect::<Vec<_>>().join(",")
        ),
        I::Const { value } => format!("Const({})", literal(value)),
        I::UniqueItems => "UniqueItems".into(),
        I::Pattern { program } => format!("Pattern({})", pattern(program)),
        I::If {
            condition,
            then_target,
            else_target,
        } => format!(
            "If {{ condition: {condition}, then_target: {then_target:?}, else_target: {else_target:?} }}"
        ),
        I::DependentRequired { dependencies } => format!(
            "DependentRequired(&[{}])",
            dependencies
                .iter()
                .map(|(trigger, names)| format!(
                    "Dependency {{ source: {}, trigger: {trigger:?}, required: &{names:?} }}",
                    source_child(at, trigger)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        I::DependentSchemas { dependencies } => format!(
            "DependentSchemas(&[{}])",
            dependencies
                .iter()
                .map(|p| format!("({:?},{})", p.name, p.target))
                .collect::<Vec<_>>()
                .join(",")
        ),
        I::Contains {
            target,
            minimum,
            maximum,
        } => {
            let limit = |token: &Option<String>, keyword: &str| {
                token
                    .as_ref()
                    .map(|token| {
                        format!(
                            "Some(CountLimit {{ source: {}, token: {token:?} }})",
                            source_child(node, keyword)
                        )
                    })
                    .unwrap_or_else(|| "None".into())
            };
            format!(
                "Contains {{ target: {target}, minimum: {}, maximum: {} }}",
                limit(minimum, "minContains"),
                limit(maximum, "maxContains")
            )
        }
        I::PatternProperties { patterns } => format!(
            "PatternProperties(&[{}])",
            patterns
                .iter()
                .map(|(text, program, target)| format!("({text:?},{},{target})", pattern(program)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        I::AdditionalPropertiesWithPatterns { declared, target } => format!(
            "AdditionalPropertiesWithPatterns {{ declared: &{declared:?}, target: {target} }}"
        ),
        I::PropertyNames { target } => format!("PropertyNames({target})"),
        I::UnevaluatedProperties { target } => format!("UnevaluatedProperties({target})"),
        I::UnevaluatedItems { target } => format!("UnevaluatedItems({target})"),
        I::DynamicRef {
            target,
            initial_resource,
            anchor,
        } => format!(
            "DynamicRef {{ target: {target}, initial_resource: {initial_resource}, anchor: {anchor:?} }}"
        ),
    };
    Ok(format!("Instruction::{body}"))
}

fn source_child(at: &ProgramSource, token: &str) -> String {
    source(&ProgramSource {
        document: at.document.clone(),
        pointer: format!(
            "{}/{}",
            at.pointer,
            token.replace('~', "~0").replace('/', "~1")
        ),
    })
}

fn pattern(program: &suspect_schema::PatternProgram) -> String {
    let states = program
        .states
        .iter()
        .map(|s| match s {
            PatternState::Match => "PatternState::Match".into(),
            PatternState::Char { ranges, target } => {
                format!("PatternState::Char {{ ranges: &{ranges:?}, target: {target} }}")
            }
            PatternState::Split { first, second } => {
                format!("PatternState::Split {{ first: {first}, second: {second} }}")
            }
            PatternState::Jump { target } => {
                format!("PatternState::Jump {{ target: {target} }}")
            }
            PatternState::Start { target } => {
                format!("PatternState::Start {{ target: {target} }}")
            }
            PatternState::End { target } => {
                format!("PatternState::End {{ target: {target} }}")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "PatternProgram {{ start: {}, states: &[{states}] }}",
        program.start
    )
}
