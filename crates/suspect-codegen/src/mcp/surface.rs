//! The deterministic application surface manifest.
//!
//! It records every public tool and input property, its requiredness, its exact
//! value representation and the contract source it came from, so a reviewer can
//! compare two generations without reading TypeScript. It carries no build time
//! and no generator path: contract sources are recorded relative to the entry
//! document's directory whenever they live beside it.
use serde_json::{Value, json};
use suspect_ir::contract::SourceId;

use super::{
    MCP_CLIENT_VERSION, MCP_SERVER_VERSION, SURFACE_FORMAT, ServerPlan, TYPESCRIPT_VERSION,
    planning,
};
use crate::OutFile;

/// How every schema-declared numeric value is represented at the tool boundary.
pub(super) const NUMBER_REPRESENTATION: &str = "exact_json_number_token";

pub(super) fn artifact(plan: &ServerPlan) -> OutFile {
    let config = &plan.config;
    let manifest = json!({
        "format": SURFACE_FORMAT,
        "transport": "stdio",
        "numberRepresentation": NUMBER_REPRESENTATION,
        "server": {
            "name": config.server_name,
            "version": config.version,
            "description": plan.mapping.description,
        },
        "package": {
            "name": config.package_name,
            "version": config.version,
            "bin": config.bin_name,
            "entry": super::package::ENTRY,
        },
        "sdk": {
            "mcpServer": MCP_SERVER_VERSION,
            "mcpClient": MCP_CLIENT_VERSION,
            "typescript": TYPESCRIPT_VERSION,
            "node": config.node_version,
            "nodeMinimumMajor": config.node_minimum_major,
        },
        "runtime": {
            "callDeadlineMs": config.runtime.call_deadline_ms,
            "maxInputBytes": config.runtime.max_input_bytes,
            "maxResultBytes": config.runtime.max_result_bytes,
            "logPolicy": config.logs.policy.surface(),
            "logDestination": "stderr",
        },
        "credentialEnv": config.credential_env,
        "credentialEnvBindings": plan.credential_bindings,
        "serverUrlEnv": config.server_url_env,
        "resources": false,
        "prompts": false,
        "retries": false,
        "paginationTraversal": false,
        "tools": plan.tools.iter().map(|tool| self::tool(plan, tool)).collect::<Vec<_>>(),
    });
    OutFile {
        path: "application-surface.json".into(),
        content: format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    }
}

fn tool(plan: &ServerPlan, tool: &planning::BoundTool) -> Value {
    let operation = &plan.sdk.operations()[tool.operation];
    json!({
        "name": tool.name,
        "title": tool.title,
        "description": tool.description,
        "operationId": operation.operation_id,
        "httpMethod": operation.protocol().method().as_str(),
        "httpPath": operation.path,
        "nativeFunction": operation.function_name,
        "source": location(plan, &operation.source),
        "annotations": {
            "readOnlyHint": tool.annotations.read_only,
            "destructiveHint": tool.annotations.destructive,
            "idempotentHint": tool.annotations.idempotent,
            "openWorldHint": tool.annotations.open_world,
        },
        "input": tool.inputs.iter().map(|input| json!({
            "property": input.property,
            "parameter": input.parameter,
            "in": input.location,
            "required": input.required,
            "representation": input.representation.surface(),
            "mediaType": input.media_type,
            "member": input.member,
            "model": input.model,
            "codec": input.codec,
            "schema": location(plan, &input.schema_source),
        })).collect::<Vec<_>>(),
        "responses": tool.responses.iter().map(|response| json!({
            "status": response.status,
            "representation": response.representation(),
            "successStatuses": response.success_document.iter().chain(&response.success_empty).collect::<Vec<_>>(),
            "failureStatuses": response.failure_document.iter().chain(&response.failure_empty).collect::<Vec<_>>(),
            "success": response.success(),
            "failure": response.failure(),
            "mediaType": response.media_type,
            "model": response.model,
            "codec": response.codec,
            "schema": response.schema_source.as_ref().map(|source| location(plan, source)),
        })).collect::<Vec<_>>(),
    })
}

fn location(plan: &ServerPlan, source: &SourceId) -> Value {
    json!({"document": relative(plan, source.document().as_str()), "pointer": source.pointer()})
}

/// Record a contract document relative to the entry document's directory, so
/// the manifest carries no generator path. Planning admitted every document
/// this manifest names through the same scope, refusing any that lies outside
/// the entry tree, so no absolute URI can reach this point.
fn relative(plan: &ServerPlan, uri: &str) -> String {
    plan.scope
        .relative(uri)
        .expect("planning admitted every manifest document into the entry tree")
        .to_owned()
}
