//! PHPDoc companion reference and executable native examples from retained descriptors.

use super::super::{
    models::{Extras, Initializer, Shape},
    samples::{Expression, NativeExample},
};
use super::{Context, comment_inline, php};
use crate::http_examples::{InputSlot, bindings};
use serde_json::{Value, json};
use std::fmt::Write;

impl Context<'_> {
    pub(super) fn readme(&self) -> String {
        let package = &self.plan.config.package_name;
        let namespace = &self.plan.config.namespace;
        let quickstart = self.quickstart_code().map(|(code, _, _)| format!("## First request\n\nThis complete call uses actual allocated names and source-validated input values. The `SDK_API_TOKEN` environment variable is application scaffolding; the library only uses credentials you pass explicitly. The same function is typechecked and executed against a local fixture in `examples/quickstart.php`.\n\n```php\n<?php\ndeclare(strict_types=1);\nrequire __DIR__ . '/vendor/autoload.php';\n\n{code}\n$token = getenv('SDK_API_TOKEN');\nif ($token === false || $token === '') {{\n    throw new RuntimeException('Set SDK_API_TOKEN in your application');\n}}\n$response = firstRequest($token);\necho $response->response->status, PHP_EOL;\n```\n\n")).unwrap_or_default();
        format!(
            "# {package}\n\nSource-selected synchronous PHP SDK. PHP 8.3+ (64-bit); the default adapter needs ext-curl with libcurl 7.85+. Composer classmap autoloading exposes `{namespace}\\Client`, `ClientInterface`, native model classes/enums and `Codecs`.\n\n## Install\n\n```sh\ncomposer require {package}:{}\n```\n\n{quickstart}[Browse the source-bound reference](docs/index.html), [validated examples and findings](docs/examples.md), and [machine-readable coverage](docs/coverage.json).\n\n## Native values\n\nUse `declare(strict_types=1)` in PHP callers and PHPStan for collection element types. Required fields are constructor parameters. Named arguments are supported. Source-required single-value string tags are initialized as readonly enum properties. Other model properties are mutable and checked again on every encode. Native unions retain each model/enum/list/scalar carrier and enforce the selected branch and parent schema. `Absent::Value` is omission; PHP `null` is explicit JSON null in nullable native fields. Optional non-nullable fields exclude null. Arbitrary JSON fields use `JsonValue::null()` for null. Open objects retain typed or `JsonValue` extras in `$extra`; declared wire keys cannot also appear there.\n\n`JsonNumber::fromString('9007199254740993.000000000000000001')` retains exact numeric tokens, including symbolic exponents. `fromInt`, `isInteger`, `compare`, `toInt` and bounded `toDecimalString` provide explicit conversions. Floats are never an exact-number input. `JsonValue` is the schema's genuine arbitrary JSON domain, with distinct object/list/boolean/number/null representations. JSON parsing rejects duplicate decoded keys, malformed UTF-8 and unpaired surrogates.\n\n## HTTP, authentication and failures\n\nThe first-request example uses the actual source security-scheme name. An HTTP bearer declaration writes `Authorization: Bearer …`; a name such as `apiKey` does not change that declaration into API-key authentication. Any management-key guidance remains source prose in the operation reference. `Transport::send(HttpRequest): HttpResponse` supports framework/PSR-18 bridges and recording fixtures. Adapters honor `HttpRequest::check()`, timeout and byte ceilings while reading, preserve repeated headers and raw content encoding, and close owned resources before returning. Returned data is checked again by the client.\n\nOperations with no required input support an omitted input argument. A single success has a concrete return type; several successes use a native union. Each wrapper has typed `$body` plus `$response` status/headers. Each operation has an API-error base and exact-status subclasses with typed `$body`. SDK failures have a stable `$kind`, operation/source identity, optional bounded `ResponseCapture`, and a separately accessible cause. Default error formatting omits payloads and causes.\n\nThe cURL adapter sends once, verifies TLS, uses explicit credentials, disables redirects, cookie storage, implicit proxies/netrc and content decompression, and closes each exchange and handle in `finally`. Content encoding must be identity. `RequestOptions` may lower client deadlines/response limits and supply a `CancellationToken`. Cancellation is cooperative in this synchronous profile; a blocking custom adapter must participate. The whole call checks deadlines before request conversion, throughout generated work, during cURL callbacks and after a custom adapter returns.\n\n## Verify a generated package checkout\n\n```sh\ncomposer install\ncomposer typecheck\ncomposer examples\n```\n\nThe package uses PHPStan max level; `treatPhpDocTypesAsCertain: false` keeps defensive runtime collection checks meaningful. No analysis baseline or ignored-error list is emitted. An independent installed consumer can execute packaged samples with `SUSPECT_SDK_AUTOLOAD=/absolute/path/to/consumer/vendor/autoload.php php vendor/{package}/examples/quickstart.php`. All packaged samples use explicit local fixture transports.\n\n## Finite policy\n\nGenerated request/body ceiling: {} bytes; response: {}; headers: {}; failure capture: {}; JSON/conversion depth: {}; node visits: {}; conversion bytes: {}. Callers may lower limits. Schema, numeric, equality and native-union trials retain shared finite budgets. Exhaustion is an explicit failure, never a successful or ordinary-invalid branch.\n\n## Admitted profile\n\nOpenAPI 3.1 / JSON Schema 2020-12 checked static subset; selected JSON operations, exact statuses, one explicit HTTP bearer scheme, one static HTTPS server, simple string paths and form scalar/list queries. Unsupported dialects, opcodes and native shapes produce source-linked planning diagnostics before artifacts. Multi-carrier intersections, tuple model carriers, active directional annotations and broader HTTP media/security/stream profiles require their own admitted native implementations. Source examples remain distinct from labeled synthesized examples; their original findings remain visible.\n",
            self.plan.config.package_version,
            self.plan.config.max_request_bytes,
            self.plan.config.max_response_bytes,
            self.plan.config.max_header_bytes,
            self.plan.config.max_capture_bytes,
            self.plan.config.max_depth,
            self.plan.config.max_nodes,
            self.plan.config.max_conversion_bytes
        )
    }

    fn quickstart_code(&self) -> Option<(String, u16, String)> {
        let bound = bindings(&self.plan.examples);
        let mut candidates: Vec<_> = self.plan.operations.iter().collect();
        candidates.sort_by_key(|op| self.arguments(op).iter().any(|(_, _, required)| *required));
        for op in candidates {
            let binding = &bound[&op.source];
            let Some((status, response)) = binding.operation.entries.iter().find_map(|entry| {
                if let crate::examples::ExampleRole::Response { status } = entry.role
                    && (200..300).contains(&status)
                {
                    return Some((status, entry));
                }
                None
            }) else {
                continue;
            };
            let slots = op
                .parameters
                .iter()
                .filter(|p| p.required)
                .map(|p| InputSlot {
                    name: p.name.as_str(),
                    required: true,
                    container: p.source.clone(),
                })
                .chain(
                    op.wire
                        .body
                        .iter()
                        .filter(|b| b.required)
                        .map(|b| InputSlot {
                            name: "body",
                            required: true,
                            container: b.media_source.clone(),
                        }),
                );
            let Some(args) = binding.bind(slots) else {
                continue;
            };
            let input = if args.is_empty() {
                String::new()
            } else {
                format!(
                    "new Sdk\\{}({}), ",
                    op.input_type,
                    args.iter()
                        .map(|a| format!(
                            "{}: {}",
                            a.name,
                            render_at(
                                &self.sample(&op.source, a.example_index).expression,
                                "Sdk\\"
                            )
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let result = op
                .success_types
                .iter()
                .map(|(_, n)| format!("Sdk\\{n}"))
                .collect::<Vec<_>>()
                .join("|");
            let mut code = format!(
                "use {} as Sdk;\n\n// Inputs are source-validated; declared/synthesized origins are in docs/examples.json.\nfunction firstRequest(\n    string $token,\n    Sdk\\Transport $transport = new Sdk\\CurlTransport(),\n): {result} {{\n    $client = new Sdk\\Client(\n        new Sdk\\Credentials([{} => $token]),\n        transport: $transport,\n    );\n    try {{\n        return $client->{}({input}options: new Sdk\\RequestOptions(timeoutMilliseconds: 3000));\n",
                self.plan.config.namespace,
                php(&op.wire.security_scheme_name),
                op.method_name
            );
            if !op.error_types.is_empty() {
                write!(code, "    }} catch (Sdk\\{} $error) {{\n        fwrite(STDERR, 'API status: ' . $error->status . PHP_EOL);\n        throw $error;\n", op.error_type).unwrap();
            }
            code.push_str("    } catch (Sdk\\SdkError $error) {\n        fwrite(STDERR, 'SDK failure: ' . $error->kind . PHP_EOL);\n        throw $error;\n    }\n}\n");
            return Some((code, status, response.value.to_string()));
        }
        None
    }

    pub(super) fn quickstart_example(&self) -> String {
        let mut out =
            "<?php\ndeclare(strict_types=1);\nrequire __DIR__ . '/autoload.php';\n\n".to_owned();
        if let Some((code, status, json)) = self.quickstart_code() {
            out.push_str(&code);
            write!(out, "\n// Executable local fixture for the exact guide function above.\n$transport = new class implements Sdk\\Transport {{\n    public function send(Sdk\\HttpRequest $request): Sdk\\HttpResponse\n    {{\n        $request->check();\n        return new Sdk\\HttpResponse({status}, ['Content-Type' => 'application/json'], {});\n    }}\n}};\n$response = firstRequest('example-token', $transport);\nif ($response->response->status !== {status}) {{ throw new RuntimeException('quickstart fixture mismatch'); }}\necho 'first-request quickstart passed', PHP_EOL;\n", php(&json)).unwrap();
        } else {
            out.push_str("echo 'No complete source-valid first-request example; see docs/examples.json findings.', PHP_EOL;\n");
        }
        out
    }

    pub(super) fn reference(&self) -> String {
        let mut out = String::from(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>PHP SDK reference</title><style>body{font:16px system-ui;max-width:1000px;margin:2rem auto;padding:1rem;line-height:1.5}code,pre{font-family:monospace;overflow-wrap:anywhere}table{border-collapse:collapse;width:100%}th,td{border:1px solid #aaa;padding:.5rem;text-align:left;vertical-align:top}section{margin-block:3rem}a{color:#1659a8}pre{white-space:pre-wrap}nav{display:flex;flex-wrap:wrap;gap:1rem}</style></head><body><h1>PHP SDK reference</h1><p>Native symbols and PHPDoc are emitted from the same source-bound plan. All codecs validate on decode and revalidate mutable models on encode.</p><nav><a href=\"../README.md\">Guide</a><a href=\"examples.md\">Examples</a><a href=\"coverage.json\">Coverage</a><a href=\"#operations\">Operations</a><a href=\"#models\">Models/codecs</a></nav><h2 id=\"operations\">Operations</h2>",
        );
        for op in &self.plan.operations {
            let method = &op.method_name;
            write!(out, "<section id=\"operation-{method}\"><h3>Client::{method}</h3><p>operationId: <code>{}</code></p><p>{}</p><p>{}</p><p><code>{} {}</code>; server <code>{}</code>; bearer scheme <code>{}</code>.</p><p>Input: <code>new {}(...)</code>.</p><table><tr><th>Member</th><th>Wire</th><th>Type</th><th>Presence</th></tr>", html(&op.operation_id), html(&op.wire.description), source_link(&op.source), html(&op.wire.method), html(&op.wire.path), html(&op.wire.server), html(&op.wire.security_scheme_name), op.input_type).unwrap();
            for p in &op.parameters {
                write!(out, "<tr><td><code>${}</code></td><td>{:?} <code>{}</code><p>{}</p></td><td><a href=\"#codec-{}\">{}</a></td><td>{}</td></tr>", p.name, p.location, html(&p.wire_name), html(&p.description), self.name(&p.schema), html(&self.field_type(&p.schema, p.required, true)), presence(p.required)).unwrap();
            }
            if let Some(b) = &op.body {
                write!(out, "<tr><td><code>$body</code></td><td>{}<p>{}</p></td><td><a href=\"#codec-{}\">{}</a></td><td>{}</td></tr>", html(&b.media_type), html(&b.description), self.name(&b.schema), html(&self.field_type(&b.schema, b.required, true)), presence(b.required)).unwrap();
            }
            write!(out, "</table><p>API-error base: <code>{}</code>. Other failures: <code>SdkError</code> (validation, transport, cancellation, timeout, limits, unexpected status/media/encoding).</p><ul>", op.error_type).unwrap();
            for r in &op.responses {
                write!(out, "<li id=\"response-{}\">{}: <code>{}</code> — status {}, {}, typed <code>$body: {}</code>. {} <a href=\"#codec-{}\">Body codec</a>.<p>{}</p></li>", r.type_name, if r.success { "Returns" } else { "Throws" }, r.type_name, r.status, html(&r.media_type), html(&self.ty(&r.schema, true)), source_link(&r.source), self.name(&r.schema), html(&r.description)).unwrap();
            }
            out.push_str("</ul></section>");
        }
        out.push_str("<h2 id=\"models\">Models and exact codecs</h2>");
        for node in self.plan.models.nodes.values() {
            write!(out, "<section id=\"codec-{}\"><h3>{}</h3><p>{}</p><p>{}</p><p>Native/PHPDoc type: <code>{}</code>.</p><p><code>Codecs::{}(string $json)</code>, <code>Codecs::{}($value): string</code>, <code>Codecs::{}(JsonValue $value)</code>, <code>Codecs::{}($value): JsonValue</code>.</p>", node.name, node.name, html(&node.description), source_link(&node.source), html(&self.ty(&node.source, true)), node.codecs.decode, node.codecs.encode, node.codecs.from_value, node.codecs.to_value).unwrap();
            if let Some(c) = &node.constructor {
                write!(out, "<p>Constructor: <code>new {}({}{})</code>. Required parameters precede optional ones. Models expose <code>fromJson</code> / <code>toJson</code>.</p>", node.name, c.parameters.iter().map(|p| format!("${p}")).collect::<Vec<_>>().join(", "), if c.extra_parameter.is_some() { if c.parameters.is_empty() { "$extra = []" } else { ", $extra = []" } } else { "" }).unwrap();
            }
            match &node.shape {
                Shape::Object { fields, extras } => {
                    out.push_str("<table><tr><th>Property</th><th>Wire name</th><th>Type/presence</th><th>Source guidance</th></tr>");
                    for f in fields {
                        let constant = if let Some(Initializer::EnumCase {
                            type_name,
                            case_name,
                            ..
                        }) = &f.initializer
                        {
                            format!("; readonly constant {type_name}::{case_name}")
                        } else {
                            String::new()
                        };
                        write!(out, "<tr><td><code>${}</code></td><td><code>{}</code></td><td><a href=\"#codec-{}\">{}</a>; {}{}</td><td>{}<br>{}</td></tr>", f.name, html(&f.wire), self.name(&f.source), html(&self.field_type(&f.source, f.required, true)), presence(f.required), html(&constant), html(&f.description), source_link(&f.source)).unwrap();
                    }
                    write!(
                        out,
                        "</table><p>Additional members: {}.</p>",
                        match extras {
                            Extras::Closed => "closed object".into(),
                            Extras::Json =>
                                "<code>array&lt;array-key, JsonValue&gt; $extra</code>".into(),
                            Extras::Patterned(_) =>
                                "<code>array&lt;array-key, JsonValue&gt; $extra</code>; every matching source pattern and the additional-member policy are revalidated".into(),
                            Extras::Typed(id) => format!(
                                "<code>array&lt;array-key, {}&gt; $extra</code>",
                                html(&self.ty(id, true))
                            ),
                        }
                    )
                    .unwrap();
                }
                Shape::Enum { cases } => {
                    out.push_str("<ul>");
                    for (case, value) in cases {
                        write!(
                            out,
                            "<li><code>{}::{case}</code> = <code>{}</code></li>",
                            node.name,
                            html(&serde_json::to_string(value).unwrap())
                        )
                        .unwrap();
                    }
                    out.push_str("</ul>");
                }
                Shape::Union(branches) => {
                    out.push_str("<p>Source branch order; branch and parent assertions remain checked:</p><ul>");
                    for id in branches {
                        write!(
                            out,
                            "<li><a href=\"#codec-{}\">{}</a></li>",
                            self.name(id),
                            html(&self.ty(id, true))
                        )
                        .unwrap();
                    }
                    out.push_str("</ul>");
                }
                _ => {}
            }
            out.push_str("</section>");
        }
        out.push_str("</body></html>\n");
        out
    }

    pub(super) fn coverage(&self) -> String {
        let identity = |s: &suspect_ir::contract::SourceId| json!({"document":s.document().to_string(),"pointer":s.pointer()});
        let value = json!({
            "version":"suspect.php-native.v1", "namespace":self.plan.config.namespace,
            "operations":self.plan.operations.iter().map(|op| json!({
                "source":identity(&op.source),"operationId":op.operation_id,"method":op.method_name,
                "inputType":op.input_type,"inputConstructor":op.input_constructor,"errorType":op.error_type,
                "reference":format!("index.html#operation-{}",op.method_name),
                "parameters":op.parameters.iter().map(|p|json!({"name":p.name,"wire":p.wire_name,"required":p.required,"schema":identity(&p.schema),"description":p.description})).collect::<Vec<_>>(),
                "body":op.body.as_ref().map(|b|json!({"name":b.name,"source":identity(&b.source),"schema":identity(&b.schema),"required":b.required,"description":b.description})),
                "responses":op.responses.iter().map(|r| json!({"source":identity(&r.source),"status":r.status,"type":r.type_name,"body":r.body_member,"metadata":r.metadata_member,"schema":identity(&r.schema),"description":r.description})).collect::<Vec<_>>()
            })).collect::<Vec<_>>(),
            "models":self.plan.models.nodes.values().map(|node| json!({
                "source":identity(&node.source),"name":node.name,"type":self.ty(&node.source,false),"phpdocType":self.ty(&node.source,true),
                "constructor":node.constructor.as_ref().map(|c|json!({"method":c.method,"parameters":c.parameters,"extras":c.extra_parameter})),
                "codecs":{"decode":node.codecs.decode,"encode":node.codecs.encode,"fromValue":node.codecs.from_value,"toValue":node.codecs.to_value},
                "reference":format!("index.html#codec-{}",node.name),
            })).collect::<Vec<_>>(),
            "validatedNativeExamples":self.plan.samples.len(),
        });
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap())
    }

    fn sample(&self, operation: &suspect_ir::contract::SourceId, index: usize) -> &NativeExample {
        self.plan
            .samples
            .iter()
            .find(|s| &s.operation == operation && s.entry_index == index)
            .expect("planned native example")
    }

    pub(super) fn codec_examples(&self) -> String {
        let mut out = self.head();
        out.push_str("require __DIR__ . '/autoload.php';\n\n// Literal source values and separately labeled synthesized values.\n");
        for op in self.plan.examples.operations() {
            for (i, entry) in op.entries.iter().enumerate() {
                writeln!(
                    out,
                    "// {} {} at {}#{}",
                    crate::http_examples::origin(&entry.origin),
                    comment_inline(&op.operation_id),
                    comment_inline(entry.schema.document().as_str()),
                    comment_inline(entry.schema.pointer())
                )
                .unwrap();
                let native = render(&self.sample(&op.source, i).expression);
                let node = &self.plan.models.nodes[&entry.schema];
                writeln!(out, "$value = {native};\n$encoded = Codecs::{}($value);\n$expected = JsonValue::parse({})->toJson();\nif ($encoded !== $expected || Codecs::{}(Codecs::{}($encoded)) !== $expected) {{\n    throw new \\RuntimeException('example round trip failed');\n}}\n", node.codecs.encode, php(&entry.value.to_string()), node.codecs.encode, node.codecs.decode).unwrap();
            }
        }
        writeln!(
            out,
            "echo '{} validated native examples passed', PHP_EOL;",
            self.plan.samples.len()
        )
        .unwrap();
        out
    }

    pub(super) fn client_examples(&self) -> String {
        let mut out = self.head();
        out.push_str("require __DIR__ . '/autoload.php';\n\n// Explicit local fixture transport: no API calls or environment credentials.\n$responses = [];\n");
        let bindings = bindings(&self.plan.examples);
        let mut calls = String::new();
        let mut count = 0;
        for op in &self.plan.operations {
            let bound = &bindings[&op.source];
            let slots = op
                .parameters
                .iter()
                .map(|p| InputSlot {
                    name: p.name.as_str(),
                    required: p.required,
                    container: p.source.clone(),
                })
                .chain(op.wire.body.iter().map(|b| InputSlot {
                    name: "body",
                    required: b.required,
                    container: b.media_source.clone(),
                }));
            let Some(args) = bound.bind(slots) else {
                continue;
            };
            let response = bound.operation.entries.iter().find_map(|e| {
                if let crate::examples::ExampleRole::Response { status } = e.role
                    && (200..300).contains(&status)
                {
                    return Some((status, e));
                }
                None
            });
            let Some((status, response)) = response else {
                continue;
            };
            writeln!(out, "$responses[] = new HttpResponse({status}, ['Content-Type' => 'application/json'], {});", php(&response.value.to_string())).unwrap();
            let arguments = args
                .iter()
                .map(|a| {
                    format!(
                        "{}: {}",
                        a.name,
                        render(&self.sample(&op.source, a.example_index).expression)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                calls,
                "// {} — {}\n$client->{}(new {}({arguments}));",
                comment_inline(&op.operation_id),
                comment_inline(&op.wire.description),
                op.method_name,
                op.input_type
            )
            .unwrap();
            count += 1;
        }
        out.push_str("$transport = new class($responses) implements Transport {\n    /** @param list<HttpResponse> $responses */\n    public function __construct(private array $responses) {}\n    public function send(HttpRequest $request): HttpResponse\n    {\n        $request->check();\n        return array_shift($this->responses) ?? throw new \\LogicException('fixture response missing');\n    }\n};\n$client = new Client(new Credentials([\n");
        let schemes: std::collections::BTreeSet<_> = self
            .plan
            .operations
            .iter()
            .map(|op| &op.wire.security_scheme_name)
            .collect();
        for scheme in schemes {
            writeln!(out, "    {} => 'example-token',", php(scheme)).unwrap();
        }
        out.push_str("]), transport: $transport);\n");
        out.push_str(&calls);
        writeln!(
            out,
            "echo '{count} mock-backed typed operations passed', PHP_EOL;"
        )
        .unwrap();
        out
    }
}

fn render(expression: &Expression) -> String {
    render_at(expression, "")
}

pub(in crate::php_sdk) fn render_at(expression: &Expression, prefix: &str) -> String {
    match expression {
        Expression::Null => "null".into(),
        Expression::Boolean(v) => v.to_string(),
        Expression::String(v) => php(v),
        Expression::Number(v) => format!("{prefix}JsonNumber::fromString({})", php(v)),
        Expression::Array(v) => format!(
            "[{}]",
            v.iter()
                .map(|v| render_at(v, prefix))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::Members(v) => format!(
            "[{}]",
            v.iter()
                .map(|(k, v)| format!("{} => {}", php(k), render_at(v, prefix)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::Construct { name, arguments } => format!(
            "new {prefix}{name}({})",
            arguments
                .iter()
                .map(|(k, v)| format!("{k}: {}", render_at(v, prefix)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::Enum { name, case } => format!("{prefix}{name}::{case}"),
        Expression::Json(value) => json_expression(value, prefix),
    }
}

fn json_expression(value: &Value, prefix: &str) -> String {
    match value {
        Value::Null => format!("{prefix}JsonValue::null()"),
        Value::Bool(v) => format!("{prefix}JsonValue::fromBool({v})"),
        Value::String(v) => format!("{prefix}JsonValue::fromString({})", php(v)),
        Value::Number(v) => format!(
            "{prefix}JsonValue::fromNumber({prefix}JsonNumber::fromString({}))",
            php(&v.to_string())
        ),
        Value::Array(v) => format!(
            "{prefix}JsonValue::fromArray([{}])",
            v.iter()
                .map(|v| json_expression(v, prefix))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(v) => format!(
            "{prefix}JsonValue::fromObject([{}])",
            v.iter()
                .map(|(k, v)| format!("{} => {}", php(k), json_expression(v, prefix)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn presence(required: bool) -> &'static str {
    if required {
        "required; absence rejected"
    } else {
        "optional; Absent::Value omits"
    }
}
fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn source_link(id: &suspect_ir::contract::SourceId) -> String {
    let source = format!("{}#{}", id.document(), id.pointer());
    if ["file:", "http:", "https:"]
        .iter()
        .any(|p| id.document().as_str().starts_with(p))
    {
        let mut url = url::Url::parse(id.document().as_str()).expect("checked document URI");
        url.set_fragment(Some(id.pointer()));
        format!("<a href=\"{}\">{}</a>", html(url.as_str()), html(&source))
    } else {
        format!("<code>{}</code>", html(&source))
    }
}
