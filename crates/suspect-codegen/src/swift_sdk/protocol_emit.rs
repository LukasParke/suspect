//! Native operations, part types and response unions from the immutable plan.
use super::{
    PlannedBody, PlannedHeader, PlannedMedia, PlannedOperation, PlannedPart, PlannedParts,
    PlannedResponse, SdkPlan,
    protocol_metadata::{self as meta, q, source},
};
use crate::http_protocol::*;
use std::fmt::Write as _;

fn doc(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "{indent}/// {}", super::emit::prose(line));
    }
}
fn optional(ty: &str, required: bool) -> String {
    if required {
        ty.into()
    } else {
        format!("OptionalField<{ty}>")
    }
}
fn fields(out: &mut String, name: &str, fields: &[(String, String, bool)]) {
    let _ = writeln!(out, "public struct {name}: Sendable {{");
    for (name, ty, _) in fields {
        let _ = writeln!(out, "    public var {name}: {ty}");
    }
    let _ = writeln!(
        out,
        "    public init({}) {{",
        fields
            .iter()
            .map(|(name, ty, required)| format!(
                "{name}: {ty}{}",
                if *required { "" } else { " = .missing" }
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (name, _, _) in fields {
        let _ = writeln!(out, "        self.{name} = {name}");
    }
    out.push_str("    }\n");
}

pub(super) fn client(plan: &SdkPlan) -> String {
    let mut out = String::from(
        "import Foundation\n\n/// Runtime credentials named by their source schemes. OAuth/OIDC values are explicit caller hooks.\npublic struct Credentials: Sendable {\n",
    );
    for c in &plan.credential_bindings {
        doc(
            &mut out,
            &format!(
                "{} credential. Source: {}#{}.",
                c.requirement.name(),
                c.requirement.scheme().use_site().source().document(),
                c.requirement.scheme().use_site().source().pointer()
            ),
            "    ",
        );
        let _ = writeln!(out, "    public var {}: {}?", c.property, c.ty());
    }
    let _ = writeln!(
        out,
        "    public init({}) {{",
        plan.credential_bindings
            .iter()
            .map(|c| format!("{}: {}? = nil", c.property, c.ty()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for c in &plan.credential_bindings {
        let _ = writeln!(out, "        self.{0} = {0}", c.property);
    }
    out.push_str("    }\n    var values: [String: HTTPAuthValue] {\n");
    if plan.credential_bindings.is_empty() {
        out.push_str("        return [:]\n");
    } else {
        out.push_str("        var result: [String: HTTPAuthValue] = [:]\n");
        for c in &plan.credential_bindings {
            let kind = match c.requirement.credential() {
                CredentialHook::Basic => "basic",
                CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. } => "provider",
                _ => "text",
            };
            let _ = writeln!(
                out,
                "        if let value = {} {{ result[{}] = .{kind}(value) }}",
                c.property,
                q(&format!(
                    "{}#{}",
                    c.requirement.scheme().use_site().source().document(),
                    c.requirement.scheme().use_site().source().pointer()
                ))
            );
        }
        out.push_str("        return result\n");
    }
    out.push_str("    }\n}\n\n/// Immutable Sendable client. Each call performs one source-selected request.\npublic struct Client: Sendable {\n    let credentials: Credentials\n    let transport: any HTTPTransport\n    let options: ClientOptions\n");
    if plan.credential_env.is_some() {
        out.push_str("    /// Explicit credentials are authoritative as a whole; missing or empty members are not filled from the environment.\n    public init(credentials: Credentials, transport: any HTTPTransport = URLSessionTransport(), options: ClientOptions = .init()) {\n        self.credentials = credentials; self.transport = transport; self.options = options\n    }\n    /// Snapshots the configured process environment once, at client creation.\n    public init(transport: any HTTPTransport = URLSessionTransport(), options: ClientOptions = .init()) {\n        self = Self.fromEnvironment(transport: transport, options: options)\n    }\n");
        environment_factory(&mut out, plan);
    } else {
        out.push_str("    public init(credentials: Credentials = .init(), transport: any HTTPTransport = URLSessionTransport(), options: ClientOptions = .init()) {\n        self.credentials = credentials; self.transport = transport; self.options = options\n    }\n");
    }
    let _ = writeln!(
        out,
        "    static let maxRequestBytes = {}\n    static let maxResponseBytes = {}\n    static let maxCaptureBytes = {}\n    static let maxPartBytes = {}\n    static let maxStreamItemBytes = {}\n    static let maxStreamBufferBytes = {}",
        plan.config.max_request_bytes,
        plan.config.max_response_bytes,
        plan.config.max_stream_capture_bytes,
        plan.config.max_part_bytes,
        plan.config.max_stream_item_bytes,
        plan.config.max_stream_buffer_bytes
    );
    out.push_str(r#"    func send(_ request: HTTPRequest, source: SourceLocation) async throws -> HTTPResponse {
        do {
            try Task.checkCancellation()
            let response = try await transport.send(request)
            try Task.checkCancellation()
            guard response.body.count <= request.maxResponseBytes else { throw TransportError.responseTooLarge(limit: request.maxResponseBytes) }
            try HTTPBuild.checkHeaders(response.headers)
            guard (100...599).contains(response.status) else { throw SDKError(.unexpectedResponse, source: source, response: response, captureLimit: Self.maxCaptureBytes) }
            return response
        } catch { throw transportFailure(error, source: source) }
    }
    func open(_ request: HTTPRequest, source: SourceLocation) async throws -> HTTPStreamResponse {
        do {
            try Task.checkCancellation()
            let response = try await transport.open(request, maxBufferedBytes: Self.maxStreamBufferBytes)
            do {
                try Task.checkCancellation()
                try HTTPBuild.checkHeaders(response.headers)
                guard (100...599).contains(response.status) else { throw SDKError(.unexpectedResponse, source: source) }
                return response
            } catch { response.close(); throw error }
        } catch { throw transportFailure(error, source: source) }
    }
    func transportFailure(_ error: any Error, source: SourceLocation) -> any Error {
        if Task.isCancelled || error is CancellationError { return CancellationError() }
        if let error = error as? SDKError { return error }
        if case TransportError.responseTooLarge = error { return SDKError(.responseTooLarge, source: source) }
        if case TransportError.streamBufferExceeded = error { return SDKError(.responseTooLarge, source: source) }
        if case TransportError.timedOut = error { return SDKError(.timeout, source: source) }
        return SDKError(.transport, source: source)
    }
    func responseValue<T>(_ response: HTTPResponse, data: T, declaredContentType: String?, links: [HTTPLink], noBody: Bool) -> APIResponse<T> {
        let bytes = noBody ? Data() : response.body
        let capture = (200..<300).contains(response.status) ? bytes.count : Self.maxCaptureBytes
        return APIResponse(status: response.status, headers: response.headers, data: data,
            rawBody: Data(bytes.prefix(capture)), rawBodyTruncated: bytes.count > capture,
            contentType: response.headers.first(where: { $0.name.lowercased() == "content-type" })?.value,
            declaredContentType: declaredContentType, links: links)
    }
}
"#);
    out
}

fn environment_factory(out: &mut String, plan: &SdkPlan) {
    let policy = plan
        .credential_env
        .as_ref()
        .expect("configured environment factory");
    let limit = super::CREDENTIAL_ENV_MAX_BYTES;
    out.push_str("    /// Creates a client from explicitly configured environment variable names.\n    /// Missing, empty or unusable values stay unavailable; protected calls report a secret-free SDKError before transport.\n    /// Later environment changes affect newly created clients only.\n");
    let _ = writeln!(
        out,
        "    /// Each credential is bounded to {limit} UTF-8 bytes, matching the native attachment policy."
    );
    out.push_str("    public static func fromEnvironment(transport: any HTTPTransport = URLSessionTransport(), options: ClientOptions = .init()) -> Client {\n        let environment = Foundation.ProcessInfo.processInfo.environment\n        func read(_ variable: String, bearer: Bool = false, header: String? = nil) -> String? {\n");
    let _ = writeln!(
        out,
        "            guard let value = environment[variable], !value.isEmpty, value.utf8.prefix({}).count <= {limit} else {{ return nil }}",
        limit + 1
    );
    out.push_str("            if bearer && !value.utf8.allSatisfy({ (33...126).contains($0) }) { return nil }\n            if let header {\n                do { try HTTPBuild.checkHeaders([HTTPHeader(header, value)]) }\n                catch { return nil }\n            }\n            return value\n        }\n");
    let arguments = plan
        .credential_bindings
        .iter()
        .filter_map(|credential| {
            let binding = policy.bindings().iter().find(|binding| {
                binding.scheme().use_site().source()
                    == credential.requirement.scheme().use_site().source()
            })?;
            let check = match (binding.kind(), credential.requirement.credential()) {
                (crate::credential_env::CredentialEnvKind::Bearer, _) => ", bearer: true".into(),
                (
                    _,
                    CredentialHook::ApiKey {
                        location: ParameterLocation::Header,
                        name,
                    },
                ) => format!(", header: {}", q(name.value())),
                _ => String::new(),
            };
            Some(format!(
                "{}: read({}{check})",
                credential.property,
                q(binding.variable())
            ))
        })
        .collect::<Vec<_>>();
    let _ = writeln!(
        out,
        "        return Client(credentials: Credentials({}), transport: transport, options: options)\n    }}",
        arguments.join(", ")
    );
}

pub(super) fn operations(plan: &SdkPlan) -> String {
    let mut out = String::from("import Foundation\n\n");
    for op in &plan.operations {
        for parameter in &op.parameters {
            if let Some(form) = &parameter.query_form {
                super::protocol_query::emit(&mut out, form, plan);
            }
        }
        if let Some(body) = &op.body {
            body_types(&mut out, body, plan);
        }
        for response in &op.responses {
            response_types(&mut out, response, plan);
        }
        doc(
            &mut out,
            &format!(
                "Input for {} {}. Source: {}#{}.",
                op.method,
                op.path,
                op.source.document(),
                op.source.pointer()
            ),
            "",
        );
        let mut input = op
            .parameters
            .iter()
            .map(|p| {
                (
                    p.field_name.clone(),
                    optional(
                        &plan.models.ty(p.wire.codec().schema().id()),
                        p.wire.required(),
                    ),
                    p.wire.required(),
                )
            })
            .collect::<Vec<_>>();
        if let Some(body) = &op.body {
            input.push((
                "body".into(),
                optional(&body.type_name, body.wire.required()),
                body.wire.required(),
            ));
        }
        fields(&mut out, &op.input_type, &input);
        out.push_str("}\n\n");
        for (error, name) in [(false, &op.success_type), (true, &op.error_type)] {
            doc(
                &mut out,
                if error {
                    "Declared API failures. Transport/codec failures throw SDKError; cancellation remains CancellationError."
                } else {
                    "Declared successful responses, classified by actual HTTP status. A sole success also exposes its payload directly through data."
                },
                "",
            );
            let _ = writeln!(
                out,
                "public enum {name}: {}Sendable {{",
                if error { "Error, " } else { "" }
            );
            let responses = op
                .responses
                .iter()
                .filter(|r| if error { r.may_fail() } else { r.may_succeed() })
                .collect::<Vec<_>>();
            for r in &responses {
                let _ = writeln!(out, "    case {}({})", r.case_name, r.response_type);
            }
            if !error && responses.len() == 1 {
                let r = responses[0];
                let _ = writeln!(
                    out,
                    "    public var response: {} {{ switch self {{ case .{}(let value): return value }} }}",
                    r.response_type, r.case_name
                );
                for (name, ty) in [
                    ("data", r.type_name.as_str()),
                    ("status", "Int"),
                    ("headers", "[HTTPHeader]"),
                    ("rawBody", "Data"),
                    ("rawBodyTruncated", "Bool"),
                    ("contentType", "String?"),
                    ("declaredContentType", "String?"),
                    ("links", "[HTTPLink]"),
                ] {
                    let _ = writeln!(out, "    public var {name}: {ty} {{ response.{name} }}");
                }
                if let Some(ty) = &r.header_type {
                    let _ = writeln!(
                        out,
                        "    public var typedHeaders: {ty} {{ response.typedHeaders }}"
                    );
                }
            }
            out.push_str("}\n\n");
        }
        doc(
            &mut out,
            "Source-backed protocol metadata. Server/authorization choices do not perform network I/O.",
            "",
        );
        let _ = writeln!(
            out,
            "public enum {} {{\n    public static let metadata = {}\n    static let responses: [HTTPResponseRule] = [{}]\n}}\n",
            op.metadata_name,
            meta::metadata(&op.wire),
            op.responses
                .iter()
                .map(|r| meta::response(&r.wire))
                .collect::<Vec<_>>()
                .join(", ")
        );
        operation(&mut out, op, plan);
    }
    out
}

fn body_types(out: &mut String, body: &PlannedBody, plan: &SdkPlan) {
    for media in &body.media {
        if let Some(parts) = &media.parts {
            parts_type(out, parts, plan);
        }
        if let Some(parts) = &media.positional {
            super::protocol_positional::emit(out, parts, plan);
        }
    }
    if body.is_enum {
        let _ = writeln!(
            out,
            "/// A caller-selected request representation, validated against Content-Type precedence.\npublic enum {}: Sendable {{",
            body.type_name
        );
        for m in &body.media {
            let actual = if matches!(m.wire.media_type().range(), MediaRange::Concrete { .. }) {
                ""
            } else {
                ", contentType: String"
            };
            let _ = writeln!(out, "    case {}({}{actual})", m.case_name, m.type_name);
        }
        out.push_str("}\n\n");
    }
}
fn response_types(out: &mut String, r: &PlannedResponse, plan: &SdkPlan) {
    for media in &r.media {
        if let Some(parts) = &media.parts {
            parts_type(out, parts, plan);
        }
        if let Some(parts) = &media.positional {
            super::protocol_positional::emit(out, parts, plan);
        }
    }
    if r.is_enum {
        let _ = writeln!(
            out,
            "/// Declared media alternatives, including explicit HTTP body suppression.\npublic enum {}: Sendable {{",
            r.type_name
        );
        if r.may_be_empty {
            out.push_str("    case none\n");
        }
        if r.media.is_empty() {
            out.push_str("    case bytes(Data)\n");
        }
        for m in &r.media {
            let _ = writeln!(out, "    case {}({})", m.case_name, m.type_name);
        }
        out.push_str("}\n\n");
    }
    if let Some(header_type) = &r.header_type {
        headers_type(out, header_type, &r.headers, plan);
        let _ = writeln!(
            out,
            "/// A response with source-typed headers and unchanged raw fields.\npublic struct {}: Sendable {{\n    let base: APIResponse<{}>\n    public let typedHeaders: {header_type}",
            r.response_type, r.type_name
        );
        for (name, ty) in [
            ("data", r.type_name.as_str()),
            ("status", "Int"),
            ("headers", "[HTTPHeader]"),
            ("rawBody", "Data"),
            ("rawBodyTruncated", "Bool"),
            ("contentType", "String?"),
            ("declaredContentType", "String?"),
            ("links", "[HTTPLink]"),
        ] {
            let _ = writeln!(out, "    public var {name}: {ty} {{ base.{name} }}");
        }
        out.push_str("}\n\n");
    }
}
fn headers_type(out: &mut String, name: &str, headers: &[PlannedHeader], plan: &SdkPlan) {
    doc(
        out,
        "Source-typed header values. Optional fields remain missing; repeated ambiguous fields are rejected.",
        "",
    );
    let fs = headers
        .iter()
        .map(|h| {
            (
                h.field_name.clone(),
                optional(&h.type_name, h.wire.required()),
                h.wire.required(),
            )
        })
        .collect::<Vec<_>>();
    fields(out, name, &fs);
    out.push_str("    static func decode(_ headers: [HTTPHeader], limit: Int) throws -> Self {\n");
    for (i, h) in headers.iter().enumerate() {
        let codec = &plan.models.codecs[h.wire.codec().schema().id()];
        let _ = writeln!(
            out,
            "        let field{i}: {}\n        if let value = try HTTPWire.header(headers, parameter: {}) {{",
            optional(&h.type_name, h.wire.required()),
            meta::header(&h.wire, plan)
        );
        let decode =
            format!("try Codecs.{codec}.decodeValue(value, limits: JsonLimits(maxBytes: limit))");
        let _ = writeln!(
            out,
            "            field{i} = {}",
            if h.wire.required() {
                decode
            } else {
                format!(".value({decode})")
            }
        );
        if h.wire.required() {
            out.push_str("        } else { throw JsonError(.representation, \"required header is absent\") }\n");
        } else {
            let _ = writeln!(out, "        }} else {{ field{i} = .missing }}");
        }
    }
    let _ = writeln!(
        out,
        "        return Self({})\n    }}",
        headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{}: field{i}", h.field_name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push_str("    func encode(limit: Int) throws -> [HTTPHeader] {\n        var headers: [HTTPHeader] = []\n");
    for h in headers {
        let value = if h.wire.required() {
            format!("self.{}", h.field_name)
        } else {
            "value".into()
        };
        if !h.wire.required() {
            let _ = writeln!(
                out,
                "        if case .value(let value) = self.{} {{",
                h.field_name
            );
        }
        let _ = writeln!(
            out,
            "        try HTTPBuild.attach(HTTPHeader({}, HTTPWire.serialize(Codecs.{}.encodeValue({value}, limits: JsonLimits(maxBytes: limit)), parameter: {}, limit: limit)), to: &headers)",
            q(h.wire.name()),
            plan.models.codecs[h.wire.codec().schema().id()],
            meta::header(&h.wire, plan)
        );
        if !h.wire.required() {
            out.push_str("        }\n");
        }
    }
    out.push_str("        return headers\n    }\n}\n\n");
}

fn operation(out: &mut String, op: &PlannedOperation, plan: &SdkPlan) {
    out.push_str("extension Client {\n");
    doc(out, &op.description, "    ");
    doc(
        out,
        &format!(
            "{} {}. Input validation precedes transport. Source: {}#{}.",
            op.method,
            op.path,
            op.source.document(),
            op.source.pointer()
        ),
        "    ",
    );
    let _ = writeln!(
        out,
        "    public func {}(_ input: {}{}, options requestOptions: RequestOptions = .init()) async throws -> {} {{",
        op.method_name,
        op.input_type,
        if op.default_input() { " = .init()" } else { "" },
        op.success_type
    );
    request_prelude(out, op, plan);
    response_decode(out, op, plan);
    out.push_str("    }\n}\n\n");
}

/// The shared request construction every emitted operation executes before its
/// response decoding: server selection, parameter/body/auth wire assembly and
/// the typed failure branding. Emitted verbatim so generated helpers (the
/// typed events exchange) reuse the exact direct-call construction.
pub(super) fn request_prelude(out: &mut String, op: &PlannedOperation, plan: &SdkPlan) {
    let _ = writeln!(
        out,
        "        try Task.checkCancellation()\n        let request: HTTPRequest\n        do {{\n            let server = try HTTPBuild.server({}.metadata, options: options, request: requestOptions, maxBytes: Self.maxRequestBytes)",
        op.metadata_name
    );
    let path_mut = op
        .parameters
        .iter()
        .any(|p| p.wire.location() == ParameterLocation::Path);
    let _ = writeln!(
        out,
        "            {} path = {}\n            var query: [String] = []\n            var cookies: [String] = []\n            var headers: [HTTPHeader] = []",
        if path_mut { "var" } else { "let" },
        q(&op.path)
    );
    let accept = op
        .responses
        .iter()
        .flat_map(|r| r.wire.media())
        .map(|m| m.media_type().declared())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    if !accept.is_empty() {
        let _ = writeln!(
            out,
            "            headers.append(HTTPHeader(\"Accept\", {}))",
            q(&accept)
        );
    }
    for (i, p) in op
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| p.wire.location() == ParameterLocation::Path)
    {
        parameter_encode(out, p, i, plan, "Self.maxRequestBytes");
    }
    out.push_str("            try HTTPBuild.safePath(path)\n");
    let query = op.parameters.iter().any(|p| {
        matches!(
            p.wire.location(),
            ParameterLocation::Query | ParameterLocation::Querystring
        )
    });
    if query {
        out.push_str("            let queryLimit = try HTTPBuild.queryBudget(server: server, path: path, limit: Self.maxRequestBytes)\n            var queryBytes = 0\n");
    }
    for (i, p) in op
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| p.wire.location() != ParameterLocation::Path)
    {
        parameter_encode(
            out,
            p,
            i,
            plan,
            if matches!(
                p.wire.location(),
                ParameterLocation::Query | ParameterLocation::Querystring
            ) {
                "queryLimit - queryBytes"
            } else {
                "min(Self.maxRequestBytes, 65_536)"
            },
        );
    }
    body_encode(out, op, plan);
    let _ = writeln!(
        out,
        "            try await HTTPBuild.authentication({}.metadata.security, selected: requestOptions.securityAlternative ?? options.securityAlternative, values: credentials.values, serverURL: server, headers: &headers, query: &query, cookies: &cookies, limit: Self.maxRequestBytes)\n            request = try HTTPBuild.makeProtocolRequest(method: {}, path: path, query: query, headers: headers, cookies: cookies, body: body, server: server, options: options, request: requestOptions, maxRequest: Self.maxRequestBytes, maxResponse: Self.maxResponseBytes)\n        }} catch is CancellationError {{ throw CancellationError() }}\n        catch let error as ValidationError {{ throw SDKError(.requestValidation, source: error.source, validation: error) }}\n        catch let error as JsonError {{ throw SDKError(.requestRepresentation, source: {}, json: error) }}\n        catch {{ if Task.isCancelled {{ throw CancellationError() }}; throw SDKError(.configuration, source: {}) }}",
        op.metadata_name,
        q(&op.method),
        source(&op.source),
        source(&op.source)
    );
}

fn parameter_encode(
    out: &mut String,
    p: &super::PlannedParameter,
    index: usize,
    plan: &SdkPlan,
    limit: &str,
) {
    let w = &p.wire;
    let indent = if w.required() {
        "            "
    } else {
        "                "
    };
    if !w.required() {
        let _ = writeln!(
            out,
            "            if case .value(let present) = input.{} {{",
            p.field_name
        );
    }
    let value = if w.required() {
        format!("input.{}", p.field_name)
    } else {
        "present".into()
    };
    let _ = writeln!(
        out,
        "{indent}let parameter{index} = try Codecs.{}.encodeValue({value}, limits: JsonLimits(maxBytes: Self.maxRequestBytes))",
        plan.models.codecs[w.codec().schema().id()]
    );
    let fast = matches!(
        w.serialization(),
        ParameterSerialization::Style {
            style: Style::Form,
            shape: WireShape::Scalar { .. } | WireShape::Array { .. },
            percent_encoding: PercentEncoding::UriComponent,
            ..
        }
    ) && w.location() == ParameterLocation::Query;
    if fast {
        if let ParameterSerialization::Style { explode, shape, .. } = w.serialization() {
            let _ = writeln!(
                out,
                "{indent}try HTTPBuild.appendQuery(parameter{index}, name: {}, array: {}, explode: {explode}, required: {}, to: &query, bytes: &queryBytes, limit: queryLimit)",
                q(w.name()),
                matches!(shape, WireShape::Array { .. }),
                w.required()
            );
        }
    } else {
        let omit = !w.required()
            && matches!(
                w.serialization(),
                ParameterSerialization::Style {
                    shape: WireShape::Array { .. } | WireShape::FlatObject { .. },
                    ..
                }
            );
        if omit {
            let _ = writeln!(out, "{indent}if !HTTPWire.empty(parameter{index}) {{");
        }
        if let Some(form) = &p.query_form {
            let _ = writeln!(
                out,
                "{indent}let wire{index} = try {}.encode(parameter{index}, limit: {limit}, partLimit: Self.maxPartBytes)",
                form.encoder_name
            );
        } else {
            let _ = writeln!(
                out,
                "{indent}let wire{index} = try HTTPWire.serialize(parameter{index}, parameter: {}, limit: {limit})",
                meta::parameter(w, plan)
            );
        }
        match w.location() {
            ParameterLocation::Path => {
                let _ = writeln!(
                    out,
                    "{indent}path = path.replacingOccurrences(of: {}, with: wire{index})\n{indent}guard path.utf8.count <= Self.maxRequestBytes else {{ throw JsonError(.resourceLimit, \"assembled path byte limit exceeded\") }}",
                    q(&format!("{{{}}}", w.name()))
                );
            }
            ParameterLocation::Query | ParameterLocation::Querystring => {
                let _ = writeln!(
                    out,
                    "{indent}let size{index} = wire{index}.utf8.count + (query.isEmpty ? 0 : 1)\n{indent}guard size{index} <= queryLimit - queryBytes else {{ throw JsonError(.resourceLimit, \"assembled query byte limit exceeded\") }}\n{indent}query.append(wire{index}); queryBytes += size{index}"
                );
            }
            ParameterLocation::Header => {
                let _ = writeln!(
                    out,
                    "{indent}try HTTPBuild.attach(HTTPHeader({}, wire{index}), to: &headers)",
                    q(w.name())
                );
            }
            ParameterLocation::Cookie => {
                let _ = writeln!(
                    out,
                    "{indent}try HTTPBuild.appendCookie(wire{index}, to: &cookies)"
                );
            }
        }
        if omit {
            let _ = writeln!(out, "{indent}}}");
        }
    }
    if !w.required() {
        out.push_str("            }\n");
    }
}
fn body_encode(out: &mut String, op: &PlannedOperation, plan: &SdkPlan) {
    let Some(body) = &op.body else {
        out.push_str("            let body: HTTPBody? = nil\n");
        return;
    };
    out.push_str("            let body: HTTPBody?\n");
    if !body.wire.required() {
        out.push_str("            if case .value(let bodyValue) = input.body {\n");
    } else {
        out.push_str("            let bodyValue = input.body\n");
    }
    if body.is_enum {
        out.push_str("            switch bodyValue {\n");
        for (i, m) in body.media.iter().enumerate() {
            let wildcard = !matches!(m.wire.media_type().range(), MediaRange::Concrete { .. });
            let _ = writeln!(
                out,
                "            case .{}(let value{}):",
                m.case_name,
                if wildcard {
                    ", let selectedContentType"
                } else {
                    ""
                }
            );
            encode_media(out, m, i, body, plan, "value", wildcard);
        }
        out.push_str("            }\n");
    } else {
        encode_media(out, &body.media[0], 0, body, plan, "bodyValue", false);
    }
    if !body.wire.required() {
        out.push_str("            } else { body = nil }\n");
    }
}
fn encode_media(
    out: &mut String,
    m: &PlannedMedia,
    index: usize,
    body: &PlannedBody,
    plan: &SdkPlan,
    value: &str,
    wildcard: bool,
) {
    let selected = if wildcard {
        "selectedContentType".into()
    } else {
        q(m.wire.media_type().declared())
    };
    let _ = writeln!(
        out,
        "                let contentType = requestOptions.contentType ?? {selected}\n                guard try HTTPMediaType.select(contentType, from: {}) == {index} else {{ throw JsonError(.representation, \"Content-Type selects a different typed request representation\") }}",
        meta::media_list(body.wire.media())
    );
    match m.wire.representation() {
        Representation::Json { codec } => {
            let expr=codec.as_ref().map(|c|format!("Codecs.{}.encode({value}, limits: JsonLimits(maxBytes: Self.maxRequestBytes))",plan.models.codecs[c.schema().id()])).unwrap_or_else(||format!("{value}.encoded(limits: JsonLimits(maxBytes: Self.maxRequestBytes))"));
            let _ = writeln!(
                out,
                "                body = HTTPBody(bytes: try {expr}, contentType: contentType)"
            );
        }
        Representation::Text { codec, scalar, .. } => {
            let json=codec.as_ref().map(|c|format!("try Codecs.{}.encodeValue({value}, limits: JsonLimits(maxBytes: Self.maxRequestBytes))",plan.models.codecs[c.schema().id()])).unwrap_or_else(||format!("JsonValue.string({value})"));
            let _ = writeln!(
                out,
                "                try HTTPMediaType.utf8(contentType)\n                let text = try HTTPWire.scalar({json}, type: {})\n                body = HTTPBody(bytes: try HTTPParts.textBytes(text, limit: Self.maxRequestBytes), contentType: contentType)",
                meta::scalar(*scalar)
            );
        }
        Representation::Binary { bytes, .. } => {
            let _ = writeln!(
                out,
                "                body = HTTPBody(bytes: try HTTPParts.bytes({value}, limit: min(Self.maxRequestBytes, {})), contentType: contentType)",
                bytes.max_bytes()
            );
        }
        Representation::Form { .. } | Representation::Multipart { .. } => {
            let _ = writeln!(
                out,
                "                body = try {value}.encode(contentType: contentType, boundary: requestOptions.multipartBoundary, limit: Self.maxRequestBytes, partLimit: Self.maxPartBytes)"
            );
        }
        Representation::Stream { .. } => unreachable!("request streams declined"),
    }
}

fn response_decode(out: &mut String, op: &PlannedOperation, plan: &SdkPlan) {
    let streaming = op
        .responses
        .iter()
        .flat_map(|r| &r.media)
        .any(|m| matches!(m.wire.representation(), Representation::Stream { .. }));
    let mutable = streaming
        && op.responses.iter().any(|r| {
            !r.always_empty
                && (r.media.is_empty()
                    || r.media
                        .iter()
                        .any(|m| !matches!(m.wire.representation(), Representation::Stream { .. })))
        });
    if streaming {
        let _ = writeln!(
            out,
            "        let stream = try await open(request, source: {})\n        {} response = HTTPResponse(status: stream.status, headers: stream.headers, body: Data())",
            source(&op.source),
            if mutable { "var" } else { "let" }
        );
    } else {
        let _ = writeln!(
            out,
            "        let response = try await send(request, source: {})",
            source(&op.source)
        );
    }
    let _ = writeln!(
        out,
        "        do {{\n            let responseIndex = try HTTPResponseRule.select(response, from: {}.responses, source: {}, capture: Self.maxCaptureBytes)\n            let noBody = HTTPBuild.forbidden(method: {}, status: response.status)\n            switch responseIndex {{",
        op.metadata_name,
        source(&op.source),
        q(&op.method)
    );
    for (i, r) in op.responses.iter().enumerate() {
        let _ = writeln!(
            out,
            "            case {i}:\n                let data: {}\n                let declaredContentType: String?",
            r.type_name
        );
        if r.always_empty {
            if streaming {
                out.push_str("                stream.close()\n");
            }
            out.push_str("                data = HTTPNoContent(); declaredContentType = nil\n");
        } else {
            if r.may_be_empty {
                out.push_str("                if noBody {\n");
                if streaming {
                    out.push_str("                    stream.close()\n");
                }
                out.push_str("                    data = .none; declaredContentType = nil\n                } else {\n");
            }
            if r.media.is_empty() {
                if streaming {
                    out.push_str("                response = try await stream.collect(limit: request.maxResponseBytes)\n");
                }
                let _ = writeln!(
                    out,
                    "                data = {}; declaredContentType = nil",
                    if r.is_enum {
                        ".bytes(response.body)"
                    } else {
                        "response.body"
                    }
                );
            } else {
                let _ = writeln!(
                    out,
                    "                let contentType = try HTTPMediaType.contentType(response.headers)\n                let mediaIndex = try HTTPMediaType.select(contentType, from: {}.responses[{i}].media)\n                switch mediaIndex {{",
                    op.metadata_name
                );
                for (j, m) in r.media.iter().enumerate() {
                    let _ = writeln!(out, "                case {j}:");
                    let is_stream =
                        matches!(m.wire.representation(), Representation::Stream { .. });
                    if streaming && !is_stream {
                        out.push_str("                    response = try await stream.collect(limit: request.maxResponseBytes)\n");
                    }
                    let expr = decode_media(m, plan, streaming);
                    let _ = writeln!(
                        out,
                        "                    data = {}\n                    declaredContentType = {}",
                        if r.is_enum {
                            format!(".{}({expr})", m.case_name)
                        } else {
                            expr
                        },
                        q(m.wire.media_type().declared())
                    );
                }
                out.push_str("                default: throw JsonError(.representation, \"unmatched response representation\")\n                }\n");
            }
            if r.may_be_empty {
                out.push_str("                }\n");
            }
        }
        let base = format!(
            "responseValue(response, data: data, declaredContentType: declaredContentType, links: {}, noBody: noBody)",
            meta::links(r.wire.links())
        );
        let expr = if let Some(h) = &r.header_type {
            format!(
                "try {}(base: {base}, typedHeaders: {h}.decode(response.headers, limit: Self.maxResponseBytes))",
                r.response_type
            )
        } else {
            base
        };
        let _ = writeln!(out, "                let value = {expr}");
        if r.may_succeed() && r.may_fail() {
            let _ = writeln!(
                out,
                "                if (200..<300).contains(response.status) {{ return .{}(value) }}\n                throw {}.{}(value)",
                r.case_name, op.error_type, r.case_name
            );
        } else if r.may_succeed() {
            let _ = writeln!(out, "                return .{}(value)", r.case_name);
        } else {
            let _ = writeln!(
                out,
                "                throw {}.{}(value)",
                op.error_type, r.case_name
            );
        }
    }
    let close = if streaming { "stream.close(); " } else { "" };
    let _ = writeln!(
        out,
        "            default: throw SDKError(.unexpectedResponse, source: {}, response: response, captureLimit: Self.maxCaptureBytes)\n            }}\n        }} catch let error as {} {{ throw error }}\n        catch is CancellationError {{ {close}throw CancellationError() }}\n        catch let error as ValidationError {{ {close}throw SDKError(.responseDecoding, source: error.source, response: response, captureLimit: Self.maxCaptureBytes, validation: error) }}\n        catch let error as HTTPWireFailure {{ {close}throw SDKError(.responseDecoding, source: error.source, response: response, captureLimit: Self.maxCaptureBytes, json: error.error) }}\n        catch let error as JsonError {{ {close}throw SDKError(.responseDecoding, source: {}, response: response, captureLimit: Self.maxCaptureBytes, json: error) }}\n        catch {{ {close}throw transportFailure(error, source: {}) }}",
        source(&op.source),
        op.error_type,
        source(&op.source),
        source(&op.source)
    );
}
pub(super) fn decode_media(m: &PlannedMedia, plan: &SdkPlan, _streaming: bool) -> String {
    match m.wire.representation(){
        Representation::Json{codec}=>codec.as_ref().map(|c|format!("try Codecs.{}.decode(response.body, limits: JsonLimits(maxBytes: request.maxResponseBytes))",plan.models.codecs[c.schema().id()])).unwrap_or_else(||"try JsonValue.parse(response.body, limits: JsonLimits(maxBytes: request.maxResponseBytes))".into()),
        Representation::Text{codec,scalar,..}=>{
            let value=format!("HTTPBuild.textValue(response.body, contentType: contentType, scalar: {})",meta::scalar(*scalar));
            codec.as_ref().map(|c|format!("try Codecs.{}.decodeValue({value}, limits: JsonLimits(maxBytes: request.maxResponseBytes))",plan.models.codecs[c.schema().id()])).unwrap_or_else(||"try HTTPBuild.textBody(response.body, contentType: contentType)".into())
        },
        Representation::Binary{bytes,..}=>format!("try HTTPParts.bytes(response.body, limit: min(request.maxResponseBytes, {}))",bytes.max_bytes()),
        Representation::Form{..}|Representation::Multipart{..}=>format!("try {}.decode(response.body, contentType: contentType, limit: request.maxResponseBytes, partLimit: Self.maxPartBytes)",m.type_name),
        Representation::Stream{stream}=>stream.item_codec().map_or_else(
            // A schemaless stream surfaces untyped whole-body JSON values because
            // the native stream runtime has no untyped codec.
            || "try JsonValue.parse(response.body, limits: JsonLimits(maxBytes: request.maxResponseBytes))".into(),
            |codec| format!("try HTTPBuild.eventStream(stream, codec: Codecs.{}, contentType: contentType, framing: {}, itemLimit: {}, totalLimit: request.maxResponseBytes, captureLimit: Self.maxCaptureBytes)",plan.models.codecs[codec.schema().id()],if stream.framing()==StreamFraming::ServerSentEvents{".serverSentEvents"}else{".jsonLines"},stream.max_item_bytes())),
    }
}

fn parts_type(out: &mut String, parts: &PlannedParts, plan: &SdkPlan) {
    for p in parts.fields.iter().chain(parts.additional.as_deref()) {
        part_input_type(out, p, plan);
    }
    doc(
        out,
        "Finite form/multipart input. Required fields, extras, counts and individual codecs are checked before transport.",
        "",
    );
    let mut fs = parts
        .fields
        .iter()
        .map(|p| {
            (
                p.field_name.clone(),
                optional(&p.type_name, p.wire.required()),
                p.wire.required(),
            )
        })
        .collect::<Vec<_>>();
    fs.sort_by_key(|(_, _, required)| !required);
    // The extra-property map is a native argument with a separate empty default.
    let _ = writeln!(out, "public struct {}: Sendable {{", parts.type_name);
    for (name, ty, _) in &fs {
        let _ = writeln!(out, "    public var {name}: {ty}");
    }
    if let Some(p) = &parts.additional {
        let _ = writeln!(
            out,
            "    public var additionalProperties: JsonObject<{}>",
            p.type_name
        );
    }
    let mut args = fs
        .iter()
        .map(|(name, ty, required)| {
            format!("{name}: {ty}{}", if *required { "" } else { " = .missing" })
        })
        .collect::<Vec<_>>();
    if let Some(p) = &parts.additional {
        args.push(format!(
            "additionalProperties: JsonObject<{}> = .init()",
            p.type_name
        ));
    }
    let _ = writeln!(out, "    public init({}) {{", args.join(", "));
    for (name, _, _) in &fs {
        let _ = writeln!(out, "        self.{name} = {name}");
    }
    if parts.additional.is_some() {
        out.push_str("        self.additionalProperties = additionalProperties\n");
    }
    out.push_str("    }\n");
    parts_encode(out, parts, plan);
    parts_decode(out, parts, plan);
    out.push_str("}\n\n");
}

pub(super) fn part_input_type(out: &mut String, p: &PlannedPart, plan: &SdkPlan) {
    if let Some(h) = &p.header_type {
        headers_type(out, h, &p.headers, plan);
        let _ = writeln!(
            out,
            "/// In-memory part value with source-typed required headers.\npublic struct {}: Sendable {{\n    public var value: {}\n    public var filename: String?\n    public var contentType: String?\n    public var headers: {h}\n    public init(value: {}, headers: {h}, filename: String? = nil, contentType: String? = nil) {{\n        self.value = value; self.headers = headers; self.filename = filename; self.contentType = contentType\n    }}\n}}",
            p.item_type, p.value_type, p.value_type
        );
    }
}

fn present_names(out: &mut String, parts: &PlannedParts) {
    out.push_str("        var present: [String] = []\n");
    for p in &parts.fields {
        let value = if p.wire.required() {
            format!("self.{}", p.field_name)
        } else {
            "value".into()
        };
        if !p.wire.required() {
            let _ = writeln!(
                out,
                "        if case .value{} = self.{} {{",
                if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
                    "(let value)"
                } else {
                    ""
                },
                p.field_name
            );
        }
        if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
            let _ = writeln!(
                out,
                "        if !{value}.isEmpty {{ present.append({}) }}",
                q(p.wire.name().unwrap())
            );
        } else {
            let _ = writeln!(out, "        present.append({})", q(p.wire.name().unwrap()));
        }
        if !p.wire.required() {
            out.push_str("        }\n");
        }
    }
    if let Some(additional) = &parts.additional {
        if additional.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
            out.push_str("        present.append(contentsOf: additionalProperties.members.filter { !$0.value.isEmpty }.map(\\.key))\n");
        } else {
            out.push_str("        present.append(contentsOf: additionalProperties.keys)\n");
        }
        let _ = writeln!(
            out,
            "        let declared: Set<JsonKey> = [{}]\n        guard !additionalProperties.keys.contains(where: {{ declared.contains(JsonKey($0)) }}) else {{ throw JsonError(.representation, \"extra part collides with a declared property\") }}",
            parts
                .fields
                .iter()
                .map(|p| format!("JsonKey({})", q(p.wire.name().unwrap())))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let _ = writeln!(out, "        try {}.check(present)", meta::rules(parts));
}
fn parts_encode(out: &mut String, parts: &PlannedParts, plan: &SdkPlan) {
    out.push_str("    func encode(contentType: String, boundary selectedBoundary: String?, limit: Int, partLimit: Int) throws -> HTTPBody {\n");
    present_names(out, parts);
    if parts.multipart {
        out.push_str("        let media = try HTTPMediaType(contentType)\n        let boundary = try HTTPParts.boundary(selectedBoundary ?? media.parameters[\"boundary\"])\n        guard media.parameters[\"boundary\"] == nil || media.parameters[\"boundary\"] == boundary else { throw JsonError(.representation, \"conflicting multipart boundaries\") }\n        var output = Data()\n");
    } else {
        out.push_str("        try HTTPMediaType.utf8(contentType)\n        var output = HTTPWireBuffer(limit: limit)\n");
    }
    for p in &parts.fields {
        if !p.wire.required() {
            let _ = writeln!(
                out,
                "        if case .value(let field) = self.{} {{",
                p.field_name
            );
        } else {
            let _ = writeln!(
                out,
                "        do {{\n        let field = self.{}",
                p.field_name
            );
        }
        encode_part(out, p, parts, plan, &q(p.wire.name().unwrap()));
        out.push_str("        }\n");
    }
    if let Some(p) = &parts.additional {
        out.push_str("        for (name, field) in additionalProperties.members {\n");
        encode_part(out, p, parts, plan, "name");
        out.push_str("        }\n");
    }
    if parts.multipart {
        out.push_str("        try HTTPParts.finishMultipart(&output, boundary: boundary, limit: limit)\n        return HTTPBody(bytes: output, contentType: media.parameters[\"boundary\"] == nil ? contentType + \"; boundary=\" + boundary : contentType)\n");
    } else {
        out.push_str("        return HTTPBody(bytes: output.data, contentType: contentType)\n");
    }
    out.push_str("    }\n");
}
fn encode_part(
    out: &mut String,
    p: &PlannedPart,
    parts: &PlannedParts,
    plan: &SdkPlan,
    name: &str,
) {
    let repeated = p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems;
    let _ = writeln!(
        out,
        "        let rule = {}\n        try rule.checkCount({})",
        meta::part(&p.wire, plan),
        if repeated { "field.count" } else { "1" }
    );
    if repeated {
        out.push_str("        for item in field {\n");
    } else {
        out.push_str("        let item = field\n");
    }
    let v = if parts.multipart {
        "item.value"
    } else {
        "item"
    };
    let codec_limit = "min(partLimit, limit - output.count)";
    match p.wire.representation() {
        PartRepresentation::Binary { bytes } => {
            let _ = writeln!(
                out,
                "        let bytes = try HTTPParts.bytes({v}, limit: min({}, {codec_limit}))",
                bytes.max_bytes()
            );
        }
        PartRepresentation::Json { codec, .. } => {
            let _ = writeln!(
                out,
                "        let bytes = try Codecs.{}.encode({v}, limits: JsonLimits(maxBytes: {codec_limit}))",
                plan.models.codecs[codec.schema().id()]
            );
        }
        PartRepresentation::Text { codec, scalar, .. } => {
            let _ = writeln!(
                out,
                "        let text = try HTTPWire.scalar(Codecs.{}.encodeValue({v}, limits: JsonLimits(maxBytes: {codec_limit})), type: {})\n        let bytes = try HTTPParts.textBytes(text, limit: {codec_limit})",
                plan.models.codecs[codec.schema().id()],
                meta::scalar(*scalar)
            );
        }
        PartRepresentation::Style {
            codec,
            serialization,
        } => {
            let descriptor = meta::parameter_value(
                "",
                "query",
                p.wire.required(),
                p.wire.source().use_site().source(),
                serialization,
                codec,
                plan,
            );
            let descriptor = descriptor.replacen("name: \"\"", &format!("name: {name}"), 1);
            if parts.multipart {
                let _ = writeln!(
                    out,
                    "        let text = try HTTPWire.multipartContent(Codecs.{}.encodeValue({v}, limits: JsonLimits(maxBytes: {codec_limit})), serialization: {}, limit: {codec_limit})\n        let bytes = Data(text.utf8)",
                    plan.models.codecs[codec.schema().id()],
                    meta::serialization(serialization, codec, plan)
                );
            } else {
                let _ = writeln!(
                    out,
                    "        let text = try HTTPWire.serialize(Codecs.{}.encodeValue({v}, limits: JsonLimits(maxBytes: {codec_limit})), parameter: {descriptor}, limit: {codec_limit})\n        let bytes = Data(text.utf8)",
                    plan.models.codecs[codec.schema().id()]
                );
            }
        }
    }
    if parts.multipart {
        let headers = if p.header_type.is_some() {
            "try item.headers.encode(limit: partLimit)"
        } else {
            "[]"
        };
        let _ = writeln!(
            out,
            "        try HTTPParts.appendPart(name: {name}, bytes: bytes, headers: {headers}, filename: item.filename, contentType: rule.contentType(item.contentType), boundary: boundary, to: &output, bodyLimit: limit, partLimit: rule.maxBytes)"
        );
    } else {
        match p.wire.representation() {
            PartRepresentation::Style { .. } => out
                .push_str("        try HTTPParts.appendStyle(HTTPWire.text(bytes), to: &output)\n"),
            PartRepresentation::Json { outer_encoding, .. }
            | PartRepresentation::Text { outer_encoding, .. } => {
                let _ = writeln!(
                    out,
                    "        try HTTPParts.appendForm(HTTPWire.text(bytes), name: {name}, encoding: {}, to: &output)",
                    meta::encoding(*outer_encoding)
                );
            }
            _ => unreachable!("form binary declined"),
        }
    }
    if repeated {
        out.push_str("        }\n");
    }
}
fn parts_decode(out: &mut String, parts: &PlannedParts, plan: &SdkPlan) {
    out.push_str("    static func decode(_ bytes: Data, contentType: String, limit: Int, partLimit: Int) throws -> Self {\n        _ = try HTTPParts.bytes(bytes, limit: limit)\n");
    if parts.multipart {
        out.push_str("        let parts = try HTTPParts.parseMultipart(bytes, contentType: contentType, partLimit: partLimit)\n");
    } else {
        out.push_str("        try HTTPMediaType.utf8(contentType)\n        let parts = try HTTPParts.parseForm(bytes, partLimit: partLimit)\n");
    }
    let _ = writeln!(
        out,
        "        try {}.check(parts.map(\\.name))",
        meta::rules(parts)
    );
    for (i, p) in parts.fields.iter().enumerate() {
        let _ = writeln!(
            out,
            "        let values{i} = parts.filter {{ $0.name.utf8.elementsEqual({}.utf8) }}\n        let field{i}: {}",
            q(p.wire.name().unwrap()),
            optional(&p.type_name, p.wire.required())
        );
        if !p.wire.required() {
            let _ = writeln!(
                out,
                "        if values{i}.isEmpty {{ field{i} = .missing }} else {{"
            );
        }
        let _ = writeln!(
            out,
            "        try {}.checkCount(values{i}.count)",
            meta::part(&p.wire, plan)
        );
        let decode = decode_part(p, parts, plan, "part");
        let collection = format!(
            "try values{i}.map {{ part -> {} in {decode} }}",
            p.item_type
        );
        if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
            let _ = writeln!(
                out,
                "        field{i} = {}",
                if p.wire.required() {
                    collection
                } else {
                    format!(".value({collection})")
                }
            );
        } else {
            let _ = writeln!(
                out,
                "        let decoded{i} = {collection}\n        guard let first{i} = decoded{i}.first else {{ throw JsonError(.representation, \"required part is absent\") }}\n        field{i} = {}",
                if p.wire.required() {
                    format!("first{i}")
                } else {
                    format!(".value(first{i})")
                }
            );
        }
        if !p.wire.required() {
            out.push_str("        }\n");
        }
    }
    if let Some(p) = &parts.additional {
        let _ = writeln!(
            out,
            "        var extras = JsonObject<{}>()\n        let names: Set<JsonKey> = [{}]\n        for key in Set(parts.map {{ JsonKey($0.name) }}).filter({{ !names.contains($0) }}).sorted(by: {{ $0.text.utf8.lexicographicallyPrecedes($1.text.utf8) }}) {{\n            let name = key.text\n            let values = parts.filter {{ $0.name.utf8.elementsEqual(name.utf8) }}\n            try {}.checkCount(values.count)\n            let decoded = try values.map {{ part -> {} in {} }}",
            p.type_name,
            parts
                .fields
                .iter()
                .map(|p| format!("JsonKey({})", q(p.wire.name().unwrap())))
                .collect::<Vec<_>>()
                .join(", "),
            meta::part(&p.wire, plan),
            p.item_type,
            decode_part(p, parts, plan, "part")
        );
        if p.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
            out.push_str("            extras[name] = decoded\n");
        } else {
            out.push_str("            guard let value = decoded.first else { throw JsonError(.representation, \"empty extra part\") }; extras[name] = value\n");
        }
        out.push_str("        }\n");
    }
    let mut constructor = parts.fields.iter().enumerate().collect::<Vec<_>>();
    constructor.sort_by_key(|(_, p)| !p.wire.required());
    let mut args = constructor
        .iter()
        .map(|(i, p)| format!("{}: field{i}", p.field_name))
        .collect::<Vec<_>>();
    if parts.additional.is_some() {
        args.push("additionalProperties: extras".into());
    }
    let _ = writeln!(out, "        return Self({})\n    }}", args.join(", "));
}
fn decode_part(p: &PlannedPart, parts: &PlannedParts, plan: &SdkPlan, var: &str) -> String {
    let prefix = if parts.multipart {
        format!(
            "_ = try {}.contentType({var}.contentType); ",
            meta::part(&p.wire, plan)
        )
    } else {
        String::new()
    };
    let value = match p.wire.representation() {
        PartRepresentation::Binary { bytes } => format!(
            "try HTTPParts.bytes({var}.bytes, limit: min(partLimit, {}))",
            bytes.max_bytes()
        ),
        PartRepresentation::Json { codec, .. } => format!(
            "try Codecs.{}.decode({var}.bytes, limits: JsonLimits(maxBytes: partLimit))",
            plan.models.codecs[codec.schema().id()]
        ),
        PartRepresentation::Text { codec, scalar, .. } => format!(
            "try Codecs.{}.decodeValue(HTTPWire.scalarValue(HTTPWire.text({var}.bytes), type: {}), limits: JsonLimits(maxBytes: partLimit))",
            plan.models.codecs[codec.schema().id()],
            meta::scalar(*scalar)
        ),
        PartRepresentation::Style {
            codec,
            serialization,
        } => format!(
            "try Codecs.{}.decodeValue(HTTPWire.parsePartStyle(HTTPWire.text({var}.bytes), name: {var}.name, serialization: {}, multipart: {}), limits: JsonLimits(maxBytes: partLimit))",
            plan.models.codecs[codec.schema().id()],
            meta::serialization(serialization, codec, plan),
            parts.multipart
        ),
    };
    if parts.multipart {
        if let Some(h) = &p.header_type {
            format!(
                "{prefix}return try {}(value: {value}, headers: {h}.decode({var}.headers, limit: partLimit), filename: {var}.filename, contentType: {var}.contentType)",
                p.item_type
            )
        } else {
            format!(
                "{prefix}return {}({value}, filename: {var}.filename, contentType: {var}.contentType)",
                p.item_type
            )
        }
    } else {
        format!("{prefix}return {value}")
    }
}
