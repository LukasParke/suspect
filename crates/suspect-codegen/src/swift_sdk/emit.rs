//! Models, codecs, operations and native documentation consume one Swift plan.
use std::fmt::Write as _;

use serde_json::json;
use suspect_ir::contract::SchemaId;
use suspect_schema::ProgramSource;

use super::{
    PackageConfig, PlannedOperation, SdkPlan,
    models::{Declaration, ModelPlan, Type},
    validation,
};
use crate::{OutFile, examples::ExampleOrigin};

pub(super) fn package(plan: &SdkPlan, package: &PackageConfig) -> Vec<OutFile> {
    let module = &package.module_name;
    let mut files = vec![OutFile {
        path: "Package.swift".into(),
        content: format!(
            "// swift-tools-version: 6.0\nimport PackageDescription\n\nlet package = Package(\n    name: {},\n    platforms: [.macOS(.v13), .iOS(.v16), .tvOS(.v16), .watchOS(.v9)],\n    products: [.library(name: {}, targets: [{}])],\n    targets: [\n        .target(name: {}, swiftSettings: [.swiftLanguageMode(.v6)]),\n        .testTarget(name: {} , dependencies: [{}], swiftSettings: [.swiftLanguageMode(.v6)])\n    ]\n)\n",
            q(&package.name),
            q(&package.name),
            q(module),
            q(module),
            q(&format!("{module}Tests")),
            q(module)
        ),
    }];
    for (name, text) in [
        ("ExactJson.swift", include_str!("json.swift")),
        ("ExactNumber.swift", include_str!("number.swift")),
        ("Presence.swift", include_str!("presence.swift")),
        ("Validation.swift", include_str!("validation.swift")),
        ("ModelCodec.swift", include_str!("codecs.swift")),
        ("HTTPTransport.swift", include_str!("transport.swift")),
        ("HTTPProtocol.swift", include_str!("protocol_runtime.swift")),
        (
            "HTTPParameters.swift",
            include_str!("protocol_parameters.swift"),
        ),
        ("HTTPParts.swift", include_str!("protocol_parts.swift")),
        ("HTTPStreams.swift", include_str!("protocol_stream.swift")),
        (
            "HTTPExactMethods.swift",
            include_str!("protocol_exact.swift"),
        ),
    ] {
        files.push(source(module, name, text.into()));
    }
    files.push(source(
        module,
        "ValidationData.swift",
        plan.validation_data.clone(),
    ));
    files.push(source(module, "Attribution.swift", attribution(plan)));
    if let Some(pagination) = super::pagination::emit(plan) {
        files.push(source(module, "Pagination.swift", pagination));
    }
    // The OAuth lifecycle runtime is emitted only for a configured policy with
    // at least one usable scheme; no-policy output stays byte-identical.
    if let Some(oauth) = super::oauth::emit(plan) {
        files.push(source(module, "OAuth.swift", oauth));
    }
    // The typed event decode is emitted only for discriminated declared SSE
    // event streams; every other document stays byte-identical.
    if let Some(streams) = super::stream_events::emit(plan) {
        files.push(source(module, "StreamEvents.swift", streams));
    }
    // The incoming receipt helpers are emitted only for declared webhooks and
    // callbacks; receipt-less documents stay byte-identical.
    if let Some(incoming) = super::incoming::emit(plan) {
        files.push(source(module, "Incoming.swift", incoming));
    }
    files.push(source(module, "Models.swift", models(plan)));
    files.push(source(module, "Codecs.swift", codecs(plan)));
    files.push(source(
        module,
        "Client.swift",
        super::protocol_emit::client(plan),
    ));
    files.push(source(
        module,
        "Operations.swift",
        super::protocol_emit::operations(plan),
    ));
    files.push(source(module, "Examples.swift", examples(plan)));
    files.push(OutFile { path: format!("Tests/{module}Tests/ExamplesTests.swift"), content: format!("import XCTest\nimport {module}\n\nfinal class GeneratedExamplesTests: XCTestCase {{\n    func testSourceValidatedExamplesRoundTrip() throws {{ try Examples.verify() }}\n}}\n") });
    files.extend(docs(plan, package));
    files.push(OutFile {
        path: "validation-program.json".into(),
        content: serde_json::to_string_pretty(&plan.program).unwrap() + "\n",
    });
    files.push(OutFile { path: "sdk-manifest.json".into(), content: serde_json::to_string_pretty(&json!({
        "format":"suspect.swift-sdk.experimental.v1", "generator":env!("CARGO_PKG_VERSION"),
        "package":package.name, "module":package.module_name, "version":package.version,
        "swiftLanguageMode":6, "minimumToolsVersion":"6.0", "validationProfile":plan.program.profile,
        "operations":plan.operations.iter().map(|op| json!({"source":loc(&op.source),"operationId":op.operation_id,"method":op.method_name,"input":op.input_type,"result":op.success_type,"apiError":op.error_type})).collect::<Vec<_>>(),
        "models":plan.models.types.iter().map(|(id,ty)| json!({"source":loc(id),"type":ty.full().render(&plan.models),"codec":plan.models.codecs[id]})).collect::<Vec<_>>(),
        "responseByteCeiling":plan.config.max_response_bytes,"requestByteCeiling":plan.config.max_request_bytes,
        "errorCaptureByteCeiling":plan.config.max_stream_capture_bytes,
        "protocolProfile":plan.protocol.capabilities().adapter(),
        "capabilities":plan.protocol.capabilities().enabled(),
        "compatibilityProfiles":plan.protocol.capabilities().profiles(),
        "partByteCeiling":plan.config.max_part_bytes,
        "streamItemByteCeiling":plan.config.max_stream_item_bytes,
        "streamBufferByteCeiling":plan.config.max_stream_buffer_bytes,
        "promotion":"selected-operation slice; release verification is a separate required gate"
    })).unwrap() + "\n" });
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn source(module: &str, name: &str, content: String) -> OutFile {
    OutFile {
        path: format!("Sources/{module}/{name}"),
        content: format!(
            "// Generated by suspect. Regenerate from the pinned source contract.\n{content}"
        ),
    }
}
/// `ua/v1` attribution constants compiled at generation time. An absent
/// descriptor emits the disabled sentinel: an empty suspect version makes the
/// runtime suppress the automatic User-Agent header entirely.
fn attribution(plan: &SdkPlan) -> String {
    match &plan.config.attribution {
        Some(attribution) => format!(
            "/// ua/v1 attribution: every request identifies suspect as the generator and the\n/// SDK or a caller-supplied application as the client.\nenum Attribution: Sendable {{\n    /// Suspect generator version captured at generation time. An empty value\n    /// disables the automatic attribution header.\n    static let suspectVersion = {}\n    static let sdkName = {}\n    static let sdkVersion = {}\n    /// Source document's declared OpenAPI or Swagger version.\n    static let specVersion = {}\n    /// `ua/v1` language tag used in the attribution header comment.\n    static let language = {}\n}}\n",
            q(&attribution.suspect_version),
            q(&attribution.sdk_name),
            q(&attribution.sdk_version),
            q(&attribution.spec_version),
            q(&attribution.language),
        ),
        None => "/// ua/v1 attribution is disabled for this package: an empty suspect version\n/// suppresses the automatic User-Agent header.\nenum Attribution: Sendable {\n    static let suspectVersion = \"\"\n    static let sdkName = \"\"\n    static let sdkVersion = \"\"\n    static let specVersion = \"\"\n    static let language = \"swift\"\n}\n"
            .into(),
    }
}
fn q(value: &str) -> String {
    validation::string(value)
}
fn loc(id: &SchemaId) -> String {
    format!("{}#{}", id.document(), id.pointer())
}
fn at(id: &SchemaId) -> String {
    validation::source(&ProgramSource::from(id))
}
fn doc(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        let safe: String = line
            .chars()
            .flat_map(|c| {
                if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                    c.escape_default().collect::<Vec<_>>()
                } else {
                    vec![c]
                }
            })
            .collect();
        let _ = writeln!(out, "{indent}/// {safe}");
    }
}

// Source prose is displayed as text. In particular, source-supplied DocC
// directives, images, raw HTML and link destinations never become instructions
// to the documentation compiler or its rendered page.
pub(super) fn prose(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '!' | '|' | '@' => {
                out.push('\\');
                out.push(ch);
            }
            ch => out.push(ch),
        }
    }
    out
}

fn closure(ty: &Type, models: &ModelPlan, encode: bool) -> String {
    format!(
        "{{ value, context, path, depth in {} }}",
        expr(ty, models, "value", "path", "depth", encode)
    )
}
fn expr(
    ty: &Type,
    models: &ModelPlan,
    value: &str,
    path: &str,
    depth: &str,
    encode: bool,
) -> String {
    match ty {
        Type::Named(id) => format!(
            "try {}.{}({value}, &context, {path}, {depth})",
            models.names[id],
            if encode { "_encode" } else { "_decode" }
        ),
        Type::Array(inner) | Type::Nullable(inner) | Type::Indirect(inner) => {
            let method = match (ty, encode) {
                (Type::Array(_), false) => "array",
                (Type::Array(_), true) => "encodeArray",
                (Type::Nullable(_), false) => "nullable",
                (Type::Nullable(_), true) => "encodeNullable",
                (_, false) => "indirect",
                (_, true) => "encodeIndirect",
            };
            format!(
                "try Conversion.{method}({value}, &context, {path}, {depth}, {})",
                closure(inner, models, encode)
            )
        }
        Type::Primitive(name) if !encode => {
            let method = match *name {
                "String" => "string",
                "Bool" => "bool",
                "JsonNumber" => "number",
                "JsonInteger" => "integer",
                "JsonNull" => "null",
                "JsonValue" => "any",
                _ => unreachable!("unsupported model blocked before emission"),
            };
            format!("try Conversion.{method}({value}, &context, {path}, {depth})")
        }
        Type::Primitive(name) => {
            let make = match *name {
                "String" => "{ .string($0) }",
                "Bool" => "{ .bool($0) }",
                "JsonNumber" => "{ .number($0) }",
                "JsonInteger" => "{ .number($0.number) }",
                "JsonNull" => "{ _ in .null }",
                "JsonValue" => "{ $0 }",
                _ => unreachable!("unsupported model blocked before emission"),
            };
            format!("try Conversion.encode({value}, &context, {path}, {depth}, {make})")
        }
    }
}
fn intrinsic_null(ty: &Type) -> bool {
    matches!(ty, Type::Primitive("JsonValue" | "JsonNull"))
}

fn model_codec(plan: &SdkPlan, id: &SchemaId, ty: &Type) -> String {
    format!(
        "ModelCodec<{}>(source: {}, index: {}, decodeNative: {}, encodeNative: {})",
        ty.render(&plan.models),
        at(id),
        validation::index(&plan.program, id),
        closure(ty, &plan.models, false),
        closure(ty, &plan.models, true)
    )
}

fn models(plan: &SdkPlan) -> String {
    let m = &plan.models;
    let mut out = String::from("import Foundation\n\n");
    for (id, declaration) in &m.declarations {
        let name = &m.names[id];
        let description = plan
            .contract
            .schema(id)
            .and_then(|s| s.raw().get("description"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        doc(&mut out, &prose(description), "");
        doc(
            &mut out,
            &format!(
                "Source: {}. Constraints are enforced on every codec boundary.",
                prose(&loc(id))
            ),
            "",
        );
        match declaration {
            Declaration::Checked { value } => {
                doc(
                    &mut out,
                    "Exact checked carrier. The value is the complete wire instance, not a wrapper object. Every codec operation validates the original source schema.",
                    "",
                );
                let _ = writeln!(
                    out,
                    "public struct {name}: SourceCodable, Equatable {{\n    public var value: {}\n    public init(value: {}) {{ self.value = value }}",
                    value.render(m),
                    value.render(m)
                );
                let _ = writeln!(
                    out,
                    "    static func _decode(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> Self {{\n        try context.visit(path, depth)"
                );
                if m.types[id].nullable {
                    out.push_str("        _ = try Conversion.nonNull(value, path)\n");
                }
                let _ = writeln!(
                    out,
                    "        return Self(value: {})\n    }}",
                    expr(value, m, "value", "path", "depth + 1", false)
                );
                let _ = writeln!(
                    out,
                    "    static func _encode(_ value: Self, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonValue {{\n        try context.visit(path, depth)\n        let json = {}",
                    expr(value, m, "value.value", "path", "depth + 1", true)
                );
                let _ = writeln!(
                    out,
                    "        return {}\n    }}",
                    if m.types[id].nullable {
                        "try Conversion.nonNull(json, path)"
                    } else {
                        "json"
                    }
                );
            }
            Declaration::Object { fields, extras } => {
                let _ = writeln!(out, "public struct {name}: SourceCodable, Equatable {{");
                for f in fields {
                    doc(
                        &mut out,
                        &format!(
                            "Wire key: {}. {}. Source: {}.",
                            prose(&q(&f.wire)),
                            if f.required {
                                "Required"
                            } else {
                                "Optional; default is missing"
                            },
                            prose(&loc(&f.source))
                        ),
                        "    ",
                    );
                    doc(&mut out, &prose(&f.description), "    ");
                    let _ = writeln!(out, "    public var {}: {}", f.name, f.ty(m));
                }
                if let Some(extra) = extras {
                    doc(
                        &mut out,
                        "Every undeclared wire property, with exact Unicode key identity. Collisions with declared names fail encoding.",
                        "    ",
                    );
                    let _ = writeln!(
                        out,
                        "    public var additionalProperties: JsonObject<{}>",
                        extra.render(m)
                    );
                }
                let constructor_fields = declaration.constructor_fields(m);
                let mut args: Vec<String> = constructor_fields
                    .iter()
                    .map(|f| {
                        format!(
                            "{}: {}{}",
                            f.name,
                            f.ty(m),
                            f.default(m).map(|d| format!(" = {d}")).unwrap_or_default()
                        )
                    })
                    .collect();
                if let Some(extra) = extras {
                    args.push(format!(
                        "additionalProperties: JsonObject<{}> = .init()",
                        extra.render(m)
                    ));
                }
                doc(
                    &mut out,
                    "Creates a mutable value. Serialization validates its current state against the source schema.",
                    "    ",
                );
                let _ = writeln!(out, "    public init({}) {{", args.join(", "));
                for f in fields {
                    let _ = writeln!(out, "        self.{} = {}", f.name, f.name);
                }
                if extras.is_some() {
                    out.push_str("        self.additionalProperties = additionalProperties\n");
                }
                out.push_str("    }\n");
                let _ = writeln!(
                    out,
                    "    static func _decode(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> Self {{\n        let object = try Conversion.object(value, &context, path, depth)"
                );
                for (field_index, f) in fields.iter().enumerate() {
                    let p = format!("childPath(path, {})", q(&f.wire));
                    let get = format!("object[{}]", q(&f.wire));
                    let conversion = if f.required {
                        expr(
                            &f.model_type.full(),
                            m,
                            &format!("try Conversion.required({get}, {p})"),
                            &p,
                            "depth + 1",
                            false,
                        )
                    } else {
                        format!(
                            "try Conversion.{}({get}, &context, {p}, depth + 1, {}{})",
                            if f.model_type.nullable {
                                "presence"
                            } else {
                                "optional"
                            },
                            if f.model_type.nullable {
                                String::new()
                            } else {
                                format!("allowsNull: {}, ", intrinsic_null(&f.model_type.core))
                            },
                            closure(&f.model_type.core, m, false)
                        )
                    };
                    let _ = writeln!(
                        out,
                        "        let _field{field_index}: {} = {conversion}",
                        f.ty(m)
                    );
                }
                let declared = fields
                    .iter()
                    .map(|f| format!("JsonKey({})", q(&f.wire)))
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(extra) = extras {
                    let _ = writeln!(out, "        var extra = JsonObject<{}>()", extra.render(m));
                    let _ = writeln!(
                        out,
                        "        let declared: Set<JsonKey> = [{declared}]\n        for (key, value) in object.members where !declared.contains(JsonKey(key)) {{\n            extra[key] = {}\n        }}",
                        expr(
                            extra,
                            m,
                            "value",
                            "childPath(path, key)",
                            "depth + 1",
                            false
                        )
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "        let declared: Set<JsonKey> = [{declared}]\n        guard object.keys.allSatisfy({{ declared.contains(JsonKey($0)) }}) else {{ throw JsonError(.representation, \"closed object has undeclared properties\", path: path) }}"
                    );
                }
                let mut values: Vec<String> = constructor_fields
                    .iter()
                    .map(|f| {
                        format!(
                            "{}: _field{}",
                            f.name,
                            fields
                                .iter()
                                .position(|original| original.name == f.name)
                                .expect("planned constructor field")
                        )
                    })
                    .collect();
                if extras.is_some() {
                    values.push("additionalProperties: extra".into());
                }
                let _ = writeln!(out, "        return Self({})\n    }}", values.join(", "));
                let _ = writeln!(
                    out,
                    "    static func _encode(_ value: Self, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonValue {{\n        try context.visit(path, depth)\n        {} object = JsonObject<JsonValue>()",
                    if fields.is_empty() && extras.is_none() {
                        "let"
                    } else {
                        "var"
                    }
                );
                for f in fields {
                    let p = format!("childPath(path, {})", q(&f.wire));
                    if f.required {
                        let _ = writeln!(
                            out,
                            "        object[{}] = {}",
                            q(&f.wire),
                            expr(
                                &f.model_type.full(),
                                m,
                                &format!("value.{}", f.name),
                                &p,
                                "depth + 1",
                                true
                            )
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "        switch value.{} {{\n        case .missing: break",
                            f.name
                        );
                        if f.model_type.nullable {
                            let _ =
                                writeln!(out, "        case .null: object[{}] = .null", q(&f.wire));
                        }
                        let encoded = expr(&f.model_type.core, m, "present", &p, "depth + 1", true);
                        let encoded =
                            if intrinsic_null(&f.model_type.core) && !f.model_type.nullable {
                                encoded
                            } else {
                                format!("try Conversion.nonNull({encoded}, {p})")
                            };
                        let _ = writeln!(
                            out,
                            "        case .value(let present): object[{}] = {encoded}\n        }}",
                            q(&f.wire)
                        );
                    }
                }
                if let Some(extra) = extras {
                    let _ = writeln!(
                        out,
                        "        try context.collection(value.additionalProperties.count, path)\n        let declared: Set<JsonKey> = [{declared}]\n        for (key, member) in value.additionalProperties.members {{\n            guard !declared.contains(JsonKey(key)) else {{ throw JsonError(.representation, \"additional property collides with a declared key\", path: childPath(path, key)) }}\n            object[key] = {}\n        }}",
                        expr(
                            extra,
                            m,
                            "member",
                            "childPath(path, key)",
                            "depth + 1",
                            true
                        )
                    );
                }
                out.push_str("        return .object(object)\n    }\n");
            }
            Declaration::Literals(values) => {
                let _ = writeln!(out, "public enum {name}: SourceCodable, Equatable {{");
                for (case, wire) in values {
                    doc(
                        &mut out,
                        &format!("Exact source literal {}.", prose(&q(wire))),
                        "    ",
                    );
                    let _ = writeln!(out, "    case {case}");
                }
                out.push_str("    /// Exact wire string, without Unicode normalization.\n    public var rawValue: String {\n        switch self {\n");
                for (case, wire) in values {
                    let _ = writeln!(out, "        case .{case}: return {}", q(wire));
                }
                out.push_str("        }\n    }\n");
                out.push_str("    static func _decode(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> Self {\n        let string = try Conversion.string(value, &context, path, depth)\n");
                for (case, wire) in values {
                    let _ = writeln!(
                        out,
                        "        if string.utf8.elementsEqual({}.utf8) {{ return .{case} }}",
                        q(wire)
                    );
                }
                out.push_str("        throw JsonError(.representation, \"value is not a declared string literal\", path: path)\n    }\n    static func _encode(_ value: Self, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonValue {\n        try context.visit(path, depth)\n        return .string(value.rawValue)\n    }\n");
            }
            Declaration::Union(variants) => {
                let _ = writeln!(
                    out,
                    "public indirect enum {name}: SourceCodable, Equatable {{"
                );
                for v in variants {
                    doc(
                        &mut out,
                        &format!(
                            "Source alternative: {}. Branch validity is checked during both conversion directions.",
                            prose(&loc(&v.source))
                        ),
                        "    ",
                    );
                    let _ = writeln!(out, "    case {}({})", v.name, v.ty.render(m));
                }
                out.push_str("    static func _decode(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> Self {\n        try context.visit(path, depth)\n");
                if m.types[id].nullable {
                    out.push_str("        _ = try Conversion.nonNull(value, path)\n");
                }
                for v in variants {
                    let _ = writeln!(
                        out,
                        "        if try context.validation.matches({}, value, path: path) {{ return .{}({}) }}",
                        validation::index(&plan.program, &v.source),
                        v.name,
                        expr(&v.ty, m, "value", "path", "depth + 1", false)
                    );
                }
                out.push_str("        throw JsonError(.representation, \"no declared union alternative accepts the value\", path: path)\n    }\n    static func _encode(_ value: Self, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonValue {\n        try context.visit(path, depth)\n        switch value {\n");
                for v in variants {
                    let _ = writeln!(
                        out,
                        "        case .{}(let member):\n            let json = {}\n            try context.validation.check({}, json, path: path)\n            return {}",
                        v.name,
                        expr(&v.ty, m, "member", "path", "depth + 1", true),
                        validation::index(&plan.program, &v.source),
                        if m.types[id].nullable {
                            "try Conversion.nonNull(json, path)"
                        } else {
                            "json"
                        }
                    );
                }
                out.push_str("        }\n    }\n");
            }
        }
        doc(
            &mut out,
            "Exact source-bound codec for this native declaration.",
            "    ",
        );
        let _ = writeln!(
            out,
            "    public static let codec = {}\n}}\n",
            model_codec(plan, id, &Type::Named(id.clone()))
        );
    }
    out
}

fn codecs(plan: &SdkPlan) -> String {
    let mut out = String::from(
        "import Foundation\n\n/// Source-bound codecs for operation roots and reachable native values.\npublic enum Codecs {\n",
    );
    for (id, ty) in &plan.models.types {
        doc(
            &mut out,
            &format!(
                "Validates and converts {}. Source: {}.",
                ty.full().render(&plan.models),
                prose(&loc(id))
            ),
            "    ",
        );
        let _ = writeln!(
            out,
            "    public static let {} = {}",
            plan.models.codecs[id],
            model_codec(plan, id, &ty.full())
        );
    }
    out.push_str("}\n");
    out
}

fn examples(plan: &SdkPlan) -> String {
    let mut out = String::from(
        "import Foundation\n\n/// Shared, source-validated examples. Synthesized values are mechanical, not service promises.\npublic enum Examples {\n    /// Revalidates every emitted example through its actual native source codec.\n    public static func verify() throws {\n",
    );
    for operation in plan.examples.operations() {
        for entry in &operation.entries {
            let text = serde_json::to_string(&entry.value).unwrap();
            let codec = &plan.models.codecs[&entry.schema];
            let _ = writeln!(
                out,
                "        _ = try Codecs.{codec}.encode(Codecs.{codec}.decode(Data({}.utf8)))",
                q(&text)
            );
        }
    }
    out.push_str("    }\n");
    for op in &plan.operations {
        if let Some(snippet) = example_call(plan, op) {
            doc(
                &mut out,
                &format!(
                    "Executable example for {}. Values and origins are listed in the package example coverage report.",
                    op.operation_id
                ),
                "    ",
            );
            let _ = writeln!(
                out,
                "    public static func {}(client: Client) async throws -> {} {{\n{snippet}    }}",
                op.method_name, op.success_type
            );
        }
    }
    out.push_str("}\n");
    out
}

fn example_call(plan: &SdkPlan, op: &PlannedOperation) -> Option<String> {
    super::protocol_examples::call(plan, op)
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('`', "&#96;")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
fn docs(plan: &SdkPlan, package: &PackageConfig) -> Vec<OutFile> {
    let module = &package.module_name;
    let overview = format!(
        "# ``{module}``\n\nSource-selected, exact JSON SDK using Swift 6 value models and async/await.\n\n## Overview\n\nUse ``Client`` with runtime ``Credentials`` and the default ``URLSessionTransport``,\nor inject a Sendable ``HTTPTransport``. Each operation performs exactly one request.\n\n## Topics\n\n### Guides\n- <doc:GettingStarted>\n- <doc:ModelsAndCodecs>\n- <doc:OperationReference>\n\n### Runtime\n- ``Client``\n- ``Credentials``\n- ``ModelCodec``\n- ``Codecs``\n- ``SDKJSONDecoder``\n- ``SDKJSONEncoder``\n- ``Presence``\n- ``OptionalField``\n- ``Nullable``\n- ``JsonNumber``\n- ``JsonInteger``\n- ``JsonValue``\n- ``JsonObject``\n- ``SDKError``\n- ``ValidationError``\n- ``Examples``\n"
    );
    let initialization = if plan.credential_env.is_some() {
        "Client.fromEnvironment()".into()
    } else {
        format!("Client(credentials: Credentials({}))", plan.credential_bindings
            .iter()
            .map(|c| format!("{}: {}",c.property,match c.requirement.credential() {
                crate::http_protocol::CredentialHook::Basic => "HTTPBasicCredential(username: \"runtime-user\", password: \"runtime-password\")",
                crate::http_protocol::CredentialHook::OAuth2 { .. } | crate::http_protocol::CredentialHook::OpenIdConnect { .. } => "{ context in runtimeAuthorization(for: context) }",
                _ => "\"runtime-token\"",
            }))
            .collect::<Vec<_>>()
            .join(", "))
    };
    let mut getting = format!(
        "# Getting started\n\nRequires Swift tools 6.0 and Swift 6 language mode. Apple baseline: macOS 13,\niOS/tvOS 16, watchOS 9. Package: `{}` version `{}`.\n\nAdd the generated directory as a local SwiftPM dependency and link its\n`{}` library product. Import `{module}`. No third-party runtime dependencies.\n\n```swift\nimport {module}\nlet client = {initialization}\n```\n\n",
        escaped(&package.name),
        escaped(&package.version),
        escaped(&package.name),
    );
    if let Some(policy) = &plan.credential_env {
        getting.push_str("## Environment credentials\n\n`Client()` and `Client.fromEnvironment(transport:options:)` snapshot the\nconfigured process variables once at construction. Both accept an injected\nSendable transport and ClientOptions; the factory returns Client without\nthrowing. Later environment changes affect newly created clients only.\n\nAn explicit `Client(credentials: Credentials(...), transport: ..., options: ...)`\nuses that entire credential value. Missing or nil members and empty strings are\nnot filled from the environment. The credential argument itself is nonoptional;\n`Client(credentials: nil)` is rejected by Swift's type checker.\n\nMissing, empty or unusable environment values remain unavailable. Security\nalternative/AND/anonymous selection is source-defined. A protected call with\nunsatisfied credentials throws `SDKError` with kind `.requestRepresentation`\nand a secret-free JsonError before transport; anonymous operations remain usable.\n\nConfigured source scheme → environment variable:\n\n");
        for binding in policy.bindings() {
            let _ = writeln!(
                getting,
                "- `{}` → `{}`",
                escaped(binding.name()),
                escaped(binding.variable())
            );
        }
        let _ = writeln!(
            getting,
            "\nValues are limited to {} UTF-8 bytes by the existing Swift attachment policy.\nThe generated SDK contains variable names only. Environment credentials do not\nchange the source-default HTTPS server or acquire/refresh credentials.\n",
            super::CREDENTIAL_ENV_MAX_BYTES
        );
    }
    for op in &plan.operations {
        if let Some(snippet) = example_call(plan, op) {
            let _ = writeln!(
                getting,
                "## {}\n\nThis direct-constructor call site is also compiled in `Examples.{}`. Values\nare validated with the source-bound codecs; numeric tokens remain exact.\n\n```swift\nfunc example(client: Client) async throws -> {} {{\n{snippet}}}\n```\n",
                escaped(&op.operation_id),
                op.method_name,
                op.success_type
            );
        }
    }
    getting.push_str("## Calling operations\n\nInputs with no required fields can be omitted: `try await client.operation()`.\nA sole successful result exposes `.data`, `.status`, `.headers`, `.links`, and\nraw-body capture directly; its declared status case remains available. Multiple\nstatuses/media are explicit enums. Success uses the actual 200–299 HTTP status,\nincluding a status matched by `default`. Exact status beats range, then default;\nmedia mismatch never falls through to a different response declaration.\n\n## Servers and credentials\n\nEach operation's generated HTTP metadata lists effective servers and security\nrequirements with original sources. ClientOptions and RequestOptions select a\nserver index and literal variable overrides. Relative servers use documentURL\nor the actual HTTP retrieval document URL. Local files have no invented origin.\nDeclared HTTP servers are honored; serverURL overrides use HTTPS or loopback\nHTTP unless the caller explicitly sets allowHTTP. Security alternatives are OR,\nmember schemes are AND. Selection defaults to the first fully configured\nalternative, including anonymous, or uses an explicit securityAlternative index.\nBasic/bearer/API-key attachment is source-defined. OAuth/OIDC hooks receive\nsource, flow, discovery and scope metadata and return a complete Authorization\nfield. No token type, acquisition, refresh or retry is inferred.\n\n## Transport and errors\n\nTimeouts are finite (1 day maximum). Per-call response ceilings may only lower\nthe client/generated ceiling. URLSession uses ephemeral storage without cookie,\ncache or credential persistence, rejects redirects and bounds received bytes.\nCustom method tokens preserve exact case. For tokens URLRequest would normalize,\nthe Apple Network transport uses bounded HTTP/1.1 with system TLS trust, no\nredirects/retries and a whole-transfer deadline. This exact-method path supports\nidentity content encoding and finite or incremental response consumption.\nTask cancellation aborts I/O and remains CancellationError. Custom transports\nimplement the same policy; returned status, headers and body are checked again.\nAPIError enums carry typed declared failures; SDKError covers request validation,\nrepresentation, response decoding and transport faults. Captures are bounded\nand error descriptions omit credentials and payloads.\n\n## Whole-query parameters\n\nAn OAS 3.2 querystring parameter supplies the complete URL query, without a name\nprefix. JSON/text receive one URI-component encoding pass. Forms use native\nobject inputs and source-bound aggregate/field codecs; their form-urlencoded\noutput receives no second encoding pass. Ordinary query parameters and query\nAPI keys cannot be implicitly merged with complete query content.\n\n## Forms, ordered parts and streams\n\nForms/multipart use native field structs, repeated arrays and in-memory Data\nparts. Required fields, extras, cardinality, part codecs and separate byte\nceilings are enforced. Positional multipart uses typed part1/part2 prefix slots\nand typed remaining items. Optional missing slots must be a trailing suffix;\nfalse prefixes forbid all later positions. Names/filenames are not inferred for\nunnamed parts; positional form-data uses declared Content-Disposition headers.\nMIME style names belong in Content-Disposition and values in the part body.\nMixed byte bodies never enter a JSON codec as null. Source-typed response headers\nenforce requiredness; repeated ambiguous fields are rejected. URLSession's\ncoalesced Set-Cookie declarations are declined. Links trigger no implicit calls.\n\nHTTPEventStream is a single-consumer AsyncSequence. `for try await` pulls parsed\nSSE envelopes or JSON-lines values through the exact itemSchema codec. Breaking\niteration, cancellation, close(), exhaustion and decoding failure close I/O.\nSSE data remains a string, including `[DONE]`; neither nested JSON nor sentinels\nare inferred. Transport queues, individual items and total received bytes have\nindependent finite ceilings. Streaming requests, nested/streamed multipart,\nexpansive MIME style grouping, composite-style form response grouping and vendor\nstream conventions retain source-linked refusals. Profiles require explicit opt-in.\n");
    getting.push_str("## Physical and logical URL metadata\n\n`HTTPServer.documentBase` is the physical retrieval document which owns the\nserver declaration. Relative server URLs use that base unless an explicit\n`documentURL` override is supplied; `$self` and schema `$id` never move the API.\n`HTTPProvenance` retains separate use-site, terminal and reference-hop\n`HTTPResourceContext` values for logical addresses, alongside physical sources.\nEncoded dot/slash segments remain data. Local-file sources need an explicit HTTP\ndocument base. Link server metadata uses the link's own physical document.\n\nOAuth/OIDC providers receive `HTTPCredentialContext.serverURL` after server\nselection and `urlBase == .effectiveServer`. Flow/discovery URL strings and their\noriginal sources remain intact; resolving them never starts token acquisition.\n\n");
    let mut reference = String::from(
        "# Operation reference\n\nEvery entry below is generated from the same operation and symbol plan.\n\n",
    );
    for op in &plan.operations {
        let _ = writeln!(
            reference,
            "## {}\n\n`{} {}`\n\nSource: <code>{}</code>.\n\n{}\n\nInput: ``{}``; success: ``{}``; declared errors: ``{}``.\n\n",
            escaped(&op.operation_id),
            op.method,
            escaped(&op.path),
            escaped(&loc(&op.source)),
            prose(&op.description),
            op.input_type,
            op.success_type,
            op.error_type
        );
    }
    let mut model_doc = String::from(
        "# Models and exact codecs\n\nRequired non-null fields have native value types. Required nullable fields use\nNullable; optional non-null fields use OptionalField; optional nullable fields\nuse Presence. Arbitrary JSON and null-only values carry null in their own JSON\nvalue domain. No source default is inserted. Indirect enum storage gives\nrecursive models value semantics, with no mutable reference graph or unsafe casts.\n\nEvery model codec validates decoding and mutable encoding against the checked\nOwnedProgram. Unions validate all branches; oneOf is exactly-one, while anyOf\nselects the first valid typed alternative without discarding wire properties.\nAn explicitly chosen native variant must satisfy its own branch on encoding.\nFormat remains an annotation unless an admitted assertion policy says otherwise.\n\nUse SDKJSONDecoder/SDKJSONEncoder for Codable adapters, or the source-bound\nModelCodec directly. Foundation JSONDecoder/JSONEncoder are deliberately rejected:\nthey cannot recover arbitrary original numeric tokens. Do not round trip this\nSDK through JSONSerialization, NSNumber, Double, or ordinary synthesized Codable.\nWhitespace, object ordering and string escape spelling may change; numeric\ntokens and Unicode scalar sequences do not. JsonObject compares UTF-8 key bytes\nso canonically equivalent Unicode strings remain distinct JSON names.\n\nParsing, writing, conversion, validation, equality and numeric work have finite\nper-call limits. Incomplete evaluation throws ValidationError.evaluationFailure\nand cannot be inverted or hidden by logical alternatives. Duplicate decoded\nkeys, invalid UTF-8 and unpaired surrogates are rejected. Numbers are strict\ntokens, with symbolic unbounded-magnitude exponents under finite byte limits.\n\n## Reachable declarations\n\n",
    );
    for id in plan.models.declarations.keys() {
        let _ = writeln!(
            model_doc,
            "### ``{}``\n\nSource: <code>{}</code>.\n\n```json\n{}\n```\n",
            plan.models.names[id],
            escaped(&loc(id)),
            serde_json::to_string_pretty(plan.contract.source(id).unwrap()).unwrap()
        );
    }
    if plan.program.version == suspect_schema::OwnedProgram::V3_VERSION {
        model_doc.push_str("## Resource-bound validation\n\nThis package executes the checked `suspect.validation.experimental.v3` /\n`oas31-jsonschema202012-resources-dynamic` program. Node scopes and dynamic\nanchor bindings are immutable indexed metadata. Validation enters the node's\nresource, selects the outermost actually entered matching binding, and restores\nscope on every return. It resolves no URI and loads no document at runtime.\n\nDynamic-dependent shapes use named checked carriers with a mutable `JsonValue`\n`value`. That value is the complete JSON instance, including null when accepted;\nthere is no synthetic wrapper key on the wire. Missing members, explicit nulls,\nnumeric tokens and exact property names remain distinct. Conversion does not\nnarrow an initial fallback's type or re-trial union branches without their\nparent context. Source-bound codecs validate decoding and current mutable\nencoding, and operations always use their actual codec roots.\n\nA native layout can be reused by more than one source codec. Starting directly\nat a nested declaration can legitimately produce a different dynamic binding\nfrom starting at an owning reference in another resource. The named model's\n`codec.source` and the operation's `Codecs` entry identify those distinct entry\npoints; do not substitute a terminal-only codec for an operation codec.\n\nResource entries and binding scans share the finite evaluation budget. Context\nidentity is exact and ordered; cycles or exhausted limits remain noninvertible\n`ValidationError.evaluationFailure` outcomes. Examples come from the bounded\nshared v3 planner, without guessing dynamic fallback annotations.\n");
    }
    let entries:Vec<_>=plan.examples.operations().iter().flat_map(|op|op.entries.iter().map(move|e|json!({"operationId":op.operation_id,"source":loc(&e.schema),"origin":if e.origin==ExampleOrigin::Declared{"declared"}else{"synthesized"},"role":format!("{:?}",e.role),"value":e.value}))).collect();
    let findings: Vec<_> = plan
        .examples
        .diagnostics()
        .iter()
        .map(|d| json!({"source":loc(&d.source),"code":d.code,"message":d.message}))
        .collect();
    let mut coverage = json!({
        "examples":entries,"diagnostics":findings,
        "missingOperationDescriptions":plan.operations.iter().filter(|o|o.description.is_empty()).map(|o|loc(&o.source)).collect::<Vec<_>>(),
        "missingModelDescriptions":plan.models.declarations.keys().filter(|id|plan.contract.source(id).and_then(|s|s.get("description")).and_then(serde_json::Value::as_str).is_none_or(str::is_empty)).map(loc).collect::<Vec<_>>(),
        "missingFieldDescriptions":plan.models.declarations.values().flat_map(|d|match d {Declaration::Object{fields,..}=>fields.iter().filter(|f|f.description.is_empty()).map(|f|loc(&f.source)).collect::<Vec<_>>(),_=>Vec::new()}).collect::<Vec<_>>(),
        "operationsWithoutCallExample":plan.operations.iter().filter(|op|example_call(plan,op).is_none()).map(|op|loc(&op.source)).collect::<Vec<_>>()
    });
    let aggregates = plan
        .examples
        .operations()
        .iter()
        .flat_map(|op| {
            op.validated_aggregates.iter().map(move |entry|json!({
        "operationId":op.operation_id,"schema":loc(&entry.schema),"container":loc(&entry.container),
        "declaredSource":entry.declared_source.as_ref().map(loc),"mediaType":entry.media_type,
        "role":format!("{:?}",entry.role),"value":entry.value
    }))
        })
        .collect::<Vec<_>>();
    if !aggregates.is_empty() {
        coverage["validatedAggregates"] = json!(aggregates);
    }
    vec![
        OutFile {
            path: format!("Sources/{module}/{module}.docc/{module}.md"),
            content: overview,
        },
        OutFile {
            path: format!("Sources/{module}/{module}.docc/GettingStarted.md"),
            content: getting,
        },
        OutFile {
            path: format!("Sources/{module}/{module}.docc/ModelsAndCodecs.md"),
            content: model_doc,
        },
        OutFile {
            path: format!("Sources/{module}/{module}.docc/OperationReference.md"),
            content: reference,
        },
        OutFile {
            path: "example-coverage.json".into(),
            content: serde_json::to_string_pretty(&coverage).unwrap() + "\n",
        },
        OutFile {
            path: "README.md".into(),
            content: format!(
                "# {}\n\nGenerated Swift 6 package, version {}.\n\n```sh\nswift build\nswift test\nswift package dump-symbol-graph\n```\n\nNative documentation is in `Sources/{module}/{module}.docc`. Build it with\n`docc convert` using the symbol graphs produced by SwiftPM.\nSee GettingStarted for the compiled source-backed call sites.\n`validation-program.json`, `sdk-manifest.json` and `example-coverage.json`\nrecord source identities, runtime policy, symbols and example origins.\n",
                escaped(&package.name),
                escaped(&package.version)
            ),
        },
    ]
}
