//! Native non-JSON payload and header construction from retained protocol bindings.
use super::{
    SdkPlan,
    models::{javadoc, q},
    protocol::{AggregateRules, JavaAggregate, JavaHeaders, JavaMedia, JavaPart, JavaValue},
};
use crate::OutFile;
use std::collections::BTreeMap;

pub(crate) fn codec(plan: &SdkPlan, value: &JavaValue) -> String {
    match value {
        JavaValue::Model(id) => {
            format!("WireCodec.model({}.CODEC)", plan.models().codec(id).holder)
        }
        JavaValue::Json => "WireCodec.json()".into(),
        JavaValue::Text => "WireCodec.text()".into(),
        JavaValue::Bytes => "WireCodec.bytes()".into(),
        JavaValue::Named(name) => format!("{name}.WIRE_CODEC"),
        JavaValue::Aggregate(a) => format!("{}.WIRE_CODEC", a.name),
        JavaValue::Stream {
            schema,
            request: true,
        } => format!(
            "WireCodec.items({}.CODEC)",
            plan.models().codec(schema).holder
        ),
        _ => unreachable!("response-only value has no request/snapshot codec"),
    }
}
pub(crate) fn read(plan: &SdkPlan, value: &JavaValue, expression: &str, context: &str) -> String {
    format!("{}.read({expression}, {context})", codec(plan, value))
}
pub(crate) fn write(plan: &SdkPlan, value: &JavaValue, expression: &str, context: &str) -> String {
    format!("{}.write({expression}, {context})", codec(plan, value))
}
pub(crate) fn snapshot(
    plan: &SdkPlan,
    value: &JavaValue,
    expression: &str,
    context: Option<&str>,
) -> String {
    format!(
        "{}.snapshot({expression}{})",
        codec(plan, value),
        context.map_or(String::new(), |c| format!(", {c}"))
    )
}

pub(crate) fn response_value(plan: &SdkPlan, media: &JavaMedia) -> String {
    let spec = format!("Protocol.object({})", q(&media.descriptor));
    match &media.value {
        JavaValue::Stream { schema, .. } => format!(
            "raw.stream({spec}, {}.CODEC)",
            plan.models().codec(schema).holder
        ),
        value => format!("raw.decode({spec}, {})", codec(plan, value)),
    }
}

pub(crate) fn files(plan: &SdkPlan) -> Vec<OutFile> {
    let prefix = format!(
        "java/src/main/java/{}",
        plan.package().package.replace('.', "/")
    );
    let header = format!(
        "package {};\nimport static {}.JsonRuntime.*;\n\n",
        plan.package().package,
        plan.package().package
    );
    let mut values = BTreeMap::new();
    for (_, group) in super::protocol::header_groups(plan.operations()) {
        values.insert(group.name.clone(), headers(plan, &group));
    }
    for op in plan.operations() {
        if let Some(body) = &op.body {
            if let Some(name) = &body.choice_type {
                values.insert(name.clone(), choice(plan, name, &body.media, true));
            }
            for media in &body.media {
                collect(plan, &media.value, &mut values);
            }
        }
        for response in &op.responses {
            if let Some(name) = &response.choice_type {
                values.insert(name.clone(), choice(plan, name, &response.media, false));
            }
            for media in &response.media {
                collect(plan, &media.value, &mut values);
            }
        }
    }
    values
        .into_iter()
        .map(|(name, code)| OutFile {
            path: format!("{prefix}/{name}.java"),
            content: format!("{header}{code}"),
        })
        .collect()
}
fn collect(plan: &SdkPlan, value: &JavaValue, out: &mut BTreeMap<String, String>) {
    if let JavaValue::Aggregate(a) = value {
        out.insert(a.name.clone(), aggregate(plan, a));
        for p in a.parts.iter().chain(a.additional.iter()) {
            if let Some(name) = &p.wrapper {
                out.insert(name.clone(), part(plan, p));
            }
        }
    }
}

fn headers(plan: &SdkPlan, group: &JavaHeaders) -> String {
    let name = &group.name;
    let mut out = format!(
        "/** Immutable source-typed headers. Source: <code>{}#{}</code>. */\npublic final class {name} {{\n",
        javadoc(group.source.document().as_str()),
        javadoc(group.source.pointer())
    );
    let ty = |p: &super::protocol::JavaHeader| {
        let t = plan.models().native_type(&p.schema);
        if p.wire.required() {
            t
        } else {
            format!("Presence<{t}>")
        }
    };
    for f in &group.fields {
        out.push_str(&format!("    private final {} {};\n    /** Wire header <code>{}</code>. @return immutable value or presence */\n    public {} {}() {{ return {}; }}\n",ty(f),f.name,javadoc(f.wire.name()),ty(f),f.name,f.name));
    }
    out.push_str(&format!("    private {name}(Builder b) {{\n"));
    for f in &group.fields {
        out.push_str(&format!("        this.{0} = b.{0};\n", f.name));
    }
    out.push_str("    }\n");
    let required = group
        .fields
        .iter()
        .filter(|f| f.wire.required())
        .collect::<Vec<_>>();
    let args = required
        .iter()
        .map(|f| format!("{} {}", plan.models().native_type(&f.schema), f.name))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("    /** Begin with required header values. */\n    public static Builder builder({args}) {{ Builder _builder = new Builder();\n"));
    for f in required {
        out.push_str(&format!("        _builder.{0} = {0};\n", f.name));
    }
    out.push_str("        return _builder;\n    }\n    /** Typed header construction. */\n    public static final class Builder {\n        private Builder() {}\n");
    for f in &group.fields {
        out.push_str(&format!("        private {} {}{};\n        /** Set this header. @param value value @return builder */\n        public Builder {}({} value) {{ this.{} = {}; return this; }}\n",ty(f),f.name,if f.wire.required(){""}else{" = Presence.absent()"},f.name,plan.models().native_type(&f.schema),f.name,if f.wire.required(){"value"}else{"Presence.of(value)"}));
        if !f.wire.required() {
            out.push_str(&format!("        /** Restore absence. @return builder */\n        public Builder omit{}() {{ this.{} = Presence.absent(); return this; }}\n",crate::rust_models::pascal(&f.name),f.name));
        }
    }
    out.push_str(&format!("        /** Validate and snapshot every header. @return immutable headers */\n        public {name} build() {{ var c = new ModelCodec.Context(); return {name}.read(new {name}(this).write(c), c); }}\n    }}\n    static {name} read(java.util.Map<String, JsonValue> values, ModelCodec.Context c) {{\n        Builder b = new Builder();\n"));
    for f in &group.fields {
        let key = q(f.wire.name());
        let codec = &plan.models().codec(&f.schema).holder;
        if f.wire.required() {
            out.push_str(&format!("        if (!values.containsKey({key})) throw c.invalid(\"required header is absent\");\n"));
        }
        out.push_str(&format!(
            "        if (values.containsKey({key})) b.{} = {};\n",
            f.name,
            if f.wire.required() {
                format!("{codec}.CODEC.decodeValue(values.get({key}), c)")
            } else {
                format!("Presence.of({codec}.CODEC.decodeValue(values.get({key}), c))")
            }
        ));
    }
    out.push_str(&format!("        return new {name}(b);\n    }}\n    java.util.Map<String, JsonValue> write(ModelCodec.Context c) {{\n        var values = new java.util.LinkedHashMap<String, JsonValue>();\n"));
    for f in &group.fields {
        out.push_str(&format!(
            "        {}values.put({}, {}.CODEC.encodeValue(this.{}{}, c));\n",
            if f.wire.required() {
                String::new()
            } else {
                format!("if ({}.isPresent()) ", f.name)
            },
            q(f.wire.name()),
            plan.models().codec(&f.schema).holder,
            f.name,
            if f.wire.required() { "" } else { ".value()" }
        ));
    }
    out.push_str("        return java.util.Collections.unmodifiableMap(values);\n    }\n}\n");
    out
}

fn part(plan: &SdkPlan, part: &JavaPart) -> String {
    let name = part.wrapper.as_ref().unwrap();
    let ty = part.value.native_type(plan.models());
    let explicit_type = part.wire.content_types().len() > 1
        || part.wire.content_types().first().is_some_and(|m| {
            !matches!(m.range(), crate::http_protocol::MediaRange::Concrete { .. })
        });
    let header_required = part
        .headers
        .as_ref()
        .is_some_and(|h| h.fields.iter().any(|f| f.wire.required()));
    let mut args = vec![format!("{ty} value")];
    if explicit_type {
        args.push("String contentType".into());
    }
    if header_required {
        args.push(format!("{} headers", part.headers.as_ref().unwrap().name));
    }
    let mut out = format!(
        "/** A source-typed MIME part with immutable content and explicit metadata. */\npublic final class {name} {{\n    private final {ty} value;\n    private final String contentType, filename;\n    /** Checked part content. @return value */\n    public {ty} value() {{ return value; }}\n    /** Actual or explicitly selected Content-Type. @return media type */\n    public String contentType() {{ return contentType; }}\n    /** Explicit filename metadata; never read as a path. @return filename or null */\n    public String filename() {{ return filename; }}\n"
    );
    if let Some(h) = &part.headers {
        out.push_str(&format!("    private final {} headers;\n    /** Typed part headers. @return headers */\n    public {} headers() {{ return headers; }}\n",h.name,h.name));
    }
    out.push_str(&format!("    private {name}(Builder b) {{ value=b.value; contentType=b.contentType; filename=b.filename;{} }}\n    /** Begin with the required part content and metadata. @return builder */\n    public static Builder builder({}) {{ Builder b = new Builder(); b.value=value; {} {} return b; }}\n    /** Typed MIME part construction. */\n    public static final class Builder {{\n        private Builder() {{}}\n        private {ty} value;\n        private String contentType{}, filename;\n",if part.headers.is_some(){" headers=b.headers;"}else{""},args.join(", "),if explicit_type{"b.contentType=contentType;"}else{""},if header_required{"b.headers=headers;"}else{""},if !explicit_type&&part.wire.content_types().len()==1{format!(" = {}",q(part.wire.content_types()[0].declared()))}else{String::new()}));
    if let Some(h) = &part.headers {
        out.push_str(&format!("        private {} headers{};\n        /** Supply source-typed part headers. @param value headers @return builder */\n        public Builder headers({} value) {{ headers=value; return this; }}\n",h.name,if header_required{String::new()}else{format!(" = {}.builder().build()",h.name)},h.name));
    }
    out.push_str(&format!("        /** Supply native content. @param value content @return builder */\n        public Builder value({ty} value) {{ this.value=value; return this; }}\n        /** Choose a concrete declared part media. @param value media @return builder */\n        public Builder contentType(String value) {{ contentType=value; return this; }}\n        /** Explicit filename metadata. @param value filename @return builder */\n        public Builder filename(String value) {{ filename=value; return this; }}\n        /** Validate and deeply snapshot. @return immutable part */\n        public {name} build() {{ return WIRE_CODEC.snapshot(new {name}(this)); }}\n    }}\n    static final WireCodec<{name}> WIRE_CODEC = new WireCodec<>({}, {name}::read, {name}::write);\n    private static {name} read(WireValue input, ModelCodec.Context c) {{\n        var part=(WireValue.Part)input; HttpWire.validatePart(Protocol.object({}),part,c); Builder b=new Builder();\n        b.value={}; b.contentType=part.contentType(); b.filename=part.filename();\n",q(&format!("{}#{}",part.wire.source().use_site().source().document(),part.wire.source().use_site().source().pointer())),q(&part.descriptor),read(plan,&part.value,"part.value()","c")));
    if let Some(h) = &part.headers {
        out.push_str(&format!(
            "        b.headers={}.read(part.headers(),c);\n",
            h.name
        ));
    }
    out.push_str(&format!("        return new {name}(b);\n    }}\n    private static WireValue write({name} value, ModelCodec.Context c) {{\n        var part=new WireValue.Part({},value.contentType,value.filename,{});\n        HttpWire.validatePart(Protocol.object({}),part,c); return part;\n    }}\n}}\n",write(plan,&part.value,"value.value","c"),if part.headers.is_some(){"value.headers.write(c)"}else{"java.util.Map.of()"},q(&part.descriptor)));
    out
}

fn part_read(plan: &SdkPlan, p: &JavaPart, value: &str) -> String {
    if let Some(name) = &p.wrapper {
        format!("{name}.WIRE_CODEC.read({value}, c)")
    } else {
        read(plan, &p.value, &format!("{value}.value()"), "c")
    }
}
fn part_write(plan: &SdkPlan, p: &JavaPart, value: &str) -> String {
    if let Some(name) = &p.wrapper {
        format!("(WireValue.Part){name}.WIRE_CODEC.write({value}, c)")
    } else {
        format!(
            "new WireValue.Part({}, null, null, java.util.Map.of())",
            write(plan, &p.value, value, "c")
        )
    }
}

fn aggregate(plan: &SdkPlan, a: &JavaAggregate) -> String {
    let name = &a.name;
    let positional = matches!(&a.rules, AggregateRules::Positional { .. });
    let field_ty = |p: &JavaPart| {
        let t = p.native_type(plan.models());
        if p.wire.required() {
            t
        } else {
            format!("Presence<{t}>")
        }
    };
    let mut out = format!(
        "/** Immutable source-typed {}. Aggregate rules are checked without JSON stand-ins. */\npublic final class {name} {{\n",
        if a.multipart {
            "multipart content"
        } else {
            "form content"
        }
    );
    if !positional {
        let declared = a
            .parts
            .iter()
            .filter_map(|p| p.wire.name())
            .map(q)
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("    private static final java.util.Set<String> _DECLARED_PARTS=java.util.Set.of({declared});\n"));
    }
    for p in &a.parts {
        out.push_str(&format!("    private final {} {};\n    /** Source part. @return immutable value or presence */\n    public {} {}() {{ return {}; }}\n",field_ty(p),p.name,field_ty(p),p.name,p.name));
    }
    if let Some(extra) = &a.additional {
        let ty = if positional {
            format!("java.util.List<{}>", extra.item_type(plan.models()))
        } else {
            format!(
                "java.util.Map<String, {}>",
                extra.native_type(plan.models())
            )
        };
        out.push_str(&format!("    private final {ty} additional;\n    /** Additional source-typed {}. @return immutable container */\n    public {ty} {}() {{ return additional; }}\n",if positional{"items"}else{"parts"},if positional{"items"}else{"additionalParts"}));
    }
    out.push_str(&format!("    private {name}(Builder b) {{\n"));
    for p in &a.parts {
        out.push_str(&format!("        this.{0}=b.{0};\n", p.name));
    }
    if a.additional.is_some() {
        out.push_str(&format!(
            "        additional=java.util.Collections.{}(b.additional);\n",
            if positional {
                "unmodifiableList"
            } else {
                "unmodifiableMap"
            }
        ));
    }
    out.push_str("    }\n");
    let required = a
        .parts
        .iter()
        .filter(|p| p.wire.required())
        .collect::<Vec<_>>();
    out.push_str(&format!("    /** Begin with required fields. @return builder */\n    public static Builder builder({}) {{ Builder _builder=new Builder();\n",required.iter().map(|p|format!("{} {}",p.native_type(plan.models()),p.name)).collect::<Vec<_>>().join(", ")));
    for p in required {
        out.push_str(&format!("        _builder.{0}={0};\n", p.name));
    }
    out.push_str("        return _builder;\n    }\n    /** Mutable construction state. */\n    public static final class Builder {\n        private Builder() {}\n");
    for p in &a.parts {
        out.push_str(&format!("        private {} {}{};\n        /** Set a part. @param value part @return builder */\n        public Builder {}({} value) {{ this.{}={}; return this; }}\n",field_ty(p),p.name,if p.wire.required(){""}else{"=Presence.absent()"},p.name,p.native_type(plan.models()),p.name,if p.wire.required(){"value"}else{"Presence.of(value)"}));
        if !p.wire.required() {
            out.push_str(&format!("        /** Restore absence. @return builder */\n        public Builder omit{}() {{ this.{}=Presence.absent(); return this; }}\n",crate::rust_models::pascal(&p.name),p.name));
        }
    }
    if let Some(extra) = &a.additional {
        if positional {
            let ty = extra.item_type(plan.models());
            out.push_str(&format!("        private final java.util.List<{ty}> additional=new java.util.ArrayList<>();\n        /** Append a source item. @param value item @return builder */\n        public Builder addItem({ty} value) {{ additional.add(value); return this; }}\n"));
        } else {
            let ty = extra.native_type(plan.models());
            out.push_str(&format!("        private final java.util.Map<String,{ty}> additional=new java.util.LinkedHashMap<>();\n        /** Supply an undeclared typed part. @param name wire name @param value part @return builder */\n        public Builder putAdditionalPart(String name,{ty} value) {{ if(_DECLARED_PARTS.contains(name))throw new IllegalArgumentException(\"declared part cannot be an extra\");additional.put(name,value);return this; }}\n"));
        }
    }
    out.push_str(&format!("        /** Validate all values and structural rules, then snapshot. @return immutable content */\n        public {name} build() {{ return WIRE_CODEC.snapshot(new {name}(this)); }}\n    }}\n    static final WireCodec<{name}> WIRE_CODEC=new WireCodec<>({}, {name}::read, {name}::write);\n    private static {name} read(WireValue input,ModelCodec.Context c) {{\n        var parts=(WireValue.Parts)input;HttpWire.validateParts(Protocol.object({}),parts,c);Builder b=new Builder();\n",q(&format!("{}#{}",a.source.document(),a.source.pointer())),q(&a.descriptor)));
    for (i, p) in a.parts.iter().enumerate() {
        let present = if positional {
            format!("parts.positional().size()>{i}")
        } else {
            format!("parts.named().containsKey({})", q(p.wire.name().unwrap()))
        };
        let access = if positional {
            format!("parts.positional().get({i})")
        } else {
            format!(
                "parts.named().get({}).getFirst()",
                q(p.wire.name().unwrap())
            )
        };
        let read = if p.wire.multiplicity()
            == crate::http_protocol::PartMultiplicity::RepeatedArrayItems
        {
            format!(
                "parts.named().get({}).stream().map(item -> {}).toList()",
                q(p.wire.name().unwrap()),
                part_read(plan, p, "item")
            )
        } else {
            part_read(plan, p, &access)
        };
        out.push_str(&format!(
            "        if({present}) b.{}={};\n",
            p.name,
            if p.wire.required() {
                read
            } else {
                format!("Presence.of({read})")
            }
        ));
    }
    if let Some(extra) = &a.additional {
        if positional {
            out.push_str(&format!(
                "        for(int i={};i<parts.positional().size();i++) b.additional.add({});\n",
                a.parts.len(),
                part_read(plan, extra, "parts.positional().get(i)")
            ));
        } else {
            let read = if extra.wire.multiplicity()
                == crate::http_protocol::PartMultiplicity::RepeatedArrayItems
            {
                format!(
                    "entry.getValue().stream().map(item -> {}).toList()",
                    part_read(plan, extra, "item")
                )
            } else {
                part_read(plan, extra, "entry.getValue().getFirst()")
            };
            out.push_str(&format!("        for(var entry:parts.named().entrySet()) if(!_DECLARED_PARTS.contains(entry.getKey())) b.additional.put(entry.getKey(),{read});\n"));
        }
    }
    out.push_str(&format!("        return new {name}(b);\n    }}\n    private static WireValue write({name} value,ModelCodec.Context c) {{\n        var named=new java.util.LinkedHashMap<String,java.util.List<WireValue.Part>>();var positions=new java.util.ArrayList<WireValue.Part>();\n"));
    if positional {
        out.push_str("        boolean gap=false;\n");
    }
    for p in &a.parts {
        let access = format!(
            "value.{}{}",
            p.name,
            if p.wire.required() { "" } else { ".value()" }
        );
        let encoded = part_write(plan, p, &access);
        let writer = if positional {
            format!(
                "if(gap)throw c.invalid(\"positional multipart cannot contain holes\");positions.add({encoded});"
            )
        } else if p.wire.multiplicity()
            == crate::http_protocol::PartMultiplicity::RepeatedArrayItems
        {
            format!(
                "named.put({}, {access}.stream().map(item -> {}).toList());",
                q(p.wire.name().unwrap()),
                part_write(plan, p, "item")
            )
        } else {
            format!(
                "named.put({},java.util.List.of({encoded}));",
                q(p.wire.name().unwrap())
            )
        };
        if p.wire.required() {
            out.push_str(&format!("        {writer}\n"));
        } else {
            out.push_str(&format!(
                "        if(value.{}.isPresent()) {{ {writer} }}{}\n",
                p.name,
                if positional { " else {gap=true;}" } else { "" }
            ));
        }
    }
    if let Some(extra) = &a.additional {
        if positional {
            out.push_str(&format!("        if(gap&&!value.additional.isEmpty())throw c.invalid(\"positional multipart cannot contain holes\");for(var item:value.additional) positions.add({});\n",part_write(plan,extra,"item")));
        } else {
            let expression = if extra.wire.multiplicity()
                == crate::http_protocol::PartMultiplicity::RepeatedArrayItems
            {
                format!(
                    "entry.getValue().stream().map(item -> {}).toList()",
                    part_write(plan, extra, "item")
                )
            } else {
                format!(
                    "java.util.List.of({})",
                    part_write(plan, extra, "entry.getValue()")
                )
            };
            out.push_str(&format!("        for(var entry:value.additional.entrySet()) named.put(entry.getKey(),{expression});\n"));
        }
    }
    out.push_str(&format!("        var parts=new WireValue.Parts(named,positions);HttpWire.validateParts(Protocol.object({}),parts,c);return parts;\n    }}\n}}\n",q(&a.descriptor)));
    out
}

fn choice(plan: &SdkPlan, name: &str, media: &[JavaMedia], request: bool) -> String {
    let mut out = format!(
        "/** Explicit source-declared media alternatives. */\npublic sealed abstract class {name} {{\n    private {name}() {{}}\n    /** Actual concrete media type. @return Content-Type */\n    public abstract String contentType();\n"
    );
    for m in media {
        let ty = m.value.native_type(plan.models());
        let variant = &m.name;
        let concrete = matches!(
            m.wire.media_type().range(),
            crate::http_protocol::MediaRange::Concrete { .. }
        );
        out.push_str(&format!("    /** Source media <code>{}</code>. */\n    public static final class {variant} extends {name} {{\n        private final {ty} value; private final String contentType;\n",javadoc(m.wire.media_type().declared())));
        if request {
            if concrete {
                out.push_str(&format!("        /** Use the declared media. @param value content */\n        public {variant}({ty} value) {{ this(value,{}); }}\n",q(m.wire.media_type().declared())));
            }
            out.push_str(&format!("        /** Select explicit concrete media. @param value content @param contentType actual media */\n        public {variant}({ty} value,String contentType) {{ this.value={};HttpWire.media(contentType);this.contentType=contentType; }}\n",snapshot(plan,&m.value,"value",None)));
            out.push_str(&format!("        private {variant}({ty} value,String contentType,ModelCodec.Context checked) {{ this.value=value;this.contentType=contentType; }}\n"));
        } else {
            out.push_str(&format!("        {variant}({ty} value,String contentType) {{ this.value=value;this.contentType=contentType; }}\n"));
        }
        out.push_str(&format!("        /** Typed content. @return value */\n        public {ty} value() {{ return value; }}\n        @Override public String contentType() {{ return contentType; }}\n    }}\n"));
    }
    if request {
        out.push_str(&format!("    static final WireCodec<{name}> WIRE_CODEC=new WireCodec<>(\"\",{name}::read,{name}::write);\n    private static {name} read(WireValue value,ModelCodec.Context c) {{ var selected=(WireValue.Selected)value;\n"));
        for m in media {
            out.push_str(&format!("        if(selected.declaration().equals({})) return new {}({},selected.contentType(),c);\n",q(&m.descriptor),m.name,read(plan,&m.value,"selected.value()","c")));
        }
        out.push_str(&format!("        throw c.invalid(\"unknown media alternative\");\n    }}\n    private static WireValue write({name} value,ModelCodec.Context c) {{\n"));
        for m in media {
            out.push_str(&format!("        if(value instanceof {} selected) return new WireValue.Selected({},selected.contentType(),{});\n",m.name,q(&m.descriptor),write(plan,&m.value,"selected.value()","c")));
        }
        out.push_str("        throw c.invalid(\"missing media alternative\");\n    }\n");
    }
    out.push_str("}\n");
    out
}
