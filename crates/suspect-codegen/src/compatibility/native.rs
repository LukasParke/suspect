use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SourceId};

use crate::{
    OutFile,
    backend::{GenerationOptions, TargetConfig},
};

use super::{
    Change, Impact, Location, NativeReport, Summary, change, native_models, provenance,
    schema::Correspondence, swift_models, wire::OperationMatch,
};

const FORMAT: &str = "suspect-native-interface-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanStatus {
    Planned,
    EmptySelection,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanFinding {
    pub code: String,
    pub source: Option<Location>,
    pub message: String,
}

/// Runtime assets currently ship with the generator, not as an independently
/// versioned runtime release. The exact fingerprint scope is recorded below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProvenance {
    pub generator_version: String,
    pub runtime_version: Option<String>,
    pub profile: String,
    pub plan_and_runtime_sha256: String,
    pub fingerprinted_assets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeOperation {
    pub source: Location,
    pub operation_id: String,
    /// Actual collision-allocated names: method, module, input/constructor,
    /// result and errors.
    pub symbols: BTreeMap<String, String>,
    /// Native parameter names, presence, model bindings and response variants.
    /// Missing portions are accompanied by explicit snapshot findings.
    pub descriptor: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeModel {
    pub source: Location,
    pub name: String,
    /// Model/non-null model, directional view, or source-bound codec property.
    pub role: String,
    pub descriptor: Option<Value>,
}

/// A serializable interface record captured from the actual native plan.
/// Runtime public APIs outside this descriptor profile need their native gates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeSnapshot {
    pub format: String,
    pub target: TargetConfig,
    /// Explicit interpretation used to produce this native plan.
    #[serde(default)]
    pub generation: GenerationOptions,
    /// Source-bound runtime defaults, captured from the actual native plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_env: Option<crate::credential_env::CredentialEnvDescriptor>,
    /// Compiled `ua/v1` package attribution, captured from the actual native plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<crate::attribution::AttributionDescriptor>,
    /// Typed client-default policy, captured from the actual native plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaultsDescriptor>,
    pub status: PlanStatus,
    pub runtime: RuntimeProvenance,
    pub operations: Vec<NativeOperation>,
    pub models: Vec<NativeModel>,
    pub findings: Vec<PlanFinding>,
}

pub(super) fn capture(
    contract: Arc<Contract>,
    selected: &[SourceId],
    target: &TargetConfig,
    generation: &GenerationOptions,
) -> NativeSnapshot {
    let mut snapshot = NativeSnapshot {
        format: FORMAT.into(),
        target: target.clone(),
        generation: generation.clone(),
        credential_env: None,
        attribution: None,
        sdk_defaults: None,
        status: PlanStatus::Planned,
        runtime: provenance::capture(target.backend.name()),
        operations: Vec::new(),
        models: Vec::new(),
        findings: Vec::new(),
    };
    if let Some(error) = crate::backend::credential_env_admission_error(target, generation) {
        snapshot.status = PlanStatus::Unavailable;
        snapshot.findings.push(PlanFinding {
            code: error.code.into(),
            source: error
                .source
                .as_ref()
                .map(|source| Location::at(&contract, source)),
            message: error.message,
        });
        return snapshot;
    }
    if selected.is_empty() && generation.credential_env.is_none() {
        snapshot.status = PlanStatus::EmptySelection;
        return snapshot;
    }
    let result = match target.backend.name() {
        #[cfg(feature = "cpp-sdk")]
        "cpp-http" => super::cpp::capture(contract.clone(), selected, &mut snapshot),
        #[cfg(feature = "dart-sdk")]
        "dart-http" => super::dart::capture(contract.clone(), selected, &mut snapshot),
        "typescript-http" => typescript(contract.clone(), selected, &mut snapshot),
        "rust-http" => rust(contract.clone(), selected, &mut snapshot),
        "python-http" => python(contract.clone(), selected, &mut snapshot),
        "go-http" => go(contract.clone(), selected, &mut snapshot),
        #[cfg(feature = "php-sdk")]
        "php-http" => super::php::capture(contract.clone(), selected, &mut snapshot),
        // Registration is owned by the backend integrator. This adapter is ready
        // without making this module the owner of Backend's variant list.
        "swift-http" | "swift-sdk" => swift(contract.clone(), selected, &mut snapshot),
        #[cfg(feature = "csharp-sdk")]
        "csharp-http" => super::csharp::capture(contract.clone(), selected, &mut snapshot),
        #[cfg(feature = "ruby-sdk")]
        "ruby-http" => super::ruby::capture(contract.clone(), selected, &mut snapshot),
        #[cfg(feature = "kotlin-sdk")]
        "kotlin-http" => super::kotlin::capture(contract.clone(), selected, &mut snapshot),
        #[cfg(feature = "java-sdk")]
        "java-http" => super::java::capture(contract.clone(), selected, &mut snapshot),
        other => Err(vec![PlanFinding {
            code: "native-backend-unregistered".into(),
            source: None,
            message: format!("No compatibility adapter for {other}"),
        }]),
    };
    if let Err(findings) = result {
        snapshot.status = PlanStatus::Unavailable;
        snapshot.findings.extend(findings);
    }
    if snapshot
        .models
        .iter()
        .any(|model| model.descriptor.is_none())
        && snapshot.findings.is_empty()
    {
        gap(
            &mut snapshot,
            "native-model-descriptor-unavailable",
            "At least one allocated model has no typed native declaration descriptor; model source compatibility is unknown.",
        );
    }
    snapshot.operations.sort_by(|a, b| {
        (&a.source.document, &a.source.pointer, &a.operation_id).cmp(&(
            &b.source.document,
            &b.source.pointer,
            &b.operation_id,
        ))
    });
    snapshot.models.sort_by(|a, b| {
        (&a.source.document, &a.source.pointer, &a.role, &a.name).cmp(&(
            &b.source.document,
            &b.source.pointer,
            &b.role,
            &b.name,
        ))
    });
    snapshot
        .findings
        .sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
    snapshot
}

type ResultPlan = Result<(), Vec<PlanFinding>>;
fn errors(
    contract: &Contract,
    errors: Vec<crate::http_contract::HttpDiagnostic>,
) -> Vec<PlanFinding> {
    errors
        .into_iter()
        .map(|error| PlanFinding {
            code: error.code.into(),
            source: Some(Location::at(contract, &error.source)),
            message: error.message,
        })
        .collect()
}
fn package_error(error: impl std::fmt::Display) -> Vec<PlanFinding> {
    vec![PlanFinding {
        code: "native-package-invalid".into(),
        source: None,
        message: error.to_string(),
    }]
}
/// Compile the same `ua/v1` attribution that canonical generation records for
/// one target, so compatibility capture describes the emitted package identity.
fn compiled_attribution(
    contract: &Contract,
    target: &TargetConfig,
) -> crate::attribution::AttributionDescriptor {
    crate::attribution::AttributionDescriptor::plan(
        env!("CARGO_PKG_VERSION"),
        &target.package_name,
        &target.package_version,
        contract.openapi_version(),
        target.backend.language_tag(),
    )
}
/// Attribution semantics without the generator and package versions: those
/// bump constantly and are recorded separately as runtime provenance and
/// package metadata. Grammar version, package identity, language and the
/// source document version participate in interface equality.
fn native_attribution_semantic(
    descriptor: &crate::attribution::AttributionDescriptor,
) -> crate::attribution::AttributionDescriptor {
    crate::attribution::AttributionDescriptor {
        template_version: descriptor.template_version,
        suspect_version: String::new(),
        sdk_name: descriptor.sdk_name.clone(),
        sdk_version: String::new(),
        spec_version: descriptor.spec_version.clone(),
        language: descriptor.language.clone(),
    }
}
fn gap(snapshot: &mut NativeSnapshot, code: &str, message: &str) {
    snapshot.findings.push(PlanFinding {
        code: code.into(),
        source: None,
        message: message.into(),
    });
}
fn symbols<const N: usize>(values: [(&str, &str); N]) -> BTreeMap<String, String> {
    values
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
}

fn typescript(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> ResultPlan {
    use crate::typescript;
    let attribution = compiled_attribution(&contract, &snapshot.target);
    let config = crate::backend::typescript_options(&snapshot.generation, Some(&attribution));
    snapshot.runtime.profile = config.capabilities().adapter().into();
    let plan = typescript::http::plan_http(contract.clone(), selected, config)
        .map_err(|e| errors(&contract, e))?;
    snapshot.credential_env = plan
        .credential_env()
        .map(crate::credential_env::CredentialEnvPlan::semantic_descriptor);
    snapshot.attribution = plan.attribution().cloned();
    snapshot.sdk_defaults = plan
        .sdk_defaults()
        .map(|defaults| defaults.semantic_descriptor());
    typescript::package::emit_http(
        &plan,
        &typescript::package::PackageConfig {
            name: snapshot.target.package_name.clone(),
            version: snapshot.target.package_version.clone(),
        },
    )
    .map_err(package_error)?;
    for op in plan.operations() {
        snapshot.operations.push(NativeOperation {
            source: Location::at(&contract, &op.source),
            operation_id: op.operation_id.clone(),
            symbols: symbols([
                ("method", &op.function_name),
                ("input", &op.input_type),
                ("success", &op.success_type),
                ("api-error", &op.error_type),
                ("error-guard", &op.error_guard),
            ]),
            descriptor: op.interface().clone(),
        });
    }
    snapshot.models = native_models::typescript(&contract, plan.codecs().models());
    for model in &mut snapshot.models {
        model
            .descriptor
            .as_mut()
            .expect("TypeScript has typed model descriptors")["codec"] =
            plan.codecs().interfaces()[&model.name].clone();
    }
    Ok(())
}

fn rust(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> ResultPlan {
    snapshot.runtime.profile = crate::rust_http::native_capabilities().adapter().into();
    let attribution = compiled_attribution(&contract, &snapshot.target);
    let plan = crate::rust_http::plan_http_v3(
        contract.clone(),
        selected,
        crate::backend::rust_options(&snapshot.generation, Some(&attribution)),
    )
    .map_err(|e| errors(&contract, e))?;
    snapshot.runtime.profile = plan.protocol().capabilities().adapter().into();
    snapshot.credential_env = plan
        .credential_env()
        .map(|policy| policy.semantic_descriptor());
    snapshot.attribution = plan.attribution().cloned();
    snapshot.sdk_defaults = plan
        .sdk_defaults()
        .map(|defaults| defaults.semantic_descriptor());
    let files = crate::rust_http::emit_http(
        &plan,
        &crate::rust_http::PackageConfig {
            name: snapshot.target.package_name.clone(),
            version: snapshot.target.package_version.clone(),
        },
    )
    .map_err(package_error)?;
    // Validate the packaged manifest, but capture native bindings directly from
    // the public typed plan rather than reconstructing them from artifacts.
    manifest(&files, "rust/http-manifest.json")?;
    for op in plan.operations() {
        snapshot.operations.push(NativeOperation {
            source: Location::at(&contract, &op.source),
            operation_id: op.operation_id.clone(),
            symbols: symbols([
                ("method", &op.function_name),
                ("module", &op.module_name),
                ("input", &op.input_type),
                ("success", &op.success_type),
                ("error", &op.error_type),
                ("api-error", &op.api_error_type),
            ]),
            descriptor: op.interface(),
        });
    }
    snapshot.models = native_models::rust(&contract, plan.codecs().models());
    Ok(())
}

fn python(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> ResultPlan {
    snapshot.runtime.profile = crate::python_http::capabilities().adapter().into();
    let attribution = compiled_attribution(&contract, &snapshot.target);
    let plan = crate::python_http::plan_http(
        contract.clone(),
        selected,
        crate::backend::python_options(&snapshot.generation, Some(&attribution)),
    )
    .map_err(|e| errors(&contract, e))?;
    snapshot.credential_env = plan
        .credential_env()
        .map(crate::credential_env::CredentialEnvPlan::semantic_descriptor);
    snapshot.attribution = plan.attribution().cloned();
    snapshot.sdk_defaults = plan
        .sdk_defaults()
        .map(|defaults| defaults.semantic_descriptor());
    let import = snapshot
        .target
        .import_name
        .clone()
        .unwrap_or_else(|| snapshot.target.package_name.replace('-', "_"));
    let files = crate::python_http::emit_http(
        &plan,
        &crate::python_http::PackageConfig {
            name: snapshot.target.package_name.clone(),
            version: snapshot.target.package_version.clone(),
            import_name: import.clone(),
        },
    )
    .map_err(package_error)?;
    let metadata = manifest(&files, &format!("python/src/{import}/http-manifest.json"))?;
    // Keep source locations in the snapshot's Location fields, not in interface
    // equality. Located literal values (notably Link parameters) are opaque data
    // and must never have their own "source"/"schema" properties stripped.
    fn surface(value: &Value) -> Value {
        match value {
            Value::Object(object)
                if object.contains_key("source")
                    && object.contains_key("value")
                    && object.len() == 2 =>
            {
                object["value"].clone()
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "source"
                                | "provenance"
                                | "span"
                                | "schema"
                                | "description"
                                | "summary"
                                | "examples"
                                | "annotations"
                                | "path_item"
                        )
                    })
                    .map(|(key, value)| (key.clone(), surface(value)))
                    .collect(),
            ),
            Value::Array(values) => Value::Array(values.iter().map(surface).collect()),
            value => value.clone(),
        }
    }
    let module = format!("{import}.operations");
    for (index, op) in plan.operations().iter().enumerate() {
        let record = &metadata["operations"][index];
        snapshot.operations.push(NativeOperation {
            source: Location::at(&contract, &op.source), operation_id: op.operation_id.clone(),
            symbols: symbols([("method", &op.snake_name), ("async-method", &op.snake_name), ("success", &op.success_type), ("async-success", &op.async_success_type), ("api-error", &op.error_type), ("async-api-error", &op.async_error_type), ("module", &module)]),
            descriptor: json!({
                "parameters":surface(&record["parameters"]),"body":surface(&record["body"]),"responses":surface(&record["responses"]),
                "operationsModule":metadata["operationsModule"],"resultModule":record["resultModule"],
                "publicExports":metadata["publicExports"],"groups":surface(&metadata["groups"]),
                "protocol":surface(&metadata["protocol"]["operations"][index]),"limits":metadata["limits"],
                "validation":{"version":plan.codecs().validation_program().version,"profile":plan.codecs().validation_program().profile},
                "credentialContext":{"type":format!("{import}.CredentialRequest"),"effectiveServerUrl":{"member":"effective_server_url","type":"str | None","default":null}},
            }),
        });
    }
    snapshot.models = native_models::python(&contract, plan.codecs().models());
    Ok(())
}

fn go(contract: Arc<Contract>, selected: &[SourceId], snapshot: &mut NativeSnapshot) -> ResultPlan {
    // Native surface records exclude wire provenance and source prose. Those
    // have their own Location/wire records and cannot change a Go signature.
    fn part(part: &crate::go_http::PlannedPart) -> Value {
        json!({"member":part.field_name,"setter":part.setter_name,"type":part.native_type,
            "valueType":part.data_type,"required":part.wire.required()})
    }
    fn media(media: &crate::go_http::PlannedMedia) -> Value {
        json!({"type":media.native_type,"choiceType":media.choice_type,"constructor":media.constructor,
            "requiresContentType":media.requires_content_type(),
            "aggregate":media.aggregate.as_ref().map(|aggregate|json!({"type":aggregate.type_name,
                "constructor":aggregate.constructor,"positional":aggregate.positional,
                "fields":aggregate.parts.iter().map(part).collect::<Vec<_>>(),
                "constructorParameters":aggregate.parts.iter().filter(|part|part.wire.required())
                    .map(|part|json!({"member":part.field_name,"type":part.native_type})).collect::<Vec<_>>(),
                "additional":aggregate.additional.as_ref().map(part)}))})
    }
    fn status(value: crate::http_protocol::ResponseStatus) -> Value {
        match value {
            crate::http_protocol::ResponseStatus::Exact(status) => json!(status),
            crate::http_protocol::ResponseStatus::Range(class) => json!(format!("{class}XX")),
            crate::http_protocol::ResponseStatus::Default => json!("default"),
        }
    }
    let attribution = compiled_attribution(&contract, &snapshot.target);
    let plan = crate::go_http::plan_http(
        contract.clone(),
        selected,
        crate::backend::go_options(&snapshot.generation, Some(&attribution)),
    )
    .map_err(|e| errors(&contract, e))?;
    snapshot.credential_env = plan
        .credential_env()
        .map(crate::credential_env::CredentialEnvPlan::semantic_descriptor);
    snapshot.attribution = plan.attribution().cloned();
    snapshot.sdk_defaults = plan
        .sdk_defaults()
        .map(|defaults| defaults.semantic_descriptor());
    crate::go_http::emit_http(
        &plan,
        &crate::go_http::PackageConfig {
            module_path: snapshot.target.package_name.clone(),
            package_name: "sdk".into(),
            version: snapshot.target.package_version.clone(),
        },
    )
    .map_err(|e| package_error(e.join("; ")))?;
    let response_union = |name: &str, error: bool| {
        let mut descriptor = go_response_union(name, error);
        descriptor["embedsCloser"] = json!(true);
        descriptor
    };
    let credentials=plan.credentials().iter().map(|(scheme,constructor)| {
        let requirement=plan.protocol().operations().iter().flat_map(|operation|operation.security().alternatives())
            .flat_map(|alternative|alternative.requirements()).find(|requirement|requirement.name()==scheme)
            .expect("allocated Go credential");
        let parameters=match requirement.credential() {
            crate::http_protocol::CredentialHook::Basic=>vec!["string","string"],
            crate::http_protocol::CredentialHook::Bearer{..} | crate::http_protocol::CredentialHook::ApiKey{..}=>vec!["string"],
            _=>vec!["CredentialHook"],
        };
        json!({"scheme":scheme,"constructor":constructor,"parameters":parameters,"result":"Credentials"})
    }).collect::<Vec<_>>();
    for op in plan.operations() {
        let constructor_parameters = op.parameters().iter()
            .filter(|parameter| parameter.wire().required())
            .map(|parameter| json!({"member":parameter.field_name,"model":plan.symbols()[parameter.schema()]}))
            .chain(op.body().filter(|body|body.wire().required()).map(|body|json!({"member":body.field_name,"model":body.native_type})))
            .collect::<Vec<_>>();
        let mut captured = NativeOperation {
            source: Location::at(&contract, &op.source),
            operation_id: op.operation_id.clone(),
            symbols: symbols([
                ("method", &op.method_name),
                ("input", &op.input_type),
                ("input-constructor", &op.input_constructor),
                ("success", &op.success_type),
                ("api-error", &op.error_variant),
            ]),
            descriptor: json!({
                "constructor":{"parameters":constructor_parameters,"result":op.input_type},
                "parameters":op.parameters().iter().map(|p|json!({"member":p.field_name,"setter":p.setter_name,"wire":p.wire().name(),"location":format!("{:?}",p.wire().location()),"required":p.wire().required(),"model":plan.symbols()[p.schema()]})).collect::<Vec<_>>(),
                "body":op.body().map(|b|json!({"member":b.field_name,"setter":b.setter_name,"required":b.wire().required(),"model":b.native_type,"media":b.media().iter().map(media).collect::<Vec<_>>()})),
                "dataMethod":op.data_method,"optionalInput":op.optional_input,
                "validation":{"version":plan.codecs().validation_program().version,"profile":plan.codecs().validation_program().profile,"errorType":"ValidationError","findingType":"ValidationFinding"},
                "responseUnions":{
                    "success":response_union(&op.success_type, false),
                    "apiError":response_union(&op.error_variant, true),
                },
                "responses":op.responses().iter().map(|r|{
                    let success = r.can_succeed();
                    let memberships=[r.can_succeed().then(||json!({"interface":op.success_type,"receiver":"value"})),r.can_fail().then(||json!({"interface":op.error_variant,"receiver":"pointer"}))].into_iter().flatten().collect::<Vec<_>>();
                    json!({"type":r.type_name,"status":status(r.status()),"mediaType":r.media().map(|m|m.wire().media_type().declared()),"model":r.native_type,"headersType":r.headers_type,
                          "headers":r.headers.iter().map(|header|json!({"member":header.field_name,"required":header.wire.required(),
                              "model":plan.symbols()[header.wire.codec().schema().id()]})).collect::<Vec<_>>(),
                          "linksType":"[]HTTPLink","media":r.media().map(media),
                         "membership":{"interface":if success {&op.success_type} else {&op.error_variant},"receiver":if success {"value"} else {"pointer"}},
                         "memberships":memberships})
                }).collect::<Vec<_>>(),
                "credentials":credentials,
            }),
        };
        if let Some(factory) = plan.credential_env_factory() {
            // The package-level factory participates in each source-paired
            // client's construction surface. Its name is collision-allocated;
            // the signature is the fixed rule in the fingerprinted Go emitter.
            captured
                .symbols
                .insert("env-factory".into(), factory.into());
            captured.descriptor["constructor"]["client"] = json!({"environmentFactory":{
                "kind":"function", "name":factory,
                "parameters":[{"name":"options","type":"ClientOptions","variadic":true}],
                "returns":["*Client","error"], "acceptedOptions":{"minimum":0,"maximum":1}
            }});
        }
        snapshot.operations.push(captured);
    }
    snapshot.models = native_models::go(&contract, plan.codecs().models());
    Ok(())
}

fn go_response_union(name: &str, api_error: bool) -> Value {
    // Unlike constructors and wrappers, this marker is a fixed emission rule
    // over the *allocated interface name*. It is not independently allocated
    // from operationId. The hashed Go emitter defines this closed method set.
    json!({"type":name,"closed":true,"privateMarker":format!("is{name}"),"embedsError":api_error})
}

fn swift(
    contract: Arc<Contract>,
    selected: &[SourceId],
    snapshot: &mut NativeSnapshot,
) -> ResultPlan {
    fn form_field(p: &crate::http_protocol::PartPlan) -> Value {
        use crate::http_protocol::PartRepresentation;
        let representation = match p.representation() {
            PartRepresentation::Json { outer_encoding, .. } => {
                json!({"kind":"json","outerEncoding":outer_encoding})
            }
            PartRepresentation::Text {
                scalar,
                outer_encoding,
                ..
            } => json!({"kind":"text","scalar":scalar,"outerEncoding":outer_encoding}),
            PartRepresentation::Style { serialization, .. } => {
                json!({"kind":"style","serialization":serialization})
            }
            PartRepresentation::Binary { bytes } => {
                json!({"kind":"bytes","maxBytes":bytes.max_bytes()})
            }
        };
        json!({"name":p.name(),"required":p.required(),"multiplicity":p.multiplicity(),"contentTypes":p.content_types(),"representation":representation})
    }
    fn part(p: &crate::swift_sdk::PlannedPart) -> Value {
        json!({"member":p.field_name,"wire":p.wire.name(),"required":p.wire.required(),
            "type":p.type_name,"itemType":p.item_type,"valueType":p.value_type,
            "multiplicity":p.wire.multiplicity(),"minItems":p.wire.min_items().map(|v|v.value()),
            "maxItems":p.wire.max_items().map(|v|v.value()),"headerType":p.header_type,
            "headers":p.headers.iter().map(|h|json!({"member":h.field_name,"type":h.type_name,"required":h.wire.required(),"serialization":h.wire.serialization()})).collect::<Vec<_>>()})
    }
    fn media(m: &crate::swift_sdk::PlannedMedia) -> Value {
        let constructor = m.parts.as_ref().map(|p| {
            let mut fields = p.fields.iter().collect::<Vec<_>>(); fields.sort_by_key(|f| !f.wire.required());
            fields.into_iter().map(|f| json!({"name":f.field_name,"type":f.type_name,"hasDefault":!f.wire.required()})).collect::<Vec<_>>()
        });
        json!({"case":m.case_name,"type":m.type_name,"mediaType":m.wire.media_type(),
            "parts":m.parts.as_ref().map(|p|json!({"type":p.type_name,"multipart":p.multipart,
                "constructorParameters":constructor,
                "fields":p.fields.iter().map(part).collect::<Vec<_>>(),"additional":p.additional.as_deref().map(part),
                "required":p.rules.required().iter().map(|v|v.value()).collect::<Vec<_>>(),
                "minProperties":p.rules.min_properties().map(|v|v.value()),"maxProperties":p.rules.max_properties().map(|v|v.value())})),
            "positional":m.positional.as_ref().map(|p|json!({"type":p.type_name,"formData":p.form_data,
                "prefix":p.prefix.iter().map(part).collect::<Vec<_>>(),"items":p.items.as_deref().map(part),
                "minItems":p.min_items.as_ref().map(|v|v.value()),"maxItems":p.max_items.as_ref().map(|v|v.value()),
                "constructorParameters":p.prefix.iter().map(|f|json!({"name":f.field_name,"type":f.item_type,"hasDefault":!f.wire.required()})).chain(p.items.as_ref().map(|f|json!({"name":"items","type":format!("[{}]",f.item_type),"hasDefault":true}))).collect::<Vec<_>>()}))})
    }
    fn hook(h: &crate::http_protocol::CredentialHook) -> Value {
        use crate::http_protocol::CredentialHook;
        match h {
            CredentialHook::Bearer { bearer_format } => {
                json!({"kind":"bearer","bearerFormat":bearer_format.as_ref().map(|v|v.value())})
            }
            CredentialHook::Basic => json!({"kind":"basic"}),
            CredentialHook::ApiKey { location, name } => {
                json!({"kind":"api-key","location":location,"name":name.value()})
            }
            CredentialHook::OpenIdConnect { discovery_url } => {
                json!({"kind":"openid-connect","discoveryUrl":discovery_url.value()})
            }
            CredentialHook::OAuth2 {
                flows,
                metadata_url,
            } => json!({"kind":"oauth2","metadataUrl":metadata_url.as_ref().map(|v|v.value()),
                "flows":flows.iter().map(|flow|json!({"kind":flow.kind(),"authorizationUrl":flow.authorization_url().map(|v|v.value()),
                    "tokenUrl":flow.token_url().map(|v|v.value()),"refreshUrl":flow.refresh_url().map(|v|v.value()),
                    "deviceAuthorizationUrl":flow.device_authorization_url().map(|v|v.value()),"scopes":flow.scopes().keys().collect::<Vec<_>>()})).collect::<Vec<_>>()}),
        }
    }
    let plan = crate::swift_sdk::plan_sdk(
        contract.clone(),
        selected,
        crate::backend::swift_options(&snapshot.generation, None),
    )
    .map_err(|e| errors(&contract, e))?;
    // V1 uses Swift's fixed 8192-UTF-8-byte bearer/API-key value ceiling
    // (CREDENTIAL_ENV_MAX_BYTES). The typed descriptor records source-bound
    // variable names/kinds; the fixed native policy is fingerprinted runtime code.
    snapshot.credential_env = plan
        .credential_env()
        .map(|policy| policy.semantic_descriptor());
    crate::swift_sdk::emit_sdk(
        &plan,
        &crate::swift_sdk::PackageConfig {
            name: snapshot.target.package_name.clone(),
            version: snapshot.target.package_version.clone(),
            module_name: snapshot
                .target
                .import_name
                .clone()
                .unwrap_or_else(|| snapshot.target.package_name.clone()),
        },
    )
    .map_err(|e| package_error(e.join("; ")))?;
    for op in plan.operations() {
        let media_schema =
            |media: &crate::swift_sdk::PlannedMedia| match media.wire.representation() {
                crate::http_protocol::Representation::Json { codec }
                | crate::http_protocol::Representation::Text { codec, .. } => {
                    codec.as_ref().map(|c| c.schema().id().clone())
                }
                crate::http_protocol::Representation::Stream { stream } => {
                    stream.item_codec().map(|codec| codec.schema().id().clone())
                }
                _ => None,
            };
        let constructor_parameters = op.parameters.iter().map(|parameter| {
            let wire = parameter.wire();
            json!({"name":parameter.field_name,"type":swift_models::input_type(plan.models(), wire.codec().schema().id(), wire.required()),
                "hasDefault":!wire.required(),"initialization":{"kind":if wire.required() {"argument"} else {"missing"}}})
        }).chain(op.body.iter().map(|body|json!({"name":"body","type":body.media.first().filter(|_|!body.is_enum).and_then(&media_schema).map(|id|swift_models::input_type(plan.models(), &id, body.wire.required())).unwrap_or_else(||json!({"kind":"native","name":body.type_name,"optional":!body.wire.required()})),
            "hasDefault":!body.wire.required(),"initialization":{"kind":if body.wire.required() {"argument"} else {"missing"}}}))).collect::<Vec<_>>();
        snapshot.operations.push(NativeOperation {
            source: Location::at(&contract, &op.source), operation_id: op.operation_id.clone(),
            symbols: symbols([("method", &op.method_name), ("input", &op.input_type), ("success", &op.success_type), ("api-error", &op.error_type), ("protocol-metadata", &op.metadata_name)]),
            descriptor: json!({
                "constructor":{"name":"init","parameters":constructor_parameters,"result":op.input_type},
                "defaultInput":op.default_input(),"httpMethod":op.method,
                "parameters":op.parameters.iter().map(|p|json!({"member":p.field_name,"wire":p.wire().name(),"location":format!("{:?}",p.wire().location()),"required":p.wire().required(),
                    "model":plan.symbols()[p.wire().codec().schema().id()],"type":swift_models::input_type(plan.models(), p.wire().codec().schema().id(), p.wire().required()),"codec":swift_models::codec_name(plan.models(), p.wire().codec().schema().id()),"serialization":p.wire().serialization(),
                    "queryForm":p.query_form.as_ref().map(|form|json!({"encoder":form.encoder_name,"fields":form.wire.fields().iter().map(form_field).collect::<Vec<_>>(),
                        "additional":match form.wire.additional(){crate::http_protocol::AdditionalParts::Forbidden=>None,crate::http_protocol::AdditionalParts::Allowed(p)=>Some(form_field(p))}}))})).collect::<Vec<_>>(),
                "body":op.body.as_ref().map(|b|{let id=b.media.first().filter(|_|!b.is_enum).and_then(&media_schema);json!({"member":"body","required":b.wire.required(),"model":b.type_name,
                    "type":id.as_ref().map(|id|swift_models::input_type(plan.models(),id,b.wire.required())).unwrap_or_else(||json!({"kind":"native","name":b.type_name})),"codec":id.as_ref().map(|id|swift_models::codec_name(plan.models(),id)),
                    "media":b.media.iter().map(media).collect::<Vec<_>>()})}),
                "responses":op.responses.iter().map(|r|{let id=r.media.first().filter(|_|!r.is_enum).and_then(&media_schema);json!({"status":match r.wire.status(){crate::http_protocol::ResponseStatus::Exact(s)=>json!(s),_=>json!(r.wire.status_key())},"mediaType":r.media.first().map(|m|m.wire.media_type().declared()),"model":r.type_name,
                    "enum":if r.may_succeed() {&op.success_type} else {&op.error_type},"case":r.case_name,"responseType":r.response_type,"headerType":r.header_type,
                    "successMembership":r.may_succeed(),"errorMembership":r.may_fail(),"mayHaveNoBody":r.may_be_empty,
                    "type":{"kind":"api-response","of":id.as_ref().map(|id|swift_models::type_at(plan.models(),id)).unwrap_or_else(||json!({"kind":"native","name":r.type_name}))},"codec":id.as_ref().map(|id|swift_models::codec_name(plan.models(),id)),
                    "media":r.media.iter().map(media).collect::<Vec<_>>(),
                    "headers":r.headers.iter().map(|h|json!({"member":h.field_name,"required":h.wire.required(),"type":h.type_name,"serialization":h.wire.serialization()})).collect::<Vec<_>>()})}).collect::<Vec<_>>(),
                "credential":op.protocol().security().alternatives().iter().map(|a|a.requirements().iter().map(|r|json!({"property":plan.credential_property(r.scheme().use_site().source()),"hook":hook(r.credential()),
                    "permissions":match r.permissions(){crate::http_protocol::Permissions::Scopes(v)=>json!({"kind":"scopes","names":v.iter().map(|v|v.value()).collect::<Vec<_>>()}),crate::http_protocol::Permissions::Roles(v)=>json!({"kind":"roles","names":v.iter().map(|v|v.value()).collect::<Vec<_>>()})}})).collect::<Vec<_>>()).collect::<Vec<_>>(),
            }),
        });
    }
    snapshot.models = swift_models::capture(&contract, plan.models());
    Ok(())
}

fn manifest(files: &[OutFile], path: &str) -> Result<Value, Vec<PlanFinding>> {
    let file = files
        .iter()
        .find(|file| file.path == path)
        .ok_or_else(|| package_error(format!("missing plan manifest {path}")))?;
    serde_json::from_str(&file.content).map_err(package_error)
}

pub(super) fn compare(
    old: &[NativeSnapshot],
    new: &[NativeSnapshot],
    operations: &[OperationMatch],
    schemas: &Correspondence,
) -> Vec<NativeReport> {
    let backends: BTreeSet<_> = old.iter().chain(new).map(|s| s.target.backend).collect();
    backends.into_iter().map(|backend| {
        let before = old.iter().find(|s|s.target.backend == backend);
        let after = new.iter().find(|s|s.target.backend == backend);
        let mut changes = Vec::new();
        for (snapshot, side) in [(before, "before"), (after, "after")] {
            if let Some(snapshot) = snapshot {
                if snapshot.status == PlanStatus::Unavailable && snapshot.findings.is_empty() {
                    changes.push(change("native-plan-unavailable", Impact::Unknown, side, "The native plan is unavailable without a diagnostic; compatibility cannot be established."));
                }
                for finding in &snapshot.findings {
                    let mut c = change("native-plan-unknown", Impact::Unknown, format!("{side}: {}", finding.code), &finding.message);
                    if side == "before" { c.source_before = finding.source.clone(); } else { c.source_after = finding.source.clone(); }
                    c.reasoning.push(finding.code.clone());
                    c.migration = "Resolve planner diagnostics or supply the missing native-plan descriptor before treating this target as compatible.".into();
                    changes.push(c);
                }
            }
        }
        match (before, after) {
            (Some(a), Some(b)) => {
                package_changes(a, b, &mut changes);
                if a.generation.compatibility_profiles != b.generation.compatibility_profiles {
                    let mut c = change("native-interpretation-profile-changed", Impact::Unknown, backend.name(), "The explicit source interpretation changed; identical native signatures do not prove equivalent wire behavior.");
                    c.before = Some(json!(a.generation)); c.after = Some(json!(b.generation));
                    c.migration = "Review profile-specific byte behavior and rerun the affected native wire consumers.".into();
                    changes.push(c);
                }
                if a.generation.credential_env != b.generation.credential_env || a.credential_env != b.credential_env {
                    let mut c = change("native-credential-env-changed", Impact::PotentiallyBreaking, backend.name(), "The configured runtime credential defaults changed; omitted-credential client setup may now select different credentials or require different variables.");
                    c.before = Some(json!({"configuration":a.generation.credential_env,"binding":a.credential_env}));
                    c.after = Some(json!({"configuration":b.generation.credential_env,"binding":b.credential_env}));
                    c.migration = "Supply the mapped variables before creating clients, or keep explicit credentials to retain caller-selected setup. No credential values are recorded in this report.".into();
                    changes.push(c);
                }
                if a.attribution.as_ref().map(native_attribution_semantic)
                    != b.attribution.as_ref().map(native_attribution_semantic)
                {
                    let mut c = change(
                        "native-attribution-changed",
                        Impact::Unknown,
                        backend.name(),
                        format!(
                            "The generated package attribution changed for {}; the recorded User-Agent identity or source document version no longer matches the compared target.",
                            backend.name()
                        ),
                    );
                    c.before = a
                        .attribution
                        .as_ref()
                        .map(|descriptor| json!(native_attribution_semantic(descriptor)));
                    c.after = b
                        .attribution
                        .as_ref()
                        .map(|descriptor| json!(native_attribution_semantic(descriptor)));
                    c.migration = "Review the attribution identity change and rerun User-Agent-sensitive traffic classification before accepting this transition; generator and package versions are recorded separately as provenance and package metadata.".into();
                    changes.push(c);
                }
                if a.generation.sdk_defaults != b.generation.sdk_defaults
                    || a.sdk_defaults != b.sdk_defaults
                {
                    let mut c = change(
                        "native-sdk-defaults-changed",
                        Impact::PotentiallyBreaking,
                        backend.name(),
                        format!(
                            "The configured client behavior defaults changed for {}; automatic pagination policy, the proposed page size or the automatic API-key environment prefix may now differ for existing consumers.",
                            backend.name()
                        ),
                    );
                    c.before = Some(json!({"configuration":a.generation.sdk_defaults,"binding":a.sdk_defaults}));
                    c.after = Some(json!({"configuration":b.generation.sdk_defaults,"binding":b.sdk_defaults}));
                    c.migration = "Regenerate with the intended defaults and review pagination accessors, proposed page size and automatic credential environment setup in existing consumers.".into();
                    changes.push(c);
                }
                if a.runtime != b.runtime {
                    let mut c = change("native-runtime-provenance-changed", Impact::Unknown, backend.name(), "The recorded generator/runtime provenance changed; matching descriptors alone cannot establish runtime or toolchain compatibility.");
                    c.before = Some(json!(a.runtime)); c.after = Some(json!(b.runtime));
                    c.migration = "Run native consumer and wire conformance gates for the new generator/runtime.".into(); changes.push(c);
                }
                if a.format != FORMAT || b.format != FORMAT {
                    changes.push(change("native-descriptor-format-unknown", Impact::Unknown, backend.name(), "An interface record has an unsupported descriptor format."));
                } else if a.status != PlanStatus::Unavailable && b.status != PlanStatus::Unavailable {
                    operation_changes(a, b, operations, &mut changes);
                    model_changes(a, b, schemas, &mut changes);
                }
            },
            (Some(_), None) => changes.push(change("native-target-removed", Impact::Breaking, backend.name(), "This generated package target is no longer selected.")),
            (None, Some(_)) => changes.push(change("native-target-added", Impact::Compatible, backend.name(), "A generated package target was added.")),
            _ => {},
        }
        let summary = Summary::from_changes(changes.iter());
        NativeReport { backend, before: before.cloned(), after: after.cloned(), changes, summary }
    }).collect()
}

fn package_changes(old: &NativeSnapshot, new: &NativeSnapshot, changes: &mut Vec<Change>) {
    if old.target.package_name != new.target.package_name {
        let mut c = change(
            "native-package-name-changed",
            Impact::Breaking,
            old.target.backend.name(),
            "The dependency/package identity changed.",
        );
        c.before = Some(json!(old.target.package_name));
        c.after = Some(json!(new.target.package_name));
        c.migration = "Update dependency declarations and any package-qualified imports.".into();
        changes.push(c);
    }
    let import = |target: &TargetConfig| match target.backend.name() {
        "python-http" => Some(
            target
                .import_name
                .clone()
                .unwrap_or_else(|| target.package_name.replace('-', "_")),
        ),
        "swift-http" | "swift-sdk" => Some(
            target
                .import_name
                .clone()
                .unwrap_or_else(|| target.package_name.clone()),
        ),
        _ => None,
    };
    if import(&old.target) != import(&new.target) {
        let mut c = change(
            "native-import-name-changed",
            Impact::Breaking,
            old.target.backend.name(),
            "The importable module name changed.",
        );
        c.before = Some(json!(import(&old.target)));
        c.after = Some(json!(import(&new.target)));
        c.migration = "Update source imports and qualified model/client references.".into();
        changes.push(c);
    }
    if old.target.package_version != new.target.package_version {
        let mut c = change(
            "native-package-version-changed",
            Impact::Compatible,
            old.target.backend.name(),
            "Package version metadata changed; a version increment alone is not a source break.",
        );
        c.before = Some(json!(old.target.package_version));
        c.after = Some(json!(new.target.package_version));
        c.migration =
            "Update the pinned package version after reviewing the other compatibility findings."
                .into();
        changes.push(c);
    }
}

fn operation_changes(
    old: &NativeSnapshot,
    new: &NativeSnapshot,
    matches: &[OperationMatch],
    changes: &mut Vec<Change>,
) {
    for pair in matches {
        let before = pair.before.as_ref().and_then(|source| {
            old.operations
                .iter()
                .find(|op| op.source.identifies(source))
        });
        let after = pair.after.as_ref().and_then(|source| {
            new.operations
                .iter()
                .find(|op| op.source.identifies(source))
        });
        match (before, after) {
            (Some(a), Some(b)) => {
                for role in a
                    .symbols
                    .keys()
                    .chain(b.symbols.keys())
                    .collect::<BTreeSet<_>>()
                {
                    let (l, r) = (a.symbols.get(role), b.symbols.get(role));
                    if l == r {
                        continue;
                    }
                    let mut c = native_operation_change(
                        "native-operation-symbol-changed",
                        if l.is_some() {
                            Impact::Breaking
                        } else {
                            Impact::Compatible
                        },
                        role,
                        Some(a),
                        Some(b),
                        "An allocated public operation symbol changed.",
                    );
                    c.before = l.map(|n| json!(n));
                    c.after = r.map(|n| json!(n));
                    c.migration = format!(
                        "Update {role} references using the recorded before/after native names."
                    );
                    changes.push(c);
                }
                for (key, code) in [
                    ("constructor", "native-input-constructor-changed"),
                    ("parameters", "native-input-changed"),
                    ("body", "native-input-changed"),
                    ("responseUnions", "native-responses-changed"),
                    ("responses", "native-responses-changed"),
                    ("credential", "native-credentials-changed"),
                ] {
                    if a.descriptor.get(key).map(api_descriptor)
                        == b.descriptor.get(key).map(api_descriptor)
                    {
                        continue;
                    }
                    let mut c = native_operation_change(
                        code,
                        Impact::PotentiallyBreaking,
                        key,
                        Some(a),
                        Some(b),
                        "Native input/output/credential bindings changed independently of wire compatibility.",
                    );
                    c.before = a.descriptor.get(key).cloned();
                    c.after = b.descriptor.get(key).cloned();
                    c.migration = "Adjust native arguments, model construction, result/error handling or credential members; compile existing consumers.".into();
                    changes.push(c);
                }
            }
            (Some(a), None) => {
                let mut c = native_operation_change(
                    "native-operation-removed",
                    Impact::Breaking,
                    &a.operation_id,
                    Some(a),
                    None,
                    "The selected native operation and its exported related types were removed.",
                );
                c.before = Some(json!(a.symbols));
                c.migration =
                    "Replace calls and references to the removed operation symbols.".into();
                changes.push(c);
            }
            (None, Some(b)) => {
                let mut c = native_operation_change(
                    "native-operation-added",
                    Impact::Compatible,
                    &b.operation_id,
                    None,
                    Some(b),
                    "A native operation was added.",
                );
                c.after = Some(json!(b.symbols));
                changes.push(c);
            }
            _ => {}
        }
    }
}

fn native_operation_change(
    code: &str,
    impact: Impact,
    subject: &str,
    old: Option<&NativeOperation>,
    new: Option<&NativeOperation>,
    message: &str,
) -> Change {
    let mut c = change(code, impact, subject, message);
    c.operation_id_before = old.map(|op| op.operation_id.clone());
    c.operation_id_after = new.map(|op| op.operation_id.clone());
    c.source_before = old.map(|op| op.source.clone());
    c.source_after = new.map(|op| op.source.clone());
    c
}

fn model_changes(
    old: &NativeSnapshot,
    new: &NativeSnapshot,
    schemas: &Correspondence,
    changes: &mut Vec<Change>,
) {
    let mut used = BTreeSet::new();
    for model in &old.models {
        let mapped = schemas
            .iter()
            .find(|(source, _)| model.source.identifies(source))
            .map(|(_, targets)| targets);
        let candidates: Vec<_> = new
            .models
            .iter()
            .enumerate()
            .filter(|(i, m)| {
                !used.contains(i)
                    && m.role == model.role
                    && mapped
                        .is_some_and(|targets| targets.iter().any(|id| m.source.identifies(id)))
            })
            .collect();
        let found = if candidates.len() == 1 {
            Some(candidates[0])
        } else {
            new.models
                .iter()
                .enumerate()
                .find(|(i, m)| {
                    !used.contains(i)
                        && m.role == model.role
                        && m.source.same_address(&model.source)
                })
                .or_else(|| {
                    new.models.iter().enumerate().find(|(i, m)| {
                        !used.contains(i) && m.role == model.role && m.name == model.name
                    })
                })
        };
        if let Some((index, next)) = found {
            used.insert(index);
            if model.name != next.name {
                let old_name_still_exists = new
                    .models
                    .iter()
                    .any(|m| m.name == model.name && m.role == model.role);
                let mut c = change(
                    "native-model-renamed",
                    if old_name_still_exists {
                        Impact::PotentiallyBreaking
                    } else {
                        Impact::Breaking
                    },
                    &model.name,
                    "A source-corresponding native model has a different allocated name.",
                );
                c.source_before = Some(model.source.clone());
                c.source_after = Some(next.source.clone());
                c.before = Some(json!(model.name));
                c.after = Some(json!(next.name));
                c.migration = "Update model imports, construction and type references to the new native symbol.".into();
                changes.push(c);
            }
            if model.descriptor.as_ref().map(api_descriptor)
                != next.descriptor.as_ref().map(api_descriptor)
            {
                let mut c = change(
                    "native-model-shape-changed",
                    Impact::PotentiallyBreaking,
                    &model.name,
                    "The native declaration changed: fields, presence/null wrappers, literal/union variants, constructors or representation types may affect source consumers.",
                );
                c.source_before = Some(model.source.clone());
                c.source_after = Some(next.source.clone());
                c.before = model.descriptor.clone();
                c.after = next.descriptor.clone();
                c.migration = "Review the structured before/after declaration and compile existing native consumers, including constructors and exhaustive matches.".into();
                changes.push(c);
            }
        } else {
            let mut c = change(
                "native-model-removed",
                Impact::Breaking,
                &model.name,
                "An exported native model/representation was removed.",
            );
            c.source_before = Some(model.source.clone());
            c.before =
                Some(json!({"name":model.name,"role":model.role,"descriptor":model.descriptor}));
            c.migration =
                "Replace references to this model with the new source-bound representation.".into();
            changes.push(c);
        }
    }
    for (index, model) in new
        .models
        .iter()
        .enumerate()
        .filter(|(index, _)| !used.contains(index))
    {
        let _ = index;
        let mut c = change(
            "native-model-added",
            Impact::Compatible,
            &model.name,
            "A native model/representation was added.",
        );
        c.source_after = Some(model.source.clone());
        c.after = Some(json!({"name":model.name,"role":model.role,"descriptor":model.descriptor}));
        changes.push(c);
    }
}

/// Wire bindings remain in the record as provenance, but are not native
/// signature changes when the actual member, type and presence are unchanged.
/// Literal/fixed instance values are opaque; identically named instance keys
/// must not be mistaken for descriptor metadata.
fn api_descriptor(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(api_descriptor).collect()),
        Value::Object(object) => {
            if matches!(
                object.get("kind").and_then(Value::as_str),
                Some("literal" | "literals")
            ) {
                return value.clone();
            }
            let parameter = object.contains_key("member")
                && object.contains_key("model")
                && object.contains_key("required");
            let field = object.contains_key("name") && object.contains_key("type");
            Value::Object(
                object
                    .iter()
                    .filter_map(|(key, value)| {
                        if ((parameter || field) && key == "wire")
                            || (parameter && key == "location")
                        {
                            None
                        } else {
                            Some((
                                key.clone(),
                                if matches!(key.as_str(), "fixed" | "value" | "values" | "token") {
                                    value.clone()
                                } else {
                                    api_descriptor(value)
                                },
                            ))
                        }
                    })
                    .collect(),
            )
        }
        _ => value.clone(),
    }
}
