//! Compile selected Contract graph nodes without expanding reference cycles.

use iri_string::types::UriReferenceStr;
use suspect_ir::contract::{ContractSeverity, SchemaDialect};

use super::dialect::Dialect;
use super::*;
use crate::number::NumberError;

pub(super) fn compile(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: Config,
    profile: CompileProfile,
) -> Result<OwnedSchema, Vec<OwnedCompileError>> {
    let mut errors: Vec<_> = roots
        .iter()
        .filter(|id| contract.schema(id).is_none())
        .map(|id| {
            error(
                &contract,
                id,
                OwnedCompileErrorKind::UnknownRoot,
                "selected root is not an indexed Contract schema",
            )
        })
        .collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let closure = contract.effective_schema_closure(roots);
    let resource_profile = profile == CompileProfile::V3;
    let applicators = resource_profile
        || profile == CompileProfile::V2
            && closure.iter().any(|id| {
                contract.schema(id).is_some_and(|schema| {
                    !matches!(schema.dialect(), SchemaDialect::OpenApi30) && needs_v2(schema.raw())
                })
            });
    let indices: BTreeMap<_, _> = closure
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    for diagnostic in contract
        .diagnostics()
        .iter()
        .filter(|d| d.severity == ContractSeverity::Error)
    {
        // The Contract owns effective traversal and diagnostic applicability.
        // Dialect admission below reports its precise declaration location.
        if !contract.schema_diagnostic_applies(&closure, diagnostic)
            || diagnostic.code == "unsupported-schema-dialect"
        {
            continue;
        }
        errors.push(OwnedCompileError {
            source: diagnostic.source.clone(),
            span: Some(diagnostic.at.clone()),
            kind: if diagnostic.code.starts_with("invalid-") {
                OwnedCompileErrorKind::Invalid
            } else {
                OwnedCompileErrorKind::Unsupported
            },
            message: format!(
                "Contract diagnostic {}: {}",
                diagnostic.code, diagnostic.message
            ),
        });
    }
    let resource_context = if resource_profile {
        // Verify the declared dialect first: an unsupported vocabulary must not
        // be guessed from a missing/invalid resource scope.
        for id in &closure {
            if let Err(error) = dialect::admit(&contract, id, true) {
                errors.push(error);
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Some(resource::context(&contract, &closure, &indices).map_err(|error| vec![error])?)
    } else {
        None
    };
    let mut nodes = Vec::with_capacity(closure.len());
    for id in &closure {
        match dialect::admit(&contract, id, resource_profile).and_then(|dialect| {
            node(
                &contract,
                id,
                &indices,
                &config,
                dialect,
                applicators,
                resource_context.as_ref(),
            )
        }) {
            Ok(node) => nodes.push(node),
            Err(error) => errors.push(error),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let roots = roots
        .iter()
        .cloned()
        .map(|id| {
            let index = indices[&id];
            (id, index)
        })
        .collect();
    Ok(OwnedSchema {
        contract,
        nodes,
        roots,
        config,
        applicators,
        resources: resource_context,
    })
}

fn needs_v2(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("contains")
        || object.contains_key("propertyNames")
        || ["patternProperties", "dependentSchemas"].iter().any(|key| {
            object
                .get(*key)
                .is_some_and(|value| value.as_object().is_none_or(|value| !value.is_empty()))
        })
        || object.get("dependentRequired").is_some_and(|value| {
            value.as_object().is_none_or(|deps| {
                deps.iter().any(|(name, values)| {
                    values.as_array().is_none_or(|values| {
                        values.iter().any(|value| value.as_str() != Some(name))
                    })
                })
            })
        })
        || object.contains_key("if") && (object.contains_key("then") || object.contains_key("else"))
        || ["unevaluatedProperties", "unevaluatedItems"]
            .iter()
            .any(|key| {
                object
                    .get(*key)
                    .is_some_and(|value| value != &Value::Bool(true))
            })
}

fn contains(parent: &SourceId, child: &SourceId) -> bool {
    parent.document() == child.document()
        && (parent.pointer() == child.pointer()
            || child
                .pointer()
                .strip_prefix(parent.pointer())
                .is_some_and(|tail| tail.starts_with('/')))
}

pub(super) fn error(
    contract: &Contract,
    source: &SourceId,
    kind: OwnedCompileErrorKind,
    message: &str,
) -> OwnedCompileError {
    OwnedCompileError {
        source: source.clone(),
        span: contract.source_span(source),
        kind,
        message: message.into(),
    }
}

pub(super) fn invalid(contract: &Contract, source: &SourceId, message: &str) -> OwnedCompileError {
    error(contract, source, OwnedCompileErrorKind::Invalid, message)
}

fn node(
    contract: &Contract,
    id: &SchemaId,
    indices: &BTreeMap<SchemaId, usize>,
    config: &Config,
    dialect: Dialect,
    applicators: bool,
    resources: Option<&ProgramResourceContext>,
) -> Result<Node, OwnedCompileError> {
    let schema = contract.schema(id).expect("indexed schema");
    let oas30 = dialect == Dialect::OpenApi30;
    if let Some(value) = schema.raw().as_bool() {
        let additional = Pointer::parse(id.pointer()).ok().is_some_and(|pointer| {
            pointer
                .tokens()
                .last()
                .is_some_and(|token| token.as_ref() == "additionalProperties")
                && pointer.parent().is_some_and(|parent| {
                    contract
                        .schema(&SchemaId::new(id.document().clone(), parent))
                        .is_some()
                })
        });
        if oas30 && !additional {
            return Err(invalid(
                contract,
                id,
                "OpenAPI 3.0 requires a Schema Object; booleans are only allowed as literal additionalProperties values",
            ));
        }
        return Ok(Node {
            source: id.clone(),
            checks: vec![Check {
                source: id.clone(),
                kind: Kind::Always(value),
            }],
        });
    }
    let object = schema
        .raw()
        .as_object()
        .ok_or_else(|| invalid(contract, id, "schema must be an object or boolean"))?;
    let reference_only = oas30 && object.contains_key("$ref");
    if oas30 && !reference_only {
        admit_30(contract, id, object, config)?;
    } else if !oas30 {
        admit_modern(contract, id, object, config)?;
    }
    let child = |source: &SchemaId| {
        indices.get(source).copied().ok_or_else(|| {
            invalid(
                contract,
                source,
                "schema child is missing from the indexed closure",
            )
        })
    };
    let schema_array =
        |source: &SourceId, value: &Value| -> Result<Vec<usize>, OwnedCompileError> {
            let array = value.as_array().filter(|v| !v.is_empty()).ok_or_else(|| {
                invalid(
                    contract,
                    source,
                    "applicator requires a nonempty array of schemas",
                )
            })?;
            array
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    let at = source.child(&i.to_string());
                    schema_value(contract, &at, value, !oas30)?;
                    child(&at)
                })
                .collect()
        };
    let mut checks = Vec::new();
    for (keyword, value) in object {
        if reference_only && keyword != "$ref" {
            continue;
        }
        let source = id.child(keyword);
        let kind = match keyword.as_str() {
            "$ref" => {
                let raw = value
                    .as_str()
                    .ok_or_else(|| invalid(contract, &source, "$ref must be a string"))?;
                // The same strict parser underlies the source runtime's URI
                // helper. Reject repaired URL input before trusting its edge.
                UriReferenceStr::new(raw).map_err(|_| {
                    invalid(
                        contract,
                        &source,
                        "$ref must be a valid RFC 3986 URI-reference without raw whitespace",
                    )
                })?;
                let target = schema
                    .references()
                    .iter()
                    .find(|r| r.keyword == "$ref")
                    .and_then(|r| r.target.as_ref())
                    .ok_or_else(|| {
                        invalid(
                            contract,
                            &source,
                            "static reference has no resolved Contract target",
                        )
                    })?;
                if resources.is_some() {
                    let resolved = contract
                        .resolve_resource_reference(id, raw)
                        .map_err(|cause| invalid(contract, &source, &cause.to_string()))?;
                    if resolved != *target {
                        return Err(invalid(
                            contract,
                            &source,
                            "static reference edge disagrees with indexed resource resolution",
                        ));
                    }
                }
                if oas30 && let Some(target_schema) = contract.schema(target) {
                    if !target_schema.raw().is_object() {
                        return Err(invalid(
                            contract,
                            &source,
                            "an OpenAPI 3.0 Reference Object must target a Schema Object, not a boolean schema",
                        ));
                    }
                    if !matches!(target_schema.dialect(), SchemaDialect::OpenApi30) {
                        return Err(error(
                            contract,
                            &source,
                            OwnedCompileErrorKind::Unsupported,
                            "an OpenAPI 3.0 Reference Object requires an OAS 3.0 Schema Object target; cross-dialect target conversion is not implemented",
                        ));
                    }
                }
                Kind::Reference(child(target)?)
            }
            "type" => {
                let mut bits = types(contract, &source, value)?;
                if oas30 && object.get("nullable") == Some(&Value::Bool(true)) {
                    bits.0 |= TypeBits::NULL;
                }
                Kind::Type(bits)
            }
            "properties" => {
                let fields = value
                    .as_object()
                    .ok_or_else(|| invalid(contract, &source, "properties must be an object"))?;
                Kind::Properties(
                    fields
                        .iter()
                        .map(|(name, value)| {
                            let at = source.child(name);
                            schema_value(contract, &at, value, !oas30)?;
                            Ok((name.clone(), child(&at)?))
                        })
                        .collect::<Result<_, OwnedCompileError>>()?,
                )
            }
            "additionalProperties" => {
                schema_value(contract, &source, value, true)?;
                let declared = object
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|p| p.keys().cloned().collect())
                    .unwrap_or_default();
                if applicators
                    && object
                        .get("patternProperties")
                        .and_then(Value::as_object)
                        .is_some_and(|patterns| !patterns.is_empty())
                {
                    Kind::AdditionalPropertiesWithPatterns {
                        declared,
                        schema: child(&source)?,
                    }
                } else {
                    Kind::AdditionalProperties {
                        declared,
                        schema: child(&source)?,
                    }
                }
            }
            "required" => Kind::Required(strings(contract, &source, value)?),
            "items" => {
                schema_value(contract, &source, value, !oas30)?;
                Kind::Items {
                    schema: child(&source)?,
                    start: object
                        .get("prefixItems")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                }
            }
            "prefixItems" => Kind::PrefixItems(schema_array(&source, value)?),
            "allOf" => Kind::AllOf(schema_array(&source, value)?),
            "anyOf" => Kind::AnyOf(schema_array(&source, value)?),
            "oneOf" => Kind::OneOf(schema_array(&source, value)?),
            "not" => {
                schema_value(contract, &source, value, !oas30)?;
                Kind::Not(child(&source)?)
            }
            "exclusiveMinimum" | "exclusiveMaximum" if oas30 => {
                if !value.is_boolean() {
                    return Err(invalid(
                        contract,
                        &source,
                        "OpenAPI 3.0 exclusive bounds must be Boolean modifiers of minimum/maximum",
                    ));
                }
                // A flag without its corresponding bound has nothing to modify.
                // Never coerce false/true to numeric 0/1 or invent a missing bound.
                continue;
            }
            "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" => Kind::Bound {
                number: number(contract, &source, value, config)?,
                maximum: matches!(keyword.as_str(), "maximum" | "exclusiveMaximum"),
                exclusive: keyword.starts_with("exclusive")
                    || oas30
                        && object.get(if keyword == "minimum" {
                            "exclusiveMinimum"
                        } else {
                            "exclusiveMaximum"
                        }) == Some(&Value::Bool(true)),
            },
            "multipleOf" => {
                let value = number(contract, &source, value, config)?;
                if !value.is_positive() {
                    return Err(invalid(
                        contract,
                        &source,
                        "multipleOf must be strictly positive",
                    ));
                }
                Kind::MultipleOf(Divisor::new(value))
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" | "minProperties"
            | "maxProperties" => {
                let number = number(contract, &source, value, config)?;
                if !number.is_integral() || number.is_negative() {
                    return Err(invalid(
                        contract,
                        &source,
                        "cardinality must be a nonnegative mathematical integer",
                    ));
                }
                Kind::Count {
                    bound: number
                        .to_usize()
                        .map_or(CountBound::BeyondAddressable, CountBound::Finite),
                    token: number.to_string(),
                    maximum: keyword.starts_with("max"),
                    target: if keyword.ends_with("Length") {
                        CountTarget::String
                    } else if keyword.ends_with("Items") {
                        CountTarget::Array
                    } else {
                        CountTarget::Object
                    },
                }
            }
            "enum" => {
                if !value.is_array() {
                    return Err(invalid(contract, &source, "enum must be an array"));
                }
                Kind::Enum
            }
            "const" => Kind::Const,
            "uniqueItems" => match value.as_bool() {
                Some(true) => Kind::UniqueItems,
                Some(false) => continue,
                None => return Err(invalid(contract, &source, "uniqueItems must be a boolean")),
            },
            "pattern" => {
                let source_pattern = value
                    .as_str()
                    .ok_or_else(|| invalid(contract, &source, "pattern must be a string"))?;
                match crate::compile_pattern(source_pattern) {
                    Ok(program) => {
                        if oas30 {
                            dialect::pattern_30(contract, &source, source_pattern, &program)?;
                        }
                        Kind::Pattern(program)
                    }
                    Err(cause) => {
                        if oas30 {
                            return Err(error(
                                contract,
                                &source,
                                OwnedCompileErrorKind::Unsupported,
                                &format!(
                                    "OpenAPI 3.0 pattern is outside the shared ECMA-262 5.1-compatible profile; Unicode-pattern compiler reported: {}",
                                    cause.message
                                ),
                            ));
                        }
                        return Err(error(
                            contract,
                            &source,
                            match cause.kind {
                                crate::PatternErrorKind::Invalid => OwnedCompileErrorKind::Invalid,
                                crate::PatternErrorKind::Unsupported
                                | crate::PatternErrorKind::Limit => {
                                    OwnedCompileErrorKind::Unsupported
                                }
                            },
                            &cause.message,
                        ));
                    }
                }
            }
            "patternProperties" | "dependentSchemas"
                if value.as_object().is_some_and(serde_json::Map::is_empty) =>
            {
                continue;
            }
            "dependentRequired"
                if value.as_object().is_some_and(|dependencies| {
                    dependencies.iter().all(|(trigger, names)| {
                        names.as_array().is_some_and(|names| {
                            names.iter().all(|name| name.as_str() == Some(trigger))
                        })
                    })
                }) =>
            {
                continue;
            }
            "minContains" | "maxContains" if !object.contains_key("contains") => continue,
            "if" if !applicators
                && !object.contains_key("then")
                && !object.contains_key("else") =>
            {
                continue;
            }
            "then" | "else" if !object.contains_key("if") => continue,
            "unevaluatedProperties" | "unevaluatedItems"
                if !applicators && value == &Value::Bool(true) =>
            {
                continue;
            }
            "if" if applicators => Kind::If {
                condition: child(&source)?,
                then_target: object
                    .get("then")
                    .map(|_| child(&id.child("then")))
                    .transpose()?,
                else_target: object
                    .get("else")
                    .map(|_| child(&id.child("else")))
                    .transpose()?,
            },
            "then" | "else" | "minContains" | "maxContains" if applicators => continue,
            "dependentRequired" if applicators => Kind::DependentRequired(
                value
                    .as_object()
                    .expect("admitted dependencies")
                    .iter()
                    .map(|(name, value)| {
                        Ok((name.clone(), strings(contract, &source.child(name), value)?))
                    })
                    .collect::<Result<_, OwnedCompileError>>()?,
            ),
            "dependentSchemas" if applicators => Kind::DependentSchemas(
                value
                    .as_object()
                    .expect("admitted dependencies")
                    .keys()
                    .map(|name| Ok((name.clone(), child(&source.child(name))?)))
                    .collect::<Result<_, OwnedCompileError>>()?,
            ),
            "contains" if applicators => Kind::Contains {
                schema: child(&source)?,
                minimum: object
                    .get("minContains")
                    .map(|value| contains_limit(contract, &id.child("minContains"), value, config))
                    .transpose()?,
                maximum: object
                    .get("maxContains")
                    .map(|value| contains_limit(contract, &id.child("maxContains"), value, config))
                    .transpose()?,
            },
            "patternProperties" if applicators => Kind::PatternProperties(
                value
                    .as_object()
                    .expect("admitted patterns")
                    .keys()
                    .map(|pattern| {
                        let at = source.child(pattern);
                        let program = crate::compile_pattern(pattern).map_err(|cause| {
                            error(
                                contract,
                                &at,
                                if cause.kind == crate::PatternErrorKind::Invalid {
                                    OwnedCompileErrorKind::Invalid
                                } else {
                                    OwnedCompileErrorKind::Unsupported
                                },
                                &cause.message,
                            )
                        })?;
                        Ok((pattern.clone(), program, child(&at)?))
                    })
                    .collect::<Result<_, OwnedCompileError>>()?,
            ),
            "propertyNames" if applicators => Kind::PropertyNames(child(&source)?),
            "unevaluatedProperties" if applicators => Kind::UnevaluatedProperties(child(&source)?),
            "unevaluatedItems" if applicators => Kind::UnevaluatedItems(child(&source)?),
            "patternProperties" => {
                return Err(needed(
                    contract,
                    &source,
                    "patternProperties",
                    "pattern-to-property-value application plus matching-name exclusions in additionalProperties",
                ));
            }
            "propertyNames" => {
                return Err(needed(
                    contract,
                    &source,
                    "propertyNames",
                    "evaluation against property-name string instances; properties only visits property values",
                ));
            }
            "contains" | "minContains" | "maxContains" => {
                return Err(needed(
                    contract,
                    &source,
                    "contains",
                    "array-only subschema match counting with exact minContains/maxContains and noninvertible evaluation failures",
                ));
            }
            "if" | "then" | "else" => {
                return Err(needed(
                    contract,
                    &source,
                    "conditional",
                    "one condition trial and only the selected then/else branch; eager allOf/anyOf rewrites would evaluate skipped branches and change resource failures",
                ));
            }
            "dependentRequired" => {
                return Err(needed(
                    contract,
                    &source,
                    "dependentRequired",
                    "an object-only property-presence trigger and required names; absent and null-valued properties must remain distinct",
                ));
            }
            "dependentSchemas" => {
                return Err(needed(
                    contract,
                    &source,
                    "dependentSchemas",
                    "an object-only property-presence trigger applying a schema to the entire object, not to the property's value",
                ));
            }
            "unevaluatedProperties" | "unevaluatedItems" => {
                return Err(needed(
                    contract,
                    &source,
                    keyword,
                    "scoped successful-evaluation property/item sets across references and logical branches; local additionalProperties/items cannot substitute",
                ));
            }
            "$dynamicRef" if resources.is_some() => resource::dynamic(
                contract,
                id,
                indices,
                resources.expect("v3 resource context"),
            )?,
            "$dynamicRef" | "$recursiveRef" => {
                dialect::uri_reference(contract, &source, value)?;
                return Err(needed(
                    contract,
                    &source,
                    "dynamicRef",
                    "dialect-specific resource/dynamic-scope resolution; a static ref target cannot substitute",
                ));
            }
            "definitions" | "dependencies" | "additionalItems" => {
                return Err(error(
                    contract,
                    &source,
                    OwnedCompileErrorKind::Unsupported,
                    &format!(
                        "legacy `{keyword}` is outside this checked 2020-12 SDK profile; legacy dialect traversal/normalization must be explicit"
                    ),
                ));
            }
            "format" => {
                if !value.is_string() {
                    return Err(invalid(contract, &source, "`format` must be a string"));
                }
                if config.format_assertion {
                    return Err(error(
                        contract,
                        &source,
                        OwnedCompileErrorKind::Unsupported,
                        "owned format assertion is not implemented; annotation mode is supported",
                    ));
                }
                continue;
            }
            "$schema" | "$comment" | "title" | "description" | "contentEncoding"
            | "contentMediaType" => {
                if !value.is_string() {
                    return Err(invalid(
                        contract,
                        &source,
                        &format!("`{keyword}` must be a string"),
                    ));
                }
                continue;
            }
            "$anchor" => {
                dialect::anchor(contract, &source, value)?;
                continue;
            }
            "$defs" => {
                if !value.is_object() {
                    return Err(invalid(contract, &source, "$defs must be an object"));
                }
                continue;
            }
            "readOnly" | "writeOnly" | "deprecated" => {
                if !value.is_boolean() {
                    return Err(invalid(
                        contract,
                        &source,
                        &format!("`{keyword}` must be a boolean"),
                    ));
                }
                continue;
            }
            "examples" => {
                if !value.is_array() {
                    return Err(invalid(contract, &source, "examples must be an array"));
                }
                continue;
            }
            "discriminator" | "xml" | "externalDocs" if dialect != Dialect::JsonSchema202012 => {
                dialect::annotation(contract, &source, keyword, value, dialect)?;
                continue;
            }
            // Core/default/example/content and OAS documentation annotations
            // are retained in Contract. Unknown modern keywords are annotations;
            // known unsupported assertions are exhaustively listed above.
            _ => continue,
        };
        checks.push(Check { source, kind });
    }
    if applicators {
        checks.sort_by_key(|check| {
            matches!(
                check.kind,
                Kind::UnevaluatedProperties(_) | Kind::UnevaluatedItems(_)
            )
        });
    }
    Ok(Node {
        source: id.clone(),
        checks,
    })
}

fn contains_limit(
    contract: &Contract,
    source: &SourceId,
    value: &Value,
    config: &Config,
) -> Result<ContainsLimit, OwnedCompileError> {
    let value = number(contract, source, value, config)?;
    if !value.is_integral() || value.is_negative() {
        return Err(invalid(
            contract,
            source,
            "cardinality must be a nonnegative mathematical integer",
        ));
    }
    Ok(ContainsLimit {
        bound: value
            .to_usize()
            .map_or(CountBound::BeyondAddressable, CountBound::Finite),
        token: value.to_string(),
    })
}

fn needed(
    contract: &Contract,
    source: &SourceId,
    operation: &str,
    semantics: &str,
) -> OwnedCompileError {
    error(
        contract,
        source,
        OwnedCompileErrorKind::Unsupported,
        &format!(
            "the checked SDK program needs a `{operation}` operation for {semantics}; no existing instruction lowering preserves these semantics and source identities"
        ),
    )
}

fn schema_value(
    contract: &Contract,
    source: &SourceId,
    value: &Value,
    boolean: bool,
) -> Result<(), OwnedCompileError> {
    if value.is_object() || boolean && value.is_boolean() {
        Ok(())
    } else {
        Err(invalid(
            contract,
            source,
            if boolean {
                "schema must be an object or boolean"
            } else {
                "OpenAPI 3.0 requires a Schema Object or Reference Object here, not a boolean or array"
            },
        ))
    }
}

fn admit_30(
    contract: &Contract,
    id: &SchemaId,
    object: &serde_json::Map<String, Value>,
    config: &Config,
) -> Result<(), OwnedCompileError> {
    for keyword in object.keys() {
        if !dialect::keyword_in_30(keyword) {
            return Err(invalid(
                contract,
                &id.child(keyword),
                &format!(
                    "`{keyword}` is outside the OpenAPI 3.0 Schema Object vocabulary; only its fixed fields and x- extensions are admitted"
                ),
            ));
        }
    }
    if let Some(value) = object.get("nullable")
        && !value.is_boolean()
    {
        return Err(invalid(
            contract,
            &id.child("nullable"),
            "OpenAPI 3.0 nullable must be a boolean",
        ));
    }
    if let Some(value) = object.get("type") {
        let at = id.child("type");
        if !value.is_string() || value.as_str() == Some("null") {
            return Err(invalid(
                contract,
                &at,
                "OpenAPI 3.0 type must be one non-null type name; use same-object nullable to allow null",
            ));
        }
        let mut bits = types(contract, &at, value)?;
        if object.get("nullable") == Some(&Value::Bool(true)) {
            bits.0 |= TypeBits::NULL;
        }
        if value.as_str() == Some("array") && !object.contains_key("items") {
            return Err(invalid(
                contract,
                &at,
                "OpenAPI 3.0 type array requires an items Schema Object in the same object",
            ));
        }
        if let Some(default) = object.get("default") {
            use crate::equality::EqualityValue;
            let at = id.child("default");
            let integral = bits.0 & TypeBits::INT != 0
                && default.is_number()
                && number(contract, &at, default, config)?.is_integral();
            if !bits.matches(default.kind(), integral) {
                return Err(invalid(
                    contract,
                    &at,
                    "OpenAPI 3.0 default must conform to the same-object type (including nullable); defaults are never inserted",
                ));
            }
        }
    }
    if object
        .get("required")
        .is_some_and(|value| value.as_array().is_some_and(Vec::is_empty))
    {
        return Err(invalid(
            contract,
            &id.child("required"),
            "OpenAPI 3.0 required must be a nonempty array of unique strings",
        ));
    }
    if object.get("readOnly") == Some(&Value::Bool(true))
        && object.get("writeOnly") == Some(&Value::Bool(true))
    {
        return Err(invalid(
            contract,
            &id.child("writeOnly"),
            "OpenAPI 3.0 readOnly and writeOnly cannot both be true",
        ));
    }
    directional_required(contract, id)
}

/// A neutral program has no request/response parameter. Reject directional
/// requirements before they can silently become unconditional requirements.
/// In-place composition is followed conservatively; no instance is evaluated
/// and no synthetic schema or required source location is created.
fn directional_required(contract: &Contract, id: &SchemaId) -> Result<(), OwnedCompileError> {
    let nodes = in_place(contract, id);
    let mut directional = BTreeSet::new();
    for at in &nodes {
        if let Some(properties) = contract
            .source(at)
            .and_then(|v| v.get("properties"))
            .and_then(Value::as_object)
        {
            for name in properties.keys() {
                let property = at.child("properties").child(name);
                if in_place(contract, &property).iter().any(|at| {
                    contract.source(at).is_some_and(|value| {
                        value.get("readOnly") == Some(&Value::Bool(true))
                            || value.get("writeOnly") == Some(&Value::Bool(true))
                    })
                }) {
                    directional.insert(name.clone());
                }
            }
        }
    }
    if directional.is_empty() {
        return Ok(());
    }
    for at in nodes {
        if let Some(names) = contract
            .source(&at)
            .and_then(|v| v.get("required"))
            .and_then(Value::as_array)
            && names
                .iter()
                .filter_map(Value::as_str)
                .any(|name| directional.contains(name))
        {
            return Err(error(
                contract,
                &at.child("required"),
                OwnedCompileErrorKind::Unsupported,
                "OpenAPI 3.0 readOnly/writeOnly changes required by request/response direction; this neutral program needs an explicit directional validation view before lowering this requirement",
            ));
        }
    }
    Ok(())
}

fn in_place(contract: &Contract, root: &SchemaId) -> Vec<SchemaId> {
    let mut pending = vec![root.clone()];
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(schema) = contract.schema(&id) else {
            continue;
        };
        pending.extend(schema.references().iter().filter_map(|r| r.target.clone()));
        if dialect::reference_only(contract, &id) {
            continue;
        }
        for keyword in ["allOf", "anyOf", "oneOf", "not"] {
            let at = id.child(keyword);
            pending.extend(
                schema
                    .children()
                    .iter()
                    .filter(|child| contains(&at, child))
                    .cloned(),
            );
        }
        out.push(id);
    }
    out
}

fn admit_modern(
    contract: &Contract,
    id: &SchemaId,
    object: &serde_json::Map<String, Value>,
    config: &Config,
) -> Result<(), OwnedCompileError> {
    for key in ["minContains", "maxContains"] {
        if let Some(value) = object.get(key) {
            let at = id.child(key);
            let bound = number(contract, &at, value, config)?;
            if !bound.is_integral() || bound.is_negative() {
                return Err(invalid(
                    contract,
                    &at,
                    "cardinality must be a nonnegative mathematical integer",
                ));
            }
        }
    }
    for key in [
        "propertyNames",
        "contains",
        "if",
        "then",
        "else",
        "unevaluatedProperties",
        "unevaluatedItems",
        "contentSchema",
    ] {
        if let Some(value) = object.get(key) {
            schema_value(contract, &id.child(key), value, true)?;
        }
    }
    for key in ["$defs", "patternProperties", "dependentSchemas"] {
        if let Some(value) = object.get(key) {
            let at = id.child(key);
            let fields = value.as_object().ok_or_else(|| {
                invalid(
                    contract,
                    &at,
                    &format!("`{key}` must be an object of schemas"),
                )
            })?;
            for (name, value) in fields {
                schema_value(contract, &at.child(name), value, true)?;
            }
        }
    }
    if let Some(value) = object.get("dependentRequired") {
        let at = id.child("dependentRequired");
        let fields = value.as_object().ok_or_else(|| {
            invalid(
                contract,
                &at,
                "dependentRequired must be an object of unique string arrays",
            )
        })?;
        for (name, names) in fields {
            strings(contract, &at.child(name), names)?;
        }
    }
    Ok(())
}

fn types(
    contract: &Contract,
    source: &SourceId,
    value: &Value,
) -> Result<TypeBits, OwnedCompileError> {
    let names: Vec<&Value> = match value {
        Value::String(_) => vec![value],
        Value::Array(values) if !values.is_empty() => values.iter().collect(),
        _ => {
            return Err(invalid(
                contract,
                source,
                "type must be a string or nonempty array of unique type names",
            ));
        }
    };
    let mut bits = 0;
    for (index, value) in names.into_iter().enumerate() {
        let at = if value.is_string() && !contract.source(source).is_some_and(Value::is_array) {
            source.clone()
        } else {
            source.child(&index.to_string())
        };
        let bit = match value.as_str() {
            Some("null") => TypeBits::NULL,
            Some("boolean") => TypeBits::BOOL,
            Some("integer") => TypeBits::INT,
            Some("number") => TypeBits::NUM,
            Some("string") => TypeBits::STR,
            Some("array") => TypeBits::ARR,
            Some("object") => TypeBits::OBJ,
            _ => return Err(invalid(contract, &at, "unknown or non-string type name")),
        };
        if bits & bit != 0 {
            return Err(invalid(contract, &at, "type array entries must be unique"));
        }
        bits |= bit;
    }
    Ok(TypeBits(bits))
}

fn strings(
    contract: &Contract,
    source: &SourceId,
    value: &Value,
) -> Result<Vec<String>, OwnedCompileError> {
    let array = value.as_array().ok_or_else(|| {
        invalid(
            contract,
            source,
            "required must be an array of unique strings",
        )
    })?;
    let mut seen = BTreeSet::new();
    array
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let at = source.child(&index.to_string());
            let value = value
                .as_str()
                .ok_or_else(|| invalid(contract, &at, "required entries must be strings"))?;
            if !seen.insert(value) {
                return Err(invalid(contract, &at, "required entries must be unique"));
            }
            Ok(value.to_owned())
        })
        .collect()
}

fn number(
    contract: &Contract,
    source: &SourceId,
    value: &Value,
    config: &Config,
) -> Result<ExactNumber, OwnedCompileError> {
    let value = value
        .as_number()
        .ok_or_else(|| invalid(contract, source, "numeric keyword requires a finite number"))?;
    ExactNumber::parse(value.as_str().as_bytes(), config.max_number_bytes).map_err(|cause| {
        let kind = if matches!(cause, NumberError::ResourceLimit { .. }) {
            OwnedCompileErrorKind::ResourceLimit
        } else {
            OwnedCompileErrorKind::Invalid
        };
        error(contract, source, kind, &cause.to_string())
    })
}
