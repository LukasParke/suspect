//! The `workspace/executeCommand` dispatcher, its progress reporting and CLI gates.

use std::collections::HashMap;

use serde_json::Value;
use suspect_source::Uri;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::state::offset_of_utf16;
use crate::{commands, impact, latency, links, meaning, navigation, refactor, run_lenses};

/// Verifies a contract package by invoking the CLI in check mode. The
/// editor shells out rather than re-implementing: one implementation of
/// "is the package current", used by both the editor and CI.
fn suspect_cli_contract_check(spec: &std::path::Path, package: &std::path::Path) -> i32 {
    let status = std::process::Command::new("suspect")
        .arg("contract")
        .arg(spec)
        .arg("--out")
        .arg(package)
        .arg("--check")
        .output();
    match status {
        Ok(output) => output.status.code().unwrap_or(1),
        // No CLI on PATH: the editor cannot verify, and saying so is better
        // than reporting the package as current.
        Err(_) => 1,
    }
}

/// One service's gate, run by the CLI so the editor and CI share it.
fn suspect_cli_service_gate(dir: &std::path::Path) -> GateReport {
    let output = std::process::Command::new("suspect")
        .arg("ci")
        .arg(dir)
        .arg("--format")
        .arg("json")
        .output();
    match output {
        Ok(output) => {
            let parsed: serde_json::Value =
                serde_json::from_slice(&output.stdout).unwrap_or(serde_json::Value::Null);
            let passed = parsed["passed"].as_u64().unwrap_or(0);
            let failed = parsed["failed"].as_u64().unwrap_or(0);
            GateReport {
                summary: format!("{passed} project(s) passed, {failed} failed"),
                value: serde_json::json!({
                    "passed": passed,
                    "failed": failed,
                    "exit": output.status.code().unwrap_or(2),
                }),
            }
        }
        Err(error) => GateReport {
            summary: format!("could not run `suspect ci`: {error}"),
            value: serde_json::json!({ "passed": 0, "failed": 0, "exit": 2 }),
        },
    }
}

/// What a service gate run reported.
struct GateReport {
    /// A one-line summary for the progress notification.
    summary: String,
    /// Structured data for the client.
    value: serde_json::Value,
}

impl Backend {
    /// Executes a `suspect.*` command; returns a JSON summary for the client.
    pub(crate) async fn run_command(&self, params: ExecuteCommandParams) -> Option<Value> {
        let command = params.command.as_str();
        match command {
            links::SHOW_REFS_COMMAND => {
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let ptr = params.arguments.get(1)?.as_str()?.to_owned();
                let Ok(uri) = Uri::parse(&uri_s) else {
                    return None;
                };
                let ws = self.workspace_for(&uri).await?;
                let st = self.state.read().await;
                let doc = st.docs.get(&uri)?;
                let off = links::pointer_offset(&doc.low, &ptr)?;
                let refs = navigation::references(ws.as_ref(), &doc.low, off, true);
                let n = refs.len();
                for loc in refs.iter().take(10) {
                    self.client
                        .log_message(MessageType::INFO, format!("referenced at {}", loc.uri))
                        .await;
                }
                Some(serde_json::json!({ "references": n }))
            }
            links::ADD_OPERATION_ID_COMMAND => {
                let ptr = params.arguments.first()?.as_str()?.to_owned();
                let uri_s = params.arguments.get(1)?.as_str()?.to_owned();
                let Ok(uri) = Uri::parse(&uri_s) else {
                    return None;
                };
                let st = self.state.read().await;
                let doc = st.docs.get(&uri)?;
                let edit = links::operation_id_edit(doc, &ptr)?;
                drop(st);
                self.client
                    .apply_edit(WorkspaceEdit::new(
                        [(Url::parse(uri.as_str()).ok()?, vec![edit])]
                            .into_iter()
                            .collect(),
                    ))
                    .await
                    .ok()?;
                Some(serde_json::json!({ "applied": true }))
            }
            // Extract an inline schema to components/schemas, or inline one
            // back: structural refactors over the lossless tree.
            "suspect.extractSchema" | "suspect.inlineSchema" => {
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let offset = params.arguments.get(1)?.as_u64()? as usize;
                let name = params
                    .arguments
                    .get(2)
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                let Ok(uri) = Uri::parse(&uri_s) else {
                    return None;
                };
                let ws = self.workspace_for(&uri).await?;
                // The index comes from the cache and the document is cloned
                // out, so the read guard below is held for pointer work
                // only. Building the index under it walked the whole
                // workspace while holding the lock.
                let index = self.index(&ws).await;
                let planned = {
                    let st = self.state.read().await;
                    let doc = st.docs.get(&uri)?;
                    if command == "suspect.extractSchema" {
                        let name = name.unwrap_or_else(|| refactor::suggest_name(&doc.low, offset));
                        let name = refactor::component_name(&name);
                        refactor::plan_extract(&doc.low, &index, offset, &name)
                    } else {
                        refactor::plan_inline(&doc.low, &index, offset)
                    }
                };
                let plan = match planned {
                    Ok(plan) if !plan.is_empty() => plan,
                    Ok(_) => {
                        return Some(
                            serde_json::json!({ "applied": false, "reason": "nothing to change" }),
                        );
                    }
                    Err(reason) => {
                        self.client
                            .log_message(
                                MessageType::INFO,
                                format!("refactor unavailable: {reason:?}"),
                            )
                            .await;
                        return Some(
                            serde_json::json!({ "applied": false, "reason": format!("{reason:?}") }),
                        );
                    }
                };
                // Turn each planned pointer edit into a range edit.
                let mut changes: std::collections::HashMap<Url, Vec<TextEdit>> =
                    std::collections::HashMap::new();
                for (document, pointer, text) in &plan.edits {
                    let st = self.state.read().await;
                    let Ok(document_uri) = Uri::parse(document) else {
                        continue;
                    };
                    let Some(doc) = st.docs.get(&document_uri) else {
                        continue;
                    };
                    let inner = doc.low.inner();
                    let node = doc.low.root().pointer(pointer);
                    let range = node
                        .map(|node| {
                            let start = inner
                                .line_index()
                                .line_col(inner.bytes(), node.byte_range().start);
                            let end = inner
                                .line_index()
                                .line_col(inner.bytes(), node.byte_range().end);
                            Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1))
                        })
                        .unwrap_or_default();
                    changes
                        .entry(Url::parse(document).ok()?)
                        .or_default()
                        .push(TextEdit {
                            range,
                            new_text: if text.is_empty() {
                                String::new()
                            } else {
                                format!("{text}\n")
                            },
                        });
                }
                drop(self.state.read().await);
                self.client
                    .apply_edit(WorkspaceEdit::new(
                        changes
                            .into_iter()
                            .collect::<std::collections::HashMap<_, _>>(),
                    ))
                    .await
                    .ok()?;
                Some(serde_json::json!({
                    "applied": true,
                    "name": plan.name,
                    "summary": plan.summary,
                }))
            }
            // The editor-latency budgets, measured on the same machine.
            "suspect.editorLatency" => {
                self.progress_begin("suspect.latency", "Measuring editor latency")
                    .await;
                let report = latency::report(1000);
                self.progress_end("suspect.latency", Some("latency measured".to_owned()))
                    .await;
                Some(serde_json::json!({ "report": report }))
            }
            "suspect.generateExample" => {
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let line = params.arguments.get(1)?.as_u64()? as u32;
                let col = params.arguments.get(2)?.as_u64()? as u32;
                let Ok(uri) = Uri::parse(&uri_s) else {
                    return None;
                };
                let ws = self.workspace_for(&uri).await?;
                let st = self.state.read().await;
                let doc = st.docs.get(&uri)?;
                let inner = doc.low.inner();
                let off = offset_of_utf16(inner.bytes(), inner.line_index(), line, col)?;
                let ws = ws.as_ref();
                let generated = commands::generate_example(ws, doc, off).ok()?;
                let mut changes = HashMap::new();
                changes.insert(Url::parse(uri.as_str()).ok()?, vec![generated.insert_edit?]);
                drop(st);
                self.client
                    .apply_edit(WorkspaceEdit::new(changes))
                    .await
                    .ok()?;
                Some(serde_json::json!({ "yaml": generated.yaml_snippet }))
            }
            "suspect.showRefGraph" => {
                let ws = self.any_workspace().await?;
                let mermaid = commands::show_ref_graph(ws.as_ref());
                self.client
                    .log_message(MessageType::INFO, mermaid.clone())
                    .await;
                Some(serde_json::json!({ "mermaid": mermaid }))
            }
            "suspect.breakingChanges" => {
                self.progress_begin("suspect.breaking", "Detecting breaking changes")
                    .await;
                let ws = self.any_workspace().await?;
                let base = std::env::var("SUSPECT_GIT_BASE").unwrap_or_else(|_| "HEAD~1".into());
                let mut old = HashMap::new();
                for uri in ws.uris() {
                    let path = uri.as_str().strip_prefix("file://").unwrap_or(uri.as_str());
                    if let Ok(out) = std::process::Command::new("git")
                        .args(["show", &format!("{base}:{path}")])
                        .current_dir(self.state.read().await.root.clone()?)
                        .output()
                        && out.status.success()
                    {
                        old.insert(
                            uri.to_string(),
                            String::from_utf8_lossy(&out.stdout).into_owned(),
                        );
                    }
                }
                let changes = commands::breaking_changes(ws.as_ref(), &old);
                let n = changes.len();
                for c in changes.iter().take(20) {
                    self.client
                        .log_message(
                            MessageType::WARNING,
                            format!("{:?}: {}", c.severity, c.message),
                        )
                        .await;
                }
                self.progress_end("suspect.breaking", Some(format!("{n} breaking change(s)")))
                    .await;
                Some(serde_json::json!({ "breaking_changes": n }))
            }
            links::OPEN_REF_COMMAND => {
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let line = params.arguments.get(1)?.as_u64()? as u32;
                let character = params.arguments.get(2)?.as_u64()? as u32;
                let Ok(url) = Url::parse(&uri_s) else {
                    return None;
                };
                let selection =
                    Range::new(Position { line, character }, Position { line, character });
                match self
                    .client
                    .show_document(ShowDocumentParams {
                        uri: url,
                        external: None,
                        take_focus: Some(true),
                        selection: Some(selection),
                    })
                    .await
                {
                    Ok(opened) => Some(serde_json::json!({ "opened": opened })),
                    Err(_) => None,
                }
            }
            "suspect.contractCoverage" => {
                self.progress_begin("suspect.coverage", "Computing contract coverage")
                    .await;
                let ws = self.any_workspace().await?;
                let gaps = commands::contract_coverage(ws.as_ref());
                let uncovered = gaps.iter().filter(|g| g.gap).count();
                self.progress_end(
                    "suspect.coverage",
                    Some(format!("{uncovered} uncovered operation(s)")),
                )
                .await;
                Some(serde_json::json!({ "operations": gaps.len(), "uncovered": uncovered }))
            }
            // What a change here reaches, as structured data a client can
            // render however it likes.
            "suspect.changeImpact" => {
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let uri = Uri::parse(&uri_s).ok()?;
                let ws = self.workspace_for(&uri).await?;
                // Clone the document out and drop the guard: `index_for` below takes
                // the write lock, and `doc` would otherwise keep the read
                // guard alive across it.
                let doc = {
                    let st = self.state.read().await;
                    st.docs.get(&uri)?.clone()
                };
                let offset = params
                    .arguments
                    .get(1)
                    .and_then(|value| value.as_u64())
                    .map_or(0, |value| value as usize);
                let model = meaning::Model::new(&doc.low);
                let m = model.at(offset)?;
                let index = self.index(&ws).await;
                let empty: [(String, suspect_arazzo::ArazzoDoc<'_>); 0] = [];
                let no_artifacts = std::collections::BTreeMap::new();
                let no_traffic = std::collections::BTreeMap::new();
                let context = impact::ImpactContext {
                    index: &index,
                    workflows: &empty,
                    artifacts: &no_artifacts,
                    traffic: &no_traffic,
                };
                let report = context.impact_of(&uri_s, &m);
                Some(serde_json::json!({
                    "origin": report.origin,
                    "summary": report.summary(),
                    "operations": report.operations,
                    "workflows": report.workflows,
                    "artifacts": report.artifacts,
                    "traffic": report.traffic,
                    "impacts": report.impacts,
                    "hint": refactor::hint_for(&index),
                }))
            }
            // The contract package this document belongs to, verified
            // against the source the same way CI verifies it.
            "suspect.verifyContract" => {
                self.progress_begin("suspect.contract", "Verifying the contract package")
                    .await;
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let path = std::path::PathBuf::from(&uri_s);
                if !path.is_file() {
                    return None;
                }
                let dir = path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("contract");
                let exit = suspect_cli_contract_check(&path, &dir);
                self.progress_end(
                    "suspect.contract",
                    Some(if exit == 0 {
                        "contract package is current".to_owned()
                    } else {
                        "contract package is stale — run `suspect contract`".to_owned()
                    }),
                )
                .await;
                Some(serde_json::json!({ "current": exit == 0, "package": dir.to_string_lossy() }))
            }
            // One service's whole gate, the same way CI runs it.
            "suspect.runService" => {
                // An explicit directory, else the workspace the document is in.
                let dir = match params.arguments.first().and_then(|value| value.as_str()) {
                    Some(dir) => dir.to_owned(),
                    None => self.state.read().await.workspace_root().map_or_else(
                        || ".".to_owned(),
                        |root| root.to_string_lossy().into_owned(),
                    ),
                };
                self.progress_begin("suspect.service", "Running the service gate")
                    .await;
                let report = suspect_cli_service_gate(std::path::Path::new(&dir));
                self.progress_end("suspect.service", Some(report.summary.clone()))
                    .await;
                Some(report.value)
            }
            run_lenses::RUN_WORKFLOW_COMMAND => {
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let workflow = params.arguments.get(1)?.as_str()?.to_owned();
                let summary = self.run_workflow_uri(&uri_s, &workflow).await.ok()?;
                Some(serde_json::to_value(summary).ok()?)
            }
            run_lenses::RENDER_PREVIEW_COMMAND => {
                let uri_s = params.arguments.first()?.as_str()?.to_owned();
                let preset = params.arguments.get(1)?.as_str()?.to_owned();
                self.render_preview(&uri_s, &preset).await
            }
            _ => None,
        }
    }

    /// Opens a `window/workDoneProgress` report; no-op when the client
    /// declines the create request (capability unsupported).
    pub(crate) async fn progress_begin(&self, token: &str, title: &str) {
        let token = NumberOrString::String(token.to_owned());
        if self
            .client
            .send_request::<request::WorkDoneProgressCreate>(WorkDoneProgressCreateParams {
                token: token.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        self.client
            .send_notification::<tower_lsp::lsp_types::notification::Progress>(ProgressParams {
                token,
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                    WorkDoneProgressBegin {
                        title: title.to_owned(),
                        cancellable: None,
                        message: None,
                        percentage: None,
                    },
                )),
            })
            .await;
    }

    /// Closes a progress report opened by [`Backend::progress_begin`].
    pub(crate) async fn progress_end(&self, token: &str, message: Option<String>) {
        self.client
            .send_notification::<tower_lsp::lsp_types::notification::Progress>(ProgressParams {
                token: NumberOrString::String(token.to_owned()),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(WorkDoneProgressEnd {
                    message,
                })),
            })
            .await;
    }
}
