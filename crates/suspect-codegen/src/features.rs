//! Machine-readable feature manifest for the generated-SDK backends.
//!
//! Every feature claim is tied to the acceptance evidence that proves it:
//! the test file (and, where relevant, a marker inside it) that exercises
//! the feature for that backend. The manifest test in
//! `tests/feature_manifest.rs` verifies every claim against its evidence
//! file, verifies the converse (an evidence file without a claim fails),
//! and regenerates `docs/SDK-FEATURES.md` byte-for-byte, so the matrix
//! cannot drift from reality in either direction.
//!
//! Where a claim is absent, the manifest is recording test debt, not
//! absence of implementation: C# implements environment-variable
//! credentials (`csharp_sdk/credential_env.rs`) and Go emits the ua/v1
//! attribution constants (`go_http`), but neither has an acceptance test
//! file yet, so the manifest refuses the claim until the evidence lands.

/// One manifest feature: the capability a generated SDK exposes.
pub struct Feature {
    /// Stable kebab-case identifier.
    pub id: &'static str,
    /// Human title used in the docs table.
    pub title: &'static str,
    /// One-line description of what generation produces for the feature.
    pub description: &'static str,
    /// Marker that must appear inside the evidence file, when the evidence
    /// is a shared file that needs a feature-specific anchor.
    pub evidence_marker: Option<&'static str>,
    /// The conventional per-backend evidence file suffix
    /// (`tests/<backend><suffix>`). For backends whose evidence lives in a
    /// shared file, the claim carries the override; for unclaimed backends,
    /// this file must NOT exist (it would be untested support).
    pub evidence_suffix: &'static str,
}

/// Generated-SDK backends, canonical order used by the manifest.
pub const BACKENDS: &[&str] = &[
    "cpp",
    "csharp",
    "dart",
    "go",
    "java",
    "kotlin",
    "php",
    "python",
    "ruby",
    "rust",
    "swift",
    "typescript",
];

/// The manifest features. Order is the docs table row order.
pub const FEATURES: &[Feature] = &[
    Feature {
        id: "pagination",
        title: "Pagination",
        description: "Detected pagination is compiled into native page iterators with `patterns` and `page_size` overrides.",
        evidence_marker: None,
        evidence_suffix: "_pagination.rs",
    },
    Feature {
        id: "oauth2-client-credentials",
        title: "OAuth 2.0 client credentials",
        description: "Token acquisition, refresh, and revocation/introspection runtimes with OIDC discovery.",
        evidence_marker: None,
        evidence_suffix: "_oauth.rs",
    },
    Feature {
        id: "auth-replay-401",
        title: "401 auth replay",
        description: "A 401 response triggers one credential refresh and request replay before surfacing the failure.",
        evidence_marker: Some("replay"),
        evidence_suffix: "_oauth.rs",
    },
    Feature {
        id: "typed-stream-events",
        title: "Typed server-sent events",
        description: "`text/event-stream` responses compile into typed event iteration instead of raw strings.",
        evidence_marker: None,
        evidence_suffix: "_stream_typed.rs",
    },
    Feature {
        id: "incoming-webhooks",
        title: "Incoming webhooks",
        description: "Webhook payloads compile into decoders and constructors with strict package compilation.",
        evidence_marker: None,
        evidence_suffix: "_incoming.rs",
    },
    Feature {
        id: "env-var-credentials",
        title: "Environment-variable credentials",
        description: "Security schemes resolve credentials from documented environment variables at client construction.",
        evidence_marker: None,
        evidence_suffix: "_credential_env.rs",
    },
    Feature {
        id: "user-agent-attribution",
        title: "User-agent attribution",
        description: "Requests carry the `ua/v1` attribution grammar identifying the SDK, engine, and application.",
        evidence_marker: None,
        evidence_suffix: "_attribution.rs",
    },
];

/// One claim: feature `fi` (index into [`FEATURES`]) is supported by
/// backend `bi` (index into [`BACKENDS`]), proven by the named test file
/// under `crates/suspect-codegen/tests/`.
pub struct Claim {
    /// Index into [`FEATURES`].
    pub feature: usize,
    /// Index into [`BACKENDS`].
    pub backend: usize,
    /// Evidence test-file name.
    pub evidence: &'static str,
}

const fn claim(feature: usize, backend: usize, evidence: &'static str) -> Claim {
    Claim {
        feature,
        backend,
        evidence,
    }
}

/// All claims, in `(feature, backend)` order.
const CLAIMS: &[Claim] = &[
    // pagination (0)
    claim(0, 0, "cpp_pagination.rs"),
    claim(0, 1, "csharp_pagination.rs"),
    claim(0, 2, "dart_pagination.rs"),
    claim(0, 3, "go_pagination.rs"),
    claim(0, 4, "java_pagination.rs"),
    claim(0, 5, "kotlin_pagination.rs"),
    claim(0, 6, "php_pagination.rs"),
    claim(0, 7, "python_pagination.rs"),
    claim(0, 8, "ruby_pagination.rs"),
    claim(0, 9, "rust_pagination.rs"),
    claim(0, 10, "swift_pagination.rs"),
    claim(0, 11, "typescript_pagination.rs"),
    // oauth2-client-credentials (1)
    claim(1, 0, "cpp_oauth.rs"),
    claim(1, 1, "csharp_oauth.rs"),
    claim(1, 2, "dart_oauth.rs"),
    claim(1, 3, "go_oauth.rs"),
    claim(1, 4, "java_oauth.rs"),
    claim(1, 5, "kotlin_oauth.rs"),
    claim(1, 6, "php_oauth.rs"),
    claim(1, 7, "python_oauth.rs"),
    claim(1, 8, "ruby_oauth.rs"),
    claim(1, 9, "rust_oauth.rs"),
    claim(1, 10, "swift_oauth.rs"),
    claim(1, 11, "typescript_oauth.rs"),
    // auth-replay-401 (2) — same evidence files, marker-checked
    claim(2, 0, "cpp_oauth.rs"),
    claim(2, 1, "csharp_oauth.rs"),
    claim(2, 2, "dart_oauth.rs"),
    claim(2, 3, "go_oauth.rs"),
    claim(2, 4, "java_oauth.rs"),
    claim(2, 5, "kotlin_oauth.rs"),
    claim(2, 6, "php_oauth.rs"),
    claim(2, 7, "python_oauth.rs"),
    claim(2, 8, "ruby_oauth.rs"),
    claim(2, 9, "rust_oauth.rs"),
    claim(2, 10, "swift_oauth.rs"),
    claim(2, 11, "typescript_oauth.rs"),
    // typed-stream-events (3)
    claim(3, 0, "cpp_stream_typed.rs"),
    claim(3, 1, "csharp_stream_typed.rs"),
    claim(3, 2, "dart_stream_typed.rs"),
    claim(3, 3, "go_stream_typed.rs"),
    claim(3, 4, "java_stream_typed.rs"),
    claim(3, 5, "kotlin_stream_typed.rs"),
    claim(3, 6, "php_stream_typed.rs"),
    claim(3, 7, "python_stream_typed.rs"),
    claim(3, 8, "ruby_stream_typed.rs"),
    claim(3, 9, "rust_stream_typed.rs"),
    claim(3, 10, "swift_stream_typed.rs"),
    claim(3, 11, "typescript_stream_typed.rs"),
    // incoming-webhooks (4)
    claim(4, 0, "cpp_incoming.rs"),
    claim(4, 1, "csharp_incoming.rs"),
    claim(4, 2, "dart_incoming.rs"),
    claim(4, 3, "go_incoming.rs"),
    claim(4, 4, "java_incoming.rs"),
    claim(4, 5, "kotlin_incoming.rs"),
    claim(4, 6, "php_incoming.rs"),
    claim(4, 7, "python_incoming.rs"),
    claim(4, 8, "ruby_incoming.rs"),
    claim(4, 9, "rust_incoming.rs"),
    claim(4, 10, "swift_incoming.rs"),
    claim(4, 11, "typescript_incoming.rs"),
    // env-var-credentials (5) — csharp deliberately unclaimed (test debt)
    claim(5, 0, "cpp_credential_env.rs"),
    claim(5, 2, "dart_credential_env.rs"),
    claim(5, 3, "go_credential_env.rs"),
    claim(5, 4, "java_credential_env.rs"),
    claim(5, 5, "kotlin_credential_env.rs"),
    claim(5, 6, "php_credential_env.rs"),
    claim(5, 7, "python_credential_env.rs"),
    claim(5, 8, "ruby_credential_env.rs"),
    claim(5, 9, "rust_credential_env.rs"),
    claim(5, 10, "swift_credential_env.rs"),
    claim(5, 11, "typescript_credential_env.rs"),
    // user-agent-attribution (6) — go deliberately unclaimed (test debt);
    // typescript/python/rust share the sdk-level suite
    claim(6, 0, "cpp_attribution.rs"),
    claim(6, 1, "csharp_attribution.rs"),
    claim(6, 2, "dart_attribution.rs"),
    claim(6, 4, "java_attribution.rs"),
    claim(6, 5, "kotlin_attribution.rs"),
    claim(6, 6, "php_attribution.rs"),
    claim(6, 7, "sdk_attribution.rs"),
    claim(6, 8, "ruby_attribution.rs"),
    claim(6, 9, "sdk_attribution.rs"),
    claim(6, 10, "swift_attribution.rs"),
    claim(6, 11, "sdk_attribution.rs"),
];

/// Whether the feature is claimed for the backend, and the evidence file
/// that proves it.
#[must_use]
pub fn claim_for(feature: usize, backend: usize) -> Option<&'static str> {
    CLAIMS
        .iter()
        .find(|c| c.feature == feature && c.backend == backend)
        .map(|c| c.evidence)
}

/// Whether the feature is claimed for the backend.
#[must_use]
pub fn is_supported(feature: usize, backend: usize) -> bool {
    claim_for(feature, backend).is_some()
}

/// Renders the manifest as the `docs/SDK-FEATURES.md` table.
#[must_use]
pub fn feature_matrix_markdown() -> String {
    let mut out = String::from("# Generated-SDK feature matrix\n\n");
    out.push_str("Machine-generated from [`crates/suspect-codegen/src/features.rs`](../crates/suspect-codegen/src/features.rs). Every claim is verified against the acceptance test named in the evidence column; the sync test (`tests/feature_manifest.rs`) fails if this file, the registry, or the evidence drift apart. An unclaimed cell records test debt, not necessarily absence of implementation.\n\n");
    out.push_str("| Feature | ");
    for backend in BACKENDS {
        out.push_str(backend);
        out.push_str(" | ");
    }
    out.push_str("Evidence |\n");
    out.push_str("|---|");
    for _ in BACKENDS {
        out.push_str("---|");
    }
    out.push_str("---|\n");
    for (fi, feature) in FEATURES.iter().enumerate() {
        out.push_str("| **");
        out.push_str(feature.title);
        out.push_str("** (");
        out.push_str(feature.id);
        out.push_str(") | ");
        for backend in 0..BACKENDS.len() {
            out.push_str(if is_supported(fi, backend) {
                "yes | "
            } else {
                "— | "
            });
        }
        out.push_str(&evidence_summary(fi));
        out.push_str(" |\n");
    }
    out.push_str("\n## Feature notes\n\n");
    for feature in FEATURES {
        out.push_str("- **");
        out.push_str(feature.title);
        out.push_str("** — ");
        out.push_str(feature.description);
        out.push('\n');
    }
    out.push_str("\n## Known evidence gaps\n\n");
    for (fi, feature) in FEATURES.iter().enumerate() {
        let unclaimed: Vec<&str> = BACKENDS
            .iter()
            .enumerate()
            .filter(|(bi, _)| !is_supported(fi, *bi))
            .map(|(_, b)| *b)
            .collect();
        if !unclaimed.is_empty() {
            out.push_str("- **");
            out.push_str(feature.title);
            out.push_str("**: no acceptance evidence for ");
            out.push_str(&unclaimed.join(", "));
            out.push_str(".\n");
        }
    }
    out
}

/// Compact evidence description for one feature's table row: the common
/// file pattern, or the specific files when evidence is split.
fn evidence_summary(feature: usize) -> String {
    let mut files: Vec<&str> = Vec::new();
    for backend in 0..BACKENDS.len() {
        if let Some(evidence) = claim_for(feature, backend)
            && !files.contains(&evidence)
        {
            files.push(evidence);
        }
    }
    if files.len() == 1 {
        return format!("`tests/{}`", files[0]);
    }
    files
        .iter()
        .map(|f| format!("`tests/{f}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
