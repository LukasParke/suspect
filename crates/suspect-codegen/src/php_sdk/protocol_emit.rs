//! PHP classes and protocol methods rendered from allocated native descriptors.
use super::{BodyObject, HeaderObject, Media, Operation, Part, Payload, SdkPlan};
use crate::{
    OutFile,
    http_protocol::{self as wire, ResponseStatus},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fmt::Write};
use suspect_ir::contract::SchemaId;

#[path = "protocol_docs.rs"]
mod docs;

pub(super) fn package(plan: &SdkPlan) -> Vec<OutFile> {
    let mut files = super::super::emit::model_assets(&plan.core);
    let mut put = |path: &str, content: String| {
        files.push(OutFile {
            path: format!("php/{path}"),
            content,
        })
    };
    let context = Emitter { plan };
    put("src/ProtocolModels.php", context.models());
    put("src/ProtocolData.php", context.data());
    put("src/Client.php", context.client());
    // Compiled OAuth lifecycle exists only under configured SDK defaults with
    // at least one usable scheme; without them this contributes nothing at all.
    if let Some(oauth) = super::oauth::source(plan) {
        put("src/OAuth.php", oauth);
    }
    // Compiled incoming receipt helpers exist only for declared
    // webhooks/callbacks; receipt-less packages contribute nothing at all.
    if super::incoming::emittable(&plan.incoming) {
        put(
            "src/Incoming.php",
            super::incoming::source(plan, &plan.incoming_receipts),
        );
    }
    put("composer.json",format!("{}\n",serde_json::to_string_pretty(&json!({"name":plan.config().package_name,"version":plan.config().package_version,
        "description":"Source-selected typed PHP SDK with checked exact codecs","type":"library","license":"proprietary",
        "require":{"php":"^8.3","ext-json":"*"},"suggest":{"ext-curl":"Default bounded HTTP adapter"},"autoload":{"classmap":["src/"]},
        "require-dev":{"phpstan/phpstan":"2.2.13"},"config":{"allow-plugins":false},"archive":{"exclude":["/vendor","/build","/composer.lock"]},"scripts":{"typecheck":"phpstan analyse --no-progress"}})).unwrap()));
    put("phpstan.neon","parameters:\n    level: max\n    phpVersion: 80300\n    treatPhpDocTypesAsCertain: false\n    paths: [src, examples]\n    tmpDir: build/phpstan\n".into());
    put(
        "docs/protocol.json",
        format!(
            "{}\n",
            serde_json::to_string_pretty(&plan.surface.protocol).unwrap()
        ),
    );
    put(
        "docs/examples.json",
        crate::http_examples::manifest(&plan.core.examples),
    );
    put("README.md", context.readme());
    put("docs/index.html", context.reference());
    put("docs/coverage.json", context.coverage());
    put("examples/codecs.php", context.examples());
    put("examples/client.php", docs::quickstart(&context, true));
    put("examples/quickstart.php", context.quickstart());
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}
pub(super) struct Emitter<'a> {
    plan: &'a SdkPlan,
}
impl Emitter<'_> {
    /// The emitter over one admitted plan, shared with the typed-events
    /// emission so both paths render byte-identical request preparation.
    pub(super) fn new(plan: &SdkPlan) -> Emitter<'_> {
        Emitter { plan }
    }
    /// The request preparation shared by the direct operation method and the
    /// typed-events generator: call context, protocol metadata and encoded
    /// native values.
    pub(super) fn request_preparation(&self, op: &Operation) -> String {
        let mut out = String::new();
        out.push_str(
            "        $call = new CallContext($this->options, $options);\n        $context = new CodecContext($call->control);\n",
        );
        writeln!(
            out,
            "        $metadata = ProtocolData::{}();\n        $values = []; $payload = null;",
            op.method
        )
        .unwrap();
        if !self.arguments(op, false).is_empty() {
            out.push_str("        try {\n");
        }
        for parameter in &op.parameters {
            if !parameter.wire.required() {
                writeln!(
                    out,
                    "        if ($input->{} !== Absent::Value) {{",
                    parameter.name
                )
                .unwrap();
            }
            writeln!(
                out,
                "            $values[{}] = Codecs::{}($input->{}, $context);",
                php(&format!(
                    "{}:{}",
                    location(parameter.wire.location()),
                    parameter.wire.name()
                )),
                self.plan.models().nodes[&parameter.schema].codecs.to_value,
                parameter.name
            )
            .unwrap();
            if !parameter.wire.required() {
                out.push_str("        }\n");
            }
        }
        if !op.body.is_empty() {
            let max_bytes = format!(
                "min($this->options->maxRequestBytes, {})",
                op.wire.body().unwrap().limits().body()
            );
            if !op.body_required {
                out.push_str("        if ($input->body !== Absent::Value) {\n");
            }
            if op.body.iter().all(|m| m.wrapper.is_some()) {
                writeln!(
                    out,
                    "            $payload = $input->body->toPayload($context, {max_bytes});"
                )
                .unwrap();
            } else {
                let media = &op.body[0];
                writeln!(
                    out,
                    "            $payload = {};",
                    self.payload(
                        media,
                        0,
                        "$input->body",
                        &php(media.wire.media_type().declared()),
                        &max_bytes
                    )
                )
                .unwrap();
            }
            if !op.body_required {
                out.push_str("        }\n");
            }
        }
        if !self.arguments(op, false).is_empty() {
            out.push_str("        } catch (JsonError|ValidationError|\\Error $error) { throw new SdkError('request_validation', 'invalid native request', previous: $error); }\n");
        }
        out
    }
    fn head(&self) -> String {
        format!(
            "<?php\ndeclare(strict_types=1);\nnamespace {};\n\n",
            self.plan.config().namespace
        )
    }
    fn ty(&self, payload: &Payload, doc: bool) -> String {
        match payload {
            Payload::Schema(id) => self.plan.models().type_name(id, doc),
            Payload::Json => "JsonValue".into(),
            Payload::Text => "string".into(),
            Payload::Bytes => "Bytes".into(),
            Payload::NoBody => "NoBody".into(),
            Payload::Object(name) => name.clone(),
            Payload::Stream(id) => {
                if doc {
                    format!("ItemStream<{}>", self.plan.models().type_name(id, true))
                } else {
                    "ItemStream".into()
                }
            }
        }
    }
    fn encode(&self, payload: &Payload, value: &str) -> String {
        match payload {
            Payload::Schema(id) => format!(
                "Codecs::{}({value}, $context)",
                self.plan.models().nodes[id].codecs.to_value
            ),
            Payload::Text => format!("JsonValue::fromString({value})"),
            Payload::Json => format!("$context->json({value})"),
            Payload::Bytes => value.into(),
            _ => format!("{value}->toPayload($context)"),
        }
    }
    fn payload(
        &self,
        media: &Media,
        index: usize,
        value: &str,
        content_type: &str,
        max_bytes: &str,
    ) -> String {
        if let Payload::Stream(id) = &media.payload {
            let wire::Representation::Stream { stream } = media.wire.representation() else {
                unreachable!()
            };
            let framing = if stream.framing() == wire::StreamFraming::JsonLines {
                "json-lines"
            } else {
                "server-sent-events"
            };
            return format!(
                "new PayloadValue({}, {content_type}, Protocol::encodeItems({value}, static fn ({} $item): JsonValue => Codecs::{}($item, $context), {}, {}, $context, {max_bytes}))",
                php(&index.to_string()),
                self.plan.models().type_name(id, false),
                self.plan.models().nodes[id].codecs.to_value,
                php(framing),
                stream.max_item_bytes()
            );
        }
        if matches!(media.payload, Payload::Object(_)) {
            format!(
                "new PayloadValue({}, {content_type}, [], {value}->toParts($context))",
                php(&index.to_string())
            )
        } else {
            format!(
                "new PayloadValue({}, {content_type}, {})",
                php(&index.to_string()),
                self.encode(&media.payload, value)
            )
        }
    }
    fn decode(&self, payload: &Payload, value: &str) -> String {
        match payload {
            Payload::Schema(id) => format!(
                "Codecs::{}({value}, $context)",
                self.plan.models().nodes[id].codecs.from_value
            ),
            Payload::Json => value.into(),
            Payload::Text => format!("{value}->asString()"),
            Payload::Bytes => format!("new Bytes({value})"),
            Payload::NoBody => "NoBody::Value".into(),
            _ => unreachable!("native complex payload emission"),
        }
    }
    fn input_type(&self, op: &Operation, doc: bool) -> String {
        super::models::union(
            op.body
                .iter()
                .map(|m| {
                    m.wrapper
                        .clone()
                        .unwrap_or_else(|| self.request_ty(&m.payload, doc))
                })
                .collect(),
        )
    }
    fn request_ty(&self, payload: &Payload, doc: bool) -> String {
        if let Payload::Stream(id) = payload {
            if doc {
                format!("iterable<{}>", self.plan.models().type_name(id, true))
            } else {
                "iterable".into()
            }
        } else {
            self.ty(payload, doc)
        }
    }
    fn arguments(&self, op: &Operation, doc: bool) -> Vec<(String, String, bool)> {
        let mut args = op
            .parameters
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    self.plan.models().type_name(&p.schema, doc),
                    p.wire.required(),
                )
            })
            .collect::<Vec<_>>();
        if !op.body.is_empty() {
            args.push(("body".into(), self.input_type(op, doc), op.body_required));
        }
        args.sort_by_key(|(_, _, required)| !*required);
        args
    }
    fn data(&self) -> String {
        let mut out = self.head();
        out.push_str(
            "/** Immutable admitted protocol metadata. @internal */\nfinal class ProtocolData {\n",
        );
        for op in self.plan.operations() {
            writeln!(out,"    public static function {}(): JsonValue {{\n        /** @var JsonValue|null $value */\n        static $value = null;\n        return $value ??= JsonValue::parse({});\n    }}",op.method,php(&serde_json::to_string(&op.wire).unwrap())).unwrap();
        }
        out.push_str("}\n");
        out
    }
    fn models(&self) -> String {
        let mut out = self.head();
        let mut emitted = BTreeSet::new();
        for object in &self.plan.surface.objects {
            for part in object.parts.iter().chain(object.extra.iter()) {
                if part.wrapper.is_some() {
                    if emitted.insert(part.headers.name.clone()) {
                        out.push_str(&self.headers(&part.headers));
                    }
                    out.push_str(&self.part_class(part));
                }
            }
            out.push_str(&self.body_class(object));
        }
        for op in self.plan.operations() {
            doc(
                &mut out,
                &format!(
                    "{}\nSource: {}#{}",
                    op.id,
                    op.source.document(),
                    op.source.pointer()
                ),
            );
            writeln!(out, "final class {} {{", op.input).unwrap();
            out.push_str("    /**\n");
            for (name, ty, required) in self.arguments(op, true) {
                writeln!(out, "     * @param {} ${name}", optional(ty, required)).unwrap();
            }
            out.push_str("     */\n    public function __construct(\n");
            for (name, ty, required) in self.arguments(op, false) {
                writeln!(
                    out,
                    "        public {} ${name}{},",
                    optional(ty, required),
                    if required { "" } else { " = Absent::Value" }
                )
                .unwrap();
            }
            out.push_str("    ) {}\n}\n");
            let metadata_type = error_metadata_type(op);
            doc_tags(
                &mut out,
                &format!("Declared API errors for {}.", op.id),
                &[format!("@property-read {metadata_type} $response")],
            );
            writeln!(out,"abstract class {} extends ApiError {{\n    public function __construct({metadata_type} $response) {{ parent::__construct({}, {}, $response); }}\n}}",op.error,php(&op.id),php(&format!("{}#{}",op.source.document(),op.source.pointer()))).unwrap();
            for (index, media) in op.body.iter().enumerate() {
                if let Some(wrapper) = &media.wrapper {
                    doc(
                        &mut out,
                        &format!(
                            "Request media selector: {}",
                            media.wire.media_type().declared()
                        ),
                    );
                    writeln!(out,"final readonly class {wrapper} {{\n    /** @param {} $value */\n    public function __construct(public {} $value, public string $contentType{}) {{}}\n    public function toPayload(CodecContext $context, int $maxBytes = RuntimeConfig::MAX_REQUEST_BYTES): PayloadValue {{ return {}; }}\n}}",self.request_ty(&media.payload,true),self.request_ty(&media.payload,false),if matches!(media.wire.media_type().range(),wire::MediaRange::Concrete{..}){format!(" = {}",php(media.wire.media_type().declared()))}else{String::new()},self.payload(media,index,"$this->value","$this->contentType","$maxBytes")).unwrap();
                }
            }
            for response in &op.responses {
                if emitted.insert(response.headers.name.clone()) {
                    out.push_str(&self.headers(&response.headers));
                }
                doc_tags(
                    &mut out,
                    &format!(
                        "{} response {}: {}\nSource: {}#{}",
                        op.id,
                        response.status_key,
                        response.wire.description().value(),
                        response.source.document(),
                        response.source.pointer()
                    ),
                    &if response.success {
                        Vec::new()
                    } else {
                        vec![format!(
                            "@property-read {} $response",
                            if matches!(response.payload, Payload::Stream(_)) {
                                "StreamResponse"
                            } else {
                                "HttpResponse"
                            }
                        )]
                    },
                );
                let parent = if response.success {
                    String::new()
                } else {
                    format!(" extends {}", op.error)
                };
                writeln!(
                    out,
                    "final {}class {}{parent} {{",
                    if response.success { "readonly " } else { "" },
                    response.name
                )
                .unwrap();
                writeln!(
                    out,
                    "    public const STATUS_PATTERN = {};",
                    php(&response.status_key)
                )
                .unwrap();
                if let ResponseStatus::Exact(status) = response.wire.status() {
                    writeln!(out, "    public const STATUS = {status};").unwrap();
                }
                let metadata_type = if matches!(response.payload, Payload::Stream(_)) {
                    "StreamResponse"
                } else {
                    "HttpResponse"
                };
                writeln!(out,"    /** @param {} $body\n     * @param list<Link> $links\n     */\n    public function __construct(public {}{} $body, {}{metadata_type} $response, public {}{} $headers, public {}array $links = []) {{ {} }}\n}}",self.ty(&response.payload,true),if response.success{""}else{"readonly "},self.ty(&response.payload,false),if response.success{"public "}else{""},if response.success{""}else{"readonly "},response.headers.name,if response.success{""}else{"readonly "},if response.success{""}else{"parent::__construct($response);"}).unwrap();
            }
        }
        out
    }
    fn headers(&self, headers: &HeaderObject) -> String {
        let mut out = format!(
            "/** Typed source HTTP/part header values. Requiredness is checked at the codec boundary. */\nfinal class {} {{\n    /**\n",
            headers.name
        );
        let mut fields = headers.fields.iter().collect::<Vec<_>>();
        fields.sort_by_key(|h| !h.wire.required());
        for field in &fields {
            writeln!(
                out,
                "     * @param {} ${}",
                optional(
                    self.plan.models().type_name(&field.schema, true),
                    field.wire.required()
                ),
                field.name
            )
            .unwrap();
        }
        out.push_str("     */\n    public function __construct(\n");
        for field in &fields {
            writeln!(
                out,
                "        public {} ${}{},",
                optional(
                    self.plan.models().type_name(&field.schema, false),
                    field.wire.required()
                ),
                field.name,
                if field.wire.required() {
                    ""
                } else {
                    " = Absent::Value"
                }
            )
            .unwrap();
        }
        out.push_str("    ) {}\n    public static function fromResponse(HttpResponse|StreamResponse $response, CodecContext $context): self {\n");
        for (i, field) in fields.iter().enumerate() {
            writeln!(
                out,
                "        $values{i} = $response->headerValues({});",
                php(field.wire.name())
            )
            .unwrap();
            if field.wire.required() {
                writeln!(out,"        if ($values{i} === []) {{ throw new JsonError('conversion', 'required response header absent'); }}").unwrap();
            }
            writeln!(out,"        $field{i} = {}Codecs::{}(Protocol::header(JsonValue::parse({}), $values{i}, {}), $context);",if field.wire.required(){String::new()}else{format!("$values{i} === [] ? Absent::Value : ")},self.plan.models().nodes[&field.schema].codecs.from_value,php(&serde_json::to_string(&field.wire).unwrap()),php(scalar(self.plan,&field.schema))).unwrap();
        }
        writeln!(
            out,
            "        return new self({});\n    }}",
            fields
                .iter()
                .enumerate()
                .map(|(i, f)| format!("{}: $field{i}", f.name))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        out.push_str("    /** @return array<array-key,string> */\n    public function toHeaders(CodecContext $context): array {\n        $headers = [];\n");
        for field in &headers.fields {
            if !field.wire.required() {
                writeln!(
                    out,
                    "        if ($this->{} !== Absent::Value) {{",
                    field.name
                )
                .unwrap();
            }
            writeln!(out,"            $value = Codecs::{}($this->{}, $context);\n            $headers[{}] = Protocol::parameter(JsonValue::parse({}), $value, 'header', {});",self.plan.models().nodes[&field.schema].codecs.to_value,field.name,php(field.wire.name()),php(&serde_json::to_string(&field.wire).unwrap()),php(field.wire.name())).unwrap();
            if !field.wire.required() {
                out.push_str("        }\n");
            }
        }
        out.push_str("        return $headers;\n    }\n}\n");
        out
    }
    fn part_ty(&self, part: &Part, doc: bool) -> String {
        let ty = part
            .wrapper
            .clone()
            .unwrap_or_else(|| self.ty(&part.payload, doc));
        if part.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
            if doc {
                format!("list<{ty}>")
            } else {
                "array".into()
            }
        } else {
            ty
        }
    }
    fn part_class(&self, part: &Part) -> String {
        let name = part.wrapper.as_ref().unwrap();
        let required_headers = part.headers.fields.iter().any(|h| h.wire.required());
        let mut out = format!(
            "/** Native value and metadata for a source-declared part. */\nfinal class {name} {{\n    /** @param {} $value\n     * @param array<array-key,string> $extraHeaders\n     */\n    public function __construct(\n        public {} $value,\n",
            self.ty(&part.payload, true),
            self.ty(&part.payload, false)
        );
        if required_headers {
            writeln!(out, "        public {} $headers,", part.headers.name).unwrap();
        }
        out.push_str("        public ?string $filename = null,\n        public ?string $contentType = null,\n");
        if !required_headers {
            writeln!(
                out,
                "        public {} $headers = new {}(),",
                part.headers.name, part.headers.name
            )
            .unwrap();
        }
        out.push_str("        public array $extraHeaders = [],\n    ) {}\n    public function toPart(CodecContext $context): PartValue {\n        $headers = $this->headers->toHeaders($context);\n        foreach ($this->extraHeaders as $key => $value) {\n            foreach ($headers as $known => $_) { if (strtolower((string)$key) === strtolower((string)$known)) { throw new JsonError('conversion', 'part header collision'); } }\n            $headers[$key] = $value;\n        }\n");
        if matches!(part.payload, Payload::Bytes) {
            out.push_str("        $context->enter();\n        try { $context->bytes(strlen($this->value->value)); } finally { $context->leave(); }\n");
        }
        writeln!(out,"        return new PartValue({}, $this->filename, $this->contentType, $headers);\n    }}\n    public static function fromPart(PartValue $part, CodecContext $context): self {{",self.encode(&part.payload,"$this->value")).unwrap();
        let (guard, decode): (String, String) = match &part.payload {
            Payload::Bytes => ("Bytes".into(), "$part->value".into()),
            payload => ("JsonValue".into(), self.decode(payload, "$part->value")),
        };
        writeln!(out,"        if (!$part->value instanceof {guard}) {{ throw new JsonError('conversion', 'part representation mismatch'); }}\n        $value = {decode};\n        $headers = {}::fromResponse(new HttpResponse(200, $part->headers, ''), $context);\n        $extra = $part->headers;",part.headers.name).unwrap();
        for header in &part.headers.fields {
            writeln!(
                out,
                "        unset($extra[{}]);",
                php(&header.wire.name().to_ascii_lowercase())
            )
            .unwrap();
        }
        out.push_str("        unset($extra['content-type'], $extra['content-disposition'], $extra['content-length']);\n        return new self(value: $value, filename: $part->filename, contentType: $part->contentType, headers: $headers, extraHeaders: $extra);\n    }\n}\n");
        out
    }
    fn body_class(&self, body: &BodyObject) -> String {
        let mut parts = body.parts.iter().collect::<Vec<_>>();
        parts.sort_by_key(|p| !p.wire.required());
        let mut out = format!(
            "/** Source-native {} body. Requiredness and counts are checked before serialization. */\nfinal class {} {{\n    /**\n",
            if body.multipart { "multipart" } else { "form" },
            body.name
        );
        for part in &parts {
            writeln!(
                out,
                "     * @param {} ${}",
                optional(self.part_ty(part, true), part.wire.required()),
                part.name
            )
            .unwrap();
        }
        if let Some(extra) = &body.extra {
            writeln!(
                out,
                "     * @param array<array-key,{}> $extra",
                self.part_ty(extra, true)
            )
            .unwrap();
        }
        out.push_str("     */\n    public function __construct(\n");
        for part in &parts {
            writeln!(
                out,
                "        public {} ${}{},",
                optional(self.part_ty(part, false), part.wire.required()),
                part.name,
                if part.wire.required() {
                    ""
                } else {
                    " = Absent::Value"
                }
            )
            .unwrap();
        }
        if body.extra.is_some() {
            out.push_str("        public array $extra = [],\n");
        }
        out.push_str("    ) {}\n    /** @return array<array-key,list<PartValue>> */\n    public function toParts(CodecContext $context): array {\n        $parts = [];\n");
        for part in &parts {
            if !part.wire.required() {
                writeln!(
                    out,
                    "        if ($this->{} !== Absent::Value) {{",
                    part.name
                )
                .unwrap();
            }
            self.write_part(
                &mut out,
                part,
                &php(part.wire.name().unwrap()),
                &format!("$this->{}", part.name),
            );
            if !part.wire.required() {
                out.push_str("        }\n");
            }
        }
        if let Some(extra) = &body.extra {
            let declared = parts
                .iter()
                .map(|p| format!("{} => true", php(p.wire.name().unwrap())))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(out,"        $declared = [{declared}];\n        foreach ($this->extra as $key => $item) {{\n            if (isset($declared[$key])) {{ throw new JsonError('conversion', 'body extra collides with declared name'); }}").unwrap();
            self.write_part(&mut out, extra, "$key", "$item");
            out.push_str("        }\n");
        }
        out.push_str("        return $parts;\n    }\n    /** @param array<array-key,list<PartValue>> $parts */\n    public static function fromParts(array $parts, CodecContext $context): self {\n");
        for part in &parts {
            let key = php(part.wire.name().unwrap());
            let var = format!("$field{}", part.name);
            if part.wire.required() {
                writeln!(out,"        if (!array_key_exists({key}, $parts)) {{ throw new JsonError('conversion', 'required body part absent'); }}").unwrap();
            }
            if !part.wire.required() {
                writeln!(out,"        {var} = Absent::Value;\n        if (array_key_exists({key}, $parts)) {{").unwrap();
            }
            self.read_part(&mut out, part, &format!("$parts[{key}]"), &var);
            if !part.wire.required() {
                out.push_str("        }\n");
            }
        }
        if let Some(extra) = &body.extra {
            let declared = parts
                .iter()
                .map(|p| format!("{} => true", php(p.wire.name().unwrap())))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(out,"        $extra = []; $declared = [{declared}];\n        foreach ($parts as $key => $items) {{\n            if (isset($declared[$key])) {{ continue; }}").unwrap();
            self.read_part(&mut out, extra, "$items", "$extra[$key]");
            out.push_str("        }\n");
        }
        let mut args = parts
            .iter()
            .map(|p| format!("{}: $field{}", p.name, p.name))
            .collect::<Vec<_>>();
        if body.extra.is_some() {
            args.push("extra: $extra".into());
        }
        writeln!(
            out,
            "        return new self({});\n    }}\n}}",
            args.join(", ")
        )
        .unwrap();
        out
    }
    fn write_part(&self, out: &mut String, part: &Part, key: &str, value: &str) {
        writeln!(out, "            $context->bytes(strlen((string) {key}));").unwrap();
        if part.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
            writeln!(out,"            if (!array_is_list({value})) {{ throw new JsonError('conversion', 'part collection must be a list'); }}\n            $parts[{key}] = [];\n            foreach ({value} as $entry) {{").unwrap();
            let encoded = part
                .wrapper
                .as_ref()
                .map(|_| "$entry->toPart($context)".into())
                .unwrap_or_else(|| {
                    format!("new PartValue({})", self.encode(&part.payload, "$entry"))
                });
            writeln!(
                out,
                "                $parts[{key}][] = {encoded};\n            }}"
            )
            .unwrap();
        } else {
            let encoded = part
                .wrapper
                .as_ref()
                .map(|_| format!("{value}->toPart($context)"))
                .unwrap_or_else(|| format!("new PartValue({})", self.encode(&part.payload, value)));
            writeln!(out, "            $parts[{key}] = [{encoded}];").unwrap();
        }
    }
    fn read_part(&self, out: &mut String, part: &Part, items: &str, target: &str) {
        let repeated = part.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems;
        if repeated {
            writeln!(
                out,
                "            {target} = [];\n            foreach ({items} as $part) {{"
            )
            .unwrap();
        } else {
            writeln!(out,"            if (count({items}) !== 1) {{ throw new JsonError('conversion', 'non-repeated part count mismatch'); }}\n            $part = {items}[0];").unwrap();
        }
        let decode = if let Some(wrapper) = &part.wrapper {
            format!("{wrapper}::fromPart($part, $context)")
        } else {
            let guard = if matches!(part.payload, Payload::Bytes) {
                "Bytes"
            } else {
                "JsonValue"
            };
            writeln!(out,"            if (!$part->value instanceof {guard}) {{ throw new JsonError('conversion', 'part representation mismatch'); }}").unwrap();
            if matches!(part.payload, Payload::Bytes) {
                "$part->value".into()
            } else {
                self.decode(&part.payload, "$part->value")
            }
        };
        writeln!(
            out,
            "            {target}{} = {decode};",
            if repeated { "[]" } else { "" }
        )
        .unwrap();
        if repeated {
            out.push_str("            }\n");
        }
    }
    fn result(&self, op: &Operation) -> String {
        let values = op
            .responses
            .iter()
            .filter(|r| r.success)
            .map(|r| r.name.clone())
            .collect::<Vec<_>>();
        if values.is_empty() {
            "never".into()
        } else {
            super::models::union(values)
        }
    }
    fn client(&self) -> String {
        let mut out = self.head();
        out.push_str("interface ClientInterface {\n");
        for op in self.plan.operations() {
            self.operation_doc(&mut out, op);
            let default = if self.arguments(op, false).iter().any(|(_, _, r)| *r) {
                String::new()
            } else {
                format!(" = new {}()", op.input)
            };
            writeln!(
                out,
                "    public function {}({} $input{default}, ?RequestOptions $options = null): {};",
                op.method,
                op.input,
                self.result(op)
            )
            .unwrap();
        }
        out.push_str("}\nfinal class Client implements ClientInterface {\n    public function __construct(private readonly Credentials $credentials, private readonly Transport $transport = new CurlTransport(), private readonly ClientOptions $options = new ClientOptions()) {}\n");
        if self.plan.credential_env().is_some() {
            out.push_str(&self.environment_factory());
        }
        for op in self.plan.operations() {
            self.operation_doc(&mut out, op);
            let default = if self.arguments(op, false).iter().any(|(_, _, r)| *r) {
                String::new()
            } else {
                format!(" = new {}()", op.input)
            };
            writeln!(out,"    public function {}({} $input{default}, ?RequestOptions $options = null): {} {{\n        try {{",op.method,op.input,self.result(op),).unwrap();
            out.push_str(&self.request_preparation(op));
            let streaming = op
                .responses
                .iter()
                .any(|r| matches!(r.payload, Payload::Stream(_)));
            writeln!(out,"        try {{ $response = Protocol::exchange($metadata, $this->credentials, $this->transport, $this->options, $options, $call, $values, $payload, {streaming}); }}\n        catch (JsonError|ValidationError|\\Error $error) {{ throw new SdkError('request_validation', 'request violates its protocol representation', previous: $error); }}").unwrap();
            out.push_str("        try {\n        $status = Protocol::matchStatus(Protocol::get($metadata, 'responses'), $response->status);\n        $declaration = Protocol::list($metadata, 'responses')[$status];\n        $forbidden = Protocol::text($metadata, 'method') === 'HEAD' || $response->status < 200 || in_array($response->status, [204, 205, 304], true);\n        $media = -1;\n        if (!$forbidden && Protocol::list($declaration, 'media') !== []) {\n            $contentType = $response->headerValues('content-type');\n            if (count($contentType) !== 1) { throw new SdkError('unexpected_media', 'exactly one Content-Type is required'); }\n            $media = Protocol::matchMedia(Protocol::get($declaration, 'media'), $contentType[0]);\n        }\n        $context = new CodecContext($call->control);\n");
            for response in &op.responses {
                let status = op
                    .wire
                    .responses()
                    .iter()
                    .position(|r| r.status_key() == response.status_key)
                    .unwrap();
                let media = response
                    .media
                    .as_ref()
                    .map(|m| {
                        response
                            .wire
                            .media()
                            .iter()
                            .position(|r| r.source().use_site().source() == &m.source)
                            .unwrap()
                            .to_string()
                    })
                    .unwrap_or("-1".into());
                writeln!(
                    out,
                    "        if ($status === {status} && $media === {media} && {} && {}) {{",
                    if response.success {
                        "$response->status >= 200 && $response->status < 300"
                    } else {
                        "!($response->status >= 200 && $response->status < 300)"
                    },
                    if matches!(response.payload, Payload::NoBody) {
                        "$forbidden"
                    } else {
                        "!$forbidden"
                    }
                )
                .unwrap();
                out.push_str("            try {\n");
                if streaming && !matches!(response.payload, Payload::Stream(_)) {
                    writeln!(out,"                $response = $response->buffer(min($call->maxResponseBytes, {}), $call->maxCaptureBytes, $call->control);",response.wire.max_body_bytes()).unwrap();
                }
                if !matches!(response.payload, Payload::Stream(_)) {
                    writeln!(out,"                if (strlen($response->body) > {}) {{ throw new SdkError('resource_limit', 'source response byte ceiling exceeded'); }}",response.wire.max_body_bytes()).unwrap();
                    if let Some(Media { wire, .. }) = &response.media
                        && let wire::Representation::Binary { bytes, .. } = wire.representation()
                        && bytes.max_bytes() < response.wire.max_body_bytes()
                    {
                        writeln!(out,"                if (strlen($response->body) > {}) {{ throw new SdkError('resource_limit', 'binary response byte ceiling exceeded'); }}",bytes.max_bytes()).unwrap();
                    }
                }
                match &response.payload {
                    Payload::NoBody=>out.push_str("                if ($response->body !== '') { throw new JsonError('conversion', 'HTTP forbids a response body'); }\n                $decoded = NoBody::Value;\n"),
                    Payload::Bytes=>out.push_str("                $decoded = new Bytes($response->body);\n"),
                    Payload::Object(name)=>writeln!(out,"                $parts = Parts::decode(Protocol::get(Protocol::list($declaration, 'media')[$media], 'representation'), $response->body, $response->header('content-type') ?? '', $this->options->maxHeaderBytes, $call->control);\n                $decoded = {name}::fromParts($parts, $context);").unwrap(),
                    Payload::Stream(id)=>{
                        let wire::Representation::Stream{stream}=response.media.as_ref().unwrap().wire.representation() else{unreachable!()};
                        let framing=match stream.framing(){wire::StreamFraming::ServerSentEvents=>"server-sent-events",wire::StreamFraming::JsonLines=>"json-lines"};
                        writeln!(out,"                $decoded = new ItemStream($response, {}, static fn (JsonValue $item): {} => Codecs::{}($item, new CodecContext($call->control)), $call, {}, {}, {}, {});",php(framing),self.plan.models().type_name(id,false),self.plan.models().nodes[id].codecs.from_value,stream.max_item_bytes(),php(&op.id),php(&format!("{}#{}",op.source.document(),op.source.pointer())),response.wire.max_body_bytes()).unwrap();
                    }
                    payload=>{
                        let text=response.media.as_ref().is_some_and(|m|matches!(m.wire.representation(),wire::Representation::Text{..}));
                        let json=if text {format!("Protocol::parseScalar($response->body, {})",php(match payload{Payload::Schema(id)=>scalar(self.plan,id),_=>"string"}))}else{"JsonValue::parse($response->body, new JsonLimits(control: $call->control))".into()};
                        writeln!(out,"                $json = {json};\n                $decoded = {};",self.decode(payload,"$json")).unwrap();
                    }
                }
                writeln!(out,"                $headers = {}::fromResponse($response, $context);\n                $call->check();\n            }} catch (JsonError|ValidationError|\\Error $error) {{ throw new SdkError('response_validation', 'response violates its source contract', Protocol::capture($response, $call), $error); }}\n            $links = [];",response.headers.name).unwrap();
                for link in response.wire.links() {
                    writeln!(
                        out,
                        "            $links[] = new Link({}, JsonValue::parse({}));",
                        php(link.name()),
                        php(&serde_json::to_string(link).unwrap())
                    )
                    .unwrap();
                }
                writeln!(
                    out,
                    "            {} new {}($decoded, $response, $headers, $links);\n        }}",
                    if response.success { "return" } else { "throw" },
                    response.name
                )
                .unwrap();
            }
            out.push_str("        throw new SdkError('unexpected_status', 'no admitted native response variant');\n        } catch (SdkError $error) { throw $error->withCapture(Protocol::capture($response, $call)); }\n");
            writeln!(
                out,
                "        }} catch (SdkError $error) {{ throw $error->at({}, {}); }}\n    }}",
                php(&op.id),
                php(&format!("{}#{}", op.source.document(), op.source.pointer()))
            )
            .unwrap();
        }
        // Compiled pagination walks are appended under the configured SDK
        // defaults; without them this contributes nothing at all.
        out.push_str(&super::pagination::client_methods(self.plan));
        // Compiled typed-event generators are appended for exactly the
        // discriminated SSE operations; without them this contributes nothing
        // at all.
        out.push_str(&super::stream::client_methods(self.plan));
        out.push_str("}\n");
        // The per-kind event, unknown-event and completion classes join the
        // generated file only when a typed stream operation exists.
        out.push_str(&super::stream::classes(self.plan));
        out
    }
    fn environment_factory(&self) -> String {
        use crate::credential_env::CredentialEnvKind;
        let policy = self
            .plan
            .credential_env()
            .expect("bound environment policy");
        let variables = policy
            .bindings()
            .iter()
            .map(|binding| binding.variable())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name, index))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut out = String::from(
            "    /** Snapshot the configured runtime variables once. Explicit construction remains authoritative.\n     * Missing, empty, unusable or unavailable values remain missing until operation auth selection.\n     * @throws SdkError Invalid client configuration.\n     */\n    public static function fromEnv(Transport $transport = new CurlTransport(), ClientOptions $options = new ClientOptions()): self {\n        $tokens = [];\n        if (\\function_exists('getenv')) {\n",
        );
        for (variable, index) in &variables {
            writeln!(out,"            $env{index} = \\getenv({});\n            if (!\\is_string($env{index}) || $env{index} === '' || \\strlen($env{index}) > 8192) {{ $env{index} = null; }}",php(variable)).unwrap();
        }
        for (index, binding) in policy.bindings().iter().enumerate() {
            let value = format!("$env{}", variables[binding.variable()]);
            writeln!(
                out,
                "            if ({value} !== null) {{\n                try {{"
            )
            .unwrap();
            let credential = match binding.kind() {
                CredentialEnvKind::Bearer => value.clone(),
                CredentialEnvKind::ApiKey => format!("new ApiKeyCredential({value})"),
            };
            writeln!(
                out,
                "                    $credential{index} = {credential};"
            )
            .unwrap();
            let cookie = self
                .plan
                .operations()
                .iter()
                .flat_map(|op| op.wire.security().alternatives())
                .flat_map(|alternative| alternative.requirements())
                .any(|requirement| {
                    requirement.name() == binding.name()
                        && requirement.scheme().use_site().source()
                            == binding.scheme().use_site().source()
                        && matches!(
                            requirement.credential(),
                            wire::CredentialHook::ApiKey {
                                location: wire::ParameterLocation::Cookie,
                                ..
                            }
                        )
                });
            if cookie {
                writeln!(
                    out,
                    "                    Protocol::encode({value}, 'none', 'cookie');"
                )
                .unwrap();
            }
            writeln!(out,"                    new Credentials([{} => $credential{index}]);\n                    $tokens[{}] = $credential{index};\n                }} catch (SdkError $error) {{\n                    // Unusable environment credentials do not block anonymous or alternate requirements.\n                }}\n            }}",php(binding.name()),php(binding.name())).unwrap();
        }
        out.push_str("        }\n        return new self(new Credentials($tokens), $transport, $options);\n    }\n");
        out
    }
    fn operation_doc(&self, out: &mut String, op: &Operation) {
        let mut tags = vec![
            format!("@param {} $input Source-bound native arguments.", op.input),
            "@param RequestOptions|null $options Per-call limits and explicit security selection."
                .into(),
            format!("@return {}", self.result(op)),
        ];
        tags.extend(
            op.responses
                .iter()
                .filter(|r| !r.success)
                .map(|r| format!("@throws {}", r.name)),
        );
        tags.push(
            "@throws SdkError Preparation, transport, response validation or resource failure."
                .into(),
        );
        doc_tags(
            out,
            &format!(
                "{}\n{}\nSource: {}#{}",
                op.id,
                op.wire
                    .description()
                    .map(|d| d.value().as_str())
                    .unwrap_or_default(),
                op.source.document(),
                op.source.pointer()
            ),
            &tags,
        );
    }
    fn readme(&self) -> String {
        let mut guide = format!(
            "# {}\n\nPHP 8.3+ typed Composer package generated from the admitted source protocol plan.\n\n```sh\ncomposer install\ncomposer typecheck\nphp examples/codecs.php\nphp examples/quickstart.php\nphp examples/client.php\n```\n\nUse native named constructors, `Absent::Value` for omission, `JsonNumber` for exact values, and `Bytes` for raw data. Explicit credentials use source scheme keys; `BasicCredential`, `ApiKeyCredential` and caller OAuth/OIDC hooks never acquire credentials automatically.\n\nThe client returns declared variants with `body`, `response`, typed `headers`, and source `links`. Actual status controls success/error for range/default declarations. See [native reference](docs/index.html), [source protocol](docs/protocol.json), [native coverage](docs/coverage.json) and [example findings](docs/examples.json).\n",
            self.plan.config().package_name
        );
        guide.push_str("\n## Complete first request\n\nThe executable recipe below uses source-validated values and the actual allocated PHP constructors. Raw bytes and credentials are explicitly labeled application fixtures. A selection without a complete source-compatible recipe reports that fixture obligation. `examples/client.php` contains all available complete calls. For a real call, supply your credential values and use `new CurlTransport()` with a declared server choice (or an explicit override). OAuth/OIDC hooks receive source metadata and permissions; they return an `AuthorizationCredential` and own acquisition/refresh.\n\n```php\n");
        guide.push_str(&self.quickstart());
        guide.push_str(r#"
```

## Values, presence and errors

Required nullable fields still require a constructor argument. `Absent::Value` omits an optional input; native `null` is an explicit null only where its source codec admits it. Source defaults are guidance, never silently inserted. Model fields are mutable and are validated again before transmission. Source-required constant tags are readonly constructor-initialized fields. String enums use native PHP enum cases.

Use `JsonNumber::fromString()` for exact decimals and unbounded integers. `toInt()` and `toDecimalString()` are explicit, checked conversions. Use `JsonValue` for an actual arbitrary JSON value. Checked `Codecs` methods and model `fromJson()`/`toJson()` methods preserve unknown-member and selected-union constraints.

Declared API errors carry a typed `body`, typed `headers`, source `links` and response metadata. Their constructor/PHPDoc narrows the inherited `ApiError::$response` union to the actual response kind. `SdkError` exposes a classification, operation/source identity, bounded capture and cause. Formatting omits payloads and causes. Exact status wins over range, then default; actual status determines success. A media mismatch never falls back to a less specific status. HEAD, 1xx, 204, 205 and 304 use `NoBody::Value`; undeclared content uses bounded `Bytes`. Undeclared statuses remain classified failures with bounded captures.

## Credentials, servers and parameters

Credential keys are source scheme names. Strings mean bearer tokens. Supply `BasicCredential`, `ApiKeyCredential` or `AuthorizationCredential` for those explicit source mechanisms. OR alternatives are considered in source order; all credentials in an AND alternative must be available. An anonymous alternative may win even when credentials are present. `RequestOptions(securityAlternative: index)` selects a specific alternative. OAuth/OIDC callbacks receive immutable flow/discovery metadata and source scopes/roles in `CredentialRequest`.

`ClientOptions(serverIndex: ..., serverVariables: [...], serverBaseUrl: ...)` selects a source server and variable values. Source defaults and enums remain authoritative. File-based source fixtures need an explicit HTTPS base (HTTP loopback is available for native test fixtures); HTTP(S) document origins can supply a relative base. `serverUrl` is an explicit complete override. Reserved expansion requires caller escaping where delimiters would change structure. Undefined combinations are refused. Whole-query form parameters serialize their checked object once, without a second URI-encoding pass.

Every request carries the automatic `ua/v1` attribution User-Agent. `ClientOptions(userAgent: ...)` overrides it entirely; an explicit empty string suppresses the header. `ClientOptions(applicationId: ...)` replaces the SDK identity token with `<name>` or `<name>/<version>` of RFC 9110 tokens; an invalid identifier omits the header rather than sending a malformed value. A declared User-Agent header parameter always wins over the automatic value.

## Media, forms and parts

Multiple request media use allocated selector classes with a typed `value` and `contentType`. Wildcards require a concrete media type. Responses preserve exact/range/default and media-specific variants, including `+json`, declared media parameters, text and actual bytes. Typed response headers are case-insensitive on the wire; Link metadata is available for explicit caller use.

Use `new Bytes($octets)` for actual binary contents. Generated named-part classes expose a typed value plus filename, content type and typed part headers. Their enclosing form/multipart classes enforce required fields, additional-member policy and property/item counts. Binary aggregates use structural checks and per-part codecs. MIME preambles/epilogues are ignored, and valid bytes resembling a boundary prefix are preserved. Positional multipart and ambiguous composite response styles require a separately admitted profile.

## Streaming and transport lifetime

Response `ItemStream<T>` is a single-use, closable iterable. Call `close()` in `finally` when retaining a response or iterator. Exhaustion, early iterator disposal, failure, cancellation and deadlines release the reader. The default cURL adapter is pull-based and queues one native chunk. `BodyReader::read()` returns a nonempty chunk or null at EOF; `close()` must be idempotent. Custom adapters must honor `HttpRequest::check()` and release resources.

OAS 3.2 SSE items are parsed envelopes: `data` stays a string, repeated data lines join with LF, comments/unknown fields and invalid id/retry values are ignored, and UTF-8 decoding follows the SSE replacement rule. `[DONE]` has no special meaning. JSON-lines validate every record against the source `itemSchema`, including a final record without LF; a BOM or malformed JSON is rejected. Request `iterable<T>` values are consumed once and buffered under finite request/item ceilings before transport. A caller-owned producer remains the caller's responsibility.

Per-call settings only lower generated ceilings. Whole-call deadlines include preparation and item consumption. Redirects, retries, cookie persistence, pagination loops, token acquisition and JSON-inside-SSE inference are never added.

## Source examples and interpretation

`docs/examples.json` retains v2 request/response, header, part and item roles with original provenance. Invalid declared examples remain findings; synthesized values are labeled. `examples/codecs.php` constructs native values and exercises their exact codecs. Complete client recipes add explicit byte/transport scaffolding and report unavailable recipes without substituting JSON null for bytes.

Standard generation has no compatibility profiles. `LegacyBinaryStringV1` is an explicit, versioned option for legacy binary string markers in binary media/parts. JSON string behavior remains the source's JSON behavior. Generation/session/compatibility records carry the selected profiles.
"#);
        let config = self.plan.config();
        if self.plan.program().version == "suspect.validation.experimental.v3" {
            guide.push_str("\n## Indexed resources and dynamic validation\n\nThis package uses `suspect.validation.experimental.v3` / `oas31-jsonschema202012-resources-dynamic`. It includes the nine scoped applicators. Resource metadata keeps physical source identities separate from logical canonical/base/alias URIs. No source loading, URI acquisition or schema parsing occurs during validation.\n\nEach entered node selects its indexed resource, including a nested entry whose resource-root schema is not evaluated. Dynamic plain-name bindings search actually entered resources outermost first. Empty/pointer/static-anchor fallbacks remain static. Unentered candidates remain inert. Every return and trial restores resource scope; cycle keys include the exact ordered resource context. Work and annotation budgets remain shared and failures cannot be inverted into a match.\n\nDynamic JSON values keep a checked `JsonValue` carrier wherever no fixed native representation is proved. All public codecs validate the selected source root, so a model admitted through a base resource still needs the stricter root's codec when used independently. Ordinary source closures retain their established v1/v2 program when resources are not needed.\n");
        }
        if self.plan.program().version == "suspect.validation.experimental.v2" {
            guide.push_str("\n## Scoped schema validation\n\nThis package uses `suspect.validation.experimental.v2` / `oas31-jsonschema202012-static-applicators`. Conditionals, dependent required names/schemas, contains bounds, pattern properties, property-name constraints and unevaluated properties/items run in the native checked validator. Each child starts with fresh evaluated-location sets; only the documented successful sets propagate. Trial failures share work/equality/numeric budgets and cannot turn into ordinary mismatches.\n\nDeclared object fields remain typed and mutable. Pattern-matched extras are retained as `array<array-key, JsonValue>` and every matching pattern plus the source additional-member policy is rechecked on encoding. Constraint-only/intersection shapes use a checked `JsonValue` carrier; positional arrays with scoped rules use `list<JsonValue>` when element types cannot be expressed faithfully. These values still pass the complete source codec on decode and encode.\n\nCall `Codecs::decode...` / `encode...` or the generated model methods for model-only use. Supplying a JSON-value carrier does not bypass source constraints. Resource/dynamic declarations use the separately checked v3 profile; v1/v2 envelopes reject v3 metadata and instructions.\n");
        }
        write!(guide,"\n## Generated ceilings\n\n| Policy | Ceiling |\n| --- | ---: |\n| Request bytes | {} |\n| Response bytes | {} |\n| Aggregate headers | {} |\n| Failure capture | {} |\n| Conversion depth | {} |\n| Conversion node visits | {} |\n| Conversion bytes | {} |\n| Protocol stream-item bytes | {} |\n\nActive explicit profiles: `{}`.\n",config.max_request_bytes,config.max_response_bytes,config.max_header_bytes,config.max_capture_bytes,config.max_depth,config.max_nodes,config.max_conversion_bytes,self.plan.surface.protocol.capabilities().limits().stream_item(),self.plan.surface.protocol.capabilities().profiles().iter().map(|p|p.name()).collect::<Vec<_>>().join(", ")).unwrap();
        if let Some(policy) = self.plan.credential_env() {
            write!(guide,"\n## Explicit runtime environment credentials\n\nThis package's versioned configuration maps source scheme names to environment **variable names**:\n\n```json\n{}\n```\n\nUse `Client::fromEnv(Transport $transport = new CurlTransport(), ClientOptions $options = new ClientOptions()): self` to snapshot these variables at factory invocation. Each mapped variable is read once. PHP `getenv` must be available; missing, empty, unusable or unavailable values remain missing. Anonymous and alternative security choices remain usable, and missing protected credentials fail with a secret-free `SdkError` of kind `credentials` before HTTP. The existing cURL/synchronous policy and source-default server selection apply.\n\n```php\n$client = \\{}\\Client::fromEnv();\n",serde_json::to_string_pretty(&policy.semantic_descriptor()).unwrap(),config.namespace).unwrap();
            if let Some(op) = self
                .plan
                .operations()
                .iter()
                .find(|op| op.id == "getCurrentKey")
            {
                writeln!(
                    guide,
                    "$key = $client->{}();\necho $key->response->status, PHP_EOL;",
                    op.method
                )
                .unwrap();
            }
            guide.push_str("```\n\nA custom adapter can be supplied with `Client::fromEnv(transport: $transport, options: $options)`. Explicit construction remains `new Client($credentials, $transport, $options)` and performs no environment lookup: empty maps and missing members are authoritative. Explicit null/Absent credential arguments retain their native type errors; invalid explicit tokens retain their existing credential validation. The factory has no credential parameter. It does not read `.env` files or acquire/refresh credentials. Later environment changes affect newly created clients only.\n");
            if self
                .plan
                .operations()
                .iter()
                .any(|op| op.id == "getCredits")
            {
                guide.push_str("\n`getCredits` is an explicitly chosen management-key operation; the factory does not infer key privileges or switch operation modes.\n");
            }
        }
        guide
    }
    fn reference(&self) -> String {
        let mut out = String::from(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>PHP SDK reference</title><style>body{font:16px system-ui;max-width:1100px;margin:2rem auto;padding:1rem;line-height:1.5}table{border-collapse:collapse;width:100%}td,th{border:1px solid #bbb;padding:.5rem;text-align:left}code{overflow-wrap:anywhere}section{margin-block:2rem}</style></head><body><h1>PHP native reference</h1><p><a href=\"../README.md\">First request and runtime guide</a> · <a href=\"examples.json\">Source example findings</a></p>",
        );
        out.push_str("<nav><h2>Operations</h2><ul>");
        for op in self.plan.operations() {
            write!(
                out,
                "<li><a href=\"#operation-{}\">Client::{}</a></li>",
                op.method, op.method
            )
            .unwrap();
        }
        out.push_str("</ul><p><a href=\"#native-types\">Inputs, results, parts and headers</a> · <a href=\"#runtime\">Runtime values and transport</a></p></nav>");
        for op in self.plan.operations() {
            write!(out,"<section id=\"operation-{}\"><h2>Client::{}</h2><p><code>{} {}</code></p><p>{}</p><p>operationId: <code>{}</code>. Input: <code>{}</code>. Result: <code>{}</code>. API-error base: <code>{}</code>.</p><p>Source: <code>{}#{}</code>.</p><table><tr><th>Input member</th><th>Wire binding</th><th>PHPDoc type</th><th>Required</th></tr>",op.method,html(&op.method),op.wire.method().as_str(),html(op.wire.path()),html(op.wire.description().map(|v|v.value().as_str()).unwrap_or_default()),html(&op.id),html(&op.input),html(&self.result(op)),html(&op.error),html(op.source.document().as_str()),html(op.source.pointer())).unwrap();
            for p in &op.parameters {
                write!(out,"<tr><td><code>${}</code></td><td>{}: <code>{}</code><p>{}</p></td><td><code>{}</code></td><td>{}</td></tr>",p.name,location(p.wire.location()),html(p.wire.name()),html(p.wire.description().map(|v|v.value().as_str()).unwrap_or_default()),html(&optional(self.plan.models().type_name(&p.schema,true),p.wire.required())),p.wire.required()).unwrap();
            }
            if !op.body.is_empty() {
                write!(out,"<tr><td><code>$body</code></td><td>Selected source media</td><td><code>{}</code></td><td>{}</td></tr>",html(&optional(self.input_type(op,true),op.body_required)),op.body_required).unwrap();
            }
            out.push_str("</table><h3>Responses</h3><ul>");
            for r in &op.responses {
                write!(out,"<li id=\"response-{}\"><strong>{}</strong> <code>{}</code>: status <code>{}</code>, <code>{}</code>; body <code>{}</code>, headers <code>{}</code>. <p>{}</p></li>",r.name,if r.success{"Returns"}else{"Throws"},r.name,r.status_key,html(r.media.as_ref().map(|m|m.wire.media_type().declared()).unwrap_or("HTTP body disposition")),html(&self.ty(&r.payload,true)),r.headers.name,html(r.wire.description().value())).unwrap();
            }
            out.push_str("</ul></section>");
        }
        for model in self.plan.models().nodes.values() {
            write!(
                out,
                "<section id=\"model-{}\"><h2>{}</h2><p>{}</p><p>Source: <code>{}#{}</code></p><p>PHPDoc type: <code>{}</code></p>",
                html(&model.name),
                html(&model.name),
                html(&model.description),
                html(model.source.document().as_str()),
                html(model.source.pointer()),
                html(&self.plan.models().type_name(&model.source, true))
            )
            .unwrap();
            if let Some(ctor) = &model.constructor {
                let args = ctor
                    .parameters
                    .iter()
                    .map(|name| format!("${name}"))
                    .chain(
                        ctor.extra_parameter
                            .iter()
                            .map(|name| format!("${name} = []")),
                    )
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(out,"<p>Constructor: <code>new {}({args})</code>. Required fields precede optional fields; optional values default to <code>Absent::Value</code>.</p>",model.name).unwrap();
            }
            write!(out,"<p>Exact codecs: <code>Codecs::{}</code>, <code>Codecs::{}</code>, <code>Codecs::{}</code>, <code>Codecs::{}</code>.</p>",model.codecs.decode,model.codecs.encode,model.codecs.from_value,model.codecs.to_value).unwrap();
            match &model.shape {
                super::models::Shape::Object { fields, .. } => {
                    out.push_str("<table><tr><th>Property</th><th>Wire name</th><th>Type</th><th>Source guidance</th></tr>");
                    for f in fields {
                        write!(out,"<tr><td>${}</td><td><code>{}</code></td><td><code>{}</code>{}</td><td>{}</td></tr>",f.name,html(&f.wire),html(&optional(self.plan.models().type_name(&f.source,true),f.required)),if f.initializer.is_some(){" (readonly source constant)"}else{""},html(&f.description)).unwrap();
                    }
                    out.push_str("</table>");
                    if let super::models::Shape::Object{extras:super::models::Extras::Patterned(patterns),..}=&model.shape {
                        out.push_str("<p><code>$extra: array&lt;array-key, JsonValue&gt;</code> preserves dynamic members. All matching patterns apply; additional members remain source-validated. Declared wire names cannot also occur in this map.</p><ul>");
                        for (pattern,schema) in patterns {write!(out,"<li><code>{}</code> → <a href=\"#model-{}\">{}</a></li>",html(pattern),self.plan.models().nodes[schema].name,html(&self.plan.models().type_name(schema,true))).unwrap();}
                        out.push_str("</ul>");
                    }
                }
                super::models::Shape::Enum { cases } => {
                    out.push_str("<ul>");
                    for (name, value) in cases {
                        write!(
                            out,
                            "<li><code>{}::{name}</code> = <code>{}</code></li>",
                            model.name,
                            html(&serde_json::to_string(value).unwrap())
                        )
                        .unwrap();
                    }
                    out.push_str("</ul>");
                }
                super::models::Shape::Json => out.push_str("<p>This native <code>JsonValue</code> carrier is checked against the complete source schema on every public decode/encode. Scoped constraints are runtime obligations.</p>"),
                _ => {}
            }
            out.push_str("</section>");
        }
        for body in &self.plan.surface.objects {
            write!(out,"<section id=\"body-{}\"><h2>{}</h2><p>Native {} aggregate. No byte member enters a JSON codec.</p><table><tr><th>Member</th><th>Wire name</th><th>Native/PHPDoc type</th></tr>",body.name,body.name,if body.multipart{"multipart"}else{"form"}).unwrap();
            for part in &body.parts {
                write!(
                    out,
                    "<tr><td>${}</td><td>{}</td><td><code>{}</code></td></tr>",
                    part.name,
                    html(part.wire.name().unwrap_or("additional")),
                    html(&optional(self.part_ty(part, true), part.wire.required()))
                )
                .unwrap();
            }
            out.push_str("</table></section>");
        }
        out.push_str("<h2 id=\"native-types\">Complete protocol declarations</h2>");
        for declaration in self.protocol_types() {
            let name = declaration["name"].as_str().unwrap();
            write!(out,"<section id=\"type-{name}\"><h3><code>{name}</code></h3><p>Source: <code>{}</code>. {}.</p><h4>Constructor arguments in order</h4><table><tr><th>Argument</th><th>Native type</th><th>PHPDoc type</th><th>Default</th></tr>",html(declaration["source"].as_str().unwrap()),declaration["role"].as_str().unwrap()).unwrap();
            for parameter in declaration["parameters"].as_array().unwrap() {
                write!(out,"<tr><td>${}</td><td><code>{}</code></td><td><code>{}</code></td><td>{}</td></tr>",parameter["name"].as_str().unwrap(),html(parameter["native"].as_str().unwrap()),html(parameter["phpdoc"].as_str().unwrap()),html(parameter["default"].as_str().unwrap())).unwrap();
            }
            out.push_str("</table><h4>Public fields</h4><ul>");
            for field in declaration["fields"].as_array().unwrap() {
                write!(
                    out,
                    "<li><code>${}: {}</code> — {}{}</li>",
                    field["name"].as_str().unwrap(),
                    html(field["phpdoc"].as_str().unwrap()),
                    if field["readonly"] == true {
                        "readonly"
                    } else {
                        "mutable"
                    },
                    if field["inherited"] == true {
                        "; inherited native HttpResponse|StreamResponse storage"
                    } else {
                        ""
                    }
                )
                .unwrap();
            }
            out.push_str("</ul></section>");
        }
        out.push_str("<section id=\"runtime\"><h2>Runtime values and transport</h2><p><code>Bytes(string $value)</code> holds actual octets. <code>NoBody::Value</code> marks HTTP-forbidden content. <code>Absent::Value</code> marks omission, independently of nullable values.</p><p><code>JsonNumber::fromString()</code>, <code>fromInt()</code>, <code>toInt()</code>, <code>toDecimalString()</code> and <code>compare()</code> preserve exact decimal values. <code>JsonValue</code> is immutable; its factories, <code>parse()</code> and <code>toJson()</code> enforce finite limits.</p><p><code>Credentials(array $tokens)</code> maps source scheme names to bearer strings, <code>BasicCredential</code>, <code>ApiKeyCredential</code>, <code>AuthorizationCredential</code> or caller OAuth/OIDC closures. <code>CredentialRequest</code> exposes operationId, scheme, kind, permissions and immutable metadata.</p><p><code>Transport::send(HttpRequest): HttpResponse</code> and <code>StreamTransport::open(HttpRequest): StreamResponse</code> are the adapter seams. A <code>BodyReader</code> returns nonempty byte chunks or null at EOF and closes idempotently. <code>ItemStream&lt;T&gt;</code> is single-use, closable, bounded, and releases its reader on exhaustion or failure. Close it in finally when retaining a response or iterator.</p><p>Every operation documents its concrete API-error variants and <code>SdkError</code>. Errors retain stable source/operation identity, a bounded response capture and a cause. Metadata Links remain explicit caller guidance.</p></section>");
        if let Some(policy) = self.plan.credential_env() {
            out.push_str("<section id=\"credential-env\"><h2>Client::fromEnv</h2><p><code>public static fromEnv(Transport $transport = new CurlTransport(), ClientOptions $options = new ClientOptions()): self</code></p><p>Snapshots mapped variables once during factory execution. Explicit Client construction is authoritative and performs no environment lookup. Missing/empty/unavailable values remain missing; protected calls fail before HTTP with secret-free credential errors.</p><table><tr><th>Source scheme</th><th>Variable name</th><th>Kind</th></tr>");
            for binding in policy.bindings() {
                write!(
                    out,
                    "<tr><td>{}</td><td>{}</td><td>{:?}</td></tr>",
                    html(binding.name()),
                    html(binding.variable()),
                    binding.kind()
                )
                .unwrap();
            }
            out.push_str("</table></section>");
        }
        out.push_str("</body></html>");
        out
    }
    fn coverage(&self) -> String {
        let id = |id: &SchemaId| json!({"document":id.document().as_str(),"pointer":id.pointer()});
        let mut value = json!({"version":"suspect.php-native.v2","namespace":self.plan.config().namespace,"adapter":self.plan.surface.protocol.capabilities().adapter(),"compatibilityProfiles":self.plan.surface.protocol.capabilities().profiles(),"validationVersion":self.plan.program().version,"validationProfile":self.plan.program().profile,
            "operations":self.plan.operations().iter().map(|op|json!({"source":id(&op.source),"operationId":op.id,"method":op.method,"inputType":op.input,"errorType":op.error,"resultType":self.result(op),
                "responses":op.responses.iter().map(|r|json!({"source":id(&r.source),"status":r.status_key,"type":r.name,"success":r.success,"body":"body","metadata":"response"})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "models":self.plan.models().nodes.values().map(|node|json!({"source":id(&node.source),"name":node.name,"type":self.plan.models().type_name(&node.source,false),"phpdocType":self.plan.models().type_name(&node.source,true),
                "constructor":node.constructor.as_ref().map(|c|json!({"method":c.method,"parameters":c.parameters,"extras":c.extra_parameter})),
                "codecs":{"decode":node.codecs.decode,"encode":node.codecs.encode,"fromValue":node.codecs.from_value,"toValue":node.codecs.to_value}})).collect::<Vec<_>>(),
            "protocolTypes":self.protocol_types(),"validatedNativeExamples":self.plan.core.samples.len(),"nativeRecipes":docs::availability(self),"exampleRoles":self.plan.core.examples.operations().iter().flat_map(|op|op.entries.iter().map(|e|format!("{:?}",e.role))).collect::<BTreeSet<_>>() });
        if let Some(policy) = self.plan.credential_env() {
            value["credentialEnv"] = json!({"policy":policy.semantic_descriptor(),"factory":{"owner":"Client","name":"fromEnv","static":true,"parameters":["transport","options"],"environmentRead":"factory-time","explicitConstructorUsesEnvironment":false}});
        }
        serde_json::to_string_pretty(&value).unwrap()
    }
    fn protocol_types(&self) -> Vec<Value> {
        let field = |name: &str,
                     native: String,
                     phpdoc: String,
                     required: bool,
                     default: &str,
                     readonly: bool| json!({"name":name,"native":native,"phpdoc":phpdoc,"required":required,"default":if required{"required argument"}else{default},"readonly":readonly,"inherited":false});
        let declaration = |name: &str, source: String, role: &str, fields: Vec<Value>| json!({"name":name,"source":source,"role":role,"parameters":fields,"fields":fields});
        let mut out = Vec::new();
        let mut headers = Vec::new();
        for op in self.plan.operations() {
            let doc = self.arguments(op, true);
            let fields = self
                .arguments(op, false)
                .iter()
                .zip(doc.iter())
                .map(|((name, native, required), (_, phpdoc, _))| {
                    field(
                        name,
                        optional(native.clone(), *required),
                        optional(phpdoc.clone(), *required),
                        *required,
                        "Absent::Value",
                        false,
                    )
                })
                .collect::<Vec<_>>();
            out.push(declaration(
                &op.input,
                format!("{}#{}", op.source.document(), op.source.pointer()),
                "operation input",
                fields,
            ));
            for media in &op.body {
                if let Some(name) = &media.wrapper {
                    let fields = vec![
                        field(
                            "value",
                            self.request_ty(&media.payload, false),
                            self.request_ty(&media.payload, true),
                            true,
                            "",
                            true,
                        ),
                        field(
                            "contentType",
                            "string".into(),
                            "string".into(),
                            !matches!(
                                media.wire.media_type().range(),
                                wire::MediaRange::Concrete { .. }
                            ),
                            media.wire.media_type().declared(),
                            true,
                        ),
                    ];
                    out.push(declaration(
                        name,
                        format!("{}#{}", media.source.document(), media.source.pointer()),
                        "request media selector",
                        fields,
                    ));
                }
            }
            for response in &op.responses {
                headers.push((&response.headers, &response.source));
                let metadata = if matches!(response.payload, Payload::Stream(_)) {
                    "StreamResponse"
                } else {
                    "HttpResponse"
                };
                let fields = vec![
                    field(
                        "body",
                        self.ty(&response.payload, false),
                        self.ty(&response.payload, true),
                        true,
                        "",
                        true,
                    ),
                    field("response", metadata.into(), metadata.into(), true, "", true),
                    field(
                        "headers",
                        response.headers.name.clone(),
                        response.headers.name.clone(),
                        true,
                        "",
                        true,
                    ),
                    field(
                        "links",
                        "array".into(),
                        "list<Link>".into(),
                        false,
                        "[]",
                        true,
                    ),
                ];
                let mut record = declaration(
                    &response.name,
                    format!(
                        "{}#{}",
                        response.source.document(),
                        response.source.pointer()
                    ),
                    if response.success {
                        "response result"
                    } else {
                        "declared API error"
                    },
                    fields,
                );
                if !response.success {
                    record["fields"][1]["native"] = json!("HttpResponse|StreamResponse");
                    record["fields"][1]["inherited"] = json!(true);
                }
                out.push(record);
            }
        }
        for body in &self.plan.surface.objects {
            let mut parts = body.parts.iter().collect::<Vec<_>>();
            parts.sort_by_key(|p| !p.wire.required());
            let mut fields = parts
                .iter()
                .map(|p| {
                    field(
                        &p.name,
                        optional(self.part_ty(p, false), p.wire.required()),
                        optional(self.part_ty(p, true), p.wire.required()),
                        p.wire.required(),
                        "Absent::Value",
                        false,
                    )
                })
                .collect::<Vec<_>>();
            if let Some(extra) = &body.extra {
                fields.push(field(
                    "extra",
                    "array".into(),
                    format!("array<array-key,{}>", self.part_ty(extra, true)),
                    false,
                    "[]",
                    false,
                ));
            }
            out.push(declaration(
                &body.name,
                format!("{}#{}", body.source.document(), body.source.pointer()),
                if body.multipart {
                    "named multipart body"
                } else {
                    "form body"
                },
                fields,
            ));
            for part in body.parts.iter().chain(body.extra.iter()) {
                if let Some(name) = &part.wrapper {
                    headers.push((&part.headers, part.wire.source().use_site().source()));
                    let required_headers = part.headers.fields.iter().any(|h| h.wire.required());
                    let mut fields = vec![field(
                        "value",
                        self.ty(&part.payload, false),
                        self.ty(&part.payload, true),
                        true,
                        "",
                        false,
                    )];
                    if required_headers {
                        fields.push(field(
                            "headers",
                            part.headers.name.clone(),
                            part.headers.name.clone(),
                            true,
                            "",
                            false,
                        ));
                    }
                    fields.push(field(
                        "filename",
                        "string|null".into(),
                        "string|null".into(),
                        false,
                        "null",
                        false,
                    ));
                    fields.push(field(
                        "contentType",
                        "string|null".into(),
                        "string|null".into(),
                        false,
                        "null",
                        false,
                    ));
                    if !required_headers {
                        fields.push(field(
                            "headers",
                            part.headers.name.clone(),
                            part.headers.name.clone(),
                            false,
                            &format!("new {}()", part.headers.name),
                            false,
                        ));
                    }
                    fields.push(field(
                        "extraHeaders",
                        "array".into(),
                        "array<array-key,string>".into(),
                        false,
                        "[]",
                        false,
                    ));
                    let source = part.wire.source().use_site().source();
                    out.push(declaration(
                        name,
                        format!("{}#{}", source.document(), source.pointer()),
                        "native part value and metadata",
                        fields,
                    ));
                }
            }
        }
        let mut seen = BTreeSet::new();
        for (header, source) in headers {
            if !seen.insert(&header.name) {
                continue;
            }
            let mut ordered = header.fields.iter().collect::<Vec<_>>();
            ordered.sort_by_key(|h| !h.wire.required());
            let fields = ordered
                .iter()
                .map(|h| {
                    field(
                        &h.name,
                        optional(
                            self.plan.models().type_name(&h.schema, false),
                            h.wire.required(),
                        ),
                        optional(
                            self.plan.models().type_name(&h.schema, true),
                            h.wire.required(),
                        ),
                        h.wire.required(),
                        "Absent::Value",
                        false,
                    )
                })
                .collect::<Vec<_>>();
            out.push(declaration(
                &header.name,
                format!("{}#{}", source.document(), source.pointer()),
                "typed HTTP/part headers",
                fields,
            ));
        }
        out
    }
    fn examples(&self) -> String {
        let mut out = self.head();
        out.push_str("require getenv('SUSPECT_SDK_AUTOLOAD') ?: dirname(__DIR__) . '/vendor/autoload.php';\n");
        for op in self.plan.core.examples.operations() {
            for (index, entry) in op.entries.iter().enumerate() {
                let node = &self.plan.models().nodes[&entry.schema];
                let sample = self
                    .plan
                    .core
                    .samples
                    .iter()
                    .find(|s| s.operation == op.source && s.entry_index == index)
                    .unwrap();
                doc(
                    &mut out,
                    &format!(
                        "{} source-bound {:?} example.",
                        crate::http_examples::origin(&entry.origin),
                        entry.role
                    ),
                );
                writeln!(
                    out,
                    "$value = {};\n$encoded = Codecs::{}($value);\nCodecs::{}($encoded);",
                    super::super::emit::docs::render_at(&sample.expression, ""),
                    node.codecs.encode,
                    node.codecs.decode
                )
                .unwrap();
            }
        }
        out.push_str("echo 'source codec examples passed', PHP_EOL;\n");
        out
    }
    fn quickstart(&self) -> String {
        docs::quickstart(self, false)
    }
}
fn scalar(plan: &SdkPlan, id: &SchemaId) -> &'static str {
    match &plan.models().nodes[id].shape {
        super::models::Shape::Ref(id) => scalar(plan, id),
        super::models::Shape::Boolean => "boolean",
        super::models::Shape::Number => "number",
        _ => "string",
    }
}
fn optional(ty: String, required: bool) -> String {
    if required {
        ty
    } else {
        super::models::union(vec![ty, "Absent".into()])
    }
}
fn location(location: wire::ParameterLocation) -> &'static str {
    match location {
        wire::ParameterLocation::Path => "path",
        wire::ParameterLocation::Query => "query",
        wire::ParameterLocation::Querystring => "querystring",
        wire::ParameterLocation::Header => "header",
        wire::ParameterLocation::Cookie => "cookie",
    }
}
pub(super) fn php(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_ascii_control() => write!(out, "\\x{:02x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn doc(out: &mut String, value: &str) {
    doc_tags(out, value, &[]);
}
pub(super) fn doc_tags(out: &mut String, value: &str, tags: &[String]) {
    out.push_str("/**\n");
    for line in value.lines() {
        writeln!(
            out,
            " * {}",
            line.replace("*/", "* /")
                .replace("?>", "? >")
                .replace('@', "&#64;")
        )
        .unwrap();
    }
    for tag in tags {
        writeln!(out, " * {tag}").unwrap();
    }
    out.push_str(" */\n");
}
fn error_metadata_type(op: &Operation) -> String {
    let types = op
        .responses
        .iter()
        .filter(|r| !r.success)
        .map(|r| {
            if matches!(r.payload, Payload::Stream(_)) {
                "StreamResponse".into()
            } else {
                "HttpResponse".into()
            }
        })
        .collect::<Vec<_>>();
    if types.is_empty() {
        "HttpResponse|StreamResponse".into()
    } else {
        super::models::union(types)
    }
}
fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
