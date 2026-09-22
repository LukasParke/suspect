//! M2 "perfect tree shaking" acceptance gates for the TypeScript HTTP package
//! (golden-defaults plan §12): per-operation, per-codec, per-grant and
//! per-adapter elimination, measured with the repo-pinned esbuild, plus a
//! generation-level launcher-size stability gate that pins the
//! one-operation artifact-independence property without any bundler.
//!
//! Two gates exist by design and every report states which one ran:
//! - `bundler`: the pinned esbuild bundles each consumer profile twice (with
//!   and without an unrelated source operation). Retained-code markers, the
//!   esbuild metafile's resolved module graph, and artifact byte counts gate
//!   the plan's additivity property.
//! - `graph`: a dependency-free static import-graph model of a *perfect*
//!   tree shaker (name-propagating reachability over the emitted ESM graph)
//!   gates the same properties at module level. Intra-module retention and
//!   byte counts are then CI-pending bundler work, and the launcher-stability
//!   test pins generation-level independence instead.
//!
//! The graph model is deliberately stronger than what esbuild achieves today:
//! wherever the model says a module's bindings are unreachable but the bundler
//! still evaluates the module (or retains its descriptor data), this test
//! records a tree-shaking defeat in the report instead of weakening a marker.
//!
//! The emitted top-level descriptor-data initializers — the frozen pagination
//! descriptor map, the frozen OAuth scheme map and the typed stream-events
//! descriptor clones — carry `/* @__PURE__ */`: each only freezes or
//! reassembles plain generated data, so esbuild can shed them (and every
//! unrelated operation's descriptor data with them) from consumer bundles that
//! reference none of their bindings. The absent-marker lists gate exactly that
//! shedding: dead pager-descriptor data in non-pager bundles, dead OAuth
//! scheme data in non-OAuth bundles, and dead stream descriptor data in
//! non-stream bundles.
#![cfg(feature = "http-protocol")]

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::{OAuthDefaults, OAuthSchemeConfig, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// Fixed logical source URI for both fixture documents: emitted source
/// references stay byte-comparable across the two generations.
const ENTRY: &str = "https://source.elimination.test/openapi.json";

/// The pinned esbuild toolchain used by the tracked bundle harness.
const BUNDLE_TOOL: &str = "tools/typescript-http-bundle";

/// Profiles whose artifact must not grow when the source gains an unrelated
/// operation (the plan's additivity property).
const ADDITIVE_PROFILES: &[&str] = &[
    "empty",
    "one-json-operation",
    "one-pager",
    "one-stream",
    "one-oauth-provider",
    "one-codec",
    "type-only",
    "type-position-only",
];

/// Small constant slack for bundler bookkeeping differences between two
/// otherwise-identical reachable sets. Zero retention of the unrelated
/// operation is asserted independently by its markers.
const ADDITIVITY_SLACK_BYTES: u64 = 64;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

/// The canonical fixture: one limit/offset paginated list operation, one
/// discriminated SSE stream operation, one plain JSON read operation, one
/// OAuth2 client-credentials operation, and one device-flow operation, all
/// under an API-key document policy.
fn elimination_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Elimination fixture", "version": "1.0.0"},
        "servers": [{"url": "https://api.elimination.test/v1"}],
        "security": [{"apiKey": []}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "summary": "List widgets with limit/offset pagination.",
                "parameters": [
                    {"name": "limit", "in": "query", "schema": {"type": "integer", "minimum": 1}},
                    {"name": "offset", "in": "query", "schema": {"type": "integer", "minimum": 0}},
                    {"name": "filter", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Widget page", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "data": {"type": "array", "items": {"type": "string"}},
                        "total": {"type": "integer"}
                    },
                    "required": ["data", "total"]
                }}}}}
            }},
            "/chat": {"post": {
                "operationId": "streamChat",
                "summary": "Stream chat completion events.",
                "responses": {"200": {"description": "Chat events", "content": {"text/event-stream": {"itemSchema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "done"]},
                        "data": {"type": "string"}
                    },
                    "required": ["event", "data"]
                }}}}}
            }},
            "/gadgets/{gadgetId}": {"get": {
                "operationId": "getGadget",
                "summary": "Fetch one gadget.",
                "parameters": [{"name": "gadgetId", "in": "path", "required": true, "schema": {"type": "string"}}],
                "responses": {"200": {"description": "Gadget", "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/Gadget"}
                }}}}
            }},
            "/banners": {"post": {
                "operationId": "createBanner",
                "summary": "Create a banner with OAuth2 client credentials.",
                "security": [{"serviceOAuth": ["read"]}],
                "requestBody": {"required": true, "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/BannerCreate"}
                }}},
                "responses": {"200": {"description": "Banner acknowledged", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]
                }}}}}
            }},
            "/licenses": {"get": {
                "operationId": "listLicenses",
                "summary": "List licenses with the device-authorization scheme.",
                "security": [{"deviceOAuth": []}],
                "responses": {"200": {"description": "Licenses", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {"items": {"type": "array", "items": {"type": "string"}}},
                    "required": ["items"]
                }}}}}
            }}
        },
        "components": {
            "securitySchemes": {
                "apiKey": {"type": "http", "scheme": "bearer"},
                "serviceOAuth": {"type": "oauth2", "flows": {
                    "authorizationCode": {
                        "authorizationUrl": "https://auth.elimination.test/authorize",
                        "tokenUrl": "https://auth.elimination.test/token",
                        "refreshUrl": "https://auth.elimination.test/token-refresh",
                        "scopes": {"read": "Read access", "write": "Write access"}
                    },
                    "clientCredentials": {
                        "tokenUrl": "https://auth.elimination.test/token",
                        "refreshUrl": "https://auth.elimination.test/token-refresh",
                        "scopes": {"read": "Read access"}
                    }
                }},
                "deviceOAuth": {"type": "oauth2", "flows": {"deviceAuthorization": {
                    "deviceAuthorizationUrl": "https://auth.elimination.test/device",
                    "tokenUrl": "https://auth.elimination.test/token",
                    "scopes": {}
                }}}
            },
            "schemas": {
                "BannerCreate": {"type": "object", "additionalProperties": false,
                    "properties": {"text": {"type": "string"}, "weight": {"type": "integer"}},
                    "required": ["text"]},
                "Gadget": {"type": "object", "additionalProperties": false,
                    "properties": {
                        "kind": {"type": "string", "enum": ["standard", "compact"]},
                        "label": {"type": "string"}
                    },
                    "required": ["kind", "label"]}
            }
        }
    })
}

/// The same contract plus one operation that no consumer profile imports. Its
/// pointers sort strictly between existing ones, so every shared source
/// pointer is stable and the two generations are byte-comparable.
fn document_with_unrelated_operation() -> Value {
    let mut document = elimination_document();
    let root = document.as_object_mut().unwrap();
    root.get_mut("paths")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "/gizmos".to_owned(),
            json!({"get": {
                "operationId": "listGizmos",
                "summary": "Unrelated gizmo inventory probe.",
                "responses": {"200": {"description": "Gizmos", "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/UnrelatedGizmo"}
                }}}}
            }}),
        );
    root.get_mut("components")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("schemas")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "UnrelatedGizmo".to_owned(),
            json!({"type": "object", "additionalProperties": false,
                "properties": {
                    "flavor": {"type": "string", "enum": ["zeta-quantum"]},
                    "serial": {"type": "string"}
                },
                "required": ["flavor", "serial"]}),
        );
    document
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@elimination/fixture".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    }
}

fn elimination_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([
                    (
                        "serviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_ELIMINATION_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_ELIMINATION_CLIENT_SECRET".into()),
                            revocation_endpoint: Some(
                                "https://auth.elimination.test/revoke".into(),
                            ),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                    (
                        "deviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_ELIMINATION_DEVICE_ID".into()),
                            client_secret_env: Some("SUSPECT_ELIMINATION_DEVICE_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                ]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(contract, &selected, &target(), &elimination_options()).unwrap()
}

fn files_map(files: &[OutFile]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|file| (file.path.clone(), file.content.clone()))
        .collect()
}

fn file<'map>(files: &'map BTreeMap<String, String>, path: &str) -> &'map str {
    files
        .get(path)
        .unwrap_or_else(|| panic!("{path} is emitted"))
}

/// Retention markers. Bundles minify away identifiers, so every marker is a
/// string literal or wire value that survives only inside retained code or
/// retained descriptor data.
const MARKER_OPERATION_IDS: &[&str] = &[
    "listWidgets",
    "getGadget",
    "streamChat",
    "createBanner",
    "listLicenses",
];
const MARKER_OAUTH_GRANT_CODE: &[&str] = &[
    "client_credentials",
    "authorization_code",
    "code_challenge_method",
    "S256",
    "urn:ietf:params:oauth:grant-type:device_code",
];
const MARKER_PAGINATION_CODE: &[&str] = &["PaginationError", "limit-offset"];
const MARKER_STREAM_WIRE: &[&str] = &["text/event-stream"];
/// Compiled OAuth scheme-descriptor data: the frozen `oauthSchemes` map is the
/// only place these endpoint URLs and environment-variable names exist, so
/// their retention in a bundle proves the scheme *data* (not just OAuth code)
/// survived bundling.
const MARKER_OAUTH_SCHEME_DATA: &[&str] = &[
    "https://auth.elimination.test/authorize",
    "https://auth.elimination.test/device",
    "https://auth.elimination.test/revoke",
    "SUSPECT_ELIMINATION_DEVICE_ID",
    "SUSPECT_ELIMINATION_DEVICE_SECRET",
];
const MARKER_UNRELATED_OPERATION: &[&str] = &["listGizmos", "zeta-quantum"];

/// Precomputed marker sets. Const items (unlike locals) back `'static` slice
/// literals, so every profile's absent list is a named const.
const ABSENT_CORE_CODE: &[&str] = &[
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
    MARKER_OAUTH_GRANT_CODE[0],
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[2],
    MARKER_OAUTH_GRANT_CODE[3],
    MARKER_OAUTH_GRANT_CODE[4],
    // The explicitly configured OAuth supplements exist only in the frozen
    // scheme map, so even this all-operations client must shed them with it.
    // (The flow URLs cannot be gated here: createBanner's and listLicenses'
    // retained Source provenance legitimately embeds them.)
    MARKER_OAUTH_SCHEME_DATA[2],
    MARKER_OAUTH_SCHEME_DATA[3],
    MARKER_OAUTH_SCHEME_DATA[4],
];
const ABSENT_ONE_JSON_OPERATION: &[&str] = &[
    MARKER_OPERATION_IDS[0],
    MARKER_OPERATION_IDS[2],
    MARKER_OPERATION_IDS[3],
    MARKER_OPERATION_IDS[4],
    MARKER_STREAM_WIRE[0],
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
    MARKER_OAUTH_GRANT_CODE[0],
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[2],
    MARKER_OAUTH_GRANT_CODE[3],
    MARKER_OAUTH_GRANT_CODE[4],
    MARKER_OAUTH_SCHEME_DATA[0],
    MARKER_OAUTH_SCHEME_DATA[1],
    MARKER_OAUTH_SCHEME_DATA[2],
    MARKER_OAUTH_SCHEME_DATA[3],
    MARKER_OAUTH_SCHEME_DATA[4],
];
const ABSENT_ONE_PAGER: &[&str] = &[
    MARKER_OPERATION_IDS[1],
    MARKER_OPERATION_IDS[2],
    MARKER_OPERATION_IDS[3],
    MARKER_OPERATION_IDS[4],
    MARKER_STREAM_WIRE[0],
    MARKER_OAUTH_GRANT_CODE[0],
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[2],
    MARKER_OAUTH_GRANT_CODE[3],
    MARKER_OAUTH_GRANT_CODE[4],
    MARKER_OAUTH_SCHEME_DATA[0],
    MARKER_OAUTH_SCHEME_DATA[1],
    MARKER_OAUTH_SCHEME_DATA[2],
    MARKER_OAUTH_SCHEME_DATA[3],
    MARKER_OAUTH_SCHEME_DATA[4],
];
const ABSENT_ONE_OAUTH_PROVIDER: &[&str] = &[
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[2],
    MARKER_OAUTH_GRANT_CODE[3],
    MARKER_OAUTH_GRANT_CODE[4],
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
    MARKER_STREAM_WIRE[0],
    MARKER_OPERATION_IDS[0],
    MARKER_OPERATION_IDS[1],
    MARKER_OPERATION_IDS[2],
    MARKER_OPERATION_IDS[3],
    MARKER_OPERATION_IDS[4],
];
const ABSENT_ONE_CODEC: &[&str] = &[
    MARKER_OPERATION_IDS[0],
    MARKER_OPERATION_IDS[1],
    MARKER_OPERATION_IDS[2],
    MARKER_OPERATION_IDS[3],
    MARKER_OPERATION_IDS[4],
    MARKER_STREAM_WIRE[0],
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
    MARKER_OAUTH_GRANT_CODE[0],
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[2],
    MARKER_OAUTH_GRANT_CODE[3],
    MARKER_OAUTH_GRANT_CODE[4],
];
const ABSENT_EMPTY: &[&str] = &[
    MARKER_OPERATION_IDS[0],
    MARKER_OPERATION_IDS[1],
    MARKER_OPERATION_IDS[2],
    MARKER_OPERATION_IDS[3],
    MARKER_OPERATION_IDS[4],
    MARKER_STREAM_WIRE[0],
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
    MARKER_OAUTH_GRANT_CODE[0],
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[2],
    MARKER_OAUTH_GRANT_CODE[4],
];
const PRESENT_ONE_JSON_OPERATION: &[&str] = &[MARKER_OPERATION_IDS[1]];
const PRESENT_ONE_PAGER: &[&str] = &[
    MARKER_OPERATION_IDS[0],
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
];
const ABSENT_ONE_STREAM: &[&str] = &[
    MARKER_OPERATION_IDS[0],
    MARKER_OPERATION_IDS[1],
    MARKER_OPERATION_IDS[3],
    MARKER_OPERATION_IDS[4],
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
    MARKER_OAUTH_GRANT_CODE[0],
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[2],
    MARKER_OAUTH_GRANT_CODE[3],
    MARKER_OAUTH_GRANT_CODE[4],
    MARKER_OAUTH_SCHEME_DATA[0],
    MARKER_OAUTH_SCHEME_DATA[1],
    MARKER_OAUTH_SCHEME_DATA[2],
    MARKER_OAUTH_SCHEME_DATA[3],
    MARKER_OAUTH_SCHEME_DATA[4],
];
const PRESENT_ONE_STREAM: &[&str] = &[MARKER_OPERATION_IDS[2], MARKER_STREAM_WIRE[0]];
const PRESENT_ONE_OAUTH_PROVIDER: &[&str] = &[MARKER_OAUTH_GRANT_CODE[0]];
const PRESENT_ONE_CODEC: &[&str] = &["standard", "compact"];
const PRESENT_ALL_OPERATIONS: &[&str] = &[
    MARKER_OPERATION_IDS[0],
    MARKER_OPERATION_IDS[1],
    MARKER_OPERATION_IDS[2],
    MARKER_OPERATION_IDS[3],
    MARKER_OPERATION_IDS[4],
    MARKER_STREAM_WIRE[0],
    MARKER_PAGINATION_CODE[0],
    MARKER_PAGINATION_CODE[1],
    MARKER_OAUTH_GRANT_CODE[0],
    MARKER_OAUTH_GRANT_CODE[1],
    MARKER_OAUTH_GRANT_CODE[4],
];

/// A consumer profile: a tiny entry module plus the retention markers that
/// must be present or absent in its bundle, the emitted modules the resolved
/// module graph must include or exclude, and the graph-model expectation.
struct Profile {
    name: &'static str,
    entry: &'static str,
    /// Markers that must appear in every bundle of this profile.
    present: &'static [&'static str],
    /// Markers that must never appear, in either document.
    absent: &'static [&'static str],
    /// Emitted modules (package-relative) that the bundle must include.
    included_modules: &'static [&'static str],
    /// Emitted modules (package-relative) that the bundle must exclude.
    excluded_modules: &'static [&'static str],
    /// Whether the bundle may grow when the unrelated operation is added.
    additivity: Additivity,
    /// The perfect-shaker graph-model expectation for this profile.
    graph: Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Additivity {
    /// Artifact bytes must not grow beyond a small constant when an unrelated
    /// operation is added to the source.
    Invariant,
    /// The profile intentionally retains every operation (e.g. `createClient`).
    Grows,
}

/// Module-level expectation of the dependency-free graph model (what a
/// perfect tree shaker could reach by propagating needed names).
enum Graph {
    Decidable {
        /// Modules whose bindings must be reachable.
        reachable: &'static [&'static str],
        /// Modules whose bindings must not be reachable.
        unreachable: &'static [&'static str],
    },
    /// Type-position-only imports need TypeScript knowledge; only the bundler
    /// (or a type-aware consumer build) can decide this profile.
    BundlerOnly,
}

/// Consumer entry source for each profile; imports resolve relatively into the
/// generated `typescript/` source tree, which the bundler compiles directly.
fn profile(name: &str) -> Profile {
    match name {
        "empty" => Profile {
            name: "empty",
            entry: "export const eliminatedEmpty = 0;\n",
            present: &[],
            absent: ABSENT_EMPTY,
            included_modules: &[],
            excluded_modules: &[
                "typescript/operations.ts",
                "typescript/models.ts",
                "typescript/model-codecs.ts",
                "typescript/pagination.ts",
                "typescript/oauth.ts",
                "typescript/runtime.ts",
            ],
            additivity: Additivity::Invariant,
            graph: Graph::Decidable {
                reachable: &[],
                unreachable: &[
                    "typescript/operations.ts",
                    "typescript/models.ts",
                    "typescript/model-codecs.ts",
                    "typescript/pagination.ts",
                    "typescript/oauth.ts",
                ],
            },
        },
        "core-client-operations" => Profile {
            name: "core-client-operations",
            entry: "import { createClient } from '../typescript/operations.js';\nexport const bound = createClient;\n",
            // createClient is the all-operations surface by design: it returns
            // one bound method per source operation, so every operation and
            // its codec chain is retained here.
            present: MARKER_OPERATION_IDS,
            absent: ABSENT_CORE_CODE,
            included_modules: &["typescript/operations.ts", "typescript/runtime.ts"],
            excluded_modules: &["typescript/models.ts"],
            additivity: Additivity::Grows,
            graph: Graph::Decidable {
                reachable: &[
                    "typescript/operations.ts",
                    "typescript/runtime.ts",
                    "typescript/model-codecs.ts",
                ],
                unreachable: &["typescript/oauth.ts", "typescript/models.ts"],
            },
        },
        "core-client-root" => Profile {
            name: "core-client-root",
            entry: "import { createClient } from '../typescript/source/index.js';\nexport const bound = createClient;\n",
            present: MARKER_OPERATION_IDS,
            absent: ABSENT_CORE_CODE,
            included_modules: &["typescript/operations.ts", "typescript/source/index.ts"],
            excluded_modules: &[],
            additivity: Additivity::Grows,
            graph: Graph::Decidable {
                reachable: &[
                    "typescript/source/index.ts",
                    "typescript/operations.ts",
                    "typescript/runtime.ts",
                ],
                unreachable: &["typescript/oauth.ts", "typescript/models.ts"],
            },
        },
        "one-json-operation" => Profile {
            name: "one-json-operation",
            entry: "import { getGadget, isGetGadgetApiError } from '../typescript/operations.js';\nexport const call = getGadget;\nexport const guard = isGetGadgetApiError;\n",
            present: PRESENT_ONE_JSON_OPERATION,
            absent: ABSENT_ONE_JSON_OPERATION,
            included_modules: &[
                "typescript/operations.ts",
                "typescript/model-codecs.ts",
                "typescript/codecs.ts",
                "typescript/runtime.ts",
            ],
            excluded_modules: &["typescript/models.ts"],
            additivity: Additivity::Invariant,
            graph: Graph::Decidable {
                reachable: &[
                    "typescript/operations.ts",
                    "typescript/model-codecs.ts",
                    "typescript/codecs.ts",
                    "typescript/runtime.ts",
                ],
                unreachable: &["typescript/oauth.ts", "typescript/models.ts"],
            },
        },
        "one-pager" => Profile {
            name: "one-pager",
            entry: "import { listWidgetsItems } from '../typescript/operations.js';\nexport const walk = listWidgetsItems;\n",
            // The pager wraps its own operation by design and reads the frozen
            // per-operation descriptor map, so both stay retained.
            present: PRESENT_ONE_PAGER,
            absent: ABSENT_ONE_PAGER,
            included_modules: &[
                "typescript/operations.ts",
                "typescript/pagination.ts",
                "typescript/runtime.ts",
            ],
            excluded_modules: &["typescript/models.ts"],
            additivity: Additivity::Invariant,
            graph: Graph::Decidable {
                reachable: &[
                    "typescript/operations.ts",
                    "typescript/pagination.ts",
                    "typescript/runtime.ts",
                ],
                unreachable: &["typescript/oauth.ts", "typescript/models.ts"],
            },
        },
        "one-stream" => Profile {
            name: "one-stream",
            entry: "import { streamChat } from '../typescript/operations.js';\nexport const call = streamChat;\n",
            // The stream operation keeps its own descriptor (Source/Wire data
            // included); every other operation's descriptor data, the pager's
            // frozen descriptor map and the OAuth scheme map must shed. The
            // stream op uses no pagination and no OAuth.
            present: PRESENT_ONE_STREAM,
            absent: ABSENT_ONE_STREAM,
            included_modules: &[
                "typescript/operations.ts",
                "typescript/runtime.ts",
                "typescript/model-codecs.ts",
                "typescript/codecs.ts",
            ],
            excluded_modules: &["typescript/models.ts"],
            additivity: Additivity::Invariant,
            graph: Graph::Decidable {
                reachable: &[
                    "typescript/operations.ts",
                    "typescript/runtime.ts",
                    "typescript/model-codecs.ts",
                    "typescript/codecs.ts",
                ],
                unreachable: &["typescript/oauth.ts", "typescript/models.ts"],
            },
        },
        "one-oauth-provider" => Profile {
            name: "one-oauth-provider",
            entry: "import { createClientCredentialsProvider } from '../typescript/oauth.js';\nexport const provider = createClientCredentialsProvider;\n",
            // One grant, one provider: the client-credentials flow shares the
            // refresh-token helper and the single compiled MemoryTokenStore
            // adapter, and must shed the authorization-code (PKCE), refresh-
            // provider and device-authorization grants entirely. The frozen
            // scheme *map* itself stays reachable here by design: the provider
            // resolves `options.scheme` dynamically at call time, so no tree
            // shaker may prune unrelated schemes from this bundle. Scheme-data
            // shedding is gated on the profiles that use no OAuth at all
            // (`one-json-operation`, `one-pager`, `one-stream`, `one-codec`).
            present: PRESENT_ONE_OAUTH_PROVIDER,
            absent: ABSENT_ONE_OAUTH_PROVIDER,
            included_modules: &["typescript/oauth.ts"],
            excluded_modules: &[
                "typescript/operations.ts",
                "typescript/models.ts",
                "typescript/model-codecs.ts",
                "typescript/pagination.ts",
            ],
            additivity: Additivity::Invariant,
            graph: Graph::Decidable {
                reachable: &["typescript/oauth.ts"],
                unreachable: &[
                    "typescript/operations.ts",
                    "typescript/models.ts",
                    "typescript/model-codecs.ts",
                    "typescript/pagination.ts",
                ],
            },
        },
        "one-codec" => Profile {
            name: "one-codec",
            entry: "import { GadgetCodec } from '../typescript/model-codecs.js';\nexport const codec = GadgetCodec;\n",
            // One codec pulls exactly the codec runtime, the validation
            // runtime and its own root's validator; the Gadget enum values
            // prove the validator data is the Gadget closure.
            present: PRESENT_ONE_CODEC,
            absent: ABSENT_ONE_CODEC,
            included_modules: &[
                "typescript/model-codecs.ts",
                "typescript/codecs.ts",
                "typescript/validation.ts",
                "typescript/validation-program.ts",
            ],
            excluded_modules: &[
                "typescript/operations.ts",
                "typescript/models.ts",
                "typescript/pagination.ts",
                "typescript/oauth.ts",
            ],
            additivity: Additivity::Invariant,
            graph: Graph::Decidable {
                reachable: &[
                    "typescript/model-codecs.ts",
                    "typescript/codecs.ts",
                    "typescript/validation.ts",
                ],
                unreachable: &[
                    "typescript/operations.ts",
                    "typescript/models.ts",
                    "typescript/pagination.ts",
                    "typescript/oauth.ts",
                ],
            },
        },
        "type-only" => Profile {
            name: "type-only",
            entry: "import type { Gadget } from '../typescript/models.js';\nimport type { ListWidgetsInput } from '../typescript/operations.js';\nexport type Holder = { readonly value: Gadget | null; readonly input?: ListWidgetsInput };\n",
            present: &[],
            absent: ABSENT_EMPTY,
            included_modules: &[],
            excluded_modules: &[
                "typescript/operations.ts",
                "typescript/models.ts",
                "typescript/model-codecs.ts",
            ],
            additivity: Additivity::Invariant,
            graph: Graph::Decidable {
                reachable: &[],
                unreachable: &[
                    "typescript/operations.ts",
                    "typescript/models.ts",
                    "typescript/model-codecs.ts",
                ],
            },
        },
        // Stronger than `type-only`: a plain (non-`import type`) import whose
        // bindings are used only in type positions must erase entirely too.
        // TypeScript-only knowledge, so only the bundler can gate it.
        "type-position-only" => Profile {
            name: "type-position-only",
            entry: "import { Gadget } from '../typescript/models.js';\nimport { ListWidgetsInput } from '../typescript/operations.js';\nexport type Holder = { readonly value: Gadget | null; readonly input?: ListWidgetsInput };\n",
            present: &[],
            absent: ABSENT_EMPTY,
            included_modules: &[],
            excluded_modules: &[
                "typescript/operations.ts",
                "typescript/models.ts",
                "typescript/model-codecs.ts",
            ],
            additivity: Additivity::Invariant,
            graph: Graph::BundlerOnly,
        },
        "all-operations" => Profile {
            name: "all-operations",
            entry: "import * as operations from '../typescript/operations.js';\nexport const everything = operations;\n",
            present: PRESENT_ALL_OPERATIONS,
            absent: &[],
            included_modules: &[
                "typescript/operations.ts",
                "typescript/pagination.ts",
                "typescript/oauth.ts",
            ],
            excluded_modules: &[],
            additivity: Additivity::Grows,
            graph: Graph::Decidable {
                reachable: &[
                    "typescript/operations.ts",
                    "typescript/pagination.ts",
                    "typescript/oauth.ts",
                    "typescript/model-codecs.ts",
                ],
                unreachable: &["typescript/models.ts"],
            },
        },
        other => unreachable!("unknown profile {other}"),
    }
}

const PROFILES: &[&str] = &[
    "empty",
    "core-client-operations",
    "core-client-root",
    "one-json-operation",
    "one-pager",
    "one-stream",
    "one-oauth-provider",
    "one-codec",
    "type-only",
    "type-position-only",
    "all-operations",
];

// ---------------------------------------------------------------------------
// Bundler invocation (pinned esbuild from the tracked harness toolchain)
// ---------------------------------------------------------------------------

struct Esbuild {
    program: PathBuf,
    node: Option<PathBuf>,
    version: String,
}

impl Esbuild {
    /// Locate the pinned esbuild installed for the tracked bundle harness: the
    /// native platform binary first, then the Node shim. The version must
    /// match the harness package.json pin; anything else falls back to the
    /// graph gate. Returns `None` when the toolchain is not invocable.
    fn pinned() -> Option<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(BUNDLE_TOOL);
        let modules = root.join("node_modules");
        let pinned_version: String = serde_json::from_str::<Value>(
            &std::fs::read_to_string(root.join("package.json")).ok()?,
        )
        .ok()?
        .get("dependencies")?
        .get("esbuild")?
        .as_str()?
        .to_owned();
        let mut candidate = None;
        if let Ok(platforms) = std::fs::read_dir(modules.join("@esbuild")) {
            for platform in platforms.flatten() {
                let binary = platform.path().join("bin").join("esbuild");
                if binary.is_file() {
                    candidate = Some((binary, None));
                    break;
                }
            }
        }
        let shim = modules.join("esbuild").join("bin").join("esbuild");
        if candidate.is_none() && shim.is_file() {
            candidate = Some((shim, Some(node_program()?)));
        }
        let (program, node) = candidate?;
        let mut command = match &node {
            Some(node) => {
                let mut command = Command::new(node);
                command.arg(&program);
                command
            }
            None => Command::new(&program),
        };
        let output = command.arg("--version").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let version = String::from_utf8(output.stdout).ok()?.trim().to_owned();
        if version != pinned_version {
            eprintln!(
                "esbuild {version} does not match the harness pin {pinned_version}; falling back to the graph gate"
            );
            return None;
        }
        Some(Self {
            program,
            node,
            version,
        })
    }

    fn command(&self) -> Command {
        match &self.node {
            Some(node) => {
                let mut command = Command::new(node);
                command.arg(&self.program);
                command
            }
            None => Command::new(&self.program),
        }
    }
}

fn node_program() -> Option<PathBuf> {
    let selected = std::env::var_os("SUSPECT_PACKAGE_NODE")
        .or_else(|| std::env::var_os("SUSPECT_DOCS_NODE"))
        .unwrap_or_else(|| "node".into());
    let program = PathBuf::from(selected);
    let ok = Command::new(&program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    ok.then_some(program)
}

struct Bundle {
    bytes: u64,
    gzip_bytes: Option<u64>,
    /// Resolved input modules from the esbuild metafile, package-relative.
    modules: BTreeSet<String>,
    text: String,
}

/// Bundle one consumer entry with the pinned esbuild: ESM, minified, with a
/// metafile so the resolved module graph can be gated too.
fn bundle_with(
    esbuild: &Esbuild,
    package_root: &Path,
    consumer: &Path,
    profile: &Profile,
    tag: &str,
) -> Bundle {
    let entry = consumer.join(format!("{tag}-{}.ts", profile.name));
    std::fs::write(&entry, profile.entry).unwrap();
    let outfile = consumer.join(format!("{tag}-{}.mjs", profile.name));
    let metafile = consumer.join(format!("{tag}-{}.meta.json", profile.name));
    let output = esbuild
        .command()
        .current_dir(consumer)
        .arg(entry.file_name().unwrap())
        .args([
            "--bundle",
            "--minify",
            "--format=esm",
            "--target=es2022",
            "--legal-comments=none",
        ])
        .arg(format!("--outfile={}", outfile.display()))
        .arg(format!("--metafile={}", metafile.display()))
        .arg("--log-level=error")
        .output()
        .expect("esbuild must run");
    assert!(
        output.status.success(),
        "esbuild failed for {} ({}): {}{}",
        profile.name,
        tag,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&outfile).unwrap();
    let bytes = text.len() as u64;
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(&metafile).unwrap()).unwrap();
    let modules = meta["inputs"]
        .as_object()
        .expect("metafile inputs")
        .keys()
        .map(|path| package_relative(path, package_root, consumer))
        .collect::<BTreeSet<_>>();
    let outputs = meta["outputs"].as_object().expect("metafile outputs");
    assert_eq!(
        outputs.len(),
        1,
        "the generated package emits no dynamic imports, so every consumer bundle is exactly one chunk without code splitting (counting asynchronous chunks stays trivial)"
    );
    Bundle {
        bytes,
        gzip_bytes: gzip_bytes(text.as_bytes()),
        modules,
        text,
    }
}

fn gzip_bytes(bytes: &[u8]) -> Option<u64> {
    let mut child = Command::new("gzip")
        .arg("-n")
        .arg("-6")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take().unwrap().write_all(bytes).ok()?;
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then_some(output.stdout.len() as u64)
}

/// esbuild metafile keys are emitted relative to varying bases (the working
/// directory, the consumer directory); resolve them lexically and normalize to
/// paths relative to the generated package root.
fn package_relative(key: &str, package_root: &Path, consumer: &Path) -> String {
    let path = Path::new(key);
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(package_root) {
            return relative.to_string_lossy().replace('\\', "/");
        }
        if let Ok(canonical) = std::fs::canonicalize(package_root)
            && let Ok(relative) = path.strip_prefix(&canonical)
        {
            return relative.to_string_lossy().replace('\\', "/");
        }
    }
    for base in [consumer, package_root] {
        let mut normalized = PathBuf::new();
        for component in base.join(path).components() {
            match component {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::CurDir => {}
                component => normalized.push(component.as_os_str()),
            }
        }
        let text = normalized.to_string_lossy().to_string();
        if let Some(index) = text.find("typescript/") {
            return text[index..].to_owned();
        }
        if let Ok(relative) = normalized.strip_prefix(package_root) {
            return relative.to_string_lossy().replace('\\', "/");
        }
    }
    key.to_owned()
}

// ---------------------------------------------------------------------------
// Dependency-free perfect-shaker graph model
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq)]
enum Need {
    /// Every exported binding is needed (namespace imports).
    All,
    /// Exactly these local names are needed.
    Names(BTreeSet<String>),
    /// Only module evaluation is needed; no bindings.
    SideEffect,
}

#[derive(Clone)]
enum HardKind {
    /// `import * as ns from 'spec'`
    Namespace(String),
    /// `import { a, b as c } from 'spec'` (value bindings only).
    Named(Vec<String>),
    /// `import 'spec'`
    SideEffect,
}

#[derive(Clone)]
struct HardEdge {
    spec: String,
    kind: HardKind,
    /// Whether at least one imported binding occurs in the module body
    /// (approximating bundler binding-level shedding).
    used: bool,
}

#[derive(Clone)]
enum ReExportKind {
    /// `export { a as c, b } from 'spec'`: provides the aliases, requests the
    /// source names from the target.
    Named {
        provided: Vec<String>,
        requested: Vec<String>,
    },
    /// `export * as ns from 'spec'`
    Namespace(String),
    /// `export * from 'spec'`
    Star,
}

#[derive(Clone)]
struct ReExport {
    spec: String,
    kind: ReExportKind,
}

struct ParsedModule {
    hard: Vec<HardEdge>,
    reexports: Vec<ReExport>,
    /// Value exports declared in this module itself.
    local_exports: BTreeSet<String>,
}

fn module_specifier(statement: &str) -> Option<String> {
    let index = statement.find("from '")?;
    let rest = &statement[index + "from '".len()..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_owned())
}

fn brace_bindings(clause: &str) -> Vec<String> {
    let Some(open) = clause.find('{') else {
        return Vec::new();
    };
    let close = clause[open..].find('}').unwrap() + open;
    clause[open + 1..close]
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            // Inline `type` specifiers are erased at runtime.
            let value = match part.strip_prefix("type ") {
                Some(_) => return None,
                None => part,
            };
            let name = match value.split_once(" as ") {
                Some((_, alias)) => alias.trim(),
                None => value,
            };
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect()
}

fn parse_module(source: &str) -> ParsedModule {
    let mut parsed = ParsedModule {
        hard: Vec::new(),
        reexports: Vec::new(),
        local_exports: BTreeSet::new(),
    };
    for line in source.lines() {
        let trimmed = line.trim_start();
        let is_type = trimmed.starts_with("import type") || trimmed.starts_with("export type");
        let starts_import = trimmed.starts_with("import ");
        let starts_export = trimmed.starts_with("export ");
        if !starts_import && !starts_export {
            continue;
        }
        if is_type {
            continue;
        }
        if starts_import {
            if let Some(specifier) = module_specifier(trimmed) {
                let clause = trimmed.trim_start_matches("import").trim_start();
                let clause = &clause[..clause.find("from").unwrap()];
                let clause = clause.trim_end();
                let kind = if let Some(rest) = clause.strip_prefix("* as ") {
                    HardKind::Namespace(rest.trim().to_owned())
                } else if let Some(bindings) = clause.strip_prefix('{') {
                    HardKind::Named(brace_bindings(&format!("{{{bindings}")))
                } else {
                    // Default import: generated sources use none, but treat a
                    // bare default binding as a named need.
                    let name = clause.split_whitespace().next().unwrap_or("");
                    HardKind::Named(if !name.is_empty() {
                        vec![name.to_owned()]
                    } else {
                        Default::default()
                    })
                };
                let used = match &kind {
                    HardKind::Namespace(binding) => body_uses(source, line, binding),
                    HardKind::Named(bindings) => bindings
                        .iter()
                        .any(|binding| body_uses(source, line, binding)),
                    HardKind::SideEffect => true,
                };
                parsed.hard.push(HardEdge {
                    spec: specifier,
                    kind,
                    used,
                });
            } else if trimmed.starts_with("import '") || trimmed.starts_with("import \"") {
                let spec = trimmed["import ".len()..]
                    .trim_end_matches(';')
                    .trim()
                    .trim_matches(['\'', '"']);
                parsed.hard.push(HardEdge {
                    spec: spec.to_owned(),
                    kind: HardKind::SideEffect,
                    used: true,
                });
            }
            continue;
        }
        // export ... from
        if let Some(specifier) = module_specifier(trimmed) {
            let clause = &trimmed["export ".len()..];
            let clause = &clause[..clause.find("from").unwrap()];
            let clause = clause.trim_end();
            if let Some(rest) = clause.strip_prefix("* as ") {
                parsed.reexports.push(ReExport {
                    spec: specifier,
                    kind: ReExportKind::Namespace(rest.trim().to_owned()),
                });
            } else if clause == "*" {
                parsed.reexports.push(ReExport {
                    spec: specifier,
                    kind: ReExportKind::Star,
                });
            } else {
                let open = clause.find('{').unwrap();
                let close = clause[open..].find('}').unwrap() + open;
                let mut provided = Vec::new();
                let mut requested = Vec::new();
                for part in clause[open + 1..close].split(',') {
                    let part = part.trim();
                    if part.is_empty() || part.starts_with("type ") {
                        continue;
                    }
                    match part.split_once(" as ") {
                        Some((source_name, alias)) => {
                            provided.push(alias.trim().to_owned());
                            requested.push(source_name.trim().to_owned());
                        }
                        None => {
                            provided.push(part.to_owned());
                            requested.push(part.to_owned());
                        }
                    }
                }
                parsed.reexports.push(ReExport {
                    spec: specifier,
                    kind: ReExportKind::Named {
                        provided,
                        requested,
                    },
                });
            }
            continue;
        }
        // Local exports.
        let declaration = trimmed
            .strip_prefix("export const ")
            .or_else(|| trimmed.strip_prefix("export function "))
            .or_else(|| trimmed.strip_prefix("export async function* "))
            .or_else(|| trimmed.strip_prefix("export class "));
        if let Some(declaration) = declaration {
            let name: String = declaration
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            if !name.is_empty() {
                parsed.local_exports.insert(name);
            }
            continue;
        }
        if trimmed.starts_with("export {") {
            for binding in brace_bindings(trimmed) {
                parsed.local_exports.insert(binding);
            }
        }
    }
    parsed
}

fn body_uses(source: &str, import_line: &str, binding: &str) -> bool {
    let start = source.find(import_line).unwrap_or(0) + import_line.len();
    let body = &source[start..];
    let bytes = body.as_bytes();
    let pattern = binding.as_bytes();
    let mut index = 0;
    while let Some(position) = body[index..].find(binding) {
        let absolute = index + position;
        let before = bytes.get(absolute.wrapping_sub(1)).copied();
        let after = bytes.get(absolute + pattern.len()).copied();
        let boundary = |byte: Option<u8>| {
            byte.is_none_or(|byte| !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'))
        };
        if boundary(before) && boundary(after) {
            return true;
        }
        index = absolute + 1;
    }
    false
}

fn resolve_specifier(from: &str, specifier: &str) -> String {
    let base = Path::new(from).parent().unwrap();
    let mut normalized = PathBuf::new();
    for component in base.join(specifier).components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            component => normalized.push(component.as_os_str()),
        }
    }
    let mut text = normalized.to_string_lossy().to_string();
    if let Some(stem) = text.strip_suffix(".js") {
        text = format!("{stem}.ts");
    }
    text
}

/// Reachability over the emitted ESM graph, modeling a *perfect* tree shaker:
/// bindings propagate only along needed names, re-export edges only carry the
/// names actually requested, unused binding imports are pruned, and explicit
/// side-effect imports force evaluation-only reach. Returns the
/// binding-reachable and evaluation-reachable module sets.
fn graph_reach(
    files: &BTreeMap<String, String>,
    entry_source: &str,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let entry = parse_module(entry_source);
    let mut binding: BTreeSet<String> = BTreeSet::new();
    let mut evaluation: BTreeSet<String> = BTreeSet::new();
    // Work list of (module, need, bindings_only_from_side_effects).
    let mut queue: Vec<(String, Need, bool)> = Vec::new();
    let mut seen_names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut seen_all: BTreeSet<String> = BTreeSet::new();
    let mut seen_side_effect: BTreeSet<String> = BTreeSet::new();

    for edge in &entry.hard {
        let target = resolve_specifier("__entry__", &edge.spec);
        let need = match &edge.kind {
            HardKind::Namespace(_) => Need::All,
            HardKind::Named(bindings) => Need::Names(bindings.iter().cloned().collect()),
            HardKind::SideEffect => Need::SideEffect,
        };
        // Entry imports are as written; type-position-only usage is
        // undecidable here, so named entry needs are honored verbatim.
        queue.push((target, need, false));
    }

    while let Some((module, need, evaluation_only)) = queue.pop() {
        let Some(source) = files.get(&module) else {
            continue;
        };
        match &need {
            Need::SideEffect => {
                if !seen_side_effect.insert(module.clone()) {
                    continue;
                }
                evaluation.insert(module.clone());
            }
            Need::All => {
                if !seen_all.insert(module.clone()) {
                    continue;
                }
                binding.insert(module.clone());
            }
            Need::Names(names) => {
                let seen = seen_names.entry(module.clone()).or_default();
                let fresh: BTreeSet<_> = names.difference(seen).cloned().collect();
                if fresh.is_empty() {
                    continue;
                }
                seen.extend(fresh.iter().cloned());
                binding.insert(module.clone());
            }
        }
        let parsed = parse_module(source);
        let binding_reached = !matches!(need, Need::SideEffect) && !evaluation_only;
        if binding_reached {
            // This module's own code is live, so its imports are live too.
            for edge in &parsed.hard {
                let side_effect = matches!(edge.kind, HardKind::SideEffect);
                if !side_effect && !edge.used {
                    // Perfect model: an unused binding import from a pure
                    // module is shed.
                    continue;
                }
                let need = match &edge.kind {
                    HardKind::Namespace(_) => Need::All,
                    HardKind::Named(bindings) => Need::Names(bindings.iter().cloned().collect()),
                    HardKind::SideEffect => Need::SideEffect,
                };
                queue.push((resolve_specifier(&module, &edge.spec), need, false));
            }
        } else {
            // Evaluation-only reach propagates as evaluation-only.
            for edge in &parsed.hard {
                queue.push((
                    resolve_specifier(&module, &edge.spec),
                    Need::SideEffect,
                    true,
                ));
            }
        }
        // Re-exports satisfy needed names and evaluate their targets.
        for reexport in &parsed.reexports {
            let target = resolve_specifier(&module, &reexport.spec);
            if !binding_reached {
                queue.push((target, Need::SideEffect, true));
                continue;
            }
            match &reexport.kind {
                ReExportKind::Namespace(binding_name) => {
                    let needed = match &need {
                        Need::All => true,
                        Need::Names(names) => names.contains(binding_name),
                        Need::SideEffect => false,
                    };
                    if needed {
                        queue.push((target, Need::All, false));
                    } else {
                        // Named binding re-exports still evaluate their target
                        // module under ESM semantics: evaluation-only reach.
                        queue.push((target, Need::SideEffect, true));
                    }
                }
                ReExportKind::Named {
                    provided,
                    requested,
                } => {
                    let wanted: Vec<&String> = match &need {
                        Need::All => provided.iter().collect(),
                        Need::Names(names) => provided
                            .iter()
                            .filter(|name| names.contains(*name))
                            .collect(),
                        Need::SideEffect => Vec::new(),
                    };
                    if wanted.is_empty() {
                        queue.push((target, Need::SideEffect, true));
                    } else {
                        let mut names = BTreeSet::new();
                        for name in wanted {
                            if let Some(position) = provided.iter().position(|p| p == name) {
                                names.insert(requested[position].clone());
                            }
                        }
                        queue.push((target, Need::Names(names), false));
                    }
                }
                ReExportKind::Star => {
                    // A star re-export can supply any residual name.
                    let residual = match &need {
                        Need::All => true,
                        Need::Names(names) => !names.is_subset(&parsed.local_exports),
                        Need::SideEffect => false,
                    };
                    if residual {
                        queue.push((target, need.clone(), false));
                    } else {
                        queue.push((target, Need::SideEffect, true));
                    }
                }
            }
        }
    }
    (binding, evaluation)
}

// ---------------------------------------------------------------------------
// Generation-level launcher stability
// ---------------------------------------------------------------------------

/// Files that must be byte-identical when only an unrelated operation is
/// added: every runtime, transport shim, conditional module with identical
/// compiled policy, and package infrastructure file.
const INVARIANT_FILES: &[&str] = &[
    "typescript/runtime.ts",
    "typescript/http/types.ts",
    "typescript/http/common.ts",
    "typescript/http/wire.ts",
    "typescript/http/media.ts",
    "typescript/http/security.ts",
    "typescript/http/parts.ts",
    "typescript/http/multipart-style.ts",
    "typescript/http/streams.ts",
    "typescript/json.ts",
    "typescript/codecs.ts",
    "typescript/validation.ts",
    "typescript/pattern.ts",
    "typescript/validation-resources.ts",
    "typescript/uri.ts",
    "typescript/pagination.ts",
    "typescript/oauth.ts",
    "typescript/source/index.ts",
    "typescript/package.json",
    "typescript/package-lock.json",
    "typescript/tsconfig.json",
];

/// Files that may differ when only an unrelated operation is added: the
/// operations module and every per-contract model, validation-program,
/// documentation, manifest and example artifact. Anything outside this set
/// drifting is a launcher regression.
const VARIANT_FILES: &[&str] = &[
    "typescript/operations.ts",
    "typescript/models.ts",
    "typescript/model-codecs.ts",
    "typescript/validation-program.ts",
    "typescript/validation-program-request.ts",
    "typescript/validation-program-response.ts",
    "typescript/models.md",
    "typescript/codecs.md",
    "typescript/validation.md",
    "typescript/http.md",
    "typescript/docs-readme.md",
    "typescript/README.md",
    "typescript/docs-manifest.json",
    "typescript/http-manifest.json",
    "typescript/examples.json",
    "typescript/examples.md",
    "typescript/examples/validated.ts",
    "typescript/examples/first-request.ts",
    "typescript/typedoc.json",
    "typescript/tsconfig.docs.json",
];

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn launcher_size_is_stable_when_only_unrelated_operations_are_added() {
    let baseline = files_map(&generate(elimination_document()));
    let extended = files_map(&generate(document_with_unrelated_operation()));
    assert_eq!(
        baseline.len(),
        extended.len(),
        "adding one unrelated operation must not add or remove generated files"
    );

    let mut differing = Vec::new();
    for path in baseline.keys() {
        if baseline[path] != extended[path] {
            differing.push(path.clone());
        }
    }

    for path in INVARIANT_FILES {
        if let (Some(baseline), Some(extended)) = (baseline.get(*path), extended.get(*path)) {
            assert_eq!(
                baseline, extended,
                "shared launcher artifact {path} must be byte-identical when only unrelated operations are added"
            );
        }
    }

    for path in &differing {
        assert!(
            VARIANT_FILES.contains(&path.as_str()),
            "unexpected launcher drift in {path}; only the operations module and per-contract model, validation-program, documentation, manifest and example artifacts may differ"
        );
    }
    for required in [
        "typescript/operations.ts",
        "typescript/models.ts",
        "typescript/model-codecs.ts",
    ] {
        assert!(
            differing.iter().any(|path| path == required),
            "sanity: {required} must differ so this gate cannot pass vacuously"
        );
    }

    assert!(
        !file(&baseline, "typescript/operations.ts").contains("listGizmos"),
        "baseline operations module must not mention the unrelated operation"
    );
    assert!(
        file(&extended, "typescript/operations.ts").contains("listGizmos"),
        "extended operations module must contain the unrelated operation"
    );
    assert!(
        file(&baseline, "typescript/models.ts").contains("type Gadget ="),
        "sanity: baseline models module must contain the shared model"
    );
    assert!(
        !file(&baseline, "typescript/models.ts").contains("UnrelatedGizmo"),
        "baseline models module must not contain the unrelated model"
    );
    assert!(
        file(&extended, "typescript/models.ts").contains("UnrelatedGizmo"),
        "extended models module must contain the unrelated model"
    );
    // The conditional modules must not move at all: the unrelated operation is
    // neither paginated nor OAuth-protected nor a stream.
    assert_eq!(
        file(&baseline, "typescript/pagination.ts"),
        file(&extended, "typescript/pagination.ts"),
        "pagination walker module is shared infrastructure and must not change"
    );
    assert_eq!(
        file(&baseline, "typescript/oauth.ts"),
        file(&extended, "typescript/oauth.ts"),
        "OAuth descriptor module is shared infrastructure and must not change"
    );
}

// ---------------------------------------------------------------------------
// Consumer-profile gates
// ---------------------------------------------------------------------------

#[ignore = "requires esbuild on the test host for bundler byte gates"]
#[test]
fn consumer_profiles_eliminate_unrelated_surfaces() {
    let baseline_files = files_map(&generate(elimination_document()));
    let extended_files = files_map(&generate(document_with_unrelated_operation()));

    let esbuild = Esbuild::pinned();
    let bundler_mode = esbuild.is_some();
    let gate = match &esbuild {
        Some(esbuild) => format!("bundler (esbuild {})", esbuild.version),
        None => {
            "graph (dependency-free perfect-shaker import graph; bundler byte gates CI-pending)"
                .to_owned()
        }
    };
    eprintln!("elimination gate mode: {gate}");

    let directory = tempfile::tempdir().unwrap();
    let baseline_root = directory.path().join("baseline");
    let extended_root = directory.path().join("extended");
    suspect_codegen::write_files(&generate(elimination_document()), &baseline_root).unwrap();
    suspect_codegen::write_files(
        &generate(document_with_unrelated_operation()),
        &extended_root,
    )
    .unwrap();
    // Consumer entries import '../typescript/*.js', so each consumer directory
    // sits beside the generation it measures.
    let baseline_consumer = baseline_root.join("consumer");
    let extended_consumer = extended_root.join("consumer");
    std::fs::create_dir(&baseline_consumer).unwrap();
    std::fs::create_dir(&extended_consumer).unwrap();

    let mut report = json!({
        "gate": gate,
        "esbuild": esbuild.as_ref().map(|esbuild| esbuild.version.clone()),
        "profiles": [],
    });
    let mut sizes: BTreeMap<String, (u64, u64)> = BTreeMap::new();

    for name in PROFILES {
        let profile = profile(name);
        let baseline_graph = graph_reach(&baseline_files, profile.entry);
        let extended_graph = graph_reach(&extended_files, profile.entry);
        let (baseline_bundle, extended_bundle) = match &esbuild {
            Some(esbuild) => (
                Some(bundle_with(
                    esbuild,
                    &baseline_root,
                    &baseline_consumer,
                    &profile,
                    "base",
                )),
                Some(bundle_with(
                    esbuild,
                    &extended_root,
                    &extended_consumer,
                    &profile,
                    "ext",
                )),
            ),
            None => (None, None),
        };

        if let (Some(baseline), Some(extended)) = (&baseline_bundle, &extended_bundle) {
            for (tag, bundle) in [("base", baseline), ("ext", extended)] {
                for marker in profile.present {
                    assert!(
                        bundle.text.contains(marker),
                        "[{}/{tag}] retained-marker missing: {marker:?} must appear in the {tag} bundle",
                        profile.name
                    );
                }
                for marker in profile.absent {
                    assert!(
                        !bundle.text.contains(marker),
                        "[{}/{tag}] tree-shaking defeat: {marker:?} is retained in the {tag} bundle ({} bytes)",
                        profile.name,
                        bundle.bytes
                    );
                }
            }
            // The plan's additivity property: a single-operation consumer
            // artifact must not retain the unrelated operation. Profiles that
            // retain every operation by design (`createClient`, the
            // all-operations namespace) are excluded.
            if profile.additivity == Additivity::Invariant {
                for (tag, bundle) in [("base", baseline), ("ext", extended)] {
                    for marker in MARKER_UNRELATED_OPERATION {
                        assert!(
                            !bundle.text.contains(marker),
                            "[{}/{tag}] additivity defeat: the unrelated operation's marker {marker:?} is retained in the single-operation consumer bundle",
                            profile.name
                        );
                    }
                }
            }
            // Resolved module graph gates from the bundler metafile.
            for module in profile.included_modules {
                assert!(
                    baseline.modules.contains(*module),
                    "[{}] bundler resolved graph lacks required module {module}; metafile has {:?}",
                    profile.name,
                    baseline.modules
                );
            }
            for module in profile.excluded_modules {
                assert!(
                    !baseline.modules.contains(*module),
                    "[{}] bundler resolved graph retains excluded module {module}",
                    profile.name
                );
            }
        }

        // Graph-model gate: module-level reachability of a perfect tree
        // shaker. Asserted whenever the profile is decidable, in both modes;
        // in bundler mode it additionally documents where reality diverges.
        if let Graph::Decidable {
            reachable,
            unreachable,
        } = &profile.graph
        {
            for module in reachable.iter() {
                assert!(
                    baseline_graph.0.contains(*module),
                    "[{}] graph model lacks required binding reach for {module}",
                    profile.name
                );
            }
            for module in unreachable.iter() {
                assert!(
                    !baseline_graph.0.contains(*module),
                    "[{}] graph model retains binding reach for {module}",
                    profile.name
                );
            }
        }

        match (&baseline_bundle, &extended_bundle) {
            (Some(baseline), Some(extended)) => match profile.additivity {
                Additivity::Invariant => {
                    let delta = extended.bytes.abs_diff(baseline.bytes);
                    assert!(
                        delta <= ADDITIVITY_SLACK_BYTES,
                        "[{}] additivity: adding an unrelated operation moved the single-operation artifact by {delta} bytes ({} -> {})",
                        profile.name,
                        baseline.bytes,
                        extended.bytes
                    );
                }
                Additivity::Grows => {
                    assert!(
                        extended.bytes > baseline.bytes,
                        "[{}] sanity: the all-operations surface must retain the unrelated operation",
                        profile.name
                    );
                }
            },
            _ => {
                assert!(
                    matches!(&profile.graph, Graph::Decidable { .. }),
                    "[{}] no bundler and no decidable graph gate: nothing gates this profile",
                    profile.name
                );
                eprintln!(
                    "{:<24} graph gate only; bundler byte gate CI-pending",
                    profile.name
                );
            }
        }

        if let (Some(baseline), Some(extended)) = (&baseline_bundle, &extended_bundle) {
            eprintln!(
                "{:<24} base {:>6} B (gzip {})  ext {:>6} B (gzip {})  modules {}",
                profile.name,
                baseline.bytes,
                baseline
                    .gzip_bytes
                    .map_or_else(|| "n/a".to_owned(), |bytes| format!("{bytes} B")),
                extended.bytes,
                extended
                    .gzip_bytes
                    .map_or_else(|| "n/a".to_owned(), |bytes| format!("{bytes} B")),
                baseline.modules.len(),
            );
            sizes.insert(profile.name.to_owned(), (baseline.bytes, extended.bytes));
        }
        let mut profile_report = json!({
            "name": profile.name,
            "additivity": if profile.additivity == Additivity::Invariant { "invariant" } else { "grows" },
            "bytesBaseline": baseline_bundle.as_ref().map(|bundle| bundle.bytes),
            "bytesExtended": extended_bundle.as_ref().map(|bundle| bundle.bytes),
            "gzipBaseline": baseline_bundle.as_ref().and_then(|bundle| bundle.gzip_bytes),
            "gzipExtended": extended_bundle.as_ref().and_then(|bundle| bundle.gzip_bytes),
            "graphModelBindingsBaseline": baseline_graph.0,
            "graphModelEvaluationBaseline": baseline_graph.1,
            "graphModelBindingsExtended": extended_graph.0,
            "graphModelEvaluationExtended": extended_graph.1,
            "unrelatedOperationMarkers": MARKER_UNRELATED_OPERATION,
        });
        if let (Some(baseline), Some(extended)) = (&baseline_bundle, &extended_bundle) {
            profile_report["modulesBaseline"] = json!(baseline.modules);
            profile_report["modulesExtended"] = json!(extended.modules);
        }
        report["profiles"]
            .as_array_mut()
            .unwrap()
            .push(profile_report);
    }

    if bundler_mode {
        // Partial elimination must actually shrink artifacts relative to the
        // all-operations baseline.
        let (all_base, all_ext) = sizes["all-operations"];
        for name in [
            "one-json-operation",
            "one-pager",
            "one-stream",
            "one-oauth-provider",
            "one-codec",
            "empty",
            "type-only",
        ] {
            let (base, ext) = sizes[name];
            assert!(
                base < all_base && ext < all_ext,
                "[{name}] single-surface artifact must be strictly smaller than the all-operations artifact ({base}/{ext} vs {all_base}/{all_ext})"
            );
        }
        let (empty_base, _) = sizes["empty"];
        for name in [
            "core-client-operations",
            "core-client-root",
            "one-json-operation",
            "one-pager",
            "one-stream",
            "one-oauth-provider",
            "one-codec",
        ] {
            let (base, _) = sizes[name];
            assert!(
                base > empty_base,
                "[{name}] a profile with real surface must exceed the empty import ({base} vs {empty_base})"
            );
        }
        // The additivity declaration and the per-profile table must agree.
        for name in ADDITIVE_PROFILES {
            assert_eq!(
                profile(name).additivity,
                Additivity::Invariant,
                "[{name}] declared additive but profile table disagrees"
            );
        }
    }

    let report_path =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("typescript-elimination-report.json");
    std::fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    eprintln!("report: {}", report_path.display());
}
