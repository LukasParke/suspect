//! Dependency-free exact JSON runtime emitted for canonical TypeScript codecs.

use crate::OutFile;

/// Emits the JSON representation/parser/encoder used by generated codecs.
/// This checks JSON representation, not an OpenAPI schema. Schema codecs and
/// clients must enforce their compiled source contracts separately.
#[must_use]
pub fn runtime() -> OutFile {
    OutFile {
        path: "typescript/json.ts".into(),
        content: include_str!("json.ts").into(),
    }
}
