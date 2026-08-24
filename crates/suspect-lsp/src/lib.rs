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
pub mod diagnostics;
pub mod hover_detail;
pub mod links;
pub mod navigation;
pub mod pull;
pub mod rename;
pub mod run_lenses;
pub mod semantic;
pub mod state;
pub mod symbols;
pub mod type_hierarchy_fmt;
pub mod workspace_symbol;

use std::sync::Arc;
use std::time::Duration;

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
            let diags = diagnostics::compute_diagnostics(ws.as_ref(), &doc.low, &cfg);
            // Superseded while computing? Drop the stale result.
            let superseded = {
                let st = state.read().await;
                st.generations.get(&uri) != Some(&generation)
            };
            if superseded {
                return;
            }
            client.publish_diagnostics(url, diags, None).await;
        });
    }

    /// Returns the workspace, building it on first use and making sure the
    /// given document plus its `$ref` closure is loaded. Best-effort.
    async fn workspace_for(&self, uri: &Uri) -> Option<Arc<suspect_ref::Workspace>> {
        let ws = {
            let mut st = self.state.write().await;
            st.ensure_workspace()?
        };
        if ws.get(uri).is_none()
            && let Some(path) = uri.as_path()
        {
            let _ = ws.load_all(&path.to_string_lossy());
        }
        Some(ws)
    }
}

/// Parses a [`Uri`] into an LSP [`Url`], or `None` when it does not parse.
fn to_url(uri: &Uri) -> Option<Url> {
    Url::parse(uri.as_str()).ok()
}

/// Converts a navigation target into an LSP `Location`, preferring the open
/// (possibly dirty) buffer when it is the same document, else the
/// workspace-loaded copy.
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
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        Ok(Some(SemanticTokensResult::Tokens(
            semantic::semantic_tokens_full(doc),
        )))
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
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(Vec::new());
        };
        Ok(colors::document_colors(doc))
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
            st.workspace = None;
        }
    }

    async fn did_change_watched_files(&self, _params: DidChangeWatchedFilesParams) {
        // Drop the cached workspace so the next query reloads from disk.
        let mut st = self.state.write().await;
        st.workspace = None;
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
        Ok(
            navigation::hover_markdown(&ws, &doc.low, offset).map(|value| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                }),
                range: None,
            }),
        )
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
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let ranges = symbols::folding_ranges(&doc.low);
        Ok((!ranges.is_empty()).then_some(ranges))
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
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(full_report(None, Vec::new()));
        };
        let cfg = st.config.clone();
        let (id, items) = match &ws {
            Some(ws) => {
                pull::pull_diagnostics(ws, &doc.low, params.previous_result_id.clone(), &cfg)
            }
            None => {
                let items = diagnostics::compute_diagnostics(ws.as_ref(), &doc.low, &cfg);
                (pull::diagnostics_result_id(&items), items)
            }
        };
        if params.previous_result_id.as_deref() == Some(id.as_str()) {
            return Ok(unchanged_report(id));
        }
        Ok(full_report(Some(id), items))
    }

    async fn workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> JsonRpcResult<WorkspaceDiagnosticReportResult> {
        let st = self.state.read().await;
        let mut items = Vec::new();
        let previous: HashMap<String, String> = params
            .previous_result_ids
            .iter()
            .map(|p| (p.uri.to_string(), p.value.clone()))
            .collect();
        if let Some(ws) = &st.workspace {
            let cfg = st.config.clone();
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
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let full = semantic::semantic_tokens_full(doc);
        let new_id = pull::tokens_result_id(&full.data);
        if params.previous_result_id == new_id {
            return Ok(Some(SemanticTokensFullDeltaResult::Tokens(
                SemanticTokens {
                    result_id: Some(new_id),
                    data: Vec::new(),
                },
            )));
        }
        let cached = st.token_cache.get(&uri).cloned();
        drop(st);
        if let Some((prev_id, prev)) = cached
            && prev_id == params.previous_result_id
        {
            let delta = pull::semantic_tokens_delta(&prev, &full.data);
            self.state
                .write()
                .await
                .token_cache
                .insert(uri, (new_id, full.data));
            return Ok(Some(delta));
        }
        let out_data = full.data.clone();
        self.state
            .write()
            .await
            .token_cache
            .insert(uri, (new_id, full.data));
        Ok(Some(SemanticTokensFullDeltaResult::Tokens(
            SemanticTokens {
                result_id: None,
                data: out_data,
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
        let ws = self.workspace_for(&uri).await;
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        // Arazzo documents get workflow run lenses instead of the
        // OpenAPI component/operation lenses.
        if doc.low.sniff_family() == suspect_low::SpecFamily::Arazzo10 {
            return Ok(Some(run_lenses::run_lenses(&doc.low)));
        }
        match ws {
            Some(ws) => Ok(Some(links::code_lens(&ws, &doc.low))),
            None => Ok(None),
        }
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
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(links::document_link(ws.as_ref(), &doc.low)))
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
        let client_cfg = cfg_json.as_ref().and_then(config_files::parse_config);
        {
            let mut st = self.state.write().await;
            let init_opts = std::mem::take(&mut st.pending_init_options);
            st.config = config_files::merge(init_opts, client_cfg, Default::default());
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
        let changed = {
            let mut st = self.state.write().await;
            match parsed {
                Some(c) if st.config != c => {
                    st.config = c;
                    true
                }
                Some(_) | None => false,
            }
        };
        if !changed {
            return;
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
        let items = match completion::context_at(&doc.low, offset) {
            completion::CompletionContext::Keys(keys) => completion::key_items(keys),
            completion::CompletionContext::Refs => match ws {
                Some(ws) => completion::ref_items(
                    completion::ref_candidates(&ws, doc.low.uri()),
                    doc.low.uri(),
                ),
                None => Vec::new(),
            },
            completion::CompletionContext::None => return Ok(None),
        };
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
        // two-space indent regardless of editor configuration.
        let _ = &params.options;
        let Some(url) = to_url(&uri) else {
            return Ok(None);
        };
        Ok(actions::format_document(doc, &url).map(|e| vec![e]))
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
        match params.command.as_str() {
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
    let (service, socket) = LspService::build(Backend::new)
        .custom_method(
            <run_lenses::RunWorkflowRequest as tower_lsp::lsp_types::request::Request>::METHOD,
            Backend::run_workflow_request,
        )
        .finish();
    Server::new(stdin, stdout, socket).serve(service).await;
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
