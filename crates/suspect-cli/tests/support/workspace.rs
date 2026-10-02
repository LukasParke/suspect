//! A realistic OpenAPI workspace to drive the language server against.
//!
//! Small fixtures are the second reason the server's deadlocks survived: a
//! thirty-operation document finishes linting before the index warm-up can
//! queue a writer behind it, so the bug never appears. This builds a project
//! with the shapes that make real specifications slow and real editors
//! interesting:
//!
//! - enough operations that the lint pass outlives an index build,
//! - `$ref`s *across files*, so document-to-document navigation is exercised,
//! - `oneOf`/`allOf`/`not`, enums, nullable unions and examples,
//! - an overlay document and an Arazzo workflow, which are separate specs.

use std::path::{Path, PathBuf};

/// A generated workspace: enough shape to be slow, enough depth to be real.
pub struct Workspace {
    pub root: PathBuf,
    pub openapi: PathBuf,
    pub schemas: PathBuf,
    pub overlay: PathBuf,
    pub workflow: PathBuf,
    pub manifest: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
}

const RESOURCES: &[&str] = &[
    "accounts",
    "activity",
    "analytics",
    "channels",
    "clients",
    "devices",
    "discover",
    "dvr",
    "history",
    "home",
    "identity",
    "library",
    "machine",
    "media",
    "metadata",
    "playback",
    "playlist",
    "preferences",
    "providers",
    "rating",
    "recommendations",
    "search",
    "sessions",
    "settings",
    "statistics",
    "status",
    "sync",
    "transcode",
    "timeline",
    "users",
    "vpn",
    "webhooks",
];

const VERBS: &[&str] = &["get", "post", "put", "delete", "patch"];

/// How many operations to generate. Enough that the lint pass measurably
/// outlives the index warm-up — the interleaving that used to wedge.
const OPERATIONS: usize = 60;

impl Workspace {
    /// Writes the project into `root`, which must already exist.
    #[must_use]
    pub fn build(root: &Path) -> Self {
        let schemas = root.join("schemas.yaml");
        let openapi = root.join("openapi.yaml");
        let overlay = root.join("overlays").join("internal.overlay.yaml");
        let workflow = root.join("workflows").join("health.arazzo.yaml");
        let config = root.join(".suspect.yaml");
        let manifest = root.join("suspect.project.json");
        let data = root.join("catalog.json");
        let _ = (&manifest, &data);
        let manifest = root.join("suspect.project.json");
        let data = root.join("catalog.json");
        let _ = (&manifest, &data);

        std::fs::create_dir_all(root.join("overlays")).expect("overlays dir");
        std::fs::create_dir_all(root.join("workflows")).expect("workflows dir");
        std::fs::write(&schemas, schemas_document()).expect("schemas");
        std::fs::write(&openapi, openapi_document()).expect("openapi");
        std::fs::write(&overlay, overlay_document()).expect("overlay");
        std::fs::write(&workflow, workflow_document()).expect("workflow");
        std::fs::write(&config, config_document()).expect("config");
        std::fs::write(&manifest, manifest_document()).expect("manifest");
        std::fs::write(&data, data_document()).expect("data");

        Self {
            root: root.to_path_buf(),
            openapi,
            schemas,
            overlay,
            workflow,
            manifest,
            data,
            config,
        }
    }

    /// A workspace description pointing at a real project on disk.
    #[must_use]
    pub fn describing(root: &Path, openapi: PathBuf) -> Self {
        Self {
            root: root.to_path_buf(),
            schemas: root.join("schemas.yaml"),
            overlay: root.join("overlays/internal.overlay.yaml"),
            workflow: root.join("workflows/health.arazzo.yaml"),
            manifest: root.join("suspect.project.json"),
            data: root.join("catalog.json"),
            config: root.join(".suspect.yaml"),
            openapi,
        }
    }

    /// Every file in the workspace, as (path, text).
    #[must_use]
    pub fn files(&self) -> Vec<(PathBuf, String)> {
        [
            self.openapi.clone(),
            self.schemas.clone(),
            self.overlay.clone(),
            self.workflow.clone(),
        ]
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("read");
            (p, text)
        })
        .collect()
    }

    /// The 1-based line and column of the first occurrence of `needle`.
    #[must_use]
    pub fn locate(&self, path: &Path, needle: &str) -> (usize, usize) {
        let text = std::fs::read_to_string(path).expect("read");
        let offset = text
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` is not in the document"));
        let line = text[..offset].lines().count().max(1);
        let column = offset - text[..offset].rfind('\n').map_or(0, |n| n + 1);
        (line, column + 1)
    }
}

fn schemas_document() -> String {
    let mut out = String::from(
        "openapi: 3.1.0\ninfo:\n  title: Shared\n  version: '1.0.0'\npaths: {}\ncomponents:\n  parameters:\n    traceToken:\n      name: X-Trace-Token\n      in: header\n      required: false\n      schema:\n        type: string\n  responses:\n    notFound:\n      description: No such resource\n      content:\n        application/json:\n          schema:\n            $ref: '#/components/schemas/Problem'\n  schemas:\n",
    );
    for (index, name) in RESOURCES.iter().enumerate() {
        out.push_str(&format!(
            "    {Pascal}:\n      type: object\n      required: [id]\n      properties:\n        id:\n          type: string\n          format: uuid\n        name:\n          type: string\n          maxLength: 120\n        kind:\n          type: string\n          enum: [{kind_a}, {kind_b}]\n        score:\n          oneOf:\n            - type: number\n            - type: 'null'\n        tags:\n          type: array\n          items:\n            type: string\n        nested:\n          allOf:\n            - $ref: '#/components/schemas/Meta'\n            - type: object\n              properties:\n                index:\n                  type: integer\n                  minimum: {index}\n      example:\n        id: 2f1c0000-0000-4000-8000-000000000001\n        name: example {name}\n",
            Pascal = pascal(name),
            kind_a = if index % 2 == 0 { "primary" } else { "secondary" },
            kind_b = if index % 2 == 0 { "mirror" } else { "edge" },
        ));
    }
    out.push_str(
        "    Meta:\n      type: object\n      properties:\n        createdAt:\n          type: string\n          format: date-time\n        etag:\n          type: string\n    Problem:\n      type: object\n      required: [title]\n      properties:\n        title:\n          type: string\n        detail:\n          type: string\n    ListMeta:\n      type: object\n      properties:\n        total:\n          type: integer\n    # A component nothing references: still a valid definition target.\n    Unreferenced:\n      type: object\n      description: Deliberately orphaned\n",
    );
    out
}

fn openapi_document() -> String {
    let mut out = String::from(
        "openapi: 3.1.0\ninfo:\n  title: Session API\n  version: '2.4.0'\n  description: Every server in one place.\nservers:\n  - url: https://plex.example\n    description: production\n  - url: http://localhost:32400\n    description: local\ntags:\n  - name: accounts\n    description: sign-in and identity\n  - name: library\n    description: what is playing\nsecurity:\n  - tokenAuth: []\npaths:\n",
    );
    for (path_index, resource) in RESOURCES.iter().enumerate() {
        out.push_str(&format!("  /{}:\n", plural(resource)));
        for (verb_index, verb) in VERBS.iter().copied().enumerate() {
            if (path_index + verb_index) % 7 == 6 {
                continue; // not every verb on every resource
            }
            let op = format!("{}{}", pascal(resource), pascal(verb));
            out.push_str(&format!("    {verb}:\n"));
            out.push_str(&format!("      operationId: {op}\n"));
            out.push_str(&format!(
                "      summary: {verb} the {resource} collection\n"
            ));
            out.push_str(&format!("      tags: [{resource}]\n"));
            if verb == "get" {
                out.push_str(
                    "      parameters:\n        - name: limit\n          in: query\n          required: false\n          schema:\n            type: integer\n            minimum: 1\n            maximum: 500\n        - $ref: '#/components/parameters/traceToken'\n",
                );
            }
            if verb != "get" {
                out.push_str(
                    "      requestBody:\n        required: true\n        content:\n          application/json:\n            schema:\n              $ref: 'schemas.yaml#/components/schemas/",
                );
                out.push_str(&format!("{}'\n", pascal(resource)));
                out.push_str(
                    "          application/x-www-form-urlencoded:\n            schema:\n              type: object\n              properties:\n                force:\n                  type: boolean\n      responses:\n        '200':\n          description: ok\n          headers:\n            X-Rate-Limit:\n              schema:\n                type: integer\n          content:\n            application/json:\n              schema:\n                $ref: 'schemas.yaml#/components/schemas/",
                );
                out.push_str(&format!("{}'\n", pascal(resource)));
                out.push_str("        '404':\n          $ref: '#/components/responses/notFound'\n");
            } else {
                out.push_str(
                    "      responses:\n        '200':\n          description: a page of results\n          content:\n            application/json:\n              schema:\n                type: object\n                required: [data]\n                properties:\n                  data:\n                    type: array\n                    items:\n                      $ref: 'schemas.yaml#/components/schemas/",
                );
                out.push_str(&format!("{}'\n", pascal(resource)));
                out.push_str(
                    "                  meta:\n                    $ref: 'schemas.yaml#/components/schemas/ListMeta'\n        '404':\n          $ref: '#/components/responses/notFound'\n",
                );
            }
        }
    }
    out.push_str(&format!(
        "components:\n  securitySchemes:\n    tokenAuth:\n      type: http\n      scheme: bearer\n  parameters:\n    traceToken:\n      $ref: 'schemas.yaml#/components/parameters/traceToken'\n  responses:\n    notFound:\n      $ref: 'schemas.yaml#/components/responses/notFound'\n  schemas:\n    {pascal}:\n      $ref: 'schemas.yaml#/components/schemas/{pascal}'\n",
        pascal = pascal(RESOURCES[0])
    ));
    let _ = OPERATIONS;
    out
}

fn overlay_document() -> String {
    String::from(
        "overlay: 1.0.0\ninfo:\n  title: Hide the internals\n  version: '1.0.0'\nextends: openapi.yaml\nactions:\n  - target: '$.info'\n    update:\n      description: Public surface only.\n  - target: '$.paths.*.*.responses.2XX'\n    remove: true\n",
    )
}

fn workflow_document() -> String {
    String::from(
        "arazzo: 1.1.0\ninfo:\n  title: Health check\n  version: '1.0.0'\nsourceDescriptions:\n  local:\n    type: openapi\n    url: http://localhost:32400/openapi.json\nworkflows:\n  - workflowId: health\n    summary: Is the server up?\n    inputs:\n      - name: token\n        type: string\n    steps:\n      - stepId: ping\n        operationId: getStatus\n        parameters:\n          - name: Authorization\n            in: header\n            value: $inputs.token\n      - stepId: confirm\n        operationId: getStatus\n        dependsOn: [ping]\n    successCriteria:\n      - condition: $statusCode == 200\n",
    )
}

fn config_document() -> String {
    String::from(
        "lint:\n  min_severity: warning\nvalidate:\n  strict_format: true\nformat:\n  yaml: true\ndocs:\n  style: sveltekit\n",
    )
}

/// English-ish pluralisation for generated path segments: `accounts` →
/// `accounts`, `history` → `histories`.
fn plural(name: &str) -> String {
    if let Some(stem) = name.strip_suffix('y')
        && !stem.ends_with(['a', 'e', 'i', 'o', 'u'])
    {
        return format!("{stem}ies");
    }
    if ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|ending| name.ends_with(ending))
    {
        return format!("{name}es");
    }
    format!("{name}s")
}

/// A project manifest, so the JSON feature set and the configuration
/// schema both have something real to work on.
fn manifest_document() -> String {
    let mut out = String::from(
        "{\n  \"version\": 1,\n  \"name\": \"session-api\",\n  \"entry\": \"openapi.yaml\",\n  \"overlays\": [\"overlays/internal.overlay.yaml\"],\n  \"publish\": {\n    \"output\": \".suspect/spec.yaml\",\n    \"profiles\": {\n      \"public\": [\"overlays/internal.overlay.yaml\"]\n    }\n  },\n  \"contract\": {\"output\": \".suspect/contract\"},\n  \"lint\": {\"min_severity\": \"warning\"},\n  \"docs\": {\"style\": \"sveltekit\", \"output\": \".suspect/docs\"},\n  \"codegen\": [\n",
    );
    for (language, profile, package) in [
        ("typescript", "typescript-http", "@session/api"),
        ("python", "python-http", "session-api"),
        ("go", "go-http", "github.com/session/api"),
        ("rust", "rust-http", "session-api"),
    ] {
        out.push_str(&format!(
            "    {{\n      \"name\": \"{language}\",\n      \"profile\": \"{profile}\",\n      \"package_name\": \"{package}\",\n      \"package_version\": \"1.0.0\",\n      \"out\": \".suspect/sdk/{language}\",\n      \"operation_id\": [\"AccountsGet\", \"AccountsPut\"]\n    }}{}\n",
            if language == "rust" { "" } else { "," }
        ));
    }
    out.push_str(
        "  ],\n  \"tests\": {\n    \"arazzo\": [\"workflows/health.arazzo.yaml\"],\n    \"base_url\": \"http://localhost:32400\"\n  }\n}\n",
    );
    out
}

/// A large JSON document: package manifests, lockfiles and machine-generated
/// data all sit in the same editor session as the specification, and JSON
/// highlighting has to keep up with them.
fn data_document() -> String {
    let mut out =
        String::from("{\n  \"generated\": \"fixture\",\n  \"version\": \"1\",\n  \"records\": [\n");
    for index in 0..600 {
        out.push_str(&format!(
            "    {{\"id\": {index}, \"kind\": \"{}\", \"active\": {}, \"score\": {}, \"label\": \"record-{index}\", \"tags\": [\"a\", \"b\"]}}{}\n",
            if index % 3 == 0 { "primary" } else { "secondary" },
            index % 2 == 0,
            index as f64 / 3.0,
            if index == 599 { "" } else { "," }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

fn pascal(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in name.chars() {
        if ch == '_' || ch == '-' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Total operations written into the generated document, for test asserts.
#[must_use]
pub fn operation_count() -> usize {
    RESOURCES
        .iter()
        .enumerate()
        .map(|(path_index, _)| {
            VERBS
                .iter()
                .enumerate()
                .filter(|(verb_index, _)| (path_index + verb_index) % 7 != 6)
                .count()
        })
        .sum()
}
