//! Source-declared form/multipart aggregates, separate from native part codecs.
use super::*;
use crate::http_protocol::{
    AdditionalParts, MediaPlan, MultipartPlan, PartMultiplicity, PartPlan, PartRepresentation,
    Representation,
};
use std::collections::BTreeMap;

type Fields = BTreeMap<(SourceId, Option<ExamplePartPosition>), Vec<Projection>>;

pub(super) struct Projection {
    pub path: Vec<String>,
    pub wire_name: Option<String>,
}

pub(super) struct Plan {
    pub values: Vec<ExampleEntry>,
    pub fields: Fields,
}

pub(super) fn plan(
    state: &mut State<'_>,
    compiler: &OwnedCompiler,
    compile: super::protocol::CompileExamples,
    media: &MediaPlan,
    status: Option<&str>,
    diagnostics: &mut Vec<ExampleDiagnostic>,
) -> Option<Plan> {
    let (schema, parts) = match media.representation() {
        Representation::Form { form } => (
            form.rules().schema().id(),
            parts(form.fields(), form.additional()),
        ),
        Representation::Multipart { multipart } => match multipart {
            MultipartPlan::Named {
                rules,
                parts: named,
                additional,
            } => (rules.schema().id(), parts(named, additional)),
            MultipartPlan::Positional {
                schema,
                prefix,
                items,
                ..
            } => (schema.id(), parts(prefix, items)),
        },
        _ => return None,
    };
    let definition = media.source().terminal().source();
    let slot = Slot {
        role: super::protocol::response_role(status),
        media: media.media_type().declared().into(),
        schema: schema.clone(),
        container: media.source().use_site().source().clone(),
        part_position: None,
        definition: definition.clone(),
        named: state.contract.examples_at(definition),
        schema_only: false,
    };
    let declared = declared_candidates(
        state.contract,
        &slot,
        state.config.max_declared,
        diagnostics,
    );
    if declared.is_empty() {
        return None;
    }
    if parts
        .iter()
        .any(|part| matches!(part.representation(), PartRepresentation::Binary { .. }))
    {
        for candidate in declared {
            diagnostics.push(diagnostic(state.contract, &candidate.source,
                "examples-declared-unavailable",
                "this declared aggregate includes a native byte representation; its wire value requires an explicit byte fixture and is not validated with JSON placeholders"));
        }
        return None;
    }
    if !state.supported(schema, diagnostics) {
        return None;
    }
    let roots = crate::schema_view::closure(state.contract, std::slice::from_ref(schema));
    if let Err(failure) = state.charge(roots.len()) {
        state.report(schema, failure, diagnostics);
        return None;
    }
    // The aggregate is an example-only validation root, never an invented native
    // JSON codec input. Its failure cannot suppress the admitted part examples.
    let validator = match compile(compiler, state.validator.contract().clone(), &roots) {
        Ok(validator) => validator,
        Err(errors) => {
            for error in errors {
                diagnostics.push(diagnostic(
                    state.contract,
                    &error.source,
                    "examples-aggregate-schema-unavailable",
                    error.message,
                ));
            }
            for candidate in declared {
                diagnostics.push(diagnostic(state.contract, &candidate.source,
                    "examples-declared-unavailable",
                    "the complete declared aggregate cannot be checked under this explicit example validation profile"));
            }
            return None;
        }
    };
    let mut check = State {
        contract: state.contract,
        validator: &validator,
        config: state.config,
        remaining: state.remaining,
    };
    let values = check.declared_values(&slot, declared, diagnostics);
    state.remaining = check.remaining;
    let first = values.first()?;
    let mut fields = BTreeMap::new();
    match media.representation() {
        Representation::Form { form } => {
            named_fields(&mut fields, form.fields(), form.additional(), &first.value);
        }
        Representation::Multipart { multipart } => match multipart {
            MultipartPlan::Named {
                parts, additional, ..
            } => {
                named_fields(&mut fields, parts, additional, &first.value);
            }
            MultipartPlan::Positional { prefix, items, .. } => {
                for (index, part) in prefix.iter().enumerate() {
                    field(
                        &mut fields,
                        part,
                        Some(ExamplePartPosition::Prefix(index)),
                        vec![index.to_string()],
                        None,
                        &first.value,
                    );
                }
                if let AdditionalParts::Allowed(part) = items {
                    fields
                        .entry((
                            part.source().use_site().source().clone(),
                            Some(ExamplePartPosition::Items),
                        ))
                        .or_default();
                    for index in prefix.len()..first.value.as_array().map_or(0, Vec::len) {
                        field(
                            &mut fields,
                            part,
                            Some(ExamplePartPosition::Items),
                            vec![index.to_string()],
                            None,
                            &first.value,
                        );
                    }
                }
            }
        },
        _ => unreachable!("aggregate representation selected above"),
    }
    Some(Plan { values, fields })
}

fn parts<'a>(named: &'a [PartPlan], additional: &'a AdditionalParts) -> Vec<&'a PartPlan> {
    named
        .iter()
        .chain(match additional {
            AdditionalParts::Allowed(part) => Some(part.as_ref()),
            AdditionalParts::Forbidden => None,
        })
        .collect()
}

fn named_fields(
    fields: &mut Fields,
    named: &[PartPlan],
    additional: &AdditionalParts,
    value: &Value,
) {
    for part in named {
        if let Some(name) = part.name() {
            field(
                fields,
                part,
                None,
                vec![name.into()],
                Some(name.into()),
                value,
            );
        }
    }
    if let AdditionalParts::Allowed(part) = additional {
        fields
            .entry((part.source().use_site().source().clone(), None))
            .or_default();
        for name in value
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys())
        {
            if !named.iter().any(|part| part.name() == Some(name.as_str())) {
                field(
                    fields,
                    part,
                    None,
                    vec![name.clone()],
                    Some(name.clone()),
                    value,
                );
            }
        }
    }
}

fn field(
    fields: &mut Fields,
    part: &PartPlan,
    position: Option<ExamplePartPosition>,
    path: Vec<String>,
    wire_name: Option<String>,
    value: &Value,
) {
    let field = fields
        .entry((part.source().use_site().source().clone(), position))
        .or_default();
    let Some(value) = value_at(value, &path) else {
        return;
    };
    if part.multiplicity() == PartMultiplicity::RepeatedArrayItems {
        if let Some(values) = value.as_array() {
            for index in 0..values.len() {
                let mut item = path.clone();
                item.push(index.to_string());
                field.push(Projection {
                    path: item,
                    wire_name: wire_name.clone(),
                });
            }
        }
    } else {
        field.push(Projection { path, wire_name });
    }
}

pub(super) fn value_at<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, segment| match value {
        Value::Object(object) => object.get(segment),
        Value::Array(array) => segment
            .parse::<usize>()
            .ok()
            .and_then(|index| array.get(index)),
        _ => None,
    })
}
