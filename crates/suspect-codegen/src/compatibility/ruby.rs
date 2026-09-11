//! Native Ruby interface records from retained shapes and allocated symbols.

use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SourceId};

use super::{Location, NativeModel, NativeOperation, NativeSnapshot, PlanFinding};
use crate::{
    backend, http_protocol as wire,
    ruby_sdk::{self, ExtraFields, ModelPlan, ModelShape, ScalarKind},
};

fn type_name(models: &ModelPlan, index: usize, namespace: &str) -> Value {
    let symbol = models.symbol(index).expect("planned Ruby type");
    json!({"kind":"named","name":format!("{namespace}::Types::{}", symbol.type_name)})
}

// Location evidence lives on the native record. Native interface comparison
// excludes retrieval spans/prose while preserving literal Link instance values.
fn semantic(value: Value) -> Value {
    match value {
        Value::Array(v) => Value::Array(v.into_iter().map(semantic).collect()),
        Value::Object(mut map) => {
            if map.len() == 2 && map.contains_key("source") && map.contains_key("value") {
                return map.remove("value").unwrap();
            }
            for key in [
                "source",
                "use_site",
                "terminal",
                "references",
                "span",
                "description",
                "summary",
                "deprecated",
                "default_from",
                "operation",
            ] {
                map.remove(key);
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, semantic(v))).collect())
        }
        value => value,
    }
}

// Capture the attachment surface from typed descriptors. A requirement's
// `scheme` is Provenance, not its credential type or API URL base. Its resource
// names must not become signature equality; physical locations live on native
// records and the retained Contract. Located auth values are read explicitly so
// real scope names such as "source" or "description" remain ordinary data.
fn credential_surface(security: &wire::SecurityPlan) -> Value {
    match security {
        wire::SecurityPlan::Undeclared { .. } => json!({"kind":"undeclared"}),
        wire::SecurityPlan::NoAuth { .. } => json!({"kind":"no-auth"}),
        wire::SecurityPlan::Alternatives { alternatives, .. } => json!({
            "kind":"alternatives",
            "alternatives":alternatives.iter().map(|alternative| json!({
                "requirements":alternative.requirements().iter().map(|requirement| json!({
                    "name":requirement.name(),
                    "permissions":match requirement.permissions() {
                        wire::Permissions::Scopes(names) => json!({"kind":"scopes","names":names.iter().map(wire::Located::value).collect::<Vec<_>>()}),
                        wire::Permissions::Roles(names) => json!({"kind":"roles","names":names.iter().map(wire::Located::value).collect::<Vec<_>>()}),
                    },
                    "credential":credential_hook(requirement.credential()),
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        }),
    }
}

fn credential_hook(hook: &wire::CredentialHook) -> Value {
    match hook {
        wire::CredentialHook::Bearer { bearer_format } => json!({
            "kind":"bearer","bearer_format":bearer_format.as_ref().map(wire::Located::value)
        }),
        wire::CredentialHook::Basic => json!({"kind":"basic"}),
        wire::CredentialHook::ApiKey { location, name } => json!({
            "kind":"api-key","location":location,"name":name.value()
        }),
        wire::CredentialHook::OAuth2 {
            flows,
            metadata_url,
        } => json!({
            "kind":"o-auth2","url_base":hook.url_base(),
            "metadata_url":metadata_url.as_ref().map(wire::Located::value),
            "flows":flows.iter().map(|flow| json!({
                "kind":flow.kind(),"url_base":flow.url_base(),
                "authorization_url":flow.authorization_url().map(wire::Located::value),
                "token_url":flow.token_url().map(wire::Located::value),
                "refresh_url":flow.refresh_url().map(wire::Located::value),
                "device_authorization_url":flow.device_authorization_url().map(wire::Located::value),
                // Names are data, including "name", "type" and "wire". Typed
                // entries cannot be mistaken for native field descriptors by
                // the shared interface comparison's field normalization.
                "scopes":flow.scopes().iter().map(|(name, description)| json!({"name":name,"value":description.value()})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()
        }),
        wire::CredentialHook::OpenIdConnect { discovery_url } => json!({
            "kind":"open-id-connect","url_base":hook.url_base(),"discovery_url":discovery_url.value()
        }),
    }
}

fn value_type(models: &ModelPlan, value: &ruby_sdk::NativeType, namespace: &str) -> Value {
    use ruby_sdk::NativeType::*;
    match value {
        Codec(i) => type_name(models, *i, namespace),
        Json => json!({"kind":"primitive","name":"JsonValue"}),
        Bytes => json!({"kind":"bytes","name":format!("{namespace}::Bytes")}),
        NoContent => json!({"kind":"no-content"}),
        Scalar(s) => json!({"kind":"text-scalar","scalar":s}),
        Record(n) => json!({"kind":"named","name":format!("{namespace}::Models::{n}")}),
        Array(t) => json!({"kind":"array","item":value_type(models,t,namespace)}),
        Part(t) => json!({"kind":"native-or-part","value":value_type(models,t,namespace)}),
        Stream(t) => json!({"kind":"closeable-enumerator","item":value_type(models,t,namespace)}),
        Union(v) => {
            json!({"kind":"union","members":v.iter().map(|t|value_type(models,t,namespace)).collect::<Vec<_>>()})
        }
    }
}

fn shape(
    models: &ModelPlan,
    program: &suspect_schema::OwnedProgram,
    value: &ModelShape,
    namespace: &str,
) -> Value {
    match value {
        ModelShape::Json => json!({"kind":"primitive","name":"JsonValue"}),
        ModelShape::RefinedJson => json!({"kind":"refined-json","codecValidationRequired":true}),
        ModelShape::Dynamic {
            initial_target,
            initial_resource,
            anchor,
            candidates,
        } => {
            json!({"kind":"resource-scoped-json","initialTarget":program.nodes[*initial_target].source,"initialResource":program.resource_context.as_ref().expect("v3 context").resources[*initial_resource].source,"anchor":anchor,"candidates":candidates.iter().map(|i|&program.nodes[*i].source).collect::<Vec<_>>(),"selection":"runtime-entered-resource-context"})
        }
        ModelShape::Literal(values) => json!({"kind":"literals","values":values}),
        ModelShape::Never => json!({"kind":"primitive","name":"bottom"}),
        ModelShape::Scalar(kinds) => {
            json!({"kind":"scalar-union","members":kinds.iter().map(|kind| match kind {
            ScalarKind::Null => "nil", ScalarKind::Boolean => "bool", ScalarKind::String => "String",
            ScalarKind::Integer => "JsonNumber(integer) | Integer", ScalarKind::Number => "JsonNumber | Integer",
        }).collect::<Vec<_>>()})
        }
        ModelShape::Alias(index) => {
            json!({"kind":"alias","type":type_name(models, *index, namespace)})
        }
        ModelShape::Array {
            items,
            prefix,
            nullable,
        } => json!({"kind":"array","nullable":nullable,
            "items":items.map(|index|type_name(models,index,namespace)),
            "prefix":prefix.iter().map(|&index|type_name(models,index,namespace)).collect::<Vec<_>>()}),
        ModelShape::Union {
            branches,
            exclusive,
        } => json!({"kind":"union","exclusive":exclusive,
            "variants":branches.iter().map(|&index|type_name(models,index,namespace)).collect::<Vec<_>>()}),
        ModelShape::Object {
            fields,
            extras,
            nullable,
        } => {
            let members = fields.iter().map(|field| json!({
                "name":field.name,"wire":field.wire_name,"type":type_name(models,field.schema_index,namespace),
                "required":field.required,"literal":field.literal,
                "initialization":if let Some(value)=&field.literal {json!({"kind":"literal","value":value})}
                    else if field.required {json!({"kind":"argument"})} else {json!({"kind":"unset"})},
            })).collect::<Vec<_>>();
            json!({"kind":"object","nullable":nullable,"fields":members,
                "extraType":match extras { ExtraFields::Closed=>Value::Null,ExtraFields::Json=>json!({"kind":"primitive","name":"JsonValue"}),ExtraFields::Typed(index)=>type_name(models,*index,namespace),ExtraFields::Scoped{patterns,additional,unevaluated}=>json!({"kind":"scoped-json-extras","patterns":patterns.iter().map(|p|json!({"pattern":p.pattern,"type":type_name(models,p.schema_index,namespace)})).collect::<Vec<_>>(),"additional":additional.map(|i|type_name(models,i,namespace)),"unevaluated":unevaluated.map(|i|type_name(models,i,namespace))})},
                "constructor":{"name":"new","arguments":"keyword-only","parameters":fields.iter().map(|field|json!({
                    "name":field.name,"type":type_name(models,field.schema_index,namespace),
                    "required":field.required && field.literal.is_none(),
                    "default":if let Some(value)=&field.literal {json!({"literal":value})}else if !field.required {json!("UNSET")}else {Value::Null},
                })).collect::<Vec<_>>()}})
        }
    }
}

pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> Result<(), Vec<PlanFinding>> {
    snapshot.runtime.profile = "ruby-http-protocol-v1".into();
    let plan = ruby_sdk::plan_sdk(
        contract.clone(),
        selected,
        crate::backend::ruby_options(&snapshot.generation),
    )
    .map_err(|errors| {
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
        .map(crate::credential_env::CredentialEnvPlan::semantic_descriptor);
    let package = backend::ruby_package(&snapshot.target);
    ruby_sdk::emit_sdk(&plan, &package).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| PlanFinding {
                code: error.code.into(),
                source: None,
                message: error.message,
            })
            .collect::<Vec<_>>()
    })?;
    let namespace = &package.namespace;
    let models = plan.models();
    for operation in plan.operations() {
        let symbols = BTreeMap::from([
            ("client".into(), format!("{namespace}::Client")),
            ("method".into(), operation.method_name.clone()),
            (
                "api-error".into(),
                format!("{namespace}::{}", operation.error_class),
            ),
        ]);
        snapshot.operations.push(NativeOperation {
            source:Location::at(&contract,&operation.source),operation_id:operation.operation_id.clone(),symbols,
            descriptor:json!({"arguments":"keyword-only",
                "parameters":operation.parameters.iter().map(|parameter|json!({"member":parameter.keyword,"wire":parameter.wire_name,
                    "location":format!("{:?}",parameter.location()),"required":parameter.required,
                    "type":value_type(models,&parameter.value_type,namespace),"serialization":parameter.serialization()})).collect::<Vec<_>>(),
                "body":operation.body.as_ref().map(|body|json!({"member":"body","required":body.required,"media":body.media.iter().map(|m|json!({"media":m.wire.media_type(),"type":value_type(models,&m.value_type,namespace)})).collect::<Vec<_>>()})),
                "responses":operation.responses.iter().map(|response|json!({"class":format!("{namespace}::{}",response.class_name),
                    "errorClass":response.error_class,"status":response.status,"bodyForbidden":response.body_forbidden,"media":response.media.iter().map(|m|json!({"media":m.wire.media_type(),"type":value_type(models,&m.value_type,namespace)})).collect::<Vec<_>>(),"headers":response.headers.iter().map(|h|json!({"name":h.name,"wire":h.wire_name,"required":h.required,"type":type_name(models,h.schema_index,namespace)})).collect::<Vec<_>>(),"links":semantic(serde_json::to_value(response.wire.links()).unwrap())})).collect::<Vec<_>>(),
                "credential":credential_surface(operation.wire.security()),"servers":semantic(serde_json::to_value(operation.wire.servers()).unwrap()),"wireMethod":operation.method,
            }),
        });
    }
    for record in plan.records() {
        snapshot.models.push(NativeModel{source:Location::at(&contract,&record.source),name:format!("{namespace}::Models::{}",record.name),role:"wire-record".into(),descriptor:Some(json!({"kind":"structural-wire-record","fields":record.fields.iter().map(|f|json!({"name":f.name,"wire":f.wire_name,"required":f.required,"type":value_type(models,&f.value_type,namespace)})).collect::<Vec<_>>(),"additional":record.additional.as_ref().map(|t|value_type(models,t,namespace))}))});
    }
    for symbol in models.symbols() {
        let declaration = shape(models, plan.program(), &symbol.shape, namespace);
        let source = Location::at(&contract, &symbol.source);
        snapshot.models.push(NativeModel {
            source:source.clone(),name:format!("{namespace}::Types::{}",symbol.type_name),role:"signature".into(),
            descriptor:Some(json!({"kind":"alias","uninhabited":symbol.signature_uninhabited,"type":declaration})),
        });
        if matches!(symbol.shape, ModelShape::Object { .. }) {
            snapshot.models.push(NativeModel {
                source: source.clone(),
                name: format!("{namespace}::Models::{}", symbol.name),
                role: "model".into(),
                descriptor: Some(declaration),
            });
        }
        snapshot.models.push(NativeModel {
            source,
            name: format!("{namespace}::Codecs::{}", symbol.name),
            role: "codec".into(),
            descriptor: Some(
                json!({"kind":"model-codec","type":type_name(models,symbol.schema_index,namespace),
                "validationVersion":plan.program().version,"validationProfile":plan.program().profile,
                "methods":["decode","decode_json","encode","encode_json"]}),
            ),
        });
    }
    Ok(())
}
