//! SDK-generation readiness notes.
//!
//! Informational diagnostics that mirror how the suspect SDK compiler
//! treats a document. They are not spec violations: an operation without
//! an `operationId` is still selected (as `METHOD /path`), a
//! `text/event-stream` response without a schema generates untyped event
//! payloads, and `:var` path templates are consumed through the
//! colon-path-parameters compatibility profile. Surfacing them at authoring
//! time makes the generation contract visible while writing the spec.

use suspect_oas::OpenApi;

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

pub(crate) fn check_sdk_readiness(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    check_operation_ids(api, out);
    check_stream_schemas(api, out);
    check_colon_paths(api, out);
}

/// The path key an operation lives under (pointer token 1).
fn op_path(op: suspect_oas::Operation<'_>) -> String {
    op.node()
        .path_from_root()
        .tokens()
        .get(1)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

/// Operations without an `operationId` are selectable only by method and
/// path; named operations are the supported authoring surface.
fn check_operation_ids(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for op in api.operations() {
        if op.node().get("operationId").is_none() {
            out.push(diag(
                api,
                "sdk-operation-missing-id",
                Severity::Info,
                op.node().byte_range(),
                format!(
                    "{} operation has no `operationId`; SDK generation falls back to `{} {}` selection",
                    op.method().to_ascii_uppercase(),
                    op.method().to_ascii_uppercase(),
                    op_path(op),
                ),
            ));
        }
    }
}

/// `text/event-stream` responses without a schema generate untyped event
/// payloads; declaring one unlocks typed stream iteration.
fn check_stream_schemas(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for op in api.operations() {
        let Some(responses) = op.node().get("responses") else {
            continue;
        };
        for resp in responses.resolved().entries() {
            let Some(resp_val) = resp.value else {
                continue;
            };
            let Some(content) = resp_val.get("content") else {
                continue;
            };
            for media in content.resolved().entries() {
                if media.key != "text/event-stream" {
                    continue;
                }
                let has_schema = media
                    .value
                    .as_ref()
                    .is_some_and(|m| m.get("schema").is_some());
                if !has_schema {
                    out.push(diag(
                        api,
                        "sdk-stream-response-untyped",
                        Severity::Info,
                        media.key_node.byte_range(),
                        format!(
                            "`text/event-stream` response on {} {} has no `schema`; generated SDK emits untyped stream events",
                            op.method().to_ascii_uppercase(),
                            op_path(op),
                        ),
                    ));
                }
            }
        }
    }
}

/// `:var` path templates are a nonstandard convention consumed through a
/// compatibility profile; `{var}` templates need no profile.
fn check_colon_paths(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let Some(paths) = api.root().get("paths") else {
        return;
    };
    for entry in paths.entries() {
        if entry.key.split('/').any(|seg| seg.starts_with(':')) {
            out.push(diag(
                api,
                "sdk-colon-path-parameters",
                Severity::Info,
                entry.key_node.byte_range(),
                format!(
                    "path `{}` uses `:var` templates; SDK generation treats them via the colon-path-parameters compatibility profile",
                    entry.key
                ),
            ));
        }
    }
}
