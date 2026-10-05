#![deny(missing_docs)]
//! suspect-lsp: language server for OpenAPI/Arazzo/Overlay documents.
//!
//! Built on tower-lsp over stdio. Documents sync **incrementally** (ranged
//! edits applied against the live buffer, full-text fallback), and
//! diagnostics (syntax, semantic validation, spectral lint, Arazzo checks)
//! are debounced 150 ms after every change unless superseded. Navigation
//! (`$ref` go-to-definition/declaration/type-definition/implementation,
//! reverse references), hover, document/workspace symbols, folding ranges,
//! selection ranges, highlights, and pragmatic key/`$ref` completion are
//! served from the parsed open documents plus a lazily built
//! [`suspect_ref::Workspace`].
//!
//! Richer surface: semantic tokens (full/range/delta), inlay hints with
//! deferred tooltips, call and type hierarchies, monikers, code lenses
//! (resolve-backed), `$ref` document links, linked editing ranges, hex
//! color swatches, pull diagnostics (document + workspace), and six
//! `suspect.*` commands (show refs, add operationId, generate example,
//! ref graph, breaking changes with progress, contract coverage). Quick
//! fixes for known diagnostic codes ship alongside a deferred
//! `source.fixAll.suspect` action resolved on demand; `window/showDocument`
//! powers the open-target action. Configuration merges initialization
//! options with client settings and drives refresh requests.

pub mod actions;
pub mod call_hierarchy;
pub mod colors;
pub mod commands;
pub mod completion;
pub mod config_files;
pub mod config_schema;
pub mod diagnostics;
pub mod docs_gen;
mod editor_config;
pub mod extensions_config;
pub mod extensions_registry;
pub mod format_order;
pub mod generation_contract;
pub mod hover_detail;
mod impact;
pub mod keys;
pub mod keyword_docs;
mod latency;
pub mod links;
pub mod markdown;
mod meaning;
pub mod navigation;
pub mod pull;
mod rank;
mod refactor;
pub mod rename;
pub mod run_lenses;
pub mod semantic;
pub mod state;
pub mod symbols;
pub mod type_hierarchy_fmt;
pub mod workspace_symbol;

use std::sync::Arc;
use std::time::Duration;

use crate::generation_contract::GenerationContractResult;
use state::{OpenDoc, State, lsp_range, offset_of_utf16};
use std::collections::HashMap;
use suspect_source::Uri;
use tower_lsp::Client;
use tower_lsp::async_trait;

use serde_json::Value;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, CodeLens,
    ConfigurationItem, CreateFilesParams, DeleteFilesParams, DidChangeConfigurationParams,
    DidChangeWorkspaceFoldersParams, DocumentLink, FileRename, FullDocumentDiagnosticReport,
    GotoDefinitionResponse, LinkedEditingRangeServerCapabilities, Location, MessageType, Moniker,
    RelatedFullDocumentDiagnosticReport, RelatedUnchangedDocumentDiagnosticReport,
    RenameFilesParams, SemanticToken, SemanticTokensFullDeltaResult, SemanticTokensPartialResult,
    SemanticTokensRangeResult, TextDocumentPositionParams, TypeHierarchyItem,
    UnchangedDocumentDiagnosticReport, Url, WorkspaceDiagnosticReport,
    WorkspaceDiagnosticReportResult, WorkspaceDocumentDiagnosticReport,
    WorkspaceFullDocumentDiagnosticReport,
};
use tower_lsp::{LanguageServer, LspService, Server};

/// The tower-lsp [`LanguageServer`] backend.
///
/// All mutable server state lives in [`State`], guarded by a single async
/// `RwLock` and shared with spawned debounce tasks via `Arc`. Request
/// handlers take short-lived read locks; only `initialize`/open/change
/// paths take write locks, so requests never block each other for long.
///
/// Lifecycle ("state machine"): `initialize` records the workspace root →
/// each `did_open`/`did_change` reparses the document into the
/// [`State::docs`] cache and bumps [`State::generation`] → a debounce task
/// publishes diagnostics 150 ms later unless superseded →
/// `did_change_watched_files` drops the cached ref workspace so the next
/// navigation query rebuilds it from disk. There is no explicit teardown
/// beyond what `LspService` drops on shutdown.
struct Backend {
    /// Client handle used to publish diagnostics back to the editor.
    client: Client,
    /// Shared mutable server state; also captured by debounce tasks.
    state: Arc<tokio::sync::RwLock<State>>,
}

impl Backend {
    /// Creates a backend with empty [`State`]; called by `LspService::new`.
    fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(tokio::sync::RwLock::new(State::default())),
        }
    }

    /// Schedules a debounced diagnostics publish: 150 ms after the last
    /// change, only the newest generation publishes.
    fn schedule_diagnostics(&self, uri: Uri) {
        let state = Arc::clone(&self.state);
        let client = self.client.clone();
        let Some(url) = Url::parse(uri.as_str()).ok() else {
            return;
        };
        tokio::spawn(async move {
            let generation = {
                let mut st = state.write().await;
                let g = st.generations.entry(uri.clone()).or_insert(0);
                *g += 1;
                *g
            };
            tokio::time::sleep(Duration::from_millis(150)).await;
            // Compute WITHOUT holding the state lock: clone the cheap handles
            // (OpenDoc is small; LowDoc parse tree is Arc-shared internally).
            let (doc, ws, cfg) = {
                let st = state.read().await;
                if st.generations.get(&uri) != Some(&generation) {
                    return; // a newer edit superseded this publish
                }
                match st.docs.get(&uri) {
                    Some(doc) => (doc.clone(), st.workspace.clone(), st.config.clone()),
                    None => return,
                }
            };
            // On the blocking pool. The lint pass is seconds of synchronous
            // CPU work on a 63k-line specification, and running it on a
            // runtime worker means the worker cannot poll anything else:
            // a hover sent during an open waited 4.7 seconds behind it. The
            // state lock is not held here, so this was never a lock
            // problem — it was a scheduler problem.
            let diags = match tokio::task::spawn_blocking({
                let ws = ws.clone();
                let cfg = cfg.clone();
                let doc = doc.clone();
                move || diagnostics::compute_diagnostics(ws.as_ref(), &doc.low, &cfg)
            })
            .await
            {
                Ok(diags) => diags,
                Err(_) => return,
            };
            // The same severity floor the pull path applies. Publishing the
            // unfiltered set was a pre-existing inconsistency — the editor's
            // problem panel showed findings the configured floor suppressed —
            // and caching the push's result made the pull serve it too.
            let floor = {
                let st = state.read().await;
                st.editor_config.min_severity()
            };
            let diags = diagnostics::filter_at_least(diags, floor);
            let id = pull::diagnostics_result_id(&diags);
            // Superseded while computing? Drop the stale result.
            let superseded = {
                let st = state.read().await;
                st.generations.get(&uri) != Some(&generation)
            };
            if superseded {
                return;
            }
            // Keep the result so the pull that follows this push is a cache
            // hit rather than a second full lint pass.
            {
                let mut st = state.write().await;
                if st.generations.get(&uri) == Some(&generation) {
                    let epoch = st.generation();
                    st.diag_cache
                        .insert(uri.clone(), (epoch, id, diags.clone()));
                }
            }
            client.publish_diagnostics(url, diags, None).await;
        });
    }

    /// The semantic index for the current content, building it off-lock.
    ///
    /// `State::cached_index` answers the common case with a read guard. A
    /// miss is built with no lock held at all: the build walks the whole
    /// workspace, and holding the write lock through it blocks every other
    /// request the editor has in flight. Two requests that miss together may
    /// both build; the second store wins and both are equivalent.
    async fn index(
        &self,
        ws: &std::sync::Arc<suspect_ref::Workspace>,
    ) -> std::sync::Arc<crate::meaning::Index> {
        let generation = self.state.read().await.generation();
        if let Some(index) = self.state.read().await.cached_index(generation) {
            return index;
        }
        let built = tokio::task::spawn_blocking({
            let ws = ws.clone();
            move || std::sync::Arc::new(crate::meaning::Index::build(&ws))
        })
        .await;
        // A panicked build must not fail the request: an empty index answers
        // the question with less detail rather than not at all.
        let index = built.unwrap_or_default();
        self.state
            .write()
            .await
            .store_index(generation, index.clone());
        index
    }

    /// A whole-document response from the cache, if the content is unchanged
    /// since it was computed.
    ///
    /// The response types are big — links for a 63k-line specification
    /// serialise to 1.7MB — so the cache stores `Arc`s and a hit is a
    /// pointer clone rather than a payload copy.
    async fn doc_cached<T>(
        &self,
        uri: &Uri,
        pick: fn(&state::DocCache) -> Option<std::sync::Arc<T>>,
    ) -> Option<std::sync::Arc<T>> {
        let st = self.state.read().await;
        let (epoch, cache) = st.doc_cache.get(uri)?;
        if *epoch != st.generation() {
            return None;
        }
        pick(cache)
    }

    /// Records a whole-document response against the epoch it was computed
    /// for. Stale entries are replaced rather than merged: a lookup checks
    /// the epoch before trusting anything, so an edit landing mid-compute
    /// cannot resurrect a stale answer.
    async fn doc_store<T>(
        &self,
        uri: &Uri,
        epoch: u64,
        set: fn(&mut state::DocCache, Option<std::sync::Arc<T>>),
        value: std::sync::Arc<T>,
    ) {
        let mut st = self.state.write().await;
        let entry = st
            .doc_cache
            .entry(uri.clone())
            .or_insert_with(|| (epoch, state::DocCache::default()));
        if entry.0 != epoch {
            *entry = (epoch, state::DocCache::default());
        }
        set(&mut entry.1, Some(value));
    }

    /// Returns the workspace, building it on first use and making sure the
    /// given document plus its `$ref` closure is loaded. Best-effort.
    async fn workspace_for(&self, uri: &Uri) -> Option<Arc<suspect_ref::Workspace>> {
        // A cached workspace is cloned out under a *read* guard. Taking a
        // write lock here deadlocked any handler already holding a read
        // guard: tokio's RwLock is write-preferring, so the writer waits
        // for a reader that is this same task, and never wakes. One such
        // call wedged the whole server, because the queued writer then
        // blocked every later reader too.
        let cached = self.state.read().await.workspace.clone();
        let ws = match cached {
            Some(ws) => ws,
            None => {
                let mut st = self.state.write().await;
                st.ensure_workspace()?
            }
        };
        if ws.get(uri).is_none()
            && let Some(path) = uri.as_path()
        {
            let _ = ws.load_all(&path.to_string_lossy());
        }
        Some(ws)
    }

    /// Builds the reference index before anyone asks for it.
    ///
    /// An open document is a near-certainty that a hover follows, so the
    /// workspace walk happens now — off the interaction path, on the
    /// blocking pool, without the state lock held while it runs. Editing
    /// invalidates the result, so the next edit warms it again and the
    /// build never lands in front of a cursor move.
    fn warm_index(&self) {
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let (ws, generation) = {
                let st = state.read().await;
                let Some(ws) = st.workspace.as_ref() else {
                    return;
                };
                (Arc::clone(ws), st.generation())
            };
            let built =
                tokio::task::spawn_blocking(move || Arc::new(meaning::Index::build(&ws))).await;
            let Ok(index) = built else {
                return;
            };
            let mut st = state.write().await;
            // A document changed while we were walking the workspace: what
            // we built is already stale, so leave the cache to whoever
            // needs it next.
            if st.generation() == generation {
                st.index_cache = Some((generation, index));
            }
        });
    }
}

/// The model-derived half of a hover: where the cursor is, and what a
/// change here would reach.
///
/// A competitor's hover stops at the schema. Ours answers the question an
/// author actually has before editing a definition: who depends on this?
fn hover_meaning(
    low: &suspect_low::LowDoc,
    offset: usize,
    index: &meaning::Index,
) -> Option<String> {
    let model = meaning::Model::new(low);
    let m = model.at(offset)?;
    // The italic kind/dialect line matches the subtitle register of the
    // card it is appended to, so the context reads as the card's footer
    // rather than a second, clashing design.
    let mut out = format!("*{} — {}*", m.kind.label(), model.dialect().label());
    // The pointer of a property buried in an operation is mostly path
    // noise: what an author wants is the way down from the schema they
    // are editing. Everything after the last `schema` keyword is it.
    let tokens: Vec<&str> = m
        .pointer
        .tokens()
        .iter()
        .map(|token| token.as_ref())
        .collect();
    if m.within(meaning::ObjectKind::Schema)
        && let Some(at) = tokens.iter().rposition(|token| *token == "schema")
    {
        let below: Vec<&str> = tokens[at + 1..]
            .iter()
            .copied()
            .filter(|token| !token.is_empty())
            .collect();
        if !below.is_empty() {
            out.push_str(&format!("\n\ninside `{}`", below.join(" \u{203a} ")));
        }
    }
    if m.pointer.to_path() != "/" {
        out.push_str(&format!("\n\n`{}`", m.pointer.to_path()));
    }
    if let Some(target) = &m.ref_target {
        out.push_str(&format!("\n\nresolves to `{}`", target.pointer.to_path()));
    }

    // Change impact, from the reference index only: no contract compile.
    let uri = low.uri().as_str();
    let empty: [(String, suspect_arazzo::ArazzoDoc<'_>); 0] = [];
    let no_artifacts = std::collections::BTreeMap::new();
    let no_traffic = std::collections::BTreeMap::new();
    let context = impact::ImpactContext {
        index,
        workflows: &empty,
        artifacts: &no_artifacts,
        traffic: &no_traffic,
    };
    let report = context.impact_of(uri, &m);
    if !report.is_local() {
        out.push_str(&format!("\n\n**Impact** — {}\n\n", report.summary()));
        for entry in report.impacts.iter().take(6) {
            out.push_str(&format!(
                "- {} **{}** — {}\n",
                entry.kind.label(),
                entry.subject,
                entry.via
            ));
        }
    }
    Some(out)
}

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

/// The workspace root directory, when the workspace has one on disk.
#[must_use]
pub fn workspace_root(ws: &Arc<suspect_ref::Workspace>) -> Option<std::path::PathBuf> {
    ws.root_path().map(std::path::Path::to_path_buf)
}

/// Parses a [`Uri`] into an LSP [`Url`], or `None` when it does not parse.
fn to_url(uri: &Uri) -> Option<Url> {
    Url::parse(uri.as_str()).ok()
}

/// Converts a navigation target into an LSP `Location`, preferring the open
/// (possibly dirty) buffer when it is the same document, else the
/// workspace-loaded copy.
/// The configuration flavour of a document, when it is one of suspect's own
/// configuration files.
fn config_kind(uri: &Uri) -> Option<config_schema::FileKind> {
    let path = uri.as_path()?;
    config_schema::kind_of(path.file_name()?.to_str()?)
}

fn to_location(
    ws: Option<&Arc<suspect_ref::Workspace>>,
    open: Option<&OpenDoc>,
    def: &navigation::Definition,
) -> Option<Location> {
    let (bytes, li) = if open.is_some_and(|d| d.low.uri() == &def.uri) {
        let inner = open.unwrap().low.inner();
        (inner.bytes(), inner.line_index())
    } else {
        let handle = ws?.get(&def.uri)?;
        let inner = handle.doc().inner();
        (inner.bytes(), inner.line_index())
    };
    Some(Location {
        uri: to_url(&def.uri)?,
        range: lsp_range(bytes, li, def.range.clone()),
    })
}

/// `**/*.{yaml,yml,json}` file-operation filter used by all six hooks.
fn yaml_file_filter() -> Vec<FileOperationFilter> {
    vec![FileOperationFilter {
        scheme: Some("file".to_owned()),
        pattern: FileOperationPattern {
            glob: "**/*.{yaml,yml,json}".to_owned(),
            matches: None,
            options: None,
        },
    }]
}

#[async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> JsonRpcResult<InitializeResult> {
        {
            let mut st = self.state.write().await;
            st.root = params.root_uri.as_ref().and_then(|u| u.to_file_path().ok());
            st.pending_init_options = params.initialization_options.clone();
            st.client_caps = Some(params.capabilities.clone());
            if st.workspace.is_none()
                && let Some(root) = &st.root
                && let Ok(ws) = suspect_ref::WorkspaceBuilder::new().root(root).build()
            {
                let _ = ws.load_all("main.yaml");
                st.workspace = Some(Arc::new(ws));
            }
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        will_save: Some(true),
                        will_save_wait_until: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(false),
                        })),
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(true),
                    ..CompletionOptions::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::new("source.fixAll.suspect"),
                        ]),
                        resolve_provider: Some(true),
                    },
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                            legend: semantic::legend(),
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
                        },
                    ),
                ),
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        resolve_provider: Some(true),
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                    },
                ))),
                document_highlight_provider: Some(OneOf::Left(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                color_provider: Some(ColorProviderCapability::Simple(true)),
                declaration_provider: Some(DeclarationCapability::Simple(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                moniker_provider: Some(OneOf::Left(true)),
                linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(
                    true,
                )),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some("suspect".to_owned()),
                        inter_file_dependencies: true,
                        workspace_diagnostics: true,
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                    },
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: [
                        links::SHOW_REFS_COMMAND.to_owned(),
                        links::OPEN_REF_COMMAND.to_owned(),
                        links::ADD_OPERATION_ID_COMMAND.to_owned(),
                        "suspect.generateExample".to_owned(),
                        "suspect.showRefGraph".to_owned(),
                        "suspect.breakingChanges".to_owned(),
                        "suspect.contractCoverage".to_owned(),
                        "suspect.changeImpact".to_owned(),
                        "suspect.verifyContract".to_owned(),
                        "suspect.runService".to_owned(),
                        "suspect.extractSchema".to_owned(),
                        "suspect.inlineSchema".to_owned(),
                        "suspect.editorLatency".to_owned(),
                        run_lenses::RUN_WORKFLOW_COMMAND.to_owned(),
                        run_lenses::RENDER_PREVIEW_COMMAND.to_owned(),
                    ]
                    .to_vec(),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                        did_create: Some(FileOperationRegistrationOptions {
                            filters: yaml_file_filter(),
                        }),
                        will_create: Some(FileOperationRegistrationOptions {
                            filters: yaml_file_filter(),
                        }),
                        did_rename: Some(FileOperationRegistrationOptions {
                            filters: yaml_file_filter(),
                        }),
                        will_rename: Some(FileOperationRegistrationOptions {
                            filters: yaml_file_filter(),
                        }),
                        did_delete: Some(FileOperationRegistrationOptions {
                            filters: yaml_file_filter(),
                        }),
                        will_delete: Some(FileOperationRegistrationOptions {
                            filters: yaml_file_filter(),
                        }),
                    }),
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "suspect-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> JsonRpcResult<Option<SemanticTokensResult>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // The delta handler keeps the token cache warm on every request, so
        // this is usually a hit; a miss computes once and stores for both.
        // One guard for the epoch and the entry: reading them under two
        // separate guards leaves a window where an edit lands between them
        // and stale tokens are served as current.
        if let Some(data) = {
            let st = self.state.read().await;
            let epoch = st.generation();
            st.token_cache
                .get(&uri)
                .and_then(|(cached, _id, data)| (*cached == epoch).then(|| data.clone()))
        } {
            return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data,
            })));
        }
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (doc.clone(), st.generation())
        };
        let tokens = semantic::semantic_tokens_full(doc.as_ref());
        let id = pull::tokens_result_id(&tokens.data);
        self.state
            .write()
            .await
            .token_cache
            .insert(uri, (epoch, id, tokens.data.clone()));
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens.data,
        })))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> JsonRpcResult<Option<Vec<InlayHint>>> {
        let uri = Uri::parse(params.text_document.uri.as_str()).ok();
        let Some(uri) = uri else { return Ok(None) };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let mut hints = semantic::inlay_hints(doc, &ws, params.range);
        if !st.config.inlay_ref_targets() {
            hints.retain(|h| h.data.is_none());
        }
        Ok(Some(hints))
    }

    async fn inlay_hint_resolve(&self, hint: InlayHint) -> JsonRpcResult<InlayHint> {
        let Some(uri) = hint
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
            .and_then(|s| Uri::parse(s).ok())
        else {
            return Ok(hint);
        };
        match self.workspace_for(&uri).await {
            Some(ws) => Ok(semantic::resolve_inlay_hint(hint, &ws)),
            None => Ok(hint),
        }
    }

    async fn document_color(
        &self,
        params: DocumentColorParams,
    ) -> JsonRpcResult<Vec<ColorInformation>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(Vec::new());
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.colors.clone()).await {
            return Ok((*cached).clone());
        }
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(Vec::new());
            };
            (doc.clone(), st.generation())
        };
        let colors_out = std::sync::Arc::new(colors::document_colors(doc.as_ref()));
        self.doc_store(&uri, epoch, |cache, v| cache.colors = v, colors_out.clone())
            .await;
        Ok((*colors_out).clone())
    }

    async fn color_presentation(
        &self,
        params: ColorPresentationParams,
    ) -> JsonRpcResult<Vec<ColorPresentation>> {
        Ok(colors::color_presentations(&params.color, params.range))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> JsonRpcResult<Option<Vec<DocumentHighlight>>> {
        let pos = params.text_document_position_params.position;
        let Ok(uri) = Uri::parse(
            params
                .text_document_position_params
                .text_document
                .uri
                .as_str(),
        ) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        Ok(Some(semantic::document_highlights(doc, offset)))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> JsonRpcResult<Option<Vec<SelectionRange>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        Ok(Some(semantic::selection_ranges(doc, &params.positions)))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        let Ok(uri) = Uri::parse(item.uri.as_str()) else {
            return;
        };
        {
            let mut st = self.state.write().await;
            st.open_doc(uri.clone(), item.text);
        }
        // Force the workspace load here rather than letting the first
        // hover discover it, then warm the index against it.
        let _ = self.workspace_for(&uri).await;
        self.warm_index();
        self.schedule_diagnostics(uri);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return;
        };
        {
            let mut st = self.state.write().await;
            st.close_doc(&uri);
        }
        // Clear diagnostics for the closed document.
        if let Ok(url) = Url::parse(uri.as_str()) {
            self.client.publish_diagnostics(url, Vec::new(), None).await;
        }
    }
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Incremental sync: apply every change in order against the live
        // buffer (full-text changes are the range-less special case), then
        // reparse once.
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return;
        };
        {
            let mut st = self.state.write().await;
            let Some(doc) = st.docs.get(&uri).map(|d| d.text.clone()) else {
                return;
            };
            let Some(text) = state::apply_content_changes(&doc, &params.content_changes) else {
                return; // malformed ranges: keep the last good buffer
            };
            // Skip the reparse + republish cycle when the text is unchanged.
            if text == doc {
                return;
            }
            st.open_doc(uri.clone(), text);
        }
        self.warm_index();
        self.schedule_diagnostics(uri);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // The open buffer stays authoritative (include_text is false, the
        // save carries no text); what changes is the on-disk copy that
        // *other* documents' `$ref`s resolve against, so drop the cached
        // workspace for a fresh disk-backed rebuild.
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return;
        };
        if self.state.read().await.docs.contains_key(&uri) {
            let mut st = self.state.write().await;
            st.drop_workspace();
        }
    }

    async fn did_change_watched_files(&self, _params: DidChangeWatchedFilesParams) {
        // Drop the cached workspace so the next query reloads from disk.
        let mut st = self.state.write().await;
        st.drop_workspace();
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        let pos = params.text_document_position_params.position;
        let Ok(uri) = Uri::parse(
            params
                .text_document_position_params
                .text_document
                .uri
                .as_str(),
        ) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        let def = navigation::goto_definition(&ws, &doc.low, offset)
            .or_else(|| navigation::self_definition(&doc.low, offset));
        let open = st.docs.get(&uri).map(|d| d.as_ref());
        Ok(def
            .as_ref()
            .and_then(|d| to_location(Some(&ws), open, d))
            .map(GotoDefinitionResponse::Scalar))
    }

    async fn references(&self, params: ReferenceParams) -> JsonRpcResult<Option<Vec<Location>>> {
        let pos = params.text_document_position.position;
        let Ok(uri) = Uri::parse(params.text_document_position.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        let defs =
            navigation::references(&ws, &doc.low, offset, params.context.include_declaration);
        let open = st.docs.get(&uri).map(|d| d.as_ref());
        let locs = defs
            .iter()
            .filter_map(|d| to_location(Some(&ws), open, d))
            .collect::<Vec<_>>();
        Ok(Some(locs).filter(|l| !l.is_empty()))
    }

    async fn hover(&self, params: HoverParams) -> JsonRpcResult<Option<Hover>> {
        let pos = params.text_document_position_params.position;
        let Ok(uri) = Uri::parse(
            params
                .text_document_position_params
                .text_document
                .uri
                .as_str(),
        ) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        // Take only what the answer needs, then release the lock. Holding
        // the read guard across the computation blocked the diagnostics
        // writer and every other hover, which is what a client sees as a
        // hover that never resolves.
        let doc = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            doc.clone()
        };
        let index = self.index(&ws).await;
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        // A suspect configuration file is not an API document, and its keys
        // have documentation no OpenAPI vocabulary carries.
        if let Some(kind) = config_kind(&uri) {
            return Ok(
                config_schema::hover(kind, &doc.low, offset).map(|value| Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    }),
                    range: None,
                }),
            );
        }
        let Some(base) = navigation::hover_markdown(&ws, &doc.low, offset) else {
            return Ok(None);
        };
        // The semantic model narrates where the cursor is and what a change
        // here would cost, on top of whatever the hover already resolved.
        let value = match hover_meaning(&doc.low, offset, &index) {
            Some(extra) if !base.is_empty() => format!("{base}\n\n---\n{extra}"),
            Some(extra) => extra,
            None => base,
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> JsonRpcResult<Option<DocumentSymbolResponse>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let syms = symbols::document_symbols(&doc.low);
        Ok((!syms.is_empty()).then_some(DocumentSymbolResponse::Nested(syms)))
    }

    async fn folding_range(
        &self,
        params: FoldingRangeParams,
    ) -> JsonRpcResult<Option<Vec<FoldingRange>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.folds.clone()).await {
            return Ok((!cached.is_empty()).then(|| (*cached).clone()));
        }
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (doc.clone(), st.generation())
        };
        let ranges = std::sync::Arc::new(symbols::folding_ranges(&doc.low));
        self.doc_store(&uri, epoch, |cache, v| cache.folds = v, ranges.clone())
            .await;
        Ok((!ranges.is_empty()).then(|| (*ranges).clone()))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> JsonRpcResult<Option<PrepareRenameResponse>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(offset) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            params.position.line,
            params.position.character,
        ) else {
            return Ok(None);
        };
        match rename::prepare_rename(doc, offset) {
            Some(ph) => Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                range: lsp_range(inner.bytes(), inner.line_index(), ph.range),
                placeholder: ph.placeholder,
            })),
            // Honest failure: the position is not on a renameable key.
            None => Err(tower_lsp::jsonrpc::Error::invalid_params(
                "no renameable component key at this position".to_owned(),
            )),
        }
    }

    async fn rename(&self, params: RenameParams) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let pos = params.text_document_position.position;
        let Ok(uri) = Uri::parse(params.text_document_position.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        match rename::rename(&ws, doc, &params.new_name, offset) {
            Ok(edit) => Ok(Some(edit)),
            Err(msg) => Err(tower_lsp::jsonrpc::Error::invalid_params(msg)),
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> JsonRpcResult<Option<Vec<SymbolInformation>>> {
        let ws = {
            let mut st = self.state.write().await;
            st.ensure_workspace()
        };
        let Some(ws) = ws else { return Ok(None) };
        let syms = workspace_symbol::workspace_symbols(&ws, &params.query);
        Ok((!syms.is_empty()).then_some(syms))
    }

    async fn symbol_resolve(&self, symbol: WorkspaceSymbol) -> JsonRpcResult<WorkspaceSymbol> {
        // Flat `symbol` responses carry no `data`, so ordinary clients
        // never hit this; a caller that does gets a range refreshed
        // against the live workspace.
        match self.any_workspace().await {
            Some(ws) => Ok(workspace_symbol::resolve_workspace_symbol(symbol, &ws)),
            None => Ok(symbol),
        }
    }

    // ---------- pull diagnostics ----------
    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> JsonRpcResult<DocumentDiagnosticReportResult> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(full_report(None, Vec::new()));
        };
        let ws = self.workspace_for(&uri).await;
        // Take everything the report needs and drop the guard before the
        // lint runs. Holding a read lock across the computation and then
        // taking a second read for the severity floor deadlocks against
        // `warm_index`, which `didOpen` spawns and which queues a writer
        // once its blocking build finishes: tokio's `RwLock` is
        // write-preferring, so that second read waits for a writer that is
        // itself waiting for the first guard to drop. The queued writer
        // then blocks every later reader, so one diagnostic silences the
        // whole server — and an editor sends this in the same instant as
        // `didOpen`, so it is every file open rather than a rare race.
        // A push for this exact content already ran the battery; reuse it.
        // The epoch guard means any edit since invalidates the entry.
        let cached = {
            let st = self.state.read().await;
            match st.diag_cache.get(&uri) {
                Some((epoch, id, items)) if *epoch == st.generation() => {
                    Some((id.clone(), items.clone()))
                }
                _ => None,
            }
        };
        if let Some((id, items)) = cached {
            if params.previous_result_id.as_deref() == Some(id.as_str()) {
                return Ok(unchanged_report(id));
            }
            return Ok(full_report(Some(id), items));
        }

        let (doc, cfg, floor) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(full_report(None, Vec::new()));
            };
            // The shared severity floor, applied identically in the editor
            // and on the command line.
            (
                doc.clone(),
                st.config.clone(),
                st.editor_config.min_severity(),
            )
        };
        // Off the runtime, for the same reason as the push: this is seconds
        // of synchronous work, and holding a worker for that long stalls
        // every other request the editor has in flight.
        let previous = params.previous_result_id.clone();
        let id_and_items = tokio::task::spawn_blocking({
            let ws = ws.clone();
            let cfg = cfg.clone();
            let doc = doc.clone();
            move || match &ws {
                Some(ws) => pull::pull_diagnostics(ws, &doc.low, previous, &cfg),
                None => {
                    let items = diagnostics::compute_diagnostics(ws.as_ref(), &doc.low, &cfg);
                    (pull::diagnostics_result_id(&items), items)
                }
            }
        })
        .await;
        let Ok((id, items)) = id_and_items else {
            return Ok(full_report(None, Vec::new()));
        };
        let items = diagnostics::filter_at_least(items, floor);
        {
            let mut st = self.state.write().await;
            let epoch = st.generation();
            st.diag_cache
                .insert(uri, (epoch, id.clone(), items.clone()));
        }
        if params.previous_result_id.as_deref() == Some(id.as_str()) {
            return Ok(unchanged_report(id));
        }
        Ok(full_report(Some(id), items))
    }

    async fn workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> JsonRpcResult<WorkspaceDiagnosticReportResult> {
        // Served from cache when the content and the config are unchanged:
        // this walks every document in the project, and an editor asks for it
        // in the same burst as the requests it is waiting on.
        let cached: Option<(String, Vec<WorkspaceDocumentDiagnosticReport>)> = {
            let st = self.state.read().await;
            st.ws_diag_cache
                .as_ref()
                .and_then(|(epoch, config, id, items)| {
                    let fresh = *epoch == st.generation() && *config == st.config;
                    fresh.then(|| (id.clone(), items.clone()))
                })
        };
        if let Some((_id, items)) = cached {
            return Ok(WorkspaceDiagnosticReportResult::Report(
                WorkspaceDiagnosticReport { items },
            ));
        }

        // Same discipline as `diagnostic`: nothing expensive runs while a state
        // guard is alive, because a queued writer behind it blocks every
        // later reader — including the hover that follows.
        let (workspace, cfg) = {
            let st = self.state.read().await;
            (st.workspace.clone(), st.config.clone())
        };
        let mut items = Vec::new();
        let previous: HashMap<String, String> = params
            .previous_result_ids
            .iter()
            .map(|p| (p.uri.to_string(), p.value.clone()))
            .collect();
        if let Some(ws) = &workspace {
            let cfg = cfg.clone();
            for (uri, diags) in pull::workspace_pull(ws, &cfg) {
                let Ok(url) = Url::parse(uri.as_str()) else {
                    continue;
                };
                let id = pull::diagnostics_result_id(&diags);
                if previous.get(uri.as_str()) == Some(&id) {
                    items.push(WorkspaceDocumentDiagnosticReport::Unchanged(
                        WorkspaceUnchangedDocumentDiagnosticReport {
                            uri: url,
                            version: None,
                            unchanged_document_diagnostic_report:
                                UnchangedDocumentDiagnosticReport { result_id: id },
                        },
                    ));
                    continue;
                }
                items.push(WorkspaceDocumentDiagnosticReport::Full(
                    WorkspaceFullDocumentDiagnosticReport {
                        uri: url,
                        version: None,
                        full_document_diagnostic_report: FullDocumentDiagnosticReport {
                            result_id: Some(id),
                            items: diags,
                        },
                    },
                ));
            }
        }
        let id = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hasher::write(
                &mut hasher,
                serde_json::to_string(&items).unwrap_or_default().as_bytes(),
            );
            format!("ws-{:016x}", std::hash::Hasher::finish(&hasher))
        };
        {
            let mut st = self.state.write().await;
            let epoch = st.generation();
            let config = st.config.clone();
            st.ws_diag_cache = Some((epoch, config, id.clone(), items.clone()));
        }
        Ok(WorkspaceDiagnosticReportResult::Report(
            WorkspaceDiagnosticReport { items },
        ))
    }

    // ---------- semantic tokens range / delta ----------
    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> JsonRpcResult<Option<SemanticTokensRangeResult>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let bytes = inner.bytes();
        let li = inner.line_index();
        let start = offset_of_utf16(
            bytes,
            li,
            params.range.start.line,
            params.range.start.character,
        )
        .unwrap_or(0);
        let end = offset_of_utf16(bytes, li, params.range.end.line, params.range.end.character)
            .unwrap_or(bytes.len());
        let tokens = pull::semantic_tokens_range(doc, li, start..end.max(start));
        Ok(Some(SemanticTokensRangeResult::Partial(
            SemanticTokensPartialResult {
                data: encode_tokens(tokens),
            },
        )))
    }

    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> JsonRpcResult<Option<SemanticTokensFullDeltaResult>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // Read the cached entry before any compute: it is both the
        // unchanged-content fast path and the "previous" set a delta is
        // computed from. This path used to recompute every token on each
        // request — 272ms on a 63k-line specification, per request — with no
        // way to tell whether the cache was current.
        let (doc, epoch, cached) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (
                doc.clone(),
                st.generation(),
                st.token_cache.get(&uri).cloned(),
            )
        };
        let (new_id, full_data) = match &cached {
            // Unchanged content: the cached set is the answer.
            Some((cached_epoch, id, data)) if *cached_epoch == epoch => (id.clone(), data.clone()),
            // Changed content (or nothing cached): compute, and keep the
            // previous set around long enough to delta from it below.
            _ => {
                let full = semantic::semantic_tokens_full(doc.as_ref());
                let id = pull::tokens_result_id(&full.data);
                let data = full.data;
                self.state
                    .write()
                    .await
                    .token_cache
                    .insert(uri.clone(), (epoch, id.clone(), data.clone()));
                (id, data)
            }
        };
        if params.previous_result_id == new_id {
            return Ok(Some(SemanticTokensFullDeltaResult::Tokens(
                SemanticTokens {
                    result_id: Some(new_id),
                    data: Vec::new(),
                },
            )));
        }
        // The client's previous result is one we still hold: delta from it.
        if let Some((_, prev_id, prev)) = cached
            && prev_id == params.previous_result_id
        {
            let delta = pull::semantic_tokens_delta(&prev, &full_data);
            return Ok(Some(delta));
        }
        // Otherwise the client's previous result is one this server never
        // held (a restart, or eviction): hand back the full set.
        Ok(Some(SemanticTokensFullDeltaResult::Tokens(
            SemanticTokens {
                result_id: None,
                data: full_data,
            },
        )))
    }

    // ---------- navigation additions ----------
    async fn goto_declaration(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        self.goto_like(
            params.text_document_position_params.clone(),
            |ws, low, off| {
                call_hierarchy::declaration(ws, low, off).map(GotoDefinitionResponse::Array)
            },
        )
        .await
    }

    async fn goto_type_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        self.goto_like(
            params.text_document_position_params.clone(),
            |ws, low, off| {
                call_hierarchy::type_definition(ws, low, off).map(GotoDefinitionResponse::Array)
            },
        )
        .await
    }

    // implementation = schemas that compose this one (allOf subtypes)
    async fn goto_implementation(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        let tdp = params.text_document_position_params.clone();
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(off) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        ) else {
            return Ok(None);
        };
        let Some(item) = type_hierarchy_fmt::prepare_type_hierarchy(ws.as_ref(), doc, off)
            .and_then(|items| items.into_iter().next())
        else {
            return Ok(None);
        };
        let locs: Vec<Location> = type_hierarchy_fmt::subtypes(ws.as_ref(), &item)
            .into_iter()
            .map(|t| Location {
                uri: t.uri,
                range: t.range,
            })
            .collect();
        if locs.is_empty() {
            Ok(None)
        } else {
            Ok(Some(GotoDefinitionResponse::Array(locs)))
        }
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> JsonRpcResult<Option<Vec<CallHierarchyItem>>> {
        self.with_doc_offset(params.text_document_position_params.clone(), |ws, low, off| {
            Ok(call_hierarchy::prepare_call_hierarchy(ws, low, off).map(|i| vec![i]))
        })
        .await
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> JsonRpcResult<Option<Vec<CallHierarchyIncomingCall>>> {
        let item = params.item;
        let Ok(uri) = Uri::parse(item.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        Ok(Some(call_hierarchy::incoming_calls(&ws, &item)))
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> JsonRpcResult<Option<Vec<CallHierarchyOutgoingCall>>> {
        let item = params.item;
        let Ok(uri) = Uri::parse(item.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        Ok(Some(call_hierarchy::outgoing_calls(&ws, &item)))
    }

    async fn prepare_type_hierarchy(
        &self,
        params: TypeHierarchyPrepareParams,
    ) -> JsonRpcResult<Option<Vec<TypeHierarchyItem>>> {
        let tdp = params.text_document_position_params.clone();
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(off) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        ) else {
            return Ok(None);
        };
        Ok(type_hierarchy_fmt::prepare_type_hierarchy(
            ws.as_ref(),
            doc,
            off,
        ))
    }

    async fn supertypes(
        &self,
        params: TypeHierarchySupertypesParams,
    ) -> JsonRpcResult<Option<Vec<TypeHierarchyItem>>> {
        let item = params.item;
        let Ok(uri) = Uri::parse(item.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        Ok(Some(type_hierarchy_fmt::supertypes(&ws, &item)))
    }

    async fn subtypes(
        &self,
        params: TypeHierarchySubtypesParams,
    ) -> JsonRpcResult<Option<Vec<TypeHierarchyItem>>> {
        let item = params.item;
        let Ok(uri) = Uri::parse(item.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        Ok(Some(type_hierarchy_fmt::subtypes(&ws, &item)))
    }

    async fn moniker(&self, params: MonikerParams) -> JsonRpcResult<Option<Vec<Moniker>>> {
        self.with_doc_offset(
            params.text_document_position_params.clone(),
            |ws, low, off| Ok(links::moniker(ws, low, off)),
        )
        .await
    }

    async fn linked_editing_range(
        &self,
        params: LinkedEditingRangeParams,
    ) -> JsonRpcResult<Option<LinkedEditingRanges>> {
        self.with_doc_offset(
            params.text_document_position_params.clone(),
            |_ws, low, off| {
                Ok(
                    pull::linked_editing_range(low, off).map(|ranges| LinkedEditingRanges {
                        ranges,
                        word_pattern: None,
                    }),
                )
            },
        )
        .await
    }

    // ---------- lenses / links ----------
    async fn code_lens(&self, params: CodeLensParams) -> JsonRpcResult<Option<Vec<CodeLens>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.lenses.clone()).await {
            return Ok(Some((*cached).clone()));
        }
        let ws = self.workspace_for(&uri).await;
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (doc.clone(), st.generation())
        };
        // Arazzo documents get workflow run lenses instead of the
        // OpenAPI component/operation lenses; those are cheap and uncached.
        if doc.low.sniff_family() == suspect_low::SpecFamily::Arazzo10 {
            return Ok(Some(run_lenses::run_lenses(&doc.low)));
        }
        let Some(ws) = ws else {
            return Ok(None);
        };
        let lenses = std::sync::Arc::new(links::code_lens(&ws, &doc.low));
        self.doc_store(&uri, epoch, |cache, v| cache.lenses = v, lenses.clone())
            .await;
        Ok(Some((*lenses).clone()))
    }

    async fn code_lens_resolve(&self, params: CodeLens) -> JsonRpcResult<CodeLens> {
        if params.data.as_ref().and_then(|d| d.get("ptr")).is_none() {
            return Ok(params);
        }
        let uri_str = params
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
            .unwrap_or_default();
        let Ok(uri) = Uri::parse(uri_str) else {
            return Ok(params);
        };
        let ws = self.workspace_for(&uri).await;
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(params);
        };
        match ws {
            Some(ws) => Ok(links::code_lens_resolve(&ws, &doc.low, params)),
            None => Ok(params),
        }
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> JsonRpcResult<Option<Vec<DocumentLink>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // The document is taken and the guard released before the
        // workspace is asked for: holding one across the other is what
        // deadlocked this handler, and with it every request behind it.
        let doc = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            doc.clone()
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.links.clone()).await {
            return Ok(Some((*cached).clone()));
        }
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(Some(Vec::new()));
        };
        let epoch = {
            let st = self.state.read().await;
            st.generation()
        };
        let built = std::sync::Arc::new(links::document_link(ws.as_ref(), &doc.low));
        self.doc_store(&uri, epoch, |cache, v| cache.links = v, built.clone())
            .await;
        Ok(Some((*built).clone()))
    }

    async fn document_link_resolve(&self, params: DocumentLink) -> JsonRpcResult<DocumentLink> {
        Ok(links::document_link_resolve(params))
    }

    // ---------- formatting extras ----------
    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let bytes = inner.bytes();
        let li = inner.line_index();
        let Some(start) = offset_of_utf16(
            bytes,
            li,
            params.range.start.line,
            params.range.start.character,
        ) else {
            return Ok(None);
        };
        let Some(end) =
            offset_of_utf16(bytes, li, params.range.end.line, params.range.end.character)
        else {
            return Ok(None);
        };
        let edits = type_hierarchy_fmt::range_formatting(doc, start..end);
        Ok((!edits.is_empty()).then_some(edits))
    }

    // ---------- workspace sync / configuration / folders ----------
    async fn initialized(&self, _params: InitializedParams) {
        // Fetch merged config when the client supports workspace/configuration.
        let cfg_json = match self
            .client
            .configuration(vec![ConfigurationItem {
                scope_uri: None,
                section: Some("suspect".to_owned()),
            }])
            .await
        {
            Ok(values) => values.into_iter().next(),
            Err(_) => None,
        };
        // The shared schema: the workspace's `.suspect.yaml` is the base,
        // and the client's settings layer on top (an explicit editor
        // setting still wins — the same precedence the CLI enforces).
        let file_config = {
            let st = self.state.read().await;
            editor_config::for_workspace(st.workspace_root().as_deref(), cfg_json.as_ref())
        };
        let client_cfg = cfg_json.as_ref().and_then(config_files::parse_config);
        {
            let mut st = self.state.write().await;
            let init_opts = std::mem::take(&mut st.pending_init_options);
            st.config = config_files::merge(init_opts, client_cfg, Default::default());
            st.editor_config = file_config;
        }
        // Dynamic registration: only clients advertising
        // `didChangeWatchedFiles.dynamicRegistration` need the watcher
        // registered explicitly; others deliver the notification natively.
        let wants_dynamic_watcher = self
            .state
            .read()
            .await
            .client_caps
            .as_ref()
            .and_then(|c| c.workspace.as_ref())
            .and_then(|w| w.did_change_watched_files.as_ref())
            .and_then(|d| d.dynamic_registration)
            .unwrap_or(false);
        if wants_dynamic_watcher {
            let options = DidChangeWatchedFilesRegistrationOptions {
                watchers: vec![FileSystemWatcher {
                    glob_pattern: GlobPattern::String("**/*.{yaml,yml,json}".to_owned()),
                    kind: None,
                }],
            };
            let registration = Registration {
                id: "suspect-watched-yaml".to_owned(),
                method: "workspace/didChangeWatchedFiles".to_owned(),
                register_options: serde_json::to_value(options).ok(),
            };
            if let Err(err) = self.client.register_capability(vec![registration]).await {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("watcher registration failed: {err}"),
                    )
                    .await;
            }
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let parsed = config_files::parse_config(&params.settings);
        // The editor-side settings are re-derived too: `lint.min_severity`
        // lives there, not in `SuspectConfig`, and this handler used to
        // parse it into one object while the severity floor read the other
        // — so changing the floor mid-session did nothing until a restart.
        let file_config = {
            let st = self.state.read().await;
            editor_config::for_workspace(st.workspace_root().as_deref(), Some(&params.settings))
        };
        let changed = {
            let mut st = self.state.write().await;
            let config_changed = match parsed {
                Some(c) if st.config != c => {
                    st.config = c;
                    true
                }
                Some(_) | None => false,
            };
            let editor_changed = st.editor_config != file_config;
            if editor_changed {
                st.editor_config = file_config;
            }
            config_changed || editor_changed
        };
        if !changed {
            return;
        }
        // The lint floor moved, so anything computed under the old one is
        // no longer the answer — including the per-document pull cache,
        // whose entries are pre-filtered by the floor.
        {
            let mut st = self.state.write().await;
            st.ws_diag_cache = None;
            st.diag_cache.clear();
        }
        // The new config re-filters lint findings and inlay tooltips:
        // republish diagnostics for every open document and ask the client
        // to re-pull hints. Semantic tokens and lenses are config-free.
        let uris: Vec<Uri> = {
            let st = self.state.read().await;
            st.docs.keys().cloned().collect()
        };
        // Pull-diagnostics results re-filter under the new config too.
        let _ = self.client.workspace_diagnostic_refresh().await;
        for uri in uris {
            self.schedule_diagnostics(uri);
        }
        if self.client.inlay_hint_refresh().await.is_err() {
            self.client
                .log_message(MessageType::WARNING, "inlayHint refresh rejected")
                .await;
        }
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        let mut st = self.state.write().await;
        for added in &params.event.added {
            if let Ok(p) = added.uri.to_file_path() {
                st.root = Some(p);
            }
        }
        if let Some(last) = params.event.removed.last()
            && st
                .root
                .as_ref()
                .zip(last.uri.to_file_path().ok())
                .is_some_and(|(r, p)| r == &p)
        {
            st.root = None;
            st.workspace = None;
        }
    }

    async fn will_create_files(
        &self,
        params: CreateFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let ws = self.any_workspace().await;
        let Some(ws) = ws else { return Ok(None) };
        Ok(config_files::will_create_files(&ws, &params))
    }

    async fn did_create_files(&self, params: CreateFilesParams) {
        self.drop_workspace_cache().await;
        for f in &params.files {
            let path = match std::path::PathBuf::from(f.uri.as_str()).canonicalize() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let Ok(uri) = Uri::from_path(&path) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            self.state.write().await.open_doc(uri, text);
        }
    }

    async fn will_rename_files(
        &self,
        params: RenameFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let ws = self.any_workspace().await;
        let Some(ws) = ws else { return Ok(None) };
        let renames: Vec<FileRename> = params
            .files
            .iter()
            .map(|f| FileRename {
                old_uri: f.old_uri.to_string(),
                new_uri: f.new_uri.to_string(),
            })
            .collect();
        Ok(config_files::will_rename_files(&ws, &renames))
    }

    async fn did_rename_files(&self, params: RenameFilesParams) {
        self.drop_workspace_cache().await;
        let mut st = self.state.write().await;
        for f in &params.files {
            let old_path = std::path::PathBuf::from(f.old_uri.as_str());
            if let Ok(old) = Uri::from_path(&old_path) {
                st.close_doc(&old);
                st.token_cache.remove(&old);
                st.diag_cache.remove(&old);
                st.doc_cache.remove(&old);
            }
            let new_path = std::path::PathBuf::from(f.new_uri.as_str());
            if let Ok(text) = std::fs::read_to_string(&new_path)
                && let Ok(nu) = Uri::from_path(&new_path)
            {
                st.open_doc(nu, text);
            }
        }
    }

    async fn will_delete_files(
        &self,
        params: DeleteFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let ws = self.any_workspace().await;
        let Some(ws) = ws else { return Ok(None) };
        match config_files::will_delete_files(&ws, &params) {
            Some(diags) => {
                for d in diags.iter().take(1) {
                    self.client
                        .log_message(MessageType::WARNING, d.message.clone())
                        .await;
                }
                Ok(Some(WorkspaceEdit::default()))
            }
            None => Ok(None),
        }
    }

    async fn did_delete_files(&self, params: DeleteFilesParams) {
        self.drop_workspace_cache().await;
        let mut st = self.state.write().await;
        for f in &params.files {
            if let Ok(u) = Uri::from_path(std::path::Path::new(f.uri.as_str())) {
                st.close_doc(&u);
                st.token_cache.remove(&u);
                st.diag_cache.remove(&u);
                st.doc_cache.remove(&u);
            }
        }
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> JsonRpcResult<Option<Value>> {
        Ok(self.run_command(params).await)
    }

    async fn shutdown(&self) -> JsonRpcResult<()> {
        Ok(())
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> JsonRpcResult<Option<CompletionResponse>> {
        let pos = params.text_document_position.position;
        let Ok(uri) = Uri::parse(params.text_document_position.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        // The document is cloned out and the guard released before the
        // match below: the `Refs` arm needs the semantic index, and taking
        // the write lock that an index build requires while this read guard
        // is alive is the exact inversion that used to wedge the server.
        let doc = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            doc.clone()
        };
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        // suspect's own configuration files complete from their schema:
        // the keys still missing from this section, or the permitted values
        // of the key under the cursor.
        if let Some(kind) = config_kind(&uri) {
            let mut items = config_schema::completions(kind, &doc.low, offset);
            items.sort_by(|a, b| {
                a.sort_text
                    .cmp(&b.sort_text)
                    .then_with(|| a.label.cmp(&b.label))
            });
            return Ok(Some(CompletionResponse::Array(items)));
        }
        let context = completion::context_at(&doc.low, offset);
        // The schema's own permitted values lead whenever the position has
        // a schema: an enum member here is worth more than any keyword.
        let mut schema_first: Vec<CompletionItem> = Vec::new();
        if matches!(
            context,
            completion::CompletionContext::Values(_)
                | completion::CompletionContext::SchemaPropertyNames(_)
        ) {
            for value in rank::schema_values(&doc.low, offset) {
                schema_first.push(CompletionItem {
                    label: value.clone(),
                    detail: Some(rank::Rank::Exact.label().to_owned()),
                    kind: Some(tower_lsp::lsp_types::CompletionItemKind::VALUE),
                    sort_text: Some(format!("0{value}")),
                    ..CompletionItem::default()
                });
            }
        }
        let items = match context {
            completion::CompletionContext::Keys(keys) => completion::key_items(keys),
            completion::CompletionContext::Refs => match ws {
                Some(ws) => {
                    // Rank by what the workspace actually uses, so the
                    // components a maintainer reaches for come first.
                    let all = completion::ref_candidates(&ws, doc.low.uri());
                    let position = rank::position_at(&doc.low, offset);
                    // The cached index, not a rebuild: this is the
                    // per-keystroke completion path, and the build walks
                    // the whole workspace.
                    let index = self.index(&ws).await;
                    let expected: &[&str] = if rank::expects_schema(&doc.low, offset) {
                        &["schemas"]
                    } else {
                        &[]
                    };
                    rank::rank_refs(&index, doc.low.uri().as_str(), &position, expected, all)
                        .into_iter()
                        .map(|proposal| {
                            let sort_text =
                                format!("{}{}", rank::rank_level(proposal.rank), proposal.insert);
                            CompletionItem {
                                label: proposal.insert,
                                detail: Some(proposal.detail),
                                kind: Some(tower_lsp::lsp_types::CompletionItemKind::REFERENCE),
                                sort_text: Some(sort_text),
                                ..CompletionItem::default()
                            }
                        })
                        .collect()
                }
                None => Vec::new(),
            },
            completion::CompletionContext::Values(values) => completion::value_items(values),
            completion::CompletionContext::ComponentNames(section) => {
                completion::component_name_items(
                    completion::component_names(&doc.low, section),
                    section,
                    doc.low.uri(),
                )
            }
            completion::CompletionContext::OperationIds => {
                completion::operation_id_items(completion::operation_id_candidates(&doc.low))
            }
            completion::CompletionContext::TagNames => {
                completion::tag_name_items(completion::tag_name_candidates(&doc.low))
            }
            completion::CompletionContext::SchemaPropertyNames(names) => {
                // Required-and-missing siblings first, then the rest, each
                // labelled with why it is offered.
                let written: Vec<String> = names.clone();
                let ranked = rank::rank_siblings(&doc.low, offset, &written);
                if ranked.is_empty() {
                    completion::property_name_items(names)
                } else {
                    let mut items: Vec<CompletionItem> = ranked
                        .into_iter()
                        .map(|proposal| {
                            let sort_text =
                                format!("{}{}", rank::rank_level(proposal.rank), proposal.insert);
                            CompletionItem {
                                label: proposal.insert,
                                detail: Some(proposal.detail),
                                kind: Some(tower_lsp::lsp_types::CompletionItemKind::PROPERTY),
                                sort_text: Some(sort_text),
                                ..CompletionItem::default()
                            }
                        })
                        .collect();
                    items.extend(completion::property_name_items(written));
                    items
                }
            }
            completion::CompletionContext::MediaTypes => {
                completion::value_items(completion::MEDIA_TYPES)
            }
            completion::CompletionContext::None => return Ok(None),
        };
        let mut items = items;
        items.extend(schema_first);
        Ok((!items.is_empty()).then_some(CompletionResponse::Array(items)))
    }

    async fn completion_resolve(&self, item: CompletionItem) -> JsonRpcResult<CompletionItem> {
        let ws = match item
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
            .and_then(|u| Uri::parse(u).ok())
        {
            Some(uri) => self.workspace_for(&uri).await,
            None => None,
        };
        Ok(completion::resolve_item(item, ws.as_deref()))
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonRpcResult<Option<CodeActionResponse>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // Resolve the workspace first: `workspace_for` may take the state
        // write lock, so it must not run under this handler's read guard.
        let ws = self.workspace_for(&uri).await;
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let Some(url) = to_url(&uri) else {
            return Ok(None);
        };
        // Defer the whole-document fixAll sweep to `codeAction/resolve`
        // only when the client declared `codeAction.resolveSupport`.
        let defer_fix_all = st
            .client_caps
            .as_ref()
            .and_then(|c| c.text_document.as_ref())
            .and_then(|td| td.code_action.as_ref())
            .and_then(|ca| ca.resolve_support.as_ref())
            .is_some();
        let mut actions = actions::code_actions(
            doc,
            &url,
            params.range,
            &params.context.diagnostics,
            ws.as_ref(),
            defer_fix_all,
        );
        if let Some(open) = ws
            .as_deref()
            .and_then(|ws| links::open_ref_action(ws, doc, params.range))
        {
            actions.push(open);
        }
        Ok((!actions.is_empty()).then(|| {
            actions
                .into_iter()
                .map(CodeActionOrCommand::CodeAction)
                .collect()
        }))
    }

    async fn code_action_resolve(&self, action: CodeAction) -> JsonRpcResult<CodeAction> {
        let is_fix_all = action
            .data
            .as_ref()
            .and_then(|d| d.get("suspect"))
            .and_then(|s| s.as_str())
            == Some("fixAll");
        if !is_fix_all {
            return Ok(action);
        }
        let Some(uri_str) = action
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
        else {
            return Ok(action);
        };
        let Ok(uri) = Uri::parse(uri_str) else {
            return Ok(action);
        };
        let ws = self.workspace_for(&uri).await;
        let cfg = self.state.read().await.config.clone();
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(action);
        };
        let Some(url) = to_url(&uri) else {
            return Ok(action);
        };
        match actions::resolve_code_action(doc, &url, ws.as_ref(), &cfg) {
            Some(resolved) => Ok(resolved),
            None => Ok(action),
        }
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        // Formatting options are ignored: the canonical form uses a
        // two-space indent regardless of editor configuration. Key
        // reordering follows `suspect.formatting.sortKeys`.
        let _ = &params.options;
        let Some(url) = to_url(&uri) else {
            return Ok(None);
        };
        let sort_keys = st.config.format_sort_keys();
        let extensions = st.config.extensions.clone().unwrap_or_default();
        Ok(actions::format_document_full(doc, &url, sort_keys, &extensions).map(|e| vec![e]))
    }
}

impl Backend {
    /// Shared plumbing for goto-style requests: resolve doc + offset, run `f`.
    async fn with_doc_offset<T>(
        &self,
        tdp: TextDocumentPositionParams,
        f: impl FnOnce(&suspect_ref::Workspace, &suspect_low::LowDoc, usize) -> JsonRpcResult<Option<T>>,
    ) -> JsonRpcResult<Option<T>> {
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let off = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        );
        let Some(off) = off else { return Ok(None) };
        f(&ws, &doc.low, off)
    }

    /// Goto-style helper returning a definition response.
    async fn goto_like(
        &self,
        tdp: TextDocumentPositionParams,
        f: impl FnOnce(
            &suspect_ref::Workspace,
            &suspect_low::LowDoc,
            usize,
        ) -> Option<GotoDefinitionResponse>,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(off) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        ) else {
            return Ok(None);
        };
        Ok(f(&ws, &doc.low, off))
    }

    /// Any usable ref workspace: cached one, else rebuilt from the root.
    async fn any_workspace(&self) -> Option<Arc<suspect_ref::Workspace>> {
        {
            let st = self.state.read().await;
            if let Some(ws) = &st.workspace {
                return Some(ws.clone());
            }
        }
        let root = { self.state.read().await.root.clone() }?;
        let ws = suspect_ref::WorkspaceBuilder::new()
            .root(&root)
            .build()
            .ok()?;
        ws.load_all("main.yaml").ok()?;
        Some(Arc::new(ws))
    }

    /// Invalidates the cached workspace so the next request rebuilds it.
    async fn drop_workspace_cache(&self) {
        self.state.write().await.workspace = None;
    }

    /// Executes a `suspect.*` command; returns a JSON summary for the client.
    async fn run_command(&self, params: ExecuteCommandParams) -> Option<Value> {
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
    async fn progress_begin(&self, token: &str, title: &str) {
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
    async fn progress_end(&self, token: &str, message: Option<String>) {
        self.client
            .send_notification::<tower_lsp::lsp_types::notification::Progress>(ProgressParams {
                token: NumberOrString::String(token.to_owned()),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(WorkDoneProgressEnd {
                    message,
                })),
            })
            .await;
    }
    /// Handler for the `suspect/runWorkflow` custom request.
    async fn run_workflow_request(
        &self,
        params: run_lenses::RunWorkflowParams,
    ) -> JsonRpcResult<run_lenses::RunResult> {
        self.run_workflow_uri(&params.uri, &params.workflow).await
    }

    /// Handler for the `suspect/generationContract` custom request: the
    /// generation admission verdict for the live document.
    async fn generation_contract_request(
        &self,
        params: generation_contract::GenerationContractParams,
    ) -> JsonRpcResult<generation_contract::GenerationContractResult> {
        let Ok(uri) = Uri::parse(&params.uri) else {
            return Ok(GenerationContractResult {
                operations: Vec::new(),
                findings: Vec::new(),
                admissible: false,
                refusals: 0,
            });
        };
        let ws = self.workspace_for(&uri).await;
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(GenerationContractResult {
                operations: Vec::new(),
                findings: Vec::new(),
                admissible: false,
                refusals: 0,
            });
        };
        let Some(ws) = ws else {
            return Ok(GenerationContractResult {
                operations: Vec::new(),
                findings: Vec::new(),
                admissible: false,
                refusals: 0,
            });
        };
        Ok(
            generation_contract::generation_contract(&ws, doc).unwrap_or(
                GenerationContractResult {
                    operations: Vec::new(),
                    findings: Vec::new(),
                    admissible: false,
                    refusals: 0,
                },
            ),
        )
    }

    /// Compiles the Arazzo document at `uri_s`, executes the workflow named
    /// `workflow` against the configured base URL, streams progress/logs to
    /// the client, and publishes failure diagnostics on the document
    /// (empty diagnostics when everything passes).
    async fn run_workflow_uri(
        &self,
        uri_s: &str,
        workflow: &str,
    ) -> JsonRpcResult<run_lenses::RunResult> {
        use tower_lsp::jsonrpc::Error as RpcError;

        let invalid = |msg: String| RpcError {
            code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
            message: msg.into(),
            data: None,
        };
        let Ok(uri) = Uri::parse(uri_s) else {
            return Err(invalid(format!("unparseable uri {uri_s:?}")));
        };

        // Prefer the live buffer; fall back to the on-disk copy so runs work
        // for closed documents too. The Arc keeps the parsed doc alive across
        // awaits even if the editor closes it mid-run.
        let open = self.state.read().await.docs.get(&uri).cloned();
        let low_owned;
        let low = match &open {
            Some(doc) => &doc.low,
            None => {
                let path = uri.as_path().ok_or_else(|| {
                    invalid(format!(
                        "document {uri_s:?} is neither open nor a local file"
                    ))
                })?;
                let bytes = std::fs::read(&path)
                    .map_err(|e| invalid(format!("cannot read {}: {e}", path.display())))?;
                low_owned = suspect_low::LowDoc::parse(
                    uri.clone(),
                    suspect_source::Source::from_vec(bytes),
                );
                &low_owned
            }
        };

        // Arazzo source descriptions reference sibling files without `$ref`,
        // so a directory scan is the reliable way to load the workspace.
        let spec_path = uri
            .as_path()
            .ok_or_else(|| invalid("non-file uri".into()))?;
        let ws = run_lenses::workspace_dir_all(&spec_path)
            .ok_or_else(|| invalid("failed to load spec directory".into()))?;
        let plan = suspect_test::compile_plan(low, &ws).map_err(|e| invalid(e.to_string()))?;
        let wf = plan
            .workflows
            .iter()
            .find(|w| w.workflow_id == workflow)
            .ok_or_else(|| invalid(format!("workflow {workflow:?} not found in {uri_s:?}")))?;

        #[cfg(not(feature = "live-run"))]
        {
            let _ = wf;
            return Err(RpcError {
                code: tower_lsp::jsonrpc::ErrorCode::InternalError,
                message: "suspect-lsp was built without the live-run feature".into(),
                data: None,
            });
        }
        #[cfg(feature = "live-run")]
        {
            let init_options = self.state.read().await.pending_init_options.clone();
            let base = base_url(init_options.as_ref());
            let transport = run_lenses::ReqwestTransport::new();

            self.progress_begin(
                "suspect.runWorkflow",
                &format!("Running workflow '{workflow}'"),
            )
            .await;

            // Forward step events as window logs while the run progresses.
            let (mirror_tx, mut mirror_rx) =
                tokio::sync::mpsc::channel::<suspect_test::TestEvent>(256);
            let logger_client = self.client.clone();
            let logger = tokio::spawn(async move {
                while let Some(ev) = mirror_rx.recv().await {
                    logger_client
                        .log_message(MessageType::INFO, run_lenses::describe_event(&ev))
                        .await;
                }
            });

            let (summary, failures) =
                run_lenses::run_workflow_core(wf, &base, &transport, Some(&mirror_tx)).await;
            drop(mirror_tx);
            let _ = logger.await;

            // Publish criterion failures anchored at their source ranges;
            // an all-pass run clears previous test diagnostics instead.
            if let Ok(url) = Url::parse(uri.as_str()) {
                let inner = low.inner();
                let diags = run_lenses::failures_to_diagnostics(
                    wf,
                    &failures,
                    inner.bytes(),
                    inner.line_index(),
                );
                self.client.publish_diagnostics(url, diags, None).await;
            }

            self.progress_end(
                "suspect.runWorkflow",
                Some(format!(
                    "{passed} passed, {failed} failed",
                    passed = summary.passed,
                    failed = summary.failed
                )),
            )
            .await;
            Ok(summary)
        }
    }

    /// Renders `preset` for the OpenAPI spec at `uri_s` under
    /// `<workspace-root>/.suspect/preview/<preset>/` and opens the first
    /// written artifact.
    async fn render_preview(&self, uri_s: &str, preset: &str) -> Option<Value> {
        let uri = Uri::parse(uri_s).ok()?;
        let spec_path = uri.as_path()?;
        let ws = run_lenses::workspace_dir_all(&spec_path)?;
        let ir = suspect_ir::IrSpec::from_workspace(&ws, &uri).ok()?;

        let root = self
            .state
            .read()
            .await
            .root
            .clone()
            .unwrap_or_else(|| run_lenses::dir_of(&spec_path));
        let out_root = root.join(".suspect").join("preview").join(preset);
        let outcomes = match run_lenses::render_preset(preset, &ir, &out_root) {
            Ok(outcomes) => outcomes,
            Err(e) => {
                self.client.log_message(MessageType::ERROR, e).await;
                return None;
            }
        };
        let opened = run_lenses::pick_outcome(&outcomes)?;
        let url = Url::from_file_path(opened).ok()?;
        let shown = self
            .client
            .show_document(ShowDocumentParams {
                uri: url,
                external: None,
                take_focus: Some(true),
                selection: None,
            })
            .await
            .ok()?;
        Some(serde_json::json!({
            "rendered": outcomes.len(),
            "opened": opened.display().to_string(),
            "shown": shown,
        }))
    }
}

/// Base URL for live runs: initialization options, then `SUSPECT_BASE_URL`,
/// then the default local address.
#[cfg(feature = "live-run")]
fn base_url(init_options: Option<&serde_json::Value>) -> String {
    run_lenses::base_url_from_options(init_options)
}

/// Runs the language server over stdio until the client disconnects.
///
/// Builds an [`LspService`] wrapping the private `Backend` server
/// implementation and serves the LSP loop on stdin/stdout.
/// Never returns an error: transport failures are handled
/// internally by tower-lsp, and the future completes only when the
/// connection closes. Await from within a tokio runtime (see the
/// `suspect-lsp` binary's `main`).
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = service().await;
    Server::new(stdin, stdout, socket)
        .concurrency_level(CONCURRENCY)
        .serve(service)
        .await;
}

/// How many requests may be in flight at once.
///
/// tower-lsp defaults to four. That is fine while every request is cheap and
/// wrong for this server: one pull-diagnostics request on a 63k-line
/// specification occupies a slot for seconds doing synchronous work, and the
/// other three slots cannot cover the dozen requests an editor sends in the
/// same moment — so a hover, which costs 52ms on its own, waited four
/// seconds for its turn. Measured, not assumed: the same burst went from
/// 4.2s to well under a second at this level.
///
/// Bounded rather than unbounded: the expensive requests are CPU-bound, so
/// letting dozens of them run at once would trade one stall for thrashing.
/// The diagnostics cache means a burst now computes the lint pass once.
const CONCURRENCY: usize = 16;

/// Builds the server plus its client socket.
///
/// Split out of [`run_server`] so a test can drive the real server — the
/// one place where lock-ordering between request handlers is observable
/// at all.
async fn service() -> (LspService<Backend>, tower_lsp::ClientSocket) {
    LspService::build(Backend::new)
        .custom_method(
            <run_lenses::RunWorkflowRequest as tower_lsp::lsp_types::request::Request>::METHOD,
            Backend::run_workflow_request,
        )
        .custom_method(
            <generation_contract::GenerationContractRequest as tower_lsp::lsp_types::request::Request>::METHOD,
            Backend::generation_contract_request,
        )
        .finish()
}

/// Builds a full pull-diagnostics report.
fn full_report(
    result_id: Option<String>,
    items: Vec<Diagnostic>,
) -> DocumentDiagnosticReportResult {
    DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(
        RelatedFullDocumentDiagnosticReport {
            related_documents: None,
            full_document_diagnostic_report: FullDocumentDiagnosticReport { result_id, items },
        },
    ))
}

/// Builds an unchanged pull-diagnostics report for `result_id`.
fn unchanged_report(result_id: String) -> DocumentDiagnosticReportResult {
    DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Unchanged(
        RelatedUnchangedDocumentDiagnosticReport {
            related_documents: None,
            unchanged_document_diagnostic_report: UnchangedDocumentDiagnosticReport { result_id },
        },
    ))
}

/// Encodes classified tokens into the LSP relative-encoding envelope.
fn encode_tokens(tokens: Vec<SemanticToken>) -> Vec<SemanticToken> {
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE on the optional Backend plumbing test (initialize + didOpen +
    // diagnostics poll through tower-lsp's request/channel plumbing): it is
    // deliberately skipped. Driving `LspService` requires the
    // `tower::Service` trait and polling the client socket requires a
    // `Stream` extension trait; neither `tower` nor `futures` is a workspace
    // dependency, and tower-lsp does not re-export them. The behavior is
    // instead covered end-to-end by the pure-function unit tests in
    // state/diagnostics/navigation/completion/symbols, plus this test of the
    // debounce generation logic shape.

    #[test]
    fn debounce_generation_is_monotonic_per_document() {
        let mut st = State::default();
        let a: Uri = "file:///a.yaml".into();
        let b: Uri = "file:///b.yaml".into();

        let cap_a = *st.generations.entry(a.clone()).or_insert(0);
        *st.generations.get_mut(&a).unwrap() += 1;
        // Editing document B must not invalidate A's pending publish.
        assert_eq!(st.generations.get(&b), None);
        assert_ne!(st.generations.get(&a), Some(&cap_a));

        // B's first generation starts at its own counter.
        assert_eq!(*st.generations.entry(b.clone()).or_insert(0), 0);
    }
}

// ---------- free plumbing helpers used by Backend handlers ----------

#[cfg(test)]
mod hover_latency_tests {
    use super::*;

    /// A spec with a realistic reference count: enough that walking every
    /// `$ref` is measurable.
    fn spec_with_refs(operations: usize) -> String {
        let mut out = String::from("openapi: 3.1.1\ninfo: {title: Hover, version: '1'}\npaths:\n");
        for index in 0..operations {
            out.push_str(&format!(
                "  /resource{index}:\n    get:\n      operationId: op{index}\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema: {{$ref: '#/components/schemas/Model'}}\n"
            ));
        }
        out.push_str(
            "components:\n  schemas:\n    Model:\n      type: object\n      required: [id]\n      properties:\n        id: {type: string}\n",
        );
        out
    }

    /// Hover must not cost a workspace walk and a contract compile per
    /// request.
    ///
    /// Three bugs met here, all found by driving the real server against a
    /// real specification: the read guard was held across the computation
    /// (so concurrent hovers queued behind the diagnostics writer), the
    /// reference index was rebuilt per hover, and the contract was
    /// recompiled per hover. Together they made a 63k-line document take
    /// the better part of a second per cursor move, which a client reports
    /// as a hover that never resolves.
    #[test]
    fn hover_answers_within_a_budget_and_reuses_its_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        let operations = 400;
        std::fs::write(dir.path().join("openapi.yaml"), spec_with_refs(operations)).expect("write");
        let ws = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(dir.path())
                .build()
                .expect("workspace"),
        );
        ws.load_all("openapi.yaml").expect("load");
        let low = ws.get(&ws.uris()[0]).expect("handle").doc();

        // Stand in for the server's cached index: built once per generation.
        let index = std::sync::Arc::new(meaning::Index::build(&ws));
        let text = spec_with_refs(operations);
        let line = text
            .lines()
            .position(|l| l.contains("$ref"))
            .expect("a ref line");
        let byte = text.lines().take(line).map(str::len).sum::<usize>()
            + text
                .lines()
                .nth(line)
                .expect("line")
                .find("$ref")
                .expect("ref")
            + 2;

        let started = std::time::Instant::now();
        let first = hover_meaning(low, byte, &index);
        let first_cost = started.elapsed();

        let started = std::time::Instant::now();
        let second = hover_meaning(low, byte, &index);
        let second_cost = started.elapsed();

        assert!(first.is_some(), "hover must say something: {first:?}");
        assert!(second.is_some());

        // Generous, but it must fail if a per-request workspace walk or a
        // contract compile returns: both are seconds on a real document.
        let budget = std::time::Duration::from_millis(250);
        assert!(
            first_cost < budget,
            "first hover took {first_cost:?} over a {operations}-operation spec"
        );
        assert!(
            second_cost < budget / 4,
            "repeated hover took {second_cost:?}; the index is not being reused"
        );
    }

    /// The cache must survive an unrelated read and die with the tree it
    /// described.
    ///
    /// Warming on open only pays off if the answer survives until the user
    /// asks, and only stays correct if it goes when the workspace does.
    #[test]
    fn the_index_cache_is_reused_until_the_documents_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("openapi.yaml"), spec_with_refs(4)).expect("write");
        let ws = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(dir.path())
                .build()
                .expect("workspace"),
        );
        ws.load_all("openapi.yaml").expect("load");

        let mut state = State::default();
        let generation = state.generation();
        let first = Arc::new(crate::meaning::Index::build(&ws));
        state.store_index(generation, first.clone());
        assert!(
            state
                .cached_index(generation)
                .is_some_and(|again| Arc::ptr_eq(&first, &again)),
            "cache not reused"
        );

        state.open_doc(
            Uri::parse("file:///tmp/warm.yaml").expect("uri"),
            "openapi: 3.1.1\n".to_owned(),
        );
        let after_edit = state.generation();
        assert!(
            state.cached_index(after_edit).is_none(),
            "an edit must invalidate the cached index"
        );

        state.drop_workspace();
        assert!(
            state.index_cache.is_none(),
            "a dropped workspace must take its index with it"
        );
    }
}

#[cfg(test)]
mod burst_tests {
    use super::*;
    use tower::Service;

    /// One request, with the timeout that turns a wedge into a failure
    /// instead of a hung test run.
    /// Hover repeatedly until it answers, or `budget` seconds run out.
    /// A wedged server never answers no matter how long you wait; a slow
    /// one answers on a later try. One sample cannot tell those apart.
    async fn hover_within(
        service: &mut LspService<Backend>,
        td: &serde_json::Value,
        at: &serde_json::Value,
        budget: f64,
    ) -> Option<tower_lsp::jsonrpc::Response> {
        let deadline = std::time::Instant::now() + Duration::from_secs_f64(budget);
        // Ids come from a process-wide counter and never repeat. tower-lsp
        // registers every request id when it arrives and only unregisters
        // it when the handler completes; abandoning a request by timeout
        // leaves the id registered, and a later request that reuses it is
        // answered `InvalidRequest`. A per-call counter was not enough —
        // each hover_within call restarted it, so a leaked id from one
        // notification's hover collided with the next one's first attempt.
        // Reproduced deterministically by shrinking the per-attempt timeout
        // (HOVER_ATTEMPT_MS) until attempts get abandoned.
        static NEXT_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(20_000);
        // Each abandoned attempt doubles the next one's budget, so a slow
        // first attempt cannot starve: given enough retries, one attempt is
        // always given more time than the request needs. Abandoning also
        // cancels — a dropped request leaves its id registered in the
        // server's cancellation map, and cancelling is both the correct
        // protocol move and what keeps that map clean.
        let mut attempt_ms = std::env::var("HOVER_ATTEMPT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(250u64);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let answer = in_flight(
                service,
                id,
                "textDocument/hover",
                serde_json::json!({"textDocument": td, "position": at}),
            );
            let budget = left.min(Duration::from_millis(attempt_ms));
            match tokio::time::timeout(budget, answer).await {
                Ok(Some(response)) => return Some(response),
                _ => {
                    let _ = in_flight(service, 0, "$/cancelRequest", serde_json::json!({"id": id}))
                        .await;
                    attempt_ms = attempt_ms.saturating_mul(2).min(left.as_millis() as u64);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    fn in_flight(
        service: &mut LspService<Backend>,
        id: i64,
        method: &str,
        params: serde_json::Value,
    ) -> Answer {
        // Each request needs its own id: the server tracks in-flight ids
        // for cancellation, and a duplicate is rejected as invalid. An id
        // of zero means a notification — and a notification method sent
        // *with* an id is silently dropped by tower-lsp, so this is not
        // optional.
        let mut builder = tower_lsp::jsonrpc::Request::build(method.to_owned()).params(params);
        if id != 0 {
            builder = builder.id(id);
        }
        let request = builder.finish();
        // `call` hands back a `'static` future, so the borrow of the
        // service ends here and the request stays in flight while the
        // next one is issued.
        let answer = service.call(request);
        Box::pin(async move {
            tokio::time::timeout(std::time::Duration::from_secs(20), answer)
                .await
                .ok()
                .and_then(|result| result.ok())
                .flatten()
        })
    }

    /// One answer, boxed so a batch of in-flight requests borrows nothing.
    type Answer = std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<tower_lsp::jsonrpc::Response>> + Send>,
    >;

    /// A specification with enough references that the parts of the server
    /// which walk the workspace have something to walk.
    fn spec_with_refs(operations: usize) -> String {
        let mut out = String::from("openapi: 3.1.1\ninfo: {title: Burst, version: '1'}\npaths:\n");
        for index in 0..operations {
            out.push_str(&format!(
                "  /resource{index}:\n    get:\n      operationId: op{index}\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema: {{$ref: '#/components/schemas/Model'}}\n"
            ));
        }
        out.push_str(
            "components:\n  schemas:\n    Model:\n      type: object\n      properties:\n        id: {type: string}\n",
        );
        out
    }

    /// Everything an editor sends when it opens a YAML document, issued
    /// together the way an editor issues it.
    ///
    /// This is the test a request-by-request probe cannot be. Hover alone
    /// always answered in milliseconds; the server only ever wedged once
    /// an editor asked for something else first, and then *every* later
    /// request queued behind a lock that would never be released — which
    /// is what "loading forever" looked like from the editor's side. One
    /// handler (`document_link`) held a read guard while asking for a
    /// write lock, and because tokio's RwLock is write-preferring, its
    /// queued writer then blocked every later reader too.
    ///
    /// All requests are put in flight before any is awaited. Awaiting
    /// them one at a time would pass: the deadlock needs a second request
    /// waiting behind the first.
    ///
    /// `textDocument/diagnostic` is the one that has to be in this burst.
    /// It is the only handler that took a read guard across the lint pass
    /// *and then took a second read* for the severity floor. `didOpen`
    /// spawns `warm_index`, which queues a writer once its blocking build
    /// finishes; against that queued writer the second read blocks, and
    /// since the first guard cannot drop until the second read returns,
    /// the pair never resolves. An editor sends `didOpen` and
    /// `textDocument/diagnostic` in the same instant, so this is not a
    /// race that needs bad luck — it is every file open.
    #[tokio::test]
    async fn every_request_an_open_triggers_answers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let text = spec_with_refs(2000);
        std::fs::write(dir.path().join("openapi.yaml"), &text).expect("write");
        let path = dir.path().join("openapi.yaml");
        let uri = Uri::parse(&format!("file://{}", path.display())).expect("uri");

        let (mut service, socket) = service().await;
        let root = format!("file://{}", dir.path().display());

        // Answer the server's own requests, as an editor does. Without
        // this the server waits forever on `workspace/configuration`
        // during `initialized`, and a test that ignores that wait spends
        // its whole budget in it.
        let answering = tokio::spawn(async move {
            use futures::{SinkExt, StreamExt};
            let mut socket = socket;
            while let Some(request) = socket.next().await {
                let result = match request.method() {
                    "workspace/configuration" => {
                        let items = request
                            .params()
                            .and_then(|p| p.get("items"))
                            .and_then(|i| i.as_array())
                            .map_or(0, Vec::len);
                        serde_json::Value::Array(vec![serde_json::Value::Null; items])
                    }
                    "workspace/applyEdit" => serde_json::json!({"applied": true}),
                    _ => serde_json::Value::Null,
                };
                if let Some(id) = request.id().cloned() {
                    socket
                        .send(tower_lsp::jsonrpc::Response::from_ok(id, result))
                        .await
                        .ok();
                }
            }
        });

        assert!(
            in_flight(
                &mut service,
                1,
                "initialize",
                serde_json::json!({
                    "processId": null,
                    "rootUri": root,
                    "capabilities": {"workspace": {"configuration": true}},
                }),
            )
            .await
            .is_some(),
            "initialize answered"
        );
        // `initialized` is a notification: sent, and its (empty) answer read.
        let _ = in_flight(&mut service, 0, "initialized", serde_json::json!({})).await;
        let _ = in_flight(
            &mut service,
            0,
            "textDocument/didOpen",
            serde_json::json!({"textDocument": {
                "uri": uri.as_str(), "languageId": "yaml", "version": 1, "text": text,
            }}),
        )
        .await;
        // Let the warmed index and the first diagnostics finish, so this
        // measures concurrency rather than cold start.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Line 11 of the fixture is the `$ref` on the first operation's
        // 200 response; character 24 is inside the key, which must always
        // render the keyword entry. The fixture is small on purpose: what
        // this test reproduces is a lock-ordering bug, which has nothing to
        // do with how much document there is.
        let at = serde_json::json!({"line": 11, "character": 24});
        let td = serde_json::json!({"uri": uri.as_str()});
        let span = serde_json::json!({
            "start": {"line": 11, "character": 0},
            "end": {"line": 12, "character": 0},
        });
        let requests: Vec<(&str, serde_json::Value)> = vec![
            (
                "textDocument/hover",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/documentSymbol",
                serde_json::json!({"textDocument": td}),
            ),
            (
                "textDocument/foldingRange",
                serde_json::json!({"textDocument": td}),
            ),
            (
                "textDocument/semanticTokens/full",
                serde_json::json!({"textDocument": td}),
            ),
            (
                "textDocument/documentLink",
                serde_json::json!({"textDocument": td}),
            ),
            (
                "textDocument/inlayHint",
                serde_json::json!({"textDocument": td, "range": span}),
            ),
            (
                "textDocument/documentHighlight",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/selectionRange",
                serde_json::json!({"textDocument": td, "positions": [at]}),
            ),
            (
                "textDocument/codeLens",
                serde_json::json!({"textDocument": td}),
            ),
            (
                "textDocument/documentColor",
                serde_json::json!({"textDocument": td}),
            ),
            (
                "textDocument/codeAction",
                serde_json::json!({"textDocument": td, "range": span, "context": {"diagnostics": []}}),
            ),
            (
                "textDocument/completion",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/prepareCallHierarchy",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/prepareTypeHierarchy",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/moniker",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/linkedEditingRange",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/prepareRename",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/diagnostic",
                serde_json::json!({"textDocument": td}),
            ),
            ("workspace/diagnostic", serde_json::json!({})),
            (
                "textDocument/definition",
                serde_json::json!({"textDocument": td, "position": at}),
            ),
            (
                "textDocument/references",
                serde_json::json!({"textDocument": td, "position": at, "context": {"includeDeclaration": true}}),
            ),
        ];

        // Every request put in flight before any is awaited.
        let mut pending: Vec<(&str, Answer)> = Vec::new();
        for (n, (method, params)) in requests.iter().enumerate() {
            pending.push((
                *method,
                in_flight(&mut service, 100 + n as i64, method, params.clone()),
            ));
        }

        for (method, answer) in pending {
            let got = answer.await;
            assert!(
                got.is_some(),
                "{method} never answered: the server is wedged"
            );
        }

        // And the one an editor sends on every cursor move, afterwards.
        let after = in_flight(
            &mut service,
            900,
            "textDocument/hover",
            serde_json::json!({"textDocument": td, "position": at}),
        )
        .await
        .expect("hover answered after the burst");
        let value = after.result().expect("hover returned a result");
        assert!(
            !value.is_null(),
            "hover must still work once the burst is over"
        );

        // Every *notification* the client sends while you edit. A
        // notification carries no id, so it produces no reply to wait on —
        // which is exactly why a notification that wedges the server is
        // invisible until the next request also stops answering. `didSave`
        // is the one that matters: the server advertises `save`, so this
        // arrives on every save, and a hover afterwards must still work.
        for (method, params) in [
            (
                "textDocument/didSave",
                serde_json::json!({"textDocument": td}),
            ),
            (
                "workspace/didChangeWatchedFiles",
                serde_json::json!({"changes": [{"uri": td["uri"], "type": 2}]}),
            ),
            (
                "workspace/didChangeConfiguration",
                serde_json::json!({"settings": {}}),
            ),
        ] {
            let _ = in_flight(&mut service, 0, method, params.clone()).await;
            let after = hover_within(&mut service, &td, &at, 20.0)
                .await
                .unwrap_or_else(|| panic!("{method} wedged the server: hover never answered"));
            // An error response has no result, and the interesting question
            // is always which error — so print it rather than guessing.
            let Some(result) = after.result() else {
                panic!(
                    "{method} made hover answer with an error: {:?}",
                    after.error()
                );
            };
            assert!(
                !result.is_null(),
                "{method} wedged the server: hover came back empty"
            );
        }

        // And the command that takes the write lock while a document
        // borrow is live — the same shape, on the other handler.
        in_flight(
            &mut service,
            920,
            "workspace/executeCommand",
            serde_json::json!({
                "command": "suspect.changeImpact",
                "arguments": [td["uri"], 11, 24],
            }),
        )
        .await
        .expect("changeImpact answered");
        let after = hover_within(&mut service, &td, &at, 20.0)
            .await
            .expect("changeImpact must not wedge the server");
        assert!(
            !after.result().expect("hover returned a result").is_null(),
            "changeImpact must not wedge the server"
        );

        answering.abort();
    }

    /// A writer queued while `textDocument/diagnostic` is running must not
    /// be able to wedge it.
    ///
    /// This is the trigger for "hover spins forever" on a large document.
    /// `didOpen` spawns `warm_index`, which builds the index off the
    /// interaction path and then queues a `state.write()`. `diagnostic`
    /// used to hold a read guard across the whole lint pass and then take
    /// a *second* read for the severity floor. tokio's `RwLock` is
    /// write-preferring, so with that writer queued the second read waits
    /// for a writer that is itself waiting for the first guard to drop —
    /// and because the queued writer also blocks every later reader, one
    /// diagnostic wedges the entire server, hover included.
    ///
    /// Scope of this test, stated honestly: it pins the request ordering
    /// that triggers the bug (`didOpen` and `textDocument/diagnostic`
    /// together, which is what an editor sends on every open) and it
    /// requires both to answer with a live hover afterwards. It does *not*
    /// reproduce the wedge on a synthetic fixture — `warm_index`'s build
    /// has to outlast the lint, which only happens at the scale of a real
    /// 63k-line specification. The deterministic reproduction of that is
    /// the JSON-RPC bisect against the real spec, which reported
    /// `textDocument/diagnostic WEDGED, hover afterwards: ALSO DEAD`
    /// before this fix and `answered` after it.
    #[tokio::test]
    async fn a_queued_writer_does_not_wedge_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let text = spec_with_refs(2000);
        std::fs::write(dir.path().join("openapi.yaml"), &text).expect("write");
        let path = dir.path().join("openapi.yaml");
        let uri = Uri::parse(&format!("file://{}", path.display())).expect("uri");
        let (mut service, socket) = service().await;
        let answering = tokio::spawn(async move {
            use futures::{SinkExt, StreamExt};
            let mut socket = socket;
            while let Some(request) = socket.next().await {
                let result = match request.method() {
                    "workspace/configuration" => {
                        let items = request
                            .params()
                            .and_then(|p| p.get("items"))
                            .and_then(|i| i.as_array())
                            .map_or(0, Vec::len);
                        serde_json::Value::Array(vec![serde_json::Value::Null; items])
                    }
                    _ => serde_json::Value::Null,
                };
                if let Some(id) = request.id().cloned() {
                    socket
                        .send(tower_lsp::jsonrpc::Response::from_ok(id, result))
                        .await
                        .ok();
                }
            }
        });

        in_flight(
            &mut service,
            1,
            "initialize",
            serde_json::json!({
                "processId": null,
                "rootUri": format!("file://{}", dir.path().display()),
                "capabilities": {"workspace": {"configuration": true}},
            }),
        )
        .await
        .expect("initialize answered");
        let _ = in_flight(&mut service, 0, "initialized", serde_json::json!({})).await;

        let td = serde_json::json!({"uri": uri.as_str()});

        // The editor sends `didOpen` and `textDocument/diagnostic` in the
        // same instant, and the order matters: `didOpen` spawns the tasks
        // that queue a writer, so if the diagnostic is already running
        // when they arrive it is holding a read guard across the whole
        // lint. Awaiting `didOpen` first would let those writers finish
        // first and hide the bug, which is exactly what a request-at-a-
        // time probe does.
        let open = in_flight(
            &mut service,
            0,
            "textDocument/didOpen",
            serde_json::json!({"textDocument": {
                "uri": uri.as_str(), "languageId": "yaml", "version": 1, "text": text,
            }}),
        );
        let diagnostic = in_flight(
            &mut service,
            100,
            "textDocument/diagnostic",
            serde_json::json!({"textDocument": td}),
        );
        let joined = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::join!(open, diagnostic)
        })
        .await
        .expect("textDocument/diagnostic never answered: a queued writer wedged it");
        let answer = joined.1.expect("the diagnostic returned no response");
        assert!(
            answer.result().is_some(),
            "textDocument/diagnostic returned no result"
        );
        let at = serde_json::json!({"line": 11, "character": 24});
        let hover = in_flight(
            &mut service,
            200,
            "textDocument/hover",
            serde_json::json!({"textDocument": td, "position": at}),
        )
        .await
        .expect("hover never answered: the server is still wedged");
        assert!(
            !hover.result().expect("hover result").is_null(),
            "hover came back empty after the diagnostic"
        );

        answering.abort();
    }

    /// Every hover answer must be markdown-kind on the wire: VS Code
    /// renders `HoverContents::Markup(Markdown)` as markdown, and any
    /// plaintext path in any hover surface would show the card's own
    /// markdown as raw text — precisely the "not visually appealing"
    /// failure the card redesign set out to fix. This drives the real
    /// server across every hover surface: a keyword key, a description
    /// value, a `$ref` value, a component key, a plain key, and a
    /// configuration key.
    #[tokio::test]
    async fn every_hover_answer_is_markdown_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec = "\
openapi: 3.1.0
info:
  title: T
  version: '1'
  description: Rates a **media item**.
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      properties:
        name: {type: string}
";
        std::fs::write(dir.path().join("openapi.yaml"), spec).expect("write");
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "lint:\n  min_severity: warning\n",
        )
        .expect("write");
        let spec_uri = format!("file://{}", dir.path().join("openapi.yaml").display());
        let conf_uri = format!("file://{}", dir.path().join(".suspect.yaml").display());

        let (mut service, socket) = service().await;
        let answering = tokio::spawn(async move {
            use futures::{SinkExt, StreamExt};
            let mut socket = socket;
            while let Some(request) = socket.next().await {
                let result = match request.method() {
                    "workspace/configuration" => {
                        let items = request
                            .params()
                            .and_then(|p| p.get("items"))
                            .and_then(|i| i.as_array())
                            .map_or(0, Vec::len);
                        serde_json::Value::Array(vec![serde_json::Value::Null; items])
                    }
                    _ => serde_json::Value::Null,
                };
                if let Some(id) = request.id().cloned() {
                    socket
                        .send(tower_lsp::jsonrpc::Response::from_ok(id, result))
                        .await
                        .ok();
                }
            }
        });

        in_flight(
            &mut service,
            1,
            "initialize",
            serde_json::json!({
                "processId": null,
                "rootUri": format!("file://{}", dir.path().display()),
                "capabilities": {"workspace": {"configuration": true}},
            }),
        )
        .await
        .expect("initialize answered");
        let _ = in_flight(&mut service, 0, "initialized", serde_json::json!({})).await;
        let _ = in_flight(
            &mut service,
            0,
            "textDocument/didOpen",
            serde_json::json!({"textDocument": {
                "uri": spec_uri, "languageId": "yaml", "version": 1, "text": spec,
            }}),
        )
        .await;
        let _ = in_flight(
            &mut service,
            0,
            "textDocument/didOpen",
            serde_json::json!({"textDocument": {
                "uri": conf_uri, "languageId": "yaml", "version": 1,
                "text": "lint:\n  min_severity: warning\n",
            }}),
        )
        .await;
        // Let the opened documents settle (workspace build, first lint)
        // so this asserts the steady-state answer, not a race with it.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // One position per hover surface, in the fixture above.
        let hovers: Vec<(&str, (u32, u32), &str)> = vec![
            ("keyword key", (0, 4), &spec_uri),
            ("description value", (4, 20), &spec_uri),
            ("$ref key", (15, 18), &spec_uri),
            ("component key", (18, 5), &spec_uri),
            ("plain key", (21, 10), &spec_uri),
            ("configuration key", (1, 6), &conf_uri),
        ];
        for (n, (index, (line, character), uri)) in hovers.iter().enumerate() {
            let answer = in_flight(
                &mut service,
                300 + n as i64,
                "textDocument/hover",
                serde_json::json!({
                    "textDocument": {"uri": uri},
                    "position": {"line": line, "character": character},
                }),
            )
            .await
            .unwrap_or_else(|| panic!("{index}: hover never answered"));
            let result = answer.result().unwrap_or_else(|| {
                panic!(
                    "{index}: hover answered with an error: {:?}",
                    answer.error()
                )
            });
            let contents = &result["contents"];
            assert_eq!(
                contents["kind"], "markdown",
                "{index}: hover must be markdown-kind, got {contents}"
            );
            assert!(
                contents["value"].as_str().is_some_and(|v| !v.is_empty()),
                "{index}: hover markdown must be non-empty"
            );
        }

        // The $ref card pins the restyled frame on the wire: the heading
        // the component renderers open with is what the editor receives.
        let answer = in_flight(
            &mut service,
            400,
            "textDocument/hover",
            serde_json::json!({
                "textDocument": {"uri": spec_uri},
                "position": {"line": 15, "character": 18},
            }),
        )
        .await
        .expect("$ref hover answered");
        let value = answer.result().expect("hover result")["contents"]["value"]
            .as_str()
            .expect("markdown string");
        assert!(value.contains("### `Pet`"), "card frame missing: {value}");

        answering.abort();
    }
}
