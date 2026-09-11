//! Native inputs, media choices, response/header variants and executable clients.
use super::super::{
    PlannedMedia, PlannedOperation, PlannedResponse, PlannedResponseCase, ValueKind,
};
use super::{SdkPlan, aggregates, prose, single_line, source_expr, string, wire};
use crate::http_protocol as w;
use std::fmt::Write;

pub(super) fn header(plan: &SdkPlan) -> String {
    let mut out = format!(
        "#pragma once\n/** @file client.hpp Native source-selected protocol client. */\n#include \"{0}/models.hpp\"\n#include \"{0}/stream.hpp\"\n\nnamespace {1} {{\n",
        plan.config.name, plan.config.namespace
    );
    out.push_str("/// Explicit credentials; provider callbacks receive source metadata and never trigger SDK acquisition.\nstruct Credentials {\n");
    for credential in plan.credentials.values() {
        writeln!(
            out,
            "    /// Source security scheme {}.\n    Presence<{}> {} = std::nullopt;",
            single_line(credential.wire.name()),
            credential.cpp_type,
            credential.field_name
        )
        .unwrap();
    }
    out.push_str("};\n");
    out.push_str(&aggregates::header(plan));
    for op in &plan.operations {
        if let Some(body) = &op.body
            && let Some(choice) = &body.choice_type
        {
            for media in &body.media {
                writeln!(out,"/// Explicit request representation: {}.\nstruct {} {{\n    {} data;\n    {} content_type;\n    explicit {}({} value{}) : data(std::move(value)){} {{}}\n}};",single_line(media.wire.media_type().declared()),media.wrapper_type,media.value.cpp_type,if media.requires_content_type{"std::string"}else{"Presence<std::string>"},media.wrapper_type,media.value.cpp_type,if media.requires_content_type{", std::string content_type_value"}else{""},if media.requires_content_type{", content_type(std::move(content_type_value))"}else{""}).unwrap();
            }
            writeln!(
                out,
                "using {choice} = std::variant<{}>;",
                body.media
                    .iter()
                    .map(|m| m.wrapper_type.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
        writeln!(
            out,
            "/// Input for {}.\nstruct {} {{",
            single_line(&op.operation_id),
            op.input_type
        )
        .unwrap();
        for p in &op.parameters {
            writeln!(
                out,
                "    /// {:?} wire member {}.\n    {} {}{};",
                p.wire.location(),
                single_line(&p.wire_name),
                p.cpp_type,
                p.field_name,
                if p.required { "" } else { " = std::nullopt" }
            )
            .unwrap();
        }
        if let Some(body) = &op.body {
            writeln!(
                out,
                "    {} body{};",
                body.cpp_type,
                if body.required { "" } else { " = std::nullopt" }
            )
            .unwrap();
        }
        let ctor = &op.constructor;
        if ctor.parameters.is_empty() {
            writeln!(out, "    {}() = default;", ctor.name).unwrap();
        } else {
            writeln!(
                out,
                "    explicit {}({}) : {} {{}}",
                ctor.name,
                ctor.parameters
                    .iter()
                    .map(|p| format!("{} {}", p.cpp_type, p.name))
                    .collect::<Vec<_>>()
                    .join(", "),
                ctor.parameters
                    .iter()
                    .map(|p| format!("{}(std::move({}))", p.member_name, p.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
        out.push_str("};\n");
        for response in &op.responses {
            for case in &response.cases {
                writeln!(out,"/// Matched status rule {}{}; response.status is the actual status.\nstruct {} {{\n    {} data;\n    ResponseMetadata response;",single_line(&response.status_key),case.media.as_ref().map(|m|format!(", {}",single_line(m.media_type().declared()))).unwrap_or_default(),case.variant_type,case.value.cpp_type).unwrap();
                if let Some(headers) = &response.headers_type {
                    writeln!(out, "    {headers} headers;").unwrap();
                }
                writeln!(out,"    explicit {}({} value, ResponseMetadata metadata{}) : data(std::move(value)), response(std::move(metadata)){} {{}}\n}};",case.variant_type,case.value.cpp_type,response.headers_type.as_ref().map(|h|format!(", {h} typed_headers")).unwrap_or_default(),if response.headers_type.is_some(){", headers(std::move(typed_headers))"}else{""}).unwrap();
            }
        }
        let success = op
            .responses
            .iter()
            .filter(|r| r.can_succeed())
            .flat_map(|r| r.cases.iter().map(|c| c.variant_type.clone()))
            .collect::<Vec<_>>();
        let error = std::iter::once("SdkError".to_owned())
            .chain(
                op.responses
                    .iter()
                    .filter(|r| r.can_fail())
                    .flat_map(|r| r.cases.iter().map(|c| c.variant_type.clone())),
            )
            .collect::<Vec<_>>();
        writeln!(
            out,
            "using {} = std::variant<{}>;\nusing {} = std::variant<{}>;",
            op.success_type,
            if success.is_empty() {
                "Never".into()
            } else {
                success.join(", ")
            },
            op.error_type,
            error.join(", ")
        )
        .unwrap();
    }
    out.push_str("/// Copies share the transport; stream responses retain their own transfer lease.\nclass Client {\n    std::shared_ptr<const Transport> transport_;\n    Credentials credentials_;\n    ClientOptions options_;\npublic:\n    explicit Client(std::shared_ptr<const Transport> transport, Credentials credentials = {}, ClientOptions options = {})\n        : transport_(std::move(transport)), credentials_(std::move(credentials)), options_(std::move(options)) {}\n");
    writeln!(out,"#if defined({}_HAS_CURL)\n    static Result<Client, TransportError> with_curl(Credentials credentials = {{}}, ClientOptions options = {{}}, CurlOptions curl = {{}});\n#endif",plan.config.name).unwrap();
    out.push_str(&super::credential_env::declarations(plan));
    for op in &plan.operations {
        writeln!(
            out,
            "    /// {} {}. Source operation {}.\n    /// Source: {}#{}",
            op.wire.method().as_str(),
            single_line(op.wire.path()),
            single_line(&op.operation_id),
            single_line(op.source.document().as_str()),
            single_line(op.source.pointer())
        )
        .unwrap();
        if let Some(description) = op.wire.description() {
            for line in prose(description.value()).lines() {
                writeln!(out, "    /// {line}").unwrap();
            }
        }
        writeln!(out,"    [[nodiscard]] Result<{}, {}> {}(const {}& input{}, CallOptions options = {{}}) const;",op.success_type,op.error_type,op.method_name,op.input_type,if op.constructor.parameters.is_empty(){" = {}"}else{""}).unwrap();
    }
    out.push_str("};\n");
    out.push_str("namespace detail_native {\n");
    out.push_str(&aggregates::declarations(plan));
    out.push_str("}\n");
    writeln!(out, "}} // namespace {}", plan.config.namespace).unwrap();
    out
}

pub(super) fn source(plan: &SdkPlan) -> String {
    let mut out = format!(
        "#include \"{}/client.hpp\"\n\nnamespace {} {{\nnamespace detail_native {{\n",
        plan.config.name, plan.config.namespace
    );
    if plan.credential_env().is_some() {
        out.insert_str(0, "#include <cstdlib>\n");
    }
    out.push_str(&aggregates::source(plan));
    out.push_str("} // namespace detail_native\n");
    writeln!(out,"#if defined({}_HAS_CURL)\nResult<Client, TransportError> Client::with_curl(Credentials credentials, ClientOptions options, CurlOptions curl) {{\n    auto transport = CurlTransport::create(std::move(curl));\n    if (!transport) return Result<Client, TransportError>::failure(std::move(transport).error());\n    return Result<Client, TransportError>::success(Client(std::move(transport).value(), std::move(credentials), std::move(options)));\n}}\n#endif",plan.config.name).unwrap();
    out.push_str(&super::credential_env::definitions(plan));
    for op in &plan.operations {
        writeln!(out,"Result<{}, {}> Client::{}(const {}& input, CallOptions options) const {{\n    (void)input; (void)credentials_; using Outcome = Result<{}, {}>;\n    static const detail::Operation operation = {};\n    Presence<ResponseMetadata> retained;\n    try {{\n        auto settings = detail::settings(options_, options, operation.source);\n        auto owned_context = std::make_unique<detail::Context>(settings.control());\n        auto& context = *owned_context; context.limits.max_bytes = settings.max_request_bytes;\n        std::vector<detail::ParameterValue> parameters;",op.success_type,op.error_type,op.method_name,op.input_type,op.success_type,op.error_type,wire::operation(plan,op)).unwrap();
        for (i, p) in op.parameters.iter().enumerate() {
            if !p.required {
                writeln!(out, "if (input.{}) {{", p.field_name).unwrap();
            }
            let expression = format!(
                "{}input.{}",
                if p.required { "" } else { "*" },
                p.field_name
            );
            let src = source_expr(plan, &p.source);
            if let ValueKind::Aggregate(name) = &p.value.kind {
                writeln!(out,"auto encoded{i} = detail_native::encode_{name}({expression}, context, settings, {});\nparameters.push_back(detail::ParameterValue{{{src}, {}, detail::Location::{}, {}, {}, JsonValue(), std::move(encoded{i}.bytes)}});",string(p.wire.content_media().unwrap().media_type().declared()),string(&p.wire_name),wire::location(p.wire.location()),p.required,wire::serialization(p.wire.serialization())).unwrap();
            } else {
                wire::encode_json(
                    plan,
                    &p.value,
                    &expression,
                    &mut out,
                    &format!("json{i}"),
                    &src,
                );
                writeln!(out,"parameters.push_back(detail::ParameterValue{{{src}, {}, detail::Location::{}, {}, {}, std::move(json{i}), std::nullopt}});",string(&p.wire_name),wire::location(p.wire.location()),p.required,wire::serialization(p.wire.serialization())).unwrap();
            }
            if !p.required {
                out.push_str("}\n");
            }
        }
        out.push_str("Presence<detail::EncodedBody> body;\n");
        if let Some(body) = &op.body {
            if !body.required {
                out.push_str("if (input.body) {\n");
            }
            let value = if body.required {
                "input.body"
            } else {
                "*input.body"
            };
            if body.choice_type.is_some() {
                writeln!(out,"const auto& choice = {value};\nif (choice.valueless_by_exception()) detail::http_fail(SdkError::Kind::RequestRepresentation, operation.source, \"valueless request media choice\");\nswitch (choice.index()) {{").unwrap();
                let media = body
                    .media
                    .iter()
                    .map(|m| wire::media_plan(&m.wire))
                    .collect::<Vec<_>>()
                    .join(", ");
                for (i, m) in body.media.iter().enumerate() {
                    writeln!(out,"case {i}: {{\nconst auto& selected = std::get<{i}>(choice);\nstd::string content_type = {};\nif (detail::select_media({{{media}}}, content_type, {}) != {i}) detail::http_fail(SdkError::Kind::RequestRepresentation, operation.source, \"request choice bypasses a more specific source representation\");",if m.requires_content_type{"selected.content_type".into()}else{format!("selected.content_type.value_or({})",string(m.wire.media_type().declared()))},source_expr(plan,&m.source)).unwrap();
                    encode_media(plan, m, "selected.data", &mut out, "content_type");
                    out.push_str("break;\n}\n");
                }
                out.push_str("default: detail::http_fail(SdkError::Kind::RequestRepresentation, operation.source, \"invalid request media index\");\n}\n");
            } else {
                let m = &body.media[0];
                encode_media(
                    plan,
                    m,
                    value,
                    &mut out,
                    &string(m.wire.media_type().declared()),
                );
            }
            if !body.required {
                out.push_str("}\n");
            }
        }
        out.push_str("detail::CredentialValues credentials;\n");
        for credential in plan.credentials.values() {
            writeln!(
                out,
                "if (credentials_.{}) credentials.emplace({}, std::cref(*credentials_.{}));",
                credential.field_name,
                string(&credential.field_name),
                credential.field_name
            )
            .unwrap();
        }
        out.push_str("auto request = detail::prepare_request(operation, parameters, std::move(body), credentials, settings, context);\nauto opened = detail::open_exchange(transport_, request, settings, operation.source);\nif (!opened) {\n    auto error = std::move(opened).error(); error.operation_source = operation.source; error.operation_id = operation.id;\n");
        writeln!(
            out,
            "    return Outcome::failure({}(std::in_place_type<SdkError>, std::move(error)));\n}}",
            op.error_type
        )
        .unwrap();
        out.push_str("auto exchange = std::move(opened).value();\nretained = detail::metadata(HttpResponse{exchange.status, exchange.headers, {}}, settings.transfer.max_capture_bytes);\ncontext.limits.max_bytes = settings.transfer.max_response_bytes;\nconst int status = exchange.status; (void)status;\nint selected_response = -1;\n");
        for (i, r) in op
            .responses
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.status, w::ResponseStatus::Exact(_)))
        {
            if let w::ResponseStatus::Exact(s) = r.status {
                writeln!(out, "if (status == {s}) selected_response = {i};").unwrap();
            }
        }
        for (i, r) in op
            .responses
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.status, w::ResponseStatus::Range(_)))
        {
            if let w::ResponseStatus::Range(s) = r.status {
                writeln!(
                    out,
                    "if (selected_response < 0 && status / 100 == {s}) selected_response = {i};"
                )
                .unwrap();
            }
        }
        for (i, _) in op
            .responses
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.status, w::ResponseStatus::Default))
        {
            writeln!(out, "if (selected_response < 0) selected_response = {i};").unwrap();
        }
        out.push_str("switch (selected_response) {\n");
        for (i, response) in op.responses.iter().enumerate() {
            writeln!(out, "case {i}: {{").unwrap();
            emit_response(plan, op, response, &mut out);
            out.push_str("}\n");
        }
        out.push_str("default: {\nauto collected = detail::collect(std::move(exchange), settings, operation.source);\nif (!collected) throw detail::HttpFailure{std::move(collected).error()};\nretained = detail::metadata(collected.value(), settings.transfer.max_capture_bytes);\ndetail::http_fail(SdkError::Kind::UnexpectedResponse, operation.source, \"status is not declared\");\n}\n}\n");
        writeln!(out,"}} catch (detail::Failure& failure) {{\n    return Outcome::failure({}(std::in_place_type<SdkError>, detail::codec_error(operation, std::move(failure.error), std::move(retained))));\n}} catch (detail::HttpFailure& failure) {{\n    failure.error.operation_source = operation.source; failure.error.operation_id = operation.id;\n    if (retained && (failure.error.kind == SdkError::Kind::RequestValidation || failure.error.kind == SdkError::Kind::RequestRepresentation)) failure.error.kind = SdkError::Kind::ResponseDecoding;\n    if (!failure.error.response) failure.error.response = std::move(retained);\n    return Outcome::failure({}(std::in_place_type<SdkError>, std::move(failure.error)));\n}}\n}}",op.error_type,op.error_type).unwrap();
    }
    writeln!(out, "}} // namespace {}", plan.config.namespace).unwrap();
    out
}

fn encode_media(
    plan: &SdkPlan,
    media: &PlannedMedia,
    expression: &str,
    out: &mut String,
    content_type: &str,
) {
    let source = source_expr(plan, &media.source);
    match &media.value.kind {
        ValueKind::Aggregate(name) => {
            writeln!(out,"body = detail_native::encode_{name}({expression}, context, settings, {content_type});").unwrap();
        }
        ValueKind::Bytes => {
            let w::Representation::Binary { bytes, .. } = media.wire.representation() else {
                unreachable!()
            };
            writeln!(out,"body = detail::EncodedBody{{detail::request_bytes({expression}, context, {source}, std::min<std::size_t>({}, settings.max_request_bytes)), {content_type}}};",bytes.max_bytes()).unwrap();
        }
        ValueKind::Stream {
            schema,
            framing,
            max_item_bytes,
        } => {
            let symbol = plan.models().get(schema);
            writeln!(out,"std::string encoded;\nfor (const auto& item : {expression}) {{\nauto json = detail::encode_{}(item, context, \"\"); context.validation.require({}, json, \"\");\nauto frame = detail::encode_item(json, StreamFraming::{}, context, {source}, std::min<std::size_t>({max_item_bytes}, settings.max_item_bytes));\ndetail::append_bounded(encoded, frame, settings.max_request_bytes, {source}, context);\n}}\nbody = detail::EncodedBody{{std::move(encoded), {content_type}}};",symbol.index,symbol.index,framing_name(*framing)).unwrap();
        }
        _ => {
            wire::encode_json(plan, &media.value, expression, out, "body_json", &source);
            let encode = match media.wire.representation() {
                w::Representation::Text { scalar, .. } => format!(
                    "detail::scalar_text(body_json, detail::Scalar::{}, {source})",
                    wire::scalar(*scalar)
                ),
                _ => format!("detail::write_document(body_json, context, {source})"),
            };
            writeln!(
                out,
                "body = detail::EncodedBody{{{encode}, {content_type}}};"
            )
            .unwrap();
        }
    }
}
fn emit_response(
    plan: &SdkPlan,
    op: &PlannedOperation,
    response: &PlannedResponse,
    out: &mut String,
) {
    let source = source_expr(plan, &response.source);
    writeln!(
        out,
        "const auto response_source = {source};\nretained->links = {{{}}};",
        wire::links(plan, response.wire.links())
    )
    .unwrap();
    if let Some(headers) = &response.headers_type {
        writeln!(
            out,
            "auto typed_headers = detail_native::decode_{headers}(exchange.headers, context);"
        )
        .unwrap();
    }
    if let Some(case) = response.cases.iter().find(|c| c.forbidden) {
        out.push_str("if (detail::forbidden_body(operation.method, status)) {\nif (exchange.body) exchange.body->close();\n");
        return_case(op, response, case, "Unit{}", out);
        out.push_str("}\n");
    }
    let ordinary = response
        .cases
        .iter()
        .filter(|c| !c.forbidden)
        .collect::<Vec<_>>();
    if ordinary.is_empty() {
        out.push_str("detail::http_fail(SdkError::Kind::UnexpectedResponse, response_source, \"response has no body representation for this status\");\n");
        return;
    }
    if ordinary[0].media.is_none() {
        decode_case(plan, op, response, ordinary[0], out);
        return;
    }
    out.push_str("auto content_type = detail::content_type(exchange.headers);\nif (!content_type) {\nauto collected = detail::collect(std::move(exchange), settings, response_source);\nif (!collected) throw detail::HttpFailure{std::move(collected).error()};\nretained = detail::metadata(collected.value(), settings.transfer.max_capture_bytes);\ndetail::http_fail(SdkError::Kind::UnexpectedResponse, response_source, \"Content-Type absent or duplicated\");\n}\nstd::size_t selected_media;\ntry {\n");
    writeln!(
        out,
        "selected_media = detail::select_media({{{}}}, *content_type, response_source);",
        ordinary
            .iter()
            .map(|c| wire::media_plan(c.media.as_ref().unwrap()))
            .collect::<Vec<_>>()
            .join(", ")
    )
    .unwrap();
    out.push_str("if (settings.response_media && status >= 200 && status < 300) { auto expected = detail::parse_media(*settings.response_media); auto actual = detail::parse_media(*content_type); if (!expected || !actual || !detail::matches_media(*expected, *actual)) detail::http_fail(SdkError::Kind::UnexpectedResponse, response_source, \"response differs from requested representation\"); }\n} catch (detail::HttpFailure& failure) {\nauto collected = detail::collect(std::move(exchange), settings, response_source);\nif (!collected) throw detail::HttpFailure{std::move(collected).error()};\nretained = detail::metadata(collected.value(), settings.transfer.max_capture_bytes); failure.error.kind = SdkError::Kind::UnexpectedResponse; throw;\n}\nswitch (selected_media) {\n");
    for (i, case) in ordinary.iter().enumerate() {
        writeln!(out, "case {i}: {{").unwrap();
        decode_case(plan, op, response, case, out);
        out.push_str("}\n");
    }
    out.push_str("default: detail::http_fail(SdkError::Kind::UnexpectedResponse, response_source, \"invalid representation selection\");\n}\n");
}
fn decode_case(
    plan: &SdkPlan,
    op: &PlannedOperation,
    response: &PlannedResponse,
    case: &PlannedResponseCase,
    out: &mut String,
) {
    let source = source_expr(plan, &case.source);
    if !matches!(case.value.kind, ValueKind::Bytes | ValueKind::Unit) {
        out.push_str("for (const auto& [name, value] : exchange.headers) if (detail::lower_ascii(name) == \"content-encoding\" && detail::lower_ascii(value) != \"identity\") detail::http_fail(SdkError::Kind::ResponseDecoding, response_source, \"structured response content encoding is not identity\");\n");
    }
    if let ValueKind::Stream {
        schema,
        framing,
        max_item_bytes,
    } = &case.value.kind
    {
        let model = plan.models().get(schema);
        writeln!(out,"auto state = std::make_unique<detail::ItemState>(std::move(exchange), settings, operation, {source}, StreamFraming::{}, {max_item_bytes}, std::move(owned_context));\nstate->response.links = retained->links;\n{} data(std::move(state), detail::decode_{}, {});",framing_name(*framing),case.value.cpp_type,model.index,model.index).unwrap();
    } else {
        out.push_str("auto collected = detail::collect(std::move(exchange), settings, response_source);\nif (!collected) throw detail::HttpFailure{std::move(collected).error()};\nauto raw = std::move(collected).value();\nretained = detail::metadata(raw, settings.transfer.max_capture_bytes);\n");
        writeln!(
            out,
            "retained->links = {{{}}};",
            wire::links(plan, response.wire.links())
        )
        .unwrap();
        match &case.value.kind {
            ValueKind::Bytes => {
                let limit = case
                    .media
                    .as_ref()
                    .and_then(|m| {
                        if let w::Representation::Binary { bytes, .. } = m.representation() {
                            Some(bytes.max_bytes())
                        } else {
                            None
                        }
                    })
                    .unwrap_or(response.wire.max_body_bytes());
                writeln!(out,"auto data = detail::response_bytes(raw.body, context, {source}, std::min<std::size_t>({limit}, settings.transfer.max_response_bytes));").unwrap();
            }
            ValueKind::Aggregate(name) => {
                writeln!(out,"auto data = detail_native::decode_{name}(raw.body, *content_type, context, settings);").unwrap();
            }
            _ => {
                let parse = case.media.as_ref().map(|m| m.representation());
                if let Some(w::Representation::Text { scalar, .. }) = parse {
                    writeln!(out,"auto json = detail::parse_scalar(raw.body, detail::Scalar::{}, context, {source});",wire::scalar(*scalar)).unwrap();
                } else {
                    writeln!(
                        out,
                        "auto json = detail::parse_document(raw.body, context, {source});"
                    )
                    .unwrap();
                }
                wire::decode_json(plan, &case.value, "json", out, "data", &source);
            }
        }
    }
    return_case(op, response, case, "std::move(data)", out);
}
fn return_case(
    op: &PlannedOperation,
    response: &PlannedResponse,
    case: &PlannedResponseCase,
    data: &str,
    out: &mut String,
) {
    let args = format!(
        "std::in_place_type<{}>, {data}, std::move(*retained){}",
        case.variant_type,
        if response.headers_type.is_some() {
            ", std::move(typed_headers)"
        } else {
            ""
        }
    );
    if response.can_succeed() && response.can_fail() {
        writeln!(out,"if (status >= 200 && status < 300) return Outcome::success({}({args}));\nreturn Outcome::failure({}({args}));",op.success_type,op.error_type).unwrap();
    } else {
        writeln!(
            out,
            "return Outcome::{}({}({args}));",
            if response.can_succeed() {
                "success"
            } else {
                "failure"
            },
            if response.can_succeed() {
                &op.success_type
            } else {
                &op.error_type
            }
        )
        .unwrap();
    }
}
fn framing_name(framing: w::StreamFraming) -> &'static str {
    match framing {
        w::StreamFraming::ServerSentEvents => "ServerSentEvents",
        w::StreamFraming::JsonLines => "JsonLines",
    }
}
