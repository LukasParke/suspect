//! The `ServerCapabilities` advertised in the `initialize` response.

use tower_lsp::lsp_types::*;

use crate::{links, run_lenses, semantic};

/// Everything this server advertises in its `initialize` response.
pub(super) fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                // Advertise only what is implemented: neither
                // willSave nor willSaveWaitUntil has a handler, and
                // claiming them made editors send `textDocument/willSave`
                // on every save and throw the MethodNotFound into
                // their extension host log. `save` below is the one
                // that is implemented — it asks the client to send
                // didSave notifications, which `did_save` handles.
                will_save: None,
                will_save_wait_until: None,
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
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            work_done_progress_options: WorkDoneProgressOptions::default(),
            code_action_kinds: Some(vec![
                CodeActionKind::QUICKFIX,
                CodeActionKind::new("source.fixAll.suspect"),
            ]),
            resolve_provider: Some(true),
        })),
        document_formatting_provider: Some(OneOf::Left(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                work_done_progress_options: WorkDoneProgressOptions::default(),
                legend: semantic::legend(),
                range: Some(true),
                full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
            },
        )),
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
        linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(true)),
        document_range_formatting_provider: Some(OneOf::Left(true)),
        diagnostic_provider: Some(DiagnosticServerCapabilities::Options(DiagnosticOptions {
            identifier: Some("suspect".to_owned()),
            inter_file_dependencies: true,
            workspace_diagnostics: true,
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),
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
    }
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
