//! Example discovery over admitted protocol slots, with no second HTTP admission.

use super::*;
use crate::http_protocol::{
    self as wire, AdditionalParts, CodecRef, MediaPlan, MultipartPlan, PartPlan,
    PartRepresentation, ProtocolPlan, Representation,
};

/// Plan source-valid JSON/text model values for an already admitted protocol.
/// Supply the same Contract used to build the protocol. Binary values, framing
/// and schema-free JSON stay explicit native-example obligations.
#[must_use]
pub fn plan_protocol_examples(
    contract: Arc<Contract>,
    protocol: &ProtocolPlan,
    config: ExampleConfig,
) -> ExamplePlan {
    plan_with_compiler(contract, protocol, config, OwnedCompiler::compile)
}

/// Plan examples with the checked scoped-applicator v2 compiler. Native adapters
/// select this entrypoint after admitting that validation profile themselves.
/// Base schemas keep the same v1 values, origins and bounded synthesis behavior.
/// This does not enable native capabilities or admit resource/dynamic programs.
#[must_use]
pub fn plan_protocol_examples_v2(
    contract: Arc<Contract>,
    protocol: &ProtocolPlan,
    config: ExampleConfig,
) -> ExamplePlan {
    plan_with_compiler(contract, protocol, config, OwnedCompiler::compile_v2)
}

/// Plan examples with the explicit resource/dynamic v3 compiler. Native adapters
/// select this only after admitting that executable profile. Source declarations
/// retain their physical locations; dynamic fallback annotations are not guessed.
#[must_use]
pub fn plan_protocol_examples_v3(
    contract: Arc<Contract>,
    protocol: &ProtocolPlan,
    config: ExampleConfig,
) -> ExamplePlan {
    plan_with_compiler(contract, protocol, config, OwnedCompiler::compile_v3)
}

pub(super) type CompileExamples = fn(
    &OwnedCompiler,
    Arc<Contract>,
    &[SchemaId],
)
    -> Result<OwnedSchema, Vec<suspect_schema::OwnedCompileError>>;

fn plan_with_compiler(
    contract: Arc<Contract>,
    protocol: &ProtocolPlan,
    config: ExampleConfig,
    compile: CompileExamples,
) -> ExamplePlan {
    let mut plan = ExamplePlan {
        contract: contract.clone(),
        operations: Vec::new(),
        diagnostics: Vec::new(),
        format: "suspect-sdk-examples-v2",
    };
    if !protocol.is_admitted() {
        plan.diagnostics.extend(
            protocol
                .diagnostics()
                .iter()
                .map(|finding| ExampleDiagnostic {
                    source: finding.source().source().clone(),
                    at: finding.source().span(),
                    code: finding.code(),
                    message: finding.message().into(),
                }),
        );
        return plan;
    }
    if let Some(error) = configuration_error(&contract, &config) {
        plan.diagnostics.push(error);
        return plan;
    }
    let roots = crate::schema_view::closure(&contract, protocol.codec_roots());
    let compiler = OwnedCompiler::new(Config {
        format_assertion: false,
        max_evaluation_steps: config.max_validation_steps,
        ..Default::default()
    });
    let validator = match compile(&compiler, contract.clone(), &roots) {
        Ok(validator) => validator,
        Err(errors) => {
            plan.diagnostics
                .extend(errors.into_iter().map(|finding| ExampleDiagnostic {
                    source: finding.source,
                    at: finding.span.unwrap_or(0..0),
                    code: match finding.kind {
                        OwnedCompileErrorKind::ResourceLimit => "examples-schema-limit",
                        OwnedCompileErrorKind::Unsupported => "examples-schema-unsupported",
                        _ => "examples-schema-invalid",
                    },
                    message: finding.message,
                }));
            return plan;
        }
    };
    let mut state = State {
        contract: &contract,
        validator: &validator,
        config: &config,
        remaining: config.max_work,
    };
    for operation in protocol.operations() {
        // The use site can be a referenced Path Item mount. The terminal source
        // is the indexed Operation; mount provenance remains on the protocol.
        let source = operation.source().terminal().source();
        let Some(declared) = contract.operations().find(|op| op.source() == source) else {
            plan.diagnostics.push(diagnostic(
                &contract,
                source,
                "examples-operation-unavailable",
                "protocol operation is not present in this Contract",
            ));
            continue;
        };
        let mut slots = Vec::new();
        let mut entries = Vec::new();
        let mut validated_aggregates = Vec::new();
        for parameter in operation.parameters() {
            let location = match parameter.location() {
                wire::ParameterLocation::Path => ParameterLocation::Path,
                wire::ParameterLocation::Query => ParameterLocation::Query,
                wire::ParameterLocation::Header => ParameterLocation::Header,
                wire::ParameterLocation::Cookie => ParameterLocation::Cookie,
                wire::ParameterLocation::Querystring => ParameterLocation::Querystring,
            };
            let definition = parameter
                .content_media()
                .map(|media| media.source().terminal().source())
                .unwrap_or_else(|| parameter.source().terminal().source());
            slots.push(slot(
                &contract,
                ExampleRole::Parameter {
                    wire_name: parameter.name().into(),
                    location,
                },
                parameter.codec(),
                parameter.source().use_site().source(),
                definition,
                parameter
                    .content_media()
                    .map_or("", |media| media.media_type().declared()),
                false,
            ));
        }
        if let Some(body) = operation.body() {
            for media in body.media() {
                let mut media_inputs = Vec::new();
                media_slots(
                    &contract,
                    protocol,
                    media,
                    None,
                    &mut media_inputs,
                    &mut plan.diagnostics,
                );
                project_aggregate(
                    &mut state,
                    &compiler,
                    compile,
                    media,
                    None,
                    &mut media_inputs,
                    &mut entries,
                    &mut validated_aggregates,
                    &mut plan.diagnostics,
                );
                slots.extend(media_inputs);
            }
        }
        for response in operation.responses() {
            let forbidden = matches!(operation.method(), wire::Method::Head)
                || matches!(
                    response.status(),
                    wire::ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                        | wire::ResponseStatus::Range(1)
                );
            if !forbidden {
                for media in response.media() {
                    let mut media_inputs = Vec::new();
                    media_slots(
                        &contract,
                        protocol,
                        media,
                        Some(response.status_key()),
                        &mut media_inputs,
                        &mut plan.diagnostics,
                    );
                    project_aggregate(
                        &mut state,
                        &compiler,
                        compile,
                        media,
                        Some(response.status_key()),
                        &mut media_inputs,
                        &mut entries,
                        &mut validated_aggregates,
                        &mut plan.diagnostics,
                    );
                    slots.extend(media_inputs);
                }
            }
            for header in response.headers() {
                slots.push(header_slot(
                    &contract,
                    ExampleRole::ResponseHeader {
                        status: response.status_key().into(),
                        wire_name: header.name().into(),
                    },
                    header,
                ));
            }
        }
        for slot in slots {
            state.slot(slot, &mut entries, &mut plan.diagnostics);
        }
        plan.operations.push(OperationExamples {
            source: source.clone(),
            operation_id: declared.operation_id().unwrap_or("").into(),
            entries,
            validated_aggregates,
        });
    }
    plan
}

fn slot<'a>(
    contract: &'a Contract,
    role: ExampleRole,
    codec: &CodecRef,
    container: &SourceId,
    definition: &SourceId,
    media: &str,
    schema_only: bool,
) -> Slot<'a> {
    Slot {
        role,
        media: media.into(),
        schema: codec.schema().id().clone(),
        container: container.clone(),
        part_position: None,
        definition: definition.clone(),
        named: if schema_only {
            Vec::new()
        } else {
            contract.examples_at(definition)
        },
        schema_only,
    }
}

fn header_slot<'a>(
    contract: &'a Contract,
    role: ExampleRole,
    header: &wire::HeaderPlan,
) -> Slot<'a> {
    let definition = header.content_media().map_or_else(
        || header.source().terminal().source(),
        |media| media.source().terminal().source(),
    );
    let media = header
        .content_media()
        .map_or("", |media| media.media_type().declared());
    slot(
        contract,
        role,
        header.codec(),
        header.source().use_site().source(),
        definition,
        media,
        false,
    )
}

pub(super) fn response_role(status: Option<&str>) -> ExampleRole {
    match status {
        None => ExampleRole::RequestBody,
        Some(status) => match status.parse() {
            Ok(status) => ExampleRole::Response { status },
            Err(_) => ExampleRole::ResponsePattern {
                status: status.into(),
            },
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn project_aggregate(
    state: &mut State<'_>,
    compiler: &OwnedCompiler,
    compile: CompileExamples,
    media: &MediaPlan,
    status: Option<&str>,
    slots: &mut Vec<Slot<'_>>,
    entries: &mut Vec<ExampleEntry>,
    validated_aggregates: &mut Vec<ExampleEntry>,
    diagnostics: &mut Vec<ExampleDiagnostic>,
) {
    let Some(aggregate) =
        super::aggregate::plan(state, compiler, compile, media, status, diagnostics)
    else {
        return;
    };
    let representative = &aggregate.values[0];
    // Every projected slot comes from one complete valid aggregate. Optional
    // missing members remain absent rather than borrowing a different example.
    slots.retain(|slot| {
        if !matches!(
            slot.role,
            ExampleRole::RequestPart { .. } | ExampleRole::ResponsePart { .. }
        ) {
            return true;
        }
        let Some(projections) = aggregate
            .fields
            .get(&(slot.container.clone(), slot.part_position))
        else {
            return true;
        };
        for projection in projections {
            let value = super::aggregate::value_at(&representative.value, &projection.path)
                .expect("projected member belongs to the validated aggregate");
            let source = projection.path.iter().fold(
                representative
                    .declared_source
                    .clone()
                    .expect("declared aggregate"),
                |source, segment| source.child(segment),
            );
            let mut values = state.declared_values(
                slot,
                vec![Candidate {
                    value,
                    source,
                    name: representative.name.clone(),
                    summary: representative.summary.clone(),
                }],
                diagnostics,
            );
            for entry in &mut values {
                match &mut entry.role {
                    ExampleRole::RequestPart { name, .. }
                    | ExampleRole::ResponsePart { name, .. }
                        if projection.wire_name.is_some() =>
                    {
                        name.clone_from(&projection.wire_name);
                    }
                    _ => {}
                }
            }
            entries.extend(values);
        }
        false
    });
    validated_aggregates.extend(aggregate.values);
}

fn media_slots<'a>(
    contract: &'a Contract,
    protocol: &ProtocolPlan,
    media: &MediaPlan,
    status: Option<&str>,
    slots: &mut Vec<Slot<'a>>,
    diagnostics: &mut Vec<ExampleDiagnostic>,
) {
    let container = media.source().use_site().source();
    let definition = media.source().terminal().source();
    let media_name = media.media_type().declared();
    match media.representation() {
        Representation::Json { codec:Some(codec) } | Representation::Text { codec:Some(codec),.. } => {
            // HEAD/1xx/204/304 keep metadata but have no response-body codec root.
            if protocol.codec_roots().contains(codec.schema().id()) {
                slots.push(slot(contract,response_role(status),codec,container,definition,media_name,false));
            }
        }
        Representation::Json { codec:None } | Representation::Text { codec:None,.. } => diagnostics.push(diagnostic(contract,container,
            "examples-schema-free-payload","this representation has no source-bound model codec; native JSON/text examples require an explicit payload recipe")),
        Representation::Binary { .. } => diagnostics.push(diagnostic(contract,container,"examples-native-bytes-required",
            "binary payload examples require an explicit native byte fixture; no JSON null or filename is substituted for bytes")),
        Representation::Stream { stream } => {
            let role=status.map_or(ExampleRole::RequestItem,|status|ExampleRole::ResponseItem { status:status.into() });
            slots.push(slot(contract,role,stream.item_codec(),container,stream.item_codec().schema().id(),media_name,true));
        }
        Representation::Form { form } => {
            for part in form.fields() { part_slots(contract,part,status,None,slots,diagnostics); }
            additional_slots(contract,form.additional(),status,None,slots,diagnostics);
        }
        Representation::Multipart { multipart } => match multipart {
            MultipartPlan::Named { parts,additional,.. } => {
                for part in parts { part_slots(contract,part,status,None,slots,diagnostics); }
                additional_slots(contract,additional,status,None,slots,diagnostics);
            }
            MultipartPlan::Positional { prefix,items,.. } => {
                for (index,part) in prefix.iter().enumerate() { part_slots(contract,part,status,Some(ExamplePartPosition::Prefix(index)),slots,diagnostics); }
                additional_slots(contract,items,status,Some(ExamplePartPosition::Items),slots,diagnostics);
            }
        },
    }
}

fn additional_slots<'a>(
    contract: &'a Contract,
    parts: &AdditionalParts,
    status: Option<&str>,
    position: Option<ExamplePartPosition>,
    slots: &mut Vec<Slot<'a>>,
    diagnostics: &mut Vec<ExampleDiagnostic>,
) {
    if let AdditionalParts::Allowed(part) = parts {
        part_slots(contract, part, status, position, slots, diagnostics);
    }
}

fn part_slots<'a>(
    contract: &'a Contract,
    part: &PartPlan,
    status: Option<&str>,
    position: Option<ExamplePartPosition>,
    slots: &mut Vec<Slot<'a>>,
    diagnostics: &mut Vec<ExampleDiagnostic>,
) {
    let name = part.name().map(str::to_owned);
    let repeated = part.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems;
    let role = match status {
        Some(status) => ExampleRole::ResponsePart {
            status: status.into(),
            name: name.clone(),
            repeated,
        },
        None => ExampleRole::RequestPart {
            name: name.clone(),
            repeated,
        },
    };
    let media = part
        .content_types()
        .first()
        .map_or("", |media| media.declared());
    match part.representation() {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => {
            let mut input = slot(
                contract,
                role,
                codec,
                part.source().use_site().source(),
                codec.schema().id(),
                media,
                true,
            );
            input.part_position = position;
            slots.push(input);
        }
        PartRepresentation::Binary { .. } => diagnostics.push(diagnostic(
            contract,
            part.source().use_site().source(),
            "examples-native-bytes-required",
            "binary part examples require actual byte data in the native fixture",
        )),
    }
    for header in part.headers() {
        let role = match status {
            Some(status) => ExampleRole::ResponsePartHeader {
                status: status.into(),
                part: name.clone(),
                wire_name: header.name().into(),
            },
            None => ExampleRole::RequestPartHeader {
                part: name.clone(),
                wire_name: header.name().into(),
            },
        };
        let mut input = header_slot(contract, role, header);
        input.part_position = position;
        slots.push(input);
    }
}
