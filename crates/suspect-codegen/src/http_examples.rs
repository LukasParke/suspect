//! Shared example provenance artifacts. Native lowering remains with each
//! language's allocated model/operation symbols.

use crate::examples::{
    ExampleEntry, ExampleOrigin, ExamplePartPosition, ExamplePlan, ExampleRole, OperationExamples,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use suspect_ir::contract::SourceId;

fn source(id: &SourceId) -> Value {
    json!({"document":id.document().to_string(), "pointer":id.pointer()})
}

/// Native member identity plus the canonical wire-slot source it consumes.
pub(crate) struct InputSlot<'a> {
    pub name: &'a str,
    pub required: bool,
    pub container: SourceId,
}
pub(crate) struct BoundInput<'a> {
    pub name: &'a str,
    pub required: bool,
    pub example_index: usize,
}
pub(crate) struct ExampleBindings<'a> {
    pub operation: &'a OperationExamples,
    slots: BTreeMap<&'a SourceId, usize>,
}
impl ExampleBindings<'_> {
    /// Prefer the first validated value for each real slot. Required missing
    /// examples prevent an input snippet; optional missing values stay absent.
    pub fn bind<'a>(
        &self,
        slots: impl IntoIterator<Item = InputSlot<'a>>,
    ) -> Option<Vec<BoundInput<'a>>> {
        let mut bound = Vec::new();
        for slot in slots {
            if let Some(&example_index) = self.slots.get(&slot.container) {
                bound.push(BoundInput {
                    name: slot.name,
                    required: slot.required,
                    example_index,
                });
            } else if slot.required {
                return None;
            }
        }
        Some(bound)
    }
}
pub(crate) fn bindings(plan: &ExamplePlan) -> BTreeMap<&SourceId, ExampleBindings<'_>> {
    plan.operations()
        .iter()
        .map(|operation| {
            let mut slots = BTreeMap::new();
            for (index, entry) in operation.entries.iter().enumerate() {
                if matches!(
                    entry.role,
                    ExampleRole::RequestBody | ExampleRole::Parameter { .. }
                ) {
                    slots.entry(&entry.container).or_insert(index);
                }
            }
            (&operation.source, ExampleBindings { operation, slots })
        })
        .collect()
}

fn role_label(role: &ExampleRole) -> String {
    match role {
        ExampleRole::RequestBody => "Request body".into(),
        ExampleRole::Response { status } => format!("HTTP {status} response"),
        ExampleRole::ResponsePattern { status } => format!("HTTP {status} response pattern"),
        ExampleRole::ResponseHeader { status, wire_name } => {
            format!("HTTP {status} header: {wire_name}")
        }
        ExampleRole::RequestPart { name, .. } => {
            format!("Request part: {}", name.as_deref().unwrap_or("positional"))
        }
        ExampleRole::RequestPartHeader { part, wire_name } => format!(
            "Request part {} header: {wire_name}",
            part.as_deref().unwrap_or("positional")
        ),
        ExampleRole::ResponsePart { status, name, .. } => format!(
            "HTTP {status} part: {}",
            name.as_deref().unwrap_or("positional")
        ),
        ExampleRole::ResponsePartHeader {
            status,
            part,
            wire_name,
        } => format!(
            "HTTP {status} part {} header: {wire_name}",
            part.as_deref().unwrap_or("positional")
        ),
        ExampleRole::RequestItem => "Request stream item".into(),
        ExampleRole::ResponseItem { status } => format!("HTTP {status} stream item"),
        ExampleRole::Parameter {
            wire_name,
            location,
        } => format!("{location:?} parameter: {wire_name}"),
    }
}

pub(crate) fn origin(origin: &ExampleOrigin) -> &'static str {
    match origin {
        ExampleOrigin::Declared => "declared",
        ExampleOrigin::Synthesized => "synthesized",
    }
}

pub(crate) fn role(role: &ExampleRole) -> Value {
    match role {
        ExampleRole::RequestBody => json!({"kind":"request-body"}),
        ExampleRole::Response { status } => json!({"kind":"response", "status":status}),
        ExampleRole::ResponsePattern { status } => {
            json!({"kind":"response-pattern","status":status})
        }
        ExampleRole::ResponseHeader { status, wire_name } => {
            json!({"kind":"response-header","status":status,"name":wire_name})
        }
        ExampleRole::RequestPart { name, repeated } => {
            json!({"kind":"request-part","name":name,"repeated":repeated})
        }
        ExampleRole::RequestPartHeader { part, wire_name } => {
            json!({"kind":"request-part-header","part":part,"name":wire_name})
        }
        ExampleRole::ResponsePart {
            status,
            name,
            repeated,
        } => json!({"kind":"response-part","status":status,"name":name,"repeated":repeated}),
        ExampleRole::ResponsePartHeader {
            status,
            part,
            wire_name,
        } => json!({"kind":"response-part-header","status":status,"part":part,"name":wire_name}),
        ExampleRole::RequestItem => json!({"kind":"request-item"}),
        ExampleRole::ResponseItem { status } => json!({"kind":"response-item","status":status}),
        ExampleRole::Parameter {
            wire_name,
            location,
        } => json!({"kind":"parameter", "name":wire_name, "in":match location {
            suspect_ir::contract::ParameterLocation::Path => "path",
            suspect_ir::contract::ParameterLocation::Query => "query",
            suspect_ir::contract::ParameterLocation::Querystring => "querystring",
            suspect_ir::contract::ParameterLocation::Header => "header",
            suspect_ir::contract::ParameterLocation::Cookie => "cookie",
        }}),
    }
}

pub(crate) fn manifest(plan: &ExamplePlan) -> String {
    let entry = |entry: &ExampleEntry| {
        let mut value = json!({
            "role":role(&entry.role), "mediaType":entry.media_type, "schema":source(&entry.schema),
            "container":source(&entry.container),
            "origin":origin(&entry.origin), "declaredSource":entry.declared_source.as_ref().map(source),
            "name":entry.name, "summary":entry.summary, "value":entry.value,
        });
        if let Some(position) = entry.part_position {
            value["partPosition"] = match position {
                ExamplePartPosition::Prefix(index) => json!({"kind":"prefix","index":index}),
                ExamplePartPosition::Items => json!({"kind":"items"}),
            };
        }
        value
    };
    let value = json!({
        "format":plan.format(),
        "operations":plan.operations().iter().map(|operation| {
            let mut value = json!({
                "operationId":operation.operation_id, "source":source(&operation.source),
                "entries":operation.entries.iter().map(&entry).collect::<Vec<_>>(),
            });
            if !operation.validated_aggregates.is_empty() {
                value["validatedAggregates"] = json!(operation.validated_aggregates.iter().map(&entry).collect::<Vec<_>>());
            }
            value
        }).collect::<Vec<_>>(),
        "diagnostics":plan.diagnostics().iter().map(|finding| json!({
            "code":finding.code,"message":finding.message,"source":source(&finding.source),
            "range":{"start":finding.at.start,"end":finding.at.end},
        })).collect::<Vec<_>>(),
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("example manifest is JSON")
    )
}

pub(crate) fn markdown(plan: &ExamplePlan, run: &str) -> String {
    let mut text = format!(
        "# Validated contract examples\n\nEach value below passed its source-schema validator before native lowering. Declared examples retain their original location; synthesized values are labeled explicitly. Located findings remain in `examples.json`.\n\nThe packaged native sample validates each value again using the emitted codecs and constructs the available typed operation inputs. Run it with:\n\n```sh\n{run}\n```\n\n"
    );
    for operation in plan.operations() {
        text.push_str(&format!("## {}\n\n", escape(&operation.operation_id)));
        for entry in operation
            .validated_aggregates
            .iter()
            .chain(&operation.entries)
        {
            text.push_str(&format!(
                "### {} example — {}\n\nSchema: `{}#{}`.\n\n",
                origin(&entry.origin),
                escape(&role_label(&entry.role)),
                escape(entry.schema.document().as_str()),
                escape(entry.schema.pointer())
            ));
            if let Some(declared) = &entry.declared_source {
                text.push_str(&format!(
                    "Declared at `{}#{}`.\n\n",
                    escape(declared.document().as_str()),
                    escape(declared.pointer())
                ));
            }
            text.push_str(&format!(
                "```json\n{}\n```\n\n",
                serde_json::to_string_pretty(&entry.value).expect("example value")
            ));
        }
    }
    if !plan.diagnostics().is_empty() {
        text.push_str("## Example findings\n\n");
        for diagnostic in plan.diagnostics() {
            text.push_str(&format!(
                "- **{}** at `{}#{}`: {}\n",
                escape(diagnostic.code),
                escape(diagnostic.source.document().as_str()),
                escape(diagnostic.source.pointer()),
                escape(&diagnostic.message)
            ));
        }
    }
    text
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('`', "&#96;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('*', "&#42;")
        .replace('_', "&#95;")
        .replace(['\n', '\r'], " ")
}
