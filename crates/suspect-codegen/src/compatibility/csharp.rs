//! C# native interface capture from retained declarations and allocated symbols.
//! Source addresses are correspondence metadata, never part of a native type's identity.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use super::{Location, NativeModel, NativeOperation, NativeSnapshot, PlanFinding};
use crate::{
    backend,
    csharp_sdk::{
        self, PlannedOperation, SdkPlan,
        models::{
            CsBranch, CsDecl, CsField, CsType, FieldInitialization, Key, ModelPlan,
            RepresentationRole,
        },
    },
};

fn qualified(namespace: &str, name: &str) -> String {
    format!("{namespace}.{name}")
}
fn named(name: impl Into<String>) -> Value {
    json!({"kind":"named","name":name.into()})
}
fn nullable(value: Value) -> Value {
    if value["kind"] == "nullable" {
        value
    } else {
        json!({"kind":"nullable","type":value})
    }
}
fn optional(namespace: &str, value: Value) -> Value {
    json!({"kind":"optional","name":qualified(namespace,"Optional"),"type":value,"default":"absent","readonly":true})
}
fn key(id: &SchemaId) -> Key {
    (id.clone(), RepresentationRole::Model)
}
fn type_at(models: &ModelPlan, id: &SchemaId, namespace: &str) -> Value {
    type_of(models, &CsType::Named(key(id)), namespace)
}
fn type_of(models: &ModelPlan, ty: &CsType, namespace: &str) -> Value {
    match ty {
        CsType::Native("JsonNull" | "Never") => {
            let CsType::Native(name) = ty else {
                unreachable!()
            };
            named(qualified(namespace, name))
        }
        CsType::Native(name) => json!({"kind":"primitive","name":name}),
        CsType::Number => named(qualified(namespace, "JsonNumber")),
        CsType::Integer => named(qualified(namespace, "JsonInteger")),
        CsType::Json => {
            json!({"kind":"primitive","name":"System.Text.Json.JsonElement","jsonNullAllowed":true})
        }
        CsType::Named(key) => match &models.declarations()[key] {
            CsDecl::Alias(inner) => type_of(models, inner, namespace),
            _ => {
                let name = named(qualified(namespace, &models.names()[key]));
                if models.is_nullable(&key.0) == Some(true) {
                    nullable(name)
                } else {
                    name
                }
            }
        },
        CsType::Nullable(inner) => nullable(type_of(models, inner, namespace)),
        CsType::List(inner) => {
            json!({"kind":"generic","name":"System.Collections.Generic.List","arguments":[type_of(models,inner,namespace)]})
        }
        CsType::Dict(inner) => {
            json!({"kind":"generic","name":"System.Collections.Generic.Dictionary","arguments":[{"kind":"primitive","name":"string"},type_of(models,inner,namespace)]})
        }
    }
}

fn property(
    name: &str,
    ty: Value,
    setter: Option<&str>,
    required: bool,
    initialization: Value,
) -> Value {
    json!({"kind":"property","name":name,"type":ty,"access":"public","getter":true,
        "setter":setter,"readonly":setter.is_none(),"required":required,"initialization":initialization})
}
fn initializer(models: &ModelPlan, field: &CsField, namespace: &str) -> (bool, Value) {
    match models.field_initialization(field) {
        FieldInitialization::Required => (true, json!({"kind":"required-member"})),
        FieldInitialization::Absent => (false, json!({"kind":"absent"})),
        FieldInitialization::Singleton {
            symbol,
            member,
            token,
        } => (
            false,
            json!({
                "kind":"literal","type":qualified(namespace,&models.names()[&symbol]),"member":member,"token":token,
            }),
        ),
    }
}
fn model_field(models: &ModelPlan, field: &CsField, namespace: &str) -> Value {
    let value = type_of(models, &field.ty, namespace);
    let (required, initialization) = initializer(models, field, namespace);
    let mut result = property(
        &field.name,
        if field.required {
            value
        } else {
            optional(namespace, value)
        },
        Some("set"),
        required,
        initialization,
    );
    result["wire"] = json!(field.wire);
    result["sourceRequired"] = json!(field.required);
    result["jsonNullAllowed"] = json!(field.nullable);
    result
}
fn object_constructor(name: &str, fields: &[Value]) -> Value {
    json!({"access":"public","type":name,"parameters":[],"style":"object-initializer",
        "requiredMembers":fields.iter().filter(|f|f["required"] == true).map(|f|f["name"].clone()).collect::<Vec<_>>()})
}
fn union_arm(models: &ModelPlan, namespace: &str, parent: &str, branch: &CsBranch) -> Value {
    let ty = type_of(models, &branch.ty, namespace);
    json!({"kind":"class","name":format!("{parent}.{}",branch.name),"access":"public","sealed":true,"base":parent,
        "fields":[property("Value",ty.clone(),None,false,json!({"kind":"constructor-argument","name":"value"}))],
        "constructor":{"access":"public","parameters":[{"name":"value","type":ty,"required":true}]},
        "encodingValidation":"selected-arm-and-parent"})
}

fn model(plan: &SdkPlan, key: &Key, decl: &CsDecl) -> Value {
    let namespace = &plan.package().namespace;
    let models = plan.models();
    let name = qualified(namespace, &models.names()[key]);
    let mut result = match decl {
        CsDecl::Alias(inner) => {
            json!({"kind":"erased-alias","publicDeclaration":false,"type":type_of(models,inner,namespace)})
        }
        CsDecl::Record { fields, extras } => {
            let mut fields = fields
                .iter()
                .map(|field| model_field(models, field, namespace))
                .collect::<Vec<_>>();
            if let Some(extra) = extras {
                fields.push(property(
                    "Extra",
                    type_of(models, &CsType::Dict(Box::new(extra.clone())), namespace),
                    Some("set"),
                    false,
                    json!({"kind":"empty-dictionary","comparer":"System.StringComparer.Ordinal"}),
                ));
            }
            json!({"kind":"record","access":"public","sealed":true,"constructor":object_constructor(&name,&fields),"fields":fields,
                "copyWith":true,"encodingValidation":"current-mutable-value"})
        }
        CsDecl::Literals { values } => {
            json!({"kind":"enum","access":"public","underlyingType":"int",
            "members":values.iter().enumerate().map(|(ordinal,(member,token))|json!({"kind":"literal","name":member,"value":ordinal,"token":token})).collect::<Vec<_>>() })
        }
        CsDecl::Union { branches } => {
            json!({"kind":"union","representation":"abstract-class","access":"public","closed":true,
            "constructor":{"access":"private","parameters":[]},
            "variants":branches.iter().map(|branch|union_arm(models,namespace,&name,branch)).collect::<Vec<_>>() })
        }
    };
    result["obsolete"] = json!(models.is_deprecated(key));
    if matches!(
        plan.program().version,
        suspect_schema::OwnedProgram::V2_VERSION | suspect_schema::OwnedProgram::V3_VERSION
    ) {
        result["sourceValidation"] = json!({"version":plan.program().version,"profile":plan.program().profile,
            "decode":"complete-source-program","encode":"current-mutable-value","scope":"fresh-per-subschema","jsonCarrier":models.is_json_carrier(&key.0)});
        if matches!(decl, CsDecl::Record { .. })
            && plan
                .contract()
                .source(&key.0)
                .and_then(|v| v.get("patternProperties"))
                .and_then(Value::as_object)
                .is_some_and(|p| !p.is_empty())
        {
            result["extraValidation"] =
                json!("all-matching-patterns-and-unmatched-additional-properties");
        }
    }
    if plan.program().version == suspect_schema::OwnedProgram::V3_VERSION {
        result["sourceValidation"]["dynamicScope"] = json!({"binding":"outermost-actually-entered-resource","cycleIdentity":"node-instance-ordered-resources","candidateSchemas":"validation-only","nullability":"conservative-native-domain-when-context-sensitive","unionTrials":"context-sensitive-unions-use-json-carriers","acquisition":false});
    }
    result
}

fn input_property(plan: &SdkPlan, name: &str, schema: &SchemaId, required: bool) -> Value {
    let value = type_at(plan.models(), schema, &plan.package().namespace);
    property(
        name,
        if required {
            value
        } else {
            optional(&plan.package().namespace, value)
        },
        Some("set"),
        required,
        json!({"kind":if required {"required-member"} else {"absent"}}),
    )
}
fn parameter(name: &str, ty: Value, required: bool, default: Value) -> Value {
    json!({"name":name,"type":ty,"required":required,"default":default})
}
fn media_type(plan: &SdkPlan, media: &csharp_sdk::protocol::PlannedMedia) -> Value {
    use crate::http_protocol::Representation;
    let ns = &plan.package().namespace;
    match media.wire.representation(){
        Representation::Json{codec}=>codec.as_ref().map_or(json!({"kind":"primitive","name":"System.Text.Json.JsonElement","jsonNullAllowed":true}),|c|type_at(plan.models(),c.schema().id(),ns)),
        Representation::Text{codec,..}=>codec.as_ref().map_or(json!({"kind":"primitive","name":"string"}),|c|type_at(plan.models(),c.schema().id(),ns)),
        Representation::Binary{..}=>json!({"kind":"array","type":{"kind":"primitive","name":"byte"}}),
        Representation::Stream{stream}=>json!({"kind":"generic","name":qualified(ns,"HttpStream"),"arguments":[type_at(plan.models(),stream.item_codec().schema().id(),ns)],"interfaces":["System.Collections.Generic.IAsyncEnumerable","System.IAsyncDisposable"]}),
        _=>named(qualified(ns,&media.native_type)),
    }
}
fn response_type(plan: &SdkPlan, response: &csharp_sdk::PlannedResponse) -> Value {
    if response.always_empty || response.union {
        named(qualified(&plan.package().namespace, &response.native_type))
    } else if response.media.is_empty() {
        json!({"kind":"array","type":{"kind":"primitive","name":"byte"}})
    } else {
        media_type(plan, &response.media[0])
    }
}
fn protocol_models(plan: &SdkPlan, snapshot: &mut NativeSnapshot) {
    use crate::http_protocol::{PartMultiplicity, PartRepresentation};
    use csharp_sdk::protocol::{PlannedHeader, PlannedMedia, PlannedPart};
    let ns = &plan.package().namespace;
    let mut seen = BTreeSet::new();
    let mut add = |name: &str, source: &SourceId, role: &str, value: Value| {
        if seen.insert((name.to_owned(), role.to_owned())) {
            snapshot.models.push(NativeModel {
                source: Location::at(plan.contract(), source),
                name: qualified(ns, name),
                role: role.into(),
                descriptor: Some(value),
            });
        }
    };
    let header_fields = |headers: &[PlannedHeader]| {
        headers
            .iter()
            .map(|h| {
                let ty = type_at(plan.models(), h.wire.codec().schema().id(), ns);
                let mut f = property(
                    &h.property_name,
                    if h.wire.required() {
                        ty
                    } else {
                        optional(ns, ty)
                    },
                    Some("set"),
                    h.wire.required(),
                    json!({"kind":if h.wire.required(){"required-member"}else{"absent"}}),
                );
                f["wire"] = json!(h.wire.name());
                f
            })
            .collect::<Vec<_>>()
    };
    let part_value = |part: &PlannedPart| match part.wire.representation() {
        PartRepresentation::Binary { .. } => {
            json!({"kind":"array","type":{"kind":"primitive","name":"byte"}})
        }
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => {
            type_at(plan.models(), codec.schema().id(), ns)
        }
    };
    let part_type = |part: &PlannedPart| {
        let item = part
            .wrapper_type
            .as_ref()
            .map_or_else(|| part_value(part), |name| named(qualified(ns, name)));
        if part.wire.multiplicity() == PartMultiplicity::RepeatedArrayItems {
            json!({"kind":"generic","name":"System.Collections.Generic.List","arguments":[item]})
        } else {
            item
        }
    };
    let media_union = |media: &[PlannedMedia], none: bool, bytes: bool| {
        let mut variants=media.iter().map(|m|json!({"name":m.variant_name,"type":media_type(plan,m),"fields":[property("Value",media_type(plan,m),None,false,json!({"kind":"constructor-argument","name":"value"})),property("ContentType",nullable(json!({"kind":"primitive","name":"string"})),None,false,json!({"kind":"constructor-argument","name":"contentType"}))],"constructor":{"access":"public","parameters":[parameter("value",media_type(plan,m),true,Value::Null),parameter("contentType",nullable(json!({"kind":"primitive","name":"string"})),false,json!({"kind":"literal","value":null}))]}})).collect::<Vec<_>>();
        if none {
            variants.push(json!({"name":"NoContent","type":named(qualified(ns,"HttpNoContent")),"constructor":{"access":"public"}}));
        }
        if bytes {
            variants.push(json!({"name":"UndeclaredBytes","type":{"kind":"array","type":{"kind":"primitive","name":"byte"}},"constructor":{"access":"public"}}));
        }
        json!({"kind":"union","representation":"abstract-class","closed":true,"constructor":{"access":"private","parameters":[]},"variants":variants})
    };
    for op in plan.operations() {
        if let Some(body) = &op.body
            && body.union
        {
            add(
                &body.native_type,
                &body.source,
                "request-media-union",
                media_union(&body.media, false, false),
            );
        }
        for response in &op.responses {
            if response.union {
                add(
                    &response.native_type,
                    &response.source,
                    "response-body-union",
                    media_union(
                        &response.media,
                        response.may_be_empty,
                        response.media.is_empty(),
                    ),
                );
            }
            if let Some(name) = &response.header_type {
                let fields = header_fields(&response.headers);
                add(
                    name,
                    &response.source,
                    "response-headers",
                    json!({"kind":"record","sealed":true,"constructor":object_constructor(&qualified(ns,name),&fields),"fields":fields}),
                );
            }
        }
        for media in op
            .body
            .iter()
            .flat_map(|b| &b.media)
            .chain(op.responses.iter().flat_map(|r| &r.media))
        {
            if let Some(parts) = &media.positional {
                let mut fields=parts.prefix.iter().map(|part|{
                    let ty=part_type(part);
                    property(&part.property_name,if part.wire.required(){ty}else{optional(ns,ty)},Some("set"),part.wire.required(),json!({"kind":if part.wire.required(){"required-member"}else{"absent"}}))
                }).collect::<Vec<_>>();
                if let Some(items) = &parts.items {
                    fields.push(property("Items",json!({"kind":"generic","name":"System.Collections.Generic.List","arguments":[part_type(items)]}),Some("set"),false,json!({"kind":"empty-list"})));
                }
                add(
                    &parts.native_type,
                    parts.schema.id(),
                    "positional-parts",
                    json!({"kind":"record","sealed":true,"ordered":true,"prefixLength":parts.prefix.len(),"closedAfterPrefix":parts.items.is_none(),"minimumCount":parts.min_items.as_ref().map(|n|n.value()),"maximumCount":parts.max_items.as_ref().map(|n|n.value()),"constructor":object_constructor(&qualified(ns,&parts.native_type),&fields),"fields":fields}),
                );
                for part in parts
                    .prefix
                    .iter()
                    .chain(parts.items.iter().map(|p| p.as_ref()))
                {
                    let wrapper = part
                        .wrapper_type
                        .as_ref()
                        .expect("positional values have typed wrappers");
                    let mut fields = vec![
                        property(
                            "Value",
                            part_value(part),
                            Some("set"),
                            true,
                            json!({"kind":"required-member"}),
                        ),
                        property(
                            "FileName",
                            nullable(json!({"kind":"primitive","name":"string"})),
                            Some("set"),
                            false,
                            json!({"kind":"literal","value":null}),
                        ),
                        property(
                            "ContentType",
                            nullable(json!({"kind":"primitive","name":"string"})),
                            Some("set"),
                            false,
                            json!({"kind":"literal","value":null}),
                        ),
                    ];
                    if let Some(h) = &part.header_type {
                        let required = part.headers.iter().any(|h| h.wire.required());
                        fields.push(property(
                            "Headers",
                            named(qualified(ns, h)),
                            Some("set"),
                            required,
                            json!({"kind":if required{"required-member"}else{"default-instance"}}),
                        ));
                    }
                    add(
                        wrapper,
                        part.wire.source().use_site().source(),
                        "positional-part",
                        json!({"kind":"record","sealed":true,"constructor":object_constructor(&qualified(ns,wrapper),&fields),"fields":fields}),
                    );
                    if let Some(name) = &part.header_type {
                        let fields = header_fields(&part.headers);
                        add(
                            name,
                            part.wire.source().use_site().source(),
                            "part-headers",
                            json!({"kind":"record","sealed":true,"constructor":object_constructor(&qualified(ns,name),&fields),"fields":fields}),
                        );
                    }
                }
            }
            let Some(parts) = &media.parts else {
                continue;
            };
            let mut fields = Vec::new();
            for part in &parts.fields {
                let ty = part_type(part);
                let mut f = property(
                    &part.property_name,
                    if part.wire.required() {
                        ty
                    } else {
                        optional(ns, ty)
                    },
                    Some("set"),
                    part.wire.required(),
                    json!({"kind":if part.wire.required(){"required-member"}else{"absent"}}),
                );
                f["wire"] = json!(part.wire.name());
                fields.push(f);
            }
            if let Some(extra) = &parts.additional {
                fields.push(property("Extra",json!({"kind":"generic","name":"System.Collections.Generic.Dictionary","arguments":[{"kind":"primitive","name":"string"},part_type(extra)]}),Some("set"),false,json!({"kind":"empty-dictionary"})));
            }
            add(
                &parts.native_type,
                parts.rules.schema().id(),
                "named-parts",
                json!({"kind":"record","sealed":true,"multipart":parts.multipart,"constructor":object_constructor(&qualified(ns,&parts.native_type),&fields),"fields":fields}),
            );
            for part in parts
                .fields
                .iter()
                .chain(parts.additional.iter().map(|p| p.as_ref()))
            {
                if let Some(wrapper) = &part.wrapper_type {
                    let mut fields = vec![
                        property(
                            "Value",
                            part_value(part),
                            Some("set"),
                            true,
                            json!({"kind":"required-member"}),
                        ),
                        property(
                            "FileName",
                            nullable(json!({"kind":"primitive","name":"string"})),
                            Some("set"),
                            false,
                            json!({"kind":"literal","value":null}),
                        ),
                        property(
                            "ContentType",
                            nullable(json!({"kind":"primitive","name":"string"})),
                            Some("set"),
                            false,
                            json!({"kind":"literal","value":null}),
                        ),
                    ];
                    if let Some(h) = &part.header_type {
                        let required = part.headers.iter().any(|h| h.wire.required());
                        fields.push(property(
                            "Headers",
                            named(qualified(ns, h)),
                            Some("set"),
                            required,
                            json!({"kind":if required{"required-member"}else{"default-instance"}}),
                        ));
                    }
                    add(
                        wrapper,
                        part.wire.source().use_site().source(),
                        "multipart-part",
                        json!({"kind":"record","sealed":true,"constructor":object_constructor(&qualified(ns,wrapper),&fields),"fields":fields}),
                    );
                }
                if let Some(name) = &part.header_type {
                    let fields = header_fields(&part.headers);
                    add(
                        name,
                        part.wire.source().use_site().source(),
                        "part-headers",
                        json!({"kind":"record","sealed":true,"constructor":object_constructor(&qualified(ns,name),&fields),"fields":fields}),
                    );
                }
            }
        }
    }
}
fn operation(plan: &SdkPlan, operation: &PlannedOperation) -> NativeOperation {
    let namespace = &plan.package().namespace;
    let input = qualified(namespace, &operation.input_type);
    let result = qualified(namespace, &operation.result_type);
    let error = qualified(namespace, &operation.error_type);
    let token = named("System.Threading.CancellationToken");
    let cancellation = || {
        parameter(
            "cancellationToken",
            token.clone(),
            false,
            json!({"kind":"default-value"}),
        )
    };
    let mut overloads = vec![
        json!({"name":operation.method_name,"access":"public","static":false,
        "parameters":[parameter("input",named(&input),true,Value::Null),
            parameter("requestOptions",nullable(named(qualified(namespace,"RequestOptions"))),false,json!({"kind":"literal","value":null})),cancellation()],
        "returns":{"kind":"generic","name":"System.Threading.Tasks.Task","arguments":[named(&result)]}}),
    ];
    if operation.has_no_input_overload() {
        overloads.push(json!({"name":operation.method_name,"access":"public","static":false,
            "parameters":[cancellation()],"omittedInputs":"absent",
            "returns":{"kind":"generic","name":"System.Threading.Tasks.Task","arguments":[named(&result)]}}));
    }
    let parameters = operation
        .parameters
        .iter()
        .map(|p| {
            let mut property = input_property(plan, &p.property_name, &p.schema, p.required);
            // These keys also let the shared comparator distinguish wire-only spelling/location changes.
            property["member"] = json!(p.property_name);
            property["model"] = type_at(plan.models(), &p.schema, namespace);
            property["wire"] = json!(p.wire_name);
            property["location"] = json!(format!("{:?}", p.location));
            property
        })
        .collect::<Vec<_>>();
    let body = operation.body.as_ref().map(|body| {
        let ty = if body.union {
            named(qualified(namespace, &body.native_type))
        } else {
            media_type(plan, &body.media[0])
        };
        property(
            "Body",
            if body.required {
                ty
            } else {
                optional(namespace, ty)
            },
            Some("set"),
            body.required,
            json!({"kind":if body.required{"required-member"}else{"absent"}}),
        )
    });
    let fields = parameters
        .iter()
        .cloned()
        .chain(body.clone())
        .collect::<Vec<_>>();
    let single_success = operation.direct_result();
    let responses = operation.responses.iter().flat_map(|r| [true,false].into_iter().filter(move |success|if *success{r.may_succeed()}else{r.may_fail()}).map(move |success| {
        let ty=response_type(plan,r);
        let mut fields=vec![json!({"name":"Data","type":ty,"access":"public","getter":true,"setter":null,"readonly":true,"hidesInheritedMember":!success})];
        if let Some(h)=&r.header_type { fields.push(property("Headers",named(qualified(namespace,h)),None,false,json!({"kind":"constructor-argument","name":"headers"}))); }
        let mut args=vec![parameter("data",ty,true,Value::Null),parameter(if success{"metadata"}else{"raw"},named(qualified(namespace,if success{"ResponseMetadata"}else{"WireResponse"})),true,Value::Null)];
        if let Some(h)=&r.header_type{args.push(parameter("headers",named(qualified(namespace,h)),true,Value::Null));}
        if success&&r.media.iter().any(|m|m.is_stream()){args.push(parameter("lease",nullable(named(qualified(namespace,"StreamLease"))),true,Value::Null));}
        json!({"class":qualified(namespace,if success{&r.type_name}else{&r.error_type_name}),"status":r.wire.status(),"success":success,"kind":"sealed-class",
            "base":if success&&single_success{Value::Null}else{json!(qualified(namespace,if success{&operation.result_type}else{&operation.error_type}))},"fields":fields,"constructor":{"access":"internal","parameters":args},
            "bodyDisposition":{"alwaysEmpty":r.always_empty,"mayBeEmpty":r.may_be_empty,"union":r.union},"media":r.media.iter().map(|m|json!({"variant":m.variant_name,"type":media_type(plan,m)})).collect::<Vec<_>>()})
    })).collect::<Vec<_>>();
    let credentials = operation
        .wire
        .security()
        .alternatives()
        .iter()
        .map(|a| {
            a.requirements()
                .iter()
                .map(|r| credential(plan, &plan.credential_key(r)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    NativeOperation {
        source: Location::at(plan.contract(), &operation.source),
        operation_id: operation.operation_id.clone(),
        symbols: BTreeMap::from([
            ("client".into(), qualified(namespace, "Client")),
            ("method".into(), operation.method_name.clone()),
            ("input".into(), input.clone()),
            ("success".into(), result.clone()),
            ("api-error".into(), error.clone()),
        ]),
        descriptor: json!({
            "constructor":{"input":object_constructor(&input,&fields),"inputDeclaration":{"kind":"record","sealed":true,"fields":fields},
                "client":{"type":qualified(namespace,"Client"),"access":"public","sealed":true,"interfaces":["System.IDisposable"],
                    "parameters":[parameter("credentials",named(qualified(namespace,"Credentials")),true,Value::Null),
                        parameter("options",nullable(named(qualified(namespace,"ClientOptions"))),false,json!({"kind":"literal","value":null})),
                        parameter("httpClient",nullable(named("System.Net.Http.HttpClient")),false,json!({"kind":"literal","value":null}))]}},
            "parameters":{"properties":parameters,"overloads":overloads},"body":body,
            "responseUnions":{"success":{"name":result,"kind":if single_success {"sealed-class"} else {"abstract-class"},
                    "directResult":single_success,"closed":true,"constructorAccess":if single_success {"internal"} else {"private"},
                    "fields":[property("Metadata",named(qualified(namespace,"ResponseMetadata")),None,false,json!({"kind":"constructor-argument","name":"metadata"})),
                        property("Status",json!({"kind":"primitive","name":"int"}),None,false,json!({"kind":"computed","from":"Metadata.Status"}))]},
                "apiError":{"name":error,"kind":"abstract-class","base":qualified(namespace,"ApiException"),"closed":true,"constructorAccess":"private"}},
            "responses":responses,
            "credential":{"alternatives":credentials,"selection":"RequestOptions.SecurityAlternative"},
            "serverResolution":{"base":"effective-physical-server-document","metadataType":qualified(namespace,"ServerInfo"),
                "documentProperty":"DocumentUrl","logicalResourceProperty":"Resource","explicitOverrides":["ClientOptions.DocumentUrl","RequestOptions.DocumentUrl","ClientOptions.ServerUrl","RequestOptions.ServerUrl"],
                "pathPolicy":"rfc3986-literal-dots-preserve-percent-encoded-data","credentialEndpointBase":"effective-server","credentialServerProperty":"CredentialContext.ServerUrl"},
        }),
    }
}
fn credential(plan: &SdkPlan, scheme: &str) -> Value {
    let namespace = &plan.package().namespace;
    let binding = plan
        .credential_bindings()
        .iter()
        .find(|c| c.key == scheme)
        .expect("retained credential binding");
    let mut value = property(
        &plan.credentials()[scheme],
        nullable(if binding.native_type == "string" {
            json!({"kind":"primitive","name":"string"})
        } else {
            named(qualified(namespace, &binding.native_type))
        }),
        Some("init"),
        false,
        json!({"kind":"literal","value":null}),
    );
    value["owner"] = json!(qualified(namespace, "Credentials"));
    value["ownerKind"] = json!("sealed-record");
    value["ownerConstructor"] = json!({"access":"public","parameters":[]});
    value["wire"] = json!(binding.requirement.name());
    value["runtimeRequired"] = json!(true);
    value
}

/// Capture precisely the configured C# package surface. Rendering is admission
/// preflight; its C# output is never parsed to discover names, shapes or defaults.
pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> Result<(), Vec<PlanFinding>> {
    let errors = |errors: Vec<csharp_sdk::HttpDiagnostic>| {
        errors
            .into_iter()
            .map(|error| PlanFinding {
                code: error.code.into(),
                source: Some(Location::at(&contract, &error.source)),
                message: error.message,
            })
            .collect::<Vec<_>>()
    };
    let plan = csharp_sdk::plan_sdk_with_options(
        contract.clone(),
        selected,
        backend::csharp_config(&snapshot.target),
        backend::csharp_options(&snapshot.generation),
    )
    .map_err(errors)?;
    plan.render().map_err(errors)?;
    snapshot.credential_env = plan
        .credential_env()
        .map(crate::credential_env::CredentialEnvPlan::semantic_descriptor);
    let namespace = &plan.package().namespace;
    let models = plan.models();
    for operation in plan.operations() {
        snapshot.operations.push(self::operation(&plan, operation));
    }
    protocol_models(&plan, snapshot);
    for (key, declaration) in models.declarations() {
        let source = Location::at(&contract, &key.0);
        let alias = matches!(declaration, CsDecl::Alias(_));
        snapshot.models.push(NativeModel {
            source: source.clone(),
            name: if alias {
                models.qualified_type(&CsType::Named(key.clone()), namespace)
            } else {
                qualified(namespace, &models.names()[key])
            },
            role: if alias {
                "erased-alias"
            } else {
                super::native_models::role(key.1)
            }
            .into(),
            descriptor: Some(model(&plan, key, declaration)),
        });
        if let CsDecl::Union { branches } = declaration {
            let parent = qualified(namespace, &models.names()[key]);
            for branch in branches {
                snapshot.models.push(NativeModel {
                    source: Location::at(&contract, &branch.source),
                    name: format!("{parent}.{}", branch.name),
                    role: "union-arm".into(),
                    descriptor: Some(union_arm(models, namespace, &parent, branch)),
                });
            }
        }
        let value_type = type_of(models, &CsType::Named(key.clone()), namespace);
        let codec = &models.names()[key];
        snapshot.models.push(NativeModel { source:source.clone(),name:qualified(namespace,&format!("Codecs.Decode{codec}")),role:"codec-decode".into(),
            descriptor:Some(json!({"kind":"static-method","access":"public","returns":value_type,
                "overloads":[{"parameters":[parameter("json",json!({"kind":"generic","name":"System.ReadOnlySpan","arguments":[{"kind":"primitive","name":"byte"}]}),true,Value::Null)]},
                    {"parameters":[parameter("json",json!({"kind":"primitive","name":"string"}),true,Value::Null)]}]})) });
        snapshot.models.push(NativeModel { source,name:qualified(namespace,&format!("Codecs.Encode{codec}")),role:"codec-encode".into(),
            descriptor:Some(json!({"kind":"static-method","access":"public","parameters":[parameter("value",value_type,true,Value::Null)],
                "returns":{"kind":"array","type":{"kind":"primitive","name":"byte"}},"validation":"current-mutable-value"})) });
    }
    let mut seen = BTreeSet::new();
    for binding in plan.credential_bindings() {
        let name = qualified(namespace, &format!("Credentials.{}", binding.property_name));
        let source = binding.requirement.scheme().terminal().source();
        if seen.insert((source.clone(), name.clone())) {
            snapshot.models.push(NativeModel {
                source: Location::at(&contract, source),
                name,
                role: "credential-property".into(),
                descriptor: Some(credential(&plan, &binding.key)),
            });
        }
    }
    Ok(())
}
