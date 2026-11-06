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
pub mod render_sdk;
pub mod run_lenses;
pub mod semantic;
pub mod state;
pub mod symbols;
pub mod type_hierarchy_fmt;
pub mod workspace_symbol;

#[cfg(test)]
mod burst_tests;
mod handlers;

use std::sync::Arc;

use state::State;
use suspect_source::Uri;
use tower_lsp::Client;
use tower_lsp::async_trait;

use serde_json::Value;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, CodeLens,
    CreateFilesParams, DeleteFilesParams, DidChangeConfigurationParams,
    DidChangeWorkspaceFoldersParams, DocumentLink, GotoDefinitionResponse, Location, Moniker,
    RenameFilesParams, SemanticTokensFullDeltaResult, SemanticTokensRangeResult,
    TextDocumentPositionParams, TypeHierarchyItem, WorkspaceDiagnosticReportResult,
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
    /// Single-flight for whole-workspace index builds: requests that miss
    /// the cache together line up behind one build instead of racing to
    /// build the same answer. Held across the build only — never across
    /// the state lock.
    index_build_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Backend {
    /// Creates a backend with empty [`State`]; called by `LspService::new`.
    fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(tokio::sync::RwLock::new(State::default())),
            index_build_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// The workspace root directory, when the workspace has one on disk.
#[must_use]
pub fn workspace_root(ws: &Arc<suspect_ref::Workspace>) -> Option<std::path::PathBuf> {
    ws.root_path().map(std::path::Path::to_path_buf)
}

#[async_trait]
impl LanguageServer for Backend {
    // ---------- lifecycle and configuration ----------
    async fn initialize(&self, params: InitializeParams) -> JsonRpcResult<InitializeResult> {
        self.handle_initialize(params).await
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.handle_initialized().await
    }

    async fn shutdown(&self) -> JsonRpcResult<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.handle_did_change_configuration(params).await
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        self.handle_did_change_workspace_folders(params).await
    }

    // ---------- document sync ----------
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.handle_did_open(params).await
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.handle_did_change(params).await
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.handle_did_save(params).await
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.handle_did_close(params).await
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        // Drop the cached workspace so the next query reloads from disk.
        self.state.write().await.drop_workspace();
        // A committed config file (`.suspect.yaml`, `suspect.project.json`)
        // edited on disk changes the lint policy: re-layer it under the
        // client's current settings and republish.
        let config_file = params.changes.iter().any(|change| {
            change
                .uri
                .to_file_path()
                .ok()
                .and_then(|path| path.file_name()?.to_str().map(config_schema::kind_of))
                .flatten()
                .is_some()
        });
        if config_file {
            let settings = self
                .state
                .read()
                .await
                .client_settings
                .clone()
                .unwrap_or(serde_json::Value::Null);
            self.handle_did_change_configuration(DidChangeConfigurationParams { settings })
                .await;
        }
    }

    // ---------- workspace file operations ----------
    async fn will_create_files(
        &self,
        params: CreateFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let ws = self.any_workspace().await;
        let Some(ws) = ws else { return Ok(None) };
        Ok(config_files::will_create_files(&ws, &params))
    }

    async fn did_create_files(&self, params: CreateFilesParams) {
        self.handle_did_create_files(params).await
    }

    async fn will_rename_files(
        &self,
        params: RenameFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        self.handle_will_rename_files(params).await
    }

    async fn did_rename_files(&self, params: RenameFilesParams) {
        self.handle_did_rename_files(params).await
    }

    async fn will_delete_files(
        &self,
        params: DeleteFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        self.handle_will_delete_files(params).await
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

    // ---------- pull diagnostics ----------
    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> JsonRpcResult<DocumentDiagnosticReportResult> {
        self.handle_diagnostic(params).await
    }

    async fn workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> JsonRpcResult<WorkspaceDiagnosticReportResult> {
        self.handle_workspace_diagnostic(params).await
    }

    // ---------- hover ----------
    async fn hover(&self, params: HoverParams) -> JsonRpcResult<Option<Hover>> {
        self.handle_hover(params).await
    }

    // ---------- completion ----------
    async fn completion(
        &self,
        params: CompletionParams,
    ) -> JsonRpcResult<Option<CompletionResponse>> {
        self.handle_completion(params).await
    }

    async fn completion_resolve(&self, item: CompletionItem) -> JsonRpcResult<CompletionItem> {
        self.handle_completion_resolve(item).await
    }

    // ---------- navigation and hierarchies ----------
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        self.handle_goto_definition(params).await
    }

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

    async fn goto_implementation(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        self.handle_goto_implementation(params).await
    }

    async fn references(&self, params: ReferenceParams) -> JsonRpcResult<Option<Vec<Location>>> {
        self.handle_references(params).await
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
        self.handle_prepare_type_hierarchy(params).await
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

    // ---------- symbols and folding ----------
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> JsonRpcResult<Option<DocumentSymbolResponse>> {
        self.handle_document_symbol(params).await
    }

    async fn folding_range(
        &self,
        params: FoldingRangeParams,
    ) -> JsonRpcResult<Option<Vec<FoldingRange>>> {
        self.handle_folding_range(params).await
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> JsonRpcResult<Option<Vec<SymbolInformation>>> {
        self.handle_symbol(params).await
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

    // ---------- semantic tokens, inlay hints, highlights, colors ----------
    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> JsonRpcResult<Option<SemanticTokensResult>> {
        self.handle_semantic_tokens_full(params).await
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> JsonRpcResult<Option<SemanticTokensRangeResult>> {
        self.handle_semantic_tokens_range(params).await
    }

    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> JsonRpcResult<Option<SemanticTokensFullDeltaResult>> {
        self.handle_semantic_tokens_full_delta(params).await
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> JsonRpcResult<Option<Vec<InlayHint>>> {
        self.handle_inlay_hint(params).await
    }

    async fn inlay_hint_resolve(&self, hint: InlayHint) -> JsonRpcResult<InlayHint> {
        self.handle_inlay_hint_resolve(hint).await
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> JsonRpcResult<Option<Vec<DocumentHighlight>>> {
        self.handle_document_highlight(params).await
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

    async fn document_color(
        &self,
        params: DocumentColorParams,
    ) -> JsonRpcResult<Vec<ColorInformation>> {
        self.handle_document_color(params).await
    }

    async fn color_presentation(
        &self,
        params: ColorPresentationParams,
    ) -> JsonRpcResult<Vec<ColorPresentation>> {
        Ok(colors::color_presentations(&params.color, params.range))
    }

    // ---------- code lenses and document links ----------
    async fn code_lens(&self, params: CodeLensParams) -> JsonRpcResult<Option<Vec<CodeLens>>> {
        self.handle_code_lens(params).await
    }

    async fn code_lens_resolve(&self, params: CodeLens) -> JsonRpcResult<CodeLens> {
        self.handle_code_lens_resolve(params).await
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> JsonRpcResult<Option<Vec<DocumentLink>>> {
        self.handle_document_link(params).await
    }

    async fn document_link_resolve(&self, params: DocumentLink) -> JsonRpcResult<DocumentLink> {
        Ok(links::document_link_resolve(params))
    }

    // ---------- rename, code actions, formatting ----------
    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> JsonRpcResult<Option<PrepareRenameResponse>> {
        self.handle_prepare_rename(params).await
    }

    async fn rename(&self, params: RenameParams) -> JsonRpcResult<Option<WorkspaceEdit>> {
        self.handle_rename(params).await
    }

    async fn linked_editing_range(
        &self,
        params: LinkedEditingRangeParams,
    ) -> JsonRpcResult<Option<LinkedEditingRanges>> {
        self.handle_linked_editing_range(params).await
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonRpcResult<Option<CodeActionResponse>> {
        self.handle_code_action(params).await
    }

    async fn code_action_resolve(&self, action: CodeAction) -> JsonRpcResult<CodeAction> {
        self.handle_code_action_resolve(action).await
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        self.handle_formatting(params).await
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        self.handle_range_formatting(params).await
    }

    // ---------- commands ----------
    async fn execute_command(&self, params: ExecuteCommandParams) -> JsonRpcResult<Option<Value>> {
        Ok(self.run_command(params).await)
    }
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
        .custom_method(
            <render_sdk::RenderSdkRequest as tower_lsp::lsp_types::request::Request>::METHOD,
            Backend::render_sdk_request,
        )
        .finish()
}
