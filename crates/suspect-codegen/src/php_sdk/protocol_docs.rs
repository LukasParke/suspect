//! Executable native recipes from retained source examples and explicit byte fixtures.
use super::{
    Emitter, HeaderObject, Media, Operation, Part, Payload, ResponseStatus, doc, php, wire,
};
use serde_json::Value;
use std::fmt::Write;
use suspect_ir::contract::SourceId;

struct Sample {
    expression: String,
    value: Value,
}
struct Recipe {
    native: String,
    wire: String,
    text: Option<String>,
}

fn sample(
    context: &Emitter<'_>,
    op: &Operation,
    container: &SourceId,
    response: bool,
) -> Option<Sample> {
    let examples = context
        .plan
        .core
        .examples
        .operations()
        .iter()
        .find(|e| &e.source == op.wire.source().terminal().source() || e.source == op.source)?;
    let (index, entry) =
        examples.entries.iter().enumerate().find(|(_, entry)| {
            &entry.container == container && entry.role.is_response() == response
        })?;
    let native = context
        .plan
        .core
        .samples
        .iter()
        .find(|s| s.operation == examples.source && s.entry_index == index)?;
    Some(Sample {
        expression: super::super::super::emit::docs::render_at(&native.expression, ""),
        value: entry.value.clone(),
    })
}

fn bytes(value: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(value) {
        return php(text);
    }
    let mut out = String::from("\"");
    for byte in value {
        write!(out, "\\x{byte:02x}").unwrap();
    }
    out.push('"');
    out
}
fn raw_bytes(max: u64) -> Recipe {
    let data = &b"\0\xffnative"[..usize::try_from(max.min(8)).unwrap()];
    let wire = bytes(data);
    Recipe {
        native: format!("new Bytes({wire})"),
        wire,
        text: None,
    }
}
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Bool(_) | Value::Number(_) => Some(value.to_string()),
        _ => None,
    }
}
fn native_headers(
    context: &Emitter<'_>,
    op: &Operation,
    headers: &HeaderObject,
    response: bool,
) -> Option<(String, Vec<(String, String)>)> {
    let mut args = Vec::new();
    let mut fields = Vec::new();
    for header in headers.fields.iter().filter(|h| h.wire.required()) {
        let value = sample(
            context,
            op,
            header.wire.source().use_site().source(),
            response,
        )?;
        let serialized = header.wire.serialize(&value.value).ok()?;
        args.push(format!("{}: {}", header.name, value.expression));
        fields.push((header.wire.name().into(), serialized.value().into()));
    }
    Some((format!("new {}({})", headers.name, args.join(", ")), fields))
}
fn part_recipe(
    context: &Emitter<'_>,
    op: &Operation,
    part: &Part,
    response: bool,
) -> Option<Recipe> {
    let mut recipe = match part.wire.representation() {
        wire::PartRepresentation::Binary { bytes, .. } => raw_bytes(bytes.max_bytes()),
        wire::PartRepresentation::Json { .. } => {
            let value = sample(
                context,
                op,
                part.wire.source().use_site().source(),
                response,
            )?;
            let text = value.value.to_string();
            Recipe {
                native: value.expression,
                wire: php(&text),
                text: Some(text),
            }
        }
        _ => {
            let value = sample(
                context,
                op,
                part.wire.source().use_site().source(),
                response,
            )?;
            let text = scalar(&value.value)?;
            Recipe {
                native: value.expression,
                wire: php(&text),
                text: Some(text),
            }
        }
    };
    if let Some(wrapper) = &part.wrapper {
        let (headers, _) = native_headers(context, op, &part.headers, response)?;
        let content_type = part
            .wire
            .content_types()
            .first()
            .filter(|m| !matches!(m.range(), wire::MediaRange::Concrete { .. }))
            .map(|media| format!(", contentType: {}", php(&concrete(media))))
            .unwrap_or_default();
        recipe.native = format!(
            "new {wrapper}(value: {}, headers: {headers}{content_type})",
            recipe.native
        );
    }
    Some(recipe)
}
fn quote_name(name: &str) -> Option<String> {
    if name.chars().any(char::is_control) {
        return None;
    }
    Some(format!(
        "\"{}\"",
        name.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}
fn body_recipe(
    context: &Emitter<'_>,
    op: &Operation,
    name: &str,
    response: bool,
) -> Option<Recipe> {
    let body = context
        .plan
        .surface
        .objects
        .iter()
        .find(|b| b.name == name)?;
    let min = body.rules.min_properties().map_or(0, |v| *v.value());
    let max = body
        .rules
        .max_properties()
        .map_or(64, |v| *v.value())
        .min(64);
    let mut chosen = body
        .parts
        .iter()
        .filter(|p| p.wire.required())
        .map(|p| (p.wire.name().unwrap().to_owned(), p, false))
        .collect::<Vec<_>>();
    for required in body.rules.required() {
        if !chosen.iter().any(|(name, _, _)| name == required.value()) {
            chosen.push((required.value().clone(), body.extra.as_ref()?, true));
        }
    }
    for part in body.parts.iter().filter(|p| !p.wire.required()) {
        if chosen.len() as u64 >= min {
            break;
        }
        chosen.push((part.wire.name().unwrap().to_owned(), part, false));
    }
    while (chosen.len() as u64) < min && (chosen.len() as u64) < max {
        let mut key = format!("fixtureExtra{}", chosen.len());
        while chosen.iter().any(|(name, _, _)| name == &key) {
            key.push('_');
        }
        chosen.push((key, body.extra.as_ref()?, true));
    }
    if (chosen.len() as u64) < min || chosen.len() as u64 > max {
        return None;
    }
    let mut args = Vec::new();
    let mut extra = Vec::new();
    let mut wire_parts = Vec::new();
    let mut form = Vec::new();
    for (wire_name, part, is_extra) in chosen {
        let value = part_recipe(context, op, part, response)?;
        let repeated = part.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems;
        let count = if repeated {
            part.wire.min_items().map_or(1, |v| *v.value()).max(1)
        } else {
            1
        };
        if count > 64 || part.wire.max_items().is_some_and(|v| count > *v.value()) {
            return None;
        }
        let native = if repeated {
            format!("[{}]", vec![value.native; count as usize].join(", "))
        } else {
            value.native
        };
        if is_extra {
            extra.push(format!("{} => {native}", php(&wire_name)));
        } else {
            args.push(format!("{}: {native}", part.name));
        }
        for _ in 0..count {
            if body.multipart {
                let mut head = format!(
                    "--native-example-boundary\r\nContent-Disposition: form-data; name={}\r\n",
                    quote_name(&wire_name)?
                );
                if let Some(media) = part.wire.content_types().first() {
                    head.push_str(&format!("Content-Type: {}\r\n", concrete(media)));
                }
                let (_, headers) = native_headers(context, op, &part.headers, response)?;
                for (name, value) in headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                let data = if matches!(
                    part.wire.representation(),
                    wire::PartRepresentation::Style { .. }
                ) {
                    format!("{} . {}", php(&(wire_name.clone() + "=")), value.wire)
                } else {
                    value.wire.clone()
                };
                wire_parts.push(format!("{} . {data} . {}", php(&head), php("\r\n")));
            } else {
                let text = value.text.as_ref()?;
                form.push(
                    url::form_urlencoded::Serializer::new(String::new())
                        .append_pair(&wire_name, text)
                        .finish(),
                );
            }
        }
    }
    if !extra.is_empty() {
        args.push(format!("extra: [{}]", extra.join(", ")));
    }
    let wire = if body.multipart {
        wire_parts.push(php("--native-example-boundary--\r\n"));
        wire_parts.join(" . ")
    } else {
        php(&form.join("&"))
    };
    Some(Recipe {
        native: format!("new {}({})", body.name, args.join(", ")),
        wire,
        text: None,
    })
}
fn concrete(media: &wire::MediaType) -> String {
    let mut value = match media.range() {
        wire::MediaRange::Concrete { .. } => return media.declared().into(),
        wire::MediaRange::Type { type_name } => format!("{type_name}/x-native-fixture"),
        wire::MediaRange::Any => "x-native-fixture/x-native-fixture".into(),
    };
    for (name, parameter) in media.parameters() {
        write!(
            value,
            "; {name}=\"{}\"",
            parameter.replace('\\', "\\\\").replace('"', "\\\"")
        )
        .unwrap();
    }
    value
}
fn payload(context: &Emitter<'_>, op: &Operation, media: &Media, response: bool) -> Option<Recipe> {
    Some(match &media.payload {
        Payload::Schema(_) => {
            let value = sample(context, op, &media.source, response)?;
            let text = if matches!(
                media.wire.representation(),
                wire::Representation::Text { .. }
            ) {
                scalar(&value.value)?
            } else {
                value.value.to_string()
            };
            Recipe {
                native: value.expression,
                wire: php(&text),
                text: Some(text),
            }
        }
        Payload::Json => Recipe {
            native: "JsonValue::fromObject([])".into(),
            wire: php("{}"),
            text: Some("{}".into()),
        },
        Payload::Text => Recipe {
            native: php("native text fixture"),
            wire: php("native text fixture"),
            text: Some("native text fixture".into()),
        },
        Payload::Bytes => {
            let wire::Representation::Binary { bytes, .. } = media.wire.representation() else {
                unreachable!()
            };
            raw_bytes(bytes.max_bytes())
        }
        Payload::Object(name) => body_recipe(context, op, name, response)?,
        Payload::Stream(_) => {
            let value = sample(context, op, &media.source, response)?;
            let wire::Representation::Stream { stream } = media.wire.representation() else {
                unreachable!()
            };
            let wire = if stream.framing() == wire::StreamFraming::JsonLines {
                php(&(value.value.to_string() + "\n"))
            } else {
                let object = value.value.as_object()?;
                let data = object.get("data")?.as_str()?;
                if data.contains('\r') {
                    return None;
                }
                let mut parts = Vec::new();
                for key in ["event", "id"] {
                    if let Some(value) = object.get(key) {
                        let text = value.as_str()?;
                        if text.contains(['\r', '\n', '\0']) {
                            return None;
                        }
                        parts.push(php(&format!("{key}: {text}\n")));
                    }
                }
                if let Some(retry) = object.get("retry") {
                    parts.push(format!(
                        "{} . JsonNumber::fromString({})->toDecimalString(maxBytes: 65536) . {}",
                        php("retry: "),
                        php(&retry.to_string()),
                        php("\n")
                    ));
                }
                let mut text = String::new();
                for line in data.split('\n') {
                    writeln!(text, "data: {line}").unwrap();
                }
                text.push('\n');
                parts.push(php(&text));
                parts.join(" . ")
            };
            Recipe {
                native: format!("[{}]", value.expression),
                wire,
                text: None,
            }
        }
        Payload::NoBody => unreachable!("NoBody has no media"),
    })
}
fn status(op: &Operation, response: &super::super::Response) -> Option<u16> {
    (200..300).find(|status| {
        let forbidden = op.wire.method() == wire::Method::Head || matches!(status, 204 | 205);
        if forbidden != matches!(response.payload, Payload::NoBody) {
            return false;
        }
        let selected = op
            .wire
            .responses()
            .iter()
            .find(|r| r.status() == ResponseStatus::Exact(*status))
            .or_else(|| {
                op.wire
                    .responses()
                    .iter()
                    .find(|r| r.status() == ResponseStatus::Range(2))
            })
            .or_else(|| {
                op.wire
                    .responses()
                    .iter()
                    .find(|r| r.status() == ResponseStatus::Default)
            });
        selected.is_some_and(|r| r.status_key() == response.status_key)
    })
}

fn call(context: &Emitter<'_>, op: &Operation) -> Option<String> {
    let mut args = Vec::new();
    for parameter in op.parameters.iter().filter(|p| p.wire.required()) {
        let value = sample(
            context,
            op,
            parameter.wire.source().use_site().source(),
            false,
        )?;
        parameter.wire.serialize(&value.value).ok()?;
        args.push(format!("{}: {}", parameter.name, value.expression));
    }
    if op.body_required {
        let (media, value) = op
            .body
            .iter()
            .find_map(|media| payload(context, op, media, false).map(|recipe| (media, recipe)))?;
        let value = media.wrapper.as_ref().map_or(value.native.clone(), |name| {
            format!(
                "new {name}({}, {})",
                value.native,
                php(&concrete(media.wire.media_type()))
            )
        });
        args.push(format!("body: {value}"));
    }
    let (response, status, wire) =
        op.responses
            .iter()
            .filter(|r| r.success)
            .find_map(|response| {
                let status = status(op, response)?;
                let wire = match (&response.payload, &response.media) {
                    (Payload::NoBody, _) => php(""),
                    (Payload::Bytes, None) => raw_bytes(response.wire.max_body_bytes()).wire,
                    (_, Some(media)) => payload(context, op, media, true)?.wire,
                    _ => return None,
                };
                Some((response, status, wire))
            })?;
    let (_, mut headers) = native_headers(context, op, &response.headers, true)?;
    if let Some(media) = &response.media {
        let mut content_type = concrete(media.wire.media_type());
        if matches!(
            media.wire.representation(),
            wire::Representation::Multipart { .. }
        ) {
            content_type.push_str("; boundary=native-example-boundary");
        }
        headers.push(("Content-Type".into(), content_type));
    }
    let headers = format!(
        "[{}]",
        headers
            .iter()
            .map(|(name, value)| format!("{} => {}", php(name), php(value)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut out = String::new();
    doc(
        &mut out,
        &format!(
            "{}: source-validated model values; raw bytes and credentials are explicit application fixtures.",
            op.id
        ),
    );
    if op
        .responses
        .iter()
        .any(|r| matches!(r.payload, Payload::Stream(_)))
    {
        writeln!(out,"$transport = new class implements StreamTransport {{\n    public function send(HttpRequest $request): HttpResponse {{ throw new \\RuntimeException('Use the stream seam'); }}\n    public function open(HttpRequest $request): StreamResponse {{\n        $reader = new class({wire}) implements BodyReader {{\n            private bool $closed = false;\n            public function __construct(private ?string $bytes) {{}}\n            public function read(): ?string {{ if ($this->closed) {{ return null; }} $bytes = $this->bytes; $this->bytes = null; return $bytes === '' ? null : $bytes; }}\n            public function close(): void {{ $this->closed = true; $this->bytes = null; }}\n        }};\n        return new StreamResponse({status}, {headers}, $reader);\n    }}\n}};").unwrap();
    } else {
        writeln!(out,"$transport = new class implements Transport {{\n    public function send(HttpRequest $request): HttpResponse {{ $request->check(); return new HttpResponse({status}, {headers}, {wire}); }}\n}};").unwrap();
    }
    out.push_str("$credentials = new Credentials([\n");
    let mut seen = std::collections::BTreeSet::new();
    for alternative in op.wire.security().alternatives() {
        for requirement in alternative.requirements() {
            if !seen.insert(requirement.name()) {
                continue;
            }
            let credential = match requirement.credential() {
                wire::CredentialHook::Bearer { .. } => "'fixture-token'",
                wire::CredentialHook::Basic => {
                    "new BasicCredential('fixture-user', 'fixture-password')"
                }
                wire::CredentialHook::ApiKey { .. } => "new ApiKeyCredential('fixture-key')",
                _ => {
                    "static fn (CredentialRequest $request): AuthorizationCredential => new AuthorizationCredential('Bearer fixture-token')"
                }
            };
            writeln!(out, "    {} => {credential},", php(requirement.name())).unwrap();
        }
    }
    writeln!(out,"]);\n$client = new Client($credentials, $transport, new ClientOptions(serverUrl: 'https://fixture.example.test'));\n$response = $client->{}(new {}({}), new RequestOptions(timeoutMilliseconds: 3000));",op.method,op.input,args.join(", ")).unwrap();
    if op.responses.iter().filter(|r| r.success).count() > 1 {
        writeln!(out,"if (!$response instanceof {}) {{ throw new \\RuntimeException('fixture response variant mismatch'); }}",response.name).unwrap();
    }
    if matches!(response.payload, Payload::Stream(_)) {
        out.push_str("try {\n    foreach ($response->body as $item) {\n        // Each item is the source's native item type. SSE data remains a string.\n        break;\n    }\n} finally { $response->body->close(); }\n");
    }
    writeln!(
        out,
        "echo {}, $response->response->status, PHP_EOL;",
        php(&format!("first request passed for {}: ", op.id))
    )
    .unwrap();
    Some(out)
}

pub(super) fn quickstart(context: &Emitter<'_>, all: bool) -> String {
    let mut out = context.head();
    out.push_str(
        "require getenv('SUSPECT_SDK_AUTOLOAD') ?: dirname(__DIR__) . '/vendor/autoload.php';\n\n",
    );
    let mut count = 0;
    for operation in context.plan.operations() {
        if let Some(recipe) = call(context, operation) {
            out.push_str(&recipe);
            count += 1;
            if !all {
                break;
            }
        } else if all {
            doc(
                &mut out,
                &format!(
                    "{} needs an explicit application fixture for this source-selected wire shape. See docs/examples.json.",
                    operation.id
                ),
            );
        }
    }
    if count == 0 {
        out.push_str("echo 'A source-compatible application wire fixture is required; see docs/examples.json.', PHP_EOL;\n");
    }
    out
}

pub(super) fn availability(context: &Emitter<'_>) -> Vec<Value> {
    context.plan.operations().iter().map(|op|serde_json::json!({"operationId":op.id,"completeCall":call(context,op).is_some(),"sourceValues":"validated native expressions","bytes":"explicit bounded in-memory fixtures"})).collect()
}
