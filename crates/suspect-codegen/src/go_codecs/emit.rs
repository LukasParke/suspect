use super::CodecConfig;
use crate::{
    OutFile,
    go_models::{GoDecl, GoType, Key, ModelPlan},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn index(
    indices: &BTreeMap<(String, String), usize>,
    id: &suspect_ir::contract::SchemaId,
) -> usize {
    indices[&(id.document().to_string(), id.pointer().into())]
}
fn ty(value: &GoType, names: &BTreeMap<Key, String>) -> Value {
    match value {
        GoType::Primitive(name) => json!({"kind":"primitive","name":name}),
        GoType::Named(key) => json!({"kind":"named","name":names[key]}),
        GoType::Nullable(inner) => json!({"kind":"nullable","inner":ty(inner,names)}),
        GoType::Optional(inner) => json!({"kind":"optional","inner":ty(inner,names)}),
        GoType::Presence(inner) => json!({"kind":"presence","inner":ty(inner,names)}),
        GoType::Slice(inner) => json!({"kind":"slice","inner":ty(inner,names)}),
        GoType::Map(inner) => json!({"kind":"map","inner":ty(inner,names)}),
        GoType::Pointer(inner) => json!({"kind":"pointer","inner":ty(inner,names)}),
    }
}
pub(crate) fn render(
    models: &ModelPlan,
    indices: &BTreeMap<(String, String), usize>,
    config: &CodecConfig,
    program: &suspect_schema::OwnedProgram,
) -> Vec<OutFile> {
    let descriptors = models.descriptors();
    let mut records = BTreeMap::new();
    let mut types = BTreeMap::new();
    for (key, decl) in &descriptors.declarations {
        let name = &descriptors.names[key];
        types.insert(name.clone(), name.clone());
        let mut metadata=match decl{
            GoDecl::Alias(value)=>json!({"kind":"alias","type":ty(value,&descriptors.names)}),
            GoDecl::Literals{underlying,..}=>json!({"kind":"literal","type":{"kind":"primitive","name":underlying}}),
            GoDecl::Struct{fields,extras}=>json!({"kind":"object","fields":fields.iter().map(|field|json!({"name":field.name,"wire":field.wire,"required":field.init.is_none(),"type":ty(&field.ty,&descriptors.names)})).collect::<Vec<_>>(),"extras":extras.as_ref().map(|extra|ty(extra,&descriptors.names))}),
            GoDecl::Union(variants)=>json!({"kind":"union","variants":variants.iter().map(|variant|{let variant_name=format!("{name}{}",variant.name);types.insert(variant_name.clone(),variant_name.clone());json!({"name":variant_name,"root":index(indices,&variant.source),"type":ty(&variant.ty,&descriptors.names)})}).collect::<Vec<_>>()}),
        }.as_object().unwrap().clone();
        metadata.insert("root".into(), json!(index(indices, &key.0)));
        metadata.insert(
            "nonNull".into(),
            json!(key.1 == crate::rust_models::RepresentationRole::NonNullValue),
        );
        metadata.insert(
            "source".into(),
            json!({"document":key.0.document().as_str(),"pointer":key.0.pointer()}),
        );
        records.insert(name.clone(), Value::Object(metadata));
    }
    let mut code = String::from(
        "// Source-bound native codec registry.\npackage sdk\nimport \"reflect\"\n\nvar codecNativeTypes = map[string]reflect.Type{\n",
    );
    for (name, ty) in types {
        code.push_str(&format!("{name:?}:reflect.TypeOf((*{ty})(nil)).Elem(),\n"));
    }
    code.push_str("}\n\n// Codecs exposes one typed validated codec per native model.\nvar Codecs = struct {\n");
    for symbol in models.symbols() {
        code.push_str(&format!("{} Codec[{}]\n", symbol.name(), symbol.name()));
    }
    code.push_str("}{\n");
    for symbol in models.symbols() {
        code.push_str(&format!(
            "{}:Codec[{}]{{name:{:?}}},\n",
            symbol.name(),
            symbol.name(),
            symbol.name()
        ));
    }
    code.push_str("}\n");
    let metadata = json!({"models":records,"validation":{"version":program.version,"profile":program.profile},"maxDepth":config.max_conversion_depth,"maxSteps":config.max_conversion_steps,"json":{"MaxBytes":config.json_limits.max_input_bytes,"MaxOutputBytes":config.json_limits.max_output_bytes,"MaxDepth":config.json_limits.max_depth,"MaxNodes":config.json_limits.max_work,"MaxNumberLength":config.schema.max_number_bytes.max(4096)}});
    vec![
        OutFile {
            path: "go/codecs.go".into(),
            content: code,
        },
        OutFile {
            path: "go/codec_runtime.go".into(),
            content: include_str!("runtime.go").into(),
        },
        OutFile {
            path: "go/codec-plan.json".into(),
            content: serde_json::to_string(&metadata).unwrap(),
        },
    ]
}
