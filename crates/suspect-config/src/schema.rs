//! The setting table: one row per configuration key.
//!
//! Parsing, environment variables, `suspect config --explain`, the
//! language server's hover/completion/validation of `.suspect.yaml` and
//! `suspect.project.json`, and the checked-in JSON Schemas all read these
//! tables, so a key documented in one place is accepted, explained and
//! validated identically everywhere.
//!
//! The typed [`crate::Settings`] struct is kept in step with [`SETTINGS`]
//! by a round-trip test (`tests/resolve.rs`), not by the compiler.
//!
//! [`problems()`] checks a configuration document against the tables.

pub mod problems;

pub use problems::{FileKind, Problem, document_problems, kind_for, kind_of, problems};

/// What a key accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A nested section; its keys are separate rows.
    Section,
    /// `true` / `false`.
    Bool,
    /// Free text.
    Str,
    /// A path, relative to the file that declares it.
    Path,
    /// An absolute URL.
    Url,
    /// A TCP port, 0-65535 (0 picks a free port).
    Port,
    /// A positive whole number.
    Count,
    /// One of a fixed set of names.
    Enum(&'static [&'static str]),
    /// A severity floor: `error`, `warning`, `info` or `hint`.
    Severity,
    /// A policy severity: a [`Kind::Severity`] name or `off`.
    PolicySeverity,
    /// A map with user-chosen keys (rule ids, profile names, ...).
    Map(&'static Kind),
    /// A list of strings.
    List,
    /// An opaque section owned by one consumer (`extensions`).
    Object,
    /// A list of SDK target objects (manifest `codegen`).
    Targets,
}

/// Canonical severity floor names, in descending order.
pub const SEVERITIES: &[&str] = &["error", "warning", "info", "hint"];
/// Canonical policy severity names: the floor names plus `off`.
pub const POLICY_SEVERITIES: &[&str] = &["error", "warning", "info", "hint", "off"];
const DOC_STYLES: &[&str] = &["html", "markdown", "sveltekit"];
const REPORTS: &[&str] = &["text", "ndjson"];
const GATEWAY_MODES: &[&str] = &["mock", "proxy", "validate", "record", "replay", "scenario"];

impl Kind {
    /// The type as it reads in a hover or an error message.
    #[must_use]
    pub const fn type_name(self) -> &'static str {
        match self {
            Self::Section => "section",
            Self::Bool => "boolean",
            Self::Str | Self::Enum(_) | Self::Severity | Self::PolicySeverity => "string",
            Self::Path => "path",
            Self::Url => "URL",
            Self::Port => "port",
            Self::Count => "integer",
            Self::Map(_) => "map",
            Self::List => "list of strings",
            Self::Object => "section",
            Self::Targets => "list of SDK targets",
        }
    }

    /// The permitted values, when the kind is an enumeration.
    #[must_use]
    pub const fn values(self) -> &'static [&'static str] {
        match self {
            Self::Enum(values) => values,
            Self::Severity => SEVERITIES,
            Self::PolicySeverity => POLICY_SEVERITIES,
            _ => &[],
        }
    }
}

/// One configuration key.
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    /// Dotted path in the file: `lint.min_severity`. `""` is the root.
    pub path: &'static str,
    /// What the key accepts.
    pub kind: Kind,
    /// Default as written in YAML, for docs and the schema; `None` = unset.
    pub default: Option<&'static str>,
    /// What it does, for hover, `--explain` and the schema.
    pub doc: &'static str,
    /// Environment variable that sets it.
    pub env: Option<&'static str>,
    /// Client settings key under `suspect.` (today's spelling).
    pub editor: Option<&'static str>,
    /// Flags that set it, for `--explain` and the CLI reference.
    pub flags: &'static [&'static str],
    /// Older spellings still accepted, with a warning naming this path.
    pub aliases: &'static [&'static str],
}

const fn row(path: &'static str, kind: Kind, doc: &'static str) -> Setting {
    Setting {
        path,
        kind,
        default: None,
        doc,
        env: None,
        editor: None,
        flags: &[],
        aliases: &[],
    }
}

const fn section(path: &'static str, doc: &'static str) -> Setting {
    row(path, Kind::Section, doc)
}

impl Setting {
    const fn default(mut self, value: &'static str) -> Self {
        self.default = Some(value);
        self
    }
    const fn env(mut self, name: &'static str) -> Self {
        self.env = Some(name);
        self
    }
    const fn editor(mut self, key: &'static str) -> Self {
        self.editor = Some(key);
        self
    }
    const fn flags(mut self, flags: &'static [&'static str]) -> Self {
        self.flags = flags;
        self
    }
    const fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    /// The permitted values, when the key is an enumeration.
    #[must_use]
    pub const fn values(&self) -> &'static [&'static str] {
        self.kind.values()
    }

    /// Whether the row holds a value (not a section header).
    #[must_use]
    pub const fn is_value(&self) -> bool {
        !matches!(self.kind, Kind::Section)
    }
}

/// `.suspect.yaml`: the settings every command, CI and the editor read.
pub static SETTINGS: &[Setting] = &[
    section(
        "",
        "Workspace-wide settings. Every suspect command, the language server and CI \
         read this file, so the editor, a shell and CI see identical findings.",
    ),
    row(
        "extends",
        Kind::Path,
        "A parent settings file, relative to this one. Its values apply under this \
         file's; one level only.",
    ),
    section("lint", "Which findings are reported, and from where."),
    row(
        "lint.ruleset",
        Kind::Path,
        "Custom ruleset document, relative to the file that sets it.",
    )
    .env("SUSPECT_LINT_RULESET")
    .editor("lint.ruleset")
    .flags(&["lint --ruleset"]),
    row(
        "lint.min_severity",
        Kind::Severity,
        "Minimum severity to report. Findings below this floor are suppressed.",
    )
    .default("hint")
    .env("SUSPECT_LINT_MIN_SEVERITY")
    .editor("lint.minSeverity")
    .flags(&["lint --min-severity"]),
    row(
        "lint.design",
        Kind::PolicySeverity,
        "Severity for design-class rules: findings that flag the API's design rather \
         than the document's accuracy. A project documenting an API it does not own \
         sets `off` (or `info` to keep them as notes); `lint.rules` wins per rule.",
    )
    .editor("lint.design"),
    row(
        "lint.rules",
        Kind::Map(&Kind::PolicySeverity),
        "Rule id to severity (`error`, `warning`, `info`, `hint`, `off`), overriding \
         the severity a rule would otherwise produce. Merged per rule across files.",
    )
    .default("{}")
    .editor("lint.rules"),
    row(
        "lint.recommended",
        Kind::Bool,
        "Apply the recommended (built-in) lint rule set. With `false` and no \
         `lint.ruleset`, no lint rules run.",
    )
    .default("true")
    .editor("lint.recommended"),
    section("validate", "How strictly documents are validated."),
    row(
        "validate.strict_format",
        Kind::Bool,
        "Assert `format` keywords. The 2020-12 default is annotation-only, so this \
         catches example drift that annotation-only mode would let through.",
    )
    .default("false")
    .env("SUSPECT_STRICT_FORMAT")
    .editor("validate.strictFormat")
    .flags(&["validate --strict-format"]),
    row(
        "validate.reference_allowlist",
        Kind::Path,
        "File listing the documents `$ref`s may reach; others are reported.",
    )
    .flags(&["validate --reference-allowlist"]),
    section("format", "Formatter output.").aliases(&["formatting"]),
    row(
        "format.json",
        Kind::Bool,
        "Emit JSON regardless of the input extension.",
    )
    .default("false")
    .flags(&["fmt --json"]),
    row(
        "format.yaml",
        Kind::Bool,
        "Emit YAML regardless of the input extension.",
    )
    .default("false")
    .flags(&["fmt --yaml"]),
    row(
        "format.sort_keys",
        Kind::Bool,
        "Reorder object keys into the canonical OpenAPI order when formatting. \
         Defaults to `true` when a `format` section exists, else `false`.",
    )
    .default("false")
    .editor("formatting.sortKeys")
    .aliases(&["formatting.sort_keys", "formatting.sortKeys"]),
    section("docs", "Reference documentation generation."),
    row(
        "docs.style",
        Kind::Enum(DOC_STYLES),
        "Output style for the reference site.",
    )
    .flags(&["docs --style"]),
    row(
        "docs.output",
        Kind::Path,
        "Output root for generated documentation.",
    )
    .flags(&["docs --output"])
    .aliases(&["docs.out"]),
    section("codegen", "Defaults applied to SDK generation."),
    row(
        "codegen.out",
        Kind::Path,
        "Default output root, relative to the file that sets it.",
    )
    .flags(&["codegen --out"]),
    row("codegen.profile", Kind::Str, "Default native profile id.").flags(&["codegen --profile"]),
    row(
        "codegen.package_version",
        Kind::Str,
        "Default package version.",
    )
    .flags(&["codegen --package-version"]),
    section("ref", "Reference-workspace tuning.").aliases(&["refs"]),
    row(
        "ref.max_docs",
        Kind::Count,
        "Maximum documents loaded in one ref workspace.",
    )
    .default("500")
    .editor("ref.maxDocs")
    .aliases(&["ref.maxDocs", "refs.max_docs", "refs.maxDocs"]),
    section("editor", "Editor-only behaviour."),
    section("editor.inlay_hints", "Inlay hint toggles.").aliases(&["inlay_hints"]),
    row(
        "editor.inlay_hints.refs",
        Kind::Bool,
        "Show resolved `$ref` target inlay hints.",
    )
    .default("true")
    .editor("inlayHints.refs")
    .aliases(&[
        "inlay_hints.refs",
        "inlay_hints.ref_targets",
        "editor.inlay_hints.ref_targets",
    ]),
    row(
        "editor.inlay_hints.properties",
        Kind::Bool,
        "Show property type-annotation inlay hints.",
    )
    .default("true")
    .editor("inlayHints.properties")
    .aliases(&["inlay_hints.properties"]),
    section("test", "Contract tests, fuzzing and load runs."),
    row("test.base_url", Kind::Url, "Base URL requests are sent to.")
        .default("http://localhost:8080")
        .env("SUSPECT_BASE_URL")
        .editor("testBaseUrl")
        .flags(&[
            "test --base-url",
            "fuzz --base-url",
            "stateful --base-url",
            "load --base-url",
        ])
        .aliases(&["run.base_url"]),
    row(
        "test.credentials",
        Kind::Path,
        "Credentials file for the security schemes tests exercise. Default: \
         `.suspect/credentials.json` discovered upward.",
    )
    .env("SUSPECT_CREDENTIALS")
    .flags(&["test --credentials", "auth check --credentials"]),
    row(
        "test.report",
        Kind::Enum(REPORTS),
        "Event stream format for `suspect test`.",
    )
    .default("text")
    .flags(&["test --report"]),
    section("gateway", "The mock/proxy gateway."),
    row("gateway.port", Kind::Port, "Port the gateway listens on.")
        .default("8080")
        .editor("gatewayPort")
        .flags(&["gateway --port"]),
    row("gateway.mode", Kind::Enum(GATEWAY_MODES), "Operating mode.")
        .default("mock")
        .flags(&["gateway --mode"]),
    row(
        "gateway.upstream",
        Kind::Url,
        "Upstream base URL for proxy, validate and record modes.",
    )
    .flags(&["gateway --upstream"]),
    section("diff", "Change detection against git history."),
    row(
        "diff.baseline",
        Kind::Str,
        "Git ref compared against for breaking changes (`suspect ci`, the editor's \
         breaking-changes command; the editor falls back to `HEAD~1`).",
    )
    .env("SUSPECT_GIT_BASE")
    .editor("ci.baseline")
    .flags(&["ci --baseline"]),
    section("rules", "TypeScript rule plugins."),
    row(
        "rules.enabled",
        Kind::Bool,
        "Run TypeScript rule plugins in the editor and `check`.",
    )
    .default("false"),
    row(
        "rules.roots",
        Kind::List,
        "Directories searched for rule plugins, relative to the file that sets them.",
    )
    .default("[\".suspect/rules\"]"),
    row(
        "extensions",
        Kind::Object,
        "Vendor extensions: ordering anchors (`before`, `after`), `contexts` and a \
         JSON Schema (`schema`, inline or a path) per `x-` key.",
    ),
];

/// `suspect.project.json`: what a project builds.
pub static MANIFEST: &[Setting] = &[
    section(
        "",
        "Project manifest: the published spec, its profiles, SDK targets, \
         documentation and contract tests.",
    ),
    row(
        "version",
        Kind::Count,
        "Manifest schema version. Must be 1.",
    ),
    row("name", Kind::Str, "Project name.").default("the directory name"),
    row(
        "entry",
        Kind::Path,
        "Entry OpenAPI document, relative to the manifest directory.",
    ),
    row(
        "config",
        Kind::Path,
        "The settings file for this project (`.suspect.yaml`), relative to the \
         manifest. Lint, validate and editor settings live there.",
    ),
    row(
        "overlays",
        Kind::List,
        "Overlay documents applied in order to produce the published spec.",
    )
    .default("[]"),
    section(
        "publish",
        "Where the published (post-overlay) spec is written.",
    ),
    row(
        "publish.output",
        Kind::Path,
        "Where the published spec is written, relative to the manifest.",
    )
    .default("build/spec.yaml"),
    row(
        "publish.profiles",
        Kind::Map(&Kind::List),
        "Named publication profiles: each is an extra overlay list applied on top \
         of the published spec (for example `public` strips internals).",
    )
    .default("{}"),
    section("contract", "The contract package for this project."),
    row(
        "contract.output",
        Kind::Path,
        "Where the contract package is written.",
    ),
    section(
        "lint",
        "Deprecated: lint policy belongs in `.suspect.yaml`. Still applied under \
         that file for one release; `suspect config migrate` moves it.",
    ),
    row(
        "lint.min_severity",
        Kind::Severity,
        "Deprecated; see `lint.min_severity` in `.suspect.yaml`.",
    ),
    row(
        "lint.rules",
        Kind::Map(&Kind::PolicySeverity),
        "Deprecated; see `lint.rules` in `.suspect.yaml`.",
    ),
    row(
        "lint.design",
        Kind::PolicySeverity,
        "Deprecated; see `lint.design` in `.suspect.yaml`.",
    ),
    row(
        "lint.ruleset",
        Kind::Path,
        "Deprecated; see `lint.ruleset` in `.suspect.yaml`.",
    ),
    row(
        "lint.recommended",
        Kind::Bool,
        "Deprecated; see `lint.recommended` in `.suspect.yaml`.",
    ),
    section(
        "validate",
        "Deprecated: validation settings belong in `.suspect.yaml`.",
    ),
    row(
        "validate.strict_format",
        Kind::Bool,
        "Deprecated; see `validate.strict_format` in `.suspect.yaml`.",
    ),
    section(
        "editor",
        "Deprecated: editor settings belong in `.suspect.yaml` (`editor`, `ref`, \
         `format`).",
    ),
    section(
        "editor.inlay_hints",
        "Deprecated; see `editor.inlay_hints`.",
    ),
    row(
        "editor.inlay_hints.refs",
        Kind::Bool,
        "Deprecated; see `editor.inlay_hints.refs`.",
    ),
    row(
        "editor.inlay_hints.properties",
        Kind::Bool,
        "Deprecated; see `editor.inlay_hints.properties`.",
    ),
    section("editor.ref", "Deprecated; see `ref`."),
    row(
        "editor.ref.max_docs",
        Kind::Count,
        "Deprecated; see `ref.max_docs`.",
    ),
    section("editor.formatting", "Deprecated; see `format`."),
    row(
        "editor.formatting.sort_keys",
        Kind::Bool,
        "Deprecated; see `format.sort_keys`.",
    ),
    section("docs", "Documentation target."),
    row("docs.style", Kind::Enum(DOC_STYLES), "Output style."),
    row(
        "docs.output",
        Kind::Path,
        "Output root for generated documentation.",
    ),
    row(
        "codegen",
        Kind::Targets,
        "SDK generation targets built from the published spec.",
    )
    .default("[]"),
    row(
        "release",
        Kind::Object,
        "SDK release: the version every target carries and how each is published \
         (`release-publish`, `release-workflow`). Replaces a standalone `release.json`.",
    ),
    section(
        "tests",
        "Contract-test targets: Arazzo documents run against `base_url`, or offline \
         from `cassette`.",
    ),
    row(
        "tests.arazzo",
        Kind::List,
        "Arazzo documents to compile and run.",
    )
    .default("[]"),
    row(
        "tests.base_url",
        Kind::Url,
        "Base URL for this project's suites. `SUSPECT_BASE_URL` overrides it; it \
         overrides `test.base_url` in `.suspect.yaml`.",
    ),
    row(
        "tests.cassette",
        Kind::Path,
        "Run offline against this cassette instead of live HTTP.",
    ),
    row(
        "tests.message_broker",
        Kind::Path,
        "Message broker directory for Arazzo 1.1 AsyncAPI steps.",
    ),
    row(
        "tests.credentials",
        Kind::Path,
        "Credentials file for the security schemes the suites exercise.",
    ),
    row(
        "tests.inputs",
        Kind::Object,
        "Workflow inputs supplied to every declared suite: name to value.",
    ),
];

/// The fields of one manifest `codegen` target.
pub static TARGET_FIELDS: &[Setting] = &[
    row("name", Kind::Str, "Target name, used in progress output.").default("the profile id"),
    row(
        "profile",
        Kind::Str,
        "The native profile id (`typescript-http`, `python-http`, ...).",
    ),
    row("package_name", Kind::Str, "Native package identity."),
    row("package_version", Kind::Str, "Package SemVer."),
    row("out", Kind::Path, "Output root, relative to the manifest.").default("sdk"),
    row(
        "operation_id",
        Kind::List,
        "Exact operationId selectors; empty selects all outgoing operations.",
    )
    .default("[]"),
    row(
        "import_name",
        Kind::Str,
        "Explicit import/module/namespace identity where the profile needs one.",
    ),
    row(
        "check",
        Kind::Bool,
        "When true, only ownership/drift is checked; nothing is written.",
    )
    .default("false"),
    row(
        "compatibility_profiles",
        Kind::List,
        "Interpretation profiles applied to the source (`--compatibility-profile`).",
    ),
    row(
        "credential_env",
        Kind::Object,
        "Credential environment variable mapping (`codegen --defaults` schema).",
    ),
    row(
        "sdk_defaults",
        Kind::Object,
        "SDK defaults: pagination, OAuth lifecycle (`codegen --defaults` schema).",
    ),
    row(
        "options",
        Kind::Object,
        "Generation options in the `codegen --defaults` schema (`compatibility_profiles`, \
         `credential_env`, `sdk_defaults`). A key may be set here or beside it, not both.",
    ),
];

/// The fields of the manifest `release` section. `release` is one
/// [`Kind::Object`] row in [`MANIFEST`]: `release-publish` validates it,
/// and the JSON Schema spells these fields out.
pub static RELEASE_FIELDS: &[Setting] = &[
    row(
        "version",
        Kind::Str,
        "The release version every target must carry.",
    ),
    row(
        "tag",
        Kind::Str,
        "The tag that triggers the release workflow; `v` plus the version when unset.",
    ),
    row(
        "targets",
        Kind::List,
        "SDK publish targets, in manifest order.",
    ),
];

/// The fields of one `release.targets` entry.
pub static RELEASE_TARGET_FIELDS: &[Setting] = &[
    row(
        "backend",
        Kind::Str,
        "Backend id (`typescript`, `python`, ...).",
    ),
    row(
        "directory",
        Kind::Path,
        "Generated package directory, relative to the manifest.",
    ),
    row("package", Kind::Str, "Native package identity."),
    row(
        "registry",
        Kind::Str,
        "Registry id (`npm`, `pypi`, `maven-central`, ...).",
    ),
    row(
        "publish",
        Kind::Str,
        "Exact publish command, run in `directory`.",
    ),
    row(
        "version",
        Kind::Str,
        "This target's version, when it differs from the release version.",
    ),
    row(
        "version_skew_reason",
        Kind::Str,
        "Why this target's version differs from the release version.",
    ),
    row(
        "depends_on",
        Kind::List,
        "Backends that must publish first.",
    ),
];

/// A `SUSPECT_*` variable the binary reads that is not a setting: it
/// changes diagnostics output only.
#[derive(Debug, Clone, Copy)]
pub struct DebugVar {
    /// Variable name.
    pub name: &'static str,
    /// What it prints.
    pub doc: &'static str,
}

/// Every debug variable the shipped binary reads.
pub static DEBUG_VARS: &[DebugVar] = &[
    DebugVar {
        name: "SUSPECT_PROFILE",
        doc: "lint/validate phase timings on stderr",
    },
    DebugVar {
        name: "SUSPECT_GW_PROFILE",
        doc: "gateway request timings on stderr",
    },
    DebugVar {
        name: "SUSPECT_BUILD_PROFILE",
        doc: "project build and contract stage timings on stderr",
    },
    DebugVar {
        name: "SUSPECT_CONTRACT_TRACE",
        doc: "contract compiler trace on stderr",
    },
    DebugVar {
        name: "SUSPECT_RULES_DEBUG",
        doc: "TypeScript rule host and worker trace on stderr",
    },
    DebugVar {
        name: "SUSPECT_HLDBG",
        doc: "semantic highlighting trace on stderr (language server)",
    },
];

/// A row found by [`lookup`], and whether it was reached through an alias.
#[derive(Debug, Clone, Copy)]
pub struct Found {
    /// The row.
    pub setting: &'static Setting,
    /// The alias spelling used, when it was not the canonical path.
    pub alias: Option<&'static str>,
}

/// The row for `path` in `table`, by canonical path or alias.
#[must_use]
pub fn lookup_in(table: &'static [Setting], path: &str) -> Option<Found> {
    if let Some(setting) = table.iter().find(|s| s.path == path) {
        return Some(Found {
            setting,
            alias: None,
        });
    }
    table.iter().find_map(|setting| {
        setting
            .aliases
            .iter()
            .find(|a| **a == path)
            .map(|alias| Found {
                setting,
                alias: Some(alias),
            })
    })
}

/// The [`SETTINGS`] row for `path`, by canonical path or alias.
#[must_use]
pub fn lookup(path: &str) -> Option<Found> {
    lookup_in(SETTINGS, path)
}

/// The [`SETTINGS`] row a client settings key names: the `editor` column
/// first (`lint.minSeverity`), then the path with each camelCase segment
/// converted to snake_case, then aliases.
#[must_use]
pub fn lookup_editor(key: &str) -> Option<Found> {
    if let Some(setting) = SETTINGS.iter().find(|s| s.editor == Some(key)) {
        return Some(Found {
            setting,
            alias: None,
        });
    }
    let snake: Vec<String> = key.split('.').map(snake_case).collect();
    lookup(&snake.join(".")).or_else(|| lookup(key))
}

/// `maxDocs` -> `max_docs`; already-snake keys are unchanged.
#[must_use]
pub fn snake_case(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len() + 4);
    for c in segment.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Every value row (no section headers), in table order.
pub fn values() -> impl Iterator<Item = &'static Setting> {
    SETTINGS.iter().filter(|s| s.is_value())
}

/// Every environment variable that sets a value, with its row.
pub fn env_vars() -> impl Iterator<Item = (&'static str, &'static Setting)> {
    SETTINGS.iter().filter_map(|s| s.env.map(|e| (e, s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_aliases_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for setting in SETTINGS {
            assert!(seen.insert(setting.path), "duplicate {}", setting.path);
            for alias in setting.aliases {
                assert!(seen.insert(alias), "duplicate alias {alias}");
            }
        }
    }

    #[test]
    fn every_row_sits_in_a_declared_section() {
        for table in [SETTINGS, MANIFEST] {
            for setting in table {
                if let Some((parent, _)) = setting.path.rsplit_once('.') {
                    assert!(
                        table
                            .iter()
                            .any(|s| s.path == parent && s.kind == Kind::Section),
                        "{} has no section row",
                        setting.path
                    );
                }
            }
        }
    }

    #[test]
    fn editor_keys_resolve_in_either_spelling() {
        assert_eq!(
            lookup_editor("lint.minSeverity").unwrap().setting.path,
            "lint.min_severity"
        );
        assert_eq!(
            lookup_editor("lint.min_severity").unwrap().setting.path,
            "lint.min_severity"
        );
        assert_eq!(
            lookup_editor("inlayHints.refTargets").unwrap().setting.path,
            "editor.inlay_hints.refs"
        );
        assert_eq!(
            lookup_editor("testBaseUrl").unwrap().setting.path,
            "test.base_url"
        );
        assert!(lookup_editor("basePath").is_none());
    }

    #[test]
    fn env_names_are_unique_and_not_debug_vars() {
        let mut seen = std::collections::BTreeSet::new();
        for (name, _) in env_vars() {
            assert!(seen.insert(name), "{name} set by two rows");
            assert!(DEBUG_VARS.iter().all(|d| d.name != name));
        }
    }
}
