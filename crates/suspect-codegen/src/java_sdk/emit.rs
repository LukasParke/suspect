//! Deterministic Maven artifacts rendered from one retained native plan.

use super::{
    SdkPlan,
    models::{JavaDeclaration, javadoc, q},
};
use crate::{OutFile, http_contract::HttpDiagnostic};
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) fn render(plan: &SdkPlan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    super::validation::check(plan.contract(), plan.program())?;
    let package = &plan.package().package;
    let group = plan.maven_group_id();
    let api = &plan.package().api_name;
    let version = &plan.package().version;
    let artifact = &plan.maven().artifact_id;
    let release = plan.maven().java_release;
    let path = package.replace('.', "/");
    let prefix = format!("java/src/main/java/{path}");
    let resources = format!("java/src/main/resources/{path}");
    let mut files = BTreeMap::new();
    for (name, code) in [
        ("JsonRuntime", include_str!("JsonRuntime.java")),
        ("Presence", include_str!("Presence.java")),
        ("Never", include_str!("Never.java")),
        ("ModelCodec", include_str!("ModelCodec.java")),
        ("CodecException", include_str!("CodecException.java")),
        ("Validation", include_str!("Validation.java")),
        (
            "ValidationResources",
            include_str!("ValidationResources.java"),
        ),
        ("HttpRuntime", include_str!("HttpRuntime.java")),
        ("SdkException", include_str!("SdkException.java")),
        ("Bytes", include_str!("Bytes.java")),
        ("NoContent", include_str!("NoContent.java")),
        ("ResponseBody", include_str!("ResponseBody.java")),
        ("Protocol", include_str!("Protocol.java")),
        ("HttpWire", include_str!("HttpWire.java")),
        ("WireValue", include_str!("WireValue.java")),
        ("WireCodec", include_str!("WireCodec.java")),
        ("EventStream", include_str!("EventStream.java")),
        ("RequestOptions", include_str!("RequestOptions.java")),
        ("ExactHttp", include_str!("ExactHttp.java")),
    ] {
        files.insert(
            format!("{prefix}/{name}.java"),
            code.replace("{package}", package),
        );
    }
    let header = format!("package {package};\nimport static {package}.JsonRuntime.*;\n\n");
    files.insert(
        format!("{prefix}/Attribution.java"),
        attribution_source(plan, package),
    );
    // Generated pagination walks exist only under configured SDK defaults with
    // at least one emittable paginated operation.
    if let Some(pagination) = super::pagination::source(plan) {
        files.insert(
            format!("{prefix}/Pagination.java"),
            format!("{header}{}", pagination),
        );
    }
    // Generated typed-event decoding exists only for discriminated SSE stream
    // operations; without them this contributes nothing at all.
    if let Some(stream_events) = super::stream::source(plan) {
        files.insert(
            format!("{prefix}/StreamEvents.java"),
            format!("{header}{}", stream_events),
        );
    }
    // Generated OAuth lifecycle exists only under configured SDK defaults with
    // at least one usable scheme; without them this contributes nothing at all.
    if let Some(oauth) = super::oauth::source(plan) {
        files.insert(format!("{prefix}/OAuth.java"), format!("{header}{}", oauth));
    }
    // Generated incoming receipt helpers exist only when the source declares
    // webhooks or callbacks; without them this contributes nothing at all, so
    // receipt-less packages stay byte-identical.
    if let Some(incoming) = super::incoming::source(plan) {
        files.insert(
            format!("{prefix}/Incoming.java"),
            format!("{header}{}", incoming),
        );
    }
    for symbol in plan.models().symbols() {
        files.insert(
            format!("{prefix}/{}.java", symbol.name()),
            format!(
                "{header}{}",
                super::model_emit::symbol(plan.models(), symbol)
            ),
        );
    }
    for file in super::wire_emit::files(plan) {
        files.insert(file.path, file.content);
    }
    files.insert(
        format!("{prefix}/{api}.java"),
        format!("{header}{}", super::http::client(plan)),
    );
    files.insert(
        format!("{prefix}/SdkExamples.java"),
        super::docs::examples_source(plan),
    );
    files.insert(format!("{prefix}/package-info.java"), format!("/**\n * Source-selected OpenAPI {} SDK for Java {release}.\n * Models are immutable snapshots. Optional values use {{@link {package}.Presence}};\n * exact numbers use {{@link {package}.JsonRuntime.JsonNumber}}.\n * Codecs share bounded sessions across every nested conversion and union arm.\n * See {{@link {package}.{api}}} for sync and cancellable asynchronous operations,\n * and {{@link {package}.SdkExamples}} for executable source-bound examples.\n */\npackage {package};\n", javadoc(plan.openapi_version())));
    files.insert(
        format!("{resources}/validation-program.json"),
        serde_json::to_string(plan.program()).expect("checked program"),
    );
    let protocol = serde_json::to_string(plan.protocol()).expect("protocol data");
    if protocol.len() > 32 * 1024 * 1024 {
        return Err(vec![super::plan_diag(
            plan.contract(),
            "java-protocol-resource-limit",
            "HTTP protocol program exceeds the native metadata loading ceiling",
        )]);
    }
    files.insert(format!("{resources}/protocol-program.json"), protocol);
    let mut manifest = json!({
        "format":"suspect-java-sdk-v3", "profile":"java-http-protocol-v1", "javaRelease":release,
        "package":package,"artifactId":artifact,"version":version,"client":api,"openapiVersion":plan.openapi_version(),
        "models":plan.models().symbols().iter().map(|s| {
            let declaration = match s.declaration() {
                JavaDeclaration::Alias(ty) => json!({"kind":"alias","nativeType":plan.models().render_type(ty)}),
                JavaDeclaration::Object { fields, extras, constructor } => json!({
                    "kind":"object","constructor":{"name":constructor.name,"arguments":constructor.arguments.iter().map(|a|json!({"name":a.name,"type":plan.models().render_type(&a.ty),"source":source(&a.source),"nullable":a.nullable})).collect::<Vec<_>>()},
                    "fields":fields.iter().map(|f|json!({"name":f.name,"wire":f.wire,"type":plan.models().render_type(&f.ty),"required":f.required,"nullable":f.nullable,"fixed":f.fixed,"omitMethod":f.omit_method,"source":source(&f.source),"description":f.description})).collect::<Vec<_>>(),
                    "extraType":extras.as_ref().map(|ty|plan.models().render_type(ty))
                }),
                JavaDeclaration::Literals { values } => json!({"kind":"literals","values":values.iter().map(|v|json!({"name":v.name,"value":v.value})).collect::<Vec<_>>()}),
                JavaDeclaration::Union { exclusive, variants } => json!({"kind":if *exclusive {"oneOf"} else {"anyOf"},"variants":variants.iter().map(|v|json!({"name":v.name,"type":plan.models().render_type(&v.ty),"source":source(&v.source),"codecRoot":plan.models().codec(&v.source).root,"constructor":v.constructor.name})).collect::<Vec<_>>()}),
            };
            json!({"name":s.name(),"nativeType":s.codec().native_type,"source":source(s.source()),"description":s.description(),"nullable":s.nullable(),"codec":{"holder":s.codec().holder,"field":s.codec().field,"root":s.codec().root},"declaration":declaration,"schema":plan.contract().source(s.source())})
        }).collect::<Vec<_>>(),
        "operations":plan.operations().iter().map(|o|json!({
            "operationId":o.operation_id,"method":o.method_name,"asyncMethod":o.async_method_name,"inputType":o.input_type,"successType":o.success_type,
            "httpMethod":o.http_method,"path":o.path,"protocol":o.wire,
            "description":o.description,"source":source(&o.source),
            "parameters":o.parameters.iter().map(|p|json!({"name":p.native_name,"type":p.native_type,"required":p.required,"schema":source(&p.schema),"source":source(&p.source),"codec":plan.models().codec(&p.schema).holder,"serialization":p.wire.serialization()})).collect::<Vec<_>>(),
            "body":o.body.as_ref().map(|b|json!({"name":b.native_name,"type":b.native_type,"required":b.required,"source":source(&b.source),"choiceType":b.choice_type,"media":b.media.iter().map(|m|json!({"name":m.name,"type":m.value.native_type(plan.models()),"mediaType":m.wire.media_type(),"codecSource":m.value.schema().map(source)})).collect::<Vec<_>>()})),
            "responses":o.responses.iter().map(|r|json!({"status":r.wire.status(),"variant":r.variant_name,"errorVariant":r.error_variant_name,"type":r.native_type,"source":source(&r.source),"typedHeaders":r.headers.as_ref().map(|h|&h.name),"choiceType":r.choice_type,"media":r.media.iter().map(|m|json!({"name":m.name,"type":m.value.native_type(plan.models()),"mediaType":m.wire.media_type(),"codecSource":m.value.schema().map(source)})).collect::<Vec<_>>()})).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
    });
    if plan.maven().group_id.is_some() {
        manifest["groupId"] = json!(group);
    }
    if let Some(policy) = plan.credential_env() {
        manifest["credentialEnv"] = serde_json::to_value(policy.semantic_descriptor())
            .expect("credential policy descriptor");
        files.insert(
            format!("{resources}/credential-env.json"),
            serde_json::to_string_pretty(policy).expect("source-bound variable names"),
        );
    }
    files.insert(
        format!("{resources}/sdk-manifest.json"),
        serde_json::to_string_pretty(&manifest).expect("native manifest"),
    );
    let examples = crate::http_examples::manifest(plan.examples());
    let coverage = super::docs::coverage(plan);
    files.insert(format!("{resources}/examples.json"), examples.clone());
    files.insert(format!("{resources}/doc-coverage.json"), coverage.clone());
    files.insert("java/examples.json".into(), examples);
    files.insert("java/doc-coverage.json".into(), coverage);
    files.insert("java/pom.xml".into(), format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 https://maven.apache.org/xsd/maven-4.0.0.xsd">
  <modelVersion>4.0.0</modelVersion>
  <groupId>{group}</groupId><artifactId>{artifact}</artifactId><version>{version}</version>
  <name>{api}</name><description>Source-selected exact OpenAPI SDK for Java {release}</description>
  <properties><maven.compiler.release>{release}</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding><project.reporting.outputEncoding>UTF-8</project.reporting.outputEncoding><project.build.outputTimestamp>2026-01-01T00:00:00Z</project.build.outputTimestamp></properties>
  <build><plugins>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.14.1</version><configuration><compilerArgs><arg>-Xlint:all</arg><arg>-Werror</arg></compilerArgs></configuration></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-jar-plugin</artifactId><version>3.4.2</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-source-plugin</artifactId><version>3.3.1</version><executions><execution><id>sources</id><goals><goal>jar-no-fork</goal></goals></execution></executions></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-javadoc-plugin</artifactId><version>3.11.3</version><configuration><doclint>all,-missing</doclint><failOnWarnings>true</failOnWarnings></configuration><executions><execution><id>javadoc</id><goals><goal>jar</goal></goals></execution></executions></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-install-plugin</artifactId><version>3.1.4</version></plugin>
  </plugins></build>
</project>
"#));
    files.insert("java/README.md".into(), super::docs::readme(plan));
    files.insert("java/docs/api.md".into(), super::docs::reference(plan));
    files.insert(
        "java/docs/examples.md".into(),
        crate::http_examples::markdown(
            plan.examples(),
            &format!("java -ea -cp target/{artifact}-{version}.jar {package}.SdkExamples"),
        ),
    );
    let quickstart = super::docs::quickstart(plan);
    if !quickstart.is_empty() {
        files.insert("java/examples/GettingStarted.java".into(), quickstart);
    }
    if files.values().map(String::len).sum::<usize>() > 256 * 1024 * 1024 {
        return Err(vec![super::plan_diag(
            plan.contract(),
            "java-artifact-resource-limit",
            "Java package exceeds the 256 MiB emitted-text ceiling",
        )]);
    }
    Ok(files
        .into_iter()
        .map(|(path, content)| OutFile { path, content })
        .collect())
}

fn source(id: &suspect_ir::contract::SourceId) -> serde_json::Value {
    json!({"document":id.document().as_str(),"pointer":id.pointer()})
}

/// `ua/v1` attribution constants compiled at generation time. The native
/// runtime resolver in `HttpRuntime.java` reads them; an absent descriptor
/// emits the disabled sentinel (empty suspect version) so the static runtime
/// compiles unchanged.
fn attribution_source(plan: &SdkPlan, package: &str) -> String {
    let (suspect, name, version, spec) = match plan.attribution() {
        Some(attribution) => (
            q(&attribution.suspect_version),
            q(&attribution.sdk_name),
            q(&attribution.sdk_version),
            q(&attribution.spec_version),
        ),
        None => (
            "\"\"".to_owned(),
            "\"\"".to_owned(),
            "\"\"".to_owned(),
            "\"\"".to_owned(),
        ),
    };
    let comment = if plan.attribution().is_some() {
        "/** ua/v1 attribution: every request identifies suspect as the generator and the\n * SDK or a caller-supplied application as the client.\n */\n"
    } else {
        "/** ua/v1 attribution is disabled for this package: an empty suspect version\n * suppresses the automatic User-Agent header.\n */\n"
    };
    format!(
        "package {package};\n\n{comment}public final class Attribution {{\n    private Attribution() {{}}\n    /** Suspect generator version captured at generation time. An empty value disables the automatic attribution header. */\n    public static final String SUSPECT_VERSION = {suspect};\n    /** Sanitized SDK package identity token. */\n    public static final String SDK_NAME = {name};\n    /** Exact SDK package version. */\n    public static final String SDK_VERSION = {version};\n    /** Source document's declared OpenAPI or Swagger version. */\n    public static final String SPEC_VERSION = {spec};\n    /** Language tag used inside the trailing User-Agent comment. */\n    public static final String LANGUAGE = \"java\";\n}}\n"
    )
}
