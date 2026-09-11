//! Python exact-JSON runtime emitter.
//!
//! Representation-only layer for generated Python packages: no schema
//! validation, no client support. `runtime_source` is the single source of
//! the emitted runtime; native model planning consumes it
//! so every generated package embeds the same audited template.

use crate::OutFile;

/// Source of the emitted `python/json_runtime.py` runtime template.
///
/// Single source of truth shared with `python_models.rs`; generated packages
/// must embed this text verbatim rather than copying it.
#[must_use]
pub fn runtime_source() -> &'static str {
    include_str!("python_json/runtime.py")
}

/// Emits the standalone Python JSON runtime template.
///
/// The path is relative to the SDK output root and matches the layout other
/// backends use (`python/json_runtime.py`). Import style for generated model
/// modules is `import json_runtime as _json`.
#[must_use]
pub fn emit() -> Vec<OutFile> {
    vec![OutFile {
        path: "python/json_runtime.py".into(),
        content: runtime_source().into(),
    }]
}
