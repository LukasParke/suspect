//! Shared admission of public portable snapshots, without revisiting schemas.

use iri_string::types::UriAbsoluteStr;

use super::*;
#[path = "resource_check.rs"]
mod resources;

fn error(source: Option<&ProgramSource>, message: impl Into<String>) -> ProgramCheckError {
    ProgramCheckError {
        source: source.cloned(),
        message: message.into(),
    }
}

fn source(source: &ProgramSource) -> Result<(), ProgramCheckError> {
    if UriAbsoluteStr::new(&source.document).is_err()
        || source.pointer.starts_with('#')
        || Pointer::parse(&source.pointer).is_err()
    {
        return Err(error(
            Some(source),
            "source requires an absolute fragment-free document URI and escaped JSON Pointer",
        ));
    }
    Ok(())
}

fn child(source: &ProgramSource, token: &str) -> ProgramSource {
    ProgramSource {
        document: source.document.clone(),
        pointer: Pointer::parse(&source.pointer)
            .expect("checked source pointer")
            .push(token)
            .to_path(),
    }
}

fn target<'a>(
    program: &'a OwnedProgram,
    index: usize,
    at: &ProgramSource,
) -> Result<&'a ProgramNode, ProgramCheckError> {
    program.nodes.get(index).ok_or_else(|| {
        error(
            Some(at),
            "schema target is outside the finite compiled graph",
        )
    })
}

fn located_target(
    program: &OwnedProgram,
    index: usize,
    at: &ProgramSource,
    expected: &ProgramSource,
) -> Result<(), ProgramCheckError> {
    if target(program, index, at)?.source != *expected {
        return Err(error(
            Some(at),
            "applicator target does not match its source child identity",
        ));
    }
    Ok(())
}

fn distinct<'a>(
    values: impl Iterator<Item = &'a str>,
    at: &ProgramSource,
    label: &str,
) -> Result<BTreeSet<&'a str>, ProgramCheckError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(error(Some(at), format!("duplicate {label}")));
        }
    }
    Ok(seen)
}

fn number(token: &str, cap: usize, at: &ProgramSource) -> Result<ExactNumber, ProgramCheckError> {
    // Admission precedes the JSON grammar check and exact-number allocation.
    if token.len() > cap {
        return Err(error(
            Some(at),
            format!("exact numeric operand exceeds {cap} source bytes"),
        ));
    }
    // ExactNumber also accepts source YAML integer forms. Portable operands
    // must be one JSON number token, so use serde only for grammar admission;
    // all arithmetic and zero/integrality semantics stay in ExactNumber.
    if !matches!(token.as_bytes().first(), Some(b'-' | b'0'..=b'9'))
        || token.trim() != token
        || serde_json::from_str::<serde_json::Number>(token).is_err()
    {
        return Err(error(
            Some(at),
            "numeric operand must be one JSON number token",
        ));
    }
    ExactNumber::parse(token.as_bytes(), cap).map_err(|cause| error(Some(at), cause.to_string()))
}

pub(super) fn check(program: &OwnedProgram) -> Result<(), ProgramCheckError> {
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(error(
            None,
            "unsupported compiled validation version or profile",
        ));
    }
    resources::check(program)?;
    let mut identities = BTreeSet::new();
    for node in &program.nodes {
        source(&node.source)?;
        if !identities.insert((&node.source.document, &node.source.pointer)) {
            return Err(error(
                Some(&node.source),
                "duplicate schema source identity",
            ));
        }
    }
    for node in &program.nodes {
        check_node(program, node)?;
    }
    let mut roots = BTreeSet::new();
    for root in &program.roots {
        source(&root.source)?;
        if target(program, root.target, &root.source)?.source != root.source
            || !roots.insert((&root.source.document, &root.source.pointer))
        {
            return Err(error(
                Some(&root.source),
                "selected root identity is inconsistent or duplicated",
            ));
        }
    }
    Ok(())
}

fn check_node(program: &OwnedProgram, node: &ProgramNode) -> Result<(), ProgramCheckError> {
    let mut locations = BTreeSet::new();
    let mut properties = BTreeSet::new();
    let mut declared = None;
    let mut prefix_length = 0;
    let mut items = None;
    let mut patterns = 0;
    let mut additional_patterns = false;
    let mut tail = false;
    for check in &node.checks {
        let at = &check.source;
        source(at)?;
        if check.instruction.requires_v2() && program.version == OwnedProgram::V1_VERSION {
            return Err(error(
                Some(at),
                "v2 applicator instruction is not admitted in a v1 program",
            ));
        }
        if check.instruction.requires_v3() && program.version != OwnedProgram::V3_VERSION {
            return Err(error(
                Some(at),
                "v3 dynamic instruction is not admitted in a v1/v2 program",
            ));
        }
        let unevaluated = matches!(
            check.instruction,
            ProgramInstruction::UnevaluatedProperties { .. }
                | ProgramInstruction::UnevaluatedItems { .. }
        );
        if program.version != OwnedProgram::V1_VERSION && tail && !unevaluated {
            return Err(error(
                Some(at),
                "unevaluated instructions must follow all other checks in the schema",
            ));
        }
        tail |= unevaluated;
        let expected = keyword(&check.instruction)
            .map_or_else(|| node.source.clone(), |key| child(&node.source, key));
        // OAS 3.0 Boolean exclusive flags modify minimum/maximum. Keep the
        // numeric operand's actual source rather than inventing a modern
        // exclusiveMinimum/Maximum numeric declaration. Existing v1 locations
        // and semantics remain accepted exactly as before.
        let normalized_bound = match check.instruction {
            ProgramInstruction::Bound {
                maximum,
                exclusive: true,
                ..
            } => *at == child(&node.source, if maximum { "maximum" } else { "minimum" }),
            _ => false,
        };
        if (*at != expected && !normalized_bound) || !locations.insert((&at.document, &at.pointer))
        {
            return Err(error(
                Some(at),
                "instruction source must uniquely identify its compiled schema keyword",
            ));
        }
        match &check.instruction {
            ProgramInstruction::Always { .. } => {
                if node.checks.len() != 1 {
                    return Err(error(
                        Some(at),
                        "boolean schema must contain exactly one check",
                    ));
                }
            }
            ProgramInstruction::Type { types } => {
                if types.is_empty()
                    || types
                        .iter()
                        .enumerate()
                        .any(|(index, ty)| types[..index].contains(ty))
                {
                    return Err(error(Some(at), "type list must be nonempty and unique"));
                }
            }
            ProgramInstruction::Ref { target: index } => {
                target(program, *index, at)?;
            }
            ProgramInstruction::DynamicRef {
                target: index,
                initial_resource,
                anchor,
            } => {
                target(program, *index, at)?;
                let context = program
                    .resource_context
                    .as_ref()
                    .expect("checked v3 context");
                if context.node_scopes[*index].0 != *initial_resource {
                    return Err(error(
                        Some(at),
                        "dynamic initial resource does not match its target scope",
                    ));
                }
                if let Some(name) = anchor
                    && (!resources::anchor_name(name)
                        || !context.resources[*initial_resource]
                            .dynamic_anchors
                            .iter()
                            .any(|(candidate, _, target)| candidate == name && target == index))
                {
                    return Err(error(
                        Some(at),
                        "dynamic name must identify an indexed anchor at its initial target",
                    ));
                }
            }
            ProgramInstruction::Properties { properties: fields } => {
                properties = distinct(
                    fields.iter().map(|field| field.name.as_str()),
                    at,
                    "property name",
                )?;
                for field in fields {
                    located_target(program, field.target, at, &child(at, &field.name))?;
                }
            }
            ProgramInstruction::AdditionalProperties {
                declared: names,
                target: index,
            }
            | ProgramInstruction::AdditionalPropertiesWithPatterns {
                declared: names,
                target: index,
            } => {
                additional_patterns = matches!(
                    check.instruction,
                    ProgramInstruction::AdditionalPropertiesWithPatterns { .. }
                );
                declared = Some((
                    at,
                    distinct(
                        names.iter().map(String::as_str),
                        at,
                        "declared property name",
                    )?,
                ));
                located_target(program, *index, at, at)?;
            }
            ProgramInstruction::Required { names } => {
                distinct(
                    names.iter().map(String::as_str),
                    at,
                    "required property name",
                )?;
            }
            ProgramInstruction::Items {
                target: index,
                start,
            } => {
                items = Some((at, *start));
                located_target(program, *index, at, at)?;
            }
            ProgramInstruction::Not { target: index } => {
                located_target(program, *index, at, at)?;
            }
            ProgramInstruction::If {
                condition,
                then_target,
                else_target,
            } => {
                located_target(program, *condition, at, at)?;
                for (key, index) in [("then", then_target), ("else", else_target)] {
                    if let Some(index) = index {
                        located_target(program, *index, at, &child(&node.source, key))?;
                    }
                }
            }
            ProgramInstruction::DependentRequired { dependencies } => {
                distinct(
                    dependencies.iter().map(|(name, _)| name.as_str()),
                    at,
                    "dependency trigger",
                )?;
                for (name, names) in dependencies {
                    distinct(
                        names.iter().map(String::as_str),
                        &child(at, name),
                        "dependent required name",
                    )?;
                }
            }
            ProgramInstruction::DependentSchemas { dependencies } => {
                distinct(
                    dependencies.iter().map(|entry| entry.name.as_str()),
                    at,
                    "dependency trigger",
                )?;
                for entry in dependencies {
                    located_target(program, entry.target, at, &child(at, &entry.name))?;
                }
            }
            ProgramInstruction::Contains {
                target: index,
                minimum,
                maximum,
            } => {
                located_target(program, *index, at, at)?;
                for (key, token) in [("minContains", minimum), ("maxContains", maximum)] {
                    if let Some(token) = token {
                        let location = child(&node.source, key);
                        let number = number(token, program.limits.max_number_bytes, &location)?;
                        if !number.is_integral() || number.is_negative() {
                            return Err(error(
                                Some(&location),
                                "contains count must be a nonnegative mathematical integer",
                            ));
                        }
                    }
                }
            }
            ProgramInstruction::PatternProperties { patterns: entries } => {
                distinct(
                    entries.iter().map(|(name, _, _)| name.as_str()),
                    at,
                    "property pattern",
                )?;
                patterns = entries.len();
                for (name, pattern, index) in entries {
                    located_target(program, *index, at, &child(at, name))?;
                    pattern
                        .check()
                        .map_err(|cause| error(Some(&child(at, name)), cause.message))?;
                }
            }
            ProgramInstruction::PropertyNames { target: index }
            | ProgramInstruction::UnevaluatedProperties { target: index }
            | ProgramInstruction::UnevaluatedItems { target: index } => {
                located_target(program, *index, at, at)?;
            }
            ProgramInstruction::PrefixItems { targets }
            | ProgramInstruction::AllOf { targets }
            | ProgramInstruction::AnyOf { targets }
            | ProgramInstruction::OneOf { targets } => {
                if targets.is_empty() {
                    return Err(error(
                        Some(at),
                        "applicator requires a nonempty target list",
                    ));
                }
                if matches!(check.instruction, ProgramInstruction::PrefixItems { .. }) {
                    prefix_length = targets.len();
                }
                for (position, index) in targets.iter().enumerate() {
                    located_target(program, *index, at, &child(at, &position.to_string()))?;
                }
            }
            ProgramInstruction::Bound { value, .. } => {
                number(value, program.limits.max_number_bytes, at)?;
            }
            ProgramInstruction::MultipleOf { value } => {
                let value = number(value, program.limits.max_number_bytes, at)?;
                if !value.is_positive() {
                    return Err(error(
                        Some(at),
                        "compiled divisor must be strictly positive",
                    ));
                }
            }
            ProgramInstruction::Count { value, .. } => {
                let value = number(value, program.limits.max_number_bytes, at)?;
                if !value.is_integral() || value.is_negative() {
                    return Err(error(
                        Some(at),
                        "compiled count must be a nonnegative mathematical integer",
                    ));
                }
            }
            // These literal operands are already typed JSON values. Budget
            // failures depend on whether evaluation actually compares them.
            ProgramInstruction::Enum { .. }
            | ProgramInstruction::Const { .. }
            | ProgramInstruction::UniqueItems => {}
            ProgramInstruction::Pattern { program: pattern } => pattern
                .check()
                .map_err(|cause| error(Some(at), cause.message))?,
        }
    }
    if let Some((at, names)) = &declared
        && names != &properties
    {
        return Err(error(
            Some(at),
            "declared names do not match adjacent properties",
        ));
    }
    if let Some((at, _)) = &declared
        && additional_patterns != (patterns != 0)
    {
        return Err(error(
            Some(at),
            "additionalProperties must use the pattern-aware opcode exactly when adjacent patterns are nonempty",
        ));
    }
    if let Some((at, start)) = items
        && start != prefix_length
    {
        return Err(error(
            Some(at),
            "items start does not match adjacent prefixItems length",
        ));
    }
    Ok(())
}

fn keyword(instruction: &ProgramInstruction) -> Option<&'static str> {
    Some(match instruction {
        ProgramInstruction::Always { .. } => return None,
        ProgramInstruction::Type { .. } => "type",
        ProgramInstruction::Ref { .. } => "$ref",
        ProgramInstruction::DynamicRef { .. } => "$dynamicRef",
        ProgramInstruction::Properties { .. } => "properties",
        ProgramInstruction::AdditionalProperties { .. } => "additionalProperties",
        ProgramInstruction::Required { .. } => "required",
        ProgramInstruction::Items { .. } => "items",
        ProgramInstruction::PrefixItems { .. } => "prefixItems",
        ProgramInstruction::AllOf { .. } => "allOf",
        ProgramInstruction::AnyOf { .. } => "anyOf",
        ProgramInstruction::OneOf { .. } => "oneOf",
        ProgramInstruction::Not { .. } => "not",
        ProgramInstruction::Bound {
            maximum, exclusive, ..
        } => match (maximum, exclusive) {
            (true, true) => "exclusiveMaximum",
            (true, false) => "maximum",
            (false, true) => "exclusiveMinimum",
            (false, false) => "minimum",
        },
        ProgramInstruction::MultipleOf { .. } => "multipleOf",
        ProgramInstruction::Count {
            maximum, target, ..
        } => match (target, maximum) {
            (ProgramCountTarget::String, false) => "minLength",
            (ProgramCountTarget::String, true) => "maxLength",
            (ProgramCountTarget::Array, false) => "minItems",
            (ProgramCountTarget::Array, true) => "maxItems",
            (ProgramCountTarget::Object, false) => "minProperties",
            (ProgramCountTarget::Object, true) => "maxProperties",
        },
        ProgramInstruction::Enum { .. } => "enum",
        ProgramInstruction::Const { .. } => "const",
        ProgramInstruction::UniqueItems => "uniqueItems",
        ProgramInstruction::Pattern { .. } => "pattern",
        ProgramInstruction::If { .. } => "if",
        ProgramInstruction::DependentRequired { .. } => "dependentRequired",
        ProgramInstruction::DependentSchemas { .. } => "dependentSchemas",
        ProgramInstruction::Contains { .. } => "contains",
        ProgramInstruction::PatternProperties { .. } => "patternProperties",
        ProgramInstruction::AdditionalPropertiesWithPatterns { .. } => "additionalProperties",
        ProgramInstruction::PropertyNames { .. } => "propertyNames",
        ProgramInstruction::UnevaluatedProperties { .. } => "unevaluatedProperties",
        ProgramInstruction::UnevaluatedItems { .. } => "unevaluatedItems",
    })
}
