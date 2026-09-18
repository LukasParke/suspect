//! Sync/CompletableFuture client methods and typed actual-status/media results.
use super::{
    SdkPlan,
    http::JavaOperation,
    models::{javadoc, q},
    protocol::{JavaMedia, JavaValue},
    wire_emit,
};

pub(crate) fn client(plan: &SdkPlan) -> String {
    let api = &plan.package().api_name;
    let mut out = format!(
        "/** Source-selected synchronous and cancellable asynchronous HTTP operations. */\npublic final class {api} implements AutoCloseable {{\n    private final HttpRuntime runtime;\n    /** Explicit transport and credential policy. @param options options */\n    public {api}(HttpRuntime.Options options) {{ runtime=new HttpRuntime(options); }}\n    /** Complete immutable source protocol metadata. @return metadata */\n    public static JsonValue operationMetadata() {{ return Protocol.at(\"/operations\"); }}\n    /** Cancel owned calls and streams. */\n    @Override public void close() {{ runtime.close(); }}\n"
    );
    if plan.credential_env().is_some() {
        out.push_str(&credential_env(plan));
    }
    for (index, op) in plan.operations().iter().enumerate() {
        let input = &op.input_type;
        let success = &op.success_type;
        let mut fields = op
            .parameters
            .iter()
            .map(|p| {
                (
                    p.native_name.clone(),
                    JavaValue::Model(p.schema.clone()),
                    p.required,
                )
            })
            .collect::<Vec<_>>();
        if let Some(body) = &op.body {
            fields.push((body.native_name.clone(), body.value.clone(), body.required));
        }
        out.push_str(&format!(
            "    /** Immutable inputs for {}. */\n    public static final class {input} {{\n",
            javadoc(&op.operation_id)
        ));
        for (name, value, required) in &fields {
            let ty = value.native_type(plan.models());
            let ty = if *required {
                ty
            } else {
                format!("Presence<{ty}>")
            };
            out.push_str(&format!("        private final {ty} {name};\n        /** Source input. @return immutable value or presence */\n        public {ty} {name}() {{ return {name}; }}\n"));
        }
        out.push_str(&format!(
            "        private {input}(Builder b,ModelCodec.Context c) {{\n"
        ));
        for (name, value, required) in &fields {
            let snapshot = wire_emit::snapshot(
                plan,
                value,
                &format!("b.{name}{}", if *required { "" } else { ".value()" }),
                Some("c"),
            );
            out.push_str(&format!(
                "            this.{name}={};\n",
                if *required {
                    snapshot
                } else {
                    format!("b.{name}.isPresent()?Presence.of({snapshot}):Presence.absent()")
                }
            ));
        }
        out.push_str(&format!("        }}\n        /** Begin with required source inputs. @return builder */\n        public static Builder builder({}) {{ Builder _builder=new Builder();\n",op.constructor.arguments.iter().map(|a|format!("{} {}",a.ty.native_type(plan.models()),a.name)).collect::<Vec<_>>().join(", ")));
        for argument in &op.constructor.arguments {
            out.push_str(&format!("            _builder.{0}={0};\n", argument.name));
        }
        out.push_str("            return _builder;\n        }\n        /** Mutable input construction. */\n        public static final class Builder {\n            private Builder() {}\n");
        for (name, value, required) in &fields {
            let ty = value.native_type(plan.models());
            out.push_str(&format!("            private {} {name}{};\n            /** Set a source input. @param value input @return builder */\n            public Builder {name}({ty} value) {{ this.{name}={}; return this; }}\n",if *required{ty.clone()}else{format!("Presence<{ty}>")},if *required{""}else{"=Presence.absent()"},if *required{"value"}else{"Presence.of(value)"}));
            if !required {
                out.push_str(&format!("            /** Restore absence. @return builder */\n            public Builder omit{}() {{ this.{name}=Presence.absent(); return this; }}\n",crate::rust_models::pascal(name)));
            }
        }
        out.push_str(&format!("            /** Validate and snapshot all supplied inputs. @return immutable input */\n            public {input} build() {{ return new {input}(this,new ModelCodec.Context()); }}\n        }}\n    }}\n"));
        let successes = op.responses.iter().filter(|r| r.can_succeed()).count();
        if successes > 1 {
            out.push_str(&format!("    /** Declared success alternatives selected by actual HTTP status. */\n    public sealed interface {success} extends AutoCloseable {{\n        /** Actual HTTP status. @return status */ int status();\n        /** Immutable received headers. @return headers */ java.util.Map<String,java.util.List<String>> headers();\n        /** Release any streaming body. */ @Override void close();\n    }}\n"));
        }
        for response in &op.responses {
            if response.can_succeed() {
                out.push_str(&result(
                    plan,
                    &response.variant_name,
                    &response.native_type,
                    response.headers.as_ref().map(|h| h.name.as_str()),
                    (successes > 1).then_some(success.as_str()),
                    false,
                ));
            }
            if let Some(name) = &response.error_variant_name {
                out.push_str(&result(
                    plan,
                    name,
                    &response.native_type,
                    response.headers.as_ref().map(|h| h.name.as_str()),
                    None,
                    true,
                ));
            }
        }
        out.push_str(&format!("    private static final HttpRuntime.Operation OP{index}=HttpRuntime.operation({index});\n    private static {success} decode{index}(HttpRuntime.RawResponse raw) {{\n        switch(raw.responseIndex) {{\n"));
        for (rindex, response) in op.responses.iter().enumerate() {
            out.push_str(&format!("            case {rindex}: {{\n"));
            if let Some(headers) = &response.headers {
                out.push_str(&format!("                var typedHeaders={}.read(raw.typedHeaders(Protocol.at({})),raw.context);\n",headers.name,q(&headers.descriptor)));
            }
            let value = decode_response(
                plan,
                &response.value,
                &response.media,
                response.choice_type.as_deref(),
            );
            out.push_str(&format!(
                "                {} data={value};\n",
                response.native_type
            ));
            let args = format!(
                "data, raw{}",
                if response.headers.is_some() {
                    ", typedHeaders"
                } else {
                    ""
                }
            );
            if response.can_succeed() && response.can_fail() {
                out.push_str(&format!("                if(raw.status>=200&&raw.status<300) return new {}({args});\n                throw new {}({args});\n",response.variant_name,response.error_variant_name.as_ref().unwrap()));
            } else if response.can_succeed() {
                out.push_str(&format!(
                    "                return new {}({args});\n",
                    response.variant_name
                ));
            } else {
                out.push_str(&format!(
                    "                throw new {}({args});\n",
                    response.error_variant_name.as_ref().unwrap()
                ));
            }
            out.push_str("            }\n");
        }
        out.push_str(
            "            default: throw raw.failure(\"unexpected-response\");\n        }\n    }\n",
        );
        let method = &op.method_name;
        let asynchronous = &op.async_method_name;
        let description = javadoc(&format!(
            "{} Source: {}#{}",
            op.description,
            op.source.document(),
            op.source.pointer()
        ));
        out.push_str(&format!("    /** {description}\n     * @param input source inputs\n     * @return declared success\n     */\n    public {success} {method}({input} input) {{ return {method}(input,RequestOptions.defaults()); }}\n    /** Explicit per-call source choices and deadline.\n     * @param input source inputs\n     * @param options request policy\n     * @return declared success\n     */\n    public {success} {method}({input} input,RequestOptions options) {{ return HttpRuntime.await({asynchronous}(input,options),OP{index}.source()); }}\n    /** Cancellable asynchronous call. @param input inputs @return future */\n    public java.util.concurrent.CompletableFuture<{success}> {asynchronous}({input} input) {{ return {asynchronous}(input,RequestOptions.defaults()); }}\n    /** Asynchronous call with explicit source choices. @param input inputs @param options policy @return future */\n    public java.util.concurrent.CompletableFuture<{success}> {asynchronous}({input} input,RequestOptions options) {{\n        return runtime.call(OP{index},options,c->{{\n"));
        out.push_str(&prepare_fragment(plan, op));
        out.push_str(&format!("            return new HttpRuntime.Prepared(parameters,body);\n        }},{api}::decode{index});\n    }}\n"));
        if fields.is_empty() {
            out.push_str(&format!("    /** No-input call. @return declared success */\n    public {success} {method}() {{ return {method}({input}.builder().build()); }}\n    /** No-input call with policy. @param options policy @return declared success */\n    public {success} {method}(RequestOptions options) {{ return {method}({input}.builder().build(),options); }}\n    /** No-input asynchronous call. @return future */\n    public java.util.concurrent.CompletableFuture<{success}> {asynchronous}() {{ return {asynchronous}({input}.builder().build()); }}\n    /** No-input asynchronous call with policy. @param options policy @return future */\n    public java.util.concurrent.CompletableFuture<{success}> {asynchronous}(RequestOptions options) {{ return {asynchronous}({input}.builder().build(),options); }}\n"));
        }
    }
    // The typed-events exchange seams share the client's request preparation
    // with the direct asynchronous methods, and only exist when a typed
    // stream operation was admitted.
    out.push_str(&super::stream::client_seams(plan));
    out.push_str("}\n");
    out
}

/// The encoded-parameter and body preparation emitted inside one runtime call,
/// shared verbatim by the direct asynchronous method and the typed-events seam.
pub(crate) fn prepare_fragment(plan: &SdkPlan, op: &JavaOperation) -> String {
    let mut out = String::from(
        "            c.require(input);var parameters=new java.util.ArrayList<HttpRuntime.Parameter>();\n",
    );
    for (pindex, p) in op.parameters.iter().enumerate() {
        out.push_str(&format!("            {}parameters.add(new HttpRuntime.Parameter({pindex},{}.CODEC.encodeValue(input.{}{},c)));\n",if p.required{String::new()}else{format!("if(input.{}.isPresent()) ",p.native_name)},plan.models().codec(&p.schema).holder,p.native_name,if p.required{""}else{".value()"}));
    }
    out.push_str("            WireValue body=null;\n");
    if let Some(body) = &op.body {
        out.push_str(&format!(
            "            {}body={};\n",
            if body.required {
                ""
            } else {
                "if(input.body.isPresent()) "
            },
            wire_emit::write(
                plan,
                &body.value,
                &format!("input.body{}", if body.required { "" } else { ".value()" }),
                "c"
            )
        ));
    }
    out
}

fn credential_env(plan: &SdkPlan) -> String {
    use crate::{
        credential_env::CredentialEnvKind,
        http_protocol::{CredentialHook, ParameterLocation},
    };
    let api = &plan.package().api_name;
    let policy = plan.credential_env().expect("configured policy");
    let mut out = format!(
        r#"    /** Snapshot configured runtime environment variables using source servers.
     * Missing/empty/unusable values stay missing; protected calls fail before HTTP.
     * @return client owning its default transport
     */
    public static {api} fromEnv() {{ return new {api}(_credentialEnvOptions(null, System::getenv)); }}
    /** Snapshot runtime variables with an explicitly supplied, caller-owned transport.
     * @param transport non-null JDK HTTP transport
     * @return client
     */
    public static {api} fromEnv(java.net.http.HttpClient transport) {{
        return fromEnv(java.util.Objects.requireNonNull(transport, "transport"), System::getenv);
    }}
    /** Snapshot an explicit runtime environment accessor once per mapped variable.
     * The accessor may return null or throw when unavailable. A null accessor is
     * unavailable and never falls back to the process environment.
     * @param transport non-null caller-owned transport
     * @param environment variable-name lookup, or null when unavailable
     * @return client retaining the creation-time snapshot
     */
    public static {api} fromEnv(java.net.http.HttpClient transport, java.util.function.Function<String,String> environment) {{
        java.util.Objects.requireNonNull(transport, "transport");
        return new {api}(_credentialEnvOptions(transport, environment));
    }}
    private static String _readCredentialEnv(java.util.function.Function<String,String> environment, java.util.Map<String,String> snapshot, String variable) {{
        if(snapshot.containsKey(variable))return snapshot.get(variable);
        String value=null;
        try {{ if(environment!=null)value=environment.apply(variable); }} catch(RuntimeException unavailable) {{ }}
        if(value!=null&&(value.isEmpty()||value.length()>16384))value=null;
        snapshot.put(variable,value);return value;
    }}
    private static HttpRuntime.Options _credentialEnvOptions(java.net.http.HttpClient transport, java.util.function.Function<String,String> environment) {{
        var options=HttpRuntime.Options.builder();
        if(transport!=null)options.httpClient(transport);
        var snapshot=new java.util.HashMap<String,String>();
"#
    );
    for binding in policy.bindings() {
        let requirement = plan
            .protocol()
            .operations()
            .iter()
            .flat_map(|o| o.security().alternatives())
            .flat_map(|a| a.requirements())
            .find(|r| {
                r.name() == binding.name()
                    && r.scheme().use_site().source() == binding.scheme().use_site().source()
            })
            .expect("shared binder retained a used declaration");
        let (method, check) = match (binding.kind(), requirement.credential()) {
            (CredentialEnvKind::Bearer, CredentialHook::Bearer { .. }) => ("credential", ""),
            (CredentialEnvKind::ApiKey, CredentialHook::ApiKey { location, .. }) => (
                "apiKey",
                match location {
                    ParameterLocation::Header => "HttpWire.headerValue(value);",
                    ParameterLocation::Cookie => "HttpWire.cookieValue(value);",
                    ParameterLocation::Query => "",
                    _ => unreachable!("admitted API key attachment"),
                },
            ),
            _ => unreachable!("environment kind is source-bound"),
        };
        out.push_str(&format!("        {{ String value=_readCredentialEnv(environment,snapshot,{});\n          if(value!=null)try{{ {check} options.{method}({},value); }}catch(RuntimeException unusable){{ }}\n        }}\n",q(binding.variable()),q(binding.name())));
    }
    out.push_str("        return options.build();\n    }\n");
    out
}

fn result(
    _plan: &SdkPlan,
    name: &str,
    data: &str,
    typed_headers: Option<&str>,
    success: Option<&str>,
    error: bool,
) -> String {
    let mut out = format!(
        "    /** Source-declared actual-status {}. Own this value when its body streams. */\n    public static final class {name} {} {{\n",
        if error { "API failure" } else { "success" },
        if error {
            "extends SdkException implements AutoCloseable".into()
        } else {
            format!("implements {}", success.unwrap_or("AutoCloseable"))
        }
    );
    if error {
        out.push_str("        private static final long serialVersionUID=1L;\n");
    }
    let transient = if error { "transient " } else { "" };
    out.push_str(&format!("        private final {transient}{data} data;\n        private final {transient}HttpRuntime.RawResponse response;\n"));
    if let Some(ty) = typed_headers {
        out.push_str(&format!(
            "        private final {transient}{ty} typedHeaders;\n"
        ));
    }
    out.push_str(&format!("        private {name}({data} data,HttpRuntime.RawResponse response{}) {{ {}this.data=data;this.response=response;{} }}\n        /** Typed source content. @return content */\n        public {data} data() {{ return data; }}\n",typed_headers.map_or(String::new(),|h|format!(",{h} typedHeaders")),if error{"super(response.failure(\"api-error\"));"}else{""},if typed_headers.is_some(){"this.typedHeaders=typedHeaders;"}else{""}));
    if !error {
        out.push_str("        /** Actual HTTP status, including range/default matches. @return status */\n        public int status() { return response.status; }\n");
    }
    out.push_str("        /** Raw immutable headers. @return headers */\n        public java.util.Map<String,java.util.List<String>> headers() { return response.headers; }\n        /** Source Link metadata; no operation is called implicitly. @return links */\n        public JsonValue links() { return response.links(); }\n        /** Close any context-owned streaming body. */\n        @Override public void close() { response.close(); }\n");
    if let Some(ty) = typed_headers {
        out.push_str(&format!("        /** Source-typed headers. @return immutable typed headers */\n        public {ty} typedHeaders() {{ return typedHeaders; }}\n"));
    }
    out.push_str("    }\n");
    out
}
fn decode_response(
    plan: &SdkPlan,
    value: &JavaValue,
    media: &[JavaMedia],
    choice: Option<&str>,
) -> String {
    match value {
        JavaValue::NoContent => "NoContent.INSTANCE".into(),
        JavaValue::ResponseBody(inner) => format!(
            "(raw.forbidden?ResponseBody.empty():ResponseBody.content({}))",
            decode_response(plan, inner, media, choice)
        ),
        _ if media.is_empty() => "raw.bytes()".into(),
        _ if choice.is_some() => {
            let name = choice.unwrap();
            let mut expression = "switch(raw.mediaIndex) { ".to_owned();
            for (i, m) in media.iter().enumerate() {
                expression.push_str(&format!(
                    "case {i} -> new {name}.{}({},raw.contentType); ",
                    m.name,
                    wire_emit::response_value(plan, m)
                ));
            }
            expression.push_str("default -> throw raw.failure(\"unexpected-content-type\"); }");
            expression
        }
        _ => wire_emit::response_value(plan, &media[0]),
    }
}
