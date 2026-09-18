//! Installable, private ESM packages for the implemented model codec views.
//!
//! Packaging preserves the plan's symbols, sources and documentation, and
//! derives the documented model views from the plan's actual symbols. It does
//! not add HTTP support or certify an SDK release.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::Serialize;
use serde_json::{Value, json};

use super::ModelView;
use super::codecs::CodecPlan;
use super::http::HttpPlan;
use crate::OutFile;

const TYPESCRIPT_VERSION: &str = "5.9.3";
const NPM_VERSION: &str = "10.9.8";
const NODE_VERSION: &str = "22.23.1";

/// Identity of a generated private npm package. Registry availability is not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    /// Lowercase portable npm identity, optionally `@scope/name`, at most 214 bytes.
    /// Each part starts with an ASCII letter/digit and then uses letters, digits,
    /// `.`, `_` or `-`. Unscoped reserved and Node core-module names are rejected.
    pub name: String,
    /// Exact SemVer, including optional prerelease/build identifiers. Ranges,
    /// prefixes, whitespace and unsafe core numeric components are rejected.
    pub version: String,
}

/// A package cannot be emitted with invalid identity or conflicting artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    /// The name is outside the documented portable npm identity subset.
    InvalidName,
    /// The version is not an exact npm-compatible SemVer.
    InvalidVersion,
    /// A package-owned path already occurs in the underlying codec plan.
    ArtifactCollision(String),
}

impl fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => formatter.write_str("invalid portable npm package name"),
            Self::InvalidVersion => formatter.write_str("invalid exact npm package version"),
            Self::ArtifactCollision(path) => {
                write!(formatter, "package artifact collision: {path}")
            }
        }
    }
}

impl std::error::Error for PackageError {}

/// Add a private npm package around an already admitted codec plan.
///
/// The package root is `typescript/`. Build with the pinned lock using `npm ci`
/// and `npm run build`; `npm pack --ignore-scripts` then packages the compiled
/// ESM, declarations, original sources and source-linked Markdown documentation.
/// No lifecycle hook builds or publishes implicitly. Runtime dependencies are empty.
///
/// # Errors
/// Invalid npm identity/version or a collision with package-owned artifact paths.
pub fn emit(plan: &CodecPlan, config: &PackageConfig) -> Result<Vec<OutFile>, PackageError> {
    emit_package(plan.render(), plan, config, None)
}

/// Add a private installable ESM package around an admitted HTTP operation plan.
///
/// The package retains the complete model, codec and operation artifacts, with
/// an `operations` namespace and `/operations` subpath. HTTP support is limited
/// to the profile proved by the plan; package emission does not promote an SDK.
///
/// # Errors
/// Invalid npm identity/version or a collision with package-owned artifact paths.
pub fn emit_http(plan: &HttpPlan, config: &PackageConfig) -> Result<Vec<OutFile>, PackageError> {
    emit_package(plan.render(), plan.codecs(), config, Some(plan))
}

fn emit_package(
    artifacts: Vec<OutFile>,
    plan: &CodecPlan,
    config: &PackageConfig,
    http: Option<&HttpPlan>,
) -> Result<Vec<OutFile>, PackageError> {
    if !valid_name(&config.name) {
        return Err(PackageError::InvalidName);
    }
    if !valid_version(&config.version) {
        return Err(PackageError::InvalidVersion);
    }
    let label = view_label(plan);
    let mut files: BTreeMap<_, _> = artifacts
        .into_iter()
        .map(|file| (file.path.clone(), file))
        .collect();
    let mut packed_files: Vec<_> = files
        .keys()
        .map(|path| {
            path.strip_prefix("typescript/")
                .expect("codec plan artifacts share the TypeScript root")
                .to_owned()
        })
        .chain(["dist", "source/index.ts", "README.md", "tsconfig.json"].map(str::to_owned))
        .collect();
    packed_files.sort();
    let mut exports = BTreeMap::from([
        (".", entry("source/index")),
        ("./models", entry("models")),
        ("./codecs", entry("model-codecs")),
        ("./json", entry("json")),
        ("./package.json", Export::Metadata("./package.json")),
    ]);
    if http.is_some() {
        exports.insert("./operations", entry("operations"));
    }
    #[cfg(feature = "http-protocol")]
    if http.is_some_and(crate::typescript::http::HttpPlan::oauth_emitted) {
        exports.insert("./oauth", entry("oauth"));
    }
    let package = json!({
        "name": config.name,
        "version": config.version,
        "private": true,
        "description": if http.is_some() { "OpenAPI HTTP protocol operations, validated native models and source-linked documentation" } else { "Prototype OpenAPI neutral model types, validated codecs and source-linked documentation" },
        "type": "module",
        "types": "./dist/source/index.d.ts",
        "files": packed_files,
        "engines": {"node": ">=22"},
        "packageManager": format!("npm@{NPM_VERSION}"),
        "scripts": {"build": "tsc --project tsconfig.json"},
        "devDependencies": {"typescript": TYPESCRIPT_VERSION},
        "suspect": {
            "kind": if http.is_some() { "http-client" } else { "model-codecs" }, "status": "prototype", "modelView": label,
            "releaseReady": false, "httpClient": http.is_some(),
            "httpManifest": http.map(|_| "http-manifest.json"),
            "unsupportedFeatures": "rejected during codec planning",
            "documentation": "docs-manifest.json",
            "toolchain": {"node": NODE_VERSION, "npm": NPM_VERSION, "typescript": TYPESCRIPT_VERSION},
        },
    });
    // Reuse the exact registry resolution and integrity from the reviewed native
    // documentation toolchain. Only TypeScript is needed to build this package.
    let reviewed_lock: Value = serde_json::from_str(include_str!(
        "../../tools/typescript-docs/package-lock.json"
    ))
    .expect("reviewed toolchain lock is JSON");
    let typescript = reviewed_lock["packages"]["node_modules/typescript"].clone();
    assert_eq!(typescript["version"], TYPESCRIPT_VERSION);
    let lock = json!({
        "name": config.name, "version": config.version,
        "lockfileVersion": 3, "requires": true,
        "packages": {
            "": {"name": config.name, "version": config.version,
                "devDependencies": {"typescript": TYPESCRIPT_VERSION}, "engines": {"node": ">=22"}},
            "node_modules/typescript": typescript,
        },
    });
    let build = json!({
        "compilerOptions": {
            "target": "ES2022", "module": "NodeNext", "moduleResolution": "NodeNext",
            "strict": true, "exactOptionalPropertyTypes": true, "noUncheckedIndexedAccess": true,
            "declaration": true, "declarationMap": true, "sourceMap": true,
            "rootDir": ".", "outDir": "dist", "noEmitOnError": true,
            "forceConsistentCasingInFileNames": true,
        },
        "files": if http.is_some() {vec!["source/index.ts", "examples/validated.ts", "examples/first-request.ts"]} else {vec!["source/index.ts"]},
    });
    for (path, content) in [
        (
            "package.json",
            json_text(&Manifest {
                metadata: &package,
                exports,
            }),
        ),
        ("package-lock.json", json_text(&lock)),
        ("tsconfig.json", json_text(&build)),
        ("source/index.ts", {
            let index = if label == "Neutral" {
                INDEX
            } else {
                INDEX_VIEWS
            };
            if http.is_some() {
                #[cfg(feature = "http-protocol")]
                let oauth = if http.is_some_and(crate::typescript::http::HttpPlan::oauth_emitted) {
                    "\nexport * as oauth from '../oauth.js';\n"
                } else {
                    ""
                };
                #[cfg(not(feature = "http-protocol"))]
                let oauth = "";
                format!(
                    "{index}\nexport * as operations from '../operations.js';\nexport {{ createClient }} from '../operations.js';\n{oauth}"
                )
            } else {
                index.into()
            }
        }),
        (
            "README.md",
            http.map_or_else(|| readme(plan, config), |http| http_readme(http, config)),
        ),
    ] {
        let path = format!("typescript/{path}");
        if files.contains_key(&path) {
            return Err(PackageError::ArtifactCollision(path));
        }
        files.insert(path.clone(), OutFile { path, content });
    }
    if http.is_some() {
        files.get_mut("typescript/README.md").expect("package guide").content.push_str("\n## Validated examples\n\n[Example values and provenance](examples.md) retain declared and explicitly synthesized origins. `examples.json` includes source-linked findings for invalid or unavailable examples. `npm run build` compiles the typed inputs; `node dist/examples/validated.js` executes their model-codec checks without making HTTP calls.\n");
    }
    #[cfg(feature = "http-protocol")]
    if let Some(policy) = http.and_then(HttpPlan::credential_env) {
        files
            .get_mut("typescript/README.md")
            .expect("package guide")
            .content
            .push_str(&super::http::credential_env_documentation(policy));
    }
    if plan.validation_profile().0 == suspect_schema::OwnedProgram::V2_VERSION {
        files
            .get_mut("typescript/README.md")
            .expect("package guide")
            .content
            .push_str(super::validation::SCOPED_DOCUMENTATION);
    }
    if plan.validation_profile().0 == suspect_schema::OwnedProgram::V3_VERSION {
        files
            .get_mut("typescript/README.md")
            .expect("package guide")
            .content
            .push_str(super::validation::RESOURCE_DOCUMENTATION);
    }
    Ok(files.into_values().collect())
}

fn http_readme(plan: &HttpPlan, config: &PackageConfig) -> String {
    #[cfg(feature = "http-protocol")]
    let example = plan.first_request().map_or_else(
        || "No operations were selected.\n".into(),
        |recipe| format!("```ts\n{}```\n", recipe.source(&config.name)),
    );
    #[cfg(not(feature = "http-protocol"))]
    let example = plan.operations().first().map_or_else(
        || "No operations were selected.\n".into(),
        |operation| format!(
            "```ts\nimport {{ operations }} from '{}';\n\nexport function callOperation(client: operations.ClientOptions, input: operations.{}): Promise<operations.{}> {{\n  return operations.{}(client, input);\n}}\n```\n",
            config.name, operation.input_type, operation.success_type, operation.function_name,
        ),
    );
    let label = view_label(plan.codecs());
    let views = if label == "Neutral" {
        "`http-manifest.json` records `directionPolicy` as `neutral-equivalent`: the selected closure declares no directional annotations, so one neutral validation program serves both request and response directions and model names stay neutral.".to_owned()
    } else {
        format!(
            "The package implements the {label} model views: operation inputs bind Request-view models and declared success bodies bind Response-view models, and `models` plus `codecs` export every view's symbols and validated codecs. `http-manifest.json` records `directionPolicy` as `oas31-required-applicability-v1`: an explicit OpenAPI 3.1 context requiredness policy under which only the required presence proven directional for the view is relaxed. Supplied read-only and write-only values stay present and are validated; nothing is stripped, defaulted or reordered. This is not unmodified neutral validation and not OpenAPI 3.0 normative behavior. Directional annotations whose applicability cannot be proven are a source-linked planning error, not a silent fallback."
        )
    };
    format!(
        "# {}\n\nOpenAPI HTTP operations with validated native model codecs. The exact admitted capability profile, source operations and wire descriptors are recorded in [http-manifest.json](http-manifest.json). This package does not claim blanket OpenAPI protocol coverage.\n\n## First request\n\n{example}\nSupply explicit client policy as documented in [http.md](http.md). Operations with no required inputs can be called without an empty object; anonymous clients need no options. Ordinary values are native object literals, strings, booleans, bigint integers and exact JsonNumber decimals. This recipe is compiled in `examples/first-request.ts`.\n\nThe operation returns its declared success union; declared API errors have operation-specific guards. Pass cancellation through call options. SSE and JSON Lines expose native AsyncIterable items: consume them, break/return the iterator, or abort the call to release its reader. JSON-looking SSE data and [DONE] remain strings. Byte bodies and multipart files use finite in-memory Uint8Array data.\n\nCredentials are caller-supplied, redirects are errors, and requests are not retried. Required response headers and declared links are available as typed data/metadata. `mediaType` identifies the matched declaration for response narrowing; wildcard request bodies explicitly name their source range. Schema defaults are never injected. Request, response, part and streaming budgets are finite and caller limits can only reduce the generated ceilings. Native Fetch platform restrictions, including browser Cookie and TRACE limitations, are documented in [http.md](http.md).\n\n## Model views\n\n{views}\n\nThe package root exports `createClient`, the `operations`, `models` and `codecs` namespaces, and exact JSON support. The `/operations`, `/models`, `/codecs` and `/json` subpaths preserve module identities. TypeScript consumers should enable `strict`, `exactOptionalPropertyTypes` and `noUncheckedIndexedAccess`. JavaScript uses the same ESM exports.\n\n## Documentation\n\n[HTTP operations](http.md), [models](models.md), [codecs](codecs.md) and [validation](validation.md) retain source identities. The TypeDoc configuration and documentation manifests are included. Native HTML is built and verified separately. The package has zero runtime dependencies.\n\n## Build and pack\n\nThe build toolchain is pinned to Node {NODE_VERSION}, npm {NPM_VERSION} and TypeScript {TYPESCRIPT_VERSION}; the source tree includes the reviewed lock.\n\n```sh\nnpm ci --ignore-scripts\nnpm run build\nnode dist/examples/validated.js\nnpm pack --ignore-scripts\n```\n\nBuild and packing are explicit. No install, prepare or publication hook runs automatically. npm omits package-lock.json from tarballs; retain the source directory for reproducible rebuilds.\n",
        config.name,
    )
}

#[derive(Serialize)]
struct Manifest<'a> {
    #[serde(flatten)]
    metadata: &'a Value,
    exports: BTreeMap<&'static str, Export>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Export {
    // Condition order is significant. Serialize fields in declaration order,
    // not serde_json::Value's alphabetical map order.
    Typed { types: String, import: String },
    Metadata(&'static str),
}

fn entry(module: &str) -> Export {
    Export::Typed {
        types: format!("./dist/{module}.d.ts"),
        import: format!("./dist/{module}.js"),
    }
}

fn json_text(value: &impl Serialize) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("package metadata is JSON");
    text.push('\n');
    text
}

const INDEX: &str = "/** Prototype OpenAPI neutral model codecs and source-linked documentation. */\nexport * as models from '../models.js';\nexport * as codecs from '../model-codecs.js';\nexport { ModelCodecError } from '../codecs.js';\nexport type { ModelCodec } from '../codecs.js';\nexport { JsonNumber, JsonCodecError, parseJson, stringifyJson } from '../json.js';\nexport type { JsonValue, WireJsonValue, JsonLimits, JsonEncodeOptions } from '../json.js';\nexport type { ValidationSource, ValidationFinding } from '../validation.js';\n";

/// Same exports as `INDEX` with a comment that does not claim a neutral-only
/// package; used when the plan's symbols carry directional views.
const INDEX_VIEWS: &str = "/** Prototype OpenAPI view-aware model codecs and source-linked documentation. */\nexport * as models from '../models.js';\nexport * as codecs from '../model-codecs.js';\nexport { ModelCodecError } from '../codecs.js';\nexport type { ModelCodec } from '../codecs.js';\nexport { JsonNumber, JsonCodecError, parseJson, stringifyJson } from '../json.js';\nexport type { JsonValue, WireJsonValue, JsonLimits, JsonEncodeOptions } from '../json.js';\nexport type { ValidationSource, ValidationFinding } from '../validation.js';\n";

/// The distinct model views the plan actually implements, in canonical
/// `Neutral` → `Request` → `Response` order. A package without symbols, or
/// with only neutral symbols, keeps the stable `Neutral` label.
fn view_label(plan: &CodecPlan) -> String {
    let views: BTreeSet<ModelView> = plan
        .models()
        .symbols()
        .iter()
        .map(|symbol| symbol.view())
        .collect();
    if views.is_empty() || views == BTreeSet::from([ModelView::Neutral]) {
        "Neutral".to_owned()
    } else {
        views
            .iter()
            .map(|view| format!("{view:?}"))
            .collect::<Vec<_>>()
            .join("+")
    }
}

fn readme(plan: &CodecPlan, config: &PackageConfig) -> String {
    let example = plan.models().symbols().first().map_or_else(
        || "This package has no selected model roots.\n".into(),
        |symbol| format!(
            "```ts\nimport {{ codecs, type models }} from '{}';\n\nexport function roundTrip(text: string): string {{\n  const value: models.{} = codecs.{}Codec.decode(text);\n  return codecs.{}Codec.encode(value);\n}}\n```\n\nPass JSON text admitted by this model's original schema. The helper does not synthesize example data. JavaScript consumers use the same imports and methods, omitting type annotations and the `type models` import. Validation failures throw `ModelCodecError` with source and instance pointers.\n",
            config.name, symbol.name(), symbol.name(), symbol.name(),
        ),
    );
    let label = view_label(plan);
    let views = if label == "Neutral" {
        "Only the neutral model view is implemented; unsupported source assertions or representations fail codec planning.".to_owned()
    } else {
        format!(
            "The package implements the {label} model views the plan proves. Request and response views apply the documented explicit OpenAPI 3.1 context requiredness policy: only required presence proven directional for the view is relaxed, every declared property stays present and is validated, and nothing is stripped, defaulted or reordered. This is not unmodified neutral validation and not OpenAPI 3.0 normative behavior; directional annotations whose applicability cannot be proven are a source-linked planning error. Other unsupported source assertions or representations also fail codec planning."
        )
    };
    format!(
        "# {}\n\nPrototype package of OpenAPI model types, validated codecs and source-linked documentation. {views} This package has no HTTP client and is not a certified SDK release.\n\nThe package is private and has zero runtime dependencies. It targets ESM and ES2022 on Node 22 or newer. Its native build toolchain is pinned to Node {NODE_VERSION}, npm {NPM_VERSION} and TypeScript {TYPESCRIPT_VERSION}; additional Node/compiler versions require their own compatibility checks.\n\n## Use the installed package\n\n{example}\nThe root exports `models` and `codecs` namespaces, along with `ModelCodec`, `ModelCodecError`, exact JSON representations and validation finding types. Direct imports from `{}/models`, `{}/codecs` and `{}/json` preserve every original model and codec name even when a schema shares a support API name. `ModelCodec` at the root and `Codec` from the codecs subpath are generic codec types; a model named `Model` has its codec value at `codecs.ModelCodec`.\n\nUse TypeScript with `strict`, `exactOptionalPropertyTypes` and `noUncheckedIndexedAccess`. Missing and null are distinct. Unbounded integers use `bigint`; general JSON numbers use `JsonNumber`. Call the model's `decode`/`encode` methods for schema validation; the lower-level `parseJson`/`stringifyJson` functions enforce JSON representation only.\n\n## Documentation and provenance\n\n[Models](models.md), [codecs](codecs.md), and [validation](validation.md) document the generated behavior. [docs-manifest.json](docs-manifest.json) binds each public model and codec to its original OpenAPI document, JSON pointer and view; models.md retains the original schemas. The original TypeDoc configuration, README and TypeScript sources are included unchanged. Native HTML documentation is a separate build output, not implied by package emission. Source maps and declaration maps resolve to the included source files.\n\n## Build and pack\n\nFrom the generated source directory, which contains `package-lock.json`:\n\n```sh\nnpm ci --ignore-scripts\nnpm run build\nnpm pack --ignore-scripts\n```\n\nThe lock records the reviewed TypeScript registry tarball and integrity. npm omits package-lock.json from tarballs; retain the generated source directory for reproducible rebuilds. Building is explicit; no install, prepare or publication hooks run automatically. Pack only after a successful build. Installation from a local tarball verifies package resolution and codec behavior; it does not establish HTTP support or SDK release readiness.\n",
        config.name, config.name, config.name, config.name,
    )
}

fn valid_name(name: &str) -> bool {
    fn part(part: &str) -> bool {
        part.as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
    }
    if name.len() > 214 {
        return false;
    }
    if let Some(scoped) = name.strip_prefix('@') {
        return scoped
            .split_once('/')
            .is_some_and(|(scope, name)| part(scope) && part(name));
    }
    // Reserved package names and Node core modules in pinned npm 10.9.8's
    // validate-npm-package-name/builtins table. Scoped forms remain valid.
    ![
        "node_modules",
        "favicon.ico",
        "assert",
        "async_hooks",
        "buffer",
        "child_process",
        "cluster",
        "console",
        "constants",
        "crypto",
        "dgram",
        "diagnostics_channel",
        "dns",
        "domain",
        "events",
        "fs",
        "http",
        "http2",
        "https",
        "inspector",
        "module",
        "net",
        "os",
        "path",
        "perf_hooks",
        "process",
        "punycode",
        "querystring",
        "readline",
        "repl",
        "stream",
        "string_decoder",
        "sys",
        "timers",
        "tls",
        "trace_events",
        "tty",
        "url",
        "util",
        "v8",
        "vm",
        "wasi",
        "worker_threads",
        "zlib",
    ]
    .contains(&name)
        && part(name)
}

fn valid_version(version: &str) -> bool {
    fn number(part: &str) -> bool {
        !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
            && part
                .parse::<u64>()
                .is_ok_and(|n| n <= 9_007_199_254_740_991)
    }
    fn identifiers(parts: &str, prerelease: bool) -> bool {
        parts.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!prerelease
                    || !part.bytes().all(|byte| byte.is_ascii_digit())
                    || part == "0"
                    || !part.starts_with('0'))
        })
    }
    if version.len() > 256 {
        return false;
    }
    let (version, build) = version
        .split_once('+')
        .map_or((version, None), |(v, b)| (v, Some(b)));
    if build.is_some_and(|build| !identifiers(build, false)) {
        return false;
    }
    let (core, prerelease) = version
        .split_once('-')
        .map_or((version, None), |(v, p)| (v, Some(p)));
    let core: Vec<_> = core.split('.').collect();
    core.len() == 3
        && core.iter().all(|part| number(part))
        && prerelease.is_none_or(|parts| identifiers(parts, true))
}
