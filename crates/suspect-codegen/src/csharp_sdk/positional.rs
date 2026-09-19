//! C# finite positional multipart values and source-bound per-position codecs.
use super::{SdkPlan, emit::xml, http, protocol::PlannedPositionalParts};

pub(super) fn declaration(out: &mut String, parts: &PlannedPositionalParts) {
    out.push_str(&format!("/// <summary>Finite ordered multipart prefix and typed remaining items. Missing prefix slots cannot precede present values.</summary>\npublic sealed record {}\n{{\n",parts.native_type));
    for (index, part) in parts.prefix.iter().enumerate() {
        out.push_str(&format!("    /// <summary>Position {}. Source: {}#{}.</summary>\n    public {}{} {} {{ get; set; }}\n",index,xml(part.wire.schema().id().document().as_str()),xml(part.wire.schema().id().pointer()),if part.wire.required(){"required "}else{""},if part.wire.required(){part.native_type.clone()}else{format!("Optional<{}>",part.native_type)},part.property_name));
    }
    if let Some(items) = &parts.items {
        out.push_str(&format!("    /// <summary>Ordered items after the complete prefix. Count and each current payload are validated before transport.</summary>\n    public List<{}> Items {{ get; set; }} = new();\n",items.native_type));
    }
    out.push_str("}\n\n");
    for part in parts
        .prefix
        .iter()
        .chain(parts.items.iter().map(|p| p.as_ref()))
    {
        http::part_decl(out, part);
    }
}

pub(super) fn codecs(plan: &SdkPlan, out: &mut String, parts: &PlannedPositionalParts) {
    for part in parts
        .prefix
        .iter()
        .chain(parts.items.iter().map(|p| p.as_ref()))
    {
        if let Some(name) = &part.header_type {
            http::headers_codecs(plan, out, name, &part.headers);
        }
    }
    out.push_str(&format!("    internal static EncodedBody Write{}(JsonElement media, int index, {} value, string? contentType)\n    {{\n        if (value is null) throw new SdkException(SdkErrorKind.RequestValidation);\n        var plan = media.GetProperty(\"representation\").GetProperty(\"multipart\");\n        var prefix = PositionalRuntime.PrefixCount(new bool[] {{ {} }});\n",parts.native_type,parts.native_type,parts.prefix.iter().map(|p|if p.wire.required(){"true".into()}else{format!("value.{}.HasValue",p.property_name)}).collect::<Vec<String>>().join(", ")));
    if parts.items.is_some() {
        out.push_str(&format!("        if (value.Items is null) throw new SdkException(SdkErrorKind.RequestValidation);\n        if (value.Items.Count != 0 && prefix != {}) throw new SdkException(SdkErrorKind.RequestRepresentation);\n        if (value.Items.Count > 1024 - prefix) throw new SdkException(SdkErrorKind.ResourceLimit);\n        PositionalRuntime.Count(plan, prefix + value.Items.Count);\n",parts.prefix.len()));
    } else {
        out.push_str("        PositionalRuntime.Count(plan, prefix);\n");
    }
    out.push_str("        var values = new List<RawPart>();\n");
    for (index, part) in parts.prefix.iter().enumerate() {
        if !part.wire.required() {
            out.push_str(&format!(
                "        if (value.{}.HasValue)\n        {{\n",
                part.property_name
            ));
        }
        let value = format!(
            "value.{}{}",
            part.property_name,
            if part.wire.required() { "" } else { ".Value" }
        );
        out.push_str(&format!("        if ({value} is null) throw new SdkException(SdkErrorKind.RequestValidation);\n        var part{index} = PositionalRuntime.Part(plan, {index});\n        PositionalRuntime.Add(plan, values, {});\n",http::part_encode(plan,part,&value,&format!("part{index}"),"string.Empty")));
        if !part.wire.required() {
            out.push_str("        }\n");
        }
    }
    if let Some(items) = &parts.items {
        out.push_str("        foreach (var item in value.Items)\n        {\n            if (item is null) throw new SdkException(SdkErrorKind.RequestValidation);\n            var part = PositionalRuntime.Part(plan, values.Count);\n");
        out.push_str(&format!(
            "            PositionalRuntime.Add(plan, values, {});\n        }}\n",
            http::part_encode(plan, items, "item", "part", "string.Empty")
        ));
    }
    out.push_str(
        "        return PositionalRuntime.Encode(media, index, values, contentType);\n    }\n",
    );
    out.push_str(&format!("    internal static {} Read{}(JsonElement media, List<RawPart> parts)\n    {{\n        var plan = media.GetProperty(\"representation\").GetProperty(\"multipart\");\n        PositionalRuntime.Count(plan, parts.Count);\n        var value = new {}\n        {{\n",parts.native_type,parts.native_type,parts.native_type));
    for (index, part) in parts.prefix.iter().enumerate() {
        let decoded = http::part_decode(
            plan,
            part,
            &format!("PositionalRuntime.Part(plan, {index})"),
            &format!("parts[{index}]"),
            true,
        );
        out.push_str(&format!(
            "            {} = {},\n",
            part.property_name,
            if part.wire.required() {
                decoded
            } else {
                format!(
                    "parts.Count > {index} ? Optional<{}>.Present({decoded}) : default",
                    part.native_type
                )
            }
        ));
    }
    out.push_str("        };\n");
    if let Some(items) = &parts.items {
        out.push_str(&format!(
            "        for (var i = {}; i < parts.Count; i++)\n            value.Items.Add({});\n",
            parts.prefix.len(),
            http::part_decode(
                plan,
                items,
                "PositionalRuntime.Part(plan, i)",
                "parts[i]",
                true
            )
        ));
    }
    out.push_str("        return value;\n    }\n");
}
