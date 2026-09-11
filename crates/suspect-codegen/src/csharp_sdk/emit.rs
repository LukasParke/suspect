//! Emission from the immutable C# model, operation and validation plans.
use super::{
    HttpDiagnostic, SdkPlan,
    models::{CsDecl, CsType, Key},
};
use crate::OutFile;
use serde_json::Value;
use suspect_ir::contract::SourceId;

pub(super) fn package(plan: &SdkPlan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    super::validation::check(&plan.contract, &plan.program)?;
    let scoped = plan.program.version == suspect_schema::OwnedProgram::V2_VERSION;
    let resources = plan.program.version == suspect_schema::OwnedProgram::V3_VERSION;
    let program = serde_json::to_string(&plan.program).expect("program JSON");
    if program.len() > 8 * 1024 * 1024 {
        return Err(vec![super::diagnostic(
            &plan.contract,
            plan.operations[0].source.clone(),
            "csharp-program-size",
            "embedded validation program exceeds 8 MiB",
        )]);
    }
    let mut files = vec![
        file("src/Models.g.cs", model_source(plan)),
        file("src/Codecs.g.cs", codecs_source(plan)),
        file(
            "src/JsonRuntime.cs",
            include_str!("JsonRuntime.cs").replace("__NAMESPACE__", &plan.config.namespace),
        ),
        file(
            if scoped {"src/ScopedValidationRuntime.cs"} else {"src/ValidationRuntime.cs"},
            (if scoped {include_str!("ScopedValidationRuntime.cs")} else {include_str!("ValidationRuntime.cs")}).replace("__NAMESPACE__", &plan.config.namespace),
        ),
        file(
            "src/HttpRuntime.cs",
            super::credential_env::http_runtime(plan.credential_env().is_some()).replace("__NAMESPACE__", &plan.config.namespace),
        ),
        file("src/Client.g.cs", super::http::render(plan)),
        file("src/ProtocolRuntime.cs", include_str!("ProtocolRuntime.cs").replace("__NAMESPACE__", &plan.config.namespace)),
        file("src/ServerRuntime.cs", include_str!("ServerRuntime.cs").replace("__NAMESPACE__", &plan.config.namespace)),
        file("src/WireEncoding.cs", include_str!("WireEncoding.cs").replace("__NAMESPACE__", &plan.config.namespace)),
        file("src/PartsRuntime.cs", include_str!("PartsRuntime.cs").replace("__NAMESPACE__", &plan.config.namespace)),
        file("src/PositionalRuntime.cs",include_str!("PositionalRuntime.cs").replace("__NAMESPACE__",&plan.config.namespace)),
        file("src/StreamRuntime.cs", include_str!("StreamRuntime.cs").replace("__NAMESPACE__", &plan.config.namespace)),
        file("protocol-plan.json", serde_json::to_string(&plan.protocol).expect("checked protocol JSON")),
        file("src/ProtocolData.g.cs", format!("{}internal static class ProtocolData\n{{\n    private static readonly JsonElement Data = Load();\n    private static JsonElement Load()\n    {{\n        using var stream = typeof(ProtocolData).Assembly.GetManifestResourceStream(\"Suspect.ProtocolPlan.json\") ?? throw new InvalidOperationException(\"Protocol plan missing\");\n        using var document = JsonDocument.Parse(stream, new JsonDocumentOptions {{ MaxDepth = 256 }});\n        return document.RootElement.Clone();\n    }}\n    internal static JsonElement Operation(int index) => Data.GetProperty(\"operations\")[index];\n    internal static byte[]? Header(JsonElement header, IReadOnlyDictionary<string, IReadOnlyList<string>> values)\n    {{\n        if (values.TryGetValue(header.GetProperty(\"name\").GetString()!, out var value)) return WireEncoding.HeaderJson(header, value);\n        if (header.GetProperty(\"required\").GetBoolean()) throw new CodecException(CodecErrorKind.InvalidValue, \"Required header is absent\", ProtocolRuntime.Source(header.GetProperty(\"source\").GetProperty(\"terminal\")));\n        return null;\n    }}\n}}\n", header(plan))),
        file("validation-program.json", program),
        file(
            "src/ValidationProgram.g.cs",
            format!(
                "{}internal static class ValidationProgram\n{{\n    internal static readonly JsonElement Data = Load();\n    private static JsonElement Load()\n    {{\n        using var stream = typeof(ValidationProgram).Assembly.GetManifestResourceStream(\"Suspect.ValidationProgram.json\") ?? throw new InvalidOperationException(\"Embedded validation program missing\");\n        using var document = JsonDocument.Parse(stream, new JsonDocumentOptions {{ MaxDepth = 256 }});\n        return document.RootElement.Clone();\n    }}\n}}\n",
                header(plan)
            ),
        ),
        file("Suspect.csproj", project(plan).replace("<Compile Include=\"src/**/*.cs\" />", "<Compile Include=\"src/**/*.cs\" />\n    <EmbeddedResource Include=\"protocol-plan.json\" LogicalName=\"Suspect.ProtocolPlan.json\" />")),
    ];
    if plan.credential_env().is_some() {
        files.push(file(
            "src/CredentialEnvironment.cs",
            include_str!("CredentialEnvironment.cs")
                .replace("__NAMESPACE__", &plan.config.namespace),
        ));
    }
    if resources {
        files.retain(|file| file.path != "csharp/src/ValidationRuntime.cs");
        files.extend(super::resources::runtime().into_iter().map(|(path, text)| {
            file(
                &format!("src/{path}"),
                text.replace("__NAMESPACE__", &plan.config.namespace),
            )
        }));
    } else if scoped {
        files.push(file(
            "src/ValidationProgramGuard.cs",
            include_str!("ValidationProgramGuard.cs")
                .replace("__NAMESPACE__", &plan.config.namespace),
        ));
    }
    files.extend(super::docs::render(plan));
    if files.iter().map(|file| file.content.len()).sum::<usize>() > 256 * 1024 * 1024 {
        return Err(vec![super::diagnostic(
            &plan.contract,
            plan.operations[0].source.clone(),
            "csharp-artifact-size",
            "the complete C# source package exceeds the 256 MiB artifact ceiling",
        )]);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
pub(super) fn file(path: &str, content: String) -> OutFile {
    OutFile {
        path: format!("csharp/{path}"),
        content,
    }
}
pub(super) fn header(plan: &SdkPlan) -> String {
    format!(
        "// <auto-generated />\n#nullable enable\n#pragma warning disable CS0618\nusing global::System;\nusing global::System.Collections.Generic;\nusing global::System.Linq;\nusing global::System.Text.Json;\nusing global::System.Threading;\nusing global::System.Threading.Tasks;\nusing global::System.Net.Http;\n\nnamespace {};\n\n",
        plan.config.namespace
    )
}
/// C# ordinary string literal, with every non-ASCII character emitted as UTF-16
/// escapes. JSON's non-BMP spelling and raw newlines are never source code.
pub(super) fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut units = [0_u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}
pub(super) fn xml(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            c if c.is_control()
                || matches!(c, '\u{2028}' | '\u{2029}' | '\u{fffe}' | '\u{ffff}') =>
            {
                " ".into()
            }
            c => c.to_string(),
        })
        .collect()
}
pub(super) fn source(id: &SourceId) -> String {
    format!("{}#{}", id.document(), id.pointer())
}
fn model_source(plan: &SdkPlan) -> String {
    let mut out = header(plan);
    for (key, decl) in &plan.models.declarations {
        if matches!(decl, CsDecl::Alias(_)) {
            continue;
        }
        let name = &plan.models.names[key];
        let raw = plan.contract.source(&key.0).expect("model source");
        out.push_str(&format!(
            "/// <summary>{} Source: {}.</summary>\n",
            xml(raw
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("Source-declared native model.")),
            xml(&source(&key.0))
        ));
        if plan.models.is_deprecated(key) {
            out.push_str("[global::System.Obsolete(\"Deprecated in the source contract\")]\n");
        }
        match decl {
            CsDecl::Record { fields, extras } => {
                out.push_str(&format!("public sealed record {name}\n{{\n"));
                for field in fields {
                    let ty = plan.models.render_type(&field.ty);
                    out.push_str(&format!(
                        "    /// <summary>{} Wire: {}. {} Source: {}.</summary>\n",
                        xml(&field.description),
                        xml(&field.wire),
                        if field.required {
                            "Required."
                        } else {
                            "Default is absent; present null requires a nullable value type."
                        },
                        xml(&source(&field.source))
                    ));
                    match plan.models.field_initialization(field) {
                        super::models::FieldInitialization::Singleton {
                            symbol, member, ..
                        } => {
                            let initial = format!("{}.{member}", plan.models.names[&symbol]);
                            out.push_str(&format!(
                                "    public {ty} {} {{ get; set; }} = {initial};\n",
                                field.name
                            ));
                        }
                        super::models::FieldInitialization::Required => {
                            out.push_str(&format!(
                                "    public required {ty} {} {{ get; set; }}\n",
                                field.name
                            ));
                        }
                        super::models::FieldInitialization::Absent => {
                            out.push_str(&format!(
                                "    public Optional<{ty}> {} {{ get; set; }}\n",
                                field.name
                            ));
                        }
                    }
                }
                if let Some(extra) = extras {
                    out.push_str(&format!("    /// <summary>Undeclared wire properties. Codecs reject collisions with declared keys and validate mutations.</summary>\n    public global::System.Collections.Generic.Dictionary<string, {}> Extra {{ get; set; }} = new(global::System.StringComparer.Ordinal);\n", plan.models.render_type(extra)));
                }
                out.push_str("}\n\n");
            }
            CsDecl::Literals { values } => {
                out.push_str(&format!("public enum {name}\n{{\n"));
                for (member, token) in values {
                    out.push_str(&format!(
                        "    /// <summary>Exact source literal: {}.</summary>\n    {member},\n",
                        xml(token)
                    ));
                }
                out.push_str("}\n\n");
            }
            CsDecl::Union { branches } => {
                out.push_str(&format!(
                    "public abstract class {name}\n{{\n    private {name}() {{ }}\n"
                ));
                for branch in branches {
                    let ty = plan
                        .models
                        .qualified_type(&branch.ty, &plan.config.namespace);
                    out.push_str(&format!("    /// <summary>Source union branch {}. Membership is codec-validated.</summary>\n    public sealed class {} : {name}\n    {{\n        /// <summary>The typed branch payload.</summary>\n        public {ty} Value {{ get; }}\n        /// <summary>Construct this source branch; encoding revalidates its payload.</summary>\n        public {}({ty} value) {{ Value = value; }}\n    }}\n", xml(&source(&branch.source)), branch.name, branch.name));
                }
                out.push_str("}\n\n");
            }
            CsDecl::Alias(_) => unreachable!(),
        }
    }
    out
}
fn codecs_source(plan: &SdkPlan) -> String {
    let mut out = header(plan);
    out.push_str("/// <summary>Source-aware, bounded codecs. Encode validates current mutable models; numeric tokens remain exact.</summary>\npublic static class Codecs\n{\n");
    for (key, decl) in &plan.models.declarations {
        let name = &plan.models.names[key];
        let ty = plan.models.native_type(&key.0);
        let index = plan.indices[&key.0];
        if let Some(resources) = &plan.program.resource_context {
            let scope = &resources.node_scopes[index];
            let resource = &resources.resources[scope.0];
            out.push_str(&format!("    /// <remarks>Indexed schema resource: {}. Logical address: {}. Physical resource: {}#{}. Dynamic targets use outermost entered bindings; logical identifiers never authorize acquisition.</remarks>\n", xml(&resource.canonical_uri), xml(&scope.2), xml(&resource.source.document), xml(&resource.source.pointer)));
        }
        if matches!(
            plan.program.version,
            suspect_schema::OwnedProgram::V2_VERSION | suspect_schema::OwnedProgram::V3_VERSION
        ) && plan.models.is_json_carrier(&key.0)
        {
            out.push_str("    /// <remarks>This JSON-value carrier is source-validated on decode and encode. Native construction alone does not prove its scoped or conditional assertions.</remarks>\n");
        }
        out.push_str(&format!("    /// <summary>Validate and decode {} from UTF-8 JSON. Source: {}.</summary>\n    public static {ty} Decode{name}(ReadOnlySpan<byte> json) => CodecRuntime.Decode(json, {index}, Read_{name});\n    /// <summary>Validate and decode {} from Unicode JSON text.</summary>\n    public static {ty} Decode{name}(string json) => Decode{name}(JsonRuntime.Bytes(json));\n    /// <summary>Validate the current native value and encode bounded UTF-8 JSON, retaining numeric tokens.</summary>\n    public static byte[] Encode{name}({ty} value) => CodecRuntime.Encode(value, {index}, Write_{name});\n", xml(&ty), xml(&source(&key.0)), xml(&ty)));
        out.push_str(&read_decl(plan, key, decl));
        out.push_str(&write_decl(plan, key, decl));
    }
    out.push_str("}\n");
    out
}
fn read_decl(plan: &SdkPlan, key: &Key, decl: &CsDecl) -> String {
    let name = &plan.models.names[key];
    let ty = plan.models.native_type(&key.0);
    let mut out = format!(
        "    private static {ty} Read_{name}(JsonElement value, ConversionContext context, string path)\n    {{\n        using var scope = context.EnterRead(path);\n"
    );
    if plan.models.nullable[&key.0] && !matches!(decl, CsDecl::Alias(_)) {
        out.push_str("        if (value.ValueKind == JsonValueKind.Null) return null;\n");
    }
    match decl {
        CsDecl::Alias(inner) => {
            out.push_str(&format!(
                "        return {};\n",
                read_type(plan, inner, "value", "context", "path", 0)
            ));
        }
        CsDecl::Literals { values } => {
            for (member, token) in values {
                let literal: Value = serde_json::from_str(token).expect("literal JSON");
                let test = match literal {
                    Value::String(s) => format!(
                        "value.ValueKind == JsonValueKind.String && value.GetString() == {}",
                        quote(&s)
                    ),
                    Value::Bool(v) => format!(
                        "value.ValueKind == JsonValueKind.{}",
                        if v { "True" } else { "False" }
                    ),
                    _ => unreachable!(),
                };
                out.push_str(&format!("        if ({test}) return {name}.{member};\n"));
            }
            out.push_str("        throw new CodecException(CodecErrorKind.Conversion, \"Unknown native literal\", path: path);\n");
        }
        CsDecl::Union { branches } => {
            for branch in branches {
                out.push_str(&format!("        if (context.Validation.Matches({}, value, path)) return new {name}.{}({});\n", plan.indices[&branch.source], branch.name, read_type(plan, &branch.ty, "value", "context", "path", 0)));
            }
            out.push_str("        throw new CodecException(CodecErrorKind.Conversion, \"No source union branch matches\", path: path);\n");
        }
        CsDecl::Record { fields, extras } => {
            out.push_str(&format!("        var result = new {name}\n        {{\n"));
            for (i, field) in fields.iter().enumerate() {
                let path = format!("JsonRuntime.Child(path, {})", quote(&field.wire));
                let expr = if field.required {
                    read_type(
                        plan,
                        &field.ty,
                        &format!("value.GetProperty({})", quote(&field.wire)),
                        "context",
                        &path,
                        0,
                    )
                } else {
                    format!(
                        "value.TryGetProperty({}, out var p{i}) ? Optional<{}>.Present({}) : default",
                        quote(&field.wire),
                        plan.models.render_type(&field.ty),
                        read_type(plan, &field.ty, &format!("p{i}"), "context", &path, 0)
                    )
                };
                out.push_str(&format!("            {} = {expr},\n", field.name));
            }
            out.push_str("        };\n");
            if let Some(extra) = extras {
                out.push_str(
                    "        foreach (var property in value.EnumerateObject())\n        {\n",
                );
                if !fields.is_empty() {
                    out.push_str(&format!(
                        "            if (property.Name is {}) continue;\n",
                        fields
                            .iter()
                            .map(|f| quote(&f.wire))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    ));
                }
                out.push_str(&format!("            result.Extra.Add(context.String(property.Name, path), {});\n        }}\n", read_type(plan, extra, "property.Value", "context", "JsonRuntime.Child(path, property.Name)", 0)));
            }
            out.push_str("        return result;\n");
        }
    }
    out.push_str("    }\n");
    out
}
fn read_type(
    plan: &SdkPlan,
    ty: &CsType,
    value: &str,
    context: &str,
    path: &str,
    depth: usize,
) -> String {
    match ty {
        CsType::Native("string") => format!("{context}.String({value}.GetString(), {path})"),
        CsType::Native("bool") => format!("{context}.Scalar({value}.GetBoolean(), {path})"),
        CsType::Native("JsonNull") => format!("{context}.Scalar(default(JsonNull), {path})"),
        CsType::Native("Never") => format!("CodecRuntime.ReadNever({path})"),
        CsType::Native(_) => unreachable!(),
        CsType::Number => format!("new JsonNumber({context}.Number({value}.GetRawText(), {path}))"),
        CsType::Integer => {
            format!("new JsonInteger({context}.Number({value}.GetRawText(), {path}))")
        }
        CsType::Json => format!("JsonRuntime.CloneElement({value}, {context}, {path})"),
        CsType::Named(key) => format!(
            "Read_{}({value}, {context}, {path})",
            plan.models.names[key]
        ),
        CsType::Nullable(inner) => format!(
            "({value}.ValueKind == JsonValueKind.Null ? default({}) : ({} )({}))",
            plan.models.render_type(ty),
            plan.models.render_type(ty),
            read_type(plan, inner, value, context, path, depth + 1)
        ),
        CsType::List(inner) | CsType::Dict(inner) => {
            let (v, c, p) = (
                format!("e{depth}"),
                format!("c{depth}"),
                format!("p{depth}"),
            );
            format!(
                "CodecRuntime.Read{}({value}, {context}, {path}, static ({v}, {c}, {p}) => {})",
                if matches!(ty, CsType::List(_)) {
                    "List"
                } else {
                    "Map"
                },
                read_type(plan, inner, &v, &c, &p, depth + 1)
            )
        }
    }
}
fn write_decl(plan: &SdkPlan, key: &Key, decl: &CsDecl) -> String {
    let name = &plan.models.names[key];
    let ty = plan.models.native_type(&key.0);
    let mut out = format!(
        "    private static void Write_{name}(Utf8JsonWriter writer, {ty} value, ConversionContext context, string path)\n    {{\n"
    );
    if plan.models.nullable[&key.0] && !matches!(decl, CsDecl::Alias(_)) {
        out.push_str("        if (value is null) { writer.WriteNullValue(); return; }\n");
    }
    match decl {
        CsDecl::Alias(inner) => {
            out.push_str(&format!(
                "        {}\n",
                write_type(plan, inner, "writer", "value", "context", "path", 0)
            ));
        }
        CsDecl::Literals { values } => {
            out.push_str("        switch (value)\n        {\n");
            for (member, token) in values {
                out.push_str(&format!(
                    "            case {name}.{member}: context.Spend({}, path); writer.WriteRawValue({}, true); return;\n",
                    token.len(), quote(token)
                ));
            }
            out.push_str("            default: throw new CodecException(CodecErrorKind.Conversion, \"Unknown native enum value\", path: path);\n        }\n");
        }
        CsDecl::Union { branches } => {
            out.push_str("        using var scope = context.Enter(value, path);\n        switch (value)\n        {\n");
            for (i, branch) in branches.iter().enumerate() {
                out.push_str(&format!("            case {name}.{} b{i}:\n                CodecRuntime.WriteBranch(writer, b{i}.Value, context, path, {}, static (w, v, c, p) => {{ {} }}); return;\n", branch.name, plan.indices[&branch.source], write_type(plan, &branch.ty, "w", "v", "c", "p", 0)));
            }
            out.push_str("            default: throw new CodecException(CodecErrorKind.Conversion, \"Unknown native union branch\", path: path);\n        }\n");
        }
        CsDecl::Record { fields, extras } => {
            out.push_str("        using var scope = context.Enter(value, path);\n        writer.WriteStartObject();\n");
            for field in fields {
                let value = format!(
                    "value.{}{}",
                    field.name,
                    if field.required { "" } else { ".Value" }
                );
                let path = format!("JsonRuntime.Child(path, {})", quote(&field.wire));
                if !field.required {
                    out.push_str(&format!(
                        "        if (value.{}.HasValue)\n        {{\n",
                        field.name
                    ));
                }
                out.push_str(&format!(
                    "            writer.WritePropertyName(context.String({}, path)); {}\n",
                    quote(&field.wire),
                    write_type(plan, &field.ty, "writer", &value, "context", &path, 0)
                ));
                if !field.required {
                    out.push_str("        }\n");
                }
            }
            if let Some(extra) = extras {
                out.push_str("        using var extraScope = context.Enter(value.Extra, path);\n        context.Spend(value.Extra.Count, path);\n        var keys = new List<string>(value.Extra.Keys); keys.Sort(StringComparer.Ordinal);\n        foreach (var key in keys)\n        {\n");
                if !fields.is_empty() {
                    out.push_str(&format!("            if (key is {}) throw new CodecException(CodecErrorKind.Conversion, \"Extra property shadows a declared wire name\", path: JsonRuntime.Child(path, key));\n", fields.iter().map(|f| quote(&f.wire)).collect::<Vec<_>>().join(" or ")));
                }
                out.push_str(&format!("            writer.WritePropertyName(context.String(key, path)); {}\n        }}\n", write_type(plan, extra, "writer", "value.Extra[key]", "context", "JsonRuntime.Child(path, key)", 0)));
            }
            out.push_str("        writer.WriteEndObject();\n");
        }
    }
    out.push_str("    }\n");
    out
}
fn value_type(plan: &SdkPlan, ty: &CsType) -> bool {
    match ty {
        CsType::Native("bool" | "JsonNull") | CsType::Number | CsType::Integer | CsType::Json => {
            true
        }
        CsType::Nullable(inner) => value_type(plan, inner),
        CsType::Named(key) => match &plan.models.declarations[key] {
            CsDecl::Literals { .. } => true,
            CsDecl::Alias(inner) => value_type(plan, inner),
            _ => false,
        },
        _ => false,
    }
}
fn write_type(
    plan: &SdkPlan,
    ty: &CsType,
    writer: &str,
    value: &str,
    context: &str,
    path: &str,
    depth: usize,
) -> String {
    match ty {
        CsType::Native("string") => {
            format!("{writer}.WriteStringValue({context}.String({value}, {path}));")
        }
        CsType::Native("bool") => {
            format!("{writer}.WriteBooleanValue({context}.Scalar({value}, {path}));")
        }
        CsType::Native("JsonNull") => {
            format!("{context}.Spend(1, {path}); {writer}.WriteNullValue();")
        }
        CsType::Native("Never") => format!(
            "throw new CodecException(CodecErrorKind.Conversion, \"Uninhabited source value\", path: {path});"
        ),
        CsType::Native(_) => unreachable!(),
        CsType::Number | CsType::Integer => {
            format!("{writer}.WriteRawValue({context}.Number({value}.Token, {path}), true);")
        }
        CsType::Json => format!("JsonRuntime.WriteElement({writer}, {value}, {context}, {path});"),
        CsType::Named(key) => format!(
            "Write_{}({writer}, {value}, {context}, {path});",
            plan.models.names[key]
        ),
        CsType::Nullable(inner) => {
            let unwrapped = if value_type(plan, inner) {
                format!("{value}.Value")
            } else {
                format!("{value}!")
            };
            format!(
                "if ({value} is null) {{ {writer}.WriteNullValue(); }} else {{ {} }}",
                write_type(plan, inner, writer, &unwrapped, context, path, depth + 1)
            )
        }
        CsType::List(inner) | CsType::Dict(inner) => {
            let (w, v, c, p) = (
                format!("w{depth}"),
                format!("v{depth}"),
                format!("c{depth}"),
                format!("p{depth}"),
            );
            format!(
                "CodecRuntime.Write{}({writer}, {value}, {context}, {path}, static ({w}, {v}, {c}, {p}) => {{ {} }});",
                if matches!(ty, CsType::List(_)) {
                    "List"
                } else {
                    "Map"
                },
                write_type(plan, inner, &w, &v, &c, &p, depth + 1)
            )
        }
    }
}
fn project(plan: &SdkPlan) -> String {
    format!(
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net8.0</TargetFramework>\n    <LangVersion>12.0</LangVersion>\n    <Nullable>enable</Nullable>\n    <ImplicitUsings>disable</ImplicitUsings>\n    <TreatWarningsAsErrors>true</TreatWarningsAsErrors>\n    <GenerateDocumentationFile>true</GenerateDocumentationFile>\n    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>\n    <Deterministic>true</Deterministic>\n    <PackageId>{}</PackageId>\n    <AssemblyName>{}</AssemblyName>\n    <Version>{}</Version>\n    <RootNamespace>{}</RootNamespace>\n    <Description>Source-selected OpenAPI .NET 8 SDK with exact validated JSON codecs.</Description>\n    <Authors>Suspect generator</Authors>\n    <PackageReadmeFile>README.md</PackageReadmeFile>\n  </PropertyGroup>\n  <ItemGroup>\n    <Compile Include=\"src/**/*.cs\" />\n    <EmbeddedResource Include=\"validation-program.json\" LogicalName=\"Suspect.ValidationProgram.json\" />\n    <None Include=\"README.md\" Pack=\"true\" PackagePath=\"/\" />\n    <None Include=\"docs/**/*\" Pack=\"true\" PackagePath=\"docs/\" />\n    <None Include=\"examples/**/*\" Exclude=\"examples/**/obj/**/*;examples/**/bin/**/*\" Pack=\"true\" PackagePath=\"examples/\" />\n    <None Include=\"http-manifest.json\" Pack=\"true\" PackagePath=\"/\" />\n  </ItemGroup>\n</Project>\n",
        xml(&plan.config.name),
        xml(&plan.config.name),
        xml(&plan.config.version),
        xml(&plan.config.namespace)
    )
}
