//! The deterministic application surface manifest.
//!
//! It records every public name, its requiredness, its exact value
//! representation and the contract source it came from, so a reviewer can
//! compare two generations without reading Go. It carries no build time and no
//! generator path: contract sources are recorded relative to the entry
//! document's directory whenever they live beside it.
use serde_json::{Value, json};
use suspect_ir::contract::SourceId;

use super::{CliPlan, SURFACE_FORMAT, planning};
use crate::OutFile;

pub(super) fn artifact(plan: &CliPlan) -> OutFile {
    let config = &plan.config;
    let manifest = json!({
        "format": SURFACE_FORMAT,
        "binary": config.binary_name,
        "module": config.module_path,
        "version": config.version,
        "outputFormat": config.output.format.surface(),
        "runtime": {
            "requestDeadlineMs": config.runtime.request_deadline_ms,
            "maxInputBytes": config.runtime.max_input_bytes,
        },
        "credentialEnv": config.credential_env,
        "credentialEnvFactory": plan.credential_factory,
        "cobraVersion": super::COBRA_VERSION,
        "commands": plan.commands.iter().map(|command| self::command(plan, command)).collect::<Vec<_>>(),
    });
    OutFile {
        path: "application-surface.json".into(),
        content: format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    }
}

fn command(plan: &CliPlan, command: &planning::BoundCommand) -> Value {
    let operation = &plan.sdk.operations()[command.operation];
    json!({
        "path": command.path,
        "name": command.path.join(" "),
        "summary": command.summary,
        "operationId": operation.operation_id,
        "httpMethod": operation.wire().method().as_str(),
        "httpPath": operation.wire().path(),
        "nativeMethod": operation.method_name,
        "source": location(plan, &operation.source),
        "confirmation": command.confirmation,
        "flags": command.flags.iter().map(|flag| json!({
            "flag": flag.flag,
            "parameter": flag.parameter,
            "in": flag.location,
            "required": flag.required,
            "representation": flag.representation.surface(),
            "model": flag.model,
            "schema": schema(plan, &flag.schema),
        })).collect::<Vec<_>>(),
        "body": command.body.as_ref().map(|body| json!({
            "flag": planning::BODY_FLAG,
            "required": body.required,
            "representation": "exact_json_document",
            "mediaType": body.media_type,
            "model": body.model,
            "schema": schema(plan, &body.schema),
        })),
        "confirmationFlag": match command.confirmation {
            super::ConfirmationPolicy::NotRequired => Value::Null,
            super::ConfirmationPolicy::Required { .. } => Value::from(planning::CONFIRM_FLAG),
        },
        "responses": command.responses.iter().map(|response| json!({
            "status": response.status,
            "representation": response.payload.surface(),
            "model": response.model,
            "success": response.success,
            "failure": response.failure,
        })).collect::<Vec<_>>(),
    })
}

fn location(plan: &CliPlan, source: &SourceId) -> Value {
    json!({"document": document(plan, source.document().as_str()), "pointer": source.pointer()})
}

fn schema(plan: &CliPlan, source: &suspect_ir::contract::SchemaId) -> Value {
    json!({"document": document(plan, source.document().as_str()), "pointer": source.pointer()})
}

/// Record a contract document relative to the entry document's directory, so
/// the manifest carries no generator path. Planning admitted every document
/// this manifest names through the same scope, refusing any that lies outside
/// the entry tree, so no absolute URI can reach this point.
fn document(plan: &CliPlan, uri: &str) -> String {
    plan.scope
        .relative(uri)
        .expect("planning admitted every manifest document into the entry tree")
        .to_owned()
}
