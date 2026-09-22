//! Native constructor examples from the same descriptors and validated values.
use super::super::models::{Extras, Shape};
use super::super::{PlannedOperation, PlannedPart, ValueKind, ValueType};
use super::{SdkPlan, string};
use crate::examples::{ExampleEntry, ExampleRole};
use crate::http_protocol as w;
use std::fmt::Write;
use suspect_schema::{OwnedCompiler, OwnedOutcome, OwnedSchema};

pub(super) struct NativeExample {
    pub source: String,
    pub callsite: String,
    pub credential_name: Option<String>,
    pub security_scheme: Option<String>,
}

pub(super) fn example(plan: &SdkPlan) -> Option<NativeExample> {
    let roots = plan
        .models
        .symbols()
        .map(|s| s.source.clone())
        .collect::<Vec<_>>();
    let compiler = OwnedCompiler::new(plan.config.validation.clone());
    let validator = if plan.program.version == suspect_schema::OwnedProgram::V3_VERSION {
        compiler.compile_v3(plan.contract.clone(), &roots)
    } else if plan.program.version == suspect_schema::OwnedProgram::V2_VERSION {
        compiler.compile_v2(plan.contract.clone(), &roots)
    } else {
        compiler.compile(plan.contract.clone(), &roots)
    }
    .ok()?;
    let mut operations = plan.operations.iter().collect::<Vec<_>>();
    operations.sort_by_key(|op| op.body.is_none());
    for op in operations {
        let credential = if let Some(alternative) = op.wire.security().alternatives().first() {
            if alternative.is_anonymous() {
                None
            } else if alternative.requirements().len() == 1
                && matches!(
                    alternative.requirements()[0].credential(),
                    w::CredentialHook::Bearer { .. }
                )
            {
                Some(plan.credential_for(&alternative.requirements()[0]))
            } else {
                continue;
            }
        } else {
            None
        };
        let Some(examples) = plan
            .examples
            .operations()
            .iter()
            .find(|e| &e.source == op.wire.source().terminal().source())
        else {
            continue;
        };
        let parameter_example = |schema: &suspect_ir::contract::SchemaId| {
            examples
                .entries
                .iter()
                .find(|e| &e.schema == schema && matches!(e.role, ExampleRole::Parameter { .. }))
        };
        if op.parameters.iter().any(|p| {
            p.required
                && p.value
                    .schema()
                    .is_none_or(|schema| parameter_example(schema).is_none())
        }) {
            continue;
        }
        let Some(response) = response_fixture(op, &examples.entries) else {
            continue;
        };
        let mut out = format!(
            "/** @file client.cpp Native constructors from validated source examples.\n * Byte payloads, where present, are explicit demonstration Bytes.\n * Run without arguments for the explicitly configured fixture transport. */\n#include \"{}/sdk.hpp\"\n#include <iostream>\nusing namespace {};\nclass FixtureTransport final : public Transport {{\npublic:\n    Result<HttpResponse, TransportError> send(const HttpRequest& request, const TransportOptions& options) const override {{\n        if (request.method != {} || options.stop.stop_requested()) {{\n            TransportError error; error.message = \"fixture request mismatch\";\n            return Result<HttpResponse, TransportError>::failure(std::move(error));\n        }}\n        return Result<HttpResponse, TransportError>::success({response});\n    }}\n}};\n",
            plan.config.name,
            plan.config.namespace,
            string(op.wire.method().as_str()),
        );
        let mut callsite = String::from("int first_request(Client& client) {\n");
        let mut args = Vec::new();
        let mut usable = true;
        for arg in &op.constructor.parameters {
            let expression = if arg.member_name == "body" {
                let Some(body) = &op.body else {
                    usable = false;
                    break;
                };
                let mut selected = None;
                for media in &body.media {
                    if media.requires_content_type {
                        continue;
                    }
                    if let Some(value) = protocol_value(
                        plan,
                        &validator,
                        &media.value,
                        &examples.entries,
                        media.wire.media_type().declared(),
                        media.wire.representation(),
                    ) {
                        selected = Some(if body.choice_type.is_some() {
                            format!("{}({value})", media.wrapper_type)
                        } else {
                            value
                        });
                        break;
                    }
                }
                let Some(value) = selected else {
                    usable = false;
                    break;
                };
                value
            } else {
                let Some(source) = arg.value.schema() else {
                    usable = false;
                    break;
                };
                let Some(entry) = parameter_example(source) else {
                    usable = false;
                    break;
                };
                let Some(value) = native_value(plan, &validator, source, &entry.value, 0) else {
                    usable = false;
                    break;
                };
                value
            };
            writeln!(
                callsite,
                "    // Source-validated values; exact origins are in docs/coverage.json."
            )
            .unwrap();
            if arg.member_name == "body" {
                let expression = if arg.value.cpp_type == "std::string" {
                    format!("std::string({expression})")
                } else {
                    expression
                };
                writeln!(callsite, "    auto body = {expression};").unwrap();
                args.push("std::move(body)".into());
            } else {
                args.push(expression);
            }
        }
        if !usable {
            continue;
        }
        let input = if args.is_empty() {
            String::new()
        } else {
            format!("{}({})", op.input_type, args.join(", "))
        };
        writeln!(callsite,"    auto result = client.{}({input});\n    if (!result) {{\n        std::visit([](const auto& error) {{\n            if constexpr (std::is_same_v<std::decay_t<decltype(error)>, SdkError>)\n                std::cerr << error.message << '\\n';\n            else std::cerr << \"API returned HTTP \" << error.response.status << '\\n';\n        }}, result.error());\n        return 1;\n    }}\n    std::visit([](const auto& response) {{ std::cout << response.response.status << '\\n'; }}, result.value());\n    return 0;\n}}",op.method_name).unwrap();
        out.push_str(&callsite);
        writeln!(out,"\nint main(int argc, char** argv) {{\n    (void)argv; Credentials credentials;\n    {}\n    if (argc > 1) {{\n#if defined({}_HAS_CURL)\n        auto connected = Client::with_curl(std::move(credentials));\n        if (!connected) {{ std::cerr << connected.error().message; return 2; }}\n        return first_request(connected.value());\n#else\n        std::cerr << \"Build with SUSPECT_SDK_WITH_CURL for network execution\"; return 2;\n#endif\n    }}\n    ClientOptions fixture; fixture.server_url = \"https://fixture.invalid\";\n    Client client(std::make_shared<FixtureTransport>(), std::move(credentials), fixture);\n    return first_request(client);\n}}",credential.map(|c|format!("credentials.{} = argc > 1 ? argv[1] : \"example-fixture-token\";",c.field_name)).unwrap_or_default(),plan.config.name).unwrap();
        return Some(NativeExample {
            source: out,
            callsite,
            credential_name: credential.map(|c| c.field_name.clone()),
            security_scheme: credential.map(|c| c.wire.name().to_owned()),
        });
    }
    None
}

// Use v2 roles to select real codec samples. In particular a RequestPart sample
// cannot accidentally become a whole-body JSON example or a response fixture.
fn protocol_value(
    plan: &SdkPlan,
    validator: &OwnedSchema,
    value: &ValueType,
    entries: &[ExampleEntry],
    media: &str,
    representation: &w::Representation,
) -> Option<String> {
    match &value.kind {
        ValueKind::Model(schema) => entries
            .iter()
            .filter(|e| {
                e.role == ExampleRole::RequestBody && &e.schema == schema && e.media_type == media
            })
            .find_map(|entry| native_value(plan, validator, schema, &entry.value, 0)),
        ValueKind::Json => Some("JsonValue(JsonValue::Object{})".into()),
        ValueKind::Scalar(kind) => Some(
            match kind {
                w::ScalarType::String => "std::string(\"example\")",
                w::ScalarType::Boolean => "true",
                w::ScalarType::Integer => "JsonInteger(0)",
                w::ScalarType::Number => "JsonNumber(0)",
            }
            .into(),
        ),
        ValueKind::Bytes => {
            let w::Representation::Binary { bytes, .. } = representation else {
                return None;
            };
            Some(byte_recipe(
                bytes.max_bytes().min(plan.config.max_request_bytes as u64),
            ))
        }
        ValueKind::Aggregate(name) => {
            let aggregate = plan.aggregates.iter().find(|a| &a.type_name == name)?;
            let mut selected = std::collections::BTreeMap::new();
            for part in aggregate.fields.iter().filter(|p| p.wire.required()) {
                selected.insert(
                    part.field_name.clone(),
                    part_recipe(plan, validator, part, entries)?,
                );
            }
            let minimum = aggregate.rules.min_properties().map_or(0, |v| *v.value()) as usize;
            for part in aggregate.fields.iter().filter(|p| !p.wire.required()) {
                if selected.len() >= minimum {
                    break;
                }
                if let Some(value) = part_recipe(plan, validator, part, entries) {
                    selected.insert(part.field_name.clone(), value);
                }
            }
            if selected.len() < minimum
                || aggregate
                    .rules
                    .max_properties()
                    .is_some_and(|v| selected.len() as u64 > *v.value())
                || aggregate.rules.required().iter().any(|key| {
                    !aggregate
                        .fields
                        .iter()
                        .any(|p| p.name.as_ref() == Some(key.value()))
                })
            {
                return None;
            }
            let args = aggregate
                .fields
                .iter()
                .filter(|p| p.wire.required())
                .map(|p| selected[&p.field_name].clone())
                .collect::<Vec<_>>();
            let assignments = aggregate
                .fields
                .iter()
                .filter(|p| !p.wire.required())
                .filter_map(|p| {
                    selected
                        .get(&p.field_name)
                        .map(|v| format!("value.{} = {v};", p.field_name))
                })
                .collect::<Vec<_>>();
            let construct = format!("{name}({})", args.join(", "));
            Some(if assignments.is_empty() {
                construct
            } else {
                format!(
                    "[] {{ auto value = {construct}; {} return value; }}()",
                    assignments.join(" ")
                )
            })
        }
        ValueKind::Stream {
            schema,
            framing,
            max_item_bytes,
        } => {
            let entry = entries.iter().find(|e| {
                e.role == ExampleRole::RequestItem && &e.schema == schema && e.media_type == media
            })?;
            if frame_bytes(&entry.value, *framing)?.len() > *max_item_bytes {
                return None;
            }
            Some(format!(
                "std::vector<{}>{{{}}}",
                plan.models.get(schema).cpp_type,
                native_value(plan, validator, schema, &entry.value, 0)?
            ))
        }
        ValueKind::Unit | ValueKind::Choice(_) => None,
    }
}

fn byte_recipe(limit: u64) -> String {
    match limit {
        0 => "Bytes{}",
        1 => "Bytes{0x00}",
        _ => "Bytes{0x00, 0xff}",
    }
    .into()
}

fn part_recipe(
    plan: &SdkPlan,
    validator: &OwnedSchema,
    part: &PlannedPart,
    entries: &[ExampleEntry],
) -> Option<String> {
    let data = match &part.value.kind {
        ValueKind::Bytes => {
            let w::PartRepresentation::Binary { bytes } = part.wire.representation() else {
                return None;
            };
            byte_recipe(bytes.max_bytes().min(plan.config.max_part_bytes as u64))
        }
        ValueKind::Model(schema) => {
            let entry = entries.iter().find(|e| &e.schema == schema && matches!(&e.role, ExampleRole::RequestPart { name, .. } if name == &part.name))?;
            if matches!(
                part.wire.representation(),
                w::PartRepresentation::Style { .. }
            ) && (entry.value.as_array().is_some_and(Vec::is_empty)
                || entry
                    .value
                    .as_object()
                    .is_some_and(serde_json::Map::is_empty))
            {
                return None;
            }
            native_value(plan, validator, schema, &entry.value, 0)?
        }
        _ => return None,
    };
    let mut item = data;
    if let Some(name) = &part.part_type {
        let mut args = vec![item];
        let header_values = part.headers.iter().filter(|h| h.wire.required()).map(|h| {
            let schema = h.value.schema()?;
            let entry = entries.iter().find(|e| &e.schema == schema && matches!(&e.role, ExampleRole::RequestPartHeader { part: p, wire_name } if p == &part.name && wire_name == h.wire.name()))?;
            native_value(plan, validator, schema, &entry.value, 0)
        }).collect::<Option<Vec<_>>>()?;
        if !header_values.is_empty() {
            args.push(format!(
                "{}({})",
                part.headers_type.as_ref()?,
                header_values.join(", ")
            ));
        }
        item = format!("{name}({})", args.join(", "));
        if part.wire.content_types().len() > 1 {
            let media = part
                .wire
                .content_types()
                .iter()
                .find(|m| matches!(m.range(), w::MediaRange::Concrete { .. }))?;
            item = format!(
                "[] {{ auto value = {item}; value.content_type = {}; return value; }}()",
                text(media.declared())
            );
        }
    }
    if part.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
        let count = part.wire.min_items().map_or(1, |n| *n.value()).max(1);
        if count > 16 || part.wire.max_items().is_some_and(|n| count > *n.value()) {
            return None;
        }
        let ty = part.part_type.as_deref().unwrap_or(&part.value.cpp_type);
        item = format!(
            "std::vector<{ty}>{{{}}}",
            vec![item; count as usize].join(", ")
        );
    }
    Some(item)
}

fn response_fixture(operation: &PlannedOperation, entries: &[ExampleEntry]) -> Option<String> {
    for status in 200..300 {
        // Pick a concrete fixture status without attributing it to a range/default
        // source declaration. The same shared precedence resolves the sample.
        let response = operation
            .responses
            .iter()
            .filter(|r| match r.status {
                w::ResponseStatus::Exact(s) => s == status,
                w::ResponseStatus::Range(c) => u16::from(c) == status / 100,
                w::ResponseStatus::Default => true,
            })
            .max_by_key(|r| match r.status {
                w::ResponseStatus::Exact(_) => 3,
                w::ResponseStatus::Range(_) => 2,
                w::ResponseStatus::Default => 1,
            });
        let Some(response) = response else {
            continue;
        };
        let headers = response.headers.iter().filter(|h| h.wire.required()).map(|h| {
            let entry = entries.iter().find(|e| e.role.is_response() && matches!(&e.role, ExampleRole::ResponseHeader { status, wire_name } if status == &response.status_key && wire_name == h.wire.name()))?;
            Some(format!("{{{}, {}}}", string(h.wire.name()), string(h.wire.serialize(&entry.value).ok()?.value())))
        }).collect::<Option<Vec<_>>>();
        let Some(mut headers) = headers else {
            continue;
        };
        let forbidden = operation.wire.method() == w::Method::Head || matches!(status, 204 | 205);
        if forbidden || response.wire.media().is_empty() {
            return Some(format!(
                "HttpResponse{{{status}, {{{}}}, {{}}}}",
                headers.join(", ")
            ));
        }
        for case in &response.cases {
            let Some(media) = &case.media else {
                continue;
            };
            if !matches!(media.media_type().range(), w::MediaRange::Concrete { .. }) {
                continue;
            }
            let sample = entries.iter().find(|e| {
                e.role.is_response()
                    && e.media_type == media.media_type().declared()
                    && match &e.role {
                        ExampleRole::Response { status: s } => s.to_string() == response.status_key,
                        ExampleRole::ResponsePattern { status: s }
                        | ExampleRole::ResponseItem { status: s } => s == &response.status_key,
                        _ => false,
                    }
            });
            let bytes = match media.representation() {
                w::Representation::Json { codec } => {
                    if codec.is_none() {
                        Some("{}".into())
                    } else {
                        sample.map(|e| e.value.to_string())
                    }
                }
                w::Representation::Text { codec, .. } => {
                    if codec.is_none() {
                        Some("example".into())
                    } else {
                        sample.map(|e| {
                            e.value
                                .as_str()
                                .map_or_else(|| e.value.to_string(), str::to_owned)
                        })
                    }
                }
                w::Representation::Stream { stream } => {
                    sample.and_then(|e| frame_bytes(&e.value, stream.framing()))
                }
                w::Representation::Binary { .. } => Some(String::new()),
                _ => None,
            };
            if let Some(bytes) = bytes {
                headers.push(format!(
                    "{{\"Content-Type\", {}}}",
                    string(media.media_type().declared())
                ));
                return Some(format!(
                    "HttpResponse{{{status}, {{{}}}, {}}}",
                    headers.join(", "),
                    string(&bytes)
                ));
            }
        }
    }
    None
}

fn frame_bytes(value: &serde_json::Value, framing: w::StreamFraming) -> Option<String> {
    if framing == w::StreamFraming::JsonLines {
        return Some(format!("{value}\n"));
    }
    let object = value.as_object()?;
    if object
        .keys()
        .any(|k| !["data", "event", "id", "retry"].contains(&k.as_str()))
    {
        return None;
    }
    let mut output = String::new();
    for key in ["event", "id", "retry"] {
        if let Some(value) = object.get(key) {
            let value = if key == "retry" {
                value.as_u64()?.to_string()
            } else {
                value.as_str()?.to_owned()
            };
            if value.contains(['\r', '\n']) || (key == "id" && value.contains('\0')) {
                return None;
            }
            writeln!(output, "{key}: {value}").unwrap();
        }
    }
    let data = object.get("data")?.as_str()?;
    if data.contains('\r') {
        return None;
    }
    for line in data.split('\n') {
        writeln!(output, "data: {line}").unwrap();
    }
    output.push('\n');
    Some(output)
}

fn text(value: &str) -> String {
    if value.contains('\0') || value.len() > 16384 {
        return string(value);
    }
    let mut out = String::from("\"");
    for byte in value.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            32..=126 => out.push(byte as char),
            _ => write!(out, "\\{byte:03o}").unwrap(),
        }
    }
    out.push('"');
    out
}
fn number(value: &serde_json::Value, integer: bool) -> Option<String> {
    let number = value.as_number()?;
    let ty = if integer { "JsonInteger" } else { "JsonNumber" };
    if let Some(value) = number.as_i64().filter(|v| i32::try_from(*v).is_ok()) {
        return Some(format!("{ty}({value})"));
    }
    Some(format!(
        "{ty}::parse({}).value()",
        text(&number.to_string())
    ))
}
fn json(value: &serde_json::Value, depth: usize) -> Option<String> {
    if depth > 32 {
        return None;
    }
    Some(match value {
        serde_json::Value::Null => "JsonValue(Null{})".into(),
        serde_json::Value::Bool(value) => format!("JsonValue({value})"),
        serde_json::Value::Number(_) => format!("JsonValue({})", number(value, false)?),
        serde_json::Value::String(value) => format!("JsonValue({})", text(value)),
        serde_json::Value::Array(values) => format!(
            "JsonValue(JsonValue::Array{{{}}})",
            values
                .iter()
                .map(|v| json(v, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ),
        serde_json::Value::Object(values) => format!(
            "JsonValue(JsonValue::Object{{{}}})",
            values
                .iter()
                .map(|(k, v)| Some(format!("{{{}, {}}}", text(k), json(v, depth + 1)?)))
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ),
    })
}
fn native_value(
    plan: &SdkPlan,
    validator: &OwnedSchema,
    schema: &suspect_ir::contract::SchemaId,
    value: &serde_json::Value,
    depth: usize,
) -> Option<String> {
    if depth > 32 {
        return None;
    }
    let symbol = plan.models.get(schema);
    if symbol.nullable && value.is_null() {
        return Some("Null{}".into());
    }
    Some(match &symbol.shape {
        Shape::Json | Shape::ValidatedJson => json(value, depth + 1)?,
        Shape::Never => return None,
        Shape::Null => "Null{}".into(),
        Shape::Boolean => value.as_bool()?.to_string(),
        Shape::String => text(value.as_str()?),
        Shape::Number => number(value, false)?,
        Shape::Integer => number(value, true)?,
        Shape::Enum(values) => format!(
            "{}::{}",
            symbol.definition,
            values
                .iter()
                .find(|(wire, _)| Some(wire.as_str()) == value.as_str())?
                .1
        ),
        Shape::Ref { target, boxed } => {
            let expression = native_value(plan, validator, target, value, depth + 1)?;
            if *boxed {
                let ty = &plan.models.get(target).cpp_type;
                format!("Box<{ty}>({ty}({expression}))")
            } else {
                expression
            }
        }
        Shape::Array(item) => {
            let values = value
                .as_array()?
                .iter()
                .map(|v| {
                    if let Some(item) = item {
                        native_value(plan, validator, item, v, depth + 1)
                    } else {
                        json(v, depth + 1)
                    }
                })
                .collect::<Option<Vec<_>>>()?;
            let ty = item
                .as_ref()
                .map(|i| plan.models.get(i).cpp_type.as_str())
                .unwrap_or("JsonValue");
            format!("std::vector<{ty}>{{{}}}", values.join(", "))
        }
        Shape::Union { branches, .. } => {
            let (index, branch) = branches
                .iter()
                .enumerate()
                .find(|(_, b)| matches!(validator.validate(b, value), OwnedOutcome::Valid))?;
            format!(
                "{}::alternative_{index}({})",
                symbol.definition,
                native_value(plan, validator, branch, value, depth + 1)?
            )
        }
        Shape::Object { fields, extras } => {
            let object = value.as_object()?;
            let args = symbol
                .constructor
                .as_ref()?
                .parameters
                .iter()
                .map(|arg| {
                    let field = fields.iter().find(|f| f.name == arg.member_name)?;
                    native_value(
                        plan,
                        validator,
                        &field.schema,
                        object.get(&field.wire)?,
                        depth + 1,
                    )
                })
                .collect::<Option<Vec<_>>>()?;
            let mut assignments = Vec::new();
            for field in fields.iter().filter(|f| !f.required) {
                if let Some(value) = object.get(&field.wire) {
                    assignments.push(format!(
                        "value.{} = {};",
                        field.name,
                        native_value(plan, validator, &field.schema, value, depth + 1)?
                    ));
                }
            }
            for (key, value) in object
                .iter()
                .filter(|(key, _)| !fields.iter().any(|f| &f.wire == *key))
            {
                let expression = match extras {
                    Extras::Closed => return None,
                    Extras::Any | Extras::Patterned => json(value, depth + 1)?,
                    Extras::Typed(schema) => {
                        native_value(plan, validator, schema, value, depth + 1)?
                    }
                };
                assignments.push(format!("value.extra.emplace({}, {expression});", text(key)));
            }
            let construct = format!("{}({})", symbol.definition, args.join(", "));
            if assignments.is_empty() {
                construct
            } else {
                format!(
                    "[] {{ auto value = {construct}; {} return value; }}()",
                    assignments.join(" ")
                )
            }
        }
    })
}
