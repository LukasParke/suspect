//! Ordinary native construction from the existing validated example/model plans.
use super::*;
use crate::{
    go_models::{GoDecl, GoType, Key},
    http_examples,
};
use emit::{location, q};
use serde_json::{Value, json};

fn native_type(ty: &GoType, plan: &HttpPlan) -> String {
    match ty {
        GoType::Primitive("string" | "bool") => {
            ty.render(&plan.codecs.models().descriptors().names)
        }
        GoType::Primitive(v) => format!("sdk.{v}"),
        GoType::Named(key) => format!("sdk.{}", plan.codecs.models().descriptors().names[key]),
        GoType::Slice(v) => format!("[]{}", native_type(v, plan)),
        GoType::Map(v) => format!("map[string]{}", native_type(v, plan)),
        GoType::Pointer(v) => format!("*{}", native_type(v, plan)),
        GoType::Nullable(v) => format!("sdk.Nullable[{}]", native_type(v, plan)),
        GoType::Optional(v) => format!("sdk.Optional[{}]", native_type(v, plan)),
        GoType::Presence(v) => format!("sdk.Presence[{}]", native_type(v, plan)),
    }
}
fn generic(value: &Value) -> String {
    match value {
        Value::Null => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => q(s),
        Value::Number(n) => format!("must(sdk.ParseNumber({}))", q(&n.to_string())),
        Value::Array(a) => format!(
            "[]sdk.Value{{{}}}",
            a.iter().map(generic).collect::<Vec<_>>().join(",")
        ),
        Value::Object(o) => format!(
            "map[string]sdk.Value{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{}:{}", q(k), generic(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
fn expression(plan: &HttpPlan, ty: &GoType, value: &Value, depth: usize) -> Option<String> {
    if depth > 32 {
        return None;
    }
    let nested = |ty, value| expression(plan, ty, value, depth + 1);
    Some(match ty {
        GoType::Primitive("string") => q(value.as_str()?),
        GoType::Primitive("bool") => value.as_bool()?.to_string(),
        GoType::Primitive("Integer") => format!(
            "must(sdk.ParseInteger({}))",
            q(&value.as_number()?.to_string())
        ),
        GoType::Primitive("Number") => format!(
            "must(sdk.ParseNumber({}))",
            q(&value.as_number()?.to_string())
        ),
        GoType::Primitive("Value") => generic(value),
        GoType::Primitive(_) => return None,
        GoType::Nullable(v) | GoType::Presence(v) => {
            let name = if matches!(ty, GoType::Nullable(_)) {
                "Nullable"
            } else {
                "Presence"
            };
            let name = if value.is_null() {
                format!("{name}Null")
            } else if name == "Nullable" {
                "NullableValue".into()
            } else {
                "PresenceSome".into()
            };
            format!(
                "sdk.{name}[{}]({})",
                native_type(v, plan),
                if value.is_null() {
                    String::new()
                } else {
                    nested(v, value)?
                }
            )
        }
        GoType::Optional(v) => format!(
            "sdk.OptionalSome[{}]({})",
            native_type(v, plan),
            nested(v, value)?
        ),
        GoType::Pointer(v) => format!("pointer({})", nested(v, value)?),
        GoType::Slice(v) => format!(
            "{}{{{}}}",
            native_type(ty, plan),
            value
                .as_array()?
                .iter()
                .map(|item| nested(v, item))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
        GoType::Map(v) => format!(
            "{}{{{}}}",
            native_type(ty, plan),
            value
                .as_object()?
                .iter()
                .map(|(k, item)| Some(format!("{}:{}", q(k), nested(v, item)?)))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
        GoType::Named(key) => {
            let descriptors = plan.codecs.models().descriptors();
            let name = &descriptors.names[key];
            match &descriptors.declarations[key] {
                GoDecl::Alias(ty) => nested(ty, value)?,
                GoDecl::Literals { values, .. } => format!(
                    "sdk.{name}{}",
                    values
                        .iter()
                        .find(|v| serde_json::from_str::<Value>(&v.token).ok().as_ref()
                            == Some(value))?
                        .name
                ),
                GoDecl::Struct { fields, extras } => {
                    let object = value.as_object()?;
                    let args = fields
                        .iter()
                        .filter(|f| f.init.is_none())
                        .map(|f| nested(&f.ty, object.get(&f.wire)?))
                        .collect::<Option<Vec<_>>>()?;
                    let constructor = format!("sdk.New{name}({})", args.join(","));
                    let mut assignments = Vec::new();
                    for field in fields.iter().filter(|f| f.init.is_some()) {
                        if let Some(value) = object.get(&field.wire) {
                            assignments.push(format!(
                                "v.{}={}",
                                field.name,
                                nested(&field.ty, value)?
                            ));
                        }
                    }
                    for (key, value) in object
                        .iter()
                        .filter(|(k, _)| !fields.iter().any(|f| &f.wire == *k))
                    {
                        let ty = extras.as_ref()?;
                        assignments.push(format!(
                            "if err:=v.SetExtra({},{});err!=nil{{panic(err)}}",
                            q(key),
                            nested(ty, value)?
                        ));
                    }
                    if assignments.is_empty() {
                        constructor
                    } else {
                        format!(
                            "func()sdk.{name}{{v:={constructor};{};return v}}()",
                            assignments.join(";")
                        )
                    }
                }
                GoDecl::Union(variants) => {
                    // Validate against actual indexed branch roots before choosing a
                    // constructor; no discriminator or operation-name heuristics.
                    let roots = variants
                        .iter()
                        .map(|v| v.source.clone())
                        .collect::<Vec<_>>();
                    let validator =
                        suspect_schema::OwnedCompiler::new(plan.config.codecs.schema.clone())
                            .compile(plan.contract.clone(), &roots)
                            .ok()?;
                    let variant = variants.iter().find(|v| {
                        matches!(
                            validator.validate(&v.source, value),
                            suspect_schema::OwnedOutcome::Valid
                        )
                    })?;
                    format!(
                        "sdk.New{name}{}({})",
                        variant.name,
                        nested(&variant.ty, value)?
                    )
                }
            }
        }
    })
}

fn input(plan: &HttpPlan, op: &PlannedOperation) -> Option<(String, Vec<Value>)> {
    let bindings = http_examples::bindings(plan.examples());
    let binding = bindings.get(&op.source)?;
    let parameters = op.parameters.iter().map(|p| http_examples::InputSlot {
        name: &p.field_name,
        required: p.wire.required(),
        container: p.wire.source().use_site().source().clone(),
    });
    let bound = binding.bind(parameters)?;
    let mut args = Vec::new();
    let mut setters = String::new();
    let mut records = Vec::new();
    for member in bound {
        let entry = &binding.operation.entries[member.example_index];
        let parameter = op.parameters.iter().find(|p| p.field_name == member.name)?;
        if parameter.wire.serialize(&entry.value).is_err() {
            if member.required {
                return None;
            };
            continue;
        }
        let key: Key = (
            entry.schema.clone(),
            crate::rust_models::RepresentationRole::Model,
        );
        let expr = expression(plan, &GoType::Named(key), &entry.value, 0)?;
        if member.required {
            args.push(expr)
        } else {
            let setter = op
                .parameters
                .iter()
                .find(|p| p.field_name == member.name)
                .and_then(|p| p.setter_name.as_ref())
                .or_else(|| {
                    op.body()
                        .filter(|b| b.field_name == member.name)
                        .and_then(|b| b.setter_name.as_ref())
                })?;
            setters.push_str(&format!(".{setter}({expr})"));
        }
        records.push(json!({"member":member.name,"entry":member.example_index,"container":location(&entry.container),"construction":"native-constructor"}));
    }
    if let Some(body) = op.body() {
        let candidate=body.media.iter().find_map(|media| {
            let mut record=Vec::new();let value=media_example(plan,media,binding.operation,&mut record)?;
            // Example media choices are explicit. A broad representation cannot
            // silently outrank a more specific schema even in generated samples.
            let content_type=concrete_media(media.wire.media_type());
            if body.wire.match_media(&content_type).ok()?.source()!=media.wire.source(){return None;}
            let value=if body.is_choice(){format!("sdk.{}({}{value})",media.constructor,if media.requires_content_type(){format!("{},",q(&content_type))}else{String::new()})}
                else if media.requires_content_type(){format!("sdk.Content[{}]{{ContentType:{},Data:{value}}}",qualified_media(media),q(&content_type))}else{value};
            if matches!(media.wire.representation(),protocol::Representation::Json{codec:Some(_)}|protocol::Representation::Text{codec:Some(_),..}) || record.first().is_some_and(|r|r.get("validatedAggregate").is_some()){
                // Construction and media selection describe the same real input
                // slot. Keep its validated entry identity in one member binding.
                let binding=record.first_mut()?;
                binding["member"]=json!(body.field_name);
                binding["mediaType"]=json!(content_type);
            }else{
                record.push(json!({"member":body.field_name,"container":location(media.wire.source().use_site().source()),"construction":"native-constructor","mediaType":content_type}));
            }
            Some((value,record))
        });
        if let Some((value, record)) = candidate {
            if body.wire.required() {
                args.push(value)
            } else {
                setters.push_str(&format!(".{}({value})", body.setter_name.as_ref()?));
            }
            records.extend(record);
        } else if body.wire.required() {
            return None;
        }
    }
    Some((
        format!("sdk.{}({}){setters}", op.input_constructor, args.join(",")),
        records,
    ))
}

fn concrete_media(media: &protocol::MediaType) -> String {
    match media.range() {
        protocol::MediaRange::Concrete { .. } => media.declared().into(),
        protocol::MediaRange::Any => "application/octet-stream".into(),
        protocol::MediaRange::Type { type_name } => format!("{type_name}/octet-stream"),
    }
}
fn qualified_media(media: &PlannedMedia) -> String {
    match media.wire.representation() {
        protocol::Representation::Binary { .. } => "[]byte".into(),
        protocol::Representation::Json { codec: None } => "sdk.Value".into(),
        protocol::Representation::Text { codec: None, .. } => "string".into(),
        _ => format!("sdk.{}", media.native_type),
    }
}
fn entry_expression(plan: &HttpPlan, entry: &crate::examples::ExampleEntry) -> Option<String> {
    let key: Key = (
        entry.schema.clone(),
        crate::rust_models::RepresentationRole::Model,
    );
    expression(plan, &GoType::Named(key), &entry.value, 0)
}
fn media_example(
    plan: &HttpPlan,
    media: &PlannedMedia,
    examples: &crate::examples::OperationExamples,
    records: &mut Vec<Value>,
) -> Option<String> {
    use protocol::Representation;
    let entries = &examples.entries;
    let at = media.wire.source().use_site().source();
    match media.wire.representation() {
        Representation::Binary { .. } => {
            records.push(json!({"container":location(at),"construction":"explicit-native-byte-recipe","recipe":"empty in-memory []byte; replace with caller bytes before live use","schemaValidation":"byte policy only; no substitute JSON value"}));
            Some("[]byte{}".into())
        }
        Representation::Json { codec: None } => {
            records.push(
                json!({"container":location(at),"construction":"explicit-schema-free-json-recipe"}),
            );
            Some("sdk.Value(map[string]sdk.Value{})".into())
        }
        Representation::Text { codec: None, .. } => Some("string(\"example\")".into()),
        Representation::Json { codec: Some(codec) }
        | Representation::Text {
            codec: Some(codec), ..
        } => {
            let (index, entry) = entries.iter().enumerate().find(|(_, e)| {
                matches!(e.role, crate::examples::ExampleRole::RequestBody)
                    && &e.container == at
                    && &e.schema == codec.schema().id()
            })?;
            records.push(json!({"entry":index,"container":location(&entry.container),"schema":location(&entry.schema),"construction":"native-constructor","origin":http_examples::origin(&entry.origin)}));
            entry_expression(plan, entry)
        }
        Representation::Stream { stream } => {
            let Some(codec) = stream.item_codec() else {
                // A schemaless request stream has no declared item codec, so no
                // validated native item example can be constructed.
                return None;
            };
            let entry = entries.iter().find(|e| {
                matches!(e.role, crate::examples::ExampleRole::RequestItem)
                    && &e.schema == codec.schema().id()
            })?;
            let value = entry_expression(plan, entry)?;
            records.push(json!({"container":location(at),"schema":location(&entry.schema),"construction":"finite-native-item-slice","origin":http_examples::origin(&entry.origin)}));
            Some(format!(
                "[]sdk.{}{{{value}}}",
                plan.symbols[codec.schema().id()]
            ))
        }
        Representation::Form { .. } | Representation::Multipart { .. } => {
            if let Some((index, entry)) =
                examples
                    .validated_aggregates
                    .iter()
                    .enumerate()
                    .find(|(_, entry)| {
                        matches!(entry.role, crate::examples::ExampleRole::RequestBody)
                            && &entry.container == at
                    })
            {
                let value = declared_aggregate(plan, media, &entry.value, entries)?;
                records.push(json!({"validatedAggregate":index,"container":location(&entry.container),"schema":location(&entry.schema),"declaredSource":entry.declared_source.as_ref().map(location),"origin":http_examples::origin(&entry.origin),"construction":"native-aggregate-constructor"}));
                return Some(value);
            }
            aggregate_example(plan, media, entries, records)
        }
    }
}

fn declared_part(
    plan: &HttpPlan,
    part: &PlannedPart,
    value: &Value,
    multipart: bool,
    entries: &[crate::examples::ExampleEntry],
) -> Option<String> {
    let codec = match part.wire.representation() {
        protocol::PartRepresentation::Json { codec, .. }
        | protocol::PartRepresentation::Text { codec, .. }
        | protocol::PartRepresentation::Style { codec, .. } => codec,
        protocol::PartRepresentation::Binary { .. } => return None,
    };
    let single = |value: &Value| -> Option<String> {
        let key = (
            codec.schema().id().clone(),
            crate::rust_models::RepresentationRole::Model,
        );
        let mut expression = expression(plan, &GoType::Named(key), value, 0)?;
        if multipart {
            expression = format!("sdk.NewPart({expression})");
            if let Some(media) = part.wire.content_types().first() {
                expression.push_str(&format!(".WithContentType({})", q(&concrete_media(media))));
            }
            for header in part.wire.headers() {
                let entry = entries.iter().find(|entry| {
                    matches!(
                        entry.role,
                        crate::examples::ExampleRole::RequestPartHeader { .. }
                    ) && &entry.container == header.source().use_site().source()
                });
                if let Some(entry) = entry {
                    let wire = header.serialize(&entry.value).ok()?;
                    expression.push_str(&format!(
                        ".WithHeader({},{})",
                        q(header.name()),
                        q(wire.value())
                    ));
                } else if header.required() {
                    return None;
                }
            }
        }
        Some(expression)
    };
    if part.wire.multiplicity() == protocol::PartMultiplicity::RepeatedArrayItems {
        let values = value
            .as_array()?
            .iter()
            .map(single)
            .collect::<Option<Vec<_>>>()?;
        Some(format!(
            "{}{{{}}}",
            part_type(part, multipart),
            values.join(",")
        ))
    } else {
        single(value)
    }
}

// Complete grouping comes from the checked aggregate entry, never from an
// invented whole-object codec or independent first-item reconstruction.
fn declared_aggregate(
    plan: &HttpPlan,
    media: &PlannedMedia,
    value: &Value,
    entries: &[crate::examples::ExampleEntry],
) -> Option<String> {
    let aggregate = media.aggregate.as_ref()?;
    let mut arguments = Vec::new();
    let mut setters = String::new();
    let mut extras = Vec::new();
    if aggregate.positional {
        let values = value.as_array()?;
        for (index, part) in aggregate.parts.iter().enumerate() {
            if let Some(value) = values.get(index) {
                let value = declared_part(plan, part, value, true, entries)?;
                if part.wire.required() {
                    arguments.push(value)
                } else {
                    setters.push_str(&format!(".{}({value})", part.setter_name.as_ref()?));
                }
            } else if part.wire.required() {
                return None;
            }
        }
        if values.len() > aggregate.parts.len() {
            let tail = aggregate.additional.as_ref()?;
            for value in &values[aggregate.parts.len()..] {
                extras.push(declared_part(plan, tail, value, true, entries)?);
            }
        }
    } else {
        let object = value.as_object()?;
        for part in &aggregate.parts {
            if let Some(value) = object.get(part.wire.name()?) {
                let value = declared_part(plan, part, value, aggregate.multipart, entries)?;
                if part.wire.required() {
                    arguments.push(value)
                } else {
                    setters.push_str(&format!(".{}({value})", part.setter_name.as_ref()?));
                }
            } else if part.wire.required() {
                return None;
            }
        }
        for (key, value) in object.iter().filter(|(key, _)| {
            !aggregate
                .parts
                .iter()
                .any(|part| part.wire.name() == Some(key.as_str()))
        }) {
            let additional = aggregate.additional.as_ref()?;
            extras.push(format!(
                "{}:{}",
                q(key),
                declared_part(plan, additional, value, aggregate.multipart, entries)?
            ));
        }
    }
    let value = format!(
        "sdk.{}({}){setters}",
        aggregate.constructor,
        arguments.join(",")
    );
    if extras.is_empty() {
        return Some(value);
    }
    let additional = aggregate.additional.as_ref()?;
    let field = if aggregate.positional {
        "Items"
    } else {
        "Extras"
    };
    let container = if aggregate.positional {
        "[]"
    } else {
        "map[string]"
    };
    Some(format!(
        "func()sdk.{}{{v:={value};v.{field}={container}{}{{{}}};return v}}()",
        aggregate.type_name,
        part_type(additional, aggregate.multipart),
        extras.join(",")
    ))
}
fn part_type(part: &PlannedPart, multipart: bool) -> String {
    let value = if matches!(
        part.wire.representation(),
        protocol::PartRepresentation::Binary { .. }
    ) {
        "[]byte".into()
    } else {
        format!("sdk.{}", part.data_type)
    };
    let value = if multipart {
        format!("sdk.Part[{value}]")
    } else {
        value
    };
    if part.wire.multiplicity() == protocol::PartMultiplicity::RepeatedArrayItems {
        format!("[]{value}")
    } else {
        value
    }
}
fn part_example(
    plan: &HttpPlan,
    part: &PlannedPart,
    multipart: bool,
    entries: &[crate::examples::ExampleEntry],
    records: &mut Vec<Value>,
) -> Option<String> {
    use protocol::PartRepresentation;
    let at = part.wire.source().use_site().source();
    let mut value = match part.wire.representation() {
        PartRepresentation::Binary { .. } => {
            records.push(json!({"container":location(at),"construction":"explicit-native-byte-recipe","recipe":"empty []byte, never a filename/string/null conversion"}));
            "[]byte{}".into()
        }
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => {
            let entry = entries.iter().find(|e| {
                matches!(e.role, crate::examples::ExampleRole::RequestPart { .. })
                    && &e.schema == codec.schema().id()
            })?;
            if matches!(part.wire.representation(), PartRepresentation::Style { .. })
                && (entry.value.as_object().is_some_and(|v| v.is_empty())
                    || entry.value.as_array().is_some_and(|v| v.is_empty()))
            {
                return None;
            }
            records.push(json!({"container":location(at),"schema":location(&entry.schema),"construction":"native-part-constructor","origin":http_examples::origin(&entry.origin)}));
            entry_expression(plan, entry)?
        }
    };
    if multipart {
        value = format!("sdk.NewPart({value})");
        if let Some(media) = part.wire.content_types().first() {
            value.push_str(&format!(".WithContentType({})", q(&concrete_media(media))));
        }
        for header in part.wire.headers() {
            let entry = entries.iter().find(|e| {
                matches!(
                    e.role,
                    crate::examples::ExampleRole::RequestPartHeader { .. }
                ) && &e.container == header.source().use_site().source()
            });
            if let Some(entry) = entry {
                let serialized = header.serialize(&entry.value).ok()?;
                value.push_str(&format!(
                    ".WithHeader({},{})",
                    q(header.name()),
                    q(serialized.value())
                ));
            } else if header.required() {
                return None;
            }
        }
    }
    if part.wire.multiplicity() == protocol::PartMultiplicity::RepeatedArrayItems {
        let count = part.wire.min_items().map_or(1, |n| *n.value()).max(1);
        if count > 32 || part.wire.max_items().is_some_and(|n| *n.value() < count) {
            return None;
        }
        value = format!(
            "{}{{{}}}",
            part_type(part, multipart),
            std::iter::repeat_n(value, count as usize)
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    Some(value)
}
fn aggregate_example(
    plan: &HttpPlan,
    media: &PlannedMedia,
    entries: &[crate::examples::ExampleEntry],
    records: &mut Vec<Value>,
) -> Option<String> {
    let aggregate = media.aggregate.as_ref()?;
    let (min, max, required) = match media.wire.representation() {
        protocol::Representation::Form { form } => (
            form.rules().min_properties(),
            form.rules().max_properties(),
            form.rules().required(),
        ),
        protocol::Representation::Multipart {
            multipart: protocol::MultipartPlan::Named { rules, .. },
        } => (
            rules.min_properties(),
            rules.max_properties(),
            rules.required(),
        ),
        protocol::Representation::Multipart {
            multipart:
                protocol::MultipartPlan::Positional {
                    min_items,
                    max_items,
                    ..
                },
        } => (min_items.as_ref(), max_items.as_ref(), &[][..]),
        _ => return None,
    };
    let maximum = max.map_or(32, |v| *v.value()).min(32);
    let minimum = min.map_or(0, |v| *v.value());
    let mut count = 0;
    let mut args = Vec::new();
    let mut setters = String::new();
    let mut names = BTreeSet::new();
    let mut gap = false;
    for part in &aggregate.parts {
        if count >= maximum && !part.wire.required() {
            gap = true;
            continue;
        }
        if let Some(value) = part_example(plan, part, aggregate.multipart, entries, records) {
            if aggregate.positional && gap {
                return None;
            };
            count += 1;
            if let Some(name) = part.wire.name() {
                names.insert(name.to_owned());
            }
            if part.wire.required() {
                args.push(value)
            } else {
                setters.push_str(&format!(".{}({value})", part.setter_name.as_ref()?));
            }
        } else if part.wire.required() {
            return None;
        } else {
            gap = true;
        }
    }
    let mut extras = Vec::new();
    for name in required
        .iter()
        .map(|r| r.value())
        .filter(|n| !names.contains(*n))
    {
        let part = aggregate.additional.as_ref()?;
        let value = part_example(plan, part, aggregate.multipart, entries, records)?;
        extras.push(format!("{}:{value}", q(name)));
        count += 1;
    }
    if count < minimum {
        return None;
    };
    if count > maximum {
        return None;
    }
    let value = format!("sdk.{}({}){setters}", aggregate.constructor, args.join(","));
    if extras.is_empty() {
        Some(value)
    } else {
        let part = aggregate.additional.as_ref()?;
        Some(format!(
            "func()sdk.{}{{v:={value};v.Extras=map[string]{}{{{}}};return v}}()",
            aggregate.type_name,
            part_type(part, aggregate.multipart),
            extras.join(",")
        ))
    }
}

pub(super) fn render(plan: &HttpPlan, package: &PackageConfig) -> (String, Vec<Value>) {
    let mut code = format!(
        "// Executable source examples; origins and availability are in source-bindings.json.\npackage main\nimport(\"context\";\"flag\";\"fmt\";\"os\";\"time\";sdk {})\nfunc must[T any](v T,e error)T{{if e!=nil{{panic(e)}};return v}}\nfunc pointer[T any](v T)*T{{return &v}}\nfunc roundTrip[T any](codec sdk.Codec[T],text string)error{{value,err:=codec.Decode([]byte(text));if err!=nil{{return err}};data,err:=codec.Encode(value);if err!=nil{{return err}};_,err=codec.Decode(data);return err}}\nfunc validate()error{{\n",
        q(&package.module_path)
    );
    let mut count = 0;
    for op in plan.examples.operations() {
        for entry in &op.entries {
            if let Some(model) = plan.symbols.get(&entry.schema) {
                code.push_str(&format!(
                    "if err:=roundTrip(sdk.Codecs.{model},{});err!=nil{{return err}}\n",
                    q(&entry.value.to_string())
                ));
                count += 1;
            }
        }
    }
    code.push_str(&format!(
        "fmt.Println(\"validated-examples {count}\");return nil}}\n"
    ));
    let mut records = Vec::new();
    let mut available = Vec::new();
    for (i, op) in plan.operations.iter().enumerate() {
        if let Some((expr, bindings)) = input(plan, op) {
            code.push_str(&format!("func call{i}(ctx context.Context,client *sdk.Client)(sdk.{},error){{input:={expr};return client.{}(ctx,input)}}\n",op.success_type,op.method_name));
            available.push(i);
            records.push(json!({"source":location(&op.source),"method":op.method_name,"available":true,"bindings":bindings}));
        } else {
            records.push(json!({"source":location(&op.source),"method":op.method_name,"available":false,"reason":"required input has no validated example","details":"native construction requires a validated value or an explicitly labeled native recipe; byte inputs are never synthesized from JSON null"}));
        }
    }
    code.push_str("func run(ctx context.Context,client *sdk.Client)error{\n");
    for i in &available {
        code.push_str(&format!(
            "if _,err:=call{i}(ctx,client);err!=nil{{return err}}\n"
        ));
    }
    code.push_str(&format!(
        "fmt.Println(\"http-examples {}\");return nil}}\n",
        available.len()
    ));
    code.push_str("func main(){server:=flag.String(\"server-url\",\"\",\"explicit fixture server URL\");token:=flag.String(\"token\",\"\",\"explicit fixture credential value\");flag.Parse();if err:=validate();err!=nil{fmt.Fprintln(os.Stderr,err);os.Exit(1)};if *server==\"\"{return};credentials:=sdk.Credentials{};_=token\n");
    for scheme in plan.credentials.keys() {
        let r = plan
            .protocol
            .operations()
            .iter()
            .flat_map(|op| op.security().alternatives())
            .flat_map(|a| a.requirements())
            .find(|r| r.name() == scheme)
            .unwrap();
        let call = match r.credential() {
            protocol::CredentialHook::Bearer { .. } => format!("WithBearer({},*token)", q(scheme)),
            protocol::CredentialHook::Basic => {
                format!("WithBasic({},\"fixture-user\",*token)", q(scheme))
            }
            protocol::CredentialHook::ApiKey { .. } => format!("WithAPIKey({},*token)", q(scheme)),
            _ => format!(
                "WithAuthorization({},sdk.Authorization{{Scheme:\"Bearer\",Value:*token}})",
                q(scheme)
            ),
        };
        code.push_str(&format!("credentials=credentials.{call}\n"));
    }
    if let Some(factory) = plan.credential_env_factory() {
        code.push_str(&format!("var client *sdk.Client;var err error;explicitToken:=false;flag.Visit(func(f *flag.Flag){{if f.Name==\"token\"{{explicitToken=true}}}});if explicitToken{{client,err=sdk.NewClient(credentials,sdk.ClientOptions{{ServerURL:*server}})}}else{{client,err=sdk.{factory}(sdk.ClientOptions{{ServerURL:*server}})}};"));
        code.push_str("if err!=nil{fmt.Fprintln(os.Stderr,err);os.Exit(1)};defer client.CloseIdleConnections();ctx,cancel:=context.WithTimeout(context.Background(),30*time.Second);defer cancel();if err=run(ctx,client);err!=nil{fmt.Fprintln(os.Stderr,err);os.Exit(1)}}\n");
    } else {
        code.push_str("client,err:=sdk.NewClient(credentials,sdk.ClientOptions{ServerURL:*server});if err!=nil{fmt.Fprintln(os.Stderr,err);os.Exit(1)};defer client.CloseIdleConnections();ctx,cancel:=context.WithTimeout(context.Background(),30*time.Second);defer cancel();if err=run(ctx,client);err!=nil{fmt.Fprintln(os.Stderr,err);os.Exit(1)}}\n");
    }
    (code, records)
}
pub(super) fn quickstart(plan: &HttpPlan, package: &PackageConfig) -> String {
    for op in plan
        .operations
        .iter()
        .filter(|op| !op.optional_input)
        .chain(plan.operations.iter().filter(|op| op.optional_input))
    {
        if let Some((expr, _)) = input(plan, op) {
            let method = op.data_method.as_ref().unwrap_or(&op.method_name);
            let call = if op.parameters.is_empty() && op.body.is_none() {
                format!("value, err := client.{method}(ctx)")
            } else {
                format!("input := {expr}\nvalue, err := client.{method}(ctx, input)")
            };
            return format!(
                "## Native callsite\n\nImport `sdk {}`. With a configured `client` and caller-owned `ctx`:\n\n```go\n{call}\nif err != nil {{ return err }}\n_ = value\n```\n\nThe executable `examples/validated/main.go` contains these native constructors. Its `must` helper only unwraps checked exact-number literals; live calls return ordinary Go errors. Values are labeled as declared or synthesized in `examples.json`.\n",
                q(&package.module_path)
            );
        }
    }
    "## Native callsites\n\nUse each operation's required-input constructor and source-typed parts. Required byte examples must be supplied explicitly as `[]byte`; the generator does not turn JSON example values into file contents.\n".into()
}
