//! Lower only checked portable opcodes; this module never interprets schemas.
use std::{borrow::Cow, fmt::Write};
use suspect_schema::{
    OwnedProgram, PatternProgram, PatternState, ProgramCheckError, ProgramInstruction as I,
    ProgramSource, ProgramType,
};

use super::emit::{literal, quote};

pub(super) fn source(value: &ProgramSource) -> String {
    format!(
        "SchemaSource({}, {})",
        quote(&value.document),
        quote(&value.pointer)
    )
}

/// Executable text is selected only after the complete typed envelope is checked.
/// No generated package accepts external programs or mutable opcode operands.
pub(super) fn runtime(program: &OwnedProgram) -> Result<Cow<'static, str>, ProgramCheckError> {
    check(program)?;
    Ok(match (program.version, program.profile) {
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE) => {
            Cow::Borrowed(include_str!("validation.dart"))
        }
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE) => {
            Cow::Borrowed(include_str!("validation_v2.dart"))
        }
        (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE) => {
            // Reuse every frozen v2 rule verbatim. Only its private class header
            // changes: v3 overrides node entry and dynamic dispatch. V1/v2
            // emitted bytes never acquire hooks, resources, or a new superclass.
            let base = include_str!("validation_v2.dart");
            assert_eq!(base.matches("final class _ValidationSession {").count(), 1);
            Cow::Owned(format!(
                "{}\n{}",
                base.replacen(
                    "final class _ValidationSession {",
                    "base class _ValidationBase {",
                    1
                ),
                include_str!("validation_v3.dart")
            ))
        }
        _ => unreachable!("checked version/profile pair"),
    })
}

pub(super) fn render(program: &OwnedProgram) -> Result<String, ProgramCheckError> {
    check(program)?;
    let limits = &program.limits;
    let mut out = format!(
        "const _validationLimits = _ValidationLimits({}, {}, {}, {}, {});\n",
        limits.max_depth,
        limits.max_errors,
        limits.max_number_bytes,
        limits.max_equality_steps,
        limits.max_evaluation_steps
    );
    out.push_str("final _validationNodes = <_ValidationNode>[\n");
    for node in &program.nodes {
        writeln!(out, "  _ValidationNode({}, <_Check>[", source(&node.source)).unwrap();
        for check in &node.checks {
            let (op, args) = match &check.instruction {
                I::Always { value } => ("always", format!(", value: {value}")),
                I::Type { types } => (
                    "type",
                    format!(
                        ", types: [{}]",
                        types
                            .iter()
                            .map(|t| format!("_JsonType.{}", type_name(*t)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
                I::Ref { target } => ("ref", format!(", target: {target}")),
                I::Properties { properties } => (
                    "properties",
                    format!(
                        ", properties: [{}]",
                        properties
                            .iter()
                            .map(|p| format!("_Property({}, {})", quote(&p.name), p.target))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
                I::AdditionalProperties { declared, target } => (
                    "additionalProperties",
                    format!(", names: {}, target: {target}", strings(declared)),
                ),
                I::Required { names } => ("required", format!(", names: {}", strings(names))),
                I::Items { target, start } => {
                    ("items", format!(", target: {target}, start: {start}"))
                }
                I::PrefixItems { targets } => ("prefixItems", format!(", targets: {targets:?}")),
                I::AllOf { targets } => ("allOf", format!(", targets: {targets:?}")),
                I::AnyOf { targets } => ("anyOf", format!(", targets: {targets:?}")),
                I::OneOf { targets } => ("oneOf", format!(", targets: {targets:?}")),
                I::Not { target } => ("not", format!(", target: {target}")),
                I::Bound {
                    value,
                    maximum,
                    exclusive,
                } => (
                    "bound",
                    format!(
                        ", number: JsonNumber.parse({}, maxBytes: 65536), maximum: {maximum}, exclusive: {exclusive}",
                        quote(value)
                    ),
                ),
                I::MultipleOf { value } => (
                    "multipleOf",
                    format!(
                        ", number: JsonNumber.parse({}, maxBytes: 65536)",
                        quote(value)
                    ),
                ),
                I::Count {
                    value,
                    maximum,
                    target,
                } => (
                    "count",
                    format!(
                        ", number: JsonNumber.parse({}, maxBytes: 65536), maximum: {maximum}, countType: _JsonType.{}",
                        quote(value),
                        match target {
                            suspect_schema::ProgramCountTarget::String => "string",
                            suspect_schema::ProgramCountTarget::Array => "array",
                            suspect_schema::ProgramCountTarget::Object => "object",
                        }
                    ),
                ),
                I::Enum { values } => (
                    "enumValue",
                    format!(
                        ", literals: [{}]",
                        values.iter().map(literal).collect::<Vec<_>>().join(", ")
                    ),
                ),
                I::Const { value } => ("constValue", format!(", literal: {}", literal(value))),
                I::UniqueItems => ("uniqueItems", String::new()),
                I::Pattern { program } => ("pattern", format!(", pattern: {}", pattern(program))),
                I::If {
                    condition,
                    then_target,
                    else_target,
                } => (
                    "ifValue",
                    format!(
                        ", condition: {condition}, thenTarget: {}, elseTarget: {}",
                        optional_target(*then_target),
                        optional_target(*else_target)
                    ),
                ),
                I::DependentRequired { dependencies } => (
                    "dependentRequired",
                    format!(
                        ", dependencies: [{}]",
                        dependencies
                            .iter()
                            .map(|(trigger, names)| format!(
                                "({}, {})",
                                quote(trigger),
                                strings(names)
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
                I::DependentSchemas { dependencies } => (
                    "dependentSchemas",
                    format!(
                        ", properties: [{}]",
                        dependencies
                            .iter()
                            .map(|p| format!("_Property({}, {})", quote(&p.name), p.target))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
                I::Contains {
                    target,
                    minimum,
                    maximum,
                } => (
                    "contains",
                    format!(
                        ", target: {target}, minimumCount: {}, maximumCount: {}",
                        optional_number(minimum.as_deref()),
                        optional_number(maximum.as_deref())
                    ),
                ),
                I::PatternProperties { patterns } => (
                    "patternProperties",
                    format!(
                        ", patterns: [{}]",
                        patterns
                            .iter()
                            .map(|(name, p, target)| format!(
                                "_PatternProperty({}, {}, {target})",
                                quote(name),
                                pattern(p)
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
                I::AdditionalPropertiesWithPatterns { declared, target } => (
                    "additionalPropertiesWithPatterns",
                    format!(", names: {}, target: {target}", strings(declared)),
                ),
                I::PropertyNames { target } => ("propertyNames", format!(", target: {target}")),
                I::UnevaluatedProperties { target } => {
                    ("unevaluatedProperties", format!(", target: {target}"))
                }
                I::UnevaluatedItems { target } => {
                    ("unevaluatedItems", format!(", target: {target}"))
                }
                // A negative target is an explicit dynamic-dispatch marker, not
                // a usable static fallback. V3's checked side table owns it.
                I::DynamicRef { .. } => ("ref", ", target: -1".into()),
            };
            writeln!(
                out,
                "    _Check({}, _Op.{op}{args}),",
                source(&check.source)
            )
            .unwrap();
        }
        out.push_str("  ]),\n");
    }
    out.push_str("] ;\nfinal _validationRoots = <SchemaSource, int>{\n");
    for root in &program.roots {
        writeln!(out, "  {}: {},", source(&root.source), root.target).unwrap();
    }
    out.push_str("};\n");
    if let Some(context) = &program.resource_context {
        writeln!(
            out,
            "const _nodeResources = <int>{:?};",
            context.node_scopes.iter().map(|s| s.0).collect::<Vec<_>>()
        )
        .unwrap();
        out.push_str("const _validationResources = <_DynamicResource>[\n");
        for resource in &context.resources {
            writeln!(
                out,
                "  _DynamicResource([{}]),",
                resource
                    .dynamic_anchors
                    .iter()
                    .map(|(name, _, target)| format!("({}, {target})", quote(name)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
        out.push_str(
            "];\nfinal _dynamicReferences = Map<_Check, _DynamicReference>.unmodifiable({\n",
        );
        for (node_index, node) in program.nodes.iter().enumerate() {
            for (check_index, check) in node.checks.iter().enumerate() {
                if let I::DynamicRef {
                    target,
                    initial_resource,
                    anchor,
                } = &check.instruction
                {
                    writeln!(out,"  _validationNodes[{node_index}].checks[{check_index}]: _DynamicReference({target}, {initial_resource}, {}),",anchor.as_deref().map_or_else(||"null".into(),quote)).unwrap();
                }
            }
        }
        out.push_str("});\n");
    }
    Ok(out)
}

fn check(program: &OwnedProgram) -> Result<(), ProgramCheckError> {
    program.check()?;
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(ProgramCheckError {
            source: program
                .nodes
                .iter()
                .flat_map(|n| &n.checks)
                .find(|c| matches!(c.instruction, I::DynamicRef { .. }))
                .map(|c| c.source.clone())
                .or_else(|| program.nodes.first().map(|n| n.source.clone())),
            message: "Dart requires a witnessed v1/v2/v3 version/profile pair".into(),
        });
    }
    Ok(())
}

fn optional_target(target: Option<usize>) -> String {
    target.map_or_else(|| "null".into(), |v| v.to_string())
}

fn optional_number(number: Option<&str>) -> String {
    number.map_or_else(
        || "null".into(),
        |v| format!("JsonNumber.parse({}, maxBytes: 65536)", quote(v)),
    )
}

fn pattern(program: &PatternProgram) -> String {
    let states = program
        .states
        .iter()
        .map(|state| match state {
            PatternState::Match => "_PatternState(_PatternOp.match)".into(),
            PatternState::Char { ranges, target } => format!(
                "_PatternState(_PatternOp.char, target: {target}, ranges: [{}])",
                ranges
                    .iter()
                    .map(|[a, b]| format!("({a}, {b})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PatternState::Split { first, second } => {
                format!("_PatternState(_PatternOp.split, target: {first}, second: {second})")
            }
            PatternState::Jump { target } => {
                format!("_PatternState(_PatternOp.jump, target: {target})")
            }
            PatternState::Start { target } => {
                format!("_PatternState(_PatternOp.start, target: {target})")
            }
            PatternState::End { target } => {
                format!("_PatternState(_PatternOp.end, target: {target})")
            }
        })
        .collect::<Vec<String>>()
        .join(",\n");
    format!("_Pattern({}, [{states}])", program.start)
}

fn strings(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|v| quote(v))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn type_name(value: ProgramType) -> &'static str {
    match value {
        ProgramType::Null => "nullValue",
        ProgramType::Boolean => "boolean",
        ProgramType::Integer => "integer",
        ProgramType::Number => "number",
        ProgramType::String => "string",
        ProgramType::Array => "array",
        ProgramType::Object => "object",
    }
}
