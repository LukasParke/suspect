use super::{
    HttpDiagnostic, Plan,
    models::{Additional, ModelPlan, Shape, Symbol},
};
use crate::OutFile;
use std::fmt::Write;
use suspect_ir::contract::SourceId;

pub(super) fn package(plan: &Plan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    super::rich_emit::package(plan)
}
pub(super) fn header(plan: &Plan) -> String {
    format!(
        "@file:Suppress(\"DEPRECATION\", \"UNUSED_PARAMETER\", \"UNREACHABLE_CODE\")\n\npackage {}\n\n",
        plan.config.package_name
    )
}
pub(super) fn quote(value: &str) -> String {
    let literal = |value: &str| {
        let mut out = String::from("\"");
        for c in value.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '$' => out.push_str("\\$"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => {
                    write!(out, "\\u{:04x}", u32::from(c)).unwrap();
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    };
    if value.len() <= 8192 {
        return literal(value);
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < value.len() {
        let mut end = (start + 8192).min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(format!("append({})", literal(&value[start..end])));
        start = end;
    }
    format!("buildString {{ {} }}", chunks.join("; "))
}
pub(super) fn kdoc(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("/*", "/&#42;")
        .replace("*/", "&#42;/")
        .replace('@', "&#64;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('`', "&#96;")
}
pub(super) fn source(id: &SourceId) -> String {
    let mut pointer = String::new();
    for byte in id.pointer().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            pointer.push(char::from(byte));
        } else {
            write!(pointer, "%{byte:02X}").unwrap();
        }
    }
    format!("[source](<{}#{pointer}>)", id.document())
}
pub(super) fn source_value(id: &SourceId) -> String {
    format!(
        "SourceLocation({}, {})",
        quote(id.document().as_str()),
        quote(id.pointer())
    )
}

pub(super) fn models(plan: &Plan) -> String {
    let mut out = header(plan);
    for s in plan.models.symbols() {
        let raw = plan.contract.source(&s.source).unwrap();
        let description = raw
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Source-defined native model.");
        if matches!(
            s.shape,
            Shape::Object { .. } | Shape::StringEnum(_) | Shape::Union { .. } | Shape::CheckedJson
        ) {
            writeln!(
                out,
                "/** {} Source: {} */",
                kdoc(&if matches!(s.shape,Shape::CheckedJson){format!("{description} Schema-bound exact JSON carrier; use its codec to validate model-only values.")}else{description.into()}),
                source(&s.source)
            )
            .unwrap();
            if raw.get("deprecated").and_then(serde_json::Value::as_bool) == Some(true) {
                out.push_str("@Deprecated(\"Deprecated in the source OpenAPI contract\")\n");
            }
        }
        match &s.shape {
            Shape::CheckedJson => {
                writeln!(out,"public data class {}(\n    /** Complete source value, retaining every property and item. */ public val value: JsonValue,\n) {{\n    /** Source-bound codec. */ public companion object {{\n        /** Validates the complete scoped schema on encode and decode. */ public val codec: ModelCodec<{}> get() = Codecs.{}\n    }}\n}}",s.name,s.kotlin_type,s.codec_name).unwrap();
            }
            Shape::Object { fields, additional } => {
                let data = fields.iter().any(|f| f.constant.is_none())
                    || !matches!(additional, Additional::Closed);
                writeln!(
                    out,
                    "public {} {}(",
                    if data { "data class" } else { "class" },
                    s.name
                )
                .unwrap();
                for f in fields.iter().filter(|f| f.constant.is_none()) {
                    writeln!(
                        out,
                        "    /** Wire &#96;{}&#96;. {} Source: {} */\n    public val {}: {}{},",
                        kdoc(&f.wire_name),
                        if f.required {
                            "Required."
                        } else {
                            "Optional; Absent omits the member."
                        },
                        source(&f.schema),
                        f.name,
                        f.kotlin_type,
                        if f.required { "" } else { " = Presence.Absent" }
                    )
                    .unwrap();
                }
                match additional{Additional::Closed=>{},Additional::Any=>out.push_str("    /** Source-permitted extras; declared keys are reserved. */\n    public val additionalProperties: Map<String, JsonValue> = emptyMap(),\n"),Additional::Scoped=>out.push_str("    /** Retained exact extras; pattern and scoped source constraints are checked by the complete codec. */\n    public val additionalProperties: Map<String, JsonValue> = emptyMap(),\n"),Additional::Typed(id)=>{writeln!(out,"    /** Source-typed extras; declared keys are reserved. */\n    public val additionalProperties: Map<String, {}> = emptyMap(),",plan.models.get(id).kotlin_type).unwrap();}}
                out.push_str(") {\n");
                for f in fields {
                    if let Some(value) = &f.constant {
                        writeln!(out,"    /** Fixed source tag for wire &#96;{}&#96;. */\n    public val {}: {} get() = {value}",kdoc(&f.wire_name),f.name,f.kotlin_type).unwrap();
                    }
                }
                writeln!(out,"    /** Source-bound codec. */\n    public companion object {{\n        /** Validates the current value on encode and decode. */\n        public val codec: ModelCodec<{}> get() = Codecs.{}\n    }}\n}}",s.kotlin_type,s.codec_name).unwrap();
            }
            Shape::StringEnum(cases) => {
                writeln!(out,"public enum class {}(/** Exact wire value. */ public val wireValue: String) {{",s.name).unwrap();
                for (i, (name, value)) in cases.iter().enumerate() {
                    writeln!(
                        out,
                        "    /** Wire &#96;{}&#96;. */\n    {name}({}){}",
                        kdoc(value),
                        quote(value),
                        if i + 1 == cases.len() { ";" } else { "," }
                    )
                    .unwrap();
                }
                out.push_str("}\n");
            }
            Shape::Union { variants, .. } => {
                writeln!(out, "public sealed interface {} {{", s.name).unwrap();
                for (name, id) in variants {
                    writeln!(out,"    /** Source union arm. */\n    public data class {name}(/** Selected-arm payload. */ public val value: {}) : {}",plan.models.get(id).kotlin_type,s.name).unwrap();
                }
                out.push_str("}\n");
            }
            _ => {}
        }
    }
    out
}

pub(super) fn codecs(plan: &Plan) -> String {
    codec_group(plan, plan.models.symbols(), "public object Codecs")
}

pub(super) fn codec_group(plan: &Plan, symbols: &[Symbol], declaration: &str) -> String {
    let mut out = header(plan);
    out.push_str(&format!(
        "/** Source-validating codecs for actual JSON/text roots. */\n{declaration} {{\n"
    ));
    for s in symbols {
        writeln!(out,"    /** Source: {} */\n    public val {}: ModelCodec<{}> get() = ModelCodec({}, source{}(), ::read{}, ::write{})",source(&s.source),s.codec_name,s.kotlin_type,s.index,s.index,s.index,s.index).unwrap();
    }
    out.push_str("}\n");
    for s in symbols {
        writeln!(
            out,
            "internal fun source{}(): SourceLocation = {}",
            s.index,
            source_value(&s.source)
        )
        .unwrap();
        writeln!(out,"internal fun read{}(json: JsonValue, budget: ModelBudget, path: String): {} = budget.at(source{}(), path) {{",s.index,s.kotlin_type,s.index).unwrap();
        if s.nullable {
            out.push_str("    if (json === JsonNull) null else {\n");
        }
        out.push_str(&read_shape(&plan.models, s));
        if s.nullable {
            out.push_str("    }\n");
        }
        out.push_str("}\n");
        writeln!(out,"internal fun write{}(value: {}, budget: ModelBudget, path: String): JsonValue = budget.at(source{}(), path) {{",s.index,s.kotlin_type,s.index).unwrap();
        if s.nullable {
            out.push_str("    if (value == null) JsonNull else {\n");
        }
        out.push_str(&write_shape(&plan.models, s));
        if s.nullable {
            out.push_str("    }\n");
        }
        out.push_str("}\n");
    }
    out
}
fn read_shape(m: &ModelPlan, s: &Symbol) -> String {
    match &s.shape {
        Shape::Any => format!("    budget.json(json, source{}(), path)\n", s.index),
        Shape::CheckedJson => format!(
            "    {}(budget.json(json, source{}(), path))\n",
            s.name, s.index
        ),
        Shape::Never => format!(
            "    throw ValidationException(listOf(ValidationFinding(source{}(), path, \"false schema has no value\")))\n",
            s.index
        ),
        Shape::Null => "    null\n".into(),
        Shape::Boolean => "    (json as JsonBoolean).value\n".into(),
        Shape::String => format!(
            "    budget.text((json as JsonString).value, source{}(), path)\n",
            s.index
        ),
        Shape::Number => format!(
            "    (json as JsonNumber).also {{ budget.text(it.token, source{}(), path) }}\n",
            s.index
        ),
        Shape::Alias(id) => format!("    read{}(json, budget, path)\n", m.get(id).index),
        Shape::Array(None) => format!(
            "    (budget.json(json, source{}(), path) as JsonArray).values\n",
            s.index
        ),
        Shape::Array(Some(id)) => format!(
            "    val items = (json as JsonArray).values\n    budget.collection(items.size, source{}(), path)\n    items.mapIndexed {{ index, item -> read{}(item, budget, childPath(path, index.toString())) }}\n",
            s.index,
            m.get(id).index
        ),
        Shape::StringEnum(_) => format!(
            "    {}.entries.first {{ it.wireValue == (json as JsonString).value }}\n",
            s.name
        ),
        Shape::Object { fields, additional } => {
            let mut out = format!(
                "    val obj = (json as JsonObject).values\n    for (key in obj.keys) budget.text(key, source{}(), path)\n    {}(\n",
                s.index, s.name
            );
            for f in fields.iter().filter(|f| f.constant.is_none()) {
                let key = quote(&f.wire_name);
                let value = format!(
                    "read{}(obj.getValue({key}), budget, childPath(path, {key}))",
                    m.get(&f.schema).index
                );
                writeln!(out,"        {} = {},",f.name,if f.required{value}else{format!("if (obj.containsKey({key})) Presence.Present({value}) else Presence.Absent")}).unwrap();
            }
            if !matches!(additional, Additional::Closed) {
                let keys = fields
                    .iter()
                    .map(|f| quote(&f.wire_name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let value = match additional {
                    Additional::Typed(id) => format!(
                        "read{}(entry.value, budget, childPath(path, entry.key))",
                        m.get(id).index
                    ),
                    _ => format!(
                        "budget.json(entry.value, source{}(), childPath(path, entry.key))",
                        s.index
                    ),
                };
                writeln!(out,"        additionalProperties = obj.filterKeys {{ it !in setOf<String>({keys}) }}.mapValues {{ entry -> {value} }},").unwrap();
            }
            out.push_str("    )\n");
            out
        }
        Shape::Union { variants, .. } => {
            let mut out = "    when {\n".to_owned();
            for (name, id) in variants {
                writeln!(out,"        budget.validation.matches({}, json, path) -> {}.{name}(read{}(json, budget, path))",m.get(id).index,s.name,m.get(id).index).unwrap();
            }
            writeln!(out,"        else -> throw ValidationException(listOf(ValidationFinding(source{}(), path, \"no native union arm\")))\n    }}",s.index).unwrap();
            out
        }
    }
}
fn write_shape(m: &ModelPlan, s: &Symbol) -> String {
    match &s.shape {
        Shape::CheckedJson => format!("    budget.json(value.value, source{}(), path)\n", s.index),
        Shape::Any | Shape::Number => {
            format!("    budget.json(value, source{}(), path)\n", s.index)
        }
        Shape::Never => format!(
            "    throw ValidationException(listOf(ValidationFinding(source{}(), path, \"false schema has no value\")))\n",
            s.index
        ),
        Shape::Null => "    JsonNull\n".into(),
        Shape::Boolean => "    JsonBoolean(value)\n".into(),
        Shape::String => format!(
            "    JsonString(budget.text(value, source{}(), path))\n",
            s.index
        ),
        Shape::Alias(id) => format!("    write{}(value, budget, path)\n", m.get(id).index),
        Shape::Array(None) => format!(
            "    budget.json(JsonArray(value), source{}(), path)\n",
            s.index
        ),
        Shape::Array(Some(id)) => format!(
            "    budget.collection(value.size, source{}(), path)\n    JsonArray(value.mapIndexed {{ index, item -> write{}(item, budget, childPath(path, index.toString())) }})\n",
            s.index,
            m.get(id).index
        ),
        Shape::StringEnum(_) => format!(
            "    JsonString(budget.text(value.wireValue, source{}(), path))\n",
            s.index
        ),
        Shape::Object { fields, additional } => {
            let mut out = "    val obj = linkedMapOf<String, JsonValue>()\n".to_owned();
            for f in fields {
                let key = quote(&f.wire_name);
                let index = m.get(&f.schema).index;
                writeln!(out, "    budget.text({key}, source{}(), path)", s.index).unwrap();
                if f.required {
                    writeln!(
                        out,
                        "    obj[{key}] = write{index}(value.{}, budget, childPath(path, {key}))",
                        f.name
                    )
                    .unwrap();
                } else {
                    writeln!(out,"    when (val member = value.{}) {{\n        Presence.Absent -> Unit\n        is Presence.Present -> obj[{key}] = write{index}(member.value, budget, childPath(path, {key}))\n    }}",f.name).unwrap();
                }
            }
            if !matches!(additional, Additional::Closed) {
                let keys = fields
                    .iter()
                    .map(|f| quote(&f.wire_name))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(out,"    for ((key, member) in value.additionalProperties) {{\n        budget.text(key, source{}(), path)\n        if (key in setOf<String>({keys})) throw ValidationException(listOf(ValidationFinding(source{}(), childPath(path, key), \"extra field collides with declared key\")))",s.index,s.index).unwrap();
                let value = match additional {
                    Additional::Typed(id) => format!(
                        "write{}(member, budget, childPath(path, key))",
                        m.get(id).index
                    ),
                    _ => format!(
                        "budget.json(member, source{}(), childPath(path, key))",
                        s.index
                    ),
                };
                writeln!(out, "        obj[key] = {value}\n    }}").unwrap();
            }
            out.push_str("    JsonObject(obj)\n");
            out
        }
        Shape::Union { variants, .. } => {
            let mut out = "    when (value) {\n".to_owned();
            for (name, id) in variants {
                let i = m.get(id).index;
                writeln!(out,"        is {}.{name} -> write{i}(value.value, budget, path).also {{ budget.validation.validate({i}, it, path) }}",s.name).unwrap();
            }
            out.push_str("    }\n");
            out
        }
    }
}
