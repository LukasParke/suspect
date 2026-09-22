//! Native finite positional MIME layouts. Prefix positions and remaining items
//! stay typed; neither their aggregate nor byte payloads become JSON placeholders.
use super::{PlannedPart, PlannedPositionalParts, SdkPlan, protocol_metadata as meta};
use crate::http_protocol::{ParameterSerialization, PartRepresentation, Style, WireShape};
use std::fmt::Write as _;

pub(super) fn emit(out: &mut String, parts: &PlannedPositionalParts, plan: &SdkPlan) {
    for part in parts.prefix.iter().chain(parts.items.as_deref()) {
        super::protocol_emit::part_input_type(out, part, plan);
    }
    let _ = writeln!(
        out,
        "/// Finite ordered MIME parts. Missing optional positions must form a trailing suffix.\npublic struct {}: Sendable {{",
        parts.type_name
    );
    let mut args = Vec::new();
    for p in &parts.prefix {
        let ty = if p.wire.required() {
            p.item_type.clone()
        } else {
            format!("OptionalField<{}>", p.item_type)
        };
        let _ = writeln!(out, "    public var {}: {ty}", p.field_name);
        args.push(format!(
            "{}: {ty}{}",
            p.field_name,
            if p.wire.required() { "" } else { " = .missing" }
        ));
    }
    if let Some(item) = &parts.items {
        let _ = writeln!(out, "    public var items: [{}]", item.item_type);
        args.push(format!("items: [{}] = []", item.item_type));
    }
    let _ = writeln!(out, "    public init({}) {{", args.join(", "));
    for p in &parts.prefix {
        let _ = writeln!(out, "        self.{0} = {0}", p.field_name);
    }
    if parts.items.is_some() {
        out.push_str("        self.items = items\n");
    }
    out.push_str("    }\n");
    encode(out, parts, plan);
    decode(out, parts, plan);
    out.push_str("}\n\n");
}

fn rules(p: &PlannedPositionalParts) -> String {
    format!(
        "HTTPPositionalRules(prefix: {}, items: {}, minimum: {}, maximum: {})",
        p.prefix.len(),
        p.items.is_some(),
        meta::count(p.min_items.as_ref()),
        meta::count(p.max_items.as_ref())
    )
}
fn encode(out: &mut String, p: &PlannedPositionalParts, plan: &SdkPlan) {
    out.push_str("    func encode(contentType: String, boundary selectedBoundary: String?, limit: Int, partLimit: Int) throws -> HTTPBody {\n");
    let _ = writeln!(
        out,
        "        {} present: [Bool] = []",
        if p.prefix.is_empty() { "let" } else { "var" }
    );
    for part in &p.prefix {
        if part.wire.required() {
            out.push_str("        present.append(true)\n");
        } else {
            let _ = writeln!(
                out,
                "        if case .value = self.{} {{ present.append(true) }} else {{ present.append(false) }}",
                part.field_name
            );
        }
    }
    let _ = writeln!(
        out,
        "        try {}.check(present: present, remaining: {})",
        rules(p),
        if p.items.is_some() {
            "items.count"
        } else {
            "0"
        }
    );
    out.push_str("        let media = try HTTPMediaType(contentType)\n        let boundary = try HTTPParts.boundary(selectedBoundary ?? media.parameters[\"boundary\"])\n        guard media.parameters[\"boundary\"] == nil || media.parameters[\"boundary\"] == boundary else { throw JsonError(.representation, \"conflicting multipart boundaries\") }\n        var output = Data()\n");
    for part in &p.prefix {
        if part.wire.required() {
            let _ = writeln!(
                out,
                "        do {{\n            let item = self.{}",
                part.field_name
            );
        } else {
            let _ = writeln!(
                out,
                "        if case .value(let item) = self.{} {{",
                part.field_name
            );
        }
        encode_item(out, part, p.form_data, plan);
        out.push_str("        }\n");
    }
    if let Some(item) = &p.items {
        out.push_str("        for item in items {\n");
        encode_item(out, item, p.form_data, plan);
        out.push_str("        }\n");
    }
    out.push_str("        try HTTPParts.finishMultipart(&output, boundary: boundary, limit: limit)\n        return HTTPBody(bytes: output, contentType: media.parameters[\"boundary\"] == nil ? contentType + \"; boundary=\" + boundary : contentType)\n    }\n");
}
fn encode_item(out: &mut String, p: &PlannedPart, form_data: bool, plan: &SdkPlan) {
    let _ = writeln!(
        out,
        "            let rule = {}\n            let remaining = min(partLimit, limit - output.count)",
        meta::part(&p.wire, plan)
    );
    let headers = if p.header_type.is_some() {
        "try item.headers.encode(limit: min(partLimit, 65_536))"
    } else {
        "[HTTPHeader]()"
    };
    let _ = writeln!(out, "            let headers = {headers}");
    let value = match p.wire.representation() {
        PartRepresentation::Binary { bytes } => format!(
            "try HTTPParts.bytes(item.value, limit: min(remaining, {}))",
            bytes.max_bytes()
        ),
        PartRepresentation::Json { codec, .. } => format!(
            "try Codecs.{}.encode(item.value, limits: JsonLimits(maxBytes: remaining))",
            plan.models.codecs[codec.schema().id()]
        ),
        PartRepresentation::Text { codec, scalar, .. } => format!(
            "try HTTPParts.textBytes(HTTPWire.scalar(Codecs.{}.encodeValue(item.value, limits: JsonLimits(maxBytes: remaining)), type: {}), limit: remaining)",
            plan.models.codecs[codec.schema().id()],
            meta::scalar(*scalar)
        ),
        PartRepresentation::Style {
            codec,
            serialization,
        } => format!(
            "try HTTPParts.textBytes(HTTPWire.multipartContent(Codecs.{}.encodeValue(item.value, limits: JsonLimits(maxBytes: remaining)), serialization: {}, limit: remaining), limit: remaining)",
            plan.models.codecs[codec.schema().id()],
            meta::serialization(serialization, codec, plan)
        ),
    };
    let _ = writeln!(
        out,
        "            let bytes = {value}\n            try HTTPParts.appendPositionalPart(bytes: bytes, headers: headers, filename: item.filename, contentType: rule.contentType(item.contentType), formData: {form_data}, boundary: boundary, to: &output, bodyLimit: limit, partLimit: rule.maxBytes)"
    );
}

fn decode(out: &mut String, p: &PlannedPositionalParts, plan: &SdkPlan) {
    let _ = writeln!(
        out,
        "    static func decode(_ bytes: Data, contentType: String, limit: Int, partLimit: Int) throws -> Self {{\n        _ = try HTTPParts.bytes(bytes, limit: limit)\n        let parts = try HTTPParts.parseMultipart(bytes, contentType: contentType, partLimit: partLimit, requireFormDataDisposition: {})\n        try {}.checkCount(parts.count)",
        p.form_data,
        rules(p)
    );
    let mut args = Vec::new();
    for (index, part) in p.prefix.iter().enumerate() {
        let ty = if part.wire.required() {
            part.item_type.clone()
        } else {
            format!("OptionalField<{}>", part.item_type)
        };
        let _ = writeln!(
            out,
            "        let field{index}: {ty}\n        if parts.count > {index} {{\n            let part = parts[{index}]"
        );
        let expression = decode_item(part, plan);
        let _ = writeln!(
            out,
            "            let decoded: {} = {}\n            field{index} = {}",
            part.item_type,
            expression,
            if part.wire.required() {
                "decoded"
            } else {
                ".value(decoded)"
            }
        );
        if part.wire.required() {
            out.push_str("        } else { throw JsonError(.representation, \"required positional part is missing\") }\n");
        } else {
            let _ = writeln!(out, "        }} else {{ field{index} = .missing }}");
        }
        args.push(format!("{}: field{index}", part.field_name));
    }
    if let Some(item) = &p.items {
        let _ = writeln!(
            out,
            "        let items = try parts.dropFirst({}).map {{ part -> {} in {} }}",
            p.prefix.len(),
            item.item_type,
            decode_item(item, plan)
        );
        args.push("items: items".into());
    }
    let _ = writeln!(out, "        return Self({})\n    }}", args.join(", "));
}
fn decode_item(p: &PlannedPart, plan: &SdkPlan) -> String {
    let value = match p.wire.representation() {
        PartRepresentation::Binary { bytes } => format!(
            "try HTTPParts.bytes(part.bytes, limit: min(partLimit, {}))",
            bytes.max_bytes()
        ),
        PartRepresentation::Json { codec, .. } => format!(
            "try Codecs.{}.decode(part.bytes, limits: JsonLimits(maxBytes: partLimit))",
            plan.models.codecs[codec.schema().id()]
        ),
        PartRepresentation::Text { codec, scalar, .. } => format!(
            "try Codecs.{}.decodeValue(HTTPWire.scalarValue(HTTPWire.text(part.bytes), type: {}), limits: JsonLimits(maxBytes: partLimit))",
            plan.models.codecs[codec.schema().id()],
            meta::scalar(*scalar)
        ),
        PartRepresentation::Style {
            codec,
            serialization,
        } => format!(
            "try Codecs.{}.decodeValue(HTTPWire.parsePartStyle(HTTPWire.text(part.bytes), name: part.name, serialization: {}, multipart: true), limits: JsonLimits(maxBytes: partLimit))",
            plan.models.codecs[codec.schema().id()],
            meta::serialization(serialization, codec, plan)
        ),
    };
    let item = if let Some(headers) = &p.header_type {
        format!(
            "try {}(value: {value}, headers: {headers}.decode(part.headers, limit: partLimit), filename: part.filename, contentType: part.contentType)",
            p.item_type
        )
    } else {
        format!(
            "{}({value}, filename: part.filename, contentType: part.contentType)",
            p.item_type
        )
    };
    format!(
        "try {{ () throws -> {} in _ = try {}.contentType(part.contentType); return {item} }}()",
        p.item_type,
        meta::part(&p.wire, plan)
    )
}

pub(super) fn style_expands(p: &crate::http_protocol::PartPlan) -> bool {
    matches!(
        p.representation(),
        PartRepresentation::Style {
            serialization: ParameterSerialization::Style {
                style: Style::DeepObject,
                ..
            },
            ..
        } | PartRepresentation::Style {
            serialization: ParameterSerialization::Style {
                explode: true,
                shape: WireShape::Array { .. } | WireShape::FlatObject { .. },
                ..
            },
            ..
        }
    )
}
