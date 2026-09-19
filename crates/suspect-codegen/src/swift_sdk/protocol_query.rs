//! Whole-query form lowering over the shared FormPlan. The parameter's native
//! object and complete codec remain the public input; fields bind their own codecs.
use super::{
    PlannedQueryForm, SdkPlan,
    protocol_metadata::{self as meta, q},
};
use crate::http_protocol::{AdditionalParts, PartMultiplicity, PartPlan, PartRepresentation};
use std::fmt::Write as _;

pub(super) fn emit(out: &mut String, form: &PlannedQueryForm, plan: &SdkPlan) {
    let rules = form.wire.rules();
    let _ = writeln!(
        out,
        "enum {} {{\n    static func encode(_ value: JsonValue, limit: Int, partLimit: Int) throws -> String {{\n        guard case .object(let object) = value else {{ throw JsonError(.representation, \"whole-query form requires an object\") }}",
        form.encoder_name
    );
    let _ = writeln!(
        out,
        "        try HTTPObjectRules(names: [{}], required: [{}], additional: {}, minProperties: {}, maxProperties: {}).check(object.keys)",
        form.wire
            .fields()
            .iter()
            .map(|p| q(p.name().unwrap()))
            .collect::<Vec<_>>()
            .join(", "),
        rules
            .required()
            .iter()
            .map(|v| q(v.value()))
            .collect::<Vec<_>>()
            .join(", "),
        matches!(form.wire.additional(), AdditionalParts::Allowed(_)),
        meta::count(rules.min_properties()),
        meta::count(rules.max_properties())
    );
    out.push_str("        var output = HTTPWireBuffer(limit: limit)\n        for (name, field) in object.members {\n");
    for (index, part) in form.wire.fields().iter().enumerate() {
        let _ = writeln!(
            out,
            "            {}if name.utf8.elementsEqual({}.utf8) {{",
            if index == 0 { "" } else { "else " },
            q(part.name().unwrap())
        );
        field(out, part, plan);
        out.push_str("            }\n");
    }
    let has_fields = !form.wire.fields().is_empty();
    if has_fields {
        out.push_str("            else {\n");
    }
    match form.wire.additional() {
        AdditionalParts::Forbidden => out.push_str("                throw JsonError(.representation, \"undeclared whole-query form field\")\n"),
        AdditionalParts::Allowed(part) => field(out, part, plan),
    }
    if has_fields {
        out.push_str("            }\n");
    }
    out.push_str("        }\n        return output.string\n    }\n}\n\n");
}

fn field(out: &mut String, part: &PartPlan, plan: &SdkPlan) {
    let _ = writeln!(out, "                let rule = {}", meta::part(part, plan));
    if part.multiplicity() == PartMultiplicity::RepeatedArrayItems {
        out.push_str("                guard case .array(let values) = field else { throw JsonError(.representation, \"repeated form field requires an array\") }\n                try rule.checkCount(values.count)\n");
    } else {
        out.push_str(
            "                let values = [field]\n                try rule.checkCount(1)\n",
        );
    }
    out.push_str("                for value in values {\n                    let remaining = min(partLimit, limit - output.count)\n");
    let codec = match part.representation() {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => codec,
        PartRepresentation::Binary { .. } => {
            unreachable!("the shared querystring form plan excludes bytes")
        }
    };
    let _ = writeln!(
        out,
        "                    _ = try Codecs.{}.decodeValue(value, limits: JsonLimits(maxBytes: remaining))",
        plan.models.codecs[codec.schema().id()]
    );
    match part.representation() {
        PartRepresentation::Json { outer_encoding, .. } => {
            let _ = writeln!(
                out,
                "                    let text = try value.encodedString(limits: JsonLimits(maxBytes: remaining))\n                    try HTTPParts.appendForm(text, name: name, encoding: {}, to: &output)",
                meta::encoding(*outer_encoding)
            );
        }
        PartRepresentation::Text {
            scalar,
            outer_encoding,
            ..
        } => {
            let _ = writeln!(
                out,
                "                    let text = try HTTPWire.scalar(value, type: {})\n                    try HTTPParts.appendForm(text, name: name, encoding: {}, to: &output)",
                meta::scalar(*scalar),
                meta::encoding(*outer_encoding)
            );
        }
        PartRepresentation::Style { serialization, .. } => {
            let _ = writeln!(
                out,
                "                    let parameter = HTTPParameter(name: name, location: \"query\", required: {}, source: {}, serialization: {})\n                    let text = try HTTPWire.serialize(value, parameter: parameter, limit: remaining)\n                    try HTTPParts.appendStyle(text, to: &output)",
                part.required(),
                meta::source(part.source().use_site().source()),
                meta::serialization(serialization, codec, plan)
            );
        }
        PartRepresentation::Binary { .. } => unreachable!(),
    }
    out.push_str("                }\n");
}
