use serde_json::Value;
use suspect_ir::contract::SourceId;

use super::planner::Planner;
use super::{DiagnosticKind, ExampleMetadata, ExamplePlan, Located, Severity};

pub(super) fn plan(p: &mut Planner<'_>, owner: &SourceId) -> ExampleMetadata {
    let Some(raw) = p.contract.source(owner).and_then(Value::as_object) else {
        return ExampleMetadata::default();
    };
    let inline = raw.get("example").map(|value| Located {
        source: p.location(&owner.child("example")),
        value: value.clone(),
    });
    let mut named = Vec::new();
    if raw.contains_key("example") && raw.contains_key("examples") {
        p.error(
            &owner.child("examples"),
            "http-examples-exclusive",
            "example and examples are mutually exclusive",
        );
    }
    if let Some(value) = raw.get("examples") {
        let source = owner.child("examples");
        if let Some(map) = value.as_object() {
            for name in map.keys() {
                if let Some(example) = example(p, &source.child(name), name) {
                    named.push(example);
                }
            }
        } else {
            p.error(
                &source,
                "http-examples-invalid",
                "examples must be a map of Example Objects/references",
            );
        }
    }
    ExampleMetadata { inline, named }
}

fn example(p: &mut Planner<'_>, source: &SourceId, name: &str) -> Option<ExamplePlan> {
    let diagnostics_start = p.diagnostics.len();
    let Some(origin) = p.resolve(source, false) else {
        // Missing example data does not alter an operation's wire contract.
        // Keep the located finding; the example stage also records unavailable
        // declared data before synthesizing a separately labelled replacement.
        // Malformed metadata and missing wire references remain blocking.
        for finding in &mut p.diagnostics[diagnostics_start..] {
            if finding.code == "http-reference-unresolved" {
                finding.severity = Severity::Warning;
                finding.kind = DiagnosticKind::Annotation;
            }
        }
        return None;
    };
    let at = &origin.terminal.source;
    let raw = p.object(at, "Example")?;
    p.known(
        at,
        raw,
        &[
            "summary",
            "description",
            "value",
            "dataValue",
            "serializedValue",
            "externalValue",
        ],
    );
    let summary = p.text(&origin, "summary", false);
    let description = p.text(&origin, "description", false);
    for field in ["dataValue", "serializedValue"] {
        if raw.contains_key(field) && !p.is_32(at) {
            p.unsupported(
                &at.child(field),
                "http-version-field",
                format!("Example.{field} requires OAS 3.2"),
            );
        }
    }
    let serialized_value = p.string(at, "serializedValue", false);
    let external_value = p.string(at, "externalValue", false);
    if raw.contains_key("value") {
        for field in ["dataValue", "serializedValue", "externalValue"] {
            if raw.contains_key(field) {
                p.error(
                    &at.child(field),
                    "http-example-exclusive",
                    "value cannot coexist with dataValue, serializedValue, or externalValue",
                );
            }
        }
    }
    if raw.contains_key("serializedValue") && raw.contains_key("externalValue") {
        p.error(&at.child("serializedValue"), "http-example-exclusive", "serializedValue and externalValue are mutually exclusive; dataValue may accompany either");
    }
    let value = raw.get("value").map(|value| Located {
        source: p.location(&at.child("value")),
        value: value.clone(),
    });
    let data_value = raw.get("dataValue").map(|value| Located {
        source: p.location(&at.child("dataValue")),
        value: value.clone(),
    });
    Some(ExamplePlan {
        source: origin,
        name: name.to_owned(),
        summary,
        description,
        value,
        data_value,
        serialized_value,
        external_value,
    })
}
