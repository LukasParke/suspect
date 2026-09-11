//! Native HTTP declarations and codecs from the admitted C# protocol projection.
use super::{
    PlannedResponse, SdkPlan,
    emit::{header, quote, source, xml},
    protocol::{PlannedHeader, PlannedMedia, PlannedPart, PlannedParts},
};
use crate::http_protocol as p;

pub(super) fn render(plan: &SdkPlan) -> String {
    let mut out = header(plan);
    out.push_str("/// <summary>Explicit caller credentials, allocated by source scheme identity. Properties are independent of OR-alternative selection.</summary>\npublic sealed record Credentials\n{\n");
    for c in &plan.credential_bindings {
        out.push_str(&format!("    /// <summary>Source scheme {}. {}.</summary>\n    public {}? {} {{ get; init; }}\n",xml(c.requirement.name()),xml(&source(c.requirement.scheme().terminal().source())),c.native_type,c.property_name));
    }
    out.push_str("    internal object? Get(string source) => source switch\n    {\n");
    for c in &plan.credential_bindings {
        out.push_str(&format!(
            "        {} => {},\n",
            quote(&c.key),
            c.property_name
        ));
    }
    out.push_str("        _ => null\n    };\n}\n\n");
    for op in &plan.operations {
        out.push_str(&format!("/// <summary>Native inputs for {}. Source: {}.</summary>\npublic sealed record {}\n{{\n",xml(&op.operation_id),xml(&source(&op.source)),op.input_type));
        for p in &op.parameters {
            field(
                &mut out,
                &p.property_name,
                &p.native_type,
                p.required,
                &format!("{:?} parameter {}", p.location, p.wire_name),
            );
        }
        if let Some(body) = &op.body {
            field(
                &mut out,
                "Body",
                &body.native_type,
                body.required,
                "Source-selected request body; omission and JSON null are distinct",
            );
        }
        out.push_str("}\n\n");
        if let Some(body) = &op.body {
            for m in &body.media {
                if let Some(parts) = &m.parts {
                    parts_decl(&mut out, parts);
                }
                if let Some(parts) = &m.positional {
                    super::positional::declaration(&mut out, parts);
                }
            }
            if body.union {
                body_union(plan, &mut out, &body.native_type, &body.media, false, false);
            }
        }
        if !op.direct_result() {
            out.push_str(&format!("/// <summary>Closed declared success alternatives for {}.</summary>\npublic abstract class {} : IAsyncDisposable\n{{\n    private {}(ResponseMetadata metadata) {{ Metadata = metadata; }}\n    /// <summary>Actual status and source-backed metadata.</summary>\n    public ResponseMetadata Metadata {{ get; }}\n    /// <summary>Actual successful status.</summary>\n    public int Status => Metadata.Status;\n    /// <summary>Dispose any unconsumed sequential body.</summary>\n    public virtual ValueTask DisposeAsync() => ValueTask.CompletedTask;\n",xml(&op.operation_id),op.result_type,op.result_type));
            for r in op.responses.iter().filter(|r| r.may_succeed()) {
                result_decl(plan, &mut out, r, true, Some(&op.result_type));
            }
            out.push_str("}\n\n");
        } else {
            for r in op.responses.iter().filter(|r| r.may_succeed()) {
                result_decl(plan, &mut out, r, true, None);
            }
        }
        out.push_str(&format!("/// <summary>Source-typed API failures for {}.</summary>\npublic abstract class {} : ApiException\n{{\n    private {}(WireResponse raw) : base(raw) {{ }}\n",xml(&op.operation_id),op.error_type,op.error_type));
        for r in op.responses.iter().filter(|r| r.may_fail()) {
            result_decl(plan, &mut out, r, false, Some(&op.error_type));
        }
        out.push_str("}\n\n");
        for r in &op.responses {
            if let Some(name) = &r.header_type {
                headers_decl(&mut out, name, &r.headers);
            }
            for m in &r.media {
                if let Some(parts) = &m.parts {
                    parts_decl(&mut out, parts);
                }
                if let Some(parts) = &m.positional {
                    super::positional::declaration(&mut out, parts);
                }
            }
            if r.union {
                body_union(
                    plan,
                    &mut out,
                    &r.native_type,
                    &r.media,
                    r.may_be_empty,
                    r.media.is_empty(),
                );
            }
        }
    }
    out.push_str("/// <summary>Source-selected Task operations. Default transport has no cookies, redirects, proxy discovery or retries.</summary>\npublic sealed class Client : IDisposable\n{\n    private readonly Credentials _credentials;\n    private readonly HttpRuntime _runtime;\n    /// <summary>Create a client; an injected HttpClient stays caller-owned.</summary>\n    public Client(Credentials credentials, ClientOptions? options = null, HttpClient? httpClient = null)\n    { _credentials = credentials ?? throw new SdkException(SdkErrorKind.RequestRepresentation); _runtime = new HttpRuntime(options, httpClient); }\n    /// <summary>Create an anonymous client. Auth-required calls still require explicit credentials.</summary>\n    public Client() : this(new Credentials()) { }\n    /// <summary>Cancel calls and release the owned transport.</summary>\n    public void Dispose() => _runtime.Dispose();\n    /// <summary>Source-backed method/server information, keyed by allocated method name.</summary>\n    public static IReadOnlyDictionary<string, OperationInfo> Operations { get; } = new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, OperationInfo>(new Dictionary<string, OperationInfo>\n    {\n");
    for (i, op) in plan.operations.iter().enumerate() {
        out.push_str(&format!(
            "        [{}] = new OperationInfo(ProtocolData.Operation({i})),\n",
            quote(&op.method_name)
        ));
    }
    out.push_str("    });\n");
    out.push_str(&super::credential_env::factory(plan));
    for (i, op) in plan.operations.iter().enumerate() {
        out.push_str(&format!("    /// <summary>{} Source: {}.</summary>\n    public Task<{}> {}({} input, RequestOptions? requestOptions = null, CancellationToken cancellationToken = default)\n    {{\n        return _runtime.CallAsync<{}>({i}, requestOptions, cancellationToken, () =>\n        {{\n            if (input is null) throw new SdkException(SdkErrorKind.RequestRepresentation);\n            var operation = ProtocolData.Operation({i});\n            var request = new WireRequest();\n",xml(&op.description),xml(&source(&op.source)),op.result_type,op.method_name,op.input_type,op.result_type));
        for (j, p) in op.parameters.iter().enumerate() {
            out.push_str(&format!("            {}request.Add(operation.GetProperty(\"parameters\")[{j}], Codecs.Encode{}(input.{}{}));\n",if p.required{String::new()}else{format!("if (input.{}.HasValue) ",p.property_name)},plan.models.codec_name(&p.schema),p.property_name,if p.required{""}else{".Value"}));
        }
        if let Some(body) = &op.body {
            if !body.required {
                out.push_str("            if (input.Body.HasValue)\n            {\n");
            }
            let value = if body.required {
                "input.Body"
            } else {
                "input.Body.Value"
            };
            if body.union {
                out.push_str(&format!("            switch ({value})\n            {{\n"));
                for (j, m) in body.media.iter().enumerate() {
                    out.push_str(&format!(
                        "                case {}.{} value{j}: request.SetBody({}); break;\n",
                        body.native_type,
                        m.variant_name,
                        encode_media(
                            plan,
                            m,
                            &format!("value{j}.Value"),
                            &format!("operation.GetProperty(\"body\").GetProperty(\"media\")[{j}]"),
                            j,
                            &format!("value{j}.ContentType")
                        )
                    ));
                }
                out.push_str("                default: throw new SdkException(SdkErrorKind.RequestRepresentation);\n            }\n");
            } else {
                out.push_str(&format!(
                    "            request.SetBody({});\n",
                    encode_media(
                        plan,
                        &body.media[0],
                        value,
                        "operation.GetProperty(\"body\").GetProperty(\"media\")[0]",
                        0,
                        "requestOptions?.ContentType"
                    )
                ));
            }
            if !body.required {
                out.push_str("            }\n");
            }
        }
        out.push_str("            return request;\n        }, static raw =>\n        {\n            switch (raw.ResponseIndex)\n            {\n");
        for (j, r) in op.responses.iter().enumerate() {
            out.push_str(&format!("                case {j}:\n                {{\n"));
            if let Some(header_type) = &r.header_type {
                out.push_str(&format!("                    var headers = HttpCodecs.Read{header_type}(raw.Response.GetProperty(\"headers\"), raw.Metadata.Headers);\n"));
            }
            out.push_str(&format!("                    {} data;\n", r.native_type));
            if r.always_empty {
                out.push_str("                    data = default;\n");
            } else if r.union {
                if r.may_be_empty {
                    out.push_str(&format!("                    if (raw.Forbidden) data = new {}.NoContent(default);\n                    else\n",r.native_type));
                }
                if r.media.is_empty() {
                    out.push_str(&format!(
                        "                    data = new {}.UndeclaredBytes(raw.Body);\n",
                        r.native_type
                    ));
                } else {
                    out.push_str(
                        "                    data = raw.MediaIndex switch\n                    {\n",
                    );
                    for (k, m) in r.media.iter().enumerate() {
                        out.push_str(&format!(
                            "                        {k} => new {}.{}({}),\n",
                            r.native_type,
                            m.variant_name,
                            decode_media(plan, m, "raw")
                        ));
                    }
                    out.push_str("                        _ => throw new UnexpectedResponseException(raw)\n                    };\n");
                }
            } else if r.media.is_empty() {
                out.push_str("                    data = raw.Body;\n");
            } else {
                out.push_str(&format!(
                    "                    data = {};\n",
                    decode_media(plan, &r.media[0], "raw")
                ));
            }
            let headers = if r.headers.is_empty() {
                ""
            } else {
                ", headers"
            };
            let lease = if r.media.iter().any(PlannedMedia::is_stream) {
                ", raw.Lease"
            } else {
                ""
            };
            if r.may_succeed() {
                out.push_str(&format!(
                    "                    {}return new {}(data, raw.Metadata{headers}{lease});\n",
                    if r.may_fail() {
                        "if (raw.Metadata.Status is >= 200 and < 300) "
                    } else {
                        ""
                    },
                    r.type_name
                ));
            }
            if r.may_fail() {
                out.push_str(&format!(
                    "                    throw new {}(data, raw{headers});\n",
                    r.error_type_name
                ));
            }
            out.push_str("                }\n");
        }
        out.push_str("                default: throw new UnexpectedResponseException(raw);\n            }\n        }, _credentials.Get);\n    }\n");
        if op.has_no_input_overload() {
            out.push_str(&format!("    /// <summary>Call with optional inputs absent.</summary>\n    public Task<{}> {}(CancellationToken cancellationToken = default) => {}(new {}(), cancellationToken: cancellationToken);\n",op.result_type,op.method_name,op.method_name,op.input_type));
        }
    }
    out.push_str("}\n\ninternal static class HttpCodecs\n{\n");
    for op in &plan.operations {
        for media in op
            .body
            .iter()
            .flat_map(|b| &b.media)
            .chain(op.responses.iter().flat_map(|r| &r.media))
        {
            if let Some(parts) = &media.parts {
                parts_codecs(plan, &mut out, parts);
            }
            if let Some(parts) = &media.positional {
                super::positional::codecs(plan, &mut out, parts);
            }
        }
        for r in &op.responses {
            if let Some(name) = &r.header_type {
                headers_codecs(plan, &mut out, name, &r.headers);
            }
        }
    }
    out.push_str("}\n");
    out
}
fn field(out: &mut String, name: &str, ty: &str, required: bool, description: &str) {
    out.push_str(&format!(
        "    /// <summary>{}.</summary>\n    public {}{} {name} {{ get; set; }}\n",
        xml(description),
        if required { "required " } else { "" },
        if required {
            ty.to_owned()
        } else {
            format!("Optional<{ty}>")
        }
    ));
}
fn headers_decl(out: &mut String, name: &str, headers: &[PlannedHeader]) {
    out.push_str(&format!(
        "/// <summary>Source-typed headers.</summary>\npublic sealed record {name}\n{{\n"
    ));
    for h in headers {
        field(
            out,
            &h.property_name,
            &h.native_type,
            h.wire.required(),
            &format!("Wire header {}", h.wire.name()),
        );
    }
    out.push_str("}\n\n");
}
fn parts_decl(out: &mut String, parts: &PlannedParts) {
    out.push_str(&format!("/// <summary>Source named {} fields. Structural rules are checked before transport.</summary>\npublic sealed record {}\n{{\n",if parts.multipart{"multipart"}else{"form"},parts.native_type));
    for p in &parts.fields {
        field(
            out,
            &p.property_name,
            &p.native_type,
            p.wire.required(),
            &format!("Wire part {}", p.wire.name().unwrap_or("additional")),
        );
    }
    if let Some(extra) = &parts.additional {
        out.push_str(&format!("    /// <summary>Source-typed additional fields.</summary>\n    public Dictionary<string, {}> Extra {{ get; set; }} = new(StringComparer.Ordinal);\n",extra.native_type));
    }
    out.push_str("}\n\n");
    for p in parts
        .fields
        .iter()
        .chain(parts.additional.iter().map(|p| p.as_ref()))
    {
        part_decl(out, p);
    }
}
pub(super) fn part_decl(out: &mut String, p: &PlannedPart) {
    if let Some(wrapper) = &p.wrapper_type {
        out.push_str(&format!("/// <summary>A named multipart value with explicit metadata.</summary>\npublic sealed record {wrapper}\n{{\n    /// <summary>Actual typed payload; binary values are bytes.</summary>\n    public required {} Value {{ get; set; }}\n    /// <summary>Optional caller filename, never a filesystem path to read.</summary>\n    public string? FileName {{ get; set; }}\n    /// <summary>Concrete part media; required when source choices are ambiguous.</summary>\n    public string? ContentType {{ get; set; }}\n",p.value_type));
        if let Some(headers) = &p.header_type {
            out.push_str(&format!("    /// <summary>Source-declared typed part headers.</summary>\n    public {}{headers} Headers {{ get; set; }}{}\n",if p.headers.iter().any(|h|h.wire.required()){"required "}else{""},if p.headers.iter().any(|h|h.wire.required()){String::new()}else{format!(" = new {headers}();")}));
        }
        out.push_str("}\n\n");
    }
    if let Some(name) = &p.header_type {
        headers_decl(out, name, &p.headers);
    }
}
fn body_union(
    plan: &SdkPlan,
    out: &mut String,
    name: &str,
    media: &[PlannedMedia],
    none: bool,
    bytes: bool,
) {
    out.push_str(&format!("/// <summary>Explicit source media/body alternatives.</summary>\npublic abstract class {name}\n{{\n    private {name}() {{ }}\n"));
    for m in media {
        let ty = qualified_media(plan, m);
        out.push_str(&format!("    /// <summary>Source media {}.</summary>\n    public sealed class {} : {name}\n    {{\n        /// <summary>The native payload.</summary>\n        public {ty} Value {{ get; }}\n        /// <summary>Optional concrete media override; wildcard media require one.</summary>\n        public string? ContentType {{ get; }}\n        /// <summary>Choose this declared representation.</summary>\n        public {}({ty} value, string? contentType = null) {{ Value = value; ContentType = contentType; }}\n    }}\n",xml(m.wire.media_type().declared()),m.variant_name,m.variant_name));
    }
    for (emit, variant, ty) in [
        (none, "NoContent", "HttpNoContent"),
        (bytes, "UndeclaredBytes", "byte[]"),
    ] {
        if emit {
            out.push_str(&format!("    /// <summary>Explicit {variant} body disposition.</summary>\n    public sealed class {variant} : {name}\n    {{\n        /// <summary>The native body value.</summary>\n        public {ty} Value {{ get; }}\n        /// <summary>Construct this body disposition.</summary>\n        public {variant}({ty} value) {{ Value = value; }}\n    }}\n"));
        }
    }
    out.push_str("}\n\n");
}
fn result_decl(
    plan: &SdkPlan,
    out: &mut String,
    r: &PlannedResponse,
    success: bool,
    parent: Option<&str>,
) {
    let full = if success {
        &r.type_name
    } else {
        &r.error_type_name
    };
    let name = if parent.is_some() {
        full.rsplit('.').next().unwrap()
    } else {
        full
    };
    let indent = if parent.is_some() { "    " } else { "" };
    let stream = success && r.media.iter().any(PlannedMedia::is_stream);
    let native_type = if r.union || r.always_empty {
        format!("global::{}.{}", plan.config.namespace, r.native_type)
    } else if r.media.is_empty() {
        "byte[]".into()
    } else {
        qualified_media(plan, &r.media[0])
    };
    out.push_str(&format!("{indent}/// <summary>Declared {} response; actual status is retained. Source: {}.</summary>\n{indent}public sealed class {name}{}\n{indent}{{\n{indent}    /// <summary>Source-typed data; null and no-content are distinct.</summary>\n{indent}    public {}{native_type} Data {{ get; }}\n",xml(r.wire.status_key()),xml(&source(&r.source)),parent.map(|p|format!(" : {p}")).unwrap_or_else(||" : IAsyncDisposable".into()),if success{""}else{"new "}));
    if success && parent.is_none() {
        out.push_str(&format!("{indent}    /// <summary>Actual status, headers and Links.</summary>\n{indent}    public ResponseMetadata Metadata {{ get; }}\n{indent}    /// <summary>Actual success status.</summary>\n{indent}    public int Status => Metadata.Status;\n"));
    }
    if let Some(headers) = &r.header_type {
        out.push_str(&format!("{indent}    /// <summary>Decoded, validated response headers.</summary>\n{indent}    public {headers} Headers {{ get; }}\n"));
    }
    if stream {
        out.push_str(&format!(
            "{indent}    private readonly StreamLease? _lease;\n"
        ));
    }
    let header_arg = r
        .header_type
        .as_ref()
        .map(|t| format!(", {t} headers"))
        .unwrap_or_default();
    let lease_arg = if stream { ", StreamLease? lease" } else { "" };
    out.push_str(&format!("{indent}    internal {name}({native_type} data, {}{header_arg}{lease_arg}){} {{ Data = data; {}{}{} }}\n",if success{"ResponseMetadata metadata"}else{"WireResponse raw"},if parent.is_some(){if success{" : base(metadata)"}else{" : base(raw)"}}else{""},if success&&parent.is_none(){"Metadata = metadata; "}else{""},if r.header_type.is_some(){"Headers = headers; "}else{""},if stream{"_lease = lease;"}else{""}));
    if success && (parent.is_none() || stream) {
        out.push_str(&format!("{indent}    /// <summary>Release an unconsumed sequential body.</summary>\n{indent}    public {}ValueTask DisposeAsync() {{ {}return ValueTask.CompletedTask; }}\n",if parent.is_some(){"override "}else{""},if stream{"_lease?.Close(true); "}else{""}));
    }
    out.push_str(&format!("{indent}}}\n\n"));
}
fn qualified_media(plan: &SdkPlan, m: &PlannedMedia) -> String {
    if let Some(schema) = m.schema() {
        let ty = plan.models.qualified_type(
            &super::models::CsType::Named(super::models::key(schema)),
            &plan.config.namespace,
        );
        if m.is_stream() {
            format!("global::{}.HttpStream<{ty}>", plan.config.namespace)
        } else {
            ty
        }
    } else if m.parts.is_some() || m.positional.is_some() {
        format!("global::{}.{}", plan.config.namespace, m.native_type)
    } else {
        m.native_type.clone()
    }
}
fn encode_media(
    plan: &SdkPlan,
    m: &PlannedMedia,
    value: &str,
    descriptor: &str,
    index: usize,
    content_type: &str,
) -> String {
    if let Some(parts) = &m.parts {
        return format!(
            "HttpCodecs.Write{}({descriptor}, {index}, {value}, {content_type})",
            parts.native_type
        );
    }
    if let Some(parts) = &m.positional {
        return format!(
            "HttpCodecs.Write{}({descriptor}, {index}, {value}, {content_type})",
            parts.native_type
        );
    }
    let bytes = match m.wire.representation() {
        p::Representation::Json { codec } => codec.as_ref().map_or_else(
            || format!("ProtocolRuntime.EncodeJson({value})"),
            |c| {
                format!(
                    "Codecs.Encode{}({value})",
                    plan.models.codec_name(c.schema().id())
                )
            },
        ),
        p::Representation::Text { codec, .. } => codec.as_ref().map_or_else(
            || format!("JsonRuntime.Bytes({value})"),
            |c| {
                format!(
                    "ProtocolRuntime.TextBytes(Codecs.Encode{}({value}))",
                    plan.models.codec_name(c.schema().id())
                )
            },
        ),
        p::Representation::Binary { .. } => {
            format!("{value} ?? throw new SdkException(SdkErrorKind.RequestRepresentation)")
        }
        _ => unreachable!(),
    };
    format!("new EncodedBody({bytes}, {index}, {content_type})")
}
fn decode_media(plan: &SdkPlan, m: &PlannedMedia, raw: &str) -> String {
    if let Some(parts) = &m.parts {
        return format!(
            "HttpCodecs.Read{}({raw}.Media, {raw}.Parts())",
            parts.native_type
        );
    }
    if let Some(parts) = &m.positional {
        return format!(
            "HttpCodecs.Read{}({raw}.Media, {raw}.Positional())",
            parts.native_type
        );
    }
    match m.wire.representation() {
        p::Representation::Json { codec } | p::Representation::Text { codec, .. } => codec
            .as_ref()
            .map(|c| {
                format!(
                    "Codecs.Decode{}({raw}.JsonBody())",
                    plan.models.codec_name(c.schema().id())
                )
            })
            .unwrap_or_else(|| {
                if matches!(m.wire.representation(), p::Representation::Json { .. }) {
                    format!("{raw}.JsonValue()")
                } else {
                    format!("JsonRuntime.Utf8.GetString({raw}.Body)")
                }
            }),
        p::Representation::Binary { .. } => format!("{raw}.Body"),
        p::Representation::Stream { stream } => format!(
            "{raw}.Stream(static bytes => Codecs.Decode{}(bytes))",
            plan.models.codec_name(stream.item_codec().schema().id())
        ),
        _ => unreachable!(),
    }
}
pub(super) fn headers_codecs(
    plan: &SdkPlan,
    out: &mut String,
    name: &str,
    headers: &[PlannedHeader],
) {
    out.push_str(&format!("    internal static {name} Read{name}(JsonElement headers, IReadOnlyDictionary<string, IReadOnlyList<string>> values)\n    {{\n        return new {name}\n        {{\n"));
    for (i, h) in headers.iter().enumerate() {
        let read = format!(
            "Codecs.Decode{}(bytes{i})",
            plan.models.codec_name(h.wire.codec().schema().id())
        );
        out.push_str(&format!("            {} = ProtocolData.Header(headers[{i}], values) is {{ }} bytes{i} ? {} : {},\n",h.property_name,if h.wire.required(){read}else{format!("Optional<{}>.Present({read})",h.native_type)},if h.wire.required(){"throw new CodecException(CodecErrorKind.InvalidValue, \"Required header missing\")"}else{"default"}));
    }
    out.push_str("        };\n    }\n");
    out.push_str(&format!("    internal static Dictionary<string, IReadOnlyList<string>> Write{name}(JsonElement headers, {name} value)\n    {{\n        if (value is null) throw new SdkException(SdkErrorKind.RequestValidation);\n        var result = new Dictionary<string, IReadOnlyList<string>>(StringComparer.OrdinalIgnoreCase);\n"));
    for (i, h) in headers.iter().enumerate() {
        out.push_str(&format!("        {}result.Add({}, Array.AsReadOnly(new[] {{ WireEncoding.Serialize(headers[{i}], JsonRuntime.Parse(Codecs.Encode{}(value.{}{}))) }}));\n",if h.wire.required(){String::new()}else{format!("if (value.{}.HasValue) ",h.property_name)},quote(h.wire.name()),plan.models.codec_name(h.wire.codec().schema().id()),h.property_name,if h.wire.required(){""}else{".Value"}));
    }
    out.push_str("        return result;\n    }\n");
}
pub(super) fn part_encode(
    plan: &SdkPlan,
    part: &PlannedPart,
    value: &str,
    descriptor: &str,
    name: &str,
) -> String {
    let payload = if part.wrapper_type.is_some() {
        format!("{value}.Value")
    } else {
        value.into()
    };
    let encoded = match part.wire.representation() {
        p::PartRepresentation::Binary { .. } => {
            format!("{payload} ?? throw new SdkException(SdkErrorKind.RequestValidation)")
        }
        p::PartRepresentation::Json { codec, .. }
        | p::PartRepresentation::Text { codec, .. }
        | p::PartRepresentation::Style { codec, .. } => format!(
            "Codecs.Encode{}({payload})",
            plan.models.codec_name(codec.schema().id())
        ),
    };
    let metadata = if part.wrapper_type.is_some() {
        format!(
            ", {value}.ContentType, {value}.FileName, {}",
            part.header_type
                .as_ref()
                .map(|h| format!(
                    "Write{h}({descriptor}.GetProperty(\"headers\"), {value}.Headers)"
                ))
                .unwrap_or("null".into())
        )
    } else {
        String::new()
    };
    format!("PartsRuntime.Create({descriptor}, {name}, {encoded}{metadata})")
}
pub(super) fn part_decode(
    plan: &SdkPlan,
    part: &PlannedPart,
    descriptor: &str,
    raw: &str,
    multipart: bool,
) -> String {
    let payload = match part.wire.representation() {
        p::PartRepresentation::Binary { .. } => format!("{raw}.Data"),
        p::PartRepresentation::Json { codec, .. }
        | p::PartRepresentation::Text { codec, .. }
        | p::PartRepresentation::Style { codec, .. } => format!(
            "Codecs.Decode{}(PartsRuntime.Value({descriptor}, {raw}, {multipart}))",
            plan.models.codec_name(codec.schema().id())
        ),
    };
    if let Some(wrapper) = &part.wrapper_type {
        format!(
            "new {wrapper} {{ Value = {payload}, FileName = {raw}.FileName, ContentType = {raw}.ContentType{} }}",
            part.header_type
                .as_ref()
                .map(|h| format!(
                    ", Headers = Read{h}({descriptor}.GetProperty(\"headers\"), {raw}.Headers)"
                ))
                .unwrap_or_default()
        )
    } else {
        payload
    }
}
fn parts_codecs(plan: &SdkPlan, out: &mut String, parts: &PlannedParts) {
    for part in parts
        .fields
        .iter()
        .chain(parts.additional.iter().map(|p| p.as_ref()))
    {
        if let Some(h) = &part.header_type {
            headers_codecs(plan, out, h, &part.headers);
        }
    }
    let key = if parts.multipart { "multipart" } else { "form" };
    out.push_str(&format!("    internal static EncodedBody Write{}(JsonElement media, int index, {} value, string? contentType)\n    {{\n        if (value is null) throw new SdkException(SdkErrorKind.RequestValidation);\n        var plan = media.GetProperty(\"representation\").GetProperty(\"{key}\");\n        var bag = new PartBag();\n",parts.native_type,parts.native_type));
    for (i, p) in parts.fields.iter().enumerate() {
        let v = format!(
            "value.{}{}",
            p.property_name,
            if p.wire.required() { "" } else { ".Value" }
        );
        let name = quote(p.wire.name().unwrap());
        let d = format!("part{i}");
        if !p.wire.required() {
            out.push_str(&format!(
                "        if (value.{}.HasValue)\n        {{\n",
                p.property_name
            ));
        }
        out.push_str(&format!("        var {d} = PartsRuntime.Part(plan, {name});\n        var values{i} = new List<RawPart>();\n"));
        if p.wire.multiplicity() == p::PartMultiplicity::RepeatedArrayItems {
            out.push_str(&format!("        if ({v} is null) throw new SdkException(SdkErrorKind.RequestValidation);\n        foreach (var item in {v}) values{i}.Add({});\n",part_encode(plan,p,"item",&d,&name)));
        } else {
            out.push_str(&format!(
                "        values{i}.Add({});\n",
                part_encode(plan, p, &v, &d, &name)
            ));
        }
        out.push_str(&format!("        bag.Add({name}, values{i});\n"));
        if !p.wire.required() {
            out.push_str("        }\n");
        }
    }
    if let Some(p) = &parts.additional {
        out.push_str("        if (value.Extra is null) throw new SdkException(SdkErrorKind.RequestValidation);\n        foreach (var entry in value.Extra)\n        {\n            var part = PartsRuntime.Part(plan, entry.Key);\n            var values = new List<RawPart>();\n");
        if p.wire.multiplicity() == p::PartMultiplicity::RepeatedArrayItems {
            out.push_str(&format!(
                "            foreach (var item in entry.Value) values.Add({});\n",
                part_encode(plan, p, "item", "part", "entry.Key")
            ));
        } else {
            out.push_str(&format!(
                "            values.Add({});\n",
                part_encode(plan, p, "entry.Value", "part", "entry.Key")
            ));
        }
        out.push_str("            bag.Add(entry.Key, values);\n        }\n");
    }
    out.push_str("        return PartsRuntime.Encode(media, index, bag, contentType);\n    }\n");
    out.push_str(&format!("    internal static {} Read{}(JsonElement media, PartBag bag)\n    {{\n        var plan = media.GetProperty(\"representation\").GetProperty(\"{key}\");\n        var value = new {}\n        {{\n",parts.native_type,parts.native_type,parts.native_type));
    for p in &parts.fields {
        let name = quote(p.wire.name().unwrap());
        let d = format!("PartsRuntime.Part(plan, {name})");
        let decoded = if p.wire.multiplicity() == p::PartMultiplicity::RepeatedArrayItems {
            format!(
                "global::System.Linq.Enumerable.ToList(global::System.Linq.Enumerable.Select(bag.Many({name}), item => {}))",
                part_decode(plan, p, &d, "item", parts.multipart)
            )
        } else {
            part_decode(plan, p, &d, &format!("bag.One({name})"), parts.multipart)
        };
        out.push_str(&format!(
            "            {} = {},\n",
            p.property_name,
            if p.wire.required() {
                decoded
            } else {
                format!(
                    "bag.Has({name}) ? Optional<{}>.Present({decoded}) : default",
                    p.native_type
                )
            }
        ));
    }
    out.push_str("        };\n");
    if let Some(p) = &parts.additional {
        out.push_str("        foreach (var entry in bag.Fields)\n        {\n");
        if !parts.fields.is_empty() {
            out.push_str(&format!(
                "            if (entry.Key is {}) continue;\n",
                parts
                    .fields
                    .iter()
                    .map(|f| quote(f.wire.name().unwrap()))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ));
        }
        let d = "PartsRuntime.Part(plan, entry.Key)";
        let decoded = if p.wire.multiplicity() == p::PartMultiplicity::RepeatedArrayItems {
            format!(
                "global::System.Linq.Enumerable.ToList(global::System.Linq.Enumerable.Select(entry.Value, item => {}))",
                part_decode(plan, p, d, "item", parts.multipart)
            )
        } else {
            part_decode(plan, p, d, "entry.Value.Single()", parts.multipart)
        };
        out.push_str(&format!(
            "            value.Extra.Add(entry.Key, {decoded});\n        }}\n"
        ));
    }
    out.push_str("        return value;\n    }\n");
}
