# OpenAPI LSP Feature Roadmap

Comprehensive enumeration of valuable LSP features for an OpenAPI toolchain,
researched from rust-analyzer, yaml-language-server, Telescope, vacuum, and
first-principles analysis of API design workflows.

## Implemented ✅

| Feature | LSP Method | Notes |
|---|---|---|
| Live diagnostics | `textDocument/publishDiagnostics` | Syntax + 22 semantic checks + 21 lint rules + Arazzo validation |
| Rich hover | `textDocument/hover` | Schema tables with properties/constraints/composition |
| Go-to-definition | `textDocument/definition` | $ref chains, multi-file, dirty buffers |
| Find references | `textDocument/references` | Workspace-wide reverse lookup |
| Completion | `textDocument/completion` | Context-aware keys + $ref candidates |
| Quick fixes | `textDocument/codeAction` | 8 fixes + fixAll |
| Rename | `textDocument/rename` | Workspace-wide $ref rewriting |
| Prepare rename | `textDocument/prepareRename` | Component sections only |
| Semantic tokens | `textDocument/semanticTokens/full` | Keywords/methods/types/properties |
| Inlay hints | `textDocument/inlayHint` | $ref targets + property types |
| Document symbols | `textDocument/documentSymbol` | Per-family (OAS/Arazzo/Overlay) |
| Folding ranges | `textDocument/foldingRange` | Mappings/sequences >3 lines |
| Document formatting | `textDocument/formatting` | Canonical YAML/JSON |
| Workspace symbols | `workspace/symbol` | Cross-document search |
| Selection ranges | `textDocument/selectionRange` | CST ancestor chain |
| Document highlights | `textDocument/documentHighlight` | Same-target $ref occurrences |

## Planned: High impact 🔴

| Feature | LSP Method | Description |
|---|---|---|
| Schema-driven property completion | `completion` | When inside `properties:` of a `$ref`'d schema, offer referenced schema's properties with types |
| Snippet completions | `completion` | Operation skeletons (`GET /pets → {get: {operationId, parameters, responses}}`), parameter blocks, response objects |
| Enum value completion | `completion` | In `enum:` arrays, offer declared values |
| Security scheme completion | `completion` | In `security:` blocks, offer declared scheme names |
| Tag completion | `completion` | In operation `tags:`, offer root-level declared tags |
| Deprecated modifier | `semanticTokens/full` | Add DEPRECATED modifier for deprecated operations/schemas |
| Expand $ref recursively | Custom command | Inline all composition ($ref/allOf) into a single resolved schema view |
| Effective-schema hover | `hover` | Merge allOf compositions into effective schema before rendering |
| Example ↔ schema validation | `publishDiagnostics` | Validate `example:` values against their schema constraints |
| Breaking-change detection | Custom | Diff current spec against git base revision; flag breaking changes as Error diagnostics |
| Call hierarchy | `callHierarchy/incomingCalls` + `outgoingCalls` | Track which operations use which schemas via $ref edges |
| Code lens (reference counts) | `textDocument/codeLens` | "N references" above each component; click to navigate |
| Diagnostic related information | `publishDiagnostics` | Link diagnostics to referenced schema definitions |

## Planned: Medium impact 🟡

| Feature | LSP Method | Description |
|---|---|---|
| Signature help | `textDocument/signatureHelp` | When typing parameters, show schema constraints inline |
| Document links | `textDocument/documentLink` | External docs URLs, $ref file links clickable |
| Auto-generate descriptions | `codeAction` | Copy summary → description when description is empty |
| Extract to components | `codeAction` | Move inline schema to `components/schemas` with auto-$ref rewrite |
| Inline $ref | `codeAction` | Replace $ref with the target schema content |
| Spec scaffold | `workspaceSymbol` | Generate skeleton spec from path list |
| Resource CRUD scaffolding | Custom | Generate full CRUD operations for a resource path |
| Convention lints | `publishDiagnostics` | Naming conventions (kebab-case paths, camelCase properties) |
| Duplicate schema detection | `publishDiagnostics` | Flag schemas with identical structure for consolidation |
| Design-smell heuristics | `publishDiagnostics` | Missing descriptions, no examples, empty responses, single-response APIs |
| Multi-document support | All methods | Handle `---` separated multi-doc YAML files |
| YAML anchor/alias navigation | `definition`, `references` | Navigate between anchors and their aliases |
| Overlay-aware editing | All methods | Understand Overlay documents targeting OAS specs |
| Validation trace ("why invalid") | Hover on diagnostic | Explain validation failure chain step by step |
| Server variable completion | `completion` | In server URLs `{variable}` placeholders |

## Planned: Lower priority 🟢

| Feature | LSP Method | Description |
|---|---|---|
| Code lenses for section stats | `textDocument/codeLens` | "N schemas", "N operations" above sections |
| On-type formatting | `textDocument/onTypeFormatting` | Auto-indent on Enter after `:` |
| Color decorators | `textDocument/documentColor` | Swatches for color-like enum values |
| Join lines assist | `codeAction` | Merge multi-line strings into folded scalars |
| Move item up/down | `codeAction` | Reorder paths/operations/schemas |
| Structural search & replace | Custom | AST-based search across workspace |
| Dependency graph view | Custom extension | Visual graph of $ref relationships |
| Spec-version migration assistant | `codeAction` | 3.0 → 3.1 upgrade suggestions |
| Contract-evolution assist | Custom | Suggest backwards-compatible changes only |
| Reference-doc generation | Custom command | Generate markdown docs from selected component |
| Form-based OpenAPI designer | Custom extension | Structured editing UI alongside text |
