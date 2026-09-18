//! Typed Python codec surface over native descriptors, never emitted-source parsing.
use super::CodecConfig;
use crate::{
    OutFile,
    python_models::{ModelPlan, PyDecl, PyType},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use suspect_ir::contract::SchemaId;

fn root(indices: &BTreeMap<(String, String), usize>, id: &SchemaId) -> usize {
    indices[&(id.document().to_string(), id.pointer().into())]
}
fn ty(
    value: &PyType,
    names: &BTreeMap<SchemaId, String>,
    indices: &BTreeMap<(String, String), usize>,
) -> Value {
    match value {
        PyType::Primitive(name) => json!({"kind":"primitive","name":name}),
        PyType::JsonValue => json!({"kind":"json"}),
        PyType::Named(id) => json!({"kind":"named","name":names[id]}),
        PyType::Nullable(inner) => json!({"kind":"nullable","inner":ty(inner,names,indices)}),
        PyType::Optional(inner) => ty(inner, names, indices),
        PyType::List(inner) => json!({"kind":"list","inner":ty(inner,names,indices)}),
        PyType::Map(inner) => json!({"kind":"map","inner":ty(inner,names,indices)}),
        PyType::Literal(values) => json!({"kind":"literal","values":values}),
        PyType::Union(values) => {
            json!({"kind":"union","alternatives":values.iter().map(|(id,value)|json!({"root":root(indices,id),"type":ty(value,names,indices)})).collect::<Vec<_>>()})
        }
    }
}
pub(crate) fn package(
    models: &ModelPlan,
    indices: &BTreeMap<(String, String), usize>,
    config: &CodecConfig,
) -> Vec<OutFile> {
    let names: BTreeMap<_, _> = models
        .symbols()
        .iter()
        .map(|symbol| (symbol.source().clone(), symbol.name().to_owned()))
        .collect();
    let mut descriptors = BTreeMap::new();
    for (id, decl) in &models.declarations {
        let name = &names[id];
        let value = match decl {
            PyDecl::Alias(value) => json!({"kind":"alias","type":ty(value,&names,indices)}),
            PyDecl::Dataclass { fields, extras } => {
                json!({"kind":"object","fields":fields.iter().map(|field|json!({"name":field.name,"wire":field.wire,"required":field.required,"fixed":field.fixed.is_some(),"type":ty(&field.ty,&names,indices)})).collect::<Vec<_>>(),"extras":extras.as_ref().map(|value|ty(value,&names,indices))})
            }
        };
        let mut value = value.as_object().unwrap().clone();
        value.insert(
            "source".into(),
            json!({"document":id.document().as_str(),"pointer":id.pointer()}),
        );
        value.insert("root".into(), json!(root(indices, id)));
        descriptors.insert(name, Value::Object(value));
    }
    let metadata = json!({"models":descriptors,"maxDepth":config.max_conversion_depth,"maxSteps":config.max_conversion_steps,"json":{"max_input_bytes":config.json_limits.max_input_bytes,"max_output_bytes":config.json_limits.max_output_bytes,"max_depth":config.json_limits.max_depth,"max_work":config.json_limits.max_work}});
    let mut code = String::from(
        "\"\"\"Source-bound model codecs; numeric/presence/union semantics are validated.\"\"\"\nfrom __future__ import annotations\nfrom typing import TYPE_CHECKING\nif TYPE_CHECKING or __package__:\n    from . import models\n    from .codec_runtime import ModelCodec\nelse:\n    import models\n    from codec_runtime import ModelCodec\n\n",
    );
    for symbol in models.symbols() {
        code.push_str(&format!(
            "{}Codec: ModelCodec[models.{}] = ModelCodec({:?})\n",
            symbol.name(),
            symbol.name(),
            symbol.name()
        ));
    }
    vec![
        OutFile {
            path: "python/model_codecs.py".into(),
            content: code,
        },
        OutFile {
            path: "python/codec_runtime.py".into(),
            content: include_str!("runtime.py").into(),
        },
        OutFile {
            path: "python/codec-plan.json".into(),
            content: serde_json::to_string(&metadata).unwrap(),
        },
    ]
}
