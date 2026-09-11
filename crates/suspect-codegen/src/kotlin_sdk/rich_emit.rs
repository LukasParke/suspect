use super::{
    emit::{header, kdoc, quote, source, source_value},
    *,
};
use crate::OutFile;
use serde_json::json;
use std::{collections::BTreeSet, fmt::Write};

pub(super) fn package(plan: &Plan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    validation::admit(&plan.contract, &plan.program)?;
    let samples = samples::plan(plan)?;
    let prefix = plan.config.package_name.replace('.', "/");
    let mut files = Vec::new();
    let mut add = |path: String, content: String| {
        files.push(OutFile {
            path: format!("kotlin/{path}"),
            content,
        })
    };
    let validation_runtime = validation::runtime(&plan.program);
    for (name, text) in [
        ("Json.kt", include_str!("Json.kt")),
        ("Validation.kt", validation_runtime.as_ref()),
        ("Http.kt", include_str!("Http.kt")),
        ("Protocol.kt", include_str!("Protocol.kt")),
        ("Payload.kt", include_str!("Payload.kt")),
        ("Streaming.kt", include_str!("Streaming.kt")),
        ("DocumentServers.kt", include_str!("DocumentServers.kt")),
    ] {
        add(
            format!("src/main/kotlin/{prefix}/{name}"),
            text.replace("__PACKAGE__", &plan.config.package_name)
                .replace(
                    "__CREDENTIAL_IDENTITY__",
                    if plan.credential_env().is_some() {
                        "if (declarationCredentials) \"use_site\" else \"terminal\""
                    } else {
                        "\"terminal\""
                    },
                )
                .replace(
                    "__ENV_MODE_DECL__",
                    if plan.credential_env().is_some() {
                        ", declarationCredentials: Boolean"
                    } else {
                        ""
                    },
                )
                .replace(
                    "__ENV_MODE_PASS__",
                    if plan.credential_env().is_some() {
                        ",declarationCredentials"
                    } else {
                        ""
                    },
                ),
        );
    }
    if let Some(environment) = plan.credential_env() {
        add(
            format!("src/main/kotlin/{prefix}/CredentialEnvironment.kt"),
            super::environment::runtime(plan),
        );
        add(
            "docs/credential-env.json".into(),
            serde_json::to_string_pretty(environment).unwrap(),
        );
    }
    add(
        format!("src/main/kotlin/{prefix}/Models.kt"),
        emit::models(plan),
    );
    for file in codec_files::files(plan) {
        add(file.path.strip_prefix("kotlin/").expect("Kotlin artifact").into(), file.content);
    }
    add(format!("src/main/kotlin/{prefix}/Client.kt"), client(plan));
    add(
        format!("src/main/resources/{prefix}/validation.json"),
        serde_json::to_string(&plan.program).unwrap(),
    );
    let protocol = serde_json::to_string(plan.protocol()).unwrap();
    if protocol.len() > 16 * 1024 * 1024 {
        return Err(vec![diagnostic(
            &plan.contract,
            plan.operations[0].source.clone(),
            "kotlin-protocol-size",
            "protocol metadata exceeds the 16 MiB program budget",
        )]);
    }
    add(
        format!("src/main/resources/{prefix}/protocol.json"),
        protocol.clone(),
    );
    add("docs/protocol.json".into(), protocol);
    add(
        format!("src/test/kotlin/{prefix}/GeneratedExamples.kt"),
        samples::examples(plan, &samples),
    );
    if let Some(quickstart) = samples::quickstart(plan, &samples) {
        add(
            format!("src/test/kotlin/{prefix}/Quickstart.kt"),
            format!("{}{quickstart}", header(plan)),
        );
    }
    if let Some(value) = samples::quickstart_manifest(plan, &samples) {
        add(
            "docs/quickstart.json".into(),
            serde_json::to_string_pretty(&value).unwrap(),
        );
    }
    let pom = include_str!("pom.xml")
        .replace("__GROUP__", &plan.config.group_id)
        .replace("__ARTIFACT__", &plan.config.artifact_id)
        .replace("__VERSION__", &plan.config.version)
        .replace("__PACKAGE__", &plan.config.package_name)
        .replace("__KOTLIN__", KOTLIN_VERSION)
        .replace("__COROUTINES__", COROUTINES_VERSION)
        .replace("__DOKKA__", DOKKA_VERSION);
    add("pom.xml".into(), pom);
    add(
        "docs/module.md".into(),
        format!(
            "# Module {}\n\nSource-selected Kotlin/JVM coroutine SDK.\n\n# Package {}\n\nNative constructors, checked codecs, exact JSON values and bounded protocol transports.\n",
            plan.config.artifact_id, plan.config.package_name
        ),
    );
    add(
        "docs/examples.json".into(),
        crate::http_examples::manifest(&plan.examples),
    );
    add(
        "docs/examples.md".into(),
        crate::http_examples::markdown(&plan.examples, "mvn test-compile exec:exec@examples"),
    );
    add("docs/symbols.json".into(), manifest(plan));
    add("docs/coverage.json".into(),format!("{}\n",serde_json::to_string_pretty(&json!({"format":"suspect-kotlin-sdk-coverage-v2","kotlin":KOTLIN_VERSION,"dokka":DOKKA_VERSION,"operations":plan.operations.len(),"schemas":plan.models.symbols().len(),"exampleFindings":plan.examples.diagnostics().len(),"capabilities":plan.protocol.capabilities()})).unwrap()));
    add("docs/reference.md".into(), reference(plan));
    add("README.md".into(), readme(plan, &samples));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn class_fields(
    out: &mut String,
    name: &str,
    fields: &[(String, String, bool)],
    description: &str,
) {
    writeln!(
        out,
        "/** {} */\npublic {} {name}(",
        kdoc(description),
        if fields.is_empty() {
            "class"
        } else {
            "data class"
        }
    )
    .unwrap();
    for (n, ty, required) in fields {
        writeln!(
            out,
            "    /** Source-bound {n}. */\n    public val {n}: {}{},",
            if *required {
                ty.clone()
            } else {
                format!("Presence<{ty}>")
            },
            if *required { "" } else { " = Presence.Absent" }
        )
        .unwrap();
    }
    out.push_str(")\n\n");
}
fn headers(
    out: &mut String,
    plan: &Plan,
    name: &str,
    fields: &[PlannedHeader],
    seen: &mut BTreeSet<String>,
) {
    if !seen.insert(name.into()) {
        return;
    }
    class_fields(
        out,
        name,
        &fields
            .iter()
            .map(|h| (h.name.clone(), h.ty.kotlin(&plan.models), h.wire.required()))
            .collect::<Vec<_>>(),
        "Typed source-declared HTTP headers. Missing optional fields remain Absent.",
    );
    writeln!(out,"internal fun write{name}(value: {name}, budget: ModelBudget): Map<String, JsonValue> {{\n    val result = linkedMapOf<String, JsonValue>()").unwrap();
    for h in fields {
        let k = quote(h.wire.name());
        let enc = format!(
            "Codecs.{}.encodeUsing({}, budget, {k})",
            h.codec_name,
            if h.wire.required() {
                format!("value.{}", h.name)
            } else {
                "member.value".into()
            }
        );
        if h.wire.required() {
            writeln!(out, "    result[{k}] = {enc}").unwrap();
        } else {
            writeln!(out,"    when (val member = value.{}) {{ Presence.Absent -> Unit; is Presence.Present -> result[{k}] = {enc} }}",h.name).unwrap();
        }
    }
    out.push_str("    return result\n}\n");
    writeln!(out,"internal fun read{name}(values: Map<String, JsonValue>, budget: ModelBudget): {name} = {name}(").unwrap();
    for h in fields {
        let k = quote(h.wire.name());
        let val = format!(
            "Codecs.{}.decodeUsing(values.getValue({k}), budget, {k})",
            h.codec_name
        );
        writeln!(
            out,
            "    {} = {},",
            h.name,
            if h.wire.required() {
                val
            } else {
                format!("if (values.containsKey({k})) Presence.Present({val}) else Presence.Absent")
            }
        )
        .unwrap();
    }
    out.push_str(")\n\n");
}
fn wire_types(out: &mut String, plan: &Plan, m: &PlannedMedia, seen: &mut BTreeSet<String>) {
    let Some(form) = &m.form else {
        return;
    };
    if !seen.insert(form.name.clone()) {
        return;
    }
    for p in form
        .fields
        .iter()
        .chain(form.additional.iter().map(|p| p.as_ref()))
    {
        if let Some(name) = &p.headers_type {
            headers(out, plan, name, &p.headers, seen);
        }
        if let Some(name) = &p.wrapper
            && seen.insert(name.clone())
        {
            writeln!(out,"/** Native part payload and its declared headers/media choice. */\npublic data class {name}(\n    /** Part payload. */ public val value: {},",p.value_type.kotlin(&plan.models)).unwrap();
            if let Some(h) = &p.headers_type {
                writeln!(
                    out,
                    "    /** Typed part headers. */ public val headers: {h}{},",
                    if p.headers.iter().all(|h| !h.wire.required()) {
                        format!(" = {h}()")
                    } else {
                        String::new()
                    }
                )
                .unwrap();
            }
            out.push_str("    /** Concrete source-permitted media choice. */ public val contentType: String? = null,\n)\n");
        }
    }
    let mut fields = form
        .fields
        .iter()
        .map(|p| (p.name.clone(), p.ty.kotlin(&plan.models), p.wire.required()))
        .collect::<Vec<_>>();
    writeln!(out,"/** Finite source-defined {} body. Binary fields remain actual bytes. Source: {} */\npublic {} {}(",if form.multipart{"multipart"}else{"form"},source(&form.source),if fields.is_empty()&&form.additional.is_none(){"class"}else{"data class"},form.name).unwrap();
    for (name, ty, required) in fields.drain(..) {
        writeln!(
            out,
            "    /** Native field {name}. */ public val {name}: {}{},",
            if required {
                ty
            } else {
                format!("Presence<{ty}>")
            },
            if required { "" } else { " = Presence.Absent" }
        )
        .unwrap();
    }
    if let Some(p) = &form.additional {
        writeln!(out,"    /** Source-permitted extra fields. Declared names are reserved. */ public val additionalProperties: Map<String, {}> = emptyMap(),",p.ty.kotlin(&plan.models)).unwrap();
    }
    out.push_str(")\n");
    writeln!(out,"internal fun write{}(value: {}, budget: ModelBudget): ProtocolValue {{\n    val fields = mutableListOf<PreparedPart>()",form.name,form.name).unwrap();
    for p in &form.fields {
        let raw = if p.wire.required() {
            format!("value.{}", p.name)
        } else {
            "member.value".into()
        };
        let expr = write_part(plan, p, &raw, "budget");
        let key = quote(p.wire.name().unwrap());
        if p.wire.required() {
            writeln!(out, "    fields.add(PreparedPart({key}, {expr}))").unwrap();
        } else {
            writeln!(out,"    when (val member = value.{}) {{ Presence.Absent -> Unit; is Presence.Present -> fields.add(PreparedPart({key}, {expr})) }}",p.name).unwrap();
        }
    }
    if let Some(p) = &form.additional {
        let keys = form
            .fields
            .iter()
            .map(|p| quote(p.wire.name().unwrap()))
            .collect::<Vec<_>>()
            .join(",");
        let expr = write_part(plan, p, "member", "budget");
        writeln!(out,"    budget.collection(value.additionalProperties.size, {}, \"\")\n    for ((key, member) in value.additionalProperties) {{\n        budget.text(key, {}, key)\n        if (key in setOf<String>({keys})) throw SdkException(FailureKind.REQUEST_REPRESENTATION, \"extra part collides with declared field\")\n        fields.add(PreparedPart(key, {expr}))\n    }}",source_value(&form.source),source_value(&form.source)).unwrap();
    }
    out.push_str("    return ProtocolValue.Parts(fields)\n}\n");
    writeln!(out,"internal fun read{}(value: ProtocolValue, budget: ModelBudget): {} {{\n    val parts = (value as ProtocolValue.Parts).values\n    budget.collection(parts.size, {}, \"\")\n    val fields = parts.associateBy {{ it.name }}\n    return {}(",form.name,form.name,source_value(&form.source),form.name).unwrap();
    for p in &form.fields {
        let key = quote(p.wire.name().unwrap());
        let expr = read_part(plan, p, &format!("fields.getValue({key}).values"));
        writeln!(
            out,
            "        {} = {},",
            p.name,
            if p.wire.required() {
                expr
            } else {
                format!(
                    "if (fields.containsKey({key})) Presence.Present({expr}) else Presence.Absent"
                )
            }
        )
        .unwrap();
    }
    if let Some(p) = &form.additional {
        let keys = form
            .fields
            .iter()
            .map(|p| quote(p.wire.name().unwrap()))
            .collect::<Vec<_>>()
            .join(",");
        let expr = read_part(plan, p, "entry.value.values");
        writeln!(out,"        additionalProperties = fields.filterKeys {{ it !in setOf<String>({keys}) }}.mapValues {{ entry -> {expr} }},").unwrap();
    }
    out.push_str("    )\n}\n");
}
fn write_part(plan: &Plan, p: &PlannedPart, value: &str, budget: &str) -> String {
    let expression = |value: &str| {
        let inner = if p.wrapper.is_some() {
            format!("{value}.value")
        } else {
            value.into()
        };
        let headers = if p.wrapper.is_some()
            && let Some(headers_type) = &p.headers_type
        {
            format!("write{headers_type}({value}.headers, {budget})")
        } else {
            "emptyMap()".into()
        };
        let (body, ctype, filename) = if matches!(&p.value_type,NativeType::Named(n)if n=="Upload")
        {
            (
                format!(
                    "ProtocolValue.Bytes({inner}.data.also {{ {budget}.bytes(it.size, {}, \"\") }})",
                    source_value(p.wire.schema().id())
                ),
                if p.wrapper.is_some() {
                    format!("{value}.contentType ?: {inner}.contentType")
                } else {
                    format!("{inner}.contentType")
                },
                format!("{inner}.filename"),
            )
        } else {
            (
                encode_type(plan, &p.value_type, &inner, budget, p.wire.schema().id()),
                if p.wrapper.is_some() {
                    format!("{value}.contentType")
                } else {
                    "null".into()
                },
                "null".into(),
            )
        };
        format!(
            "{budget}.at({}, \"\") {{ ProtocolPartValue({body}, {headers}, {ctype}, {filename}) }}",
            source_value(p.wire.schema().id())
        )
    };
    if p.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
        format!(
            "{value}.also {{ {budget}.collection(it.size, {}, \"\") }}.map {{ item -> {} }}",
            source_value(p.wire.schema().id()),
            expression("item")
        )
    } else {
        format!("listOf({})", expression(value))
    }
}
fn read_part(plan: &Plan, p: &PlannedPart, value: &str) -> String {
    let expression = |value: &str| {
        let inner = if matches!(&p.value_type,NativeType::Named(n)if n=="Upload") {
            format!(
                "Upload(({value}.value as ProtocolValue.Bytes).value.also {{ budget.bytes(it.size, {}, \"\") }}, {value}.filename, {value}.contentType)",
                source_value(p.wire.schema().id())
            )
        } else {
            decode_type(
                plan,
                &p.value_type,
                &format!("{value}.value"),
                "budget",
                p.wire.schema().id(),
            )
        };
        let decoded = if let Some(wrapper) = &p.wrapper {
            format!(
                "{wrapper}(value = {inner}, {}contentType = {value}.contentType)",
                p.headers_type
                    .as_ref()
                    .map(|name| format!("headers = read{name}({value}.headers, budget), "))
                    .unwrap_or_default()
            )
        } else {
            inner
        };
        format!(
            "budget.at({}, \"\") {{ {decoded} }}",
            source_value(p.wire.schema().id())
        )
    };
    if p.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
        format!(
            "{value}.also {{ budget.collection(it.size, {}, \"\") }}.map {{ item -> {} }}",
            source_value(p.wire.schema().id()),
            expression("item")
        )
    } else {
        expression(&format!("{value}.single()"))
    }
}
pub(super) fn encode_type(
    plan: &Plan,
    ty: &NativeType,
    value: &str,
    budget: &str,
    at: &SourceId,
) -> String {
    match ty {
        NativeType::Model(id) => format!(
            "ProtocolValue.Json(Codecs.{}.encodeUsing({value}, {budget}, \"\"))",
            plan.models.get(id).codec_name
        ),
        NativeType::Json => format!(
            "ProtocolValue.Json({budget}.representation({budget}.json({value}, {}, \"\")))",
            source_value(at)
        ),
        NativeType::String => format!(
            "ProtocolValue.Json(JsonString({budget}.text({value}, {}, \"\")))",
            source_value(at)
        ),
        NativeType::Boolean => format!("ProtocolValue.Json(JsonBoolean({value}))"),
        NativeType::Number => format!("ProtocolValue.Json({value})"),
        NativeType::Bytes => format!(
            "ProtocolValue.Bytes({value}.also {{ {budget}.bytes(it.size, {}, \"\") }})",
            source_value(at)
        ),
        NativeType::Named(name) => format!("write{name}({value}, {budget})"),
        NativeType::Unit => "ProtocolValue.None".into(),
        _ => unreachable!("aggregate encoding has its own planned converter"),
    }
}
fn decode_type(
    plan: &Plan,
    ty: &NativeType,
    value: &str,
    budget: &str,
    source: &SourceId,
) -> String {
    match ty {
        NativeType::Model(id) => format!(
            "Codecs.{}.decodeUsing(({value} as ProtocolValue.Json).value, {budget}, \"\")",
            plan.models.get(id).codec_name
        ),
        NativeType::Json => format!(
            "{budget}.representation({budget}.json(({value} as ProtocolValue.Json).value, {}, \"\"))",
            source_value(source)
        ),
        NativeType::String => {
            format!("(({value} as ProtocolValue.Json).value as JsonString).value")
        }
        NativeType::Boolean => {
            format!("(({value} as ProtocolValue.Json).value as JsonBoolean).value")
        }
        NativeType::Number => format!("({value} as ProtocolValue.Json).value as JsonNumber"),
        NativeType::Bytes => format!(
            "({value} as ProtocolValue.Bytes).value.also {{ {budget}.bytes(it.size, {}, \"\") }}",
            source_value(source)
        ),
        NativeType::Named(name) => format!("read{name}({value}, {budget})"),
        NativeType::Unit => "Unit".into(),
        _ => unreachable!(),
    }
}

fn client(plan: &Plan) -> String {
    let mut out = header(plan);
    out.push_str("import kotlinx.coroutines.channels.ProducerScope\nimport kotlinx.coroutines.flow.Flow\nimport kotlinx.coroutines.flow.channelFlow\nimport kotlinx.coroutines.flow.buffer\n\n");
    out.push_str("/** Explicit source credential values. Values never appear in diagnostics. */\npublic class Credentials(\n");
    for c in plan.credentials.values() {
        writeln!(
            out,
            "    /** Credential for source scheme &#96;{}&#96;. */ public val {}: {} = null,",
            kdoc(&c.scheme_name),
            c.name,
            c.kotlin_type
        )
        .unwrap();
    }
    out.push_str(") {\n    override fun toString(): String = \"Credentials([redacted])\"\n    internal fun values(): Map<String, ProtocolCredential> = buildMap {\n");
    for c in plan.credentials.values() {
        let kind = match c.wire.credential() {
            wire::CredentialHook::Basic => "Basic",
            wire::CredentialHook::OAuth2 { .. } | wire::CredentialHook::OpenIdConnect { .. } => {
                "Provider"
            }
            _ => "Token",
        };
        writeln!(
            out,
            "        {}?.let {{ put({}, ProtocolCredential.{kind}(it)) }}",
            c.name,
            quote(&format!("{}#{}", c.source.document(), c.source.pointer()))
        )
        .unwrap();
    }
    out.push_str("    }\n}\n");
    let mut seen = BTreeSet::new();
    for op in &plan.operations {
        if let Some(body) = &op.body {
            for m in &body.media {
                wire_types(&mut out, plan, m, &mut seen);
            }
            if let Some(name) = &body.choice_type {
                writeln!(out,"/** Explicit source request media choice. */\npublic sealed interface {name} {{").unwrap();
                for m in &body.media {
                    writeln!(out,"    /** Source media &#96;{}&#96;. */\n    public data class {}(/** Native payload. */ public val value: {}) : {name}",kdoc(m.wire.media_type().declared()),m.name,m.ty.kotlin(&plan.models)).unwrap();
                }
                out.push_str("}\n");
            }
        }
        for r in &op.responses {
            if let Some(m) = &r.media {
                wire_types(&mut out, plan, m, &mut seen);
            }
            if let Some(name) = &r.headers_type {
                headers(&mut out, plan, name, &r.headers, &mut seen);
            }
        }
        let fields = op
            .parameters
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    plan.models.get(&p.schema).kotlin_type.clone(),
                    p.required,
                )
            })
            .chain(
                op.body
                    .iter()
                    .map(|b| (b.name.clone(), b.ty.kotlin(&plan.models), b.required)),
            )
            .collect::<Vec<_>>();
        class_fields(
            &mut out,
            &op.input_type,
            &fields,
            &format!("Source inputs for {}.", op.operation_id),
        );
        writeln!(out,"/** Declared success cases for &#96;{}&#96;. */\npublic sealed interface {} {{\n    /** Actual HTTP metadata. */ public val response: ResponseInfo",kdoc(&op.operation_id),op.result_type).unwrap();
        if let Some(ty) = &op.result_data_type {
            writeln!(
                out,
                "    /** Shared native success payload type. */ public val data: {ty}"
            )
            .unwrap();
        }
        for r in op.responses.iter().filter(|r| r.success) {
            response_class(&mut out, r, &op.result_type, op.result_data_type.is_some());
        }
        out.push_str("}\n");
        writeln!(out,"/** Declared API failures for &#96;{}&#96;. */\npublic sealed class {}(response: ResponseInfo) : ApiException({}, {}, response) {{",kdoc(&op.operation_id),op.error_type,quote(&op.operation_id),source_value(&op.source)).unwrap();
        for r in op.responses.iter().filter(|r| !r.success) {
            response_class(&mut out, r, &op.error_type, false);
        }
        out.push_str("}\n");
    }
    if plan.credential_env().is_some() {
        out.push_str(super::environment::client_constructors());
    } else {
        out.push_str("/** Source-selected coroutine client. Close it with Kotlin use. */\npublic class Client(\n    private val credentials: Credentials = Credentials(),\n    private val transport: Transport = JdkTransport(),\n    private val options: ClientOptions = ClientOptions(),\n) : AutoCloseable {\n    /** Release this client's transport. */\n    override fun close() { try { (transport as? AutoCloseable)?.close() } catch (error: Exception) { throw SdkException(FailureKind.TRANSPORT, \"transport cleanup failed\", cause = error) } }\n");
    }
    for (i, op) in plan.operations.iter().enumerate() {
        operation(&mut out, plan, op, i);
    }
    out.push_str("}\n");
    out
}
fn response_class(out: &mut String, r: &PlannedResponse, parent: &str, override_data: bool) {
    writeln!(out,"    /** Source status &#96;{}&#96;, {}. */\n    public {} {}(\n        /** Validated native payload. */ public {}val data: {},\n        /** Actual bounded HTTP metadata. */ {}response: ResponseInfo,",r.status_key,if r.stream{"parsed item stream"}else{"finite response"},if r.success{"data class"}else{"class"},r.variant_name,if override_data{"override "}else{""},r.kotlin_type,if r.success{"public override val "}else{""}).unwrap();
    if let Some(ty) = &r.headers_type {
        writeln!(
            out,
            "        /** Typed declared response headers. */ public val responseHeaders: {ty},"
        )
        .unwrap();
    }
    writeln!(
        out,
        "    ) : {parent}{}",
        if r.success { "" } else { "(response)" }
    )
    .unwrap();
}
fn operation(out: &mut String, plan: &Plan, op: &PlannedOperation, index: usize) {
    writeln!(out,"    /** {} Source: {}\n     * Caller cancellation is preserved; configured deadlines cover the entire exchange/collection.\n     */",kdoc(op.wire.description().map(|d|d.value().as_str()).unwrap_or(&op.operation_id)),source(&op.source)).unwrap();
    let default = if op.input_has_default() {
        format!(" = {}()", op.input_type)
    } else {
        String::new()
    };
    if op.flow {
        writeln!(out,"    public fun {}(input: {}{default}, requestOptions: RequestOptions = RequestOptions()): Flow<{}> = channelFlow {{",op.method_name,op.input_type,op.result_type).unwrap();
    } else {
        writeln!(out,"    public suspend fun {}(input: {}{default}, requestOptions: RequestOptions = RequestOptions()): {} {{",op.method_name,op.input_type,op.result_type).unwrap();
    }
    writeln!(out,"        {}operation({}, {}, requestOptions.timeout ?: options.timeout) {{ control ->\n            val descriptor = ProtocolData.operations[{index}]\n            val budget = ModelBudget(options.codecLimits, control::check)\n            val parameters = linkedMapOf<Int, JsonValue>()",if op.flow{""}else{"return "},quote(&op.operation_id),source_value(&op.source)).unwrap();
    for (i, p) in op.parameters.iter().enumerate() {
        let value = if p.required {
            format!("input.{}", p.name)
        } else {
            "member.value".into()
        };
        let expr = format!(
            "requestValue {{ Codecs.{}.encodeUsing({value}, budget, {}) }}",
            p.codec_name,
            quote(&p.wire_name)
        );
        if p.required {
            writeln!(out, "            parameters[{i}] = {expr}").unwrap();
        } else {
            writeln!(out,"            when (val member = input.{}) {{ Presence.Absent -> Unit; is Presence.Present -> parameters[{i}] = {expr} }}",p.name).unwrap();
        }
    }
    if let Some(body) = &op.body {
        let value = if body.required {
            "input.body"
        } else {
            "member.value"
        };
        let expr = body_encode(plan, body, value);
        if body.required {
            writeln!(out, "            val body = requestValue {{ {expr} }}").unwrap();
        } else {
            writeln!(out,"            val body = when (val member = input.body) {{ Presence.Absent -> null; is Presence.Present -> requestValue {{ {expr} }} }}").unwrap();
        }
    } else {
        out.push_str("            val body: PreparedBody? = null\n");
    }
    if plan.credential_env().is_some() {
        out.push_str("            val request = requestValue { ProtocolRuntime.prepare(descriptor, PreparedInput(parameters, body), environmentCredentials ?: credentials.values(), options, requestOptions, control, environmentCredentials != null) }\n");
    } else {
        out.push_str("            val request = requestValue { ProtocolRuntime.prepare(descriptor, PreparedInput(parameters, body), credentials.values(), options, requestOptions, control) }\n");
    }
    if op.flow {
        writeln!(out,"            collectProtocol(descriptor, transport, request, options, requestOptions, control) {{ response ->\n                send(decode{index}(response, ModelBudget(options.codecLimits, control::check)))\n            }}\n        }}\n    }}.buffer(0)").unwrap();
    } else {
        writeln!(out,"            val (response, info) = exchange(transport, request, options.captureBytes, control)\n            responseWork(info) {{ decode{index}(ProtocolRuntime.response(descriptor, response, info, requestOptions, control), ModelBudget(options.codecLimits, control::check)) }}\n        }}\n    }}").unwrap();
    }
    writeln!(out,"    private fun decode{index}(response: ProtocolResponse, budget: ModelBudget): {} = responseWork(response.info) {{\n        when {{",op.result_type).unwrap();
    for r in &op.responses {
        let media = r.media_index.map_or("null".into(), |i| i.to_string());
        let value = decode_type(plan, &r.ty, "response.value", "budget", &r.source);
        let header = r
            .headers_type
            .as_ref()
            .map(|name| format!(", read{name}(response.headers, budget)"))
            .unwrap_or_default();
        writeln!(out,"            response.responseIndex == {} && response.mediaIndex == {media} && response.forbidden == {} && (response.info.status in 200..299) == {} -> {}{}({value}, response.info{header})",r.response_index,r.disposition==wire::ResponseBodyDisposition::ForbiddenByHttp,r.success,if r.success{""}else{"throw "},r.constructor).unwrap();
    }
    out.push_str("            else -> throw SdkException(FailureKind.UNEXPECTED_STATUS, \"response has no native status/media case\", response.info)\n        }\n    }\n");
}
fn body_encode(plan: &Plan, body: &PlannedBody, value: &str) -> String {
    if let Some(name) = &body.choice_type {
        let mut out = format!("when (val selected = {value}) {{ ");
        for (i, m) in body.media.iter().enumerate() {
            write!(
                out,
                "is {name}.{} -> PreparedBody({i}, {}); ",
                m.name,
                encode_request_media(plan, m, i, "selected.value", &body.source)
            )
            .unwrap();
        }
        out.push('}');
        out
    } else {
        format!(
            "PreparedBody(0, {})",
            encode_request_media(plan, &body.media[0], 0, value, &body.source)
        )
    }
}
fn encode_request_media(
    plan: &Plan,
    media: &PlannedMedia,
    index: usize,
    value: &str,
    source: &SourceId,
) -> String {
    if matches!(media.ty, NativeType::Flow(_)) {
        format!(
            "ProtocolValue.Bytes(encodeRequestItems({value}, (descriptor.obj(\"body\").array(\"media\")[{index}] as JsonObject).obj(\"representation\").obj(\"stream\"), options.maxRequestBytes, options.maxStreamItemBytes, control::check) {{ item -> Codecs.{}.encodeUsing(item, budget, \"\") }})",
            media.codec_name.as_ref().expect("stream item codec")
        )
    } else {
        encode_type(plan, &media.ty, value, "budget", source)
    }
}
fn manifest(plan: &Plan) -> String {
    serde_json::to_string_pretty(&json!({"format":"suspect-kotlin-sdk-symbols-v2","operations":plan.operations.iter().map(|op|json!({"operationId":op.operation_id,"source":{"document":op.source.document().as_str(),"pointer":op.source.pointer()},"method":op.method_name,"input":op.input_type,"constructor":op.constructor,"result":op.result_type,"resultDataType":op.result_data_type,"flow":op.flow,"apiException":op.error_type,"responses":op.responses.iter().map(|r|json!({"status":r.status_key,"success":r.success,"constructor":r.constructor,"type":r.kotlin_type,"stream":r.stream,"headers":r.headers_type})).collect::<Vec<_>>()})).collect::<Vec<_>>(),"schemas":plan.models.symbols().iter().map(|s|json!({"name":s.name,"type":s.kotlin_type,"codec":s.codec_name,"source":{"document":s.source.document().as_str(),"pointer":s.source.pointer()}})).collect::<Vec<_>>()})).unwrap()
}
fn reference(plan: &Plan) -> String {
    let mut out = String::from("# Kotlin native reference\n\n");
    for op in &plan.operations {
        writeln!(out,"## `{}`\n\n`Client.{}(input: {}, requestOptions: RequestOptions = RequestOptions())`: {}.\n\nSource: {}\n",kdoc(&op.operation_id),op.method_name,op.input_type,if op.flow{format!("Flow<{}>",op.result_type)}else{op.result_type.clone()},source(&op.source)).unwrap();
        for r in &op.responses {
            writeln!(
                out,
                "- `{}`: `{}`; status `{}`; {}.\n",
                r.constructor,
                r.kotlin_type,
                r.status_key,
                if r.success {
                    "success"
                } else {
                    "typed API exception"
                }
            )
            .unwrap();
        }
    }
    for s in plan.models.symbols() {
        writeln!(
            out,
            "### `{}`\n\nNative type `{}`; codec `Codecs.{}`. Source: {}\n",
            s.name,
            s.kotlin_type,
            s.codec_name,
            source(&s.source)
        )
        .unwrap();
    }
    out
}
fn readme(plan: &Plan, samples: &samples::Samples<'_>) -> String {
    let mut out = format!(
        "# {} — Kotlin/JVM SDK\n\nKotlin {KOTLIN_VERSION}, coroutines {COROUTINES_VERSION}, JVM 21+.\n\n```sh\nmvn verify\nmvn install\n```\n\nMaven dependency: `{}:{}:{}`.\n\n",
        plan.config.artifact_id, plan.config.group_id, plan.config.artifact_id, plan.config.version
    );
    if let Some(code) = samples::quickstart(plan, samples) {
        writeln!(
            out,
            "## First request\n\n```kotlin\nimport {}.*\n{code}\n```\n",
            plan.config.package_name
        )
        .unwrap();
    }
    if plan.program.version != OwnedProgram::V1_VERSION {
        out.push_str(&include_str!("guide.md").replace("Native intersections, positional tuples, active directional views, OpenAPI 3.0,\nimplicit extension profiles", "Active directional views, OpenAPI 3.0,\nimplicit extension profiles"));
        if plan.program.version == OwnedProgram::V2_VERSION {
            out.push_str(include_str!("guide-v2.md"));
        } else {
            out.push_str(&include_str!("guide-v2.md").replace("suspect.validation.experimental.v2", "suspect.validation.experimental.v3").replace("oas31-jsonschema202012-static-applicators", "oas31-jsonschema202012-resources-dynamic").replace("The scoped runtime is emitted only for a v2 program.", "The scoped runtime is emitted for a v2 or v3 program.").replace("Dynamic/resource-scope\nvalidation remains a separate, explicitly unimplemented native profile.", "Resource/dynamic validation uses the indexed v3 profile described below."));
            out.push_str(include_str!("guide-v3.md"));
        }
    } else {
        out.push_str(include_str!("guide.md"));
    }
    if plan.credential_env().is_some() {
        out.push_str(include_str!("guide-env.md"));
    }
    out
}
