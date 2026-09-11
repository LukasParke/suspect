//! Native constructor examples from the shared protocol example plan.
use super::{
    emit::{header, quote},
    models::{Additional, Shape},
    *,
};
use crate::examples::{ExampleEntry, ExampleRole};
use serde_json::Value;
use std::fmt::Write;
use suspect_ir::contract::SchemaId;
use suspect_schema::{Config, OwnedCompiler, OwnedOutcome, OwnedSchema};

pub(super) struct NativeValue {
    codec: String,
    expression: String,
    json: String,
}
pub(super) struct NativeCall {
    operation: usize,
    invocation: String,
    response: Option<String>,
}
pub(super) struct Samples<'a> {
    values: Vec<NativeValue>,
    calls: Vec<NativeCall>,
    first: Option<usize>,
    marker: std::marker::PhantomData<&'a Plan>,
}

pub(super) fn plan(plan: &Plan) -> Result<Samples<'_>, Vec<HttpDiagnostic>> {
    let roots = plan
        .models
        .symbols()
        .iter()
        .map(|s| s.source.clone())
        .collect::<Vec<_>>();
    let compiler = OwnedCompiler::new(Config {
        max_depth: 128,
        ..Default::default()
    });
    let validator = if plan.program.version == suspect_schema::OwnedProgram::V3_VERSION {
        compiler.compile_v3(plan.contract.clone(), &roots)
    } else if plan.program.version == suspect_schema::OwnedProgram::V2_VERSION {
        compiler.compile_v2(plan.contract.clone(), &roots)
    } else {
        compiler.compile(plan.contract.clone(), &roots)
    }
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| HttpDiagnostic {
                source: e.source,
                at: e.span.unwrap_or(0..0),
                code: "kotlin-example-validation",
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let mut result = Samples {
        values: Vec::new(),
        calls: Vec::new(),
        first: None,
        marker: std::marker::PhantomData,
    };
    let mut budget = 100_000;
    for (i, op) in plan.operations.iter().enumerate() {
        let Some(examples) = plan
            .examples
            .operations()
            .iter()
            .find(|e| e.source == op.source)
        else {
            continue;
        };
        for entry in &examples.entries {
            if let Some(symbol) = plan.models.symbol(&entry.schema)
                && let Some(expression) = lower(
                    plan,
                    &validator,
                    &entry.schema,
                    &entry.value,
                    &mut budget,
                    0,
                )
            {
                result.values.push(NativeValue {
                    codec: symbol.codec_name.clone(),
                    expression,
                    json: entry.value.to_string(),
                });
            }
        }
        let mut args = Vec::new();
        let mut request_media = None;
        let mut complete = true;
        for p in op.parameters.iter().filter(|p| p.required) {
            if let Some(entry) = examples
                .entries
                .iter()
                .find(|e| e.schema == p.schema && matches!(e.role, ExampleRole::Parameter { .. }))
            {
                if let Some(value) = lower(
                    plan,
                    &validator,
                    &entry.schema,
                    &entry.value,
                    &mut budget,
                    0,
                ) {
                    args.push(format!("{} = {value}", p.name));
                } else {
                    complete = false;
                }
            } else {
                complete = false;
            }
        }
        if let Some(body) = &op.body
            && body.required
        {
            let mut chosen = None;
            for m in &body.media {
                if let Some(value) =
                    media_expression(plan, &validator, m, &examples.entries, None, &mut budget)
                {
                    chosen = Some(if let Some(choice) = &body.choice_type {
                        format!("{choice}.{}({value})", m.name)
                    } else {
                        value
                    });
                    if !matches!(
                        m.wire.media_type().range(),
                        wire::MediaRange::Concrete { .. }
                    ) {
                        request_media =
                            Some(concrete(m.wire.media_type(), "application/octet-stream"));
                    }
                    break;
                }
            }
            if let Some(value) = chosen {
                args.push(format!("body = {value}"));
            } else {
                complete = false;
            }
        }
        if !complete {
            continue;
        }
        let mut call_args = if args.is_empty() {
            Vec::new()
        } else {
            vec![call(&op.input_type, &args)]
        };
        if let Some(media) = request_media {
            call_args.push(format!(
                "requestOptions = RequestOptions(requestMedia = {})",
                quote(&media)
            ));
        }
        let invocation = call(&format!("client.{}", op.method_name), &call_args);
        let response = op.responses.iter().filter(|r| r.success).find_map(|r| {
            response_expression(plan, &validator, op, i, r, &examples.entries, &mut budget)
        });
        result.calls.push(NativeCall {
            operation: i,
            invocation,
            response,
        });
    }
    result.first = result
        .calls
        .iter()
        .position(|c| c.response.is_some() && plan.operations[c.operation].body.is_some())
        .or_else(|| result.calls.iter().position(|c| c.response.is_some()));
    Ok(result)
}

fn entry_for<'a>(
    entries: &'a [ExampleEntry],
    id: &SchemaId,
    status: Option<&str>,
) -> Option<&'a ExampleEntry> {
    entries.iter().find(|e| {
        e.schema == *id
            && match status {
                None => !e.role.is_response(),
                Some(status) => match &e.role {
                    ExampleRole::Response { status: n } => n.to_string() == status,
                    ExampleRole::ResponsePattern { status: s }
                    | ExampleRole::ResponseItem { status: s }
                    | ExampleRole::ResponsePart { status: s, .. }
                    | ExampleRole::ResponsePartHeader { status: s, .. }
                    | ExampleRole::ResponseHeader { status: s, .. } => s == status,
                    _ => false,
                },
            }
    })
}
fn media_expression(
    plan: &Plan,
    validator: &OwnedSchema,
    media: &PlannedMedia,
    entries: &[ExampleEntry],
    status: Option<&str>,
    budget: &mut usize,
) -> Option<String> {
    if let Some(form) = &media.form {
        return form_expression(plan, validator, form, entries, status, budget);
    }
    if let Some(id) = &media.schema {
        let entry = entry_for(entries, id, status)?;
        let minimal = if status.is_none() {
            minimal(plan, validator, id, &entry.value)
        } else {
            entry.value.clone()
        };
        let expression = lower(plan, validator, id, &minimal, budget, 0)?;
        return Some(if matches!(media.ty, NativeType::Flow(_)) {
            format!("kotlinx.coroutines.flow.flowOf({expression})")
        } else {
            expression
        });
    }
    Some(match media.ty {
        NativeType::Bytes => {
            let wire::Representation::Binary { bytes, .. } = media.wire.representation() else {
                return None;
            };
            if bytes.max_bytes() == 0 {
                "byteArrayOf()".into()
            } else {
                "byteArrayOf(0)".into()
            }
        }
        NativeType::Json => "JsonObject(emptyMap())".into(),
        NativeType::String => quote("example"),
        NativeType::Boolean => "false".into(),
        NativeType::Number => "JsonNumber.of(0L)".into(),
        NativeType::Unit => "Unit".into(),
        _ => return None,
    })
}
fn form_expression(
    plan: &Plan,
    validator: &OwnedSchema,
    form: &PlannedForm,
    entries: &[ExampleEntry],
    status: Option<&str>,
    budget: &mut usize,
) -> Option<String> {
    let minimum = form
        .rules
        .min_properties()
        .map_or(0, |v| *v.value() as usize);
    let mut args = Vec::new();
    for part in &form.fields {
        if !part.wire.required() && args.len() >= minimum {
            continue;
        }
        let mut value = if matches!(&part.value_type,NativeType::Named(n)if n=="Upload") {
            let wire::PartRepresentation::Binary { bytes } = part.wire.representation() else {
                return None;
            };
            format!(
                "Upload({}, filename = \"example.bin\")",
                if bytes.max_bytes() == 0 {
                    "byteArrayOf()"
                } else {
                    "byteArrayOf(0)"
                }
            )
        } else {
            let id = match &part.value_type {
                NativeType::Model(id) => id,
                _ => return None,
            };
            let entry = entry_for(entries, id, status)?;
            lower(plan, validator, id, &entry.value, budget, 0)?
        };
        if let Some(wrapper) = &part.wrapper {
            let mut parameters = vec![format!("value = {value}")];
            if let Some(name) = &part.headers_type {
                let mut headers = Vec::new();
                for h in part.headers.iter().filter(|h| h.wire.required()) {
                    let id = h.wire.codec().schema().id();
                    let entry = entry_for(entries, id, status)?;
                    headers.push(format!(
                        "{} = {}",
                        h.name,
                        lower(plan, validator, id, &entry.value, budget, 0)?
                    ));
                }
                parameters.push(format!("headers = {}", call(name, &headers)));
            }
            if part.wire.content_types().len() != 1
                || part
                    .wire
                    .content_types()
                    .first()
                    .is_some_and(|m| !matches!(m.range(), wire::MediaRange::Concrete { .. }))
            {
                let first = part.wire.content_types().first()?;
                parameters.push(format!(
                    "contentType = {}",
                    quote(&concrete(first, "application/octet-stream"))
                ));
            }
            value = call(wrapper, &parameters);
        }
        if part.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
            value = format!("listOf({value})");
        }
        args.push(format!(
            "{} = {}",
            part.name,
            if part.wire.required() {
                value
            } else {
                format!("Presence.Present({value})")
            }
        ));
    }
    if args.len() < minimum
        || form
            .rules
            .max_properties()
            .is_some_and(|v| args.len() as u64 > *v.value())
    {
        return None;
    }
    Some(call(&form.name, &args))
}
fn concrete(media: &wire::MediaType, fallback: &str) -> String {
    match media.range() {
        wire::MediaRange::Concrete { .. } => media.declared().into(),
        wire::MediaRange::Type { type_name } => format!(
            "{type_name}/{}",
            if type_name == "text" {
                "plain"
            } else {
                "octet-stream"
            }
        ),
        _ => fallback.into(),
    }
}
fn response_expression(
    plan: &Plan,
    validator: &OwnedSchema,
    op: &PlannedOperation,
    oi: usize,
    r: &PlannedResponse,
    entries: &[ExampleEntry],
    budget: &mut usize,
) -> Option<String> {
    let status = match r.status {
        wire::ResponseStatus::Exact(n) => n,
        wire::ResponseStatus::Range(n) => (n as u16) * 100,
        wire::ResponseStatus::Default => (200..300).find(|s| {
            op.wire.match_response(*s, None).is_err()
                || op
                    .wire
                    .responses()
                    .iter()
                    .all(|r| !matches!(r.status(),wire::ResponseStatus::Exact(n)if n==*s))
        })?,
    };
    let mut headers = Vec::new();
    for h in r.headers.iter().filter(|h| h.wire.required()) {
        let entry = entry_for(entries, h.wire.codec().schema().id(), Some(&r.status_key))?;
        headers.push(format!(
            "{} to listOf({})",
            quote(h.wire.name()),
            quote(h.wire.serialize(&entry.value).ok()?.value())
        ));
    }
    if r.disposition == wire::ResponseBodyDisposition::ForbiddenByHttp {
        return Some(format!(
            "HttpResponse({status}, mapOf({}), byteArrayOf())",
            headers.join(",")
        ));
    }
    let Some(media) = &r.media else {
        return Some(format!(
            "HttpResponse({status}, mapOf({}), byteArrayOf(0))",
            headers.join(",")
        ));
    };
    let actual = concrete(
        media.wire.media_type(),
        if matches!(media.ty, NativeType::Json | NativeType::Model(_)) {
            "application/json"
        } else {
            "application/octet-stream"
        },
    );
    let bytes = if r.stream {
        let id = media.schema.as_ref()?;
        let entry = entry_for(entries, id, Some(&r.status_key))?;
        let wire::Representation::Stream { stream } = media.wire.representation() else {
            return None;
        };
        let text = if stream.framing() == wire::StreamFraming::JsonLines {
            format!("{}\n", entry.value)
        } else {
            let value = entry.value.as_object()?;
            let mut text = String::new();
            for key in ["event", "id"] {
                if let Some(value) = value.get(key) {
                    let value = value.as_str()?;
                    if value.contains(['\n', '\r', '\0']) {
                        return None;
                    }
                    writeln!(text, "{key}: {value}").unwrap();
                }
            }
            if let Some(value) = value.get("retry") {
                writeln!(text, "retry: {}", value.as_u64()?).unwrap();
            }
            for line in value.get("data")?.as_str()?.split('\n') {
                writeln!(text, "data: {line}").unwrap();
            }
            text.push('\n');
            text
        };
        format!("{}.toByteArray(Charsets.UTF_8)", quote(&text))
    } else {
        let value = media_expression(plan, validator, media, entries, Some(&r.status_key), budget)?;
        let prepared = rich_emit::encode_type(plan, &media.ty, &value, "modelBudget", &r.source);
        let mi = r.media_index?;
        return Some(format!(
            "run {{ val modelBudget = ModelBudget(CodecLimits()) {{}}; val encoded = encodeBody((ProtocolData.operations[{oi}].array(\"responses\")[{}] as JsonObject).array(\"media\")[{mi}] as JsonObject, {prepared}, {}, ProtocolBudget(Json.MAX_BYTES) {{}}); HttpResponse({status}, mapOf({}\"Content-Type\" to listOf(encoded.second)), encoded.first) }}",
            r.response_index,
            quote(&actual),
            if headers.is_empty() {
                String::new()
            } else {
                headers.join(",") + ","
            }
        ));
    };
    headers.push(format!("\"Content-Type\" to listOf({})", quote(&actual)));
    Some(format!(
        "HttpResponse({status}, mapOf({}), {bytes})",
        headers.join(",")
    ))
}

pub(super) fn examples(plan: &Plan, samples: &Samples<'_>) -> String {
    let mut out = header(plan);
    out.push_str("import kotlinx.coroutines.runBlocking\nimport kotlinx.coroutines.flow.toList\n\n/** Executable native constructors and protocol calls from validated source values. */\npublic object GeneratedExamples {\n");
    for (i, v) in samples.values.iter().enumerate() {
        writeln!(out,"    private fun value{i}() {{\n        val value = {}\n        check(Json.parse(Codecs.{}.encode(value)) == Json.parse({}))\n    }}",v.expression.replace('\n',"\n        "),v.codec,quote(&v.json)).unwrap();
    }
    let mut count = 0;
    for (c, call) in samples.calls.iter().enumerate() {
        if let Some(response) = &call.response {
            let op = &plan.operations[call.operation];
            let credentials = fixture_credentials(plan);
            writeln!(out,"    private suspend fun call{c}() {{\n        Client({credentials}, Transport {{ {response} }}, ClientOptions(serverUrl = java.net.URI(\"http://127.0.0.1/api\"))).use {{ client ->\n            {}{}\n        }}\n    }}",if op.flow{"check("}else{""},if op.flow{format!("{}.toList().isNotEmpty())",if samples.first==Some(c){"Quickstart.firstRequest(client)".into()}else{call.invocation.clone()})}else if samples.first==Some(c){"Quickstart.firstRequest(client)".into()}else{call.invocation.clone()}).unwrap();
            count += 1;
        }
    }
    out.push_str("    /** Run every example through independently bounded JVM methods. */\n    @JvmStatic public fun main(args: Array<String>) = runBlocking {\n");
    for i in 0..samples.values.len() {
        writeln!(out, "        value{i}()").unwrap();
    }
    for (i, call) in samples.calls.iter().enumerate() {
        if call.response.is_some() {
            writeln!(out, "        call{i}()").unwrap();
        }
    }
    writeln!(out,"        println(\"Kotlin protocol constructor examples passed: {count} calls\")\n    }}\n}}").unwrap();
    out
}
fn fixture_credentials(plan: &Plan) -> String {
    format!(
        "Credentials({})",
        plan.credentials
            .values()
            .map(|c| format!(
                "{} = {}",
                c.name,
                match c.wire.credential() {
                    wire::CredentialHook::Basic =>
                        "BasicCredentials(\"example-user\", \"example-token\")",
                    wire::CredentialHook::OAuth2 { .. }
                    | wire::CredentialHook::OpenIdConnect { .. } =>
                        "CredentialProvider { \"Bearer example-token\" }",
                    _ => "\"example-token\"",
                }
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}
pub(super) fn quickstart(plan: &Plan, samples: &Samples<'_>) -> Option<String> {
    let sample = &samples.calls[samples.first?];
    let op = &plan.operations[sample.operation];
    if plan.credential_env().is_some() {
        let needs_document = op
            .wire
            .servers()
            .candidates()
            .iter()
            .any(|s| s.resolve_document_url(&Default::default()).is_err());
        let options = if needs_document {
            "options = ClientOptions(documentUrl = java.net.URI(requireNotNull(System.getenv(\"API_DOCUMENT_URL\"))))"
        } else {
            ""
        };
        return Some(format!(
            "import kotlinx.coroutines.runBlocking\nimport kotlinx.coroutines.flow.Flow\nimport kotlinx.coroutines.flow.collect\n\n/** First source request with explicitly configured runtime variable names. */\npublic object Quickstart {{\n    /** Snapshot credentials at creation, then perform the source request. */\n    @JvmStatic public fun main(args: Array<String>) = runBlocking {{\n        Client.fromEnv({options}).use {{ client ->\n            {}\n        }}\n    }}\n    /** Native source inputs; explicit clients retain their whole credential argument. */\n    public {}fun firstRequest(client: Client): {} = {}\n}}\n",
            if op.flow {
                "firstRequest(client).collect { println(it.response.status) }"
            } else {
                "println(firstRequest(client).response.status)"
            },
            if op.flow { "" } else { "suspend " },
            if op.flow {
                format!("Flow<{}>", op.result_type)
            } else {
                op.result_type.clone()
            },
            sample.invocation.replace('\n', "\n        ")
        ));
    }
    let anonymous = op.wire.security().alternatives().is_empty()
        || op
            .wire
            .security()
            .alternatives()
            .iter()
            .any(|a| a.is_anonymous());
    let creds = if anonymous {
        "Credentials()".into()
    } else {
        let args=op.wire.security().alternatives().first()?.requirements().iter().map(|c|{let native=&plan.credentials[plan.credential_source(c)];format!("{} = {}",native.name,match c.credential(){wire::CredentialHook::Basic=>"BasicCredentials(requireNotNull(System.getenv(\"API_USERNAME\")), token)",wire::CredentialHook::OAuth2{..}|wire::CredentialHook::OpenIdConnect{..}=>"CredentialProvider { \"Bearer $token\" }",_=>"token"})}).collect::<Vec<_>>();
        call("Credentials", &args)
    };
    let needs_document = op.wire.servers().candidates().iter().any(|s| {
        s.resolve_url(plan.contract.entry().as_str(), &Default::default())
            .is_err()
    });
    Some(format!(
        "import kotlinx.coroutines.runBlocking\nimport kotlinx.coroutines.flow.Flow\nimport kotlinx.coroutines.flow.collect\n\n/** Native first request; environment lookup is application scaffolding. */\npublic object Quickstart {{\n    /** Run the source-defined request. */\n    @JvmStatic public fun main(args: Array<String>) = runBlocking {{\n        {}\n        Client({creds}{}).use {{ client ->\n            try {{ {} }} catch (error: {}) {{ println(\"HTTP ${{error.response.status}}\"); throw error }}\n        }}\n    }}\n    /** Required fields use native constructors; optional source values remain absent. */\n    public {}fun firstRequest(client: Client): {} = {}\n}}\n",
        if anonymous {
            ""
        } else {
            "val token = requireNotNull(System.getenv(\"API_TOKEN\"))"
        },
        if needs_document {
            ", options = ClientOptions(documentUrl = java.net.URI(requireNotNull(System.getenv(\"API_DOCUMENT_URL\"))))"
        } else {
            ""
        },
        if op.flow {
            "firstRequest(client).collect { println(it.response.status) }"
        } else {
            "println(firstRequest(client).response.status)"
        },
        op.error_type,
        if op.flow { "" } else { "suspend " },
        if op.flow {
            format!("Flow<{}>", op.result_type)
        } else {
            op.result_type.clone()
        },
        sample.invocation.replace('\n', "\n        ")
    ))
}

pub(super) fn quickstart_manifest(plan: &Plan, samples: &Samples<'_>) -> Option<Value> {
    let call = &samples.calls[samples.first?];
    let op = &plan.operations[call.operation];
    let response = op.responses.iter().find(|r| {
        r.success && !r.stream && matches!(r.disposition, wire::ResponseBodyDisposition::Declared)
    })?;
    let wire::ResponseStatus::Exact(status) = response.status else {
        return None;
    };
    let id = response.schema.as_ref()?;
    let examples = plan
        .examples
        .operations()
        .iter()
        .find(|e| e.source == op.source)?;
    let entry = entry_for(&examples.entries, id, Some(&response.status_key))?;
    Some(
        serde_json::json!({"operationId":op.operation_id,"method":op.wire.method().as_str(),"response":{"status":status,"value":entry.value}}),
    )
}
fn minimal(plan: &Plan, validator: &OwnedSchema, id: &SchemaId, value: &Value) -> Value {
    let target = plan.models.target(id);
    if let Shape::Object { fields, .. } = &target.shape
        && let Some(object) = value.as_object()
    {
        let candidate = Value::Object(
            fields
                .iter()
                .filter(|f| f.required)
                .filter_map(|f| {
                    object
                        .get(&f.wire_name)
                        .map(|v| (f.wire_name.clone(), v.clone()))
                })
                .collect(),
        );
        if validator.validate(id, &candidate) == OwnedOutcome::Valid {
            return candidate;
        }
    }
    value.clone()
}
fn call(name: &str, args: &[String]) -> String {
    let text = args.join(", ");
    if text.len() < 100 && !text.contains('\n') {
        format!("{name}({text})")
    } else {
        format!(
            "{name}(\n    {}\n)",
            args.join(",\n").replace('\n', "\n    ")
        )
    }
}
fn lower(
    plan: &Plan,
    validator: &OwnedSchema,
    id: &SchemaId,
    value: &Value,
    remaining: &mut usize,
    depth: usize,
) -> Option<String> {
    *remaining = remaining.checked_sub(1)?;
    if depth >= 128 {
        return None;
    }
    let s = plan.models.get(id);
    if value.is_null() && (s.nullable || matches!(s.shape, Shape::Null)) {
        return Some("null".into());
    }
    let mut child = |id, value| lower(plan, validator, id, value, remaining, depth + 1);
    Some(match &s.shape {
        Shape::Any => json_value(value),
        Shape::CheckedJson => call(&s.name, &[format!("value = {}", json_value(value))]),
        Shape::Never | Shape::Null => return None,
        Shape::Boolean => value.as_bool()?.to_string(),
        Shape::String => quote(value.as_str()?),
        Shape::Number => number(value),
        Shape::Alias(id) => child(id, value)?,
        Shape::Array(item) => {
            let values = value.as_array()?;
            if values.is_empty() {
                format!(
                    "emptyList<{}>()",
                    item.as_ref().map_or("JsonValue", |id| plan
                        .models
                        .get(id)
                        .kotlin_type
                        .as_str())
                )
            } else {
                call(
                    "listOf",
                    &values
                        .iter()
                        .map(|v| {
                            item.as_ref()
                                .map_or_else(|| Some(json_value(v)), |id| child(id, v))
                        })
                        .collect::<Option<Vec<_>>>()?,
                )
            }
        }
        Shape::StringEnum(values) => {
            let (name, _) = values
                .iter()
                .find(|(_, wire)| Some(wire.as_str()) == value.as_str())?;
            format!("{}.{name}", s.name)
        }
        Shape::Object { fields, additional } => {
            let object = value.as_object()?;
            let mut args = Vec::new();
            for f in fields {
                if f.constant.is_some() {
                    continue;
                }
                if let Some(value) = object.get(&f.wire_name) {
                    let value = child(&f.schema, value)?;
                    args.push(format!(
                        "{} = {}",
                        f.name,
                        if f.required {
                            value
                        } else {
                            format!("Presence.Present({value})")
                        }
                    ));
                } else if f.required {
                    return None;
                }
            }
            let mut extra = Vec::new();
            for (key, value) in object {
                if fields.iter().any(|f| f.wire_name == *key) {
                    continue;
                }
                let value = match additional {
                    Additional::Closed => return None,
                    Additional::Any | Additional::Scoped => json_value(value),
                    Additional::Typed(id) => child(id, value)?,
                };
                extra.push(format!("{} to {value}", quote(key)));
            }
            if !extra.is_empty() {
                args.push(format!("additionalProperties = {}", call("mapOf", &extra)));
            }
            call(&s.name, &args)
        }
        Shape::Union { variants, .. } => {
            let mut selected = None;
            for (name, id) in variants {
                match validator.validate(id, value) {
                    OwnedOutcome::Valid if selected.is_none() => selected = Some((name, id)),
                    OwnedOutcome::EvaluationFailure(_) => return None,
                    _ => {}
                }
            }
            let (name, id) = selected?;
            call(&format!("{}.{name}", s.name), &[child(id, value)?])
        }
    })
}
fn number(value: &Value) -> String {
    let token = value.to_string();
    if let Ok(value) = token.parse::<i64>()
        && value != i64::MIN
        && token == value.to_string()
    {
        format!("JsonNumber.of({value}L)")
    } else {
        format!("JsonNumber.parse({})", quote(&token))
    }
}
fn json_value(value: &Value) -> String {
    match value {
        Value::Null => "JsonNull".into(),
        Value::Bool(v) => format!("JsonBoolean({v})"),
        Value::String(v) => format!("JsonString({})", quote(v)),
        Value::Number(_) => number(value),
        Value::Array(v) => format!(
            "JsonArray({})",
            if v.is_empty() {
                "emptyList()".into()
            } else {
                call("listOf", &v.iter().map(json_value).collect::<Vec<_>>())
            }
        ),
        Value::Object(v) => format!(
            "JsonObject({})",
            if v.is_empty() {
                "emptyMap()".into()
            } else {
                call(
                    "mapOf",
                    &v.iter()
                        .map(|(k, v)| format!("{} to {}", quote(k), json_value(v)))
                        .collect::<Vec<_>>(),
                )
            }
        ),
    }
}
