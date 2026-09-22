//! Package rendering from the immutable native and owned-instruction plans.

use super::models::{Extras, ModelSymbol, Shape};
use super::{HttpDiagnostic, SdkConfig, SdkPlan};
use crate::{OutFile, examples::ExampleOrigin};
use std::fmt::Write;
use suspect_ir::contract::{Contract, SourceId};
use suspect_schema::{
    OwnedProgram, PatternState, ProgramCountTarget, ProgramInstruction, ProgramSource, ProgramType,
};

mod aggregates;
mod credential_env;
pub(in crate::cpp_sdk) mod http;
mod native_example;
mod wire;

use super::pagination;
use super::oauth;
use super::stream_events;
use super::incoming;

pub(super) fn package(plan: &SdkPlan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    let native_example = native_example::example(plan);
    let mut files = Vec::new();
    let mut add = |path: String, content: String| {
        files.push(OutFile {
            path: format!("cpp/{path}"),
            content,
        })
    };
    let name = &plan.config.name;
    for (path, source) in [
        (
            format!("include/{name}/runtime.hpp"),
            include_str!("runtime.hpp"),
        ),
        ("src/runtime.cpp".into(), include_str!("runtime.cpp")),
        ("src/number.hpp".into(), include_str!("number.hpp")),
        ("src/validation.cpp".into(), include_str!("validation.cpp")),
        (
            "src/validation_v2.cpp".into(),
            include_str!("validation_v2.cpp"),
        ),
        (
            "src/validation_v3.cpp".into(),
            include_str!("validation_v3.cpp"),
        ),
        (format!("include/{name}/http.hpp"), include_str!("http.hpp")),
        (
            format!("include/{name}/protocol.hpp"),
            include_str!("protocol.hpp"),
        ),
        (
            format!("include/{name}/stream.hpp"),
            include_str!("stream.hpp"),
        ),
        ("src/http.cpp".into(), include_str!("http.cpp")),
        ("src/wire.cpp".into(), include_str!("wire.cpp")),
        ("src/payload.cpp".into(), include_str!("payload.cpp")),
        ("src/stream.cpp".into(), include_str!("stream.cpp")),
        (
            "src/curl_transport.cpp".into(),
            include_str!("curl_transport.cpp"),
        ),
    ] {
        add(path, expand(plan, source));
    }
    add(
        format!("include/{name}/attribution.hpp"),
        attribution_header(plan),
    );
    add(format!("include/{name}/models.hpp"), model_header(plan));
    add("src/models.cpp".into(), model_source(plan));
    add(
        "src/program.cpp".into(),
        validation_program(&plan.contract, &plan.program, &plan.config),
    );
    add(format!("include/{name}/client.hpp"), http::header(plan));
    add("src/client.cpp".into(), http::source(plan));
    if let Some(paginated) = plan.pagination().filter(|plan| plan.emits()) {
        add(
            format!("include/{name}/pagination.hpp"),
            pagination::header(plan, paginated),
        );
    }
    if plan.stream_events().emits() {
        add(
            format!("include/{name}/stream_events.hpp"),
            stream_events::header(plan, plan.stream_events()),
        );
    }
    if let Some(incoming) = plan.incoming() {
        add(
            format!("include/{name}/incoming.hpp"),
            incoming::header(plan, incoming),
        );
    }
    if let Some(oauth) = plan.oauth().filter(|plan| plan.emits()) {
        add(format!("include/{name}/oauth.hpp"), oauth::header(plan, oauth));
    }
    add(
        format!("include/{name}/sdk.hpp"),
        format!(
            "#pragma once\n/** @file sdk.hpp Public SDK entrypoint. */\n#include \"{name}/client.hpp\"\n"
        ),
    );
    add(
        "CMakeLists.txt".into(),
        expand(plan, include_str!("CMakeLists.txt")),
    );
    add(
        format!("cmake/{name}Config.cmake.in"),
        expand(plan, include_str!("config.cmake.in")),
    );
    add("Doxyfile".into(), expand(plan, include_str!("Doxyfile")));
    add("README.md".into(), readme(plan, native_example.as_ref()));
    add("docs/reference.md".into(), reference(plan));
    add("docs/coverage.json".into(), coverage(plan));
    if let Some(policy) = plan.credential_env() {
        add(
            "docs/credential-env.json".into(),
            serde_json::to_string_pretty(policy).unwrap() + "\n",
        );
    }
    add(
        "docs/protocol-plan.json".into(),
        serde_json::to_string_pretty(plan.protocol()).unwrap() + "\n",
    );
    add(
        "docs/validation-program.json".into(),
        serde_json::to_string_pretty(&plan.program).unwrap() + "\n",
    );
    if plan.examples.operations().iter().any(|op| {
        op.entries
            .iter()
            .any(|entry| plan.models.symbol(&entry.schema).is_some())
    }) {
        add("examples/validated_examples.cpp".into(), examples(plan));
    }
    if let Some(example) = native_example {
        add("examples/client.cpp".into(), example.source);
    }
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for (uri, document) in plan.contract.documents() {
        digest.update(uri.to_string());
        digest.update([0]);
        digest.update(serde_json::to_vec(document).unwrap());
        digest.update([0]);
    }
    let mut manifest = serde_json::json!({
        "format":"suspect.sdk.cpp.v1", "generator":env!("CARGO_PKG_VERSION"),
        "package":name, "version":plan.config.version, "namespace":plan.config.namespace,
        "language":"C++20", "json":"generated-exact-symbolic-decimal-v1", "transport":"libcurl >= 7.85",
        "openapi":plan.contract.openapi_version(), "contractDigest":format!("{:x}", digest.finalize()),
        "protocolProfile":plan.protocol.capabilities().adapter(),
        "compatibilityProfiles":plan.protocol.capabilities().profiles(),
        "operations":plan.operations.iter().map(|o| serde_json::json!({"document":o.source.document().to_string(),"pointer":o.source.pointer(),"operationId":o.operation_id,"method":o.method_name})).collect::<Vec<_>>(),
        "policy":{"redirects":false,"environmentProxies":false,"netrc":false,"retries":0,"tlsVerification":true,
            "maxRequestBytes":plan.config.max_request_bytes,"maxResponseBytes":plan.config.max_response_bytes,
            "maxCaptureBytes":plan.config.max_capture_bytes,"maxHeaderBytes":plan.config.max_header_bytes,
            "maxPartBytes":plan.config.max_part_bytes,"maxParts":plan.config.max_parts,
            "maxStreamItemBytes":plan.config.max_stream_item_bytes,"maxStreamBufferBytes":plan.config.max_stream_buffer_bytes}
    });
    if let Some(policy) = plan.credential_env() {
        manifest["credential_env"] = serde_json::to_value(policy.semantic_descriptor()).unwrap();
    }
    add(
        "sdk-manifest.json".into(),
        serde_json::to_string_pretty(&manifest).unwrap() + "\n",
    );
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn expand(plan: &SdkPlan, source: &str) -> String {
    expand_config(&plan.config, source)
}
fn expand_config(c: &SdkConfig, source: &str) -> String {
    source
        .replace("@NAMESPACE@", &c.namespace)
        .replace("@PACKAGE@", &c.name)
        .replace("@VERSION@", &c.version)
        .replace(
            "@JSON_BYTES@",
            &c.max_request_bytes.max(c.max_response_bytes).to_string(),
        )
        .replace("@JSON_DEPTH@", &c.max_json_depth.to_string())
        .replace("@JSON_WORK@", &c.max_json_work.to_string())
        .replace("@SCHEMA_DEPTH@", &c.validation.max_depth.to_string())
        .replace("@NUMBER_BYTES@", &c.validation.max_number_bytes.to_string())
        .replace(
            "@EQUALITY_WORK@",
            &c.validation.max_equality_steps.to_string(),
        )
        .replace(
            "@SCHEMA_WORK@",
            &c.validation.max_evaluation_steps.to_string(),
        )
        .replace("@REQUEST_BYTES@", &c.max_request_bytes.to_string())
        .replace("@RESPONSE_BYTES@", &c.max_response_bytes.to_string())
        .replace("@CAPTURE_BYTES@", &c.max_capture_bytes.to_string())
        .replace("@HEADER_BYTES@", &c.max_header_bytes.to_string())
        .replace("@PART_BYTES@", &c.max_part_bytes.to_string())
        .replace("@PARTS@", &c.max_parts.to_string())
        .replace("@STREAM_ITEM_BYTES@", &c.max_stream_item_bytes.to_string())
        .replace(
            "@STREAM_BUFFER_BYTES@",
            &c.max_stream_buffer_bytes.to_string(),
        )
}

/// ua/v1 attribution constants compiled at generation time into one emitted
/// header; the absent descriptor lowers to the disabled sentinel so the runtime
/// keeps one resolution path. The suspect version is the enable bit.
fn attribution_header(plan: &SdkPlan) -> String {
    let (comment, constants): (&str, Vec<(&str, String)>) = match &plan.config.attribution {
        Some(attribution) => (
            "// ua/v1 attribution: every request identifies suspect as the generator and the\n// SDK or a caller-supplied application as the client.\n",
            vec![
                ("attribution_template_version", "\"v1\"".into()),
                ("attribution_suspect_version", constexpr_string(&attribution.suspect_version)),
                ("attribution_sdk_name", constexpr_string(&attribution.sdk_name)),
                ("attribution_sdk_version", constexpr_string(&attribution.sdk_version)),
                ("attribution_spec_version", constexpr_string(&attribution.spec_version)),
                ("attribution_language", constexpr_string(&attribution.language)),
            ],
        ),
        None => (
            "// An empty suspect version disables the automatic attribution header.\n",
            vec![
                ("attribution_template_version", "\"\"".into()),
                ("attribution_suspect_version", "\"\"".into()),
                ("attribution_sdk_name", "\"\"".into()),
                ("attribution_sdk_version", "\"\"".into()),
                ("attribution_spec_version", "\"\"".into()),
                ("attribution_language", "\"\"".into()),
            ],
        ),
    };
    let mut out = format!(
        "#pragma once\n/** @file attribution.hpp ua/v1 attribution constants compiled at generation time. */\n#include <string_view>\n\n{comment}namespace {}::detail {{\n",
        plan.config.namespace
    );
    for (name, value) in constants {
        let _ = writeln!(out, "inline constexpr std::string_view {name} = {value};");
    }
    let _ = writeln!(out, "}} // namespace {}::detail", plan.config.namespace);
    out
}

/// Quoted C++ string literal bytes; every escape denotes exactly one byte, so a
/// `std::string_view` carries the exact length including embedded NUL bytes.
/// Three-digit octal escapes cannot absorb adjacent digits.
fn constexpr_string(value: &str) -> String {
    let mut literal = String::new();
    for byte in value.bytes() {
        match byte {
            b'"' => literal.push_str("\\\""),
            b'\\' => literal.push_str("\\\\"),
            32..=126 => literal.push(byte as char),
            _ => write!(literal, "\\{byte:03o}").unwrap(),
        }
    }
    format!("std::string_view(\"{literal}\", {})", value.len())
}

/// C++ string expression with explicit UTF-8 length. Three-digit octal escapes
/// cannot absorb adjacent hex digits; embedded NUL and hostile source prose are
/// data, including in Source IDs, enum literals and JSON member names.
pub(super) fn string(value: &str) -> String {
    if value.len() > 16384 {
        // Runtime concatenation avoids the standard's string-literal translation
        // limit, even for long source literals, names and documentation values.
        let mut parts = Vec::new();
        let mut start = 0;
        while start < value.len() {
            let mut end = (start + 16384).min(value.len());
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            parts.push(string(&value[start..end]));
            start = end;
        }
        return parts.join(" + ");
    }
    let mut literal = String::new();
    for byte in value.bytes() {
        match byte {
            b'"' => literal.push_str("\\\""),
            b'\\' => literal.push_str("\\\\"),
            32..=126 => literal.push(byte as char),
            _ => {
                write!(literal, "\\{byte:03o}").unwrap();
            }
        }
    }
    format!("std::string(\"{literal}\", {})", value.len())
}
pub(super) fn prose(value: &str) -> String {
    let escaped = value
        .replace('&', "&amp;")
        .replace('#', "&#35;")
        .replace('%', "&#37;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "&#92;")
        .replace('@', "&#64;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('`', "&#96;");
    escaped
        .chars()
        .map(|c| {
            if (c.is_control() && c != '\n') || matches!(c, '\u{2028}' | '\u{2029}') {
                format!("&#{};", c as u32)
            } else {
                c.to_string()
            }
        })
        .collect()
}
pub(super) fn single_line(value: &str) -> String {
    prose(value).replace('\n', "&#10;")
}
fn comment(plan: &SdkPlan, symbol: &ModelSymbol) -> String {
    let mut out = format!(
        "/// Source: {}#{}\n",
        single_line(&symbol.source.document().to_string()),
        single_line(symbol.source.pointer())
    );
    if let Some(text) = plan
        .contract
        .source(&symbol.source)
        .and_then(|s| s.get("description"))
        .and_then(|s| s.as_str())
    {
        for line in prose(text).lines() {
            writeln!(out, "/// {line}").unwrap();
        }
    }
    if plan
        .contract
        .source(&symbol.source)
        .and_then(|s| s.get("deprecated"))
        .and_then(|s| s.as_bool())
        == Some(true)
    {
        out.push_str("/// @deprecated Deprecated in the source contract.\n");
    }
    out
}

fn model_header(plan: &SdkPlan) -> String {
    let mut out = format!(
        "#pragma once\n/** @file models.hpp Source-derived value types and codecs. */\n#include \"{}/runtime.hpp\"\n\nnamespace {} {{\n",
        plan.config.name, plan.config.namespace
    );
    for s in plan.models.symbols() {
        if s.has_definition() {
            writeln!(
                out,
                "{} {};",
                if matches!(s.shape, Shape::Enum(_)) {
                    "enum class"
                } else {
                    "struct"
                },
                s.definition
            )
            .unwrap();
        }
    }
    for id in &plan.models.order {
        let s = plan.models.get(id);
        out.push_str(&comment(plan, s));
        match &s.shape {
            Shape::Enum(values) => {
                writeln!(out, "enum class {} {{", s.definition).unwrap();
                for (wire, name) in values {
                    writeln!(
                        out,
                        "    /// Wire value: {}\n    {name},",
                        single_line(wire)
                    )
                    .unwrap();
                }
                out.push_str("};\n");
            }
            Shape::Object { fields, extras } => {
                writeln!(out, "struct {} {{", s.definition).unwrap();
                for f in fields {
                    let child = plan.models.get(&f.schema);
                    for line in comment(plan, child).lines() {
                        writeln!(out, "    {line}").unwrap();
                    }
                    let initializer = if !f.required {
                        " = std::nullopt".into()
                    } else if let Some(tag) = &f.initializer {
                        format!(" = {}::{}", tag.type_name, tag.case_name)
                    } else {
                        String::new()
                    };
                    writeln!(out, "    {} {}{initializer};", f.cpp_type, f.name).unwrap();
                }
                match extras {
                    Extras::Closed => {}
                    Extras::Any => out.push_str("    /// Undeclared members, preserved exactly. Declared-key collisions are errors.\n    std::map<std::string, JsonValue, std::less<>> extra;\n"),
                    Extras::Patterned => out.push_str("    /// Exact extra values; the whole-object codec checks overlapping patterns and the additional/unevaluated policies.\n    std::map<std::string, JsonValue, std::less<>> extra;\n"),
                    Extras::Typed(id) => { writeln!(out, "    /// Typed additionalProperties; declared-key collisions are errors.\n    std::map<std::string, {}, std::less<>> extra;", plan.models.get(id).cpp_type).unwrap(); }
                }
                let constructor = s
                    .constructor
                    .as_ref()
                    .expect("object constructor descriptor");
                if constructor.parameters.is_empty() {
                    writeln!(out, "    {}() = default;", s.definition).unwrap();
                } else {
                    writeln!(out, "    /// Required values; source-proved singleton tags are initialized automatically.\n    explicit {}({}) : {} {{}}", constructor.name,
                        constructor.parameters.iter().map(|p|format!("{} {}", p.cpp_type, p.name)).collect::<Vec<_>>().join(", "),
                        constructor.parameters.iter().map(|p|format!("{}(std::move({}))", p.member_name, p.name)).collect::<Vec<_>>().join(", ")).unwrap();
                }
                out.push_str("};\n");
            }
            Shape::Union {
                branches,
                exactly_one,
            } => {
                writeln!(out, "/// {} validation runs across every alternative, with shared budgets.\nstruct {} {{", if *exactly_one { "Exactly-one" } else { "Inclusive anyOf" }, s.definition).unwrap();
                for (i, branch) in branches.iter().enumerate() {
                    writeln!(
                        out,
                        "    using Alternative{i} = {};",
                        plan.models.get(branch).cpp_type
                    )
                    .unwrap();
                }
                writeln!(out, "    using Variant = std::variant<{}>;\n    Variant value;\n    explicit {}(Variant selected) : value(std::move(selected)) {{}}", (0..branches.len()).map(|i| format!("Alternative{i}")).collect::<Vec<_>>().join(", "), s.definition).unwrap();
                for i in 0..branches.len() {
                    writeln!(out, "    /// Select the source alternative at index {i}.\n    static {} alternative_{i}(Alternative{i} value) {{ return {}(Variant(std::in_place_index<{i}>, std::move(value))); }}", s.definition, s.definition).unwrap();
                }
                out.push_str("};\n");
            }
            _ => {}
        }
    }
    // Aliases occur after complete value definitions. Fields refer directly to
    // proved native types, avoiding order-dependent recursive alias expansion.
    for s in plan.models.symbols() {
        if !s.has_definition() || s.nullable {
            writeln!(
                out,
                "/// Native view of {}\nusing {} = {};",
                single_line(s.source.pointer()),
                s.name,
                s.cpp_type
            )
            .unwrap();
        }
    }
    out.push_str("namespace detail {\n");
    for s in plan.models.symbols() {
        writeln!(out, "{} decode_{}(const JsonValue&, Context&, const std::string&);\nJsonValue encode_{}(const {}&, Context&, const std::string&);", s.cpp_type, s.index, s.index, s.cpp_type).unwrap();
    }
    out.push_str("}\n");
    for s in plan.models.symbols() {
        out.push_str(&comment(plan, s));
        let codec = s.codec();
        let value = codec.value_alias;
        let error = codec.error_type;
        let methods = codec.methods;
        writeln!(out, "struct {} {{\n    using {value} = {};\n    /// Strict source-validated decoding. Invalid JSON and evaluation failure are distinct.\n    static Result<{value}, {error}> {}(std::string_view bytes);\n    /// Revalidate the current, possibly mutated, native value before writing.\n    static Result<std::string, {error}> {}(const {value}& value);\n    /// Validated exact JSON value for transport adapters.\n    static Result<JsonValue, {error}> {}(const {value}& value);\n}};", codec.owner, codec.cpp_type, methods.decode, methods.encode, methods.to_json).unwrap();
    }
    writeln!(out, "}} // namespace {}", plan.config.namespace).unwrap();
    out
}

fn model_source(plan: &SdkPlan) -> String {
    let mut out = format!(
        "#include \"{}/models.hpp\"\n\nnamespace {} {{\nnamespace detail {{\n",
        plan.config.name, plan.config.namespace
    );
    for s in plan.models.symbols() {
        writeln!(out, "{} decode_{}(const JsonValue&, Context&, const std::string&);\nJsonValue encode_{}(const {}&, Context&, const std::string&);", s.cpp_type, s.index, s.index, s.cpp_type).unwrap();
    }
    for s in plan.models.symbols() {
        let definition = format!("::{}::{}", plan.config.namespace, s.definition);
        writeln!(out, "{} decode_{}(const JsonValue& json, Context& context, const std::string& path) {{\n    (void)json;\n    const auto& source = validation_program().nodes[{}].source;\n    auto guard = context.enter(source, path);", s.cpp_type, s.index, s.index).unwrap();
        if s.nullable {
            writeln!(
                out,
                "    if (json.is<Null>()) return {}(std::in_place_index<0>);",
                s.cpp_type
            )
            .unwrap();
        }
        let ret = |expr: String| {
            if s.nullable {
                format!("return {}(std::in_place_index<1>, {expr});", s.cpp_type)
            } else if expr == "std::move(result)" {
                "return result;".into()
            } else {
                format!("return {expr};")
            }
        };
        match &s.shape {
            Shape::Json | Shape::ValidatedJson => writeln!(out, "    {}", ret("context.copy(json, source, path)".into())).unwrap(),
            Shape::Never => out.push_str("    fail(CodecError::Kind::Model, source, path, \"false schema has no native values\");\n"),
            Shape::Null => out.push_str("    as<Null>(json, source, path); return Null{};\n"),
            Shape::Boolean => writeln!(out, "    {}", ret("as<bool>(json, source, path)".into())).unwrap(),
            Shape::Number => writeln!(out, "    context.spend(as<JsonNumber>(json, source, path).token().size(), source, path);\n    {}", ret("as<JsonNumber>(json, source, path)".into())).unwrap(),
            Shape::Integer => { writeln!(out, "    context.spend(as<JsonNumber>(json, source, path).token().size(), source, path);\n    auto integer = JsonInteger::from_number(as<JsonNumber>(json, source, path));\n    if (!integer) fail(CodecError::Kind::Model, source, path, \"nonintegral native integer\");\n    {}", ret("std::move(integer).value()".into())).unwrap(); }
            Shape::String => writeln!(out, "    {}", ret("context.text(as<std::string>(json, source, path), source, path)".into())).unwrap(),
            Shape::Enum(values) => {
                out.push_str("    const auto& text = as<std::string>(json, source, path);\n");
                for (wire, name) in values { writeln!(out, "    if (text == {}) {}", string(wire), ret(format!("{definition}::{name}"))).unwrap(); }
                out.push_str("    fail(CodecError::Kind::Model, source, path, \"unrecognized enum value\");\n");
            }
            Shape::Ref { target, boxed } => {
                let t = plan.models.get(target);
                let value = format!("decode_{}(json, context, path)", t.index);
                writeln!(out, "    {}", ret(if *boxed { format!("Box<{}>({value})", t.cpp_type) } else { value })).unwrap();
            }
            Shape::Array(item) => {
                let ty = item.as_ref().map(|id| plan.models.get(id).cpp_type.as_str()).unwrap_or("JsonValue");
                writeln!(out, "    std::vector<{ty}> result;\n    std::size_t index = 0;\n    for (const auto& item : as<JsonValue::Array>(json, source, path)) {{\n        auto at = child_path(path, std::to_string(index++));").unwrap();
                writeln!(out, "        result.push_back({});\n    }}\n    {}", item.as_ref().map(|id| format!("decode_{}(item, context, at)", plan.models.get(id).index)).unwrap_or_else(|| "context.copy(item, source, at)".into()), ret("std::move(result)".into())).unwrap();
            }
            Shape::Object { fields, extras } => {
                out.push_str("    const auto& object = as<JsonValue::Object>(json, source, path);\n    (void)object;\n");
                for (i, f) in fields.iter().enumerate().filter(|(_, f)| f.required && f.initializer.is_none()) {
                    let child = plan.models.get(&f.schema);
                    writeln!(out, "    auto field{i} = decode_{}(member(object, {}, source, path), context, child_path(path, {}));", child.index, string(&f.wire), string(&f.wire)).unwrap();
                }
                let args = fields.iter().enumerate().filter(|(_, f)| f.required && f.initializer.is_none()).map(|(i, _)| format!("std::move(field{i})")).collect::<Vec<_>>().join(", ");
                writeln!(out, "    {definition} result{};", if args.is_empty() { String::new() } else { format!("({args})") }).unwrap();
                for f in fields.iter().filter(|f| !f.required) {
                    writeln!(out, "    if (auto it = object.find({}); it != object.end()) result.{} = decode_{}(it->second, context, child_path(path, {}));", string(&f.wire), f.name, plan.models.get(&f.schema).index, string(&f.wire)).unwrap();
                }
                if !matches!(extras, Extras::Closed) {
                    out.push_str("    for (const auto& [key, item] : object) {\n");
                    if !fields.is_empty() { writeln!(out, "        if ({}) continue;", fields.iter().map(|f| format!("key == {}", string(&f.wire))).collect::<Vec<_>>().join(" || ")).unwrap(); }
                    let value = match extras { Extras::Typed(id) => format!("decode_{}(item, context, child_path(path, key))", plan.models.get(id).index), _ => "context.copy(item, source, child_path(path, key))".into() };
                    writeln!(out, "        result.extra.emplace(context.text(key, source, path), {value});\n    }}").unwrap();
                }
                writeln!(out, "    {}", ret("std::move(result)".into())).unwrap();
            }
            Shape::Union { branches, .. } => {
                for (i, branch) in branches.iter().enumerate() {
                    let t = plan.models.get(branch);
                    writeln!(out, "    if (context.validation.matches({}, json, path)) {}", t.index, ret(format!("{definition}::alternative_{i}(decode_{}(json, context, path))", t.index))).unwrap();
                }
                out.push_str("    fail(CodecError::Kind::Model, source, path, \"no native union alternative matches\");\n");
            }
        }
        out.push_str("}\n");
        writeln!(out, "JsonValue encode_{}(const {}& input, Context& context, const std::string& path) {{\n    const auto& source = validation_program().nodes[{}].source;\n    auto guard = context.enter(source, path);", s.index, s.cpp_type, s.index).unwrap();
        if s.nullable {
            out.push_str("    if (input.valueless_by_exception()) fail(CodecError::Kind::Model, source, path, \"valueless nullable variant\");\n    if (input.index() == 0) return JsonValue(Null{});\n    const auto& value = std::get<1>(input);\n");
        } else {
            out.push_str("    const auto& value = input;\n");
        }
        out.push_str("    (void)value;\n");
        match &s.shape {
            Shape::Json | Shape::ValidatedJson => out.push_str("    return context.copy(value, source, path);\n"),
            Shape::Never => out.push_str("    fail(CodecError::Kind::Model, source, path, \"false schema has no native values\");\n"),
            Shape::Null | Shape::Boolean => out.push_str("    return JsonValue(value);\n"),
            Shape::Number | Shape::Integer => out.push_str("    context.spend(value.token().size(), source, path);\n    return JsonValue(value);\n"),
            Shape::String => out.push_str("    return JsonValue(context.text(value, source, path));\n"),
            Shape::Enum(values) => {
                out.push_str("    switch (value) {\n");
                for (wire, name) in values { writeln!(out, "        case {definition}::{name}: return JsonValue({});", string(wire)).unwrap(); }
                out.push_str("    }\n    fail(CodecError::Kind::Model, source, path, \"invalid native enum discriminant\");\n");
            }
            Shape::Ref { target, boxed } => {
                if *boxed { out.push_str("    if (!value.has_value()) fail(CodecError::Kind::Model, source, path, \"moved-from recursive Box\");\n"); }
                writeln!(out, "    return encode_{}({}, context, path);", plan.models.get(target).index, if *boxed { "value.value()" } else { "value" }).unwrap();
            }
            Shape::Array(item) => {
                out.push_str("    JsonValue::Array result; std::size_t index = 0;\n    for (const auto& item : value) {\n        auto at = child_path(path, std::to_string(index++));\n");
                writeln!(out, "        result.push_back({});\n    }}\n    return JsonValue(std::move(result));", item.as_ref().map(|id| format!("encode_{}(item, context, at)", plan.models.get(id).index)).unwrap_or_else(|| "context.copy(item, source, at)".into())).unwrap();
            }
            Shape::Object { fields, extras } => {
                out.push_str("    JsonValue::Object result;\n");
                for f in fields {
                    let v = if f.required { format!("value.{}", f.name) } else { format!("*value.{}", f.name) };
                    writeln!(out, "    {}result.emplace({}, encode_{}({v}, context, child_path(path, {})));", if f.required { String::new() } else { format!("if (value.{}) ", f.name) }, string(&f.wire), plan.models.get(&f.schema).index, string(&f.wire)).unwrap();
                }
                if !matches!(extras, Extras::Closed) {
                    out.push_str("    for (const auto& [key, item] : value.extra) {\n");
                    if !fields.is_empty() { writeln!(out, "        if ({}) fail(CodecError::Kind::Model, source, child_path(path, key), \"extra key collides with a declared member, including an absent member\");", fields.iter().map(|f| format!("key == {}", string(&f.wire))).collect::<Vec<_>>().join(" || ")).unwrap(); }
                    let value = match extras { Extras::Typed(id) => format!("encode_{}(item, context, child_path(path, key))", plan.models.get(id).index), _ => "context.copy(item, source, child_path(path, key))".into() };
                    writeln!(out, "        result.emplace(context.text(key, source, path), {value});\n    }}").unwrap();
                }
                out.push_str("    return JsonValue(std::move(result));\n");
            }
            Shape::Union { branches, .. } => {
                out.push_str("    if (value.value.valueless_by_exception()) fail(CodecError::Kind::Model, source, path, \"valueless native union\");\n    JsonValue result;\n    switch (value.value.index()) {\n");
                for (i, branch) in branches.iter().enumerate() {
                    let t = plan.models.get(branch);
                    writeln!(out, "        case {i}: result = encode_{}(std::get<{i}>(value.value), context, path); context.validation.require({}, result, path); break;", t.index, t.index).unwrap();
                }
                out.push_str("        default: fail(CodecError::Kind::Model, source, path, \"invalid union index\");\n    }\n    return result;\n");
            }
        }
        out.push_str("}\n");
    }
    out.push_str("} // namespace detail\n");
    for s in plan.models.symbols() {
        let codec = s.codec();
        let owner = codec.owner;
        let value = codec.value_alias;
        let error = codec.error_type;
        let methods = codec.methods;
        writeln!(out, "Result<{owner}::{value}, {error}> {owner}::{}(std::string_view bytes) {{ return detail::decode_model<{value}>({}, bytes, detail::decode_{}); }}\nResult<std::string, {error}> {owner}::{}(const {value}& value) {{ return detail::encode_bytes({}, value, detail::encode_{}); }}\nResult<JsonValue, {error}> {owner}::{}(const {value}& value) {{ return detail::encode_model({}, value, detail::encode_{}); }}", methods.decode, s.index, s.index, methods.encode, s.index, s.index, methods.to_json, s.index, s.index).unwrap();
    }
    writeln!(out, "}} // namespace {}", plan.config.namespace).unwrap();
    out
}

pub(super) fn source_expr(plan: &SdkPlan, source: &SourceId) -> String {
    let span = plan.contract.source_span(source).unwrap_or(0..0);
    format!(
        "Source{{{}, {}, {}, {}}}",
        string(&source.document().to_string()),
        string(source.pointer()),
        span.start,
        span.end
    )
}
fn program_source(contract: &Contract, source: &ProgramSource) -> String {
    let span = contract
        .documents()
        .find(|(uri, _)| uri.to_string() == source.document)
        .and_then(|(uri, _)| {
            let mut id = SourceId::new(uri.clone(), Default::default());
            for token in source.pointer.split('/').skip(1) {
                id = id.child(&token.replace("~1", "/").replace("~0", "~"));
            }
            contract.source_span(&id)
        })
        .unwrap_or(0..0);
    format!(
        "Source{{{}, {}, {}, {}}}",
        string(&source.document),
        string(&source.pointer),
        span.start,
        span.end
    )
}
fn validation_program(contract: &Contract, program: &OwnedProgram, config: &SdkConfig) -> String {
    validation_program_named(contract, program, config, "validation_program")
}
pub(super) fn validation_program_named(
    contract: &Contract,
    program: &OwnedProgram,
    config: &SdkConfig,
    name: &str,
) -> String {
    let mut out = format!(
        "#include \"{}/runtime.hpp\"\n\nnamespace {}::detail {{\nconst Program& {name}() {{\n    static const Program program = [] {{\n        Program p;\n        p.nodes.resize({});\n",
        config.name,
        config.namespace,
        program.nodes.len()
    );
    let l = &program.limits;
    if program.version == OwnedProgram::V2_VERSION || program.version == OwnedProgram::V3_VERSION {
        writeln!(
            out,
            "        p.version = {}; p.profile = {};",
            string(program.version),
            string(program.profile)
        )
        .unwrap();
    }
    writeln!(out,"        p.max_depth = {}; p.max_number_bytes = {}; p.max_equality_steps = {}; p.max_evaluation_steps = {};",l.max_depth,l.max_number_bytes,l.max_equality_steps,l.max_evaluation_steps).unwrap();
    if let Some(context) = &program.resource_context {
        out.push_str("        p.resource_context.emplace();\n");
        for resource in &context.resources {
            writeln!(out,"        {{ SchemaResource resource; resource.source = {}; resource.kind = {}; resource.canonical_uri = {}; resource.base_uri = {}; resource.aliases = {{{}}};",program_source(contract,&resource.source),string(resource.kind),string(&resource.canonical_uri),string(&resource.base_uri),resource.aliases.iter().map(|v|string(v)).collect::<Vec<_>>().join(", ")).unwrap();
            if let Some(source) = &resource.declaration_source {
                writeln!(
                    out,
                    "          resource.declaration_source = {};",
                    program_source(contract, source)
                )
                .unwrap();
            }
            for (name, source, target) in &resource.dynamic_anchors {
                writeln!(out,"          resource.dynamic_anchors.push_back(DynamicBinding{{{}, {}, {target}}});",string(name),program_source(contract,source)).unwrap();
            }
            out.push_str(
                "          p.resource_context->resources.push_back(std::move(resource)); }\n",
            );
        }
        for (resource, root, address) in &context.node_scopes {
            writeln!(out,"        p.resource_context->node_scopes.push_back(NodeResourceScope{{{resource}, {}, {}}});",program_source(contract,root),string(address)).unwrap();
        }
    }
    for root in &program.roots {
        writeln!(out, "        p.roots.insert({});", root.target).unwrap();
    }
    for (i, node) in program.nodes.iter().enumerate() {
        writeln!(
            out,
            "        p.nodes[{i}].source = {};",
            program_source(contract, &node.source)
        )
        .unwrap();
        for check in &node.checks {
            writeln!(
                out,
                "        {{ Check c; c.source = {};",
                program_source(contract, &check.source)
            )
            .unwrap();
            let targets = |v: &[usize]| {
                v.iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let strings = |v: &[String]| v.iter().map(|v| string(v)).collect::<Vec<_>>().join(", ");
            #[allow(unreachable_patterns)] // check_program is the native opcode admission fence.
            match &check.instruction {
                ProgramInstruction::Always { value } => writeln!(out, "          c.op = Op::Always; c.flag = {value};").unwrap(),
                ProgramInstruction::Type { types } => {
                    let values: Vec<_> = types.iter().map(|ty| match ty { ProgramType::Null => "null", ProgramType::Boolean => "boolean", ProgramType::Number => "number", ProgramType::Integer => "integer", ProgramType::String => "string", ProgramType::Array => "array", ProgramType::Object => "object" }.to_owned()).collect();
                    writeln!(out, "          c.op = Op::Type; c.names = {{{}}};", strings(&values)).unwrap();
                }
                ProgramInstruction::Ref { target } => writeln!(out, "          c.op = Op::Ref; c.target = {target};").unwrap(),
                ProgramInstruction::DynamicRef {target,initial_resource,anchor}=>writeln!(out,"          c.op = Op::DynamicRef; c.target = {target}; c.initial_resource = {initial_resource}; c.dynamic_anchor = {};",anchor.as_ref().map(|v|string(v)).unwrap_or_else(||"std::nullopt".into())).unwrap(),
                ProgramInstruction::Properties { properties } => writeln!(out, "          c.op = Op::Properties; c.properties = {{{}}};", properties.iter().map(|p| format!("Property{{{}, {}}}", string(&p.name), p.target)).collect::<Vec<_>>().join(", ")).unwrap(),
                ProgramInstruction::AdditionalProperties { declared, target } => writeln!(out, "          c.op = Op::AdditionalProperties; c.names = {{{}}}; c.target = {target};", strings(declared)).unwrap(),
                ProgramInstruction::Required { names } => writeln!(out, "          c.op = Op::Required; c.names = {{{}}};", strings(names)).unwrap(),
                ProgramInstruction::Items { target, start } => writeln!(out, "          c.op = Op::Items; c.target = {target}; c.start = {start};").unwrap(),
                ProgramInstruction::PrefixItems { targets: v } | ProgramInstruction::AllOf { targets: v } | ProgramInstruction::AnyOf { targets: v } | ProgramInstruction::OneOf { targets: v } => {
                    let op = match &check.instruction { ProgramInstruction::PrefixItems { .. } => "PrefixItems", ProgramInstruction::AllOf { .. } => "AllOf", ProgramInstruction::AnyOf { .. } => "AnyOf", _ => "OneOf" };
                    writeln!(out, "          c.op = Op::{op}; c.targets = {{{}}};", targets(v)).unwrap();
                }
                ProgramInstruction::Not { target } => writeln!(out, "          c.op = Op::Not; c.target = {target};").unwrap(),
                ProgramInstruction::Bound { value, maximum, exclusive } => writeln!(out, "          c.op = Op::Bound; c.operand = {}; c.flag = {maximum}; c.exclusive = {exclusive};", string(value)).unwrap(),
                ProgramInstruction::MultipleOf { value } => writeln!(out, "          c.op = Op::MultipleOf; c.operand = {};", string(value)).unwrap(),
                ProgramInstruction::Count { value, maximum, target } => writeln!(out, "          c.op = Op::Count; c.names = {{{}}}; c.flag = {maximum}; c.operand = {};", string(value), string(match target { ProgramCountTarget::String => "string", ProgramCountTarget::Array => "array", ProgramCountTarget::Object => "object" })).unwrap(),
                ProgramInstruction::Enum { values } => writeln!(out, "          c.op = Op::Enum; c.literals = {{{}}};", values.iter().map(|v| format!("literal({})", string(&serde_json::to_string(v).unwrap()))).collect::<Vec<_>>().join(", ")).unwrap(),
                ProgramInstruction::Const { value } => writeln!(out, "          c.op = Op::Const; c.literals = {{literal({})}};", string(&serde_json::to_string(value).unwrap())).unwrap(),
                ProgramInstruction::UniqueItems => out.push_str("          c.op = Op::UniqueItems;\n"),
                ProgramInstruction::Pattern { program } => {
                    writeln!(out, "          c.op = Op::Pattern; c.pattern.start = {};", program.start).unwrap();
                    for state in &program.states {
                        let (kind,first,second,ranges) = match state {
                            PatternState::Match => ("Match",0,0,String::new()),
                            PatternState::Char { ranges,target } => ("Char",*target,0,ranges.iter().map(|r|format!("{{{}, {}}}",r[0],r[1])).collect::<Vec<_>>().join(", ")),
                            PatternState::Split { first,second } => ("Split",*first,*second,String::new()),
                            PatternState::Jump { target } => ("Jump",*target,0,String::new()),
                            PatternState::Start { target } => ("Start",*target,0,String::new()),
                            PatternState::End { target } => ("End",*target,0,String::new()),
                        };
                        writeln!(out,"          c.pattern.states.push_back(PatternState{{PatternState::Kind::{kind}, {first}, {second}, {{{ranges}}}}});").unwrap();
                    }
                }
                ProgramInstruction::If {condition,then_target,else_target} => {
                    writeln!(out,"          c.op = Op::If; c.target = {condition}; c.then_target = {}; c.else_target = {};",then_target.map(|v|v.to_string()).unwrap_or("std::nullopt".into()),else_target.map(|v|v.to_string()).unwrap_or("std::nullopt".into())).unwrap();
                }
                ProgramInstruction::DependentRequired {dependencies} => {
                    out.push_str("          c.op = Op::DependentRequired;\n");
                    for (trigger,names) in dependencies {
                        let source=super::locate(contract,&check.source).child(trigger);
                        writeln!(out,"          c.dependencies.emplace_back({}, std::vector<std::string>{{{}}});\n          c.dependency_sources.push_back({});",string(trigger),strings(names),program_source(contract,&ProgramSource::from(&source))).unwrap();
                    }
                }
                ProgramInstruction::DependentSchemas {dependencies} => writeln!(out,"          c.op = Op::DependentSchemas; c.properties = {{{}}};",dependencies.iter().map(|p|format!("Property{{{}, {}}}",string(&p.name),p.target)).collect::<Vec<_>>().join(", ")).unwrap(),
                ProgramInstruction::Contains {target,minimum,maximum} => {
                    writeln!(out,"          c.op = Op::Contains; c.target = {target};").unwrap();
                    for (field,key,value) in [("minimum","minContains",minimum),("maximum","maxContains",maximum)] {
                        if let Some(value)=value {let source=super::locate(contract,&node.source).child(key);writeln!(out,"          c.{field} = {}; c.{field}_source = {};",string(value),program_source(contract,&ProgramSource::from(&source))).unwrap();}
                    }
                }
                ProgramInstruction::PatternProperties {patterns} => {
                    out.push_str("          c.op = Op::PatternProperties;\n");
                    for (text,pattern,target) in patterns {
                        writeln!(out,"          {{ PropertyPattern rule; rule.text = {}; rule.target = {target}; rule.pattern.start = {};",string(text),pattern.start).unwrap();
                        for state in &pattern.states {
                            let (kind,first,second,ranges)=match state {
                                PatternState::Match=>("Match",0,0,String::new()),PatternState::Char{ranges,target}=>("Char",*target,0,ranges.iter().map(|r|format!("{{{}, {}}}",r[0],r[1])).collect::<Vec<_>>().join(", ")),
                                PatternState::Split{first,second}=>("Split",*first,*second,String::new()),PatternState::Jump{target}=>("Jump",*target,0,String::new()),PatternState::Start{target}=>("Start",*target,0,String::new()),PatternState::End{target}=>("End",*target,0,String::new())
                            };
                            writeln!(out,"            rule.pattern.states.push_back(PatternState{{PatternState::Kind::{kind}, {first}, {second}, {{{ranges}}}}});").unwrap();
                        }
                        out.push_str("            c.patterns.push_back(std::move(rule)); }\n");
                    }
                }
                ProgramInstruction::AdditionalPropertiesWithPatterns {declared,target}=>writeln!(out,"          c.op = Op::AdditionalPropertiesWithPatterns; c.names = {{{}}}; c.target = {target};",strings(declared)).unwrap(),
                ProgramInstruction::PropertyNames {target} | ProgramInstruction::UnevaluatedProperties {target} | ProgramInstruction::UnevaluatedItems {target}=>{
                    let op=match &check.instruction {ProgramInstruction::PropertyNames {..}=>"PropertyNames",ProgramInstruction::UnevaluatedProperties {..}=>"UnevaluatedProperties",_=>"UnevaluatedItems"};writeln!(out,"          c.op = Op::{op}; c.target = {target};").unwrap();
                }
                _ => unreachable!("unadmitted native validation opcode"),
            }
            writeln!(
                out,
                "          p.nodes[{i}].checks.push_back(std::move(c)); }}"
            )
            .unwrap();
        }
    }
    out.push_str("        return p;\n    }();\n    return program;\n}\n}\n");
    out
}

pub(super) fn validation_runtime(contract: &Contract, program: &OwnedProgram) -> Vec<OutFile> {
    let config = SdkConfig::default();
    let mut files: Vec<_> = [
        (
            "include/generated_sdk/runtime.hpp",
            include_str!("runtime.hpp"),
        ),
        ("src/runtime.cpp", include_str!("runtime.cpp")),
        ("src/number.hpp", include_str!("number.hpp")),
        ("src/validation.cpp", include_str!("validation.cpp")),
        ("src/validation_v2.cpp", include_str!("validation_v2.cpp")),
        ("src/validation_v3.cpp", include_str!("validation_v3.cpp")),
    ]
    .into_iter()
    .map(|(path, text)| OutFile {
        path: format!("cpp/{path}"),
        content: expand_config(&config, text),
    })
    .collect();
    files.push(OutFile {
        path: "cpp/src/program.cpp".into(),
        content: validation_program(contract, program, &config),
    });
    files
}

fn examples(plan: &SdkPlan) -> String {
    let mut out = format!(
        "/** @file Validated shared example-plan values, executed without a network. */\n#include \"{}/sdk.hpp\"\n#include <iostream>\nusing namespace {};\nint main() {{\n",
        plan.config.name, plan.config.namespace
    );
    for op in plan.examples.operations() {
        for entry in &op.entries {
            let Some(s) = plan.models.symbol(&entry.schema) else {
                continue;
            };
            writeln!(out, "    {{ // {} {}\n        auto value = {}::decode({});\n        if (!value) {{ std::cerr << value.error().source.pointer << \": \" << value.error().message; return 1; }}\n        auto encoded = {}::encode(value.value());\n        if (!encoded || !{}::decode(encoded.value())) return 2;\n    }}", if entry.origin == ExampleOrigin::Declared { "declared" } else { "synthesized" }, single_line(&op.operation_id), s.codec_name, string(&serde_json::to_string(&entry.value).unwrap()), s.codec_name, s.codec_name).unwrap();
        }
    }
    out.push_str("    return 0;\n}\n");
    out
}

fn coverage(plan: &SdkPlan) -> String {
    let value = serde_json::json!({
        "profile":"cpp20-libcurl-protocol-v2", "language":"C++20", "nativeReference":"Doxygen",
        "models":plan.models.symbols().map(|s| serde_json::json!({"name":s.name,"type":s.cpp_type,"codec":s.codec_name,"document":s.source.document().to_string(),"pointer":s.source.pointer(),"descriptionPresent":plan.contract.source(&s.source).and_then(|v|v.get("description")).and_then(|v|v.as_str()).is_some()})).collect::<Vec<_>>(),
        "operations":plan.operations.iter().map(|o| serde_json::json!({"id":o.declared_operation_id,"method":o.method_name,"input":o.input_type,"success":o.success_type,"error":o.error_type,"descriptionPresent":o.wire.description().is_some()})).collect::<Vec<_>>(),
        "examples":plan.examples.operations().iter().flat_map(|op| op.entries.iter().map(|e| serde_json::json!({"operation":op.operation_id,"schema":e.schema.pointer(),"origin":if e.origin==ExampleOrigin::Declared{"declared"}else{"synthesized"},"role":format!("{:?}",e.role)}))).collect::<Vec<_>>(),
        "exampleDiagnostics":plan.examples.diagnostics().iter().map(|d| serde_json::json!({"document":d.source.document().to_string(),"pointer":d.source.pointer(),"code":d.code,"message":d.message})).collect::<Vec<_>>(),
        "protocolCapabilities":plan.protocol.capabilities(),
        "protocolDiagnostics":plan.protocol.diagnostics(),
        "aggregates":plan.aggregates.iter().map(|a|serde_json::json!({"name":a.type_name,"document":a.source.document().as_str(),"pointer":a.source.pointer(),"multipart":a.multipart})).collect::<Vec<_>>(),
        "unsupported":["directional model views", "general native allOf intersections", "multi-type unions other than nullable", "native tuples", "positional/streamed multipart", "legacy SSE schema/sentinel inference", "callbacks/webhooks as outgoing operations"],
        "formatPolicy":"annotation-only unless compilation explicitly admits assertion semantics"
    });
    serde_json::to_string_pretty(&value).unwrap() + "\n"
}
fn reference(plan: &SdkPlan) -> String {
    let mut out = format!(
        "# {} C++20 API reference\n\nNamespace `{}`. [Installation and policy](../README.md).\n\n",
        plan.config.name, plan.config.namespace
    );
    for op in &plan.operations {
        writeln!(out, "## {}\n\n`Result<{}, {}> Client::{}(const {}&, CallOptions) const`\n\n{}\n\nWire: `{}` `{}`. Source: {}#{}\n", op.method_name, op.success_type, op.error_type, op.method_name, op.input_type, prose(op.wire.description().map(|d|d.value().as_str()).unwrap_or("")), op.wire.method().as_str(), prose(op.wire.path()), op.source.document(), prose(op.source.pointer())).unwrap();
        for p in &op.parameters {
            writeln!(
                out,
                "- `{}`: `{}`; wire `{}`, {:?}, {}.",
                p.field_name,
                p.cpp_type,
                prose(&p.wire_name),
                p.wire.location(),
                if p.required { "required" } else { "optional" }
            )
            .unwrap();
        }
        if let Some(body) = &op.body {
            writeln!(
                out,
                "- `body`: `{}`; {}. Representations: {}.",
                body.cpp_type,
                if body.required {
                    "required"
                } else {
                    "optional"
                },
                body.media
                    .iter()
                    .map(|m| format!("`{}`", prose(m.wire.media_type().declared())))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
        for response in &op.responses {
            for case in &response.cases {
                writeln!(out,"- `{}`: status rule `{}`, body `{}`; {}. Actual status determines success/error membership.",case.variant_type,response.status_key,case.value.cpp_type,case.media.as_ref().map(|m|prose(m.media_type().declared())).unwrap_or_else(||if case.forbidden{"HTTP-forbidden body"}else{"undeclared bounded bytes"}.into())).unwrap();
            }
            for header in &response.headers {
                writeln!(
                    out,
                    "  - Header `{}` → `{}`: `{}`, {}.",
                    prose(header.wire.name()),
                    header.field_name,
                    header.value.cpp_type,
                    if header.wire.required() {
                        "required"
                    } else {
                        "optional"
                    }
                )
                .unwrap();
            }
            for link in response.wire.links() {
                writeln!(
                    out,
                    "  - Link metadata `{}` (no automatic navigation).",
                    prose(link.name())
                )
                .unwrap();
            }
        }
        out.push('\n');
    }
    for s in plan.models.symbols() {
        writeln!(out, "<a id=\"schema-{}\"></a>\n\n## {}\n\nNative type: `{}`. Codec: `{}` (`decode`, `encode`, `to_json`).\n\nSource: {}#{}\n", s.index, s.name, s.cpp_type, s.codec_name, prose(&s.source.document().to_string()), prose(s.source.pointer())).unwrap();
        if let Some(raw) = plan.contract.source(&s.source) {
            if let Some(description) = raw.get("description").and_then(|v| v.as_str()) {
                writeln!(out, "{}\n", prose(description)).unwrap();
            }
            let constraints = raw
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(k, _)| {
                            [
                                "minimum",
                                "maximum",
                                "exclusiveMinimum",
                                "exclusiveMaximum",
                                "multipleOf",
                                "minLength",
                                "maxLength",
                                "minItems",
                                "maxItems",
                                "uniqueItems",
                                "minProperties",
                                "maxProperties",
                                "const",
                                "enum",
                                "format",
                                "deprecated",
                                "readOnly",
                                "writeOnly",
                                "discriminator",
                            ]
                            .contains(&k.as_str())
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<serde_json::Map<_, _>>()
                })
                .unwrap_or_default();
            if !constraints.is_empty() {
                writeln!(out, "Source constraints/annotations (enforced according to the declared validation profile):\n\n```json\n{}\n```\n", serde_json::to_string_pretty(&constraints).unwrap()).unwrap();
            }
        }
        if let Shape::Object { fields, extras } = &s.shape {
            for f in fields {
                let ty = plan.models.get(&f.schema);
                writeln!(
                    out,
                    "- `{}` → wire `{}`: {} `{}`; [schema](#schema-{}).",
                    f.name,
                    prose(&f.wire),
                    if f.required { "required" } else { "optional" },
                    ty.cpp_type,
                    ty.index
                )
                .unwrap();
            }
            writeln!(
                out,
                "\nExtras: {}.\n",
                match extras {
                    Extras::Any => "exact JsonValue map",
                    Extras::Patterned => "exact JsonValue map checked by whole-object pattern/additional/unevaluated rules",
                    Extras::Closed => "closed object, no extra member",
                    Extras::Typed(_) => "schema-typed map",
                }
            )
            .unwrap();
        }
    }
    out
}
fn readme(plan: &SdkPlan, example: Option<&native_example::NativeExample>) -> String {
    let name = &plan.config.name;
    let examples_link = if example.is_some() {
        "[executable examples](examples/client.cpp)"
    } else {
        "[example findings](docs/coverage.json)"
    };
    let mut out = format!(
        "# {name} — C++20 SDK\n\n```sh\ncmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=\"$PWD/install\" -DSUSPECT_SDK_BUILD_DOCS=ON\ncmake --build build\nctest --test-dir build --output-on-failure\ncmake --build build --target sdk_docs\ncmake --install build\n```\n\nConsumer CMake:\n\n```cmake\nfind_package({name} {} CONFIG REQUIRED)\ntarget_link_libraries(app PRIVATE {name}::{name})\n```\n\nInclude `<{name}/sdk.hpp>`. The installed target exports its C++20 and libcurl\nrequirements. Use `SUSPECT_SDK_WITH_CURL=OFF` for a core-only package with an\ninjected transport. Native documentation requires Doxygen 1.9.8 or newer; use\n`SUSPECT_SDK_BUILD_DOCS=OFF` when only building the library.\n\n[API reference](docs/reference.md), [symbol/example coverage](docs/coverage.json),\n{examples_link}. Native HTML is in `build/docs/html`.\n",
        plan.config.version
    );
    if let Some(example) = example {
        let callsite = &example.callsite;
        write!(out,"\n## First request\n\nThese actual public names and native constructors are compiled and run in\n`examples/client.cpp`. Fixed values originate in the checked example plan;\ncomments identify declared or synthesized examples.\n\n```cpp\n#include <{name}/sdk.hpp>\n#include <iostream>\nusing namespace {};\n\n{callsite}\n```\n",plan.config.namespace).unwrap();
        if let (Some(scheme), Some(field)) = (&example.security_scheme, &example.credential_name) {
            write!(out,"\nSupply the bearer credential for source scheme `{}` explicitly:\n\n```cpp\nCredentials credentials;\ncredentials.{field} = token; // application-supplied string\nauto connected = Client::with_curl(std::move(credentials));\nif (!connected) return 2;\nreturn first_request(connected.value());\n```\n\nThe executable uses a fixture when run without arguments. Passing a bearer token\nexplicitly executes one request against the source-declared server. Credential\nacquisition belongs to the application.\n",single_line(scheme)).unwrap();
        }
    }
    out.push_str(&credential_env::documentation(plan));
    out.push_str(include_str!("guide.md"));
    write!(out,"\nGenerated ceilings: URL/body {} bytes each, response {} bytes, raw capture {}\nbytes, and cumulative response headers {} bytes. This package selects {}\noperations; the reference lists every operation and reachable schema.\n",plan.config.max_request_bytes,plan.config.max_response_bytes,plan.config.max_capture_bytes,plan.config.max_header_bytes,plan.operations.len()).unwrap();
    out
}
