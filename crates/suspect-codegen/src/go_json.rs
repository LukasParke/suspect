//! Dependency-free bounded exact JSON runtime emitted as a standalone Go
//! package.

use crate::OutFile;

/// Go source of the representation runtime (`package sdk`). Representation
/// only: this checks JSON values, not an OpenAPI schema. Schema codecs and
/// clients must enforce their compiled source contracts separately, so this
/// runtime never replaces model codecs and makes no release-readiness claim.
/// Consumers copy this source verbatim; it needs no path or package rewriting.
#[must_use]
pub fn runtime_source() -> &'static str {
    include_str!("go_json/runtime.go")
}

const GO_MOD: &str = "module example.com/generated-json\n\ngo 1.23.0\n";

/// Emits the standalone JSON runtime package: `go/json.go` (`package sdk`)
/// plus a standalone `go/go.mod` with no third-party dependencies.
#[must_use]
pub fn emit() -> Vec<OutFile> {
    vec![
        OutFile {
            path: "go/json.go".into(),
            content: runtime_source().into(),
        },
        OutFile {
            path: "go/go.mod".into(),
            content: GO_MOD.into(),
        },
    ]
}
