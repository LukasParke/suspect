//! Source-bound native construction recipes over protocol example v2 slots.
use super::{
    SdkPlan,
    http::JavaOperation,
    models::{JavaModelPlan, javadoc, q},
    protocol::{AggregateRules, JavaAggregate, JavaHeaders, JavaPart, JavaValue},
};
use crate::{
    examples::{ExampleOrigin, ExamplePlan, ExampleRole},
    http_protocol as wire,
};
use serde_json::{Value, json};
use suspect_ir::contract::{SchemaId, SourceId};
use suspect_schema::OwnedSchema;

#[derive(Debug, Clone)]
pub struct JavaExample {
    pub schema: SchemaId,
    pub container: SourceId,
    pub origin: ExampleOrigin,
    pub declared_source: Option<SourceId>,
    pub role: ExampleRole,
    pub part_position: Option<crate::examples::ExamplePartPosition>,
    pub native_type: String,
    pub codec_holder: String,
    pub value_json: String,
    pub expression: Option<String>,
}
#[derive(Debug, Clone)]
pub struct JavaOperationExample {
    pub source: SourceId,
    pub operation_id: String,
    pub method_name: String,
    pub async_method_name: String,
    pub input_expression: Option<String>,
    pub entries: Vec<JavaExample>,
    /// Native byte fixtures are explicitly synthesized and checked as octets.
    pub synthesized_native_bytes: bool,
}

pub(crate) fn plan_examples(
    examples: &ExamplePlan,
    operations: &[JavaOperation],
    models: &JavaModelPlan,
    compiled: &OwnedSchema,
) -> Vec<JavaOperationExample> {
    operations
        .iter()
        .map(|op| {
            let source_examples = examples.operations().iter().find(|e| e.source == op.source);
            let entries = source_examples
                .map(|o| {
                    o.entries
                        .iter()
                        .filter(|e| models.symbol(&e.schema).is_some())
                        .map(|e| JavaExample {
                            schema: e.schema.clone(),
                            container: e.container.clone(),
                            origin: e.origin,
                            declared_source: e.declared_source.clone(),
                            role: e.role.clone(),
                            part_position: e.part_position,
                            native_type: models.native_type(&e.schema),
                            codec_holder: models.codec(&e.schema).holder.clone(),
                            value_json: e.value.to_string(),
                            expression: models.example(&e.schema, &e.value, compiled),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut required = Vec::new();
            let mut optional = Vec::new();
            let mut available = true;
            let mut native_bytes = false;
            for p in &op.parameters {
                let entry = entries.iter().find(|e| {
                    e.container == p.source
                        && !e.role.is_response()
                        && p.wire
                            .serialize(
                                &serde_json::from_str::<Value>(&e.value_json)
                                    .expect("example JSON"),
                            )
                            .is_ok()
                        && e.expression.is_some()
                });
                if let Some(entry) = entry {
                    let expr = entry.expression.as_ref().unwrap();
                    if p.required {
                        required.push(expr.clone());
                    } else {
                        optional.push(format!(".{}({expr})", p.native_name));
                    }
                } else if p.required {
                    available = false;
                }
            }
            if let Some(body) = &op.body {
                let mut body_expr = None;
                for media in &body.media {
                    let declared = source_examples
                        .into_iter()
                        .flat_map(|e| e.validated_aggregates.iter())
                        .filter(|e| {
                            !e.role.is_response()
                                && e.container == *media.wire.source().use_site().source()
                        })
                        .collect::<Vec<_>>();
                    let value = if declared.is_empty() {
                        recipe(
                            &media.value,
                            &entries,
                            &media.wire,
                            models,
                            &mut native_bytes,
                        )
                    } else if let JavaValue::Aggregate(aggregate) = &media.value {
                        declared.iter().find_map(|entry| {
                            declared_aggregate_recipe(
                                aggregate,
                                &entry.value,
                                &entries,
                                models,
                                compiled,
                            )
                        })
                    } else {
                        None
                    };
                    if let Some(value) = value {
                        body_expr = Some(if let Some(choice) = &body.choice_type {
                            let concrete = matches!(
                                media.wire.media_type().range(),
                                wire::MediaRange::Concrete { .. }
                            );
                            format!(
                                "new {choice}.{}({value}{})",
                                media.name,
                                if concrete {
                                    String::new()
                                } else {
                                    format!(", {}", q(&concrete_media(media.wire.media_type())))
                                }
                            )
                        } else {
                            value
                        });
                        break;
                    }
                }
                if let Some(value) = body_expr {
                    if body.required {
                        required.push(value);
                    } else {
                        optional.push(format!(".body({value})"));
                    }
                } else if body.required {
                    available = false;
                }
            }
            JavaOperationExample {
                source: op.source.clone(),
                operation_id: op.operation_id.clone(),
                method_name: format!("{}Example", op.method_name),
                async_method_name: format!("{}AsyncExample", op.method_name),
                input_expression: available.then(|| {
                    format!(
                        "{}.builder({}){}.build()",
                        op.input_type,
                        required.join(", "),
                        optional.join("")
                    )
                }),
                entries,
                synthesized_native_bytes: native_bytes,
            }
        })
        .collect()
}

fn concrete_media(media: &wire::MediaType) -> String {
    match media.range() {
        wire::MediaRange::Concrete { .. } => media.declared().into(),
        wire::MediaRange::Any => "application/octet-stream".into(),
        wire::MediaRange::Type { type_name } => format!("{type_name}/octet-stream"),
    }
}
fn found(entries: &[JavaExample], source: &SourceId) -> Option<String> {
    entries
        .iter()
        .find(|e| &e.container == source && !e.role.is_response())
        .and_then(|e| e.expression.clone())
}
fn bytes(max: u64, native_bytes: &mut bool) -> String {
    *native_bytes = true;
    match max {
        0 => "Bytes.empty()".into(),
        1 => "Bytes.of(new byte[] {0})".into(),
        _ => "Bytes.of(new byte[] {0, (byte)255})".into(),
    }
}
fn header_recipe(h: &JavaHeaders, entries: &[JavaExample]) -> Option<String> {
    let mut args = Vec::new();
    let mut opts = String::new();
    for header in &h.fields {
        let value = found(entries, header.wire.source().use_site().source());
        if let Some(v) = value {
            if header.wire.required() {
                args.push(v);
            } else {
                opts.push_str(&format!(".{}({v})", header.name));
            }
        } else if header.wire.required() {
            return None;
        }
    }
    Some(format!(
        "{}.builder({}){opts}.build()",
        h.name,
        args.join(", ")
    ))
}
fn part_recipe(p: &JavaPart, entries: &[JavaExample], native_bytes: &mut bool) -> Option<String> {
    part_recipe_at(p, entries, native_bytes, None)
}
fn part_recipe_at(
    p: &JavaPart,
    entries: &[JavaExample],
    native_bytes: &mut bool,
    position: Option<crate::examples::ExamplePartPosition>,
) -> Option<String> {
    let value = match p.wire.representation() {
        wire::PartRepresentation::Binary { bytes: limit } => bytes(limit.max_bytes(), native_bytes),
        _ => entries
            .iter()
            .find(|e| {
                e.container == *p.wire.source().use_site().source()
                    && e.part_position == position
                    && !e.role.is_response()
            })
            .and_then(|e| e.expression.clone())?,
    };
    let value = wrap_part(p, value, entries)?;
    if p.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
        let count = p.wire.min_items().map_or(1, |n| (*n.value()).max(1));
        if count > 8 || p.wire.max_items().is_some_and(|n| *n.value() < count) {
            return None;
        }
        if count == 1 {
            Some(format!("java.util.Collections.singletonList({value})"))
        } else {
            Some(format!(
                "java.util.Arrays.asList({})",
                vec![value; count as usize].join(", ")
            ))
        }
    } else {
        Some(value)
    }
}
fn wrap_part(p: &JavaPart, value: String, entries: &[JavaExample]) -> Option<String> {
    Some(if let Some(name) = &p.wrapper {
        let mut args = vec![value];
        let explicit = p.wire.content_types().len() > 1
            || p.wire
                .content_types()
                .first()
                .is_some_and(|m| !matches!(m.range(), wire::MediaRange::Concrete { .. }));
        if explicit {
            args.push(q(&concrete_media(p.wire.content_types().first()?)));
        }
        let mut setter = String::new();
        if let Some(h) = &p.headers {
            let headers = header_recipe(h, entries)?;
            if h.fields.iter().any(|f| f.wire.required()) {
                args.push(headers);
            } else {
                setter = format!(".headers({headers})");
            }
        }
        format!("{name}.builder({}){setter}.build()", args.join(", "))
    } else {
        value
    })
}
fn declared_part_recipe(
    p: &JavaPart,
    value: &Value,
    entries: &[JavaExample],
    models: &JavaModelPlan,
    compiled: &OwnedSchema,
) -> Option<String> {
    let item = |value: &Value| {
        if matches!(
            p.wire.representation(),
            wire::PartRepresentation::Style { .. }
        ) && (value.as_array().is_some_and(Vec::is_empty)
            || value.as_object().is_some_and(serde_json::Map::is_empty))
        {
            return None;
        }
        let expression = models.example(p.value.schema()?, value, compiled)?;
        wrap_part(p, expression, entries)
    };
    if p.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
        let values = value.as_array()?;
        if values.len() > 4096 || p.wire.required() && values.is_empty() {
            return None;
        }
        let expressions = values.iter().map(item).collect::<Option<Vec<_>>>()?;
        if expressions.is_empty() {
            Some("java.util.List.of()".into())
        } else {
            Some(format!(
                "java.util.Arrays.asList({})",
                expressions.join(", ")
            ))
        }
    } else {
        item(value)
    }
}
fn declared_aggregate_recipe(
    a: &JavaAggregate,
    value: &Value,
    entries: &[JavaExample],
    models: &JavaModelPlan,
    compiled: &OwnedSchema,
) -> Option<String> {
    let mut required = Vec::new();
    let mut optional = String::new();
    match &a.rules {
        AggregateRules::Named(rules) => {
            let values = value.as_object()?;
            for part in &a.parts {
                if let Some(value) = values.get(part.wire.name()?) {
                    let expression = declared_part_recipe(part, value, entries, models, compiled)?;
                    if part.wire.required() {
                        required.push(expression);
                    } else {
                        optional.push_str(&format!(".{}({expression})", part.name));
                    }
                } else if part.wire.required() {
                    return None;
                }
            }
            for (name, value) in values
                .iter()
                .filter(|(name, _)| a.parts.iter().all(|p| p.wire.name() != Some(name.as_str())))
            {
                let part = a.additional.as_ref()?;
                if part.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems
                    && value.as_array().is_some_and(Vec::is_empty)
                    && rules.required().iter().any(|r| r.value() == name)
                {
                    return None;
                }
                let expression = declared_part_recipe(part, value, entries, models, compiled)?;
                optional.push_str(&format!(".putAdditionalPart({}, {expression})", q(name)));
            }
        }
        AggregateRules::Positional { .. } => {
            let values = value.as_array()?;
            for (index, part) in a.parts.iter().enumerate() {
                if let Some(value) = values.get(index) {
                    let expression = declared_part_recipe(part, value, entries, models, compiled)?;
                    if part.wire.required() {
                        required.push(expression);
                    } else {
                        optional.push_str(&format!(".{}({expression})", part.name));
                    }
                } else if part.wire.required() {
                    return None;
                }
            }
            for value in values.iter().skip(a.parts.len()) {
                let expression =
                    declared_part_recipe(a.additional.as_ref()?, value, entries, models, compiled)?;
                optional.push_str(&format!(".addItem({expression})"));
            }
        }
    }
    Some(format!(
        "{}.builder({}){optional}.build()",
        a.name,
        required.join(", ")
    ))
}
fn aggregate_recipe(
    a: &JavaAggregate,
    entries: &[JavaExample],
    native_bytes: &mut bool,
) -> Option<String> {
    let mut args = Vec::new();
    let mut optional = String::new();
    match &a.rules {
        AggregateRules::Named(rules) => {
            let mut names = std::collections::BTreeSet::new();
            let maximum = rules.max_properties().map_or(u64::MAX, |n| *n.value());
            for part in a.parts.iter().filter(|p| p.wire.required()) {
                args.push(part_recipe(part, entries, native_bytes)?);
                names.insert(part.wire.name()?.to_owned());
            }
            for required in rules.required() {
                if !names.contains(required.value()) {
                    let value = part_recipe(a.additional.as_ref()?, entries, native_bytes)?;
                    optional.push_str(&format!(
                        ".putAdditionalPart({}, {value})",
                        q(required.value())
                    ));
                    names.insert(required.value().clone());
                }
            }
            if names.len() as u64 > maximum {
                return None;
            }
            for part in a.parts.iter().filter(|p| !p.wire.required()) {
                if names.len() as u64 >= maximum {
                    break;
                }
                if let Some(value) = part_recipe(part, entries, native_bytes) {
                    optional.push_str(&format!(".{}({value})", part.name));
                    names.insert(part.wire.name()?.to_owned());
                }
            }
            let minimum = rules.min_properties().map_or(0, |n| *n.value());
            if minimum.saturating_sub(names.len() as u64) > 8 {
                return None;
            }
            while (names.len() as u64) < minimum {
                let extra = a.additional.as_ref()?;
                let value = part_recipe(extra, entries, native_bytes)?;
                let mut index = 1;
                let key = loop {
                    let key = format!("example-extra-{index}");
                    if !names.contains(&key)
                        && a.parts.iter().all(|p| p.wire.name() != Some(key.as_str()))
                    {
                        break key;
                    }
                    index += 1;
                };
                optional.push_str(&format!(".putAdditionalPart({}, {value})", q(&key)));
                names.insert(key);
            }
        }
        AggregateRules::Positional {
            min_items,
            max_items,
            ..
        } => {
            let minimum = min_items.as_ref().map_or(0, |n| *n.value());
            let maximum = max_items.as_ref().map_or(u64::MAX, |n| *n.value());
            let mut count = 0;
            for (index, part) in a.parts.iter().enumerate() {
                if count >= maximum {
                    break;
                }
                let Some(value) = part_recipe_at(
                    part,
                    entries,
                    native_bytes,
                    Some(crate::examples::ExamplePartPosition::Prefix(index)),
                ) else {
                    if part.wire.required() {
                        return None;
                    }
                    break;
                };
                if part.wire.required() {
                    args.push(value);
                } else {
                    optional.push_str(&format!(".{}({value})", part.name));
                }
                count += 1;
            }
            if minimum.saturating_sub(count) > 8 {
                return None;
            }
            if count < minimum && count < a.parts.len() as u64 {
                return None;
            }
            while count < minimum {
                let value = part_recipe_at(
                    a.additional.as_ref()?,
                    entries,
                    native_bytes,
                    Some(crate::examples::ExamplePartPosition::Items),
                )?;
                optional.push_str(&format!(".addItem({value})"));
                count += 1;
            }
        }
    }
    Some(format!(
        "{}.builder({}){optional}.build()",
        a.name,
        args.join(", ")
    ))
}
fn recipe(
    value: &JavaValue,
    entries: &[JavaExample],
    media: &wire::MediaPlan,
    models: &JavaModelPlan,
    native_bytes: &mut bool,
) -> Option<String> {
    match value {
        JavaValue::Model(_) => found(entries, media.source().use_site().source()),
        JavaValue::Json => Some("new JsonObject(java.util.Map.of())".into()),
        JavaValue::Text => Some("\"example\"".into()),
        JavaValue::Bytes => {
            if let wire::Representation::Binary { bytes: limit, .. } = media.representation() {
                Some(bytes(limit.max_bytes(), native_bytes))
            } else {
                None
            }
        }
        JavaValue::Aggregate(a) => aggregate_recipe(a, entries, native_bytes),
        JavaValue::Stream {
            schema,
            request: true,
        } => entries
            .iter()
            .find(|e| e.schema == *schema && !e.role.is_response())
            .and_then(|e| e.expression.as_ref())
            .map(|v| format!("java.util.Collections.singletonList({v})")),
        _ => {
            let _ = models;
            None
        }
    }
}

pub(crate) fn examples_source(plan: &SdkPlan) -> String {
    let package = &plan.package().package;
    let api = &plan.package().api_name;
    let mut out = format!(
        "package {package};\nimport static {package}.JsonRuntime.*;\nimport static {package}.{api}.*;\n\n/** Executable native constructors from validated protocol example slots. */\npublic final class SdkExamples {{\n    private SdkExamples() {{}}\n"
    );
    let mut checks = Vec::new();
    let mut number = 0;
    for (op, native) in plan.operations().iter().zip(plan.native_examples()) {
        for entry in &native.entries {
            if let Some(expr) = &entry.expression {
                out.push_str(&format!("    private static void value{number}() {{ {} value={expr}; var json={}.CODEC.encodeValue(value);if(!json.equals(JsonRuntime.parse({})))throw new AssertionError(\"example value changed\");{}.CODEC.decodeValue(json); }}\n",entry.native_type,entry.codec_holder,q(&entry.value_json),entry.codec_holder));
                checks.push(format!("value{number}();"));
                number += 1;
            }
        }
        if let Some(input) = &native.input_expression {
            let result = if op.success_type == "Never" {
                "Never".into()
            } else {
                format!("{api}.{}", op.success_type)
            };
            out.push_str(&format!("    private static {} input{number}() {{ return {input}; }}\n    /** Source-bound native call; response owns any stream. @param client configured client @return result */\n    public static {result} {}({api} client) {{ return client.{}(input{number}()); }}\n    /** Async source-bound native call. @param client configured client @return future */\n    public static java.util.concurrent.CompletableFuture<{result}> {}({api} client) {{ return client.{}(input{number}()); }}\n",op.input_type,native.method_name,op.method_name,native.async_method_name,op.async_method_name));
            checks.push(format!("input{number}();"));
            number += 1;
        }
    }
    out.push_str(&format!("    /** Construct and check all available values offline. */\n    public static void verify() {{ {} }}\n    /** Run without network access. @param args unused */\n    public static void main(String[] args) {{ verify();System.out.println(\"Java protocol examples verified: {}\"); }}\n}}\n",checks.join(" "),checks.len()));
    out
}
fn source(id: &SourceId) -> Value {
    json!({"document":id.document().as_str(),"pointer":id.pointer()})
}
pub(crate) fn coverage(plan: &SdkPlan) -> String {
    serde_json::to_string_pretty(&json!({"format":"suspect-java-doc-coverage-v2","package":plan.package().package,
        "models":plan.models().symbols().iter().map(|s|json!({"name":s.name(),"source":source(s.source()),"href":format!("{}.html",s.name()),"codecHref":format!("{}.html#CODEC",s.name()),"members":plan.models().fields().iter().filter(|f|f.model==s.name()).map(|f|json!({"name":f.field,"href":format!("{}.html#{}()",s.name(),f.field)})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "operations":plan.operations().iter().zip(plan.native_examples()).map(|(op,e)|json!({"operationId":op.operation_id,"source":source(&op.source),"method":op.method_name,"asyncMethod":op.async_method_name,"inputHref":format!("{}.{}.html",plan.package().api_name,op.input_type),
            "inputAvailable":e.input_expression.is_some(),"synthesizedNativeBytes":e.synthesized_native_bytes,"examples":e.entries.iter().map(|v|json!({"schema":source(&v.schema),"container":source(&v.container),"origin":crate::http_examples::origin(&v.origin),"role":crate::http_examples::role(&v.role),"available":v.expression.is_some()})).collect::<Vec<_>>(),
            "responses":op.responses.iter().map(|r|json!({"status":r.wire.status_key(),"type":r.variant_name,"href":format!("{}.{}.html",plan.package().api_name,r.variant_name)})).collect::<Vec<_>>() })).collect::<Vec<_>>()
    })).expect("coverage JSON")
}
pub(crate) fn quickstart(plan: &SdkPlan) -> String {
    let chosen = plan
        .operations()
        .iter()
        .zip(plan.native_examples())
        .filter(|(o, e)| o.success_type != "Never" && e.input_expression.is_some())
        .min_by_key(|(o, _)| (o.body.is_some(), o.parameters.len()));
    let Some((op, example)) = chosen else {
        return String::new();
    };
    let package = &plan.package().package;
    let api = &plan.package().api_name;
    let input = if op.parameters.is_empty() && op.body.is_none() {
        String::new()
    } else {
        example.input_expression.clone().unwrap()
    };
    if plan.credential_env().is_some() {
        let args = if input.is_empty() {
            "options".to_owned()
        } else {
            format!("{input},options")
        };
        return format!(
            "import {package}.*;\nimport static {package}.JsonRuntime.*;\nimport static {package}.{api}.*;\n\n/** Source-bound request using the configured creation-time environment policy. */\npublic final class GettingStarted {{\n    private GettingStarted() {{}}\n    /** Run one checked request. @param client client @param options request choices */\n    public static void run({api} client,RequestOptions options) {{\n        try(var response=client.{}({args})) {{ System.out.println(response.status()); }}\n    }}\n    /** Use source servers or an explicit server argument. @param args optional server */\n    public static void main(String[] args) {{\n        var request=RequestOptions.builder();\n        if(args.length>0)request.serverUrl(java.net.URI.create(args[0]));\n        String document=System.getenv(\"OPENAPI_DOCUMENT_URL\");if(document!=null)request.documentUrl(java.net.URI.create(document));\n        try(var client={api}.fromEnv()) {{ run(client,request.build()); }}\n    }}\n}}\n",
            op.method_name
        );
    }
    let mut credentials = String::new();
    if let wire::SecurityPlan::Alternatives { alternatives, .. } = op.wire.security()
        && !alternatives.iter().any(|a| a.is_anonymous())
    {
        for requirement in alternatives[0].requirements() {
            let scheme = q(requirement.name());
            credentials.push_str(&match requirement.credential(){
            wire::CredentialHook::Bearer{..}=>format!("\n                .credential({scheme}, System.getenv(\"API_TOKEN\"))"),
            wire::CredentialHook::Basic=>format!("\n                .basic({scheme}, System.getenv(\"API_USERNAME\"), System.getenv(\"API_PASSWORD\"))"),
            wire::CredentialHook::ApiKey{..}=>format!("\n                .apiKey({scheme}, System.getenv(\"API_KEY\"))"),
            _=>format!("\n                .authorization({scheme}, context -> HttpRuntime.Authorization.of(System.getenv(\"AUTH_SCHEME\"), System.getenv(\"AUTH_CREDENTIAL\")))"),
        });
        }
    }
    format!(
        "import {package}.*;\nimport static {package}.JsonRuntime.*;\nimport static {package}.{api}.*;\n\n/** A complete source-bound native request. */\npublic final class GettingStarted {{\n    private GettingStarted() {{}}\n    /** Run with explicit configuration. @param client client */\n    public static void run({api} client) {{\n        try (var response=client.{}({input})) {{ System.out.println(response.status()); }}\n    }}\n    /** Use source servers, or an explicit URL argument. @param args optional server */\n    public static void main(String[] args) {{\n        var options=HttpRuntime.Options.builder(){credentials};\n        if(args.length>0)options.serverUrl(java.net.URI.create(args[0]));\n        String document=System.getenv(\"OPENAPI_DOCUMENT_URL\");if(document!=null)options.documentUrl(java.net.URI.create(document));\n        try(var client=new {api}(options.build())) {{ run(client); }}\n    }}\n}}\n",
        op.method_name
    )
}
pub(crate) fn readme(plan: &SdkPlan) -> String {
    let api = &plan.package().api_name;
    let package = &plan.package().package;
    let artifact = &plan.maven().artifact_id;
    let version = &plan.package().version;
    let mut text = format!(
        "# {api}: Java SDK\n\nMaven coordinates: `{}:{artifact}:{version}`. Java release: **{}**. No runtime dependencies. Source semantics use the shared versioned HTTP protocol plan.\n\n```sh\nmvn -B install\njava -ea -cp target/{artifact}-{version}.jar {package}.SdkExamples\n```\n\n",
        plan.maven_group_id(),
        plan.maven().java_release
    );
    let quick = quickstart(plan);
    if !quick.is_empty() {
        text.push_str(&format!("## First request\n\nThis is the exact compilable `examples/GettingStarted.java`:\n\n```java\n{quick}```\n\n"));
    }
    if let Some(policy) = plan.credential_env() {
        text.push_str(&include_str!("readme-credential-env.md").replace("{client}", api));
        text.push_str("\nConfigured source bindings (variable names only):\n\n| Source scheme | Runtime variable | Attachment |\n| --- | --- | --- |\n");
        for binding in policy.bindings() {
            text.push_str(&format!(
                "| `{}` | `{}` | {} |\n",
                binding.name().replace('`', "\\`").replace('|', "\\|"),
                binding.variable(),
                match binding.kind() {
                    crate::credential_env::CredentialEnvKind::Bearer => "HTTP bearer",
                    crate::credential_env::CredentialEnvKind::ApiKey =>
                        "source header/query/cookie API key",
                }
            ));
        }
        text.push('\n');
    }
    text.push_str(include_str!("readme-runtime.md"));
    text
}
pub(crate) fn reference(plan: &SdkPlan) -> String {
    let mut out = String::from(
        "# Source-bound Java operations\n\nNative Javadoc is built by `mvn package`. All source annotations, server/security choices, response headers and links are retained in `protocol-program.json`.\n\n",
    );
    for (op, example) in plan.operations().iter().zip(plan.native_examples()) {
        out.push_str(&format!(
            "## `{}`\n\n{}\n\n`{} {}` → `{}.{}` / `{}`. Input: `{}.{}`.\n\nSource: `{}#{}`.\n\n",
            op.operation_id,
            javadoc(&op.description),
            op.http_method,
            op.path,
            plan.package().api_name,
            op.method_name,
            op.async_method_name,
            plan.package().api_name,
            op.input_type,
            op.source.document(),
            op.source.pointer()
        ));
        if let Some(input) = &example.input_expression {
            out.push_str(&format!("```java\ntry (var response=client.{}({input})) {{\n    System.out.println(response.status());\n}}\n```\n\n",op.method_name));
        }
        if example.synthesized_native_bytes {
            out.push_str("Byte recipes use explicitly synthesized, bounded native octets; they are not declared API examples or filenames.\n\n");
        }
        for response in &op.responses {
            out.push_str(&format!(
                "- HTTP `{}`: `{}` data; `{}`{}; actual status decides success.\n",
                response.wire.status_key(),
                response.native_type,
                response.variant_name,
                response
                    .error_variant_name
                    .as_ref()
                    .map_or(String::new(), |e| format!(" / `{e}`"))
            ));
        }
        out.push('\n');
    }
    out
}
