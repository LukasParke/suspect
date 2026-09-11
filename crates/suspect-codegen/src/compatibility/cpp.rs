//! C++ public interfaces from retained source-keyed plans, never emitted text.
//!
//! A nullable named shape owns a public value alias and a separate non-null
//! declaration. Codec owners always bind the full public value domain. References
//! stop at actual named declarations, so recursive Box/variant layouts stay finite.

use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use crate::{
    backend,
    cpp_sdk::{
        self, Constructor, Extras, Field, InputConstructor, ModelPlan, ModelSymbol, PlannedHeader,
        PlannedPart, PlannedResponse, PlannedResponseCase, Shape, TagInitializer, ValueKind,
        ValueType,
    },
};

use super::{Location, NativeModel, NativeOperation, NativeSnapshot, PlanFinding};

// This is a source address, not an OwnedProgram node/codec index. Only allocated
// native names enter type descriptors; source correspondence lives in Location.
type SourceKey = SchemaId;

struct Types<'a> {
    plan: &'a ModelPlan,
    namespace: &'a str,
}

impl Types<'_> {
    fn value(&self, value: &ValueType) -> Value {
        match &value.kind {
            ValueKind::Model(key) => self.at(key),
            ValueKind::Json => self.runtime("JsonValue"),
            ValueKind::Scalar(kind) => match kind {
                crate::http_protocol::ScalarType::String => primitive("std::string"),
                crate::http_protocol::ScalarType::Boolean => primitive("bool"),
                crate::http_protocol::ScalarType::Integer => self.runtime("JsonInteger"),
                crate::http_protocol::ScalarType::Number => self.runtime("JsonNumber"),
            },
            ValueKind::Bytes => {
                json!({"kind":"bytes","name":self.qualified("Bytes"),"storage":"std::vector<std::uint8_t>","jsonStandIn":false})
            }
            ValueKind::Unit => self.runtime("Unit"),
            ValueKind::Aggregate(name) | ValueKind::Choice(name) => {
                json!({"kind":"named","name":self.qualified(name)})
            }
            ValueKind::Stream {
                schema,
                framing,
                max_item_bytes,
            } => {
                json!({"kind":"sequential-items","item":self.at(schema),"framing":format!("{framing:?}"),"cppType":value.cpp_type,"maxItemBytes":max_item_bytes,"sharedCodecBudgets":true,"sentinel":false,"dataJsonInference":false})
            }
        }
    }
    fn binding(&self, value: &ValueType) -> Value {
        json!({"type":self.value(value),"cppType":value.cpp_type,
        "model":value.schema().map(|id|self.model_name(id)),"codec":value.schema().map(|id|self.qualified(self.symbol(id).codec().owner))})
    }
    fn input_constructor(&self, constructor: &InputConstructor) -> Value {
        json!({"name":constructor.name,"owner":self.qualified(&constructor.name),"arguments":"positional",
        "explicit":!constructor.parameters.is_empty(),"defaulted":constructor.parameters.is_empty(),"canInitializeWithoutArguments":constructor.parameters.is_empty(),
        "parameters":constructor.parameters.iter().map(|p|json!({"name":p.name,"member":p.member_name,"type":self.value(&p.value),"cppType":p.cpp_type,"passing":"value","hasDefault":false,"required":true})).collect::<Vec<_>>()})
    }
    fn header_fields(&self, fields: &[PlannedHeader]) -> Value {
        json!(fields.iter().map(|h|json!({"name":h.field_name,"wire":h.wire.name(),"type":self.presence(self.value(&h.value),h.wire.required()),"required":h.wire.required(),"mutable":true,"codec":h.value.schema().map(|id|self.qualified(self.symbol(id).codec().owner))})).collect::<Vec<_>>())
    }
    fn part(&self, part: &PlannedPart) -> Value {
        json!({"name":part.field_name,"wire":part.name,"value":self.binding(&part.value),"type":self.presence(self.part_value(part),part.wire.required()),
        "partType":part.part_type.as_ref().map(|name|self.qualified(name)),"headerType":part.headers_type.as_ref().map(|name|self.qualified(name)),"headers":self.header_fields(&part.headers),
        "required":part.wire.required(),"repeated":part.wire.multiplicity()==crate::http_protocol::PartMultiplicity::RepeatedArrayItems,
        "minimum":part.wire.min_items().map(|n|n.value()),"maximum":part.wire.max_items().map(|n|n.value()),"mutable":true,
        "contentTypes":part.wire.content_types().iter().map(|m|m.declared()).collect::<Vec<_>>()})
    }
    fn positional(&self, name: &str, parameters: Vec<Value>) -> Value {
        json!({"name":name,"owner":self.qualified(name),"arguments":"positional","explicit":!parameters.is_empty(),"defaulted":parameters.is_empty(),"canInitializeWithoutArguments":parameters.is_empty(),"parameters":parameters})
    }
    fn part_value(&self, part: &PlannedPart) -> Value {
        let value = part.part_type.as_ref().map_or_else(
            || self.value(&part.value),
            |name| json!({"kind":"named","name":self.qualified(name)}),
        );
        if part.wire.multiplicity() == crate::http_protocol::PartMultiplicity::RepeatedArrayItems {
            json!({"kind":"array","storage":"std::vector","item":value})
        } else {
            value
        }
    }
    fn header_constructor(&self, name: &str, fields: &[PlannedHeader]) -> Value {
        self.positional(name,fields.iter().filter(|h|h.wire.required()).enumerate().map(|(i,h)|json!({"name":format!("arg{i}"),"member":h.field_name,"type":self.value(&h.value),"passing":"value","hasDefault":false})).collect())
    }
    fn response_constructor(
        &self,
        response: &PlannedResponse,
        case: &PlannedResponseCase,
    ) -> Value {
        let mut parameters = vec![
            json!({"name":"value","member":"data","type":self.value(&case.value),"passing":"value","hasDefault":false}),
            json!({"name":"metadata","member":"response","type":self.runtime("ResponseMetadata"),"passing":"value","hasDefault":false}),
        ];
        if let Some(name) = &response.headers_type {
            parameters.push(json!({"name":"typed_headers","member":"headers","type":{"kind":"named","name":self.qualified(name)},"passing":"value","hasDefault":false}));
        }
        self.positional(&case.variant_type, parameters)
    }
    fn qualified(&self, name: &str) -> String {
        format!("::{}::{name}", self.namespace)
    }

    fn symbol(&self, key: &SourceKey) -> &ModelSymbol {
        self.plan.symbol(key).expect("admitted C++ source binding")
    }

    fn model_name(&self, key: &SourceKey) -> String {
        self.qualified(&self.symbol(key).name)
    }

    fn at(&self, key: &SourceKey) -> Value {
        let symbol = self.symbol(key);
        let value = match &symbol.shape {
            Shape::Json => self.runtime("JsonValue"),
            Shape::ValidatedJson => {
                json!({"kind":"validated-json","name":self.qualified("JsonValue"),"codecRequired":true,"mutableEncodeValidation":"whole-source-schema"})
            }
            Shape::Never => {
                json!({"kind":"never","name":self.qualified("Never"),"inhabited":false})
            }
            Shape::Null => self.runtime("Null"),
            Shape::Boolean => primitive("bool"),
            Shape::String => primitive("std::string"),
            Shape::Number | Shape::Integer => json!({"kind":"exact-number",
                "name":self.qualified(if matches!(symbol.shape,Shape::Integer){"JsonInteger"}else{"JsonNumber"}),
                "integral":matches!(symbol.shape,Shape::Integer),"exponent":"symbolic","implicitFloatingPoint":false}),
            Shape::Enum(_) | Shape::Object { .. } | Shape::Union { .. } => {
                json!({"kind":"named","name":self.qualified(&symbol.definition)})
            }
            Shape::Array(item) => json!({"kind":"array","name":"std::vector",
                "of":item.as_ref().map(|item|self.at(item)).unwrap_or_else(||self.runtime("JsonValue"))}),
            Shape::Ref { target, boxed } => {
                let target = self.at(target);
                if *boxed {
                    json!({"kind":"box","name":self.qualified("Box"),"of":target,
                        "ownership":"unique","storage":"std::unique_ptr","copy":"deep",
                        "move":"transfer-ownership","moveNoexcept":true,"destruction":"raii",
                        "movedFrom":"empty","encodeEmpty":"model-error",
                        "constructor":{"passing":"forwarded-exact-type","hasDefault":false}})
                } else {
                    target
                }
            }
        };
        if symbol.nullable {
            json!({"kind":"nullable","name":self.qualified("Nullable"),"storage":"std::variant",
                "alternatives":[self.runtime("Null"),value],"of":value,"defaultAlternative":self.qualified("Null")})
        } else {
            value
        }
    }

    fn runtime(&self, name: &str) -> Value {
        json!({"kind":"runtime","name":self.qualified(name)})
    }

    fn input(&self, key: &SourceKey, required: bool) -> Value {
        self.presence(self.at(key), required)
    }

    fn presence(&self, value: Value, required: bool) -> Value {
        if required {
            value
        } else {
            json!({"kind":"presence","name":self.qualified("Presence"),"storage":"std::optional",
                "of":value,"absent":"std::nullopt"})
        }
    }

    fn initializer(&self, field: &Field) -> Value {
        match &field.initializer {
            Some(TagInitializer {
                type_name,
                case_name,
                wire_value,
            }) => json!({
                "kind":"literal","model":self.qualified(type_name),"case":case_name,"value":wire_value,
                "origin":"source-singleton-tag","constructorArgument":false,
            }),
            None if field.required => json!({"kind":"argument"}),
            None => json!({"kind":"absent","expression":"std::nullopt"}),
        }
    }

    fn constructor(&self, constructor: &Constructor) -> Value {
        json!({"name":constructor.name,"owner":self.qualified(&constructor.name),
            "arguments":"positional","explicit":!constructor.parameters.is_empty(),
            "defaulted":constructor.parameters.is_empty(),"canInitializeWithoutArguments":constructor.parameters.is_empty(),
            "parameters":constructor.parameters.iter().map(|parameter|json!({
                "name":parameter.name,"member":parameter.member_name,"model":self.model_name(&parameter.source),
                "type":self.at(&parameter.source),"cppType":parameter.cpp_type,
                "passing":"value","required":true,"hasDefault":false,"initialization":"move-to-member",
            })).collect::<Vec<_>>()})
    }

    fn declaration(&self, symbol: &ModelSymbol) -> Value {
        let declaration = match &symbol.shape {
            Shape::Object { fields, extras } => {
                let extra_type = match extras {
                    Extras::Closed => None,
                    Extras::Any | Extras::Patterned => Some(self.runtime("JsonValue")),
                    Extras::Typed(source) => Some(self.at(source)),
                };
                json!({"kind":"struct","fields":fields.iter().map(|field|json!({
                    "name":field.name,"wire":field.wire,"required":field.required,
                    "type":self.input(&field.schema,field.required),"cppType":field.cpp_type,
                    "access":"public","mutable":true,"initialization":self.initializer(field),
                    "constructorParameter":field.required && field.initializer.is_none(),
                })).collect::<Vec<_>>(),
                "additionalProperties":extra_type.map(|value|json!({"name":"extra","mutable":true,"access":"public",
                    "type":{"kind":"map","name":"std::map","key":primitive("std::string"),"of":value,"comparator":"std::less<>"},
                    "initialization":{"kind":"empty-map"},"declaredKeyCollisions":"codec-error",
                    "wholeObjectPatternValidation":matches!(extras,Extras::Patterned)})),
                "constructor":self.constructor(symbol.constructor.as_ref().expect("C++ struct constructor"))})
            }
            Shape::Enum(values) => json!({"kind":"enum","scoped":true,"underlying":"int",
                "variants":values.iter().map(|(wire,name)|json!({"kind":"literal","name":name,"value":wire})).collect::<Vec<_>>(),
                "generatedConstructor":false,"encodeUnknownDiscriminant":"model-error"}),
            Shape::Union {
                branches,
                exactly_one,
            } => {
                // These indexed names are fixed emitter rules over source arm
                // order, not validator indices or inferred operation/model names.
                let variants=branches.iter().enumerate().map(|(ordinal,source)|json!({
                    "alias":format!("Alternative{ordinal}"),"factory":format!("alternative_{ordinal}"),
                    "static":true,"model":self.model_name(source),"type":self.at(source),
                    "cppType":self.symbol(source).cpp_type,"passing":"value",
                })).collect::<Vec<_>>();
                json!({"kind":"union","declaration":"struct","storage":"std::variant","variantAlias":"Variant",
                    "member":{"name":"value","access":"public","mutable":true},"variants":variants,
                    "constructor":{"name":symbol.definition,"owner":self.qualified(&symbol.definition),"explicit":true,
                        "canInitializeWithoutArguments":false,"parameters":[{"name":"selected","type":"Variant","passing":"value","hasDefault":false}]},
                    "validation":{"selectedArm":true,"parent":true,"exclusive":exactly_one,"decodeSelection":"first-valid-source-arm"}})
            }
            _ => unreachable!("only named C++ shapes have definitions"),
        };
        let mut declaration = declaration;
        declaration["valueSemantics"] = value_semantics();
        declaration
    }

    fn codec(&self, symbol: &ModelSymbol) -> Value {
        let codec = symbol.codec();
        let owner = self.qualified(codec.owner);
        let value_type = self.at(&symbol.source);
        let error = self.runtime(codec.error_type);
        json!({"kind":"model-codec","owner":owner,
            "valueAlias":{"name":codec.value_alias,"type":value_type,"cppType":codec.cpp_type},
            "sourceBound":true,"revalidatesMutableEncodes":true,
            "methods":[
                {"name":codec.methods.decode,"owner":owner,"static":true,
                    "parameters":[{"name":"bytes","type":primitive("std::string_view"),"passing":"value"}],
                    "result":self.result(value_type.clone(),error.clone())},
                {"name":codec.methods.encode,"owner":owner,"static":true,
                    "parameters":[{"name":"value","type":value_type,"passing":"const-reference"}],
                    "result":self.result(primitive("std::string"),error.clone())},
                {"name":codec.methods.to_json,"owner":owner,"static":true,
                    "parameters":[{"name":"value","type":value_type,"passing":"const-reference"}],
                    "result":self.result(self.runtime("JsonValue"),error)},
            ],
            "errorKinds":["InvalidJson","Validation","EvaluationFailure","Model","ResourceLimit","Cancelled","Timeout"]})
    }

    fn result(&self, value: Value, error: Value) -> Value {
        json!({"kind":"result","name":self.qualified("Result"),"storage":"std::variant","success":value,"error":error})
    }
}

fn primitive(name: &str) -> Value {
    json!({"kind":"primitive","name":name})
}

fn value_semantics() -> Value {
    json!({"ownership":"value","copy":"memberwise-value","move":"memberwise-move","destruction":"raii"})
}

pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> Result<(), Vec<PlanFinding>> {
    let mut config = backend::cpp_config(&snapshot.target);
    config.legacy_binary_strings = snapshot.generation.legacy_binary_strings();
    config.credential_env = snapshot.generation.credential_env.clone();
    let plan = cpp_sdk::plan_sdk(contract.clone(), selected, config).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| PlanFinding {
                code: error.code.into(),
                source: Some(Location::at(&contract, &error.source)),
                message: error.message,
            })
            .collect::<Vec<_>>()
    })?;
    snapshot.credential_env = plan
        .credential_env()
        .map(|policy| policy.semantic_descriptor());
    let types = Types {
        plan: plan.models(),
        namespace: &plan.config().namespace,
    };
    let package = &plan.config().name;
    for operation in plan.operations() {
        let mut constructor = types.input_constructor(&operation.constructor);
        constructor["valueSemantics"] = value_semantics();
        constructor["call"] = json!({"const":true,"execution":"synchronous",
        "parameters":[
            {"name":"input","type":types.qualified(&operation.input_type),"passing":"const-reference",
                "hasDefault":operation.constructor.parameters.is_empty(),
                "default":operation.constructor.parameters.is_empty().then_some("{}")},
            {"name":"options","type":types.qualified("CallOptions"),"passing":"value","hasDefault":true,"default":"{}"}
        ]});
        constructor["call"]["serverSelection"] = json!({"selector":"server_index","variables":"server_variables","relativeBase":"document_url","defaultBase":"effective-physical-server-document","logicalResourceRelocatesApi":false,"override":"server_url",
            "candidates":operation.wire.servers().candidates().iter().map(|server|json!({"template":server.template(),"name":server.name().map(|v|v.value()),
                "variables":server.variables().iter().map(|v|json!({"name":v.name(),"default":v.default().value(),"enum":v.values().map(|values|values.iter().map(|v|v.value()).collect::<Vec<_>>())})).collect::<Vec<_>>()
            })).collect::<Vec<_>>()});
        if plan.credential_env().is_some() {
            constructor["environmentFactories"] = json!({"from_env":{"owner":types.qualified("Client"),"static":true,"requires":"libcurl","parameters":[{"name":"options","type":types.runtime("ClientOptions"),"hasDefault":true},{"name":"curl","type":types.runtime("CurlOptions"),"hasDefault":true}],"returns":"Result<Client, TransportError>"},"from_env_with_transport":{"owner":types.qualified("Client"),"static":true,"parameters":[{"name":"transport","type":"std::shared_ptr<const Transport>","hasDefault":false},{"name":"options","type":types.runtime("ClientOptions"),"hasDefault":true}],"returns":types.qualified("Client")},"snapshot":"client-creation","explicitCredentials":"whole-argument-authoritative","maximumCredentialBytes":8192,"missingCredential":"deferred-protected-operation-preflight"});
        }
        let success = operation
            .responses
            .iter()
            .filter(|response| response.can_succeed())
            .flat_map(|response| {
                response
                    .cases
                    .iter()
                    .map(|case| types.qualified(&case.variant_type))
            })
            .collect::<Vec<_>>();
        let success = if success.is_empty() {
            vec![types.qualified("Never")]
        } else {
            success
        };
        let errors = std::iter::once(types.qualified("SdkError"))
            .chain(
                operation
                    .responses
                    .iter()
                    .filter(|response| response.can_fail())
                    .flat_map(|response| {
                        response
                            .cases
                            .iter()
                            .map(|case| types.qualified(&case.variant_type))
                    }),
            )
            .collect::<Vec<_>>();
        let symbols = BTreeMap::from([
            ("client".into(), types.qualified("Client")),
            ("method".into(), operation.method_name.clone()),
            ("input".into(), types.qualified(&operation.input_type)),
            (
                "input-constructor".into(),
                types.qualified(&operation.constructor.name),
            ),
            ("success".into(), types.qualified(&operation.success_type)),
            ("error".into(), types.qualified(&operation.error_type)),
            ("namespace".into(), plan.config().namespace.clone()),
            ("include".into(), format!("{package}/client.hpp")),
            ("entrypoint".into(), format!("{package}/sdk.hpp")),
            ("cmake-target".into(), format!("{package}::{package}")),
        ]);
        snapshot.operations.push(NativeOperation {
            source:Location::at(&contract,&operation.source), operation_id:operation.operation_id.clone(), symbols,
            descriptor:json!({
                "constructor":constructor,
                "parameters":operation.parameters.iter().map(|parameter|json!({
                    "member":parameter.field_name,"wire":parameter.wire_name,"location":format!("{:?}",parameter.wire.location()),
                    "required":parameter.required,"model":parameter.value.schema().map(|id|types.model_name(id)),
                    "type":types.presence(types.value(&parameter.value),parameter.required),"cppType":parameter.cpp_type,
                    "mutable":true,"codec":parameter.value.schema().map(|id|types.qualified(types.symbol(id).codec().owner)),
                    "initialization":{"kind":if parameter.required {"argument"}else{"absent"}},
                })).collect::<Vec<_>>(),
                "body":operation.body.as_ref().map(|body|json!({"member":"body","required":body.required,
                    "model":body.value.schema().map(|id|types.model_name(id)),"type":types.presence(types.value(&body.value),body.required),"cppType":body.cpp_type,
                    "mutable":true,"codec":body.value.schema().map(|id|types.qualified(types.symbol(id).codec().owner)),
                    "choiceType":body.choice_type.as_ref().map(|name|types.qualified(name)),
                    "media":body.media.iter().map(|m|json!({"wrapperType":body.choice_type.as_ref().map(|_|types.qualified(&m.wrapper_type)),"contentTypeRequired":m.requires_content_type,"value":types.binding(&m.value),"mediaType":m.wire.media_type().declared(),
                        "constructor":body.choice_type.as_ref().map(|_|types.positional(&m.wrapper_type,std::iter::once(json!({"name":"value","member":"data","type":types.value(&m.value),"passing":"value","hasDefault":false})).chain(m.requires_content_type.then(||json!({"name":"content_type_value","member":"content_type","type":primitive("std::string"),"passing":"value","hasDefault":false}))).collect()))})).collect::<Vec<_>>(),
                    "initialization":{"kind":if body.required {"argument"}else{"absent"}}})),
                "responseUnions":{
                    "success":{"name":types.qualified(&operation.success_type),"kind":"variant","storage":"std::variant","alternatives":success,"singleAlternativeUnwrapped":false},
                    "error":{"name":types.qualified(&operation.error_type),"kind":"variant","storage":"std::variant","alternatives":errors},
                    "result":types.result(json!({"kind":"named","name":types.qualified(&operation.success_type)}),json!({"kind":"named","name":types.qualified(&operation.error_type)})),
                    "sdkError":{"name":types.qualified("SdkError"),"kinds":["Configuration","RequestValidation","RequestRepresentation","Transport","ResourceLimit","UnexpectedResponse","ResponseDecoding","Cancelled","Timeout"],
                        "codec":types.qualified("CodecError"),"transport":types.qualified("TransportError"),"cause":"std::exception_ptr","response":types.qualified("ResponseMetadata")}
                },
                "responses":operation.responses.iter().flat_map(|response|response.cases.iter().map(|case|json!({"name":types.qualified(&case.variant_type),
                    "statusRule":response.status_key,"actualStatusPreserved":true,"httpBodyForbidden":case.forbidden,"mediaType":case.media.as_ref().map(|m|m.media_type().declared()),"binding":types.binding(&case.value),
                    "successMember":response.can_succeed(),"errorMember":response.can_fail(),"classification":"actual-status",
                    "headersType":response.headers_type.as_ref().map(|name|types.qualified(name)),"headers":types.header_fields(&response.headers),
                    "links":response.wire.links().iter().map(|link|json!({"name":link.name(),"kind":"literal","value":{
                        "target":match link.target(){crate::http_protocol::LinkTarget::OperationId{value,..}|crate::http_protocol::LinkTarget::OperationRef{value,..}=>value.value()},
                        "parameters":link.parameters().iter().map(|(key,value)|(key.clone(),value.value().clone())).collect::<serde_json::Map<_,_>>(),"requestBody":link.request_body().map(|v|v.value())}})).collect::<Vec<_>>(),
                    "constructor":types.response_constructor(response,case),
                    "valueSemantics":if matches!(case.value.kind,ValueKind::Stream{..}){json!({"copy":false,"move":true,"ownership":"raii-transfer","rangeBreakCloses":true})}else{value_semantics()},
                }))).collect::<Vec<_>>(),
                "credential":{"kind":"source-alternatives","owner":types.qualified("Credentials"),"ambientDiscovery":false,"automaticAcquisition":false,
                    "fields":plan.credentials().values().map(|credential|json!({"name":credential.field_name,"wire":credential.wire.name(),"type":credential.cpp_type,"optional":true,
                        "provider":matches!(credential.wire.credential(),crate::http_protocol::CredentialHook::OAuth2{..}|crate::http_protocol::CredentialHook::OpenIdConnect{..})})).collect::<Vec<_>>(),
                    "providerSignature":"Result<Authorization, TransportError>(const CredentialRequest&)",
                    "providerUrlContext":{"effective_server_url":"std::string","metadata_url_base":"effective-server","logicalResourceRelocatesApi":false},
                    "alternatives":operation.wire.security().alternatives().iter().map(|a|json!({"anonymous":a.is_anonymous(),"all":a.requirements().iter().map(|r|plan.credential_for(r).field_name.clone()).collect::<Vec<_>>(),
                        "requirements":a.requirements().iter().map(|r|json!({"member":plan.credential_for(r).field_name,
                            "permissions":match r.permissions(){crate::http_protocol::Permissions::Scopes(values)=>json!({"scopes":values.iter().map(|v|v.value()).collect::<Vec<_>>()}),crate::http_protocol::Permissions::Roles(values)=>json!({"roles":values.iter().map(|v|v.value()).collect::<Vec<_>>()})},
                            "attachment":match r.credential(){crate::http_protocol::CredentialHook::Bearer{..}=>json!({"kind":"bearer"}),crate::http_protocol::CredentialHook::Basic=>json!({"kind":"basic"}),crate::http_protocol::CredentialHook::ApiKey{location,name}=>json!({"kind":"api-key","location":format!("{location:?}"),"name":name.value()}),crate::http_protocol::CredentialHook::OAuth2{..}=>json!({"kind":"oauth2-provider"}),crate::http_protocol::CredentialHook::OpenIdConnect{discovery_url}=>json!({"kind":"oidc-provider","discoveryUrl":discovery_url.value()})}
                        })).collect::<Vec<_>>()
                    })).collect::<Vec<_>>()},
            }),
        });
    }
    for aggregate in plan.aggregates() {
        snapshot.models.push(NativeModel{source:Location::at(&contract,&aggregate.source),name:types.qualified(&aggregate.type_name),role:"aggregate".into(),descriptor:Some(json!({
            "kind":if aggregate.multipart{"named-multipart"}else{"form"},"fields":aggregate.fields.iter().map(|part|types.part(part)).collect::<Vec<_>>(),
            "additional":aggregate.additional.as_ref().map(|part|json!({"name":"extra","type":{"kind":"map","storage":"std::map","key":primitive("std::string"),"value":types.part_value(part)},"part":types.part(part)})),
            "constructor":types.positional(&aggregate.type_name,aggregate.fields.iter().filter(|p|p.wire.required()).enumerate().map(|(i,p)|json!({"name":format!("arg{i}"),"member":p.field_name,"type":types.part_value(p),"passing":"value","hasDefault":false})).collect()),
            "minProperties":aggregate.rules.min_properties().map(|v|v.value()),"maxProperties":aggregate.rules.max_properties().map(|v|v.value()),"binaryJsonStandIn":false,
        }))});
        for part in aggregate.fields.iter().chain(aggregate.additional.iter()) {
            if let Some(name) = &part.part_type {
                let mut arguments = vec![
                    json!({"name":"arg0","member":"data","type":types.value(&part.value),"passing":"value","hasDefault":false}),
                ];
                if let Some(headers) = &part.headers_type
                    && part.headers.iter().any(|h| h.wire.required())
                {
                    arguments.push(json!({"name":"arg1","member":"headers","type":{"kind":"named","name":types.qualified(headers)},"passing":"value","hasDefault":false}));
                }
                snapshot.models.push(NativeModel{source:Location::at(&contract,&part.source),name:types.qualified(name),role:"part".into(),descriptor:Some(json!({"kind":"mime-part","data":types.binding(&part.value),"filename":{"type":"Presence<std::string>"},"contentType":{"type":"Presence<std::string>"},
                    "headers":types.header_fields(&part.headers),"constructor":types.positional(name,arguments),"ownership":"value"}))});
            }
            if let Some(name) = &part.headers_type {
                snapshot.models.push(NativeModel{source:Location::at(&contract,&part.source),name:types.qualified(name),role:"part-headers".into(),descriptor:Some(json!({"kind":"struct","fields":types.header_fields(&part.headers),"constructor":types.header_constructor(name,&part.headers)}))});
            }
        }
    }
    for response in plan.operations().iter().flat_map(|op| &op.responses) {
        if let Some(name) = &response.headers_type {
            snapshot.models.push(NativeModel{source:Location::at(&contract,&response.source),name:types.qualified(name),role:"response-headers".into(),descriptor:Some(json!({"kind":"struct","fields":types.header_fields(&response.headers),"constructor":types.header_constructor(name,&response.headers)}))});
        }
    }
    for symbol in plan.models().symbols() {
        let source = Location::at(&contract, &symbol.source);
        let mut descriptor = if symbol.has_definition() && !symbol.nullable {
            types.declaration(symbol)
        } else {
            json!({"kind":"alias","type":types.at(&symbol.source),"generatedConstructor":false,"valueSemantics":value_semantics()})
        };
        descriptor["cppType"] = json!(symbol.cpp_type);
        descriptor["include"] = json!(format!("{package}/models.hpp"));
        snapshot.models.push(NativeModel {
            source: source.clone(),
            name: types.qualified(&symbol.name),
            role: "model".into(),
            descriptor: Some(descriptor),
        });
        if symbol.has_definition() && symbol.nullable {
            let mut descriptor = types.declaration(symbol);
            descriptor["include"] = json!(format!("{package}/models.hpp"));
            descriptor["cppType"] = json!(types.qualified(&symbol.definition));
            snapshot.models.push(NativeModel {
                source: source.clone(),
                name: types.qualified(&symbol.definition),
                role: "non-null-value".into(),
                descriptor: Some(descriptor),
            });
        }
        snapshot.models.push(NativeModel {
            source,
            name: types.qualified(symbol.codec().owner),
            role: "codec".into(),
            descriptor: Some(types.codec(symbol)),
        });
    }
    Ok(())
}

#[cfg(test)]
mod credential_env_tests {
    use super::*;

    #[test]
    fn owned_capture_retains_bound_environment_semantics() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/sdk-cpp-credential-env-gates");
        std::fs::create_dir_all(&base).unwrap();
        let directory = tempfile::tempdir_in(base).unwrap();
        let path = directory.path().join("api.json");
        std::fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Native env capture","version":"1"},"servers":[{"url":"https://example.test"}],"components":{"securitySchemes":{"token":{"type":"http","scheme":"bearer"}}},"paths":{"/who":{"get":{"operationId":"who","security":[{"token":[]}],"responses":{"204":{"description":"No content"}}}}}}).to_string()).unwrap();
        let workspace = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(directory.path())
                .build()
                .unwrap(),
        );
        let contract = Arc::new(
            Contract::from_workspace(&workspace, &suspect_source::Uri::from_path(&path).unwrap())
                .unwrap(),
        );
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let target = backend::TargetConfig {
            backend: backend::Backend::CppHttp,
            package_name: "env_capture".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        };
        // Start from the normal unconfigured public snapshot. Only this owned
        // capture seam is exercised with policy until Main opens readiness.
        let mut ordinary =
            crate::compatibility::snapshot(contract.clone(), &[], &[target]).unwrap();
        let mut native = ordinary.native.remove(0);
        assert!(native.credential_env.is_none());
        native.operations.clear();
        native.models.clear();
        native.generation.credential_env =
            Some(crate::credential_env::CredentialEnv::v1(BTreeMap::from([
                ("token".into(), "CPP_ENV_CAPTURE_TOKEN".into()),
            ])));
        capture(contract, &selected, &mut native).unwrap();
        let descriptor = native.credential_env.unwrap();
        assert_eq!(descriptor.bindings.len(), 1);
        assert_eq!(descriptor.bindings[0].name, "token");
        assert_eq!(descriptor.bindings[0].variable, "CPP_ENV_CAPTURE_TOKEN");
        assert_eq!(
            descriptor.bindings[0].kind,
            crate::credential_env::CredentialEnvKind::Bearer
        );
        let json = serde_json::to_value(descriptor).unwrap();
        assert!(!json.to_string().contains("file:"));
        assert!(!json.to_string().contains("pointer"));
        let factories = &native.operations[0].descriptor["constructor"]["environmentFactories"];
        assert_eq!(factories["snapshot"], "client-creation");
        assert_eq!(
            factories["from_env"]["returns"],
            "Result<Client, TransportError>"
        );
    }
}
