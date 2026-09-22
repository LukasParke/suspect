use super::super::{PlannedAggregate, PlannedHeader, PlannedPart, ValueKind};
use super::{SdkPlan, source_expr, string, wire};
use crate::http_protocol as w;
use std::{collections::BTreeMap, fmt::Write};

fn headers(plan: &SdkPlan) -> BTreeMap<&str, &[PlannedHeader]> {
    let mut result = BTreeMap::new();
    for op in plan.operations() {
        for response in &op.responses {
            if let Some(name) = &response.headers_type {
                result.insert(name.as_str(), response.headers.as_slice());
            }
        }
    }
    for aggregate in plan.aggregates() {
        for part in aggregate.fields.iter().chain(aggregate.additional.iter()) {
            if let Some(name) = &part.headers_type {
                result.insert(name.as_str(), part.headers.as_slice());
            }
        }
    }
    result
}
fn element(part: &PlannedPart) -> String {
    part.part_type
        .clone()
        .unwrap_or_else(|| part.value.cpp_type.clone())
}
pub(super) fn field_type(part: &PlannedPart) -> String {
    let ty = element(part);
    if part.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
        format!("std::vector<{ty}>")
    } else {
        ty
    }
}
pub(super) fn header(plan: &SdkPlan) -> String {
    let mut out = String::new();
    for (name, fields) in headers(plan) {
        writeln!(
            out,
            "/// Source-typed HTTP or MIME response/request headers.\nstruct {name} {{"
        )
        .unwrap();
        for field in fields {
            writeln!(
                out,
                "    {} {}{};",
                if field.wire.required() {
                    field.value.cpp_type.clone()
                } else {
                    format!("Presence<{}>", field.value.cpp_type)
                },
                field.field_name,
                if field.wire.required() {
                    ""
                } else {
                    " = std::nullopt"
                }
            )
            .unwrap();
        }
        constructor(
            &mut out,
            name,
            fields
                .iter()
                .filter(|f| f.wire.required())
                .map(|f| (f.field_name.clone(), f.value.cpp_type.clone()))
                .collect(),
        );
        out.push_str("};\n");
    }
    for aggregate in plan.aggregates() {
        for part in aggregate.fields.iter().chain(aggregate.additional.iter()) {
            if let Some(name) = &part.part_type {
                writeln!(out,"/// One owned MIME part; filename is metadata and data contains actual bytes/model values.\nstruct {name} {{\n    {} data;\n    Presence<std::string> filename;\n    Presence<std::string> content_type;",part.value.cpp_type).unwrap();
                let mut fields = vec![("data".into(), part.value.cpp_type.clone())];
                if let Some(header) = &part.headers_type {
                    writeln!(out, "    {header} headers;").unwrap();
                    if part.headers.iter().any(|h| h.wire.required()) {
                        fields.push(("headers".into(), header.clone()));
                    }
                }
                constructor(&mut out, name, fields);
                out.push_str("};\n");
            }
        }
        writeln!(
            out,
            "/// {} aggregate; binary members are never JSON placeholders.\nstruct {} {{",
            if aggregate.multipart {
                "Named multipart"
            } else {
                "Form"
            },
            aggregate.type_name
        )
        .unwrap();
        for field in &aggregate.fields {
            let ty = field_type(field);
            writeln!(
                out,
                "    {} {}{};",
                if field.wire.required() {
                    ty
                } else {
                    format!("Presence<{ty}>")
                },
                field.field_name,
                if field.wire.required() {
                    ""
                } else {
                    " = std::nullopt"
                }
            )
            .unwrap();
        }
        if let Some(extra) = &aggregate.additional {
            writeln!(
                out,
                "    std::map<std::string, {}, std::less<>> extra;",
                field_type(extra)
            )
            .unwrap();
        }
        constructor(
            &mut out,
            &aggregate.type_name,
            aggregate
                .fields
                .iter()
                .filter(|p| p.wire.required())
                .map(|p| (p.field_name.clone(), field_type(p)))
                .collect(),
        );
        out.push_str("};\n");
    }
    out
}
fn constructor(out: &mut String, name: &str, fields: Vec<(String, String)>) {
    if fields.is_empty() {
        writeln!(out, "    {name}() = default;").unwrap();
    } else {
        writeln!(
            out,
            "    explicit {name}({}) : {} {{}}",
            fields
                .iter()
                .enumerate()
                .map(|(i, (_, ty))| format!("{ty} arg{i}"))
                .collect::<Vec<_>>()
                .join(", "),
            fields
                .iter()
                .enumerate()
                .map(|(i, (name, _))| format!("{name}(std::move(arg{i}))"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
}
pub(super) fn declarations(plan: &SdkPlan) -> String {
    let mut out = String::new();
    for (name, _) in headers(plan) {
        writeln!(out,"Headers encode_{name}(const {name}&, detail::Context&);\n{name} decode_{name}(const Headers&, detail::Context&);").unwrap();
    }
    for aggregate in plan.aggregates() {
        let name = &aggregate.type_name;
        writeln!(out,"detail::EncodedBody encode_{name}(const {name}&, detail::Context&, const detail::Settings&, std::string);\n{name} decode_{name}(std::string_view, std::string_view, detail::Context&, const detail::Settings&);").unwrap();
    }
    out
}
pub(super) fn source(plan: &SdkPlan) -> String {
    let mut out = String::new();
    for (name, fields) in headers(plan) {
        writeln!(out,"Headers encode_{name}(const {name}& input, detail::Context& context) {{\n    (void)input; (void)context; Headers headers;").unwrap();
        for (i, h) in fields.iter().enumerate() {
            let source = source_expr(plan, &h.source);
            if !h.wire.required() {
                writeln!(out, "if (input.{}) {{", h.field_name).unwrap();
            }
            wire::encode_json(
                plan,
                &h.value,
                &format!(
                    "{}input.{}",
                    if h.wire.required() { "" } else { "*" },
                    h.field_name
                ),
                &mut out,
                &format!("json{i}"),
                &source,
            );
            writeln!(out,"headers.emplace_back({}, detail::encode_parameter({}, detail::Location::Header, {}, json{i}, context, {source}, {}));",string(h.wire.name()),string(h.wire.name()),wire::serialization(h.wire.serialization()),plan.config.max_header_bytes).unwrap();
            if !h.wire.required() {
                out.push_str("}\n");
            }
        }
        out.push_str("return headers;\n}\n");
        writeln!(out,"{name} decode_{name}(const Headers& headers, detail::Context& context) {{\n    (void)headers; (void)context;").unwrap();
        for (i, h) in fields.iter().enumerate() {
            let source = source_expr(plan, &h.source);
            writeln!(out,"Presence<{}> field{i};\nif (std::any_of(headers.begin(), headers.end(), [](const auto& h) {{ return detail::lower_ascii(h.first) == {}; }})) {{",h.value.cpp_type,string(&h.wire.name().to_ascii_lowercase())).unwrap();
            writeln!(
                out,
                "auto serialization = {};",
                wire::serialization(h.wire.serialization())
            )
            .unwrap();
            if let Some(media) = h.wire.content_media()
                && let w::Representation::Text { scalar, .. } = media.representation()
            {
                writeln!(
                    out,
                    "serialization.shape.scalar = detail::Scalar::{};",
                    wire::scalar(*scalar)
                )
                .unwrap();
            }
            writeln!(
                out,
                "auto json = detail::decode_header(headers, {}, serialization, context, {source});",
                string(h.wire.name())
            )
            .unwrap();
            wire::decode_json(plan, &h.value, "json", &mut out, "value", &source);
            writeln!(out, "field{i} = std::move(value);\n}}").unwrap();
            if h.wire.required() {
                writeln!(out,"if (!field{i}) detail::http_fail(SdkError::Kind::ResponseDecoding, {source}, \"required header absent\");").unwrap();
            }
        }
        let args = fields
            .iter()
            .enumerate()
            .filter(|(_, h)| h.wire.required())
            .map(|(i, _)| format!("std::move(*field{i})"))
            .collect::<Vec<_>>();
        writeln!(
            out,
            "{name} result{};",
            if args.is_empty() {
                String::new()
            } else {
                format!("({})", args.join(", "))
            }
        )
        .unwrap();
        for (i, h) in fields
            .iter()
            .enumerate()
            .filter(|(_, h)| !h.wire.required())
        {
            writeln!(out, "result.{} = std::move(field{i});", h.field_name).unwrap();
        }
        out.push_str("return result;\n}\n");
    }
    for aggregate in plan.aggregates() {
        out.push_str(&aggregate_source(plan, aggregate));
    }
    out
}
fn aggregate_source(plan: &SdkPlan, aggregate: &PlannedAggregate) -> String {
    let mut out = String::new();
    let name = &aggregate.type_name;
    let source = source_expr(plan, &aggregate.source);
    for (i, part) in aggregate
        .fields
        .iter()
        .chain(aggregate.additional.iter())
        .enumerate()
    {
        let ty = element(part);
        let source = source_expr(plan, &part.source);
        writeln!(out,"void encode_{name}_part{i}(const {ty}& input, std::string wire_name, detail::Context& context, const detail::Settings& settings, std::vector<detail::RawPart>& parts, std::string& form) {{\n    (void)parts; (void)form;\n    auto rules = {}; rules.name = wire_name;\n    const auto source = {source};",wire::part(plan,part)).unwrap();
        let expression = if aggregate.multipart {
            "input.data"
        } else {
            "input"
        };
        if matches!(part.value.kind, ValueKind::Bytes) {
            writeln!(out,"auto bytes = detail::request_bytes({expression}, context, source, std::min(settings.max_part_bytes, rules.max_bytes));").unwrap();
        } else {
            wire::encode_json(plan, &part.value, expression, &mut out, "json", "source");
            if aggregate.multipart {
                out.push_str("auto bytes = detail::part_json_bytes(rules, json, context, std::min(settings.max_part_bytes, rules.max_bytes));\n");
            }
        }
        if aggregate.multipart {
            out.push_str("if (parts.size() == settings.max_parts) detail::http_fail(SdkError::Kind::ResourceLimit, source, \"multipart part count exceeded\");\nHeaders headers;\n");
            if let Some(header) = &part.headers_type {
                writeln!(out, "headers = encode_{header}(input.headers, context);").unwrap();
            }
            out.push_str("if (wire_name.size() > settings.transfer.max_header_bytes || (input.filename && input.filename->size() > settings.transfer.max_header_bytes) || (input.content_type && input.content_type->size() > settings.transfer.max_header_bytes)) detail::http_fail(SdkError::Kind::ResourceLimit, source, \"part metadata exceeds header ceiling\");\n");
            if matches!(
                part.wire.representation(),
                w::PartRepresentation::Style { .. }
            ) {
                out.push_str("if (input.content_type) { if (!detail::parse_media(*input.content_type)) detail::http_fail(SdkError::Kind::RequestRepresentation, source, \"invalid part Content-Type\"); headers.emplace_back(\"Content-Type\", *input.content_type); }\n");
            } else {
                out.push_str("headers.emplace_back(\"Content-Type\", detail::select_part_media(rules, input.content_type));\n");
            }
            out.push_str("headers.emplace_back(\"Content-Disposition\", detail::disposition(wire_name, input.filename, source));\nparts.push_back(detail::RawPart{std::move(wire_name), std::move(headers), std::move(bytes)});\n");
        } else {
            out.push_str("auto encoded = detail::encode_form_field(rules, json, wire_name, context, settings.max_request_bytes - form.size());\nif (!form.empty()) detail::append_bounded(form, \"&\", settings.max_request_bytes, source, context);\ndetail::append_bounded(form, encoded, settings.max_request_bytes, source, context);\n");
        }
        out.push_str("}\n");
        writeln!(out,"{ty} decode_{name}_part{i}(const std::vector<detail::RawPart>& group, detail::Context& context, const detail::Settings& settings) {{\n    const auto source = {source};\n    auto rules = {};\n    (void)settings;\n",wire::part(plan,part)).unwrap();
        if aggregate.multipart {
            out.push_str("if (group.size() != 1) detail::http_fail(SdkError::Kind::ResponseDecoding, source, \"duplicate single MIME part\");\nconst auto& part = group[0];\nauto content_type = detail::find_header(part.headers, \"content-type\", source, false);\n");
            if matches!(
                part.wire.representation(),
                w::PartRepresentation::Style { .. }
            ) {
                out.push_str("if (content_type && !detail::parse_media(*content_type)) detail::http_fail(SdkError::Kind::ResponseDecoding, source, \"invalid part Content-Type\");\n");
            } else {
                out.push_str("(void)detail::select_media(rules.content_types, content_type.value_or(\"text/plain\"), source);\n");
            }
            if matches!(part.value.kind, ValueKind::Bytes) {
                out.push_str("auto value = detail::response_bytes(part.bytes, context, source, std::min(settings.max_part_bytes, rules.max_bytes));\n");
            } else {
                out.push_str("auto json = detail::part_json_value(rules, part.bytes, context);\n");
                wire::decode_json(plan, &part.value, "json", &mut out, "value", "source");
            }
            let headers_arg = if let Some(header) = &part.headers_type {
                writeln!(
                    out,
                    "auto typed_headers = decode_{header}(part.headers, context);"
                )
                .unwrap();
                if part.headers.iter().any(|h| h.wire.required()) {
                    ", std::move(typed_headers)"
                } else {
                    ""
                }
            } else {
                ""
            };
            writeln!(out,"{ty} result(std::move(value){headers_arg});\nresult.filename = detail::part_filename(part, source); result.content_type = std::move(content_type);").unwrap();
            if part.headers_type.is_some() && headers_arg.is_empty() {
                out.push_str("result.headers = std::move(typed_headers);\n");
            }
            out.push_str("return result;\n");
        } else {
            out.push_str("auto json = detail::form_value(rules, group, context);\n");
            wire::decode_json(plan, &part.value, "json", &mut out, "value", "source");
            out.push_str("return value;\n");
        }
        out.push_str("}\n");
    }
    writeln!(out,"detail::EncodedBody encode_{name}(const {name}& input, detail::Context& context, const detail::Settings& settings, std::string content_type) {{\n    (void)input;\n    const auto source = {source};\n    std::vector<detail::RawPart> parts; std::string form; std::set<std::string> names;").unwrap();
    for (i, part) in aggregate.fields.iter().enumerate() {
        if !part.wire.required() {
            writeln!(out, "if (input.{}) {{", part.field_name).unwrap();
        }
        let expr = format!(
            "{}input.{}",
            if part.wire.required() { "" } else { "*" },
            part.field_name
        );
        let key = string(part.name.as_ref().unwrap());
        writeln!(out, "names.insert({key});").unwrap();
        if part.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
            writeln!(out,"detail::part_count({}, ({expr}).size());\nfor (const auto& item : {expr}) encode_{name}_part{i}(item, {key}, context, settings, parts, form);",wire::part(plan,part)).unwrap();
        } else {
            writeln!(
                out,
                "encode_{name}_part{i}({expr}, {key}, context, settings, parts, form);"
            )
            .unwrap();
        }
        if !part.wire.required() {
            out.push_str("}\n");
        }
    }
    if let Some(part) = &aggregate.additional {
        let i = aggregate.fields.len();
        out.push_str("for (const auto& [key, value] : input.extra) {\n");
        if !aggregate.fields.is_empty() {
            writeln!(out,"if ({}) detail::http_fail(SdkError::Kind::RequestRepresentation, source, \"extra part collides with declared member\");",aggregate.fields.iter().map(|p|format!("key == {}",string(p.name.as_ref().unwrap()))).collect::<Vec<_>>().join(" || ")).unwrap();
        }
        out.push_str("names.insert(key);\n");
        if part.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
            writeln!(out,"detail::part_count({}, value.size()); for (const auto& item : value) encode_{name}_part{i}(item, key, context, settings, parts, form);",wire::part(plan,part)).unwrap();
        } else {
            writeln!(
                out,
                "encode_{name}_part{i}(value, key, context, settings, parts, form);"
            )
            .unwrap();
        }
        out.push_str("}\n");
    }
    writeln!(
        out,
        "detail::aggregate_rules({}, names);",
        wire::rules(plan, &aggregate.rules)
    )
    .unwrap();
    if aggregate.multipart {
        out.push_str("return detail::encode_multipart(std::move(parts), std::move(content_type), context, settings, source);\n}\n");
    } else {
        out.push_str("if (!form.empty() && 1 + static_cast<std::size_t>(std::count(form.begin(), form.end(), '&')) > settings.max_parts) detail::http_fail(SdkError::Kind::ResourceLimit, source, \"form field count exceeds ceiling\");\nreturn {std::move(form), std::move(content_type)};\n}\n");
    }
    writeln!(out,"{name} decode_{name}(std::string_view bytes, std::string_view content_type, detail::Context& context, const detail::Settings& settings) {{\n    (void)content_type;\n    const auto source = {source};\n    auto parts = {};\n    std::map<std::string, std::vector<detail::RawPart>, std::less<>> groups; std::set<std::string> names;\n    for (auto& part : parts) {{ names.insert(part.name); groups[part.name].push_back(std::move(part)); }}\n    detail::aggregate_rules({}, names);",if aggregate.multipart{"detail::decode_multipart(bytes, content_type, context, settings, source)"}else{"detail::decode_form(bytes, context, settings, source)"},wire::rules(plan,&aggregate.rules)).unwrap();
    for (i, part) in aggregate.fields.iter().enumerate() {
        let ty = field_type(part);
        writeln!(
            out,
            "Presence<{ty}> field{i};\nif (auto found = groups.find({}); found != groups.end()) {{",
            string(part.name.as_ref().unwrap())
        )
        .unwrap();
        if part.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
            writeln!(out,"detail::part_count({}, found->second.size());\n{ty} values; for (const auto& piece : found->second) values.push_back(decode_{name}_part{i}({{piece}}, context, settings));\nfield{i} = std::move(values);",wire::part(plan,part)).unwrap();
        } else {
            writeln!(
                out,
                "field{i} = decode_{name}_part{i}(found->second, context, settings);"
            )
            .unwrap();
        }
        out.push_str("groups.erase(found);\n}\n");
        if part.wire.required() {
            writeln!(out,"if (!field{i}) detail::http_fail(SdkError::Kind::ResponseDecoding, source, \"required aggregate field is absent\");").unwrap();
        }
    }
    let args = aggregate
        .fields
        .iter()
        .enumerate()
        .filter(|(_, p)| p.wire.required())
        .map(|(i, _)| format!("std::move(*field{i})"))
        .collect::<Vec<_>>();
    writeln!(
        out,
        "{name} result{};",
        if args.is_empty() {
            String::new()
        } else {
            format!("({})", args.join(", "))
        }
    )
    .unwrap();
    for (i, part) in aggregate
        .fields
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.wire.required())
    {
        writeln!(out, "result.{} = std::move(field{i});", part.field_name).unwrap();
    }
    if let Some(part) = &aggregate.additional {
        let i = aggregate.fields.len();
        out.push_str("for (const auto& [key, group] : groups) {\n");
        if part.wire.multiplicity() == w::PartMultiplicity::RepeatedArrayItems {
            writeln!(out,"detail::part_count({}, group.size());\n{} values; for (const auto& piece : group) values.push_back(decode_{name}_part{i}({{piece}}, context, settings)); result.extra.emplace(key, std::move(values));",wire::part(plan,part),field_type(part)).unwrap();
        } else {
            writeln!(
                out,
                "result.extra.emplace(key, decode_{name}_part{i}(group, context, settings));"
            )
            .unwrap();
        }
        out.push_str("}\n");
    } else {
        out.push_str("if (!groups.empty()) detail::http_fail(SdkError::Kind::ResponseDecoding, source, \"undeclared form/multipart property\");\n");
    }
    out.push_str("return result;\n}\n");
    out
}
