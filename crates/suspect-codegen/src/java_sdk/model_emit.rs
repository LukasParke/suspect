//! Java source from retained native declarations. All conversions share the
//! caller's context; public builders snapshot through one complete codec call.

use super::models::{JavaDeclaration, JavaField, JavaModelPlan, JavaSymbol, JavaType, javadoc, q};

pub(crate) fn symbol(plan: &JavaModelPlan, symbol: &JavaSymbol) -> String {
    let name = symbol.name();
    let native = &symbol.codec().native_type;
    let root = symbol.codec().root;
    let union = matches!(symbol.declaration(), JavaDeclaration::Union { .. });
    let mut out = format!(
        "/**\n * {}\n * <p>Source: <code>{}#{}</code>.\n * <p>{}\n */\npublic {} {name} {{\n",
        javadoc(symbol.description()),
        javadoc(symbol.source().document().as_str()),
        javadoc(symbol.source().pointer()),
        if symbol.nullable() {
            "The source permits JSON null; see the native codec type's null representation."
        } else {
            "JSON null is rejected by the source-bound codec."
        },
        if union {
            "sealed interface"
        } else {
            "final class"
        }
    );
    out.push_str(&format!("    /** Source-bound checked codec. Encode, decode and snapshot share bounded sessions. */\n    public static final ModelCodec<{native}> CODEC = new ModelCodec<>({root}, {name}::read, {name}::write);\n    /** Decode exact UTF-8 JSON represented by a Java string.\n     * @param json complete JSON document\n     * @return validated native value\n     * @throws CodecException if the document, value or resource budget is invalid\n     */\n    public static {native} decode(String json) {{ return CODEC.decode(json); }}\n    /** Encode and validate a native value without numeric rounding.\n     * @param value native value\n     * @return exact JSON document\n     * @throws CodecException if the value or resource budget is invalid\n     */\n    public static String encode({native} value) {{ return CODEC.encode(value); }}\n"));
    match symbol.declaration() {
        JavaDeclaration::Alias(ty) => {
            out.push_str(&format!("    private {name}() {{}}\n"));
            methods(
                &mut out,
                symbol,
                &expression(plan, ty, "_value", false, 0),
                &expression(plan, ty, "_value", true, 0),
                false,
            );
        }
        JavaDeclaration::Literals { values } => {
            out.push_str(&format!("    private final JsonValue _wireValue;\n    private {name}(JsonValue value) {{ this._wireValue = value; }}\n    /** Exact immutable wire value, including the decoded numeric spelling.\n     * @return wire value\n     */\n    public JsonValue wireValue() {{ return _wireValue; }}\n"));
            for literal in values {
                out.push_str(&format!("    /** Source literal <code>{}</code>. */\n    public static final {name} {} = new {name}(JsonRuntime.parse({}));\n", javadoc(&literal.value.to_string()), literal.name, q(&literal.value.to_string())));
            }
            out.push_str(&format!("    @Override public boolean equals(Object other) {{ return other instanceof {name} value && _wireValue.equals(value._wireValue); }}\n    @Override public int hashCode() {{ return _wireValue.hashCode(); }}\n    @Override public String toString() {{ return {}; }}\n", q(name)));
            methods(
                &mut out,
                symbol,
                &format!("new {name}(_c.json(_value))"),
                "_c.json(_c.require(_value)._wireValue)",
                true,
            );
        }
        JavaDeclaration::Union {
            variants,
            exclusive,
        } => {
            for variant in variants {
                let ty = plan.render_type(&variant.ty);
                let codec = plan.codec(&variant.source);
                out.push_str(&format!("    /** Typed {} arm; its payload is validated and deeply snapshotted on construction.\n     * Source: <code>{}#{}</code>.\n     */\n    final class {} implements {name} {{\n        private final {ty} _value;\n        /** Construct a checked, immutable arm.\n         * @param value source-valid arm payload\n         * @throws CodecException if the selected arm rejects the payload\n         */\n        public {}({ty} value) {{ this._value = {}.CODEC.snapshot(value); }}\n        private {}({ty} value, ModelCodec.Context checked) {{ this._value = value; }}\n        /** The immutable typed payload.\n         * @return selected arm value\n         */\n        public {ty} value() {{ return _value; }}\n        @Override public boolean equals(Object other) {{ return other instanceof {} arm && java.util.Objects.equals(_value, arm._value); }}\n        @Override public int hashCode() {{ return java.util.Objects.hashCode(_value); }}\n        @Override public String toString() {{ return {}; }}\n    }}\n", if *exclusive { "oneOf" } else { "anyOf" }, javadoc(variant.source.document().as_str()), javadoc(variant.source.pointer()), variant.name, variant.name, codec.holder, variant.name, variant.name, q(&format!("{name}.{}", variant.name))));
            }
            out.push_str(&format!("    private static {name} read(JsonValue _value, ModelCodec.Context _c) {{\n        _c.enter({root}, _value);\n        try {{\n"));
            for variant in variants {
                out.push_str(&format!(
                    "            if (_c.matches({}, _value)) return new {}({}, _c);\n",
                    plan.codec(&variant.source).root,
                    variant.name,
                    expression(plan, &variant.ty, "_value", false, 0)
                ));
            }
            out.push_str("            throw _c.invalid(\"no union arm matched\");\n        } finally { _c.leave(); }\n    }\n");
            out.push_str(&format!("    private static JsonValue write({name} _value, ModelCodec.Context _c) {{\n        _c.enter({root}, _value);\n        try {{\n"));
            for variant in variants {
                out.push_str(&format!("            if (_value instanceof {} _arm) {{\n                JsonValue _json = {};\n                _c.check({}, _json);\n                return _json;\n            }}\n", variant.name, expression(plan, &variant.ty, "_arm.value()", true, 0), plan.codec(&variant.source).root));
            }
            out.push_str("            throw _c.invalid(\"missing native union arm\");\n        } finally { _c.leave(); }\n    }\n");
        }
        JavaDeclaration::Object {
            fields,
            extras,
            constructor,
        } => {
            if extras.is_some() {
                out.push_str(&format!(
                    "    private static final java.util.Set<String> _DECLARED_PROPERTIES = {};\n",
                    declared(fields)
                ));
            }
            for field in fields {
                let ty = field_type(plan, field);
                out.push_str(&format!("    private final {ty} {};\n    /** {}\n     * <p>Wire property <code>{}</code>; {}{}.\n     * Source: <code>{}#{}</code>.\n     * @return {}\n     */\n    public {ty} {}() {{ return this.{}; }}\n", field.name, javadoc(&field.description), javadoc(&field.wire), if field.required { "required" } else { "optional; absence is explicit" }, if field.nullable { "; JSON null permitted" } else { "; JSON null rejected" }, javadoc(field.source.document().as_str()), javadoc(field.source.pointer()), if field.required { "immutable field value" } else { "presence and immutable value" }, field.name, field.name));
            }
            if let Some(extra) = extras {
                let ty = plan.render_type(extra);
                out.push_str(&format!("    private final java.util.Map<String, {ty}> additionalProperties;\n    /** Immutable undeclared properties, including every nested value.\n     * @return unmodifiable additional properties\n     */\n    public java.util.Map<String, {ty}> additionalProperties() {{ return additionalProperties; }}\n"));
            }
            out.push_str(&format!("    private {name}(Builder _builder) {{\n"));
            for field in fields {
                out.push_str(&format!(
                    "        this.{} = _builder.{};\n",
                    field.name, field.name
                ));
            }
            if extras.is_some() {
                out.push_str("        this.additionalProperties = java.util.Collections.unmodifiableMap(_builder.additionalProperties);\n");
            }
            out.push_str(
                "    }\n    /** Start a builder with every required non-singleton field.\n",
            );
            for argument in &constructor.arguments {
                out.push_str(&format!(
                    "     * @param {} required field{}\n",
                    argument.name,
                    if argument.nullable {
                        ", may represent JSON null"
                    } else {
                        ", JSON null rejected"
                    }
                ));
            }
            out.push_str(&format!("     * @return mutable builder; build validates and snapshots\n     */\n    public static Builder builder({}) {{\n        Builder _builder = new Builder();\n", constructor.arguments.iter().map(|a| format!("{} {}", plan.render_type(&a.ty), a.name)).collect::<Vec<_>>().join(", ")));
            for argument in &constructor.arguments {
                out.push_str(&format!(
                    "        _builder.{} = {};\n",
                    argument.name, argument.name
                ));
            }
            out.push_str("        return _builder;\n    }\n    /** Mutable construction state. Each build produces an independent immutable value. */\n    public static final class Builder {\n        private Builder() {}\n");
            for field in fields {
                let initial = if let Some(value) = &field.fixed {
                    format!(" = {}", fixed(plan, &field.source, value))
                } else if field.required {
                    String::new()
                } else {
                    " = Presence.absent()".into()
                };
                out.push_str(&format!(
                    "        private {} {}{initial};\n",
                    field_type(plan, field),
                    field.name
                ));
                if field.fixed.is_none() {
                    out.push_str(&format!("        /** Supply wire property <code>{}</code>{}.\n         * @param value field value\n         * @return this builder\n         */\n        public Builder {}({} value) {{ this.{} = {}; return this; }}\n", javadoc(&field.wire), if field.nullable { "; nullability is checked by its codec" } else { "; JSON null is rejected" }, field.name, plan.render_type(&field.ty), field.name, if field.required { "value" } else { "Presence.of(value)" }));
                    if let Some(omit) = &field.omit_method {
                        out.push_str(&format!("        /** Restore absence for <code>{}</code>.\n         * @return this builder\n         */\n        public Builder {omit}() {{ this.{} = Presence.absent(); return this; }}\n", javadoc(&field.wire), field.name));
                    }
                }
            }
            if let Some(extra) = extras {
                let ty = plan.render_type(extra);
                out.push_str(&format!("        private final java.util.Map<String, {ty}> additionalProperties = new java.util.LinkedHashMap<>();\n        /** Supply one undeclared property. Declared wire names cannot be overwritten.\n         * @param key exact wire name\n         * @param value additional value\n         * @return this builder\n         */\n        public Builder putAdditionalProperty(String key, {ty} value) {{\n            JsonRuntime.unicode(key);\n            if (_DECLARED_PROPERTIES.contains(key)) throw new IllegalArgumentException(\"declared property cannot be an extra\");\n            additionalProperties.put(key, value); return this;\n        }}\n"));
            }
            out.push_str(&format!("        /** Validate and deeply snapshot under one shared codec budget.\n         * @return independent immutable model\n         * @throws CodecException if a value or budget is invalid\n         */\n        public {name} build() {{ return CODEC.snapshot(new {name}(this)); }}\n    }}\n"));
            out.push_str(&format!("    private static {name} read(JsonValue _value, ModelCodec.Context _c) {{\n        _c.enter({root}, _value);\n        try {{\n"));
            if symbol.nullable() {
                out.push_str("            if (_value == JsonNull.INSTANCE) return null;\n");
            }
            out.push_str("            var _values = ((JsonObject) _value).values();\n            Builder _builder = new Builder();\n");
            for field in fields {
                let wire = q(&field.wire);
                let read = format!(
                    "_c.at({wire}, () -> {})",
                    expression(plan, &field.ty, &format!("_values.get({wire})"), false, 0)
                );
                out.push_str(&format!(
                    "            if (_values.containsKey({wire})) _builder.{} = {};\n",
                    field.name,
                    if field.required {
                        read
                    } else {
                        format!("Presence.of({read})")
                    }
                ));
            }
            if let Some(extra) = extras {
                out.push_str(&format!("            for (var _entry : _values.entrySet()) {{\n                _c.spend(1);\n                if (!_DECLARED_PROPERTIES.contains(_entry.getKey())) {{\n                    String _key = _c.string(_entry.getKey());\n                    _builder.additionalProperties.put(_key, _c.at(_key, () -> {}));\n                }}\n            }}\n", expression(plan, extra, "_entry.getValue()", false, 0)));
            }
            out.push_str(&format!("            return new {name}(_builder);\n        }} finally {{ _c.leave(); }}\n    }}\n    private static JsonValue write({name} _value, ModelCodec.Context _c) {{\n        _c.enter({root}, _value);\n        try {{\n"));
            if symbol.nullable() {
                out.push_str("            if (_value == null) return JsonNull.INSTANCE;\n");
            }
            out.push_str("            _c.require(_value);\n            var _values = new java.util.LinkedHashMap<String, JsonValue>();\n");
            for field in fields {
                let access = format!(
                    "_value.{}{}",
                    field.name,
                    if field.required { "" } else { ".value()" }
                );
                let wire = q(&field.wire);
                let prefix = if field.required {
                    String::new()
                } else {
                    format!("if (_value.{}.isPresent()) ", field.name)
                };
                out.push_str(&format!("            {prefix}_values.put(_c.string({wire}), _c.at({wire}, () -> {}));\n", expression(plan, &field.ty, &access, true, 0)));
            }
            if let Some(extra) = extras {
                out.push_str(&format!("            for (var _entry : _value.additionalProperties.entrySet()) {{\n                _c.spend(1);\n                String _key = _c.string(_entry.getKey());\n                if (_DECLARED_PROPERTIES.contains(_key)) throw _c.invalid(\"declared property cannot be an extra\");\n                _values.put(_key, _c.at(_key, () -> {}));\n            }}\n", expression(plan, extra, "_entry.getValue()", true, 0)));
            }
            out.push_str("            return new JsonObject(_values);\n        } finally { _c.leave(); }\n    }\n");
            let mut equality: Vec<_> = fields
                .iter()
                .map(|f| format!("java.util.Objects.equals(this.{0}, _other.{0})", f.name))
                .collect();
            let mut hashes: Vec<_> = fields.iter().map(|f| format!("this.{}", f.name)).collect();
            if extras.is_some() {
                equality.push("additionalProperties.equals(_other.additionalProperties)".into());
                hashes.push("additionalProperties".into());
            }
            out.push_str(&format!("    @Override public boolean equals(Object _value) {{ return _value instanceof {name} _other{}; }}\n    @Override public int hashCode() {{ return java.util.Objects.hash(new Object[] {{{}}}); }}\n    @Override public String toString() {{ return {}; }}\n", if equality.is_empty() { String::new() } else { format!(" && {}", equality.join(" && ")) }, hashes.join(", "), q(name)));
        }
    }
    out.push_str("}\n");
    out
}

fn methods(out: &mut String, symbol: &JavaSymbol, read: &str, write: &str, nominal_nullable: bool) {
    let native = &symbol.codec().native_type;
    let root = symbol.codec().root;
    out.push_str(&format!("    private static {native} read(JsonValue _value, ModelCodec.Context _c) {{\n        _c.enter({root}, _value);\n        try {{\n"));
    if nominal_nullable && symbol.nullable() {
        out.push_str("            if (_value == JsonNull.INSTANCE) return null;\n");
    }
    out.push_str(&format!("            return {read};\n        }} finally {{ _c.leave(); }}\n    }}\n    private static JsonValue write({native} _value, ModelCodec.Context _c) {{\n        _c.enter({root}, _value);\n        try {{\n"));
    if nominal_nullable && symbol.nullable() {
        out.push_str("            if (_value == null) return JsonNull.INSTANCE;\n");
    }
    out.push_str(&format!(
        "            return {write};\n        }} finally {{ _c.leave(); }}\n    }}\n"
    ));
}

fn field_type(plan: &JavaModelPlan, field: &JavaField) -> String {
    let ty = plan.render_type(&field.ty);
    if field.required {
        ty
    } else {
        format!("Presence<{ty}>")
    }
}
fn declared(fields: &[JavaField]) -> String {
    format!(
        "java.util.Set.<String>of({})",
        fields
            .iter()
            .map(|f| q(&f.wire))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn fixed(
    plan: &JavaModelPlan,
    id: &suspect_ir::contract::SchemaId,
    value: &serde_json::Value,
) -> String {
    match plan.symbol(id).expect("field declaration").declaration() {
        JavaDeclaration::Literals { values } if !value.is_null() => format!(
            "{}.{}",
            plan.names()[id],
            values
                .iter()
                .find(|v| &v.value == value)
                .expect("retained fixed value")
                .name
        ),
        JavaDeclaration::Alias(JavaType::Null) => "JsonNull.INSTANCE".into(),
        _ if value.is_null() => "null".into(),
        _ => format!("{}.decode({})", plan.names()[id], q(&value.to_string())),
    }
}

pub(crate) fn expression(
    plan: &JavaModelPlan,
    ty: &JavaType,
    value: &str,
    encode: bool,
    depth: usize,
) -> String {
    match ty {
        JavaType::Named(id) => format!(
            "{}.CODEC.{}({value}, _c)",
            plan.names()[id],
            if encode { "write" } else { "read" }
        ),
        JavaType::Nullable(inner) => format!(
            "({value} == {} ? {} : {})",
            if encode { "null" } else { "JsonNull.INSTANCE" },
            if encode { "JsonNull.INSTANCE" } else { "null" },
            expression(plan, inner, value, encode, depth + 1)
        ),
        JavaType::List(inner) => {
            let item = format!("_item{depth}");
            format!(
                "_c.{}List({value}, {item} -> {})",
                if encode { "write" } else { "read" },
                expression(plan, inner, &item, encode, depth + 1)
            )
        }
        JavaType::String => {
            if encode {
                format!("new JsonString(_c.string({value}))")
            } else {
                format!("_c.string(((JsonString) {value}).value())")
            }
        }
        JavaType::Boolean => {
            if encode {
                format!("new JsonBoolean(_c.require({value}))")
            } else {
                format!("((JsonBoolean) {value}).value()")
            }
        }
        JavaType::Number => format!(
            "_c.number({}{value})",
            if encode { "" } else { "(JsonNumber) " }
        ),
        JavaType::Json => format!("_c.json({value})"),
        JavaType::Null => format!("_c.nullValue({value})"),
        JavaType::Never => "_c.never()".into(),
    }
}
